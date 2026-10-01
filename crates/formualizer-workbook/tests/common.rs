//! Shared test utilities for formualizer-workbook integration tests.
//!
//! Re-exported from `formualizer-testkit` so fixture helpers are reusable
//! across tests, benches, and future benchmark corpus generation.

#![allow(unused_imports)]
#![allow(dead_code)]

pub use formualizer_testkit::{build_numeric_grid, build_standard_grid, build_workbook};

/// Assertions shared by the issue #332 "phantom default sheet" load
/// regression tests across the calamine, umya and json backends.
pub mod sheet_load {
    use formualizer_eval::engine::Engine;
    use formualizer_eval::traits::EvaluationContext;
    use formualizer_parse::parser::parse;

    /// Sheet names in Arrow-store order — the same list `Workbook::sheet_names()`
    /// (and the Python/WASM bindings) expose.
    pub fn sheet_names<R: EvaluationContext>(engine: &Engine<R>) -> Vec<String> {
        engine
            .sheet_store()
            .sheets
            .iter()
            .map(|s| s.name.as_ref().to_string())
            .collect()
    }

    /// Assert the loaded engine's sheets are exactly `expected`, in that order,
    /// and that the three views of "which sheet is where" all agree:
    /// the Arrow store, the graph sheet registry, and `SHEET()`/`SHEETS()`.
    ///
    /// Scratch formulas are written far outside any fixture's data region.
    pub fn assert_sheet_layout<R: EvaluationContext>(engine: &mut Engine<R>, expected: &[&str]) {
        let names = sheet_names(engine);
        assert_eq!(
            names,
            expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "Arrow-store sheet list"
        );

        // Graph sheet registry ids must follow the same order, starting at 0.
        for (idx, name) in expected.iter().enumerate() {
            assert_eq!(
                engine.sheet_id(name),
                Some(idx as u16),
                "graph sheet id for {name}"
            );
        }

        for name in expected {
            engine
                .set_cell_formula(name, 900, 20, parse("=SHEET()").unwrap())
                .unwrap();
            engine
                .set_cell_formula(name, 901, 20, parse("=SHEETS()").unwrap())
                .unwrap();
        }
        engine.evaluate_all().unwrap();
        for (idx, name) in expected.iter().enumerate() {
            assert_eq!(
                engine.get_cell_value(name, 900, 20),
                Some(formualizer_common::LiteralValue::Number((idx + 1) as f64)),
                "SHEET() on {name}"
            );
            assert_eq!(
                engine.get_cell_value(name, 901, 20),
                Some(formualizer_common::LiteralValue::Number(
                    expected.len() as f64
                )),
                "SHEETS() on {name}"
            );
        }
    }
}

