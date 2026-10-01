//! Formula locations/cache spans without a rich cell graph.
use super::{IoError, XlsxRecalculateOptions, unsupported, xml};
use formualizer_common::coord::parse_a1_1based;
use std::{collections::HashMap, ops::Range};

#[derive(Debug)]
pub(super) struct ValueNode {
    pub span: Range<usize>,
    pub open_end: usize,
    pub close_start: usize,
    pub empty: bool,
    pub qualified: String,
    pub text: String,
}
#[derive(Debug)]
pub(super) struct Cell {
    pub row: u32,
    pub col: u32,
    pub address: String,
    pub span: Range<usize>,
    pub open_end: usize,
    pub qualified: String,
    pub kind: Option<String>,
    pub kind_span: Option<Range<usize>>,
    pub formula_end: usize,
    pub formula_text: String,
    pub value: Option<ValueNode>,
    pub inline: Option<Range<usize>>,
    pub formula_kind: String,
    shared_id: Option<u32>,
    shared_range: Option<(u32, u32, u32, u32)>,
    shared_ref_span: Option<Range<usize>>,
    /// A replacement `ref="..."` attribute for the transient ingestion view
    /// when a shared-formula master is not the top-left cell of its range.
    pub reanchored_ref: Option<(Range<usize>, String)>,
    /// The recorded `ref` of a multi-cell array formula anchored here.
    pub array_extent: Option<(u32, u32, u32, u32)>,
    has_formula: bool,
    pub dynamic_array: bool,
    /// The formula is calculated always (`ca`).
    pub calc_always: bool,
}
/// A value cell inside a multi-cell array formula's extent; `anchor` indexes
/// the formula cell in [`Scan::cells`]. `marked`: the member holds the empty
/// `<f>` (no formula text) that Excel writes into each member of an array
/// formula that is calculated always.
#[derive(Debug)]
pub(super) struct Member {
    pub anchor: usize,
    pub cell: Cell,
    pub marked: bool,
}
#[derive(Debug, Default)]
pub(super) struct Scan {
    pub cells: Vec<Cell>,
    pub members: Vec<Member>,
}
/// Calamine's fast scalar reader consumes just one raw ASCII text event.
/// Literal cells must satisfy that assumption; formula caches may instead be
/// cleared in the transient ingestion view because they are not authority.
pub(super) fn readable_scalar_cache(cell: &Cell, bytes: &[u8]) -> bool {
    let Some(v) = &cell.value else {
        return true;
    };
    if !matches!(cell.kind.as_deref(), None | Some("n" | "s" | "b" | "e")) {
        return true;
    }
    if v.empty || v.text.is_empty() {
        return matches!(cell.kind.as_deref(), None | Some("n"));
    }
    if bytes[v.open_end..v.close_start] != *v.text.as_bytes() {
        return false;
    }
    match cell.kind.as_deref() {
        None | Some("n") => v.text.parse::<f64>().is_ok_and(f64::is_finite),
        Some("s") => v.text.parse::<u32>().is_ok(),
        Some("b") => matches!(v.text.as_str(), "0" | "1"),
        Some("e") => v.text.parse::<calamine::CellErrorType>().is_ok(),
        _ => true,
    }
}
fn coord(value: &str) -> Result<(u32, u32), IoError> {
    let (r, c, ra, ca) =
        parse_a1_1based(value).map_err(|_| unsupported("invalid A1 coordinate", "worksheet"))?;
    if ra || ca || r == 0 || r > 1_048_576 || c == 0 || c > 16_384 {
        return Err(unsupported(
            "out-of-grid or absolute cell coordinate",
            "worksheet",
        ));
    }
    Ok((r, c))
}
fn rect(value: &str) -> Result<(u32, u32, u32, u32), IoError> {
    let (a, b) = value.split_once(':').unwrap_or((value, value));
    let (r1, c1) = coord(a)?;
    let (r2, c2) = coord(b)?;
    if r1 > r2 || c1 > c2 {
        return Err(unsupported("reversed shared formula range", "worksheet"));
    }
    Ok((r1, c1, r2, c2))
}
fn integer(value: &str) -> Result<u32, IoError> {
    value
        .parse()
        .map_err(|_| unsupported("invalid integer XML attribute", "worksheet"))
}
pub(super) fn scan(
    bytes: &[u8],
    options: &XlsxRecalculateOptions,
    observed: &mut usize,
    logical_cells: &mut u64,
) -> Result<Scan, IoError> {
    let mut cells = Vec::new();
    let mut members = Vec::new();
    let mut extents: Vec<(u32, u32, u32, u32, usize)> = Vec::new();
    let mut current: Option<Cell> = None;
    let mut row = 0;
    let mut column = 0;
    let mut sheet_data_count = 0;
    let mut dimension = None;
    let mut max_row = 0u32;
    let mut max_col = 0u32;
    xml::walk(bytes, options, |path, node| {
        let Some(element) = path.last() else {
            return Ok(());
        };
        let is_cell = xml::path_is(path, xml::MAIN, &["worksheet", "sheetData", "row", "c"]);
        let direct = path.len() == 5
            && xml::path_is(
                &path[..4],
                xml::MAIN,
                &["worksheet", "sheetData", "row", "c"],
            );
        match &node.kind {
            xml::Kind::Open { empty, .. } => {
                if path.len() == 1 && !xml::path_is(path, xml::MAIN, &["worksheet"]) {
                    return Err(unsupported("worksheet root/namespace", "worksheet"));
                }
                // Foreign-namespace markup outside sheetData (extLst data
                // validation xm:f, AlternateContent control anchors xdr:row)
                // is not cell content; cell readers only consume sheetData.
                if element.ns != xml::MAIN
                    && path.len() > 2
                    && path[1].local != "sheetData"
                    && element.local != "dimension"
                {
                    return Ok(());
                }
                let structural = [
                    "worksheet",
                    "sheetData",
                    "row",
                    "c",
                    "f",
                    "v",
                    "is",
                    "t",
                    "r",
                    "rPr",
                    "dimension",
                ];
                if structural.contains(&element.local.as_str()) && element.ns != xml::MAIN {
                    return Err(unsupported("foreign worksheet lookalike", &element.local));
                }
                if matches!(element.local.as_str(), "f" | "v" | "is") && !direct {
                    return Err(unsupported("misplaced cell payload", "worksheet"));
                }
                if element.local == "dimension" {
                    if !xml::path_is(path, xml::MAIN, &["worksheet", "dimension"])
                        || dimension.is_some()
                        || sheet_data_count != 0
                    {
                        return Err(unsupported("duplicate/misplaced dimension", "worksheet"));
                    }
                    let range = rect(node.required("ref")?)?;
                    let area = u64::from(range.2) * u64::from(range.3);
                    if range.3 > options.limits.max_columns {
                        return Err(unsupported("worksheet width limit", "worksheet"));
                    }
                    if area > options.limits.max_cells as u64 {
                        return Err(unsupported("worksheet dimension cell limit", "worksheet"));
                    }
                    dimension = Some(range);
                }
                if element.local == "sheetData" {
                    if !xml::path_is(path, xml::MAIN, &["worksheet", "sheetData"]) {
                        return Err(unsupported("misplaced sheetData", "worksheet"));
                    }
                    sheet_data_count += 1;
                    if sheet_data_count != 1 {
                        return Err(unsupported("duplicate sheetData", "worksheet"));
                    }
                }
                if element.local == "row" {
                    if !xml::path_is(path, xml::MAIN, &["worksheet", "sheetData", "row"]) {
                        return Err(unsupported("misplaced row", "worksheet"));
                    }
                    let next_row = integer(node.required("r")?)?;
                    if next_row <= row || next_row > 1_048_576 {
                        return Err(unsupported(
                            "invalid/non-increasing worksheet row",
                            "worksheet",
                        ));
                    }
                    row = next_row;
                    column = 0;
                }
                if element.local == "c" {
                    if !is_cell || current.is_some() {
                        return Err(unsupported("misplaced/nested cell", "worksheet"));
                    }
                    let address = node.required("r")?;
                    let (r, c) = coord(address)?;
                    if c > options.limits.max_columns {
                        return Err(unsupported("worksheet width limit", "worksheet"));
                    }
                    max_row = max_row.max(r);
                    max_col = max_col.max(c);
                    if row != r || c <= column {
                        return Err(unsupported(
                            "duplicate/non-increasing cell coordinate",
                            "worksheet",
                        ));
                    }
                    column = c;
                    if let Some((r1, c1, r2, c2)) = dimension
                        && (!(r1..=r2).contains(&r) || !(c1..=c2).contains(&c))
                    {
                        return Err(unsupported("cell outside worksheet dimension", "worksheet"));
                    }
                    *observed = observed
                        .checked_add(1)
                        .ok_or_else(|| unsupported("cell count overflow", "worksheet"))?;
                    if *observed > options.limits.max_cells {
                        return Err(unsupported("serialized cell count limit", "worksheet"));
                    }
                    // `cm` (dynamic-array cell metadata) is admitted only on
                    // single-cell array formulas, checked when the cell closes.
                    if node.value("vm").is_some() {
                        return Err(unsupported("dynamic/rich cell metadata", "worksheet"));
                    }
                    let dynamic_array = node.value("cm").is_some();
                    let kind = node.value("t").map(str::to_owned);
                    if !matches!(
                        kind.as_deref(),
                        None | Some("n" | "b" | "e" | "str" | "s" | "inlineStr" | "d")
                    ) {
                        return Err(unsupported("unknown cell type", "worksheet"));
                    }
                    if !*empty {
                        current = Some(Cell {
                            row: r,
                            col: c,
                            address: address.to_owned(),
                            span: node.span.clone(),
                            open_end: node.span.end,
                            qualified: element.qualified.clone(),
                            kind,
                            kind_span: node.attribute("", "t").map(|a| a.span.clone()),
                            formula_end: 0,
                            formula_text: String::new(),
                            value: None,
                            inline: None,
                            formula_kind: String::new(),
                            shared_id: None,
                            shared_range: None,
                            shared_ref_span: None,
                            reanchored_ref: None,
                            array_extent: None,
                            has_formula: false,
                            dynamic_array,
                            calc_always: false,
                        });
                    }
                }
                if direct {
                    let cell = current
                        .as_mut()
                        .ok_or_else(|| unsupported("payload without cell", "worksheet"))?;
                    match element.local.as_str() {
                        "f" => {
                            if cell.has_formula || cell.value.is_some() || cell.inline.is_some() {
                                return Err(unsupported(
                                    "duplicate/misordered formula",
                                    "worksheet",
                                ));
                            }
                            cell.has_formula = true;
                            // ca is an XML Schema boolean: surrounding whitespace collapses.
                            cell.calc_always =
                                matches!(node.value("ca").map(str::trim), Some("1" | "true"));
                            cell.formula_kind = node.value("t").unwrap_or("normal").to_owned();
                            if !matches!(cell.formula_kind.as_str(), "normal" | "shared" | "array")
                            {
                                return Err(unsupported(
                                    "array/data-table/unknown formula kind",
                                    "worksheet",
                                ));
                            }
                            // An array formula (legacy CSE or dynamic array)
                            // is an ordinary formula evaluated with array
                            // semantics. A multi-cell extent anchored at its
                            // top-left keeps its members for result writeback.
                            if cell.formula_kind == "array" {
                                let extent =
                                    node.value("ref").map(rect).transpose()?.ok_or_else(|| {
                                        unsupported("array formula without extent", "worksheet")
                                    })?;
                                if (extent.0, extent.1) != (cell.row, cell.col) {
                                    return Err(unsupported(
                                        "multi-cell array formula extent",
                                        "worksheet",
                                    ));
                                }
                                let area = u64::from(extent.2 - extent.0 + 1)
                                    * u64::from(extent.3 - extent.1 + 1);
                                if area > options.limits.max_cells as u64 {
                                    return Err(unsupported(
                                        "array extent cell limit",
                                        "worksheet",
                                    ));
                                }
                                if area > 1 {
                                    cell.array_extent = Some(extent);
                                }
                            }
                            if node.value("ref").is_some()
                                && !matches!(cell.formula_kind.as_str(), "shared" | "array")
                            {
                                return Err(unsupported("non-shared formula extent", "worksheet"));
                            }
                            if cell.formula_kind == "shared" {
                                cell.shared_id = Some(integer(node.required("si")?)?);
                                cell.shared_range = node.value("ref").map(rect).transpose()?;
                                cell.shared_ref_span =
                                    node.attribute("", "ref").map(|a| a.span.clone());
                            }
                            if *empty {
                                cell.formula_end = node.span.end;
                            }
                        }
                        "v" => {
                            if cell.value.is_some() || cell.inline.is_some() {
                                return Err(unsupported(
                                    "duplicate/ambiguous cell cache",
                                    "worksheet",
                                ));
                            }
                            cell.value = Some(ValueNode {
                                span: node.span.clone(),
                                open_end: node.span.end,
                                close_start: node.span.end,
                                empty: *empty,
                                qualified: element.qualified.clone(),
                                text: String::new(),
                            });
                        }
                        "is" => {
                            if cell.inline.is_some() || cell.value.is_some() {
                                return Err(unsupported(
                                    "duplicate/ambiguous inline cache",
                                    "worksheet",
                                ));
                            }
                            cell.inline = Some(node.span.clone());
                        }
                        _ => {}
                    }
                }
                if path.len() > 5 && matches!(path[4].local.as_str(), "f" | "v") {
                    return Err(unsupported("nested formula/cache content", "worksheet"));
                }
            }
            xml::Kind::Text(text) if direct => {
                if let Some(cell) = current.as_mut() {
                    if element.local == "f" {
                        cell.formula_text.push_str(text);
                    }
                    if element.local == "v" {
                        cell.value.as_mut().expect("opened v").text.push_str(text);
                    }
                }
            }
            xml::Kind::Close => {
                if direct {
                    let cell = current
                        .as_mut()
                        .ok_or_else(|| unsupported("unbalanced cell payload", "worksheet"))?;
                    match element.local.as_str() {
                        "f" => cell.formula_end = node.span.end,
                        "v" => {
                            let v = cell.value.as_mut().expect("opened v");
                            v.close_start = node.span.start;
                            v.span.end = node.span.end;
                        }
                        "is" => cell.inline.as_mut().expect("opened is").end = node.span.end,
                        _ => {}
                    }
                }
                if is_cell {
                    let mut cell = current
                        .take()
                        .ok_or_else(|| unsupported("unbalanced cell", "worksheet"))?;
                    cell.span.end = node.span.end;
                    let array_anchor = extents
                        .iter()
                        .find(|&&(r1, c1, r2, c2, _)| {
                            (r1..=r2).contains(&cell.row) && (c1..=c2).contains(&cell.col)
                        })
                        .map(|extent| extent.4);
                    if let Some(anchor) = array_anchor {
                        let marked = cell.has_formula;
                        if marked
                            && (cell.formula_kind != "normal"
                                || !cell.formula_text.trim().is_empty())
                        {
                            return Err(unsupported(
                                "formula inside an array formula extent",
                                "worksheet",
                            ));
                        }
                        // Its cache is written after the anchor is evaluated;
                        // a missing <v> goes right after the opening tag (or
                        // its empty <f>).
                        if !marked {
                            cell.formula_end = cell.open_end;
                        }
                        members.push(Member {
                            anchor,
                            cell,
                            marked,
                        });
                        return Ok(());
                    }
                    if !cell.has_formula
                        && cell.kind.as_deref() == Some("e")
                        && cell
                            .value
                            .as_ref()
                            .is_none_or(|v| v.text.parse::<calamine::CellErrorType>().is_err())
                    {
                        return Err(unsupported(
                            "literal error value unsupported by Calamine",
                            "worksheet",
                        ));
                    }
                    if !cell.has_formula && !readable_scalar_cache(&cell, bytes) {
                        return Err(unsupported(
                            "literal scalar payload is not supported by Calamine",
                            "worksheet",
                        ));
                    }
                    if cell.dynamic_array && cell.formula_kind != "array" {
                        return Err(unsupported("dynamic/rich cell metadata", "worksheet"));
                    }
                    if cell.has_formula {
                        if cell.formula_kind != "shared" && cell.formula_text.trim().is_empty() {
                            return Err(unsupported("empty ordinary formula", "worksheet"));
                        }
                        if cell.formula_end == 0 {
                            return Err(unsupported("missing formula boundary", "worksheet"));
                        }
                        if let Some((r1, c1, r2, c2)) = cell.array_extent {
                            extents.push((r1, c1, r2, c2, cells.len()));
                        }
                        cells.push(cell);
                        if cells.len() > options.limits.max_formula_cells {
                            return Err(unsupported("formula cell count limit", "worksheet"));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    if sheet_data_count != 1 {
        return Err(unsupported("missing sheetData", "worksheet"));
    }
    if let Some((_, _, r, c)) = dimension {
        max_row = max_row.max(r);
        max_col = max_col.max(c);
    }
    *logical_cells = logical_cells
        .checked_add(u64::from(max_row) * u64::from(max_col))
        .ok_or_else(|| unsupported("logical area overflow", "workbook"))?;
    if *logical_cells > options.limits.max_cells as u64 {
        return Err(unsupported("workbook logical cell limit", "workbook"));
    }
    let mut anchors = HashMap::new();
    for cell in &mut cells {
        if let Some(id) = cell.shared_id {
            if !cell.formula_text.trim().is_empty() {
                let mut range = cell
                    .shared_range
                    .ok_or_else(|| unsupported("unbounded shared formula anchor", "worksheet"))?;
                // The master's text is relative to the master cell, while
                // readers expand from the range's top-left. Re-anchor the
                // range at the master (members are checked against it below).
                if (cell.row, cell.col) != (range.0, range.1) {
                    if cell.row < range.0
                        || cell.col < range.1
                        || cell.row > range.2
                        || cell.col > range.3
                    {
                        return Err(unsupported(
                            "non-top-left shared formula anchor",
                            "worksheet",
                        ));
                    }
                    range = (cell.row, cell.col, range.2, range.3);
                    let span = cell.shared_ref_span.clone().ok_or_else(|| {
                        unsupported("unbounded shared formula anchor", "worksheet")
                    })?;
                    let reference = format!(
                        "ref=\"{}:{}\"",
                        formualizer_common::coord::col_letters_from_1based(range.1)
                            .map_err(|_| unsupported("invalid A1 coordinate", "worksheet"))?
                            + &range.0.to_string(),
                        formualizer_common::coord::col_letters_from_1based(range.3)
                            .map_err(|_| unsupported("invalid A1 coordinate", "worksheet"))?
                            + &range.2.to_string(),
                    );
                    cell.reanchored_ref = Some((span, reference));
                }
                let area = u64::from(range.2 - range.0 + 1) * u64::from(range.3 - range.1 + 1);
                if area > options.limits.max_cells as u64 || anchors.insert(id, range).is_some() {
                    return Err(unsupported(
                        "duplicate/oversized shared formula anchor",
                        "worksheet",
                    ));
                }
            } else if cell.shared_range.is_some() {
                return Err(unsupported("shared descendant declares range", "worksheet"));
            }
        }
    }
    for cell in &cells {
        if let Some(id) = cell.shared_id {
            let &(r1, c1, r2, c2) = anchors
                .get(&id)
                .ok_or_else(|| unsupported("orphan shared formula descendant", "worksheet"))?;
            if !(r1..=r2).contains(&cell.row) || !(c1..=c2).contains(&cell.col) {
                return Err(unsupported(
                    "shared formula outside declared range",
                    "worksheet",
                ));
            }
        }
    }
    Ok(Scan { cells, members })
}
