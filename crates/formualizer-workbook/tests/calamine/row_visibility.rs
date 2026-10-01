use crate::common::build_workbook;
use formualizer_eval::engine::ingest::EngineLoadStream;
use formualizer_eval::engine::{Engine, EvalConfig, RowVisibilitySource};
use formualizer_workbook::{
    CalamineAdapter, LiteralValue, LoadStrategy, SpreadsheetReader, Workbook, WorkbookConfig,
};
use std::io::{Cursor, Write};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[test]
fn calamine_hidden_rows_load_as_manually_hidden() {
    let path = build_workbook(|book| {
        let sheet = book.get_sheet_by_name_mut("Sheet1").unwrap();
        sheet.get_cell_mut((1, 1)).set_value_number(1.0);
        sheet.get_row_dimension_mut(&3).set_hidden(true);
        sheet.get_row_dimension_mut(&4).set_hidden(true);
    });

    let mut adapter = CalamineAdapter::open_path(&path).expect("open xlsx");
    let sheet = adapter.read_sheet("Sheet1").expect("read sheet");

    assert_eq!(sheet.row_hidden_manual, vec![3, 4]);
    assert!(sheet.row_hidden_filter.is_empty());

    let ctx = formualizer_eval::test_workbook::TestWorkbook::new();
    let mut engine: Engine<_> = Engine::new(ctx, EvalConfig::default());
    adapter
        .stream_into_engine(&mut engine)
        .expect("stream into engine");

    assert_eq!(
        engine.is_row_hidden("Sheet1", 3, Some(RowVisibilitySource::Manual)),
        Some(true)
    );
    assert_eq!(
        engine.is_row_hidden("Sheet1", 3, Some(RowVisibilitySource::Filter)),
        Some(false)
    );
    assert_eq!(
        engine.is_row_hidden("Sheet1", 2, Some(RowVisibilitySource::Manual)),
        Some(false)
    );
}

/// One sheet "Data": header in A1, 10/20/30/40 in A2:A5 and 100 in A7.
/// Rows 3, 4 and 7 are hidden; the formulas sit in row 9.
fn filtered_xlsx(sheet_pr: &str, after_data: &str) -> Vec<u8> {
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{sheet_pr}<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>h</t></is></c></row><row r="2"><c r="A2"><v>10</v></c></row><row r="3" hidden="1"><c r="A3"><v>20</v></c></row><row r="4" hidden="1"><c r="A4"><v>30</v></c></row><row r="5"><c r="A5"><v>40</v></c></row><row r="7" hidden="1"><c r="A7"><v>100</v></c></row><row r="9"><c r="A9"><f>SUBTOTAL(9,A2:A7)</f></c><c r="B9"><f>SUBTOTAL(109,A2:A7)</f></c><c r="C9"><f>SUBTOTAL(3,A1:A7)</f></c><c r="D9"><f>SUM(A2:A7)</f></c></row></sheetData>{after_data}</worksheet>"#
    );
    let parts = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
    ];
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, xml) in parts {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn evaluate_row_9(bytes: Vec<u8>) -> Vec<f64> {
    let adapter = CalamineAdapter::open_bytes(bytes).expect("open xlsx");
    let mut wb =
        Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
            .expect("load workbook");
    wb.evaluate_all().expect("evaluate");
    (1..=4)
        .map(|col| match wb.get_value("Data", 9, col) {
            Some(LiteralValue::Number(n)) => n,
            Some(LiteralValue::Int(i)) => i as f64,
            other => panic!("expected a number in column {col}, got {other:?}"),
        })
        .collect()
}

#[test]
fn calamine_autofilter_hidden_rows_are_skipped_by_every_subtotal() {
    let bytes = filtered_xlsx(
        r#"<sheetPr filterMode="1"/>"#,
        r#"<autoFilter ref="A1:A5"><filterColumn colId="0"><filters><filter val="10"/><filter val="40"/></filters></filterColumn></autoFilter>"#,
    );

    let mut adapter = CalamineAdapter::open_bytes(bytes.clone()).expect("open xlsx");
    let sheet = adapter.read_sheet("Data").expect("read sheet");
    assert_eq!(sheet.row_hidden_filter, vec![3, 4]);
    assert_eq!(sheet.row_hidden_manual, vec![7]);

    // SUBTOTAL(9) keeps the manually hidden 100 but not the filtered 20 and
    // 30; SUBTOTAL(109) drops all three; SUM ignores visibility.
    assert_eq!(evaluate_row_9(bytes), vec![150.0, 50.0, 4.0, 200.0]);
}

#[test]
fn calamine_hidden_rows_under_an_unfiltered_autofilter_stay_manual() {
    let bytes = filtered_xlsx("", r#"<autoFilter ref="A1:A5"/>"#);
    assert_eq!(evaluate_row_9(bytes), vec![200.0, 50.0, 6.0, 200.0]);
}
