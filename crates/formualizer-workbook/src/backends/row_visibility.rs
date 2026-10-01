//! Rows a worksheet saves as hidden, and whether a filter hid them.
//!
//! `<row hidden="1">` does not record why a row is hidden. A hidden row counts
//! as filtered out when it lies below the header row of a filter that is
//! applied: the worksheet `<autoFilter>` (or, without one, an Advanced
//! Filter's `_xlnm._FilterDatabase` range) while `<sheetPr filterMode="1">`
//! is set, or any worksheet or table AutoFilter that holds criteria. Every
//! other hidden row was hidden by hand (Hide Rows or a collapsed outline).
//! SUBTOTAL 1-11 skips only filtered rows; 101-111 skip both kinds.
//!
//! Shared by the xlsx backends: each passes its package bytes and defined
//! names, so a saved file splits the same way whichever backend loads it.
//!
//! The same pass over each worksheet records the extent (`ref`) of every
//! array formula, legacy or dynamic. The cells of that extent other than the
//! anchor store the anchor's last result, not constants of their own; a
//! backend that streams cell values reads them here so it does not have to
//! read the worksheet twice.

use crate::traits::{DefinedName, DefinedNameDefinition};
use crate::xlsx_path::{local_attr, read_member};
use quick_xml::Reader as XmlReader;
use quick_xml::events::{BytesStart, Event};
use std::collections::HashMap;
use std::io::{BufReader, Read, Seek};
use zip::ZipArchive;

/// 1-based hidden rows of one sheet, split by what hid them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct HiddenRows {
    pub(super) manual: Vec<u32>,
    pub(super) filter: Vec<u32>,
}

/// First and last row of an `<autoFilter ref>`, and whether it holds criteria.
#[derive(Debug, Clone, Copy)]
struct AutoFilter {
    first: u32,
    last: u32,
    criteria: bool,
}

/// What one worksheet or table part says about hidden rows and filters, and
/// the extents of its array formulas.
#[derive(Debug, Default)]
struct PartScan {
    hidden: Vec<u32>,
    filter_mode: bool,
    auto_filter: Option<AutoFilter>,
    table_ids: Vec<String>,
    arrays: Vec<CellRect>,
}

/// 1-based `(first row, first col, last row, last col)` of a cell range.
pub(super) type CellRect = (u32, u32, u32, u32);

/// What the worksheet pass reads from one sheet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct SheetScan {
    pub(super) hidden_rows: HiddenRows,
    /// The `ref` of each array formula (`<f t="array">`), in file order.
    pub(super) array_extents: Vec<CellRect>,
}

/// First and last row of each sheet's `_xlnm._FilterDatabase` name, which
/// holds the range of an Advanced Filter (it leaves no `<autoFilter>`).
pub(super) fn filter_databases(names: &[DefinedName]) -> HashMap<String, (u32, u32)> {
    names
        .iter()
        .filter(|name| name.name.eq_ignore_ascii_case("_xlnm._FilterDatabase"))
        .filter_map(|name| match &name.definition {
            DefinedNameDefinition::Range { address } => {
                Some((address.sheet.clone(), (address.start_row, address.end_row)))
            }
            _ => None,
        })
        .collect()
}

/// Hidden rows and array formula extents of every sheet that has any, keyed
/// by sheet name. `filter_databases` maps a sheet to the rows of its
/// `_xlnm._FilterDatabase`.
pub(super) fn scan_sheets<R: Read + Seek>(
    reader: R,
    filter_databases: &HashMap<String, (u32, u32)>,
) -> HashMap<String, SheetScan> {
    let Ok(mut archive) = ZipArchive::new(reader) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (sheet, part) in sheet_parts(&mut archive) {
        let Some(mut scan) = scan_part(&mut archive, &part) else {
            continue;
        };
        let array_extents = std::mem::take(&mut scan.arrays);
        if scan.hidden.is_empty() {
            if !array_extents.is_empty() {
                out.insert(
                    sheet,
                    SheetScan {
                        hidden_rows: HiddenRows::default(),
                        array_extents,
                    },
                );
            }
            continue;
        }
        // (first, last) rows of each applied filter; the first is its header.
        let mut filters = Vec::new();
        match scan.auto_filter {
            Some(f) if f.criteria || scan.filter_mode => filters.push((f.first, f.last)),
            None if scan.filter_mode => filters.extend(filter_databases.get(&sheet).copied()),
            _ => {}
        }
        if !scan.table_ids.is_empty() {
            let rels = relationships(&mut archive, &part);
            for id in &scan.table_ids {
                if let Some(f) = rels
                    .get(id)
                    .and_then(|table| scan_part(&mut archive, table))
                    .and_then(|table| table.auto_filter)
                    && f.criteria
                {
                    filters.push((f.first, f.last));
                }
            }
        }
        scan.hidden.sort_unstable();
        scan.hidden.dedup();
        let (filter, manual) = scan.hidden.into_iter().partition(|row| {
            filters
                .iter()
                .any(|&(first, last)| first < *row && *row <= last)
        });
        out.insert(
            sheet,
            SheetScan {
                hidden_rows: HiddenRows { manual, filter },
                array_extents,
            },
        );
    }
    out
}

