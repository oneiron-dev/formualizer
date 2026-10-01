use crate::SheetId;
use crate::engine::graph::DependencyGraph;
use crate::engine::vertex::{VertexId, VertexKind};
use crate::reference::RangeRef;
use formualizer_common::{ExcelError, ExcelErrorKind};

#[inline]
fn normalize_table_key(name: &str) -> String {
    name.to_lowercase()
}

/// Native workbook table (Excel ListObject) metadata.
#[derive(Debug, Clone)]
pub(crate) struct TableEntry {
    pub(crate) name: String,
    pub(crate) range: RangeRef,
    pub(crate) header_row: bool,
    pub(crate) headers: Vec<String>,
    pub(crate) totals_row: bool,
    pub(crate) vertex: VertexId,
}

impl TableEntry {
    pub fn sheet_id(&self) -> SheetId {
        self.range.start.sheet_id
    }

    pub fn col_index(&self, header: &str) -> Option<usize> {
        let header_key = header.to_lowercase();
        self.headers
            .iter()
            .position(|h| h.to_lowercase() == header_key)
    }

    /// The table's placement, for resolving structured references.
    pub(crate) fn geometry(&self) -> TableGeometry<'_> {
        TableGeometry {
            start_row: self.range.start.coord.row(),
            start_col: self.range.start.coord.col(),
            end_row: self.range.end.coord.row(),
            end_col: self.range.end.coord.col(),
            header_row: self.header_row,
            totals_row: self.totals_row,
            headers: &self.headers,
        }
    }
}

impl DependencyGraph {
    #[inline]
    fn table_lookup_key(&self, name: &str) -> String {
        if self.config.case_sensitive_tables {
            name.to_string()
        } else {
            normalize_table_key(name)
        }
    }

    fn canonical_table_name(&self, name: &str) -> Option<String> {
        let key = self.table_lookup_key(name);
        self.tables_lookup.get(&key).cloned()
    }

    pub(crate) fn resolve_table_entry(&self, name: &str) -> Option<&TableEntry> {
        if self.config.case_sensitive_tables {
            self.tables.get(name)
        } else {
            let key = self.table_lookup_key(name);
            self.tables_lookup
                .get(&key)
                .and_then(|canon| self.tables.get(canon))
        }
    }

    pub(crate) fn table_by_vertex(&self, vertex: VertexId) -> Option<&TableEntry> {
        self.table_vertex_lookup
            .get(&vertex)
            .and_then(|name| self.tables.get(name))
    }

    /// Canonical names of every defined table, sorted for deterministic output.
    pub fn table_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tables.values().map(|t| t.name.clone()).collect();
        names.sort();
        names
    }

    pub fn define_table(
        &mut self,
        name: &str,
        range: RangeRef,
        header_row: bool,
        headers: Vec<String>,
        totals_row: bool,
    ) -> Result<(), ExcelError> {
        if name.is_empty() {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message("Table name cannot be empty".to_string()));
        }

        let key = self.table_lookup_key(name);
        if let Some(existing) = self.tables_lookup.get(&key) {
            return Err(ExcelError::new(ExcelErrorKind::Name).with_message(format!(
                "Table collision under normalization: '{name}' conflicts with '{existing}'"
            )));
        }

        // A table has a range, but the *vertex* that represents the table symbol has no
        // position: it is identified by name. Parking it on the range's anchor cell put it
        // in the sheet index, where grid queries and structural edits could reach it (#304).
        // Dependencies on the table's cells are carried by the stripe registration below.
        let sheet_id = range.start.sheet_id;
        let vertex = self.allocate_symbol_vertex(VertexKind::Table, sheet_id);

        // Register stripes for the full table region so cell edits inside the table
        // propagate to formulas that depend on the table.
        self.register_table_range_deps(vertex, &range);

        let entry = TableEntry {
            name: name.to_string(),
            range,
            header_row,
            headers,
            totals_row,
            vertex,
        };

        let original = name.to_string();
        self.tables.insert(original.clone(), entry);
        self.tables_lookup
            .insert(self.table_lookup_key(&original), original.clone());
        self.table_vertex_lookup.insert(vertex, original);
        self.bump_symbol_revision();
        Ok(())
    }

    pub fn update_table(
        &mut self,
        name: &str,
        new_range: RangeRef,
        header_row: bool,
        headers: Vec<String>,
        totals_row: bool,
    ) -> Result<(), ExcelError> {
        let Some(canon) = self.canonical_table_name(name) else {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Unknown table: {name}")));
        };

        let vertex = self.tables.get(&canon).map(|t| t.vertex).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Name).with_message(format!("Unknown table: {name}"))
        })?;

        // Replace range deps (cleans old stripes).
        self.remove_dependent_edges(vertex);
        self.register_table_range_deps(vertex, &new_range);

        if let Some(existing) = self.tables.get_mut(&canon) {
            existing.range = new_range;
            existing.header_row = header_row;
            existing.headers = headers;
            existing.totals_row = totals_row;
        }

        // Propagate to dependents.
        self.mark_dirty(vertex);
        self.bump_symbol_revision();
        Ok(())
    }

    pub fn delete_table(&mut self, name: &str) -> Result<(), ExcelError> {
        let Some(canon) = self.canonical_table_name(name) else {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Unknown table: {name}")));
        };

        let Some(entry) = self.tables.remove(&canon) else {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Unknown table: {name}")));
        };

        self.tables_lookup.remove(&self.table_lookup_key(&canon));

        let vertex = entry.vertex;
        self.table_vertex_lookup.remove(&vertex);

        // Clean range deps / stripes.
        self.remove_dependent_edges(vertex);

        // Mark deleted for debuggability; edges already removed.
        self.store.mark_deleted(vertex, true);
        self.vertex_values.remove(&vertex);
        self.vertex_formulas.remove(&vertex);
        self.clear_formula_vertex_dirty(vertex);
        self.volatile_vertices.remove(&vertex);
        self.bump_symbol_revision();

        Ok(())
    }

    fn register_table_range_deps(&mut self, table_vertex: VertexId, range: &RangeRef) {
        use crate::reference::SharedRangeRef;
        use crate::reference::SharedSheetLocator;
        use formualizer_common::AxisBound;

        // Reuse the same range-deps machinery as formulas/names.
        let sheet_loc = SharedSheetLocator::Id(range.start.sheet_id);
        let sr = AxisBound::new(range.start.coord.row(), range.start.coord.row_abs());
        let sc = AxisBound::new(range.start.coord.col(), range.start.coord.col_abs());
        let er = AxisBound::new(range.end.coord.row(), range.end.coord.row_abs());
        let ec = AxisBound::new(range.end.coord.col(), range.end.coord.col_abs());

        if let Ok(r) = SharedRangeRef::from_parts(sheet_loc, Some(sr), Some(sc), Some(er), Some(ec))
        {
            self.add_range_dependent_edges(table_vertex, &[r.into_owned()], range.start.sheet_id);
        }
    }
}