/// Saved filter states written as raw package XML, so the calamine and umya
/// backends read the same bytes and must split hidden rows the same way.
#[cfg(any(feature = "calamine", feature = "umya"))]
pub mod saved_filters {
    use std::io::{Cursor, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    /// One sheet "Data": header in A1, 10/20/30/40 in A2:A5 and 100 in A7.
    /// Rows 3, 4 and 7 are hidden; row 9 holds SUBTOTAL(9,A2:A7),
    /// SUBTOTAL(109,A2:A7), SUBTOTAL(3,A1:A7) and SUM(A2:A7).
    pub fn filtered_xlsx(sheet_pr: &str, after_data: &str) -> Vec<u8> {
        package(sheet_pr, after_data, "", &[])
    }

    /// `filtered_xlsx` with workbook `<definedNames>` content and extra parts.
    pub fn package(
        sheet_pr: &str,
        after_data: &str,
        defined_names: &str,
        extra: &[(&str, &str)],
    ) -> Vec<u8> {
        let sheet = format!(
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{sheet_pr}<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>h</t></is></c></row><row r="2"><c r="A2"><v>10</v></c></row><row r="3" hidden="1"><c r="A3"><v>20</v></c></row><row r="4" hidden="1"><c r="A4"><v>30</v></c></row><row r="5"><c r="A5"><v>40</v></c></row><row r="7" hidden="1"><c r="A7"><v>100</v></c></row><row r="9"><c r="A9"><f>SUBTOTAL(9,A2:A7)</f></c><c r="B9"><f>SUBTOTAL(109,A2:A7)</f></c><c r="C9"><f>SUBTOTAL(3,A1:A7)</f></c><c r="D9"><f>SUM(A2:A7)</f></c></row></sheetData>{after_data}</worksheet>"#
        );
        let workbook = format!(
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>{}</workbook>"#,
            if defined_names.is_empty() {
                String::new()
            } else {
                format!("<definedNames>{defined_names}</definedNames>")
            }
        );
        let mut parts = vec![
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            ("xl/workbook.xml", workbook.as_str()),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
            ),
            // umya requires a stylesheet.
            (
                "xl/styles.xml",
                r#"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#,
            ),
            ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ];
        parts.extend_from_slice(extra);
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in parts {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// A saved filter state and how each backend must load the sheet.
    pub struct Case {
        pub name: &'static str,
        pub bytes: Vec<u8>,
        pub filter: Vec<u32>,
        pub manual: Vec<u32>,
        /// SUBTOTAL(9), SUBTOTAL(109), SUBTOTAL(3) and SUM in row 9.
        pub row_9: Vec<f64>,
    }

    const CRITERIA: &str = r#"<autoFilter ref="A1:A5"><filterColumn colId="0"><filters><filter val="10"/><filter val="40"/></filters></filterColumn></autoFilter>"#;
    const FILTER_MODE: &str = r#"<sheetPr filterMode="1"/>"#;

    /// Rows 3 and 4 lie inside every applied filter below its header; row 7
    /// lies outside. Every other state leaves all three manual.
    pub fn cases() -> Vec<Case> {
        let filtered = |name, bytes| Case {
            name,
            bytes,
            filter: vec![3, 4],
            manual: vec![7],
            row_9: vec![150.0, 50.0, 4.0, 200.0],
        };
        let manual = |name, bytes| Case {
            name,
            bytes,
            filter: vec![],
            manual: vec![3, 4, 7],
            row_9: vec![200.0, 50.0, 6.0, 200.0],
        };
        let table = |criteria: &str| {
            let xml = format!(
                r#"<?xml version="1.0"?><table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="T" displayName="T" ref="A1:A5"><autoFilter ref="A1:A5">{criteria}</autoFilter><tableColumns count="1"><tableColumn id="1" name="h"/></tableColumns></table>"#
            );
            package(
                "",
                r#"<tableParts count="1"><tablePart r:id="rId1"/></tableParts>"#,
                "",
                &[
                    (
                        "xl/worksheets/_rels/sheet1.xml.rels",
                        r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/></Relationships>"#,
                    ),
                    ("xl/tables/table1.xml", xml.as_str()),
                ],
            )
        };
        vec![
            filtered(
                "autoFilter with criteria in filter mode",
                filtered_xlsx(FILTER_MODE, CRITERIA),
            ),
            filtered("autoFilter with criteria", filtered_xlsx("", CRITERIA)),
            filtered(
                "autoFilter in filter mode",
                filtered_xlsx(FILTER_MODE, r#"<autoFilter ref="A1:A5"/>"#),
            ),
            manual("no filter", filtered_xlsx("", "")),
            manual(
                "autoFilter without criteria",
                filtered_xlsx("", r#"<autoFilter ref="A1:A5"/>"#),
            ),
            manual(
                "autoFilter in a custom view only",
                filtered_xlsx(
                    "",
                    &format!(
                        "<customSheetViews><customSheetView guid=\"{{00000000-0000-0000-0000-000000000001}}\">{CRITERIA}</customSheetView></customSheetViews>"
                    ),
                ),
            ),
            filtered(
                "Advanced Filter in filter mode",
                package(
                    FILTER_MODE,
                    "",
                    r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$A$1:$A$5</definedName>"#,
                    &[],
                ),
            ),
            manual(
                "Advanced Filter name without filter mode",
                package(
                    "",
                    "",
                    r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$A$1:$A$5</definedName>"#,
                    &[],
                ),
            ),
            filtered(
                "table autoFilter with criteria",
                table(
                    r#"<filterColumn colId="0"><filters><filter val="10"/><filter val="40"/></filters></filterColumn>"#,
                ),
            ),
            manual("table autoFilter without criteria", table("")),
        ]
    }
}
