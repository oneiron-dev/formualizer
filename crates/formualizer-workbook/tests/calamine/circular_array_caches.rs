// With iterative calculation off, a formula on a circular reference keeps its
// last calculated value: loading a file through Calamine supplies the cached
// values of every cell of an array formula, not only of its anchor.
use formualizer_common::LiteralValue;
use formualizer_eval::engine::ingest::EngineLoadStream;
use formualizer_eval::engine::{CycleConfig, CycleDetection, CyclePolicy, Engine, EvalConfig};
use formualizer_eval::test_workbook::TestWorkbook;
use formualizer_workbook::{CalamineAdapter, SpreadsheetReader};
use std::io::{Cursor, Write};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A one-sheet package whose worksheet body (dimension and sheetData) is `sheet`.
fn xlsx(sheet: &str) -> Vec<u8> {
    let parts = [
        ("[Content_Types].xml", "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>".to_owned()),
        ("_rels/.rels", format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")),
        ("xl/workbook.xml", format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{OFFICE}\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")),
        ("xl/_rels/workbook.xml.rels", format!("<Relationships xmlns=\"{RELS}\"><Relationship Id=\"rId1\" Type=\"{OFFICE}/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>")),
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
fn a_circular_array_formula_keeps_the_cached_values_of_all_its_cells() {
    // A1:A2 reads its own cells: it keeps both caches (5 and 6), and B1
    // calculates from the second.
    let bytes = xlsx(
        "<dimension ref=\"A1:B2\"/><sheetData>\
         <row r=\"1\"><c r=\"A1\"><f t=\"array\" ref=\"A1:A2\">{1;2}+SUM(A1:A100)</f><v>5</v></c>\
         <c r=\"B1\"><f>A2*10</f><v>0</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>6</v></c></row></sheetData>",
    );
    let mut backend = CalamineAdapter::open_bytes(bytes).unwrap();
    let config = EvalConfig::default().with_cycle(CycleConfig {
        detection: CycleDetection::Runtime,
        policy: CyclePolicy::RetainLastValue,
    });
    let mut engine: Engine<_> = Engine::new(TestWorkbook::new(), config);
    backend.stream_into_engine(&mut engine).unwrap();
    engine.use_legacy_array_semantics();
    engine.declare_array_formula("Sheet1", 1, 1, 2, 1, false);
    engine.evaluate_all().unwrap();
    for (row, col, expected) in [(1, 1, 5.0), (2, 1, 6.0), (1, 2, 60.0)] {
        assert_eq!(
            engine.get_cell_value("Sheet1", row, col),
            Some(LiteralValue::Number(expected)),
            "row {row} col {col}"
        );
    }
}

#[test]
fn kept_array_member_caches_count_toward_the_load_budget() {
    // The caches of A2 and A3 are kept as populated cells: with A1's formula
    // they exceed a budget of two cells.
    let bytes = xlsx(
        "<dimension ref=\"A1:A3\"/><sheetData>\
         <row r=\"1\"><c r=\"A1\"><f t=\"array\" ref=\"A1:A3\">{1;2;3}+SUM(A1:A3)</f><v>5</v></c></row>\
         <row r=\"2\"><c r=\"A2\"><v>6</v></c></row>\
         <row r=\"3\"><c r=\"A3\"><v>7</v></c></row></sheetData>",
    );
    for (policy, loads) in [
        (CyclePolicy::RetainLastValue, false),
        (CyclePolicy::Error, true),
    ] {
        let mut backend = CalamineAdapter::open_bytes(bytes.clone()).unwrap();
        let config = EvalConfig::default().with_cycle(CycleConfig {
            detection: CycleDetection::Runtime,
            policy,
        });
        let mut engine: Engine<_> = Engine::new(TestWorkbook::new(), config);
        let mut limits = engine.workbook_load_limits().clone();
        limits.max_sheet_logical_cells = 2;
        engine.set_workbook_load_limits(limits);
        assert_eq!(
            backend.stream_into_engine(&mut engine).is_ok(),
            loads,
            "{policy:?}"
        );
    }
}