/// A table's placement for resolving structured references (0-based bounds).
pub(crate) struct TableGeometry<'a> {
    pub(crate) start_row: u32,
    pub(crate) start_col: u32,
    pub(crate) end_row: u32,
    pub(crate) end_col: u32,
    pub(crate) header_row: bool,
    pub(crate) totals_row: bool,
    pub(crate) headers: &'a [String],
}

/// The 1-based area `(r1, c1, r2, c2)` of a structured reference that the
/// symbolic table path does not evaluate: row/area combinations such as
/// `[[#Headers],[Col]]` or `[[#This Row],[A]:[C]]`, `[#This Row]`, row
/// selectors and the bare table name. `None` keeps the symbolic forms
/// (a column, a column range or a single area item) unchanged. `row0` is the
/// formula cell's 0-based row, used by `#This Row`.
pub(crate) fn static_structured_area(
    table: &TableGeometry<'_>,
    specifier: Option<&formualizer_parse::parser::TableSpecifier>,
    row0: u32,
) -> Result<Option<(u32, u32, u32, u32)>, ExcelError> {
    use formualizer_parse::parser::{SpecialItem, TableSpecifier};

    match specifier {
        Some(
            TableSpecifier::Column(_)
            | TableSpecifier::ColumnRange(..)
            | TableSpecifier::All
            | TableSpecifier::Data
            | TableSpecifier::Headers
            | TableSpecifier::Totals
            | TableSpecifier::SpecialItem(
                SpecialItem::Headers | SpecialItem::Data | SpecialItem::Totals | SpecialItem::All,
            ),
        ) => return Ok(None),
        _ => {}
    }

    let (r1, c1, r2, c2) = structured_area(table, specifier, Some(row0))?;
    Ok(Some((r1 + 1, c1 + 1, r2 + 1, c2 + 1)))
}

/// The 0-based area `(r1, c1, r2, c2)` a structured reference reads, for
/// dependency planning: a formula reading `Table1[Qty]` must be computed after
/// the formulas in that column. `None` for `#This Row` forms (resolved per
/// cell) and references that do not resolve (they evaluate to an error).
pub(crate) fn structured_dependency_area(
    table: &TableGeometry<'_>,
    specifier: Option<&formualizer_parse::parser::TableSpecifier>,
) -> Option<(u32, u32, u32, u32)> {
    structured_reference_area(table, specifier).ok()
}

/// The 0-based area `(r1, c1, r2, c2)` a structured reference selects when
/// read outside any formula row. `Table[]`, `Table[#Data]` and the bare table
/// name are the data body and `Table[Col]` is that column's data cells, so the
/// area's upper-left cell (what CELL, ISFORMULA and FORMULATEXT read) is the
/// first data cell, not the header. A specifier that does not resolve (an
/// unknown column, `#Headers` on a table without a header row) is #REF!;
/// `#This Row`, which needs the formula's row, is #VALUE!.
pub(crate) fn structured_reference_area(
    table: &TableGeometry<'_>,
    specifier: Option<&formualizer_parse::parser::TableSpecifier>,
) -> Result<(u32, u32, u32, u32), ExcelError> {
    structured_area(table, specifier, None)
}

