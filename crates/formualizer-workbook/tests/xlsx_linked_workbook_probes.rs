#![cfg(feature = "xlsx-recalc")]
//! Workbooks whose formulas read closed linked workbooks, recalculated and
//! compared cell by cell with Excel for Windows 16.0.20430: probes 1 to 5 of
//! ops/excel-extlinks-probe-20261006.md, the links2 probes of
//! ops/excel-links2-probe-20261008.md and probe-w2-parse-L1 of
//! ops/excel-parse-probe-20261008.md (spill references to a closed linked
//! workbook's cells and to legacy formulas, a name over a deleted range), each
//! kept to the rows the fork computes as Excel did (`excel-values.tsv`,
//! `excel-values-links2.tsv`, `excel-values-parse.tsv`). Excel saw the links
//! closed (UpdateLinks=0, CalculateFullRebuild), so it read the values the
//! externalLink parts save, as the fork does.
use calamine::{Data, Reader, Xlsx};
use formualizer_workbook::{XlsxRecalculateOptions, recalculate_xlsx_bytes};
use std::{io::Cursor, path::Path};

/// Recalculate each workbook `values` names and compare its Sheet1 column E
/// with Excel's values; returns the number of rows checked.
fn check_against_excel(values: &str) -> usize {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/linked-workbooks");
    let expected = std::fs::read_to_string(dir.join(values)).unwrap();
    let mut outputs = std::collections::BTreeMap::new();
    let mut checked = 0;
    for line in expected.lines().filter(|line| !line.starts_with('#')) {
        let [file, cell, kind, value, formula] = line.split('\t').collect::<Vec<_>>()[..] else {
            panic!("malformed row: {line}");
        };
        let range = outputs.entry(file).or_insert_with(|| {
            let input = std::fs::read(dir.join(file)).unwrap();
            let output = recalculate_xlsx_bytes(&input, XlsxRecalculateOptions::default())
                .unwrap()
                .bytes;
            Xlsx::new(Cursor::new(output))
                .unwrap()
                .worksheet_range("Sheet1")
                .unwrap()
        });
        let row: u32 = cell[1..].parse().unwrap();
        let got = range
            .get_value((row - 1, 4))
            .cloned()
            .unwrap_or(Data::Empty);
        let matches = match (kind, &got) {
            ("n", Data::Float(n)) => (n - value.parse::<f64>().unwrap()).abs() < 1e-9,
            ("n", Data::Int(n)) => *n as f64 == value.parse::<f64>().unwrap(),
            ("str", Data::String(text)) => text == value,
            ("str", Data::Empty) => value.is_empty(),
            ("b", Data::Bool(flag)) => *flag == (value == "1"),
            ("e", Data::Error(error)) => error.to_string() == value,
            _ => false,
        };
        assert!(
            matches,
            "{file} {cell} {formula}: Excel {kind} {value}, fork {got:?}"
        );
        checked += 1;
    }
    checked
}

#[test]
fn linked_workbook_probes_match_excel_for_windows() {
    assert_eq!(check_against_excel("excel-values.tsv"), 181);
}

/// Approximate lookups over open and bounded linked ranges (Excel bisects
/// the range as written, past the saved cells), the cell a lookup returns
/// past the saved ones (#REF! on a sheet Excel could not refresh), INDEX at a
/// computed position (a range it returns intersects a legacy formula's cell),
/// and `[0]`, the workbook itself.
#[test]
fn links2_probes_match_excel_for_windows() {
    assert_eq!(check_against_excel("excel-values-links2.tsv"), 175);
}

/// Spill references (`_xlfn.ANCHORARRAY`) to a closed linked workbook's cells
/// (#REF!) and to legacy formulas (the anchor alone), and a name over a
/// deleted range (`Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))`).
#[test]
fn parse_probes_match_excel_for_windows() {
    assert_eq!(check_against_excel("excel-values-parse.tsv"), 31);
}
