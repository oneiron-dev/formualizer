use super::*;
use formualizer_common::parse_a1_1based;

#[inline]
fn normalize_name_key(name: &str) -> String {
    name.to_lowercase()
}

/// Validate that a name conforms to Excel naming rules.
fn is_valid_excel_name(name: &str) -> bool {
    // Excel name rules:
    // 1. Must start with a letter, underscore, or backslash
    // 2. Can contain letters, numbers, periods, and underscores
    // 3. Cannot be a cell reference (like A1, B2, etc.)
    // 4. Cannot exceed 255 characters
    // 5. Cannot contain spaces

    if name.is_empty() || name.len() > 255 {
        return false;
    }

    if parse_a1_1based(name).is_ok() {
        return false;
    }

    let mut chars = name.chars();

    // First character must be letter, underscore, or backslash
    if let Some(first) = chars.next()
        && !first.is_alphabetic()
        && first != '_'
        && first != '\\'
    {
        return false;
    }

    // Remaining characters must be letters, digits, periods, or underscores
    for c in chars {
        if !c.is_alphanumeric() && c != '.' && c != '_' {
            return false;
        }
    }

    true
}

/// Helper function to adjust a named definition during structural operations.
///
/// Named definitions track structural edits regardless of `$` anchors, matching
/// formula references. Absolute markers affect copy/fill, not structural shifts.
fn adjust_named_definition(
    definition: &mut NamedDefinition,
    adjuster: &crate::engine::graph::editor::reference_adjuster::ReferenceAdjuster,
    operation: &crate::engine::graph::editor::reference_adjuster::ShiftOperation,
    context: &crate::engine::graph::editor::reference_adjuster::ReferenceContext<'_>,
) -> Result<(), ExcelError> {
    use crate::engine::graph::editor::reference_adjuster::AbsShiftPolicy;
    let mut invalidated = false;
    match definition {
        NamedDefinition::Cell(cell_ref) => {
            if let Some(adjusted) =
                adjuster.adjust_cell_ref_with_policy(cell_ref, operation, AbsShiftPolicy::Track)
            {
                *cell_ref = adjusted;
            } else {
                invalidated = true;
            }
        }
        NamedDefinition::Range(range_ref) => {
            let adjusted_start = adjuster.adjust_cell_ref_with_policy(
                &range_ref.start,
                operation,
                AbsShiftPolicy::Track,
            );
            let adjusted_end = adjuster.adjust_cell_ref_with_policy(
                &range_ref.end,
                operation,
                AbsShiftPolicy::Track,
            );

            if let (Some(start), Some(end)) = (adjusted_start, adjusted_end) {
                range_ref.start = start;
                range_ref.end = end;
            } else {
                invalidated = true;
            }
        }
        NamedDefinition::Literal(_) => {
            // Constant names are not affected by structural shifts.
        }
        NamedDefinition::Formula {
            ast,
            dependencies,
            range_deps,
        } => {
            let adjusted_ast = adjuster.adjust_ast_with_policy_in_context(
                ast,
                operation,
                AbsShiftPolicy::Track,
                context,
            );
            *ast = adjusted_ast;

            dependencies.clear();
            range_deps.clear();
        }
    }
    if invalidated {
        *definition = NamedDefinition::Formula {
            ast: formualizer_parse::parser::ASTNode::new(
                formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Error(
                    ExcelError::new(ExcelErrorKind::Ref),
                )),
                None,
            ),
            dependencies: Vec::new(),
            range_deps: Vec::new(),
        };
    }
    Ok(())
}

impl DependencyGraph {
    #[inline]
    pub(crate) fn name_lookup_key(&self, name: &str) -> String {
        if self.config.case_sensitive_names {
            name.to_string()
        } else {
            normalize_name_key(name)
        }
    }

    fn canonical_name_in_scope(&self, scope: NameScope, name: &str) -> Option<String> {
        let key = self.name_lookup_key(name);
        match scope {
            NameScope::Workbook => self.named_ranges_lookup.get(&key).cloned(),
            NameScope::Sheet(sheet_id) => self
                .sheet_named_ranges_lookup
                .get(&(sheet_id, key))
                .cloned(),
        }
    }

    /// Allocate the next address in the symbol space.
    ///
    /// Symbols are identified by name and have no position. They used to be handed
    /// fabricated grid coordinates on a real sheet, which let grid operations reach them
    /// (#302, #304); a `SymbolAddr` is not a position and cannot be reached that way.
    pub(super) fn next_symbol_addr(&mut self) -> VertexAddr {
        let seq = self.symbol_vertex_seq;
        self.symbol_vertex_seq = self.symbol_vertex_seq.wrapping_add(1);
        VertexAddr::symbol(SymbolAddr::new(seq))
    }