fn structured_area(
    table: &TableGeometry<'_>,
    specifier: Option<&formualizer_parse::parser::TableSpecifier>,
    row0: Option<u32>,
) -> Result<(u32, u32, u32, u32), ExcelError> {
    use formualizer_parse::parser::{SpecialItem, TableRowSpecifier, TableSpecifier};

    let data_start = table.start_row + u32::from(table.header_row);
    let data_end = table.end_row - u32::from(table.totals_row);
    let reference_error = || ExcelError::new(ExcelErrorKind::Ref);
    let mut rows: Option<(u32, u32)> = None;
    let mut cols: Option<(u32, u32)> = None;
    let union = |span: &mut Option<(u32, u32)>, lo: u32, hi: u32| {
        *span = Some(match *span {
            Some((a, b)) => (a.min(lo), b.max(hi)),
            None => (lo, hi),
        });
    };
    // Column names match case-insensitively, as `TableEntry::col_index` does
    // when the reference is read.
    let column = |name: &str| {
        let key = name.trim().to_lowercase();
        table
            .headers
            .iter()
            .position(|h| h.to_lowercase() == key)
            .map(|i| table.start_col + i as u32)
            .ok_or_else(reference_error)
    };
    let mut pending = specifier.map_or_else(Vec::new, |s| vec![s]);
    while let Some(part) = pending.pop() {
        match part {
            TableSpecifier::Combination(parts) => pending.extend(parts.iter().map(Box::as_ref)),
            TableSpecifier::Column(name) => {
                let c = column(name)?;
                union(&mut cols, c, c);
            }
            TableSpecifier::ColumnRange(a, b) => {
                let (a, b) = (column(a)?, column(b)?);
                union(&mut cols, a.min(b), a.max(b));
            }
            TableSpecifier::All
            | TableSpecifier::SpecialItem(SpecialItem::All)
            | TableSpecifier::Row(TableRowSpecifier::All) => {
                union(&mut rows, table.start_row, table.end_row);
            }
            TableSpecifier::Data
            | TableSpecifier::SpecialItem(SpecialItem::Data)
            | TableSpecifier::Row(TableRowSpecifier::Data) => {
                union(&mut rows, data_start, data_end);
            }
            TableSpecifier::Headers
            | TableSpecifier::SpecialItem(SpecialItem::Headers)
            | TableSpecifier::Row(TableRowSpecifier::Headers) => {
                if !table.header_row {
                    return Err(reference_error());
                }
                union(&mut rows, table.start_row, table.start_row);
            }
            TableSpecifier::Totals
            | TableSpecifier::SpecialItem(SpecialItem::Totals)
            | TableSpecifier::Row(TableRowSpecifier::Totals) => {
                if !table.totals_row {
                    return Err(reference_error());
                }
                union(&mut rows, table.end_row, table.end_row);
            }
            TableSpecifier::SpecialItem(SpecialItem::ThisRow)
            | TableSpecifier::Row(TableRowSpecifier::Current) => {
                let Some(row0) = row0 else {
                    return Err(ExcelError::new(ExcelErrorKind::Value));
                };
                // Outside the table body the implicit intersection fails.
                if row0 < data_start || row0 > table.end_row {
                    return Err(ExcelError::new(ExcelErrorKind::Value));
                }
                union(&mut rows, row0, row0);
            }
            TableSpecifier::Row(TableRowSpecifier::Index(_)) => {
                return Err(ExcelError::new(ExcelErrorKind::NImpl)
                    .with_message("Indexed table row selectors are not supported".to_string()));
            }
        }
    }
    let (r1, r2) = rows.unwrap_or((data_start, data_end));
    let (c1, c2) = cols.unwrap_or((table.start_col, table.end_col));
    if r1 > r2 {
        return Err(reference_error());
    }
    Ok((r1, c1, r2, c2))
}

/// An A1 reference for a resolved structured-reference area.
pub(crate) fn area_reference(
    sheet: Option<String>,
    area: (u32, u32, u32, u32),
) -> formualizer_parse::parser::ReferenceType {
    use formualizer_parse::parser::ReferenceType;
    let (r1, c1, r2, c2) = area;
    if r1 == r2 && c1 == c2 {
        ReferenceType::Cell {
            sheet,
            row: r1,
            col: c1,
            row_abs: true,
            col_abs: true,
        }
    } else {
        ReferenceType::Range {
            sheet,
            start_row: Some(r1),
            start_col: Some(c1),
            end_row: Some(r2),
            end_col: Some(c2),
            start_row_abs: true,
            start_col_abs: true,
            end_row_abs: true,
            end_col_abs: true,
        }
    }
}
