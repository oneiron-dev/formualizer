#![cfg(feature = "xlsx-recalc")]
//! Workbooks whose formulas read where the workbook was opened from,
//! recalculated at that place and compared cell by cell with Excel for
//! Windows 16.0.20430: probes 1 to 3 of ops/excel-hostinfo-probe-20261008.md,
//! kept to the rows the fork computes as Excel did (`excel-values.tsv`).
//! Excel opened each workbook from `C:\oracle\jobs\<job>\corpus\...`, so its
//! CELL("filename") is that folder and file name before the sheet's name, and
//! its CELL("address") of another sheet's cell names that file. The rows
//! cover sheet and file names with spaces, quotes, digits, punctuation and
//! characters beyond ASCII, the reference's sheet against the formula's,
//! defined names, the usual MID/FIND/RIGHT/LEFT/TEXTAFTER extractions, and
//! the INFO types that do not read the host.
use calamine::{Data, Reader, Xlsx};
use formualizer_workbook::{XlsxRecalculateOptions, recalculate_xlsx_bytes};
use std::collections::BTreeMap;
use std::{io::Cursor, path::Path};

#[test]
fn host_information_probes_match_excel_for_windows() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hostinfo");
    let expected = std::fs::read_to_string(dir.join("excel-values.tsv")).unwrap();
    let mut outputs = BTreeMap::new();
    let mut misses = Vec::new();
    let mut checked = 0;
    for line in expected.lines().filter(|line| !line.starts_with('#')) {
        let [file, folder, name, sheet, cell, kind, value, formula] =
            line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("malformed row: {line}");
        };
        let output = outputs.entry(file).or_insert_with(|| {
            let input = std::fs::read(dir.join(file)).unwrap();
            let mut options = XlsxRecalculateOptions::default();
            options.eval_config.workbook_file_name = Some(name.to_owned());
            options.eval_config.workbook_directory = Some(folder.to_owned());
            let output = recalculate_xlsx_bytes(&input, options).unwrap().bytes;
            Xlsx::new(Cursor::new(output)).unwrap()
        });
        let range = output.worksheet_range(sheet).unwrap();
        let (row, col) = position(cell);
        let got = range
            .get_value((row - 1, col - 1))
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
        if !matches {
            misses.push(format!(
                "{file} {sheet}!{cell} {formula}: Excel {kind} {value}, fork {got:?}"
            ));
        }
        checked += 1;
    }
    assert!(
        misses.is_empty(),
        "{} misses:\n{}",
        misses.len(),
        misses.join("\n")
    );
    assert_eq!(checked, 166);
}

/// The 1-based row and column of an A1 cell address.
fn position(cell: &str) -> (u32, u32) {
    let split = cell.find(|c: char| c.is_ascii_digit()).unwrap();
    let col = cell[..split]
        .bytes()
        .fold(0, |col, letter| col * 26 + u32::from(letter - b'A') + 1);
    (cell[split..].parse().unwrap(), col)
}
