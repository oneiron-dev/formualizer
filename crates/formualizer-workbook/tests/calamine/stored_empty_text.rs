// A cell that stores empty text (a shared string "") is text, not an empty
// cell: ISBLANK is FALSE and COUNTA counts it, as for a formula returning "".
use formualizer_common::LiteralValue;
use formualizer_eval::engine::ingest::EngineLoadStream;
use formualizer_eval::engine::{Engine, EvalConfig};
use formualizer_eval::test_workbook::TestWorkbook;
use formualizer_workbook::{CalamineAdapter, SpreadsheetReader};
use std::io::{Cursor, Write};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A1, A200 and A201 hold the empty shared string, A2 is a styled cell with
/// no value, A3 is absent, A4 holds "x". Rows 200 and 201 come after a long
/// gap, so they load through the sparse overlay.
fn empty_text_xlsx() -> Vec<u8> {
    let formulas = [
        "ISBLANK(A1)",
        "ISBLANK(A2)",
        "ISBLANK(A3)",
        "ISTEXT(A1)",
        "COUNTA(A1:A4)",
        "COUNTBLANK(A1:A4)",
        "A1=&quot;&quot;",
        "LEN(A1)",
        "ISBLANK(A200)",
        "ISBLANK(A201)",
    ];
    let mut rows = String::new();
    for (i, formula) in formulas.iter().enumerate() {
        let r = i + 1;
        let a = match r {
            1 => "<c r=\"A1\" t=\"s\"><v>0</v></c>".to_owned(),
            2 => "<c r=\"A2\" s=\"0\"/>".to_owned(),
            4 => "<c r=\"A4\" t=\"s\"><v>1</v></c>".to_owned(),
            _ => String::new(),
        };
        rows.push_str(&format!(
            "<row r=\"{r}\">{a}<c r=\"B{r}\"><f>{formula}</f><v>0</v></c></row>"
        ));
    }
    rows.push_str("<row r=\"200\"><c r=\"A200\" t=\"s\"><v>0</v></c></row>");
    rows.push_str("<row r=\"201\"><c r=\"A201\" t=\"s\"><v>0</v></c></row>");
    let parts = [
        ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/></Types>".to_owned()),
        ("_rels/.rels", format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")),
        ("xl/workbook.xml", format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{OFFICE}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")),
        ("xl/_rels/workbook.xml.rels", format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{OFFICE}/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>")),
        ("xl/sharedStrings.xml", format!("<sst xmlns=\"{MAIN}\" count=\"4\" uniqueCount=\"2\"><si><t/></si><si><t>x</t></si></sst>")),
        ("xl/worksheets/sheet1.xml", format!("<worksheet xmlns=\"{MAIN}\"><dimension ref=\"A1:B201\"/><sheetData>{rows}</sheetData></worksheet>")),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in parts {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn stored_empty_text_loads_as_text_not_blank() {
    let mut backend = CalamineAdapter::open_bytes(empty_text_xlsx()).unwrap();
    let mut engine: Engine<_> = Engine::new(TestWorkbook::new(), EvalConfig::default());
    backend.stream_into_engine(&mut engine).unwrap();
    engine.evaluate_all().unwrap();
    for (row, value) in [
        (1, LiteralValue::Text(String::new())),
        (200, LiteralValue::Text(String::new())),
        (201, LiteralValue::Text(String::new())),
    ] {
        assert_eq!(
            engine.get_cell_value("Sheet1", row, 1),
            Some(value),
            "A{row}"
        );
    }
    assert!(matches!(
        engine.get_cell_value("Sheet1", 2, 1),
        None | Some(LiteralValue::Empty)
    ));
    let expected = [
        LiteralValue::Boolean(false),
        LiteralValue::Boolean(true),
        LiteralValue::Boolean(true),
        LiteralValue::Boolean(true),
        LiteralValue::Number(2.0),
        LiteralValue::Number(3.0),
        LiteralValue::Boolean(true),
        LiteralValue::Number(0.0),
        LiteralValue::Boolean(false),
        LiteralValue::Boolean(false),
    ];
    for (i, value) in expected.into_iter().enumerate() {
        let row = i as u32 + 1;
        let got = engine.get_cell_value("Sheet1", row, 2);
        let got = match got {
            Some(LiteralValue::Int(n)) => Some(LiteralValue::Number(n as f64)),
            other => other,
        };
        assert_eq!(got, Some(value), "B{row}");
    }
}

#[test]
fn read_sheet_keeps_stored_empty_text() {
    let mut backend = CalamineAdapter::open_bytes(empty_text_xlsx()).unwrap();
    let sheet = backend.read_sheet("Sheet1").unwrap();
    let value = |row: u32, col: u32| sheet.cells.get(&(row, col)).and_then(|c| c.value.clone());
    assert_eq!(value(1, 1), Some(LiteralValue::Text(String::new())));
    assert_eq!(value(200, 1), Some(LiteralValue::Text(String::new())));
    assert_eq!(value(2, 1), None);
    assert_eq!(value(3, 1), None);
    assert_eq!(value(4, 1), Some(LiteralValue::Text("x".into())));
}
