use super::*;
use crate::engine::used_extent::{ExtentPolicy, OpenRangeBounds, resolve_used_extent};
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};

/// How many areas a reference INDEX selects in has, as far as the workbook
/// fixes it before the formula runs.
#[derive(Clone, Copy)]
enum AreaCount {
    /// A cell, a range, a union of them, or a name defined as one.
    Count(i64),
    /// `#NAME?`: a name defined nowhere, or a union or name holding one.
    Undefined,
    /// Known only when the formula runs: a function, an intersection, ...
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RangeSelfUse {
    NoMatch,
    Excluded,
    IncludedOrUnknown,
}

impl RangeSelfUse {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::IncludedOrUnknown, _) | (_, Self::IncludedOrUnknown) => Self::IncludedOrUnknown,
            (Self::Excluded, _) | (_, Self::Excluded) => Self::Excluded,
            _ => Self::NoMatch,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum StructuralEdit {
    InsertRows { before: u32 },
    DeleteRows { start: u32, end: u32 },
    InsertColumns { before: u32 },
    DeleteColumns { start: u32, end: u32 },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StructuralOccupancy {
    occupied_rows: Vec<u32>,
    occupied_columns: Vec<u32>,
    conservative: bool,
}

impl StructuralOccupancy {
    pub(crate) fn conservative() -> Self {
        Self {
            conservative: true,
            ..Self::default()
        }
    }

    fn finish(&mut self) {
        self.occupied_rows.sort_unstable();
        self.occupied_rows.dedup();
        self.occupied_columns.sort_unstable();
        self.occupied_columns.dedup();
    }

    pub(crate) fn include_arrow_sheet(&mut self, sheet: &crate::arrow_store::ArrowSheet) {
        let shapes = sheet.shape();
        for (col, column) in sheet.columns.iter().enumerate() {
            let shape_occupied = shapes.get(col).is_some_and(|shape| {
                shape.has_num || shape.has_bool || shape.has_text || shape.has_err
            });
            let sparse_meta_occupied = column.sparse_chunks.values().any(|chunk| {
                chunk.meta.non_null_num > 0
                    || chunk.meta.non_null_bool > 0
                    || chunk.meta.non_null_text > 0
                    || chunk.meta.non_null_err > 0
            });
            let overlay_occupied = column
                .chunks
                .iter()
                .chain(column.sparse_chunks.values())
                .any(|chunk| {
                    chunk.overlay.iter().next().is_some()
                        || chunk.computed_overlay.iter().next().is_some()
                });
            if shape_occupied || sparse_meta_occupied || overlay_occupied {
                self.occupied_columns.push(col as u32);
            }
        }
        self.finish();
    }

    fn intersects(sorted: &[u32], start: u32, end: u32) -> bool {
        let index = sorted.partition_point(|value| *value < start);
        sorted.get(index).is_some_and(|value| *value <= end)
    }

    fn cross_axis_occupied(self_ref: &Self, edit: StructuralEdit, start: u32, end: u32) -> bool {
        if self_ref.conservative {
            return true;
        }
        match edit {
            StructuralEdit::InsertRows { .. } | StructuralEdit::DeleteRows { .. } => {
                Self::intersects(&self_ref.occupied_columns, start, end)
            }
            StructuralEdit::InsertColumns { .. } | StructuralEdit::DeleteColumns { .. } => {
                Self::intersects(&self_ref.occupied_rows, start, end)
            }
        }
    }
}

impl DependencyGraph {
    pub(crate) fn has_compressed_range_dependencies(&self) -> bool {
        !self.formula_to_range_deps.is_empty()
    }

    pub(crate) fn structural_occupancy(&self, sheet_id: SheetId) -> StructuralOccupancy {
        let mut occupancy = StructuralOccupancy::default();
        for (id, coord) in self.grid_vertices_in_sheet(sheet_id) {
            if self.store.kind(id) != VertexKind::Empty {
                occupancy.occupied_rows.push(coord.row());
                occupancy.occupied_columns.push(coord.col());
            }
        }
        occupancy.finish();
        occupancy
    }

    pub(crate) fn compressed_range_dependents_for_structural_edit(
        &self,
        sheet_id: SheetId,
        edit: StructuralEdit,
        occupancy: &StructuralOccupancy,
    ) -> Vec<VertexId> {
        self.formula_to_range_deps
            .iter()
            .filter_map(|(&dependent, ranges)| {
                ranges
                    .iter()
                    .any(|range| {
                        // `Current` is the dependent formula's own sheet. An
                        // unresolvable sheet keeps the candidate conservative.
                        let range_sheet_id = self
                            .sheet_reg
                            .resolve_locator(&range.sheet, self.get_vertex_sheet_id(dependent))
                            .ok();
                        if range_sheet_id.is_some_and(|resolved| resolved != sheet_id) {
                            return false;
                        }
                        let start_row = range.start_row.map(|bound| bound.index).unwrap_or(0);
                        let end_row = range.end_row.map(|bound| bound.index).unwrap_or(u32::MAX);
                        let start_col = range.start_col.map(|bound| bound.index).unwrap_or(0);
                        let end_col = range.end_col.map(|bound| bound.index).unwrap_or(u32::MAX);
                        let axis_matches = match edit {
                            StructuralEdit::DeleteRows { start, end } => {
                                start_row <= end && end_row >= start
                            }
                            StructuralEdit::InsertRows { before } => {
                                (range.start_row.is_none() || start_row < before)
                                    && before <= end_row
                            }
                            StructuralEdit::DeleteColumns { start, end } => {
                                start_col <= end && end_col >= start
                            }
                            StructuralEdit::InsertColumns { before } => {
                                (range.start_col.is_none() || start_col < before)
                                    && before <= end_col
                            }
                        };
                        let (cross_start, cross_end) = match edit {
                            StructuralEdit::InsertRows { .. }
                            | StructuralEdit::DeleteRows { .. } => (start_col, end_col),
                            StructuralEdit::InsertColumns { .. }
                            | StructuralEdit::DeleteColumns { .. } => (start_row, end_row),
                        };
                        axis_matches
                            && (range_sheet_id.is_none()
                                // An unresolvable sheet candidate must remain
                                // conservative; occupancy from the edited sheet
                                // cannot prove that candidate empty.
                                || StructuralOccupancy::cross_axis_occupied(
                                    occupancy,
                                    edit,
                                    cross_start,
                                    cross_end,
                                ))
                    })
                    .then_some(dependent)
            })
            .collect()
    }

    /// Visit compressed-range formula dependents covering one cell without
    /// materializing the stripe union used by dirty propagation.
    ///
    /// This path is intentionally parallel to
    /// `collect_range_dependents_for_rect`: scheduling keeps its existing
    /// behavior, while inspection can stop before a pathological stripe has
    /// been copied into an unbounded candidate set. Work is charged for every
    /// stripe candidate and every compressed range exact-check.
    pub(crate) fn visit_range_dependents_covering_bounded(
        &self,
        sheet_id: SheetId,
        row0: u32,
        col0: u32,
        remaining_work: &mut u64,
        visitor: &mut dyn FnMut(VertexId) -> bool,
    ) -> bool {
        if self.stripe_to_dependents.is_empty() {
            return true;
        }

        let mut seen = FxHashSet::default();
        let keys = [
            StripeKey {
                sheet_id,
                stripe_type: StripeType::Column,
                index: col0,
            },
            StripeKey {
                sheet_id,
                stripe_type: StripeType::Row,
                index: row0,
            },
            StripeKey {
                sheet_id,
                stripe_type: StripeType::Block,
                index: block_index(row0, col0),
            },
        ];

        for key in keys {
            if key.stripe_type == StripeType::Block && !self.config.enable_block_stripes {
                continue;
            }
            let Some(candidates) = self.stripe_to_dependents.get(&key) else {
                continue;
            };
            for &dependent in candidates {
                if *remaining_work == 0 {
                    return false;
                }
                *remaining_work -= 1;
                if !seen.insert(dependent) {
                    continue;
                }
                let Some(ranges) = self.formula_to_range_deps.get(&dependent) else {
                    continue;
                };
                let mut covered = false;
                for range in ranges {
                    if *remaining_work == 0 {
                        return false;
                    }
                    *remaining_work -= 1;
                    // `Current` is the dependent formula's own sheet; an
                    // unresolvable sheet name is interpreted on the query sheet
                    // so the dependent is not silently dropped.
                    let range_sheet = self
                        .sheet_reg
                        .resolve_locator(&range.sheet, self.get_vertex_sheet_id(dependent))
                        .unwrap_or(sheet_id);
                    if range_sheet != sheet_id {
                        continue;
                    }
                    let start_row = range.start_row.map(|bound| bound.index).unwrap_or(0);
                    let end_row = range.end_row.map(|bound| bound.index).unwrap_or(u32::MAX);
                    let start_col = range.start_col.map(|bound| bound.index).unwrap_or(0);
                    let end_col = range.end_col.map(|bound| bound.index).unwrap_or(u32::MAX);
                    if start_row <= row0 && row0 <= end_row && start_col <= col0 && col0 <= end_col
                    {
                        covered = true;
                        break;
                    }
                }
                if covered && !visitor(dependent) {
                    return false;
                }
            }
        }
        true
    }

    /// Public wrapper to add range-dependent edges.
    pub fn add_range_edges(
        &mut self,
        dependent: VertexId,
        ranges: &[SharedRangeRef<'static>],
        current_sheet_id: SheetId,
    ) {
        self.add_range_dependent_edges(dependent, ranges, current_sheet_id);
    }

    /// Return the compressed range dependencies recorded for a formula vertex, if any.
    /// These are `SharedRangeRef` entries that were not expanded into explicit
    /// cell edges due to `range_expansion_limit` or due to infinite/partial bounds.
    pub fn get_range_dependencies(
        &self,
        vertex: VertexId,
    ) -> Option<&Vec<SharedRangeRef<'static>>> {
        self.formula_to_range_deps.get(&vertex)
    }

    #[cfg(test)]
    pub(crate) fn formula_to_range_deps(
        &self,
    ) -> &FxHashMap<VertexId, Vec<SharedRangeRef<'static>>> {
        &self.formula_to_range_deps
    }

    #[cfg(test)]
    pub(crate) fn stripe_to_dependents(&self) -> &FxHashMap<StripeKey, FxHashSet<VertexId>> {
        &self.stripe_to_dependents
    }

    /// True when a (possibly open-ended) range region on `sheet_id` covers
    /// the formula vertex's own cell. Used to record a self-loop for
    /// stripe-compressed / whole-axis self-inclusion (#120): such references
    /// never produce explicit cell edges, so the ingest self-reference check
    /// (which scans expanded cell deps) misses them. `None` bounds mean the
    /// axis is unbounded (whole column/row), which always covers the cell.
    fn range_region_contains_self(
        &self,
        dependent: VertexId,
        sheet_id: SheetId,
        s_row: Option<u32>,
        e_row: Option<u32>,
        s_col: Option<u32>,
        e_col: Option<u32>,
    ) -> bool {
        if self.store.sheet_id(dependent) != sheet_id {
            return false;
        }
        // A symbol vertex has no position, so no range region can contain it.
        let Some(coord) = self.store.grid_addr(dependent) else {
            return false;
        };
        let r0 = coord.row();
        let c0 = coord.col();
        s_row.is_none_or(|s| r0 >= s)
            && e_row.is_none_or(|e| r0 <= e)
            && s_col.is_none_or(|s| c0 >= s)
            && e_col.is_none_or(|e| c0 <= e)
    }

    /// True when one of the formula's compressed range dependencies covers
    /// its own cell, so whether the formula reads itself was decided by
    /// [`Self::compressed_range_self_use`].
    pub(super) fn compressed_range_covers_self(&self, dependent: VertexId) -> bool {
        let Some(ranges) = self.formula_to_range_deps.get(&dependent) else {
            return false;
        };
        let current_sheet_id = self.store.sheet_id(dependent);
        ranges.iter().any(|range| {
            let sheet_id = self
                .sheet_reg
                .resolve_locator(&range.sheet, current_sheet_id)
                .unwrap_or(current_sheet_id);
            self.range_region_contains_self(
                dependent,
                sheet_id,
                range.start_row.map(|b| b.index),
                range.end_row.map(|b| b.index),
                range.start_col.map(|b| b.index),
                range.end_col.map(|b| b.index),
            )
        })
    }

    /// Record a self-loop edge (vertex → itself). The edge store and Tarjan
    /// both treat self-loops as cycles (`separate_cycles` via `has_self_loop`).
    fn record_self_loop(&mut self, vertex: VertexId) {
        if !self.has_self_loop(vertex) {
            self.edges.add_edge(vertex, vertex);
        }
    }

    pub(crate) fn compressed_range_resolved_bounds(
        &self,
        sheet: SheetId,
        range: (Option<u32>, Option<u32>, Option<u32>, Option<u32>),
    ) -> Option<(u32, u32, u32, u32)> {
        let (start_row, end_row, start_col, end_col) = range;
        let extent = resolve_used_extent(
            OpenRangeBounds {
                start_row,
                start_column: start_col,
                end_row,
                end_column: end_col,
            },
            ExtentPolicy::GraphCompat {
                fallback_row: self.config.max_open_ended_rows.saturating_sub(1),
                fallback_column: self.config.max_open_ended_cols.saturating_sub(1),
            },
            |first, last| self.used_row_bounds_for_columns(sheet, first, last),
            |first, last| self.used_col_bounds_for_rows(sheet, first, last),
        )?;
        Some((
            extent.start_row,
            extent.end_row,
            extent.start_column,
            extent.end_column,
        ))
    }

    /// Classify whether every occurrence of one compressed range that covers
    /// the formula cell is narrowed away from that cell by a statically
    /// resolvable `INDEX`. The range dependency itself remains conservative so
    /// used-bound growth still invalidates the formula; only the synthetic #120
    /// self-loop is omitted when the selected reference cannot contain the
    /// formula cell.
    fn compressed_range_self_use(
        &self,
        dependent: VertexId,
        range_sheet: SheetId,
        range: (Option<u32>, Option<u32>, Option<u32>, Option<u32>),
    ) -> RangeSelfUse {
        let Some(ast) = self.get_formula(dependent) else {
            return RangeSelfUse::IncludedOrUnknown;
        };

        // A constant selector reads as INDEX reads it at run time (numbers,
        // logicals and numeric text, truncated).
        use crate::engine::refs::static_index_tree as static_index;

        fn matching_range(
            graph: &DependencyGraph,
            node: &ASTNode,
            dependent: VertexId,
            range_sheet: SheetId,
            range: (Option<u32>, Option<u32>, Option<u32>, Option<u32>),
        ) -> bool {
            let ASTNodeType::Reference {
                reference:
                    ReferenceType::Range {
                        sheet,
                        start_row,
                        start_col,
                        end_row,
                        end_col,
                        ..
                    },
                ..
            } = &node.node_type
            else {
                return false;
            };
            let sheet_id = match sheet.as_deref() {
                Some(name) => match graph.sheet_id(name) {
                    Some(id) => id,
                    None => return false,
                },
                None => graph.get_vertex_sheet_id(dependent),
            };
            sheet_id == range_sheet
                && start_row.map(|index| index.saturating_sub(1)) == range.0
                && end_row.map(|index| index.saturating_sub(1)) == range.1
                && start_col.map(|index| index.saturating_sub(1)) == range.2
                && end_col.map(|index| index.saturating_sub(1)) == range.3
        }

        fn selected_region_contains_self(
            graph: &DependencyGraph,
            dependent: VertexId,
            range_sheet: SheetId,
            range: (Option<u32>, Option<u32>, Option<u32>, Option<u32>),
            position: i64,
            explicit_col: Option<i64>,
        ) -> Option<bool> {
            // Take the bounds INDEX itself resolves. A whole column or row spans the
            // full grid, so A:C is never a single row even when every used cell sits
            // in one row; only other open ranges clamp to the used region.
            let one_based = |index: Option<u32>| index.map(|index| index.saturating_add(1));
            let (sr, er, sc, ec) = match crate::builtins::reference_fns::index_static_bounds(
                one_based(range.0),
                one_based(range.2),
                one_based(range.1),
                one_based(range.3),
            ) {
                Some((sr, sc, er, ec)) => (
                    sr.saturating_sub(1),
                    er.saturating_sub(1),
                    sc.saturating_sub(1),
                    ec.saturating_sub(1),
                ),
                None => graph.compressed_range_resolved_bounds(range_sheet, range)?,
            };
            // Mirrors INDEX: an omitted column on a multi-row range selects the entire row.
            let (row, col) = match explicit_col {
                Some(col) => (position, col),
                None if sr == er => (1, position),
                None => (position, 0),
            };
            if row < 0 || col < 0 {
                return Some(false);
            }
            // A symbol vertex has no position, so no range region can contain it.
            let coord = graph.store.grid_addr(dependent)?;
            // The `n`th (1-based) row or column of `start..=end`; `None` past
            // its end, however large `n` is, where INDEX is #REF! and reads
            // nothing.
            let nth = |start: u32, end: u32, n: i64| {
                u32::try_from(n - 1)
                    .ok()
                    .and_then(|offset| start.checked_add(offset))
                    .filter(|&at| at <= end)
            };
            let contains = if row == 0 && col == 0 {
                coord.row() >= sr && coord.row() <= er && coord.col() >= sc && coord.col() <= ec
            } else if col == 0 {
                nth(sr, er, row).is_some_and(|selected_row| {
                    coord.row() == selected_row && coord.col() >= sc && coord.col() <= ec
                })
            } else if row == 0 {
                nth(sc, ec, col).is_some_and(|selected_col| {
                    coord.col() == selected_col && coord.row() >= sr && coord.row() <= er
                })
            } else {
                nth(sr, er, row).is_some_and(|selected_row| coord.row() == selected_row)
                    && nth(sc, ec, col).is_some_and(|selected_col| coord.col() == selected_col)
            };
            Some(contains)
        }

        /// The operands of INDEX's reference in the order written: those of
        /// a `,` union, or the reference itself.
        fn union_operands<'n>(node: &'n ASTNode, operands: &mut Vec<&'n ASTNode>) {
            match &node.node_type {
                ASTNodeType::BinaryOp { op, left, right } if op == "," => {
                    union_operands(left, operands);
                    union_operands(right, operands);
                }
                _ => operands.push(node),
            }
        }

        /// How many areas a reference operand has, when the workbook fixes
        /// it: a cell or a range is one area, a `,` union adds up its
        /// operands, and a name has the areas of its definition, read on the
        /// name's own sheet when it is a sheet-level name. A name defined
        /// nowhere (nor bound by the formula's LET or LAMBDA, `locals`) is
        /// `#NAME?`, and so is a union holding one. Anything else (a
        /// function, an intersection, a constant, ...) is known only when the
        /// formula runs.
        fn static_area_count(
            graph: &DependencyGraph,
            node: &ASTNode,
            sheet: SheetId,
            locals: &FxHashSet<String>,
            depth: u8,
        ) -> AreaCount {
            use crate::engine::named_range::{NameScope, NamedDefinition};
            if depth > 16 {
                return AreaCount::Unknown;
            }
            match &node.node_type {
                ASTNodeType::BinaryOp { op, left, right } if op == "," => {
                    match (
                        static_area_count(graph, left, sheet, locals, depth + 1),
                        static_area_count(graph, right, sheet, locals, depth + 1),
                    ) {
                        (AreaCount::Undefined, _) | (_, AreaCount::Undefined) => {
                            AreaCount::Undefined
                        }
                        (AreaCount::Count(left), AreaCount::Count(right)) => left
                            .checked_add(right)
                            .map_or(AreaCount::Unknown, AreaCount::Count),
                        _ => AreaCount::Unknown,
                    }
                }
                ASTNodeType::Reference {
                    reference: ReferenceType::Cell { .. } | ReferenceType::Range { .. },
                    ..
                } => AreaCount::Count(1),
                ASTNodeType::Reference {
                    reference: ReferenceType::NamedRange(name),
                    ..
                } => {
                    if locals.contains(&name.to_uppercase()) {
                        return AreaCount::Unknown;
                    }
                    let Some(named) = graph.resolve_name_entry(name, sheet) else {
                        // A table's bare name or an external source is no
                        // workbook name, but it is defined.
                        return if graph.resolve_table_entry(name).is_some()
                            || graph.resolve_source_scalar_entry(name).is_some()
                        {
                            AreaCount::Unknown
                        } else {
                            AreaCount::Undefined
                        };
                    };
                    match &named.definition {
                        NamedDefinition::Cell(_) | NamedDefinition::Range(_) => AreaCount::Count(1),
                        NamedDefinition::Formula { ast, .. } => {
                            let sheet = match named.scope {
                                NameScope::Sheet(id) => id,
                                NameScope::Workbook => sheet,
                            };
                            static_area_count(graph, ast, sheet, locals, depth + 1)
                        }
                        NamedDefinition::Literal(_) => AreaCount::Unknown,
                    }
                }
                _ => AreaCount::Unknown,
            }
        }

        /// The names the formula binds with LET or LAMBDA, upper-cased: a
        /// reference to one is a local value, not a workbook name.
        fn local_names(node: &ASTNode, locals: &mut FxHashSet<String>) {
            match &node.node_type {
                ASTNodeType::Function { name, args } => {
                    let function = name.strip_prefix("_xlfn.").unwrap_or(name);
                    let bound: Box<dyn Iterator<Item = &ASTNode>> =
                        if function.eq_ignore_ascii_case("LET") {
                            Box::new(args.iter().take(args.len().saturating_sub(1)).step_by(2))
                        } else if function.eq_ignore_ascii_case("LAMBDA") {
                            Box::new(args.iter().take(args.len().saturating_sub(1)))
                        } else {
                            Box::new(std::iter::empty())
                        };
                    for parameter in bound {
                        if let ASTNodeType::Reference {
                            reference: ReferenceType::NamedRange(local),
                            ..
                        } = &parameter.node_type
                        {
                            locals.insert(local.to_uppercase());
                        }
                    }
                    for arg in args {
                        local_names(arg, locals);
                    }
                }
                ASTNodeType::Call { callee, args } => {
                    local_names(callee, locals);
                    for arg in args {
                        local_names(arg, locals);
                    }
                }
                ASTNodeType::UnaryOp { expr, .. } => local_names(expr, locals),
                ASTNodeType::BinaryOp { left, right, .. } => {
                    local_names(left, locals);
                    local_names(right, locals);
                }
                ASTNodeType::Array(rows) => {
                    for item in rows.iter().flatten() {
                        local_names(item, locals);
                    }
                }
                ASTNodeType::Literal(_) | ASTNodeType::Omitted | ASTNodeType::Reference { .. } => {}
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn visit(
            graph: &DependencyGraph,
            node: &ASTNode,
            dependent: VertexId,
            range_sheet: SheetId,
            range: (Option<u32>, Option<u32>, Option<u32>, Option<u32>),
            index: Option<(i64, Option<i64>)>,
            locals: &FxHashSet<String>,
        ) -> RangeSelfUse {
            if matching_range(graph, node, dependent, range_sheet, range) {
                return match index.and_then(|(row, col)| {
                    selected_region_contains_self(graph, dependent, range_sheet, range, row, col)
                }) {
                    Some(false) => RangeSelfUse::Excluded,
                    Some(true) | None => RangeSelfUse::IncludedOrUnknown,
                };
            }
            match &node.node_type {
                ASTNodeType::Function { name, args }
                    if name.eq_ignore_ascii_case("INDEX") && (2..=4).contains(&args.len()) =>
                {
                    // area_num: absent or omitted is area 1. INDEX reads only
                    // the area it selects: a range is area 1, the one area it
                    // has, so INDEX(r,i,j,1) selects like INDEX(r,i,j), and a
                    // union (A:A,C:C) numbers its areas in the order written.
                    // An area INDEX does not select is never read; a static
                    // area past the last one (or below 1) is an error.
                    let area = match args.get(3) {
                        None => Some(1),
                        Some(node) if matches!(node.node_type, ASTNodeType::Omitted) => Some(1),
                        Some(node) => static_index(node),
                    };
                    // An omitted row_num or column_num is 0, as INDEX reads it
                    // at run time: INDEX(A:A,1,) is row 1 of A:A, A1, and
                    // INDEX(1:1,,4) column 4 of 1:1, D1.
                    let selector = |node: &ASTNode| match node.node_type {
                        ASTNodeType::Omitted => Some(0),
                        _ => static_index(node),
                    };
                    let row = selector(&args[1]);
                    let col = args.get(2).and_then(selector);
                    let selection = row.and_then(|row| {
                        if args.len() == 2 || col.is_some() {
                            Some((row, col))
                        } else {
                            None
                        }
                    });
                    // The areas are numbered as the formula runs: a name in the
                    // union contributes every area of its definition. Past an
                    // operand whose areas are not fixed, the numbers are unknown.
                    // A name defined nowhere makes the reference #NAME?, which
                    // INDEX returns without reading any area.
                    let mut operands = Vec::new();
                    union_operands(&args[0], &mut operands);
                    let sheet = graph.get_vertex_sheet_id(dependent);
                    let counts = operands
                        .iter()
                        .map(|node| static_area_count(graph, node, sheet, locals, 0))
                        .collect::<Vec<_>>();
                    let undefined = counts
                        .iter()
                        .any(|count| matches!(count, AreaCount::Undefined));
                    let mut first = Some(1i64);
                    let mut use_kind = RangeSelfUse::NoMatch;
                    for (node, count) in operands.into_iter().zip(counts) {
                        let numbers = first.and_then(|first| match count {
                            AreaCount::Count(count) => Some(first..first.checked_add(count)?),
                            AreaCount::Undefined | AreaCount::Unknown => None,
                        });
                        first = numbers.as_ref().map(|numbers| numbers.end);
                        let selected = !undefined
                            && match (area, &numbers) {
                                (Some(area), Some(numbers)) => numbers.contains(&area),
                                _ => true,
                            };
                        use_kind = use_kind.merge(if selected {
                            // Whichever area INDEX selects, row_num and
                            // column_num select within it.
                            visit(
                                graph,
                                node,
                                dependent,
                                range_sheet,
                                range,
                                selection,
                                locals,
                            )
                        } else if matching_range(graph, node, dependent, range_sheet, range) {
                            RangeSelfUse::Excluded
                        } else {
                            visit(graph, node, dependent, range_sheet, range, None, locals)
                        });
                    }
                    for arg in &args[1..] {
                        use_kind = use_kind.merge(visit(
                            graph,
                            arg,
                            dependent,
                            range_sheet,
                            range,
                            None,
                            locals,
                        ));
                    }
                    use_kind
                }
                ASTNodeType::Function { args, .. } => {
                    args.iter().fold(RangeSelfUse::NoMatch, |kind, arg| {
                        kind.merge(visit(
                            graph,
                            arg,
                            dependent,
                            range_sheet,
                            range,
                            None,
                            locals,
                        ))
                    })
                }
                ASTNodeType::UnaryOp { expr, .. } => {
                    visit(graph, expr, dependent, range_sheet, range, None, locals)
                }
                ASTNodeType::BinaryOp { left, right, .. } => {
                    visit(graph, left, dependent, range_sheet, range, None, locals).merge(visit(
                        graph,
                        right,
                        dependent,
                        range_sheet,
                        range,
                        None,
                        locals,
                    ))
                }
                ASTNodeType::Call { callee, args } => {
                    let mut kind =
                        visit(graph, callee, dependent, range_sheet, range, None, locals);
                    for arg in args {
                        kind = kind.merge(visit(
                            graph,
                            arg,
                            dependent,
                            range_sheet,
                            range,
                            None,
                            locals,
                        ));
                    }
                    kind
                }
                ASTNodeType::Array(rows) => {
                    rows.iter()
                        .flatten()
                        .fold(RangeSelfUse::NoMatch, |kind, item| {
                            kind.merge(visit(
                                graph,
                                item,
                                dependent,
                                range_sheet,
                                range,
                                None,
                                locals,
                            ))
                        })
                }
                ASTNodeType::Literal(_) | ASTNodeType::Omitted | ASTNodeType::Reference { .. } => {
                    RangeSelfUse::NoMatch
                }
            }
        }

        let mut locals = FxHashSet::default();
        local_names(&ast, &mut locals);
        visit(self, &ast, dependent, range_sheet, range, None, &locals)
    }

    pub(super) fn add_range_dependent_edges(
        &mut self,
        dependent: VertexId,
        ranges: &[SharedRangeRef<'static>],
        current_sheet_id: SheetId,
    ) {
        if ranges.is_empty() {
            return;
        }

        self.formula_to_range_deps
            .insert(dependent, ranges.to_vec());

        for range in ranges {
            // `current_sheet_id` is the dependent formula's sheet, which is what
            // `Current` means. An unresolvable sheet name falls back to it so a
            // stripe is still registered rather than the edge being dropped.
            let sheet_id = self
                .sheet_reg
                .resolve_locator(&range.sheet, current_sheet_id)
                .unwrap_or(current_sheet_id);

            let s_row = range.start_row.map(|b| b.index);
            let e_row = range.end_row.map(|b| b.index);
            let s_col = range.start_col.map(|b| b.index);
            let e_col = range.end_col.map(|b| b.index);

            // #120: a compressed range whose region covers this formula's own
            // cell is a self-reference. Record a self-loop so SCC detection
            // flags the cycle (the ingest self-ref check only sees expanded
            // cell edges, which compressed ranges do not produce).
            if self.range_region_contains_self(dependent, sheet_id, s_row, e_row, s_col, e_col)
                && self.compressed_range_self_use(dependent, sheet_id, (s_row, e_row, s_col, e_col))
                    != RangeSelfUse::Excluded
            {
                self.record_self_loop(dependent);
            }

            // #376: an all-unbounded range means "the whole sheet". The stripe
            // classification below would treat it as both column- and
            // row-striped, fall through both branches, and collapse it to a
            // single row-0 stripe, hiding edits anywhere else from this
            // dependent. Register full column coverage instead; the precision
            // check against `formula_to_range_deps` already treats the missing
            // bounds as unbounded.
            if s_row.is_none() && e_row.is_none() && s_col.is_none() && e_col.is_none() {
                self.register_whole_sheet_stripes(dependent, sheet_id);
                continue;
            }

            let col_stripes = (s_row.is_none() && e_row.is_none())
                || (s_col.is_some() && e_col.is_some() && (s_row.is_none() || e_row.is_none()));
            let row_stripes = (s_col.is_none() && e_col.is_none())
                || (s_row.is_some() && e_row.is_some() && (s_col.is_none() || e_col.is_none()));

            if col_stripes && !row_stripes {
                let sc = s_col.unwrap_or(0);
                let ec = e_col.unwrap_or(sc);
                for col in sc..=ec {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Column,
                        index: col,
                    };
                    self.stripe_to_dependents
                        .entry(key.clone())
                        .or_default()
                        .insert(dependent);
                    #[cfg(test)]
                    {
                        if self.stripe_to_dependents.get(&key).map(|s| s.len()) == Some(1)
                            && let Ok(mut g) = self.instr.lock()
                        {
                            g.stripe_inserts += 1;
                        }
                    }
                }
                continue;
            }

            if row_stripes && !col_stripes {
                let sr = s_row.unwrap_or(0);
                let er = e_row.unwrap_or(sr);
                for row in sr..=er {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Row,
                        index: row,
                    };
                    self.stripe_to_dependents
                        .entry(key.clone())
                        .or_default()
                        .insert(dependent);
                    #[cfg(test)]
                    {
                        if self.stripe_to_dependents.get(&key).map(|s| s.len()) == Some(1)
                            && let Ok(mut g) = self.instr.lock()
                        {
                            g.stripe_inserts += 1;
                        }
                    }
                }
                continue;
            }

            let start_row = s_row.unwrap_or(0);
            let start_col = s_col.unwrap_or(0);
            let end_row = e_row.unwrap_or(start_row);
            let end_col = e_col.unwrap_or(start_col);

            let height = end_row.saturating_sub(start_row) + 1;
            let width = end_col.saturating_sub(start_col) + 1;

            if self.config.enable_block_stripes && height > 1 && width > 1 {
                let start_block_row = start_row / BLOCK_H;
                let end_block_row = end_row / BLOCK_H;
                let start_block_col = start_col / BLOCK_W;
                let end_block_col = end_col / BLOCK_W;

                for block_row in start_block_row..=end_block_row {
                    for block_col in start_block_col..=end_block_col {
                        let key = StripeKey {
                            sheet_id,
                            stripe_type: StripeType::Block,
                            index: block_index(block_row * BLOCK_H, block_col * BLOCK_W),
                        };
                        self.stripe_to_dependents
                            .entry(key.clone())
                            .or_default()
                            .insert(dependent);
                        #[cfg(test)]
                        {
                            if self.stripe_to_dependents.get(&key).map(|s| s.len()) == Some(1)
                                && let Ok(mut g) = self.instr.lock()
                            {
                                g.stripe_inserts += 1;
                            }
                        }
                    }
                }
            } else if height > width {
                for col in start_col..=end_col {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Column,
                        index: col,
                    };
                    self.stripe_to_dependents
                        .entry(key.clone())
                        .or_default()
                        .insert(dependent);
                    #[cfg(test)]
                    {
                        if self.stripe_to_dependents.get(&key).map(|s| s.len()) == Some(1)
                            && let Ok(mut g) = self.instr.lock()
                        {
                            g.stripe_inserts += 1;
                        }
                    }
                }
            } else {
                for row in start_row..=end_row {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Row,
                        index: row,
                    };
                    self.stripe_to_dependents
                        .entry(key.clone())
                        .or_default()
                        .insert(dependent);
                    #[cfg(test)]
                    {
                        if self.stripe_to_dependents.get(&key).map(|s| s.len()) == Some(1)
                            && let Ok(mut g) = self.instr.lock()
                        {
                            g.stripe_inserts += 1;
                        }
                    }
                }
            }
        }
    }

    /// Register stripes covering every cell of a sheet, for a dependent whose
    /// range is unbounded on both axes (#376). Dirty-propagation lookups probe
    /// the column stripe of every edited cell, so covering all columns
    /// guarantees any edit on the sheet reaches the precision check.
    fn register_whole_sheet_stripes(&mut self, dependent: VertexId, sheet_id: SheetId) {
        /// Excel sheet column capacity (column XFD), as a 0-based exclusive bound.
        const SHEET_MAX_COLS: u32 = 16_384;
        for col in 0..SHEET_MAX_COLS {
            let key = StripeKey {
                sheet_id,
                stripe_type: StripeType::Column,
                index: col,
            };
            self.stripe_to_dependents
                .entry(key)
                .or_default()
                .insert(dependent);
        }
    }

    /// Fast-path: add range dependencies using compact RangeKey.
    pub fn add_range_deps_from_keys(
        &mut self,
        dependent: VertexId,
        keys: &[crate::engine::plan::RangeKey],
        current_sheet_id: SheetId,
    ) {
        self.add_range_deps_from_keys_reporting_self_loop(dependent, keys, current_sheet_id);
    }

    /// [`Self::add_range_deps_from_keys`], returning whether one of the
    /// ranges covers the formula's own cell and so recorded the #120
    /// self-loop. A bulk builder that installs the formula's out-edges from
    /// its own adjacency row must carry that edge in the row.
    pub(crate) fn add_range_deps_from_keys_reporting_self_loop(
        &mut self,
        dependent: VertexId,
        keys: &[crate::engine::plan::RangeKey],
        current_sheet_id: SheetId,
    ) -> bool {
        use crate::engine::plan::RangeKey as RK;
        let mut self_loop = false;
        if keys.is_empty() {
            return self_loop;
        }

        let mut shared_ranges: Vec<SharedRangeRef<'static>> = Vec::with_capacity(keys.len());
        for k in keys {
            let sheet_loc = SharedSheetLocator::Id(match k {
                RK::Rect { sheet, .. }
                | RK::WholeRow { sheet, .. }
                | RK::WholeCol { sheet, .. }
                | RK::OpenRect { sheet, .. } => *sheet,
            });

            let mk_axis = |idx0: u32| formualizer_common::AxisBound::new(idx0, false);

            let built = match k {
                RK::Rect { start, end, .. } => {
                    let sr = mk_axis(start.row());
                    let sc = mk_axis(start.col());
                    let er = mk_axis(end.row());
                    let ec = mk_axis(end.col());
                    SharedRangeRef::from_parts(sheet_loc, Some(sr), Some(sc), Some(er), Some(ec))
                        .ok()
                }
                RK::WholeRow { row, .. } => {
                    let r0 = row.saturating_sub(1);
                    let b = mk_axis(r0);
                    SharedRangeRef::from_parts(sheet_loc, Some(b), None, Some(b), None).ok()
                }
                RK::WholeCol { col, .. } => {
                    let c0 = col.saturating_sub(1);
                    let b = mk_axis(c0);
                    SharedRangeRef::from_parts(sheet_loc, None, Some(b), None, Some(b)).ok()
                }
                RK::OpenRect {
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                    ..
                } => SharedRangeRef::from_parts(
                    sheet_loc,
                    start_row.map(mk_axis),
                    start_col.map(mk_axis),
                    end_row.map(mk_axis),
                    end_col.map(mk_axis),
                )
                .ok(),
            };

            if let Some(r) = built {
                shared_ranges.push(r.into_owned());
            }
        }

        if shared_ranges.is_empty() {
            return self_loop;
        }

        self.formula_to_range_deps
            .insert(dependent, shared_ranges.clone());

        for range in &shared_ranges {
            // See add_range_dependent_edges.
            let sheet_id = self
                .sheet_reg
                .resolve_locator(&range.sheet, current_sheet_id)
                .unwrap_or(current_sheet_id);

            let s_row = range.start_row.map(|b| b.index);
            let e_row = range.end_row.map(|b| b.index);
            let s_col = range.start_col.map(|b| b.index);
            let e_col = range.end_col.map(|b| b.index);

            // #120: see add_range_dependent_edges — compressed range covering
            // the formula's own cell records a self-loop for SCC detection.
            if self.range_region_contains_self(dependent, sheet_id, s_row, e_row, s_col, e_col)
                && self.compressed_range_self_use(dependent, sheet_id, (s_row, e_row, s_col, e_col))
                    != RangeSelfUse::Excluded
            {
                self.record_self_loop(dependent);
                self_loop = true;
            }

            // #376: an all-unbounded range means "the whole sheet". The stripe
            // classification below would treat it as both column- and
            // row-striped, fall through both branches, and collapse it to a
            // single row-0 stripe, hiding edits anywhere else from this
            // dependent. Register full column coverage instead; the precision
            // check against `formula_to_range_deps` already treats the missing
            // bounds as unbounded.
            if s_row.is_none() && e_row.is_none() && s_col.is_none() && e_col.is_none() {
                self.register_whole_sheet_stripes(dependent, sheet_id);
                continue;
            }

            let col_stripes = (s_row.is_none() && e_row.is_none())
                || (s_col.is_some() && e_col.is_some() && (s_row.is_none() || e_row.is_none()));
            let row_stripes = (s_col.is_none() && e_col.is_none())
                || (s_row.is_some() && e_row.is_some() && (s_col.is_none() || e_col.is_none()));

            if col_stripes && !row_stripes {
                let sc = s_col.unwrap_or(0);
                let ec = e_col.unwrap_or(sc);
                for col in sc..=ec {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Column,
                        index: col,
                    };
                    self.stripe_to_dependents
                        .entry(key)
                        .or_default()
                        .insert(dependent);
                }
                continue;
            }

            if row_stripes && !col_stripes {
                let sr = s_row.unwrap_or(0);
                let er = e_row.unwrap_or(sr);
                for row in sr..=er {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Row,
                        index: row,
                    };
                    self.stripe_to_dependents
                        .entry(key)
                        .or_default()
                        .insert(dependent);
                }
                continue;
            }

            let start_row = s_row.unwrap_or(0);
            let start_col = s_col.unwrap_or(0);
            let end_row = e_row.unwrap_or(start_row);
            let end_col = e_col.unwrap_or(start_col);

            let height = end_row.saturating_sub(start_row) + 1;
            let width = end_col.saturating_sub(start_col) + 1;

            if self.config.enable_block_stripes && height > 1 && width > 1 {
                let start_block_row = start_row / BLOCK_H;
                let end_block_row = end_row / BLOCK_H;
                let start_block_col = start_col / BLOCK_W;
                let end_block_col = end_col / BLOCK_W;

                for block_row in start_block_row..=end_block_row {
                    for block_col in start_block_col..=end_block_col {
                        let key = StripeKey {
                            sheet_id,
                            stripe_type: StripeType::Block,
                            index: block_index(block_row * BLOCK_H, block_col * BLOCK_W),
                        };
                        self.stripe_to_dependents
                            .entry(key)
                            .or_default()
                            .insert(dependent);
                    }
                }
            } else if height > width {
                for col in start_col..=end_col {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Column,
                        index: col,
                    };
                    self.stripe_to_dependents
                        .entry(key)
                        .or_default()
                        .insert(dependent);
                }
            } else {
                for row in start_row..=end_row {
                    let key = StripeKey {
                        sheet_id,
                        stripe_type: StripeType::Row,
                        index: row,
                    };
                    self.stripe_to_dependents
                        .entry(key)
                        .or_default()
                        .insert(dependent);
                }
            }
        }
        self_loop
    }
}