/// `(sheet name, worksheet part)` for each `<sheet>` in workbook.xml.
fn sheet_parts<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Vec<(String, String)> {
    const WORKBOOK: &str = "xl/workbook.xml";
    let Some(workbook) = read_member(archive, WORKBOOK) else {
        return Vec::new();
    };
    let rels = relationships(archive, WORKBOOK);
    let mut xml = XmlReader::from_reader(workbook.as_slice());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        buf.clear();
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(ref e) | Event::Empty(ref e))
                if e.local_name().as_ref() == b"sheet" =>
            {
                if let (Some(name), Some(part)) = (
                    local_attr(&xml, e, b"name"),
                    local_attr(&xml, e, b"id").and_then(|id| rels.get(&id).cloned()),
                ) {
                    out.push((name, part));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Internal relationship targets of `part`, by id, as package member names.
fn relationships<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    part: &str,
) -> HashMap<String, String> {
    let rels_part = match part.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/_rels/{name}.rels"),
        None => format!("_rels/{part}.rels"),
    };
    let mut out = HashMap::new();
    let Some(rels) = read_member(archive, &rels_part) else {
        return out;
    };
    let mut xml = XmlReader::from_reader(rels.as_slice());
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(ref e) | Event::Empty(ref e))
                if e.local_name().as_ref() == b"Relationship" =>
            {
                if local_attr(&xml, e, b"TargetMode").as_deref() == Some("External") {
                    continue;
                }
                if let (Some(id), Some(target)) = (
                    local_attr(&xml, e, b"Id"),
                    local_attr(&xml, e, b"Target")
                        .and_then(|t| crate::xlsx_path::resolve(part, &t).ok()),
                ) {
                    out.insert(id, target);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Stream a worksheet or table part for its hidden rows, filter mode,
/// top-level `<autoFilter>` and table part ids.
fn scan_part<R: Read + Seek>(archive: &mut ZipArchive<R>, part: &str) -> Option<PartScan> {
    let entry = archive.by_name(part).ok()?;
    let mut xml = XmlReader::from_reader(BufReader::new(entry));
    let mut scan = PartScan::default();
    let mut buf = Vec::new();
    // Open ancestors of the next element: the root's children are at depth 1.
    let mut depth = 0usize;
    let mut last_row = 0u32;
    let mut in_auto_filter = false;
    loop {
        buf.clear();
        let (e, empty) = match xml.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => (e, false),
            Ok(Event::Empty(e)) => (e, true),
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
                if depth <= 1 {
                    in_auto_filter = false;
                }
                continue;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => continue,
        };
        match (depth, e.local_name().as_ref()) {
            (1, b"sheetPr") => {
                scan.filter_mode = is_true(local_attr(&xml, &e, b"filterMode"));
            }
            (2, b"row") => {
                // A row without r follows the previous one. Saturate so a
                // malformed r near u32::MAX cannot overflow; anything past
                // row 1048576 is out of range and skipped below.
                let row = local_attr(&xml, &e, b"r")
                    .and_then(|r| r.parse().ok())
                    .unwrap_or_else(|| last_row.saturating_add(1));
                last_row = row;
                if (1..=1_048_576).contains(&row) && is_true(local_attr(&xml, &e, b"hidden")) {
                    scan.hidden.push(row);
                }
            }
            (1, b"autoFilter") => {
                scan.auto_filter =
                    local_attr(&xml, &e, b"ref")
                        .and_then(|r| row_span(&r))
                        .map(|(first, last)| AutoFilter {
                            first,
                            last,
                            criteria: false,
                        });
                in_auto_filter = !empty;
            }
            (
                3,
                b"filters" | b"customFilters" | b"top10" | b"dynamicFilter" | b"colorFilter"
                | b"iconFilter",
            ) if in_auto_filter => {
                if let Some(f) = scan.auto_filter.as_mut() {
                    f.criteria = true;
                }
            }
            (2, b"tablePart") => {
                if let Some(id) = local_attr(&xml, &e, b"id") {
                    scan.table_ids.push(id);
                }
            }
            // <worksheet><sheetData><row><c><f t="array" ref="B1:B3">
            (4, b"f") if is_array_formula(&e) => {
                if let Some(extent) = local_attr(&xml, &e, b"ref").and_then(|r| cell_rect(&r)) {
                    scan.arrays.push(extent);
                }
            }
            _ => {}
        }
        if !empty {
            depth += 1;
        }
    }
    Some(scan)
}

/// Whether a `<f>` element is an array formula (`t="array"`). Compares the
/// raw value so the many ordinary formulas cost no allocation.
fn is_array_formula(f: &BytesStart<'_>) -> bool {
    f.attributes()
        .filter_map(Result::ok)
        .any(|attr| attr.key.local_name().as_ref() == b"t" && attr.value.as_ref() == b"array")
}

fn is_true(value: Option<String>) -> bool {
    matches!(value.as_deref(), Some("1" | "true"))
}

/// First and last row of an A1 range such as `B2:H96`.
fn row_span(reference: &str) -> Option<(u32, u32)> {
    let (first, _, last, _) = cell_rect(reference)?;
    Some((first, last))
}

/// The cells of an A1 range such as `B2:H96` or `C4`.
fn cell_rect(reference: &str) -> Option<CellRect> {
    let (start, end) = reference.split_once(':').unwrap_or((reference, reference));
    let cell = |cell: &str| formualizer_common::coord::parse_a1_1based(cell.trim()).ok();
    let ((r1, c1, ..), (r2, c2, ..)) = (cell(start)?, cell(end)?);
    Some((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in parts {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const WORKBOOK: &str = r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    const WORKBOOK_RELS: &str = r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;

    fn scan(sheet: &str, extra: &[(&str, &str)], databases: &[(&str, u32, u32)]) -> HiddenRows {
        let mut parts = vec![
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", sheet),
        ];
        parts.extend_from_slice(extra);
        let databases = databases
            .iter()
            .map(|&(sheet, first, last)| (sheet.to_string(), (first, last)))
            .collect();
        scan_sheets(Cursor::new(package(&parts)), &databases)
            .remove("Data")
            .unwrap_or_default()
            .hidden_rows
    }

    /// Rows 1-7, with rows 1 (the filter header), 3, 4 and 7 hidden.
    fn sheet(sheet_pr: &str, after_data: &str) -> String {
        format!(
            r#"<worksheet>{sheet_pr}<sheetData><row r="1" hidden="1"/><row r="2"/><row r="3" hidden="1"/><row r="4" hidden="true"/><row r="5"/><row r="7" hidden="1"/></sheetData>{after_data}</worksheet>"#
        )
    }

    const CRITERIA: &str = r#"<autoFilter ref="A1:B5"><filterColumn colId="0"><filters><filter val="x"/></filters></filterColumn></autoFilter>"#;

    #[test]
    fn rows_below_the_header_of_an_applied_autofilter_are_filter_hidden() {
        let rows = scan(&sheet(r#"<sheetPr filterMode="1"/>"#, CRITERIA), &[], &[]);
        assert_eq!(rows.filter, vec![3, 4]);
        assert_eq!(rows.manual, vec![1, 7]);
    }

    #[test]
    fn criteria_alone_or_filter_mode_alone_apply_the_autofilter() {
        let rows = scan(&sheet("", CRITERIA), &[], &[]);
        assert_eq!(rows.filter, vec![3, 4]);
        let rows = scan(
            &sheet(
                r#"<sheetPr filterMode="1"/>"#,
                r#"<autoFilter ref="A1:B5"/>"#,
            ),
            &[],
            &[],
        );
        assert_eq!(rows.filter, vec![3, 4]);
    }

    #[test]
    fn hidden_rows_are_manual_without_an_applied_filter() {
        for after_data in [
            "",
            r#"<autoFilter ref="A1:B5"/>"#,
            r#"<autoFilter ref="A1:B5"><filterColumn colId="0" hiddenButton="1"/></autoFilter>"#,
            r#"<customSheetViews><customSheetView><autoFilter ref="A1:B5"><filterColumn colId="0"><filters><filter val="x"/></filters></filterColumn></autoFilter></customSheetView></customSheetViews>"#,
        ] {
            let rows = scan(&sheet("", after_data), &[], &[]);
            assert_eq!(rows.manual, vec![1, 3, 4, 7], "{after_data}");
            assert!(rows.filter.is_empty(), "{after_data}");
        }
    }

    #[test]
    fn advanced_filter_uses_the_filter_database_range() {
        let rows = scan(
            &sheet(r#"<sheetPr filterMode="1"/>"#, ""),
            &[],
            &[("Data", 1, 3)],
        );
        assert_eq!(rows.filter, vec![3]);
        assert_eq!(rows.manual, vec![1, 4, 7]);
        // Without filter mode the name is only a leftover.
        let rows = scan(&sheet("", ""), &[], &[("Data", 1, 3)]);
        assert!(rows.filter.is_empty());
    }

    #[test]
    fn table_autofilter_with_criteria_is_applied() {
        let sheet_rels = r#"<Relationships><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/></Relationships>"#;
        let table = |filter: &str| {
            format!(r#"<table ref="A2:B6"><autoFilter ref="A2:B5">{filter}</autoFilter></table>"#)
        };
        let filtered = table(
            r#"<filterColumn colId="1"><customFilters><customFilter val="1"/></customFilters></filterColumn>"#,
        );
        let sheet_xml = sheet(
            "",
            r#"<tableParts count="1"><tablePart r:id="rId3"/></tableParts>"#,
        );
        let rows = scan(
            &sheet_xml,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels),
                ("xl/tables/table1.xml", &filtered),
            ],
            &[],
        );
        assert_eq!(rows.filter, vec![3, 4]);
        assert_eq!(rows.manual, vec![1, 7]);

        let unfiltered = table(r#"<filterColumn colId="1" hiddenButton="1"/>"#);
        let rows = scan(
            &sheet_xml,
            &[
                ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels),
                ("xl/tables/table1.xml", &unfiltered),
            ],
            &[],
        );
        assert!(rows.filter.is_empty());
    }

    #[test]
    fn rows_without_r_follow_the_previous_row() {
        let xml = r#"<worksheet><sheetPr filterMode="1"/><sheetData><row r="2"/><row hidden="1"/><row/><row hidden="1"/></sheetData><autoFilter ref="A1:A4"/></worksheet>"#;
        let rows = scan(xml, &[], &[]);
        assert_eq!(rows.filter, vec![3]);
        assert_eq!(rows.manual, vec![5]);
    }

    #[test]
    fn rows_without_r_after_the_largest_r_do_not_overflow() {
        // r is 1..=1048576 in a valid file; a row without r after
        // r="4294967295" used to overflow computing the next row number.
        let xml = r#"<worksheet><sheetData><row r="4294967295"/><row hidden="1"/><row hidden="1"/><row r="3" hidden="1"/><row hidden="1"/></sheetData></worksheet>"#;
        let rows = scan(xml, &[], &[]);
        assert_eq!(rows.manual, vec![3, 4]);
        assert!(rows.filter.is_empty());

        // Out-of-range rows are skipped even when they are hidden.
        let xml = r#"<worksheet><sheetData><row r="1048576" hidden="1"/><row hidden="1"/></sheetData></worksheet>"#;
        let rows = scan(xml, &[], &[]);
        assert_eq!(rows.manual, vec![1_048_576]);
    }

    #[test]
    fn array_formula_extents_are_recorded_for_every_sheet_that_has_one() {
        // A legacy array (B1:B3), a dynamic array (cm, D1:E2) and a one-cell
        // array with an absolute ref, then formulas that are not arrays: a
        // shared formula and a data table carry a ref too.
        let xml = r#"<worksheet><sheetData><row r="1"><c r="B1"><f t="array" ref="B1:B3">C1:C3</f><v>1</v></c><c r="D1" cm="1"><f t="array" ref="D1:E2">C1:D2</f></c><c r="G1"><f t="array" ref="$G$1">C1:C3</f></c></row><row r="2"><c r="J2"><f t="shared" ref="J2:J4" si="0">C2</f></c><c r="K2"><f t="dataTable" ref="K2:K3" r1="A1"/></c><c r="L2"><f>C2</f></c></row></sheetData></worksheet>"#;
        let mut parts = vec![
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", xml),
        ];
        let scans = scan_sheets(Cursor::new(package(&parts)), &HashMap::new());
        let data = &scans["Data"];
        assert_eq!(
            data.array_extents,
            vec![(1, 2, 3, 2), (1, 4, 2, 5), (1, 7, 1, 7)]
        );
        assert_eq!(data.hidden_rows, HiddenRows::default());

        // A sheet without arrays or hidden rows has no entry.
        parts[2].1 = r#"<worksheet><sheetData><row r="1"><c r="A1"><f>1</f></c></row></sheetData></worksheet>"#;
        assert!(scan_sheets(Cursor::new(package(&parts)), &HashMap::new()).is_empty());

        // Hidden rows and arrays of one sheet come back together.
        parts[2].1 = r#"<worksheet><sheetData><row r="1"><c r="A1"><f t="array" ref="A1:A2">B1:B2</f></c></row><row r="3" hidden="1"/></sheetData></worksheet>"#;
        let scans = scan_sheets(Cursor::new(package(&parts)), &HashMap::new());
        assert_eq!(scans["Data"].array_extents, vec![(1, 1, 2, 1)]);
        assert_eq!(scans["Data"].hidden_rows.manual, vec![3]);
    }
}