    /// Allocate a vertex in the symbol address space.
    ///
    /// `scope_sheet_id` is recorded as lookup metadata only: it says which scope the symbol
    /// answers queries for, never where it lives. Symbol vertices are absent from
    /// `cell_to_vertex` and from every sheet index by construction, because they have no
    /// grid address to key them by.
    pub(super) fn allocate_symbol_vertex(
        &mut self,
        kind: VertexKind,
        scope_sheet_id: SheetId,
    ) -> VertexId {
        let addr = self.next_symbol_addr();
        let vertex_id = self.store.allocate(addr, scope_sheet_id, 0x01);
        self.store.set_kind(vertex_id, kind);
        self.edges.add_vertex(addr, vertex_id.0);
        vertex_id
    }

    pub(super) fn allocate_name_vertex(&mut self, scope: NameScope) -> VertexId {
        // Scope is lookup metadata, not an address: a workbook-scoped name is not a
        // resident of the default sheet.
        let scope_sheet_id = match scope {
            NameScope::Sheet(id) => id,
            NameScope::Workbook => self.default_sheet_id,
        };
        let vertex_id = self.allocate_symbol_vertex(VertexKind::NamedScalar, scope_sheet_id);
        self.mark_vertex_dirty(vertex_id);
        vertex_id
    }

    // Named Range Methods

