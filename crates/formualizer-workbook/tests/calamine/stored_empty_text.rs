// A cell that stores empty text (a shared string "") is text, not an empty
// cell: ISBLANK is FALSE and COUNTA counts it, as for a formula returning "".
use formualizer_common::{ExcelErrorKind, LiteralValue};
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
    xlsx(
        "<si><t/></si><si><t>x</t></si>",
        &format!("<dimension ref=\"A1:B201\"/><sheetData>{rows}</sheetData>"),
    )
}

/// A one-sheet package: `shared` is the shared-string items and `sheet` the
/// worksheet body (dimension and sheetData).
fn xlsx(shared: &str, sheet: &str) -> Vec<u8> {
    let parts = [
        ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/></Types>".to_owned()),
        ("_rels/.rels", format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")),
        ("xl/workbook.xml", format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{OFFICE}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")),
        ("xl/_rels/workbook.xml.rels", format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{OFFICE}/sharedStrings\" Target=\"sharedStrings.xml\"/></Relationships>")),
        ("xl/sharedStrings.xml", format!("<sst xmlns=\"{MAIN}\">{shared}</sst>")),
        ("xl/worksheets/sheet1.xml", format!("<worksheet xmlns=\"{MAIN}\">{sheet}</worksheet>")),
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

/// The members of an array formula's `ref` hold the anchor's result, as Excel
/// writes them: a cached value and no `<f>`. B1:B3 and H300:H302 cache "" in
/// every member (`<v></v>` and an empty shared string); E1:E3 is a dynamic
/// array whose members cache a stale number and "". G1 spills a plain formula
/// onto G2, which stores "" outside any array. Row 300 comes after a long gap,
/// so it and the rows below load through the sparse overlay.
fn array_member_xlsx() -> Vec<u8> {
    let rows = [
        r#"<row r="1"><c r="B1" t="str"><f t="array" ref="B1:B3">IF(C1:C3&gt;0,"","x")</f><v></v></c><c r="C1"><v>1</v></c><c r="E1" cm="1"><f t="array" ref="E1:E3">C1:C3*2</f><v>2</v></c><c r="G1"><f>C1:C3</f><v>1</v></c></row>"#,
        r#"<row r="2"><c r="B2" t="str"><v></v></c><c r="C2"><v>1</v></c><c r="E2"><v>99</v></c><c r="G2" t="s"><v>0</v></c></row>"#,
        r#"<row r="3"><c r="B3" t="str"><v></v></c><c r="C3"><v>1</v></c><c r="E3" t="s"><v>0</v></c></row>"#,
        r#"<row r="300"><c r="A300"><v>5</v></c><c r="H300" t="str"><f t="array" ref="H300:H302">IF(C1:C3&gt;0,"","x")</f><v></v></c></row>"#,
        r#"<row r="301"><c r="H301" t="str"><v></v></c></row>"#,
        r#"<row r="302"><c r="H302" t="s"><v>0</v></c></row>"#,
    ]
    .concat();
    xlsx(
        "<si><t/></si>",
        &format!("<dimension ref=\"A1:H302\"/><sheetData>{rows}</sheetData>"),
    )
}

#[test]
fn array_member_caches_do_not_block_the_anchor_spill() {
    let mut backend = CalamineAdapter::open_bytes(array_member_xlsx()).unwrap();
    let mut engine: Engine<_> = Engine::new(TestWorkbook::new(), EvalConfig::default());
    backend.stream_into_engine(&mut engine).unwrap();
    engine.evaluate_all().unwrap();
    let value = |row: u32, col: u32| match engine.get_cell_value("Sheet1", row, col) {
        Some(LiteralValue::Int(n)) => Some(LiteralValue::Number(n as f64)),
        other => other,
    };
    let empty_text = Some(LiteralValue::Text(String::new()));
    for (row, col) in [(1, 2), (2, 2), (3, 2), (300, 8), (301, 8), (302, 8)] {
        assert_eq!(value(row, col), empty_text, "R{row}C{col}");
    }
    for row in 1..=3 {
        assert_eq!(value(row, 5), Some(LiteralValue::Number(2.0)), "E{row}");
    }
    assert!(
        matches!(value(1, 7), Some(LiteralValue::Error(ref e)) if e.kind == ExcelErrorKind::Spill),
        "G1 = {:?}",
        value(1, 7)
    );
    assert_eq!(value(2, 7), empty_text);
}