    pub(crate) fn validate_define_name(
        &self,
        name: &str,
        scope: NameScope,
    ) -> Result<(), ExcelError> {
        if !is_valid_excel_name(name) {
            return Err(
                ExcelError::new(ExcelErrorKind::Name).with_message(format!("Invalid name: {name}"))
            );
        }

        let lookup_key = self.name_lookup_key(name);
        match scope {
            NameScope::Workbook => {
                if let Some(existing) = self.named_ranges_lookup.get(&lookup_key) {
                    return Err(ExcelError::new(ExcelErrorKind::Name).with_message(format!(
                        "Name collision under normalization: '{name}' conflicts with '{existing}'"
                    )));
                }
            }
            NameScope::Sheet(sheet_id) => {
                if let Some(existing) = self.sheet_named_ranges_lookup.get(&(sheet_id, lookup_key))
                {
                    return Err(ExcelError::new(ExcelErrorKind::Name).with_message(format!(
                        "Name collision under normalization in sheet: '{name}' conflicts with '{existing}'"
                    )));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_existing_name(
        &self,
        name: &str,
        scope: NameScope,
    ) -> Result<(), ExcelError> {
        self.canonical_name_in_scope(scope, name)
            .map(|_| ())
            .ok_or_else(|| {
                ExcelError::new(ExcelErrorKind::Name)
                    .with_message(format!("Name not found: {name}"))
            })
    }

    /// Define a new named range
    pub fn define_name(
        &mut self,
        name: &str,
        definition: NamedDefinition,
        scope: NameScope,
    ) -> Result<(), ExcelError> {
        self.validate_define_name(name, scope)?;

        let mut final_definition = definition;
        // Extract dependencies if formula
        if let NamedDefinition::Formula { ref ast, .. } = final_definition {
            let (deps, range_deps, _, _) = self.extract_dependencies(
                ast,
                match scope {
                    NameScope::Sheet(id) => id,
                    NameScope::Workbook => self.default_sheet_id,
                },
            )?;
            final_definition = NamedDefinition::Formula {
                ast: ast.clone(),
                dependencies: deps,
                range_deps,
            };
        }

        // Allocate vertex only after dependency extraction succeeds
        let vertex_id = self.allocate_name_vertex(scope);

        let named_range = NamedRange {
            definition: final_definition,
            scope,
            dependents: FxHashSet::default(),
            vertex: vertex_id,
        };

        if matches!(named_range.definition, NamedDefinition::Range(_)) {
            self.store.set_kind(vertex_id, VertexKind::NamedArray);
        } else {
            self.store.set_kind(vertex_id, VertexKind::NamedScalar);
        }

        // Formula dependencies are re-extracted here to share registration with update/reindex paths.
        let referenced_names =
            self.rebuild_name_dependencies(vertex_id, &named_range.definition, scope)?;
        if !referenced_names.is_empty() {
            self.attach_vertex_to_names(vertex_id, &referenced_names);
        }

        let key = name.to_string();

        match scope {
            NameScope::Workbook => {
                self.named_ranges.insert(key.clone(), named_range);
                self.named_ranges_lookup
                    .insert(self.name_lookup_key(&key), key.clone());
            }
            NameScope::Sheet(id) => {
                self.sheet_named_ranges
                    .insert((id, key.clone()), named_range);
                self.sheet_named_ranges_lookup
                    .insert((id, self.name_lookup_key(&key)), key.clone());
            }
        }

        self.name_vertex_lookup.insert(vertex_id, (scope, key));
        self.resolve_pending_name_references(scope, name);
        self.bump_symbol_revision();

        Ok(())
    }

    /// Iterate workbook-scoped named ranges (for bindings/testing)
    pub fn named_ranges_iter(&self) -> impl Iterator<Item = (&String, &NamedRange)> {
        self.named_ranges.iter()
    }

    /// Iterate sheet-scoped named ranges (for bindings/testing)
    pub fn sheet_named_ranges_iter(
        &self,
    ) -> impl Iterator<Item = (&(SheetId, String), &NamedRange)> {
        self.sheet_named_ranges.iter()
    }

    /// Resolve a name in an explicit [`NameScope`].
    ///
    /// [`NameScope::Sheet`] looks in that sheet's names first and falls back to
    /// workbook scope, matching Excel's shadowing rules. [`NameScope::Workbook`]
    /// looks in workbook-scoped names **only**: a sheet-scoped name is invisible
    /// to a workbook-scope query even when it is scoped to the default sheet.
    ///
    /// This is the single owned derivation for name scoping. A caller with no
    /// sheet context asks for [`NameScope::Workbook`], never for the default
    /// sheet's scope - substituting the default sheet for missing context is
    /// what leaked references onto unrelated sheets in issue #110.
    pub fn resolve_name_entry_in_scope(&self, name: &str, scope: NameScope) -> Option<&NamedRange> {
        if let Some((sheet_id, local)) = self.sheet_qualified_name(name) {
            return self.resolve_name_entry_in_scope(local, NameScope::Sheet(sheet_id));
        }
        let workbook_entry = || {
            if self.config.case_sensitive_names {
                self.named_ranges.get(name)
            } else {
                self.named_ranges_lookup
                    .get(&self.name_lookup_key(name))
                    .and_then(|canon| self.named_ranges.get(canon))
            }
        };

        match scope {
            NameScope::Workbook => workbook_entry(),
            NameScope::Sheet(current_sheet) => {
                if self.config.case_sensitive_names {
                    self.sheet_named_ranges
                        .get(&(current_sheet, name.to_string()))
                        .or_else(workbook_entry)
                } else {
                    let key = self.name_lookup_key(name);
                    self.sheet_named_ranges_lookup
                        .get(&(current_sheet, key))
                        .and_then(|canon| {
                            self.sheet_named_ranges.get(&(current_sheet, canon.clone()))
                        })
                        .or_else(workbook_entry)
                }
            }
        }
    }

    /// `Sheet1!Name` / `'My Sheet'!Name` names the name as seen from that
    /// sheet (its sheet-level name, else the workbook-level one): the sheet
    /// and the name, when the sheet exists.
    fn sheet_qualified_name<'n>(&self, name: &'n str) -> Option<(SheetId, &'n str)> {
        let (sheet, local) = name.rsplit_once('!')?;
        if local.is_empty() || sheet.is_empty() || sheet.starts_with('[') {
            return None;
        }
        let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
            Some(quoted) => quoted.replace("''", "'"),
            None => sheet.to_string(),
        };
        Some((self.sheet_id(&sheet)?, local))
    }

    /// Resolve a name as seen from `current_sheet`: sheet scope shadows
    /// workbook scope. Equivalent to [`Self::resolve_name_entry_in_scope`] with
    /// [`NameScope::Sheet`].
    pub fn resolve_name_entry(&self, name: &str, current_sheet: SheetId) -> Option<&NamedRange> {
        self.resolve_name_entry_in_scope(name, NameScope::Sheet(current_sheet))
    }

    /// Resolve a named range to its definition
    pub fn resolve_name(&self, name: &str, current_sheet: SheetId) -> Option<&NamedDefinition> {
        self.resolve_name_entry(name, current_sheet)
            .map(|nr| &nr.definition)
    }

    /// The folded lookup key (see [`Self::name_lookup_key`]) of the name
    /// represented by `vertex`, if it is a name vertex. Used by SCC tasks for
    /// deterministic member ordering and live name-read matching (RFC #112).
    pub(crate) fn name_key_for_vertex(&self, vertex: VertexId) -> Option<String> {
        self.name_vertex_lookup
            .get(&vertex)
            .map(|(_, name)| self.name_lookup_key(name))
    }

    pub fn named_range_by_vertex(&self, vertex: VertexId) -> Option<&NamedRange> {
        self.name_vertex_lookup
            .get(&vertex)
            .and_then(|(scope, name)| match scope {
                NameScope::Workbook => self.named_ranges.get(name),
                NameScope::Sheet(sheet_id) => {
                    self.sheet_named_ranges.get(&(*sheet_id, name.clone()))
                }
            })
    }

    /// Update an existing named range definition
    pub fn update_name(
        &mut self,
        name: &str,
        new_definition: NamedDefinition,
        scope: NameScope,
    ) -> Result<(), ExcelError> {
        let Some(canon_name) = self.canonical_name_in_scope(scope, name) else {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Name not found: {name}")));
        };

        // First collect dependents to avoid borrow checker issues
        let dependents_to_dirty = match scope {
            NameScope::Workbook => self
                .named_ranges
                .get(&canon_name)
                .map(|nr| nr.dependents.iter().copied().collect::<Vec<_>>()),
            NameScope::Sheet(id) => self
                .sheet_named_ranges
                .get(&(id, canon_name.clone()))
                .map(|nr| nr.dependents.iter().copied().collect::<Vec<_>>()),
        };

        if let Some(dependents) = dependents_to_dirty {
            // Dirty every dependent WITH propagation (#365). A dependent may
            // itself be a formula-backed name vertex; everything downstream of
            // it has to recompute against the new binding. A non-propagating
            // mark stops after one hop and leaves cells that read the
            // dependent name serving stale values.
            self.mark_dirty_many(&dependents);

            // Now update the definition
            let named_range = match scope {
                NameScope::Workbook => self.named_ranges.get_mut(&canon_name),
                NameScope::Sheet(id) => self.sheet_named_ranges.get_mut(&(id, canon_name.clone())),
            };

            let mut update_data: Option<(VertexId, NameScope, NamedDefinition, bool)> = None;
            if let Some(named_range) = named_range {
                named_range.definition = new_definition;
                let is_range = matches!(named_range.definition, NamedDefinition::Range(_));
                update_data = Some((
                    named_range.vertex,
                    named_range.scope,
                    named_range.definition.clone(),
                    is_range,
                ));
            }

            if let Some((vertex, scope_value, definition_snapshot, is_range)) = update_data {
                self.detach_vertex_from_names(vertex);

                if is_range {
                    self.store.set_kind(vertex, VertexKind::NamedArray);
                } else {
                    self.store.set_kind(vertex, VertexKind::NamedScalar);
                }

                let referenced_names =
                    self.rebuild_name_dependencies(vertex, &definition_snapshot, scope_value)?;
                if !referenced_names.is_empty() {
                    self.attach_vertex_to_names(vertex, &referenced_names);
                }
                self.mark_name_users_dynamic(vertex);
                self.rebuild_area_numbering_name_consumers(&dependents);
                // Propagate from the rebound name vertex itself, after its
                // edges are current, so the transitive closure is reached.
                self.mark_dirty_many(&[vertex]);
            }

            self.bump_symbol_revision();
            Ok(())
        } else {
            Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Name not found: {name}")))
        }
    }

    /// Delete a named range
    pub fn delete_name(&mut self, name: &str, scope: NameScope) -> Result<(), ExcelError> {
        let Some(canon_name) = self.canonical_name_in_scope(scope, name) else {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Name not found: {name}")));
        };

        let named_range = match scope {
            NameScope::Workbook => {
                let removed = self.named_ranges.remove(&canon_name);
                let key = self.name_lookup_key(&canon_name);
                self.named_ranges_lookup.remove(&key);
                removed
            }
            NameScope::Sheet(id) => {
                let removed = self.sheet_named_ranges.remove(&(id, canon_name.clone()));
                let key = self.name_lookup_key(&canon_name);
                self.sheet_named_ranges_lookup.remove(&(id, key));
                removed
            }
        };

        if let Some(named_range) = named_range {
            let mut affected: FxHashSet<VertexId> = FxHashSet::default();
            for &vertex_id in &named_range.dependents {
                affected.insert(vertex_id);
            }
            for (vertex_id, names) in self.vertex_to_names.iter() {
                if names.contains(&named_range.vertex) {
                    affected.insert(*vertex_id);
                }
            }
            let formulas_to_rebuild = affected
                .iter()
                .filter(|&&vertex_id| self.get_cell_ref_for_vertex(vertex_id).is_some())
                .filter_map(|&vertex_id| self.get_formula(vertex_id).map(|ast| (vertex_id, ast)))
                .collect::<Vec<_>>();
            // Symbol (name) vertices are excluded from `formulas_to_rebuild`
            // by the cell-ref filter above, but a formula-backed name that
            // referenced the deleted name needs the same treatment (#365):
            // its dependency edges must be re-extracted so it re-resolves to
            // #NAME? now and can be healed by a later define.
            let names_to_rebuild = affected
                .iter()
                .filter(|&&vertex_id| vertex_id != named_range.vertex)
                .filter_map(|&vertex_id| {
                    self.named_range_by_vertex(vertex_id)
                        .map(|nr| (vertex_id, nr.definition.clone(), nr.scope))
                })
                .collect::<Vec<_>>();
            let dirty_sources = affected
                .iter()
                .copied()
                .filter(|&vertex_id| vertex_id != named_range.vertex)
                .collect::<Vec<_>>();
            for vertex_id in affected {
                if let Some(names) = self.vertex_to_names.get_mut(&vertex_id) {
                    names.retain(|vid| *vid != named_range.vertex);
                    if names.is_empty() {
                        self.vertex_to_names.remove(&vertex_id);
                    }
                }
            }
            self.mark_named_vertex_deleted(&named_range);
            // Re-extract cell-formula dependencies after the registry entry is gone. This
            // preserves fallback-to-workbook resolution for a deleted sheet name and records
            // an unresolved pending-name link otherwise, allowing a later define to heal the
            // formula without requiring re-ingest.
            for (vertex_id, ast) in formulas_to_rebuild {
                self.rewire_formula_dependencies(vertex_id, &ast);
            }
            let dependent_names = names_to_rebuild
                .iter()
                .map(|(vertex_id, _, _)| *vertex_id)
                .collect::<Vec<_>>();
            for (vertex_id, definition, name_scope) in names_to_rebuild {
                let referenced_names =
                    self.rebuild_name_dependencies(vertex_id, &definition, name_scope)?;
                if !referenced_names.is_empty() {
                    self.attach_vertex_to_names(vertex_id, &referenced_names);
                }
            }
            // The formulas above are rebuilt already; those using the deleted
            // name through another name are not.
            self.rebuild_area_numbering_name_consumers(&dependent_names);
            // Dirty the affected set WITH propagation, once every dependency
            // edge above is current, so cells reading a formula-backed
            // dependent name recompute instead of serving a cached value.
            self.mark_dirty_many(&dirty_sources);
            self.bump_symbol_revision();
            Ok(())
        } else {
            Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Name not found: {name}")))
        }
    }

    /// Re-wires the dependencies of the formulas that use a name whose
    /// definition changed, given its dependents, directly or through other
    /// names, when the formula's self-reference was decided from the areas
    /// its INDEX selects: a compressed range of the formula covers its own
    /// cell. INDEX numbers the areas of a union such as `(Areas,A:A)` through
    /// every area of the names in it, so a new definition can make A:A
    /// another area: `=INDEX((Areas,A:A),1,1,2)` in A1 selects A1 itself
    /// once Areas is one area, and stops selecting it when Areas grows back
    /// to two (or is deleted, when the union is `#NAME?`). Re-wiring decides
    /// the self-reference afresh, as entering the formula again would.
    pub(super) fn rebuild_area_numbering_name_consumers(&mut self, dependents: &[VertexId]) {
        let mut stack = dependents.to_vec();
        let mut seen = FxHashSet::default();
        let mut formulas = Vec::new();
        while let Some(vertex) = stack.pop() {
            if !seen.insert(vertex) {
                continue;
            }
            if let Some(named) = self.named_range_by_vertex(vertex) {
                stack.extend(named.dependents.iter().copied());
            } else if self.compressed_range_covers_self(vertex) {
                formulas.push(vertex);
            }
        }
        formulas.sort_unstable_by_key(|vertex| vertex.0);
        for vertex in formulas {
            if let Some(ast) = self.get_formula(vertex) {
                self.rewire_formula_dependencies(vertex, &ast);
            }
        }
    }

    pub(super) fn detach_vertex_from_names(&mut self, vertex: VertexId) {
        if let Some(prior) = self.vertex_to_names.remove(&vertex) {
            for name_vertex in prior {
                if let Some((scope, name)) = self.name_vertex_lookup.get(&name_vertex).cloned() {
                    match scope {
                        NameScope::Workbook => {
                            if let Some(entry) = self.named_ranges.get_mut(&name) {
                                entry.dependents.remove(&vertex);
                            }
                        }
                        NameScope::Sheet(sheet_id) => {
                            if let Some(entry) =
                                self.sheet_named_ranges.get_mut(&(sheet_id, name.clone()))
                            {
                                entry.dependents.remove(&vertex);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn attach_vertex_to_names(&mut self, vertex: VertexId, names: &[VertexId]) {
        if names.is_empty() {
            return;
        }
        let mut unique = FxHashSet::default();
        let mut recorded = Vec::new();
        for &name_vertex in names {
            if !unique.insert(name_vertex) {
                continue;
            }
            if let Some((scope, name)) = self.name_vertex_lookup.get(&name_vertex).cloned() {
                match scope {
                    NameScope::Workbook => {
                        if let Some(entry) = self.named_ranges.get_mut(&name) {
                            entry.dependents.insert(vertex);
                        }
                    }
                    NameScope::Sheet(sheet_id) => {
                        if let Some(entry) =
                            self.sheet_named_ranges.get_mut(&(sheet_id, name.clone()))
                        {
                            entry.dependents.insert(vertex);
                        }
                    }
                }
                recorded.push(name_vertex);
            }
        }
        if self.is_formula_vertex(vertex)
            && recorded
                .iter()
                .any(|&name_vertex| self.name_reads_dynamically(name_vertex))
        {
            self.store.set_dynamic(vertex, true);
        }
        if !recorded.is_empty() {
            self.vertex_to_names.insert(vertex, recorded);
        }
    }

    fn is_formula_vertex(&self, vertex: VertexId) -> bool {
        matches!(
            self.store.kind(vertex),
            VertexKind::FormulaScalar | VertexKind::FormulaArray
        )
    }

    /// Whether the formula of the name at `name_vertex` reads references it
    /// finds while it calculates (INDIRECT, OFFSET), itself or through a name
    /// it uses. A name evaluates for the formula that uses it, so that
    /// formula reads them, from its own cell (`Loop = INDIRECT("RC",FALSE)+1`
    /// used in B2 reads B2): it is dynamic like the formula written in the
    /// name's place, and what it reads is found the same way.
    pub(crate) fn name_reads_dynamically(&self, name_vertex: VertexId) -> bool {
        let mut seen = FxHashSet::default();
        let mut pending = vec![name_vertex];
        while let Some(vertex) = pending.pop() {
            if !seen.insert(vertex) {
                continue;
            }
            let Some(named) = self.named_range_by_vertex(vertex) else {
                continue;
            };
            if let NamedDefinition::Formula { ast, .. } = &named.definition
                && self.is_ast_dynamic(ast)
            {
                return true;
            }
            pending.extend(
                self.edges
                    .out_edges(vertex)
                    .iter()
                    .copied()
                    .filter(|&dependency| {
                        matches!(
                            self.store.kind(dependency),
                            VertexKind::NamedScalar | VertexKind::NamedArray
                        )
                    }),
            );
        }
        false
    }

    /// Mark the formulas using the name at `name_vertex`, directly or through
    /// other names, dynamic when the name reads dynamically (see
    /// [`Self::name_reads_dynamically`]); a formula entered later is marked
    /// as it attaches to the name.
    fn mark_name_users_dynamic(&mut self, name_vertex: VertexId) {
        if !self.name_reads_dynamically(name_vertex) {
            return;
        }
        let mut seen = FxHashSet::default();
        let mut pending = vec![name_vertex];
        while let Some(vertex) = pending.pop() {
            if !seen.insert(vertex) {
                continue;
            }
            let Some(named) = self.named_range_by_vertex(vertex) else {
                continue;
            };
            let users: Vec<VertexId> = named.dependents.iter().copied().collect();
            for user in users {
                if self.is_formula_vertex(user) {
                    self.store.set_dynamic(user, true);
                } else {
                    pending.push(user);
                }
            }
        }
    }

    pub(super) fn unregister_name_cell_dependencies(&mut self, name_vertex: VertexId) {
        if let Some(prev) = self.name_to_cell_dependencies.remove(&name_vertex) {
            for dep in prev {
                if let Some(set) = self.cell_to_name_dependents.get_mut(&dep) {
                    set.remove(&name_vertex);
                    if set.is_empty() {
                        self.cell_to_name_dependents.remove(&dep);
                    }
                }
            }
        }
    }

    pub(super) fn register_name_cell_dependencies(
        &mut self,
        name_vertex: VertexId,
        dependencies: &[VertexId],
    ) {
        self.unregister_name_cell_dependencies(name_vertex);
        if dependencies.is_empty() {
            return;
        }
        for dep in dependencies {
            self.cell_to_name_dependents
                .entry(*dep)
                .or_default()
                .insert(name_vertex);
        }
        self.name_to_cell_dependencies
            .insert(name_vertex, dependencies.to_vec());
    }

    pub(crate) fn record_pending_name_reference(
        &mut self,
        sheet_id: SheetId,
        name: &str,
        formula_vertex: VertexId,
    ) {
        // `Sheet2!Areas` waits for Areas as seen from Sheet2.
        let (sheet_id, name) = self.sheet_qualified_name(name).unwrap_or((sheet_id, name));
        let key = self.name_lookup_key(name);
        self.pending_name_links
            .entry(key.clone())
            .or_default()
            .insert((sheet_id, formula_vertex));
        self.vertex_to_pending_names
            .entry(formula_vertex)
            .or_default()
            .insert(key);
    }

    pub(crate) fn clear_pending_name_references(&mut self, formula_vertex: VertexId) {
        let Some(keys) = self.vertex_to_pending_names.remove(&formula_vertex) else {
            return;
        };

        for key in keys {
            let mut remove_key = false;
            if let Some(entries) = self.pending_name_links.get_mut(&key) {
                entries.retain(|(_, vertex_id)| *vertex_id != formula_vertex);
                remove_key = entries.is_empty();
            }
            if remove_key {
                self.pending_name_links.remove(&key);
            }
        }
    }

    pub(super) fn resolve_pending_name_references(&mut self, scope: NameScope, name: &str) {
        let key = self.name_lookup_key(name);
        if let Some(entries) = self.pending_name_links.remove(&key) {
            for (sheet_id, formula_vertex) in entries {
                let attach = match scope {
                    NameScope::Workbook => true,
                    NameScope::Sheet(expected) => expected == sheet_id,
                };
                if attach {
                    if let Some(named) = self.named_range_by_vertex(formula_vertex) {
                        // A name defined through the new name (its definition
                        // was rebuilt while the name was undefined, after a
                        // delete or an update) now reads it.
                        let (definition, name_scope) = (named.definition.clone(), named.scope);
                        self.heal_name_dependencies(formula_vertex, &definition, name_scope);
                    } else if let Some(ast) = self.get_formula(formula_vertex) {
                        // A cycle through the new name stays a cycle the
                        // evaluator reports, as for a redefined name.
                        self.rewire_formula_dependencies(formula_vertex, &ast);
                    } else {
                        self.clear_pending_name_references(formula_vertex);
                    }
                } else {
                    self.record_pending_name_reference(sheet_id, name, formula_vertex);
                }
            }
        }
    }

    /// Rebuilds the dependencies of a formula name that reads a name just
    /// defined, and re-wires the formulas using it whose INDEX numbers its
    /// areas (see [`Self::rebuild_area_numbering_name_consumers`]); they
    /// recalculate.
    fn heal_name_dependencies(
        &mut self,
        vertex: VertexId,
        definition: &NamedDefinition,
        scope: NameScope,
    ) {
        self.detach_vertex_from_names(vertex);
        if let Ok(referenced_names) = self.rebuild_name_dependencies(vertex, definition, scope)
            && !referenced_names.is_empty()
        {
            self.attach_vertex_to_names(vertex, &referenced_names);
        }
        self.mark_name_users_dynamic(vertex);
        let dependents = self
            .named_range_by_vertex(vertex)
            .map(|named| named.dependents.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        self.rebuild_area_numbering_name_consumers(&dependents);
        self.mark_dirty_many(&[vertex]);
    }

    pub(super) fn name_depends_on_vertex(
        &self,
        name_vertex: VertexId,
        target: VertexId,
        visited: &mut FxHashSet<VertexId>,
    ) -> bool {
        if !visited.insert(name_vertex) {
            return false;
        }

        for dependency in self.edges.out_edges(name_vertex).iter().copied() {
            if dependency == target {
                return true;
            }

            if matches!(
                self.store.kind(dependency),
                VertexKind::NamedScalar | VertexKind::NamedArray
            ) && self.name_depends_on_vertex(dependency, target, visited)
            {
                return true;
            }
        }

        false
    }

    pub(super) fn rebuild_name_dependencies(
        &mut self,
        vertex: VertexId,
        definition: &NamedDefinition,
        scope: NameScope,
    ) -> Result<Vec<VertexId>, ExcelError> {
        let current_sheet_id = match scope {
            NameScope::Sheet(id) => id,
            NameScope::Workbook => self.default_sheet_id,
        };
        let mut pending_names = Vec::new();
        let formula_dependencies = if let NamedDefinition::Formula { ast, .. } = definition {
            let (dependencies, range_dependencies, _, _, pending) =
                self.extract_dependencies_with_pending_names(ast, current_sheet_id)?;
            pending_names = pending;
            Some((dependencies, range_dependencies))
        } else {
            None
        };

        self.remove_dependent_edges(vertex);
        self.unregister_name_cell_dependencies(vertex);
        // A name the definition reads that is not defined (deleted, or not
        // defined yet) is pending: defining it rebuilds this name's
        // dependencies, as it does a formula's.
        self.clear_pending_name_references(vertex);
        for name in &pending_names {
            self.record_pending_name_reference(current_sheet_id, name, vertex);
        }

        let mut dependencies: Vec<VertexId> = Vec::new();
        let mut range_dependencies: Vec<SharedRangeRef<'static>> = Vec::new();
        let mut placeholders = Vec::new();

        match definition {
            NamedDefinition::Cell(cell_ref) => {
                let vertex_id = self.get_or_create_vertex(cell_ref, &mut placeholders);
                dependencies.push(vertex_id);
            }
            NamedDefinition::Range(range_ref) => {
                let height = range_ref
                    .end
                    .coord
                    .row()
                    .saturating_sub(range_ref.start.coord.row())
                    + 1;
                let width = range_ref
                    .end
                    .coord
                    .col()
                    .saturating_sub(range_ref.start.coord.col())
                    + 1;
                let size = (width * height) as usize;

                if size <= self.config.range_expansion_limit {
                    for row in range_ref.start.coord.row()..=range_ref.end.coord.row() {
                        for col in range_ref.start.coord.col()..=range_ref.end.coord.col() {
                            let coord = Coord::new(row, col, true, true);
                            let addr = CellRef::new(range_ref.start.sheet_id, coord);
                            let vertex_id = self.get_or_create_vertex(&addr, &mut placeholders);
                            dependencies.push(vertex_id);
                        }
                    }
                } else {
                    let sheet_loc = SharedSheetLocator::Id(range_ref.start.sheet_id);
                    let sr = formualizer_common::AxisBound::new(
                        range_ref.start.coord.row(),
                        range_ref.start.coord.row_abs(),
                    );
                    let sc = formualizer_common::AxisBound::new(
                        range_ref.start.coord.col(),
                        range_ref.start.coord.col_abs(),
                    );
                    let er = formualizer_common::AxisBound::new(
                        range_ref.end.coord.row(),
                        range_ref.end.coord.row_abs(),
                    );
                    let ec = formualizer_common::AxisBound::new(
                        range_ref.end.coord.col(),
                        range_ref.end.coord.col_abs(),
                    );
                    if let Ok(r) = SharedRangeRef::from_parts(
                        sheet_loc,
                        Some(sr),
                        Some(sc),
                        Some(er),
                        Some(ec),
                    ) {
                        range_dependencies.push(r.into_owned());
                    }
                }
            }
            NamedDefinition::Literal(_) => {
                // No dependencies.
            }
            NamedDefinition::Formula { .. } => {
                let Some((formula_deps, range_deps)) = formula_dependencies else {
                    return Err(ExcelError::new(ExcelErrorKind::Error)
                        .with_message("Internal error: formula dependencies were not extracted"));
                };
                dependencies.extend(formula_deps);
                range_dependencies.extend(range_deps);
            }
        }

        if !dependencies.is_empty() {
            self.add_dependent_edges(vertex, &dependencies);
        }
        self.register_name_cell_dependencies(vertex, &dependencies);

        if !range_dependencies.is_empty() {
            let sheet_id = match scope {
                NameScope::Sheet(id) => id,
                NameScope::Workbook => self.default_sheet_id,
            };
            self.add_range_dependent_edges(vertex, &range_dependencies, sheet_id);
        }

        // A name whose formula holds a volatile function (OFFSET, INDIRECT,
        // NOW, ...) is volatile, and so is every formula that uses it: what it
        // reads is known only when it is evaluated (OFFSET's target is no
        // dependency of the name), so it and its dependents recalculate every
        // time, as a cell holding the formula would.
        let volatile = matches!(
            definition,
            NamedDefinition::Formula { ast, .. } if self.is_ast_volatile(ast)
        );
        self.mark_volatile(vertex, volatile);

        Ok(dependencies
            .iter()
            .filter(|vid| {
                matches!(
                    self.store.kind(**vid),
                    VertexKind::NamedScalar | VertexKind::NamedArray
                )
            })
            .copied()
            .collect())
    }

    pub fn adjust_named_ranges(
        &mut self,
        operation: &crate::engine::graph::editor::reference_adjuster::ShiftOperation,
    ) -> Result<(), ExcelError> {
        let adjuster = crate::engine::graph::editor::reference_adjuster::ReferenceAdjuster::new();

        let changed = !self.named_ranges.is_empty() || !self.sheet_named_ranges.is_empty();
        // Workbook-scoped formulas bind unqualified references to the default sheet.
        let workbook_context =
            crate::engine::graph::editor::reference_adjuster::ReferenceContext::new(
                self.default_sheet_id,
                &self.sheet_reg,
            );
        // Adjust cloned definitions first so a future fallible definition kind
        // cannot leave the name table half-adjusted.
        let mut adjusted_named_ranges = self.named_ranges.clone();
        let mut adjusted_sheet_named_ranges = self.sheet_named_ranges.clone();
        for named_range in adjusted_named_ranges.values_mut() {
            adjust_named_definition(
                &mut named_range.definition,
                &adjuster,
                operation,
                &workbook_context,
            )?;
        }

        // Sheet-scoped formulas bind unqualified references to their scope sheet.
        for ((scope_sheet_id, _), named_range) in adjusted_sheet_named_ranges.iter_mut() {
            let context = crate::engine::graph::editor::reference_adjuster::ReferenceContext::new(
                *scope_sheet_id,
                &self.sheet_reg,
            );
            adjust_named_definition(&mut named_range.definition, &adjuster, operation, &context)?;
        }
        let changed_names: Vec<_> = adjusted_named_ranges
            .iter()
            .filter_map(|(key, adjusted)| {
                self.named_ranges
                    .get(key)
                    .is_some_and(|current| current.definition != adjusted.definition)
                    .then_some((adjusted.vertex, adjusted.scope, adjusted.definition.clone()))
            })
            .chain(
                adjusted_sheet_named_ranges
                    .iter()
                    .filter_map(|(key, adjusted)| {
                        self.sheet_named_ranges
                            .get(key)
                            .is_some_and(|current| current.definition != adjusted.definition)
                            .then_some((
                                adjusted.vertex,
                                adjusted.scope,
                                adjusted.definition.clone(),
                            ))
                    }),
            )
            .collect();
        self.named_ranges = adjusted_named_ranges;
        self.sheet_named_ranges = adjusted_sheet_named_ranges;
        for &(vertex, scope, ref definition) in &changed_names {
            self.detach_vertex_from_names(vertex);
            self.store.set_kind(
                vertex,
                if matches!(definition, NamedDefinition::Range(_)) {
                    VertexKind::NamedArray
                } else {
                    VertexKind::NamedScalar
                },
            );
            let referenced_names = self.rebuild_name_dependencies(vertex, definition, scope)?;
            if !referenced_names.is_empty() {
                self.attach_vertex_to_names(vertex, &referenced_names);
            }
        }
        self.mark_dirty_many(
            &changed_names
                .iter()
                .map(|(vertex, _, _)| *vertex)
                .collect::<Vec<_>>(),
        );
        if changed {
            self.bump_symbol_revision();
        }

        Ok(())
    }

    /// Mark a vertex as having a #NAME! error
    pub fn mark_as_name_error(&mut self, vertex_id: VertexId) {
        // Mark the vertex as dirty
        self.mark_vertex_dirty(vertex_id);
    }

    pub(super) fn mark_named_vertex_deleted(&mut self, named_range: &NamedRange) {
        self.clear_pending_name_references(named_range.vertex);
        self.detach_vertex_from_names(named_range.vertex);
        self.remove_dependent_edges(named_range.vertex);
        self.unregister_name_cell_dependencies(named_range.vertex);
        self.store.mark_deleted(named_range.vertex, true);
        self.vertex_values.remove(&named_range.vertex);
        self.vertex_formulas.remove(&named_range.vertex);
        self.clear_formula_vertex_dirty(named_range.vertex);
        self.volatile_vertices.remove(&named_range.vertex);
        self.vertex_to_names.remove(&named_range.vertex);
        self.name_vertex_lookup.remove(&named_range.vertex);
    }
}
