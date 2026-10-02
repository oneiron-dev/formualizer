use std::sync::Arc;

use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

use crate::engine::{
    Engine, EvalConfig, FormulaIngestBatch, FormulaIngestRecord, FormulaPlaneMode,
};
use crate::test_workbook::TestWorkbook;

#[derive(Clone, Copy, Debug)]
enum Expected {
    Number(f64),
    Boolean(bool),
    Text(&'static str),
}

#[derive(Clone, Copy, Debug)]
struct OracleCase {
    row: u32,
    formula: &'static str,
    oracle: &'static str,
    expected: Expected,
}

const ORACLE_CASES: &[OracleCase] = &[
    OracleCase {
        row: 1,
        formula: "=ISBLANK(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Boolean(true),
    },
    OracleCase {
        row: 2,
        formula: "=C1&\"x\"",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Text("x"),
    },
    OracleCase {
        row: 3,
        formula: "=LEN(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 4,
        formula: "=C1=0",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Boolean(true),
    },
    OracleCase {
        row: 5,
        formula: "=C1=\"\"",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Boolean(true),
    },
    OracleCase {
        row: 6,
        formula: "=T(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Text(""),
    },
    OracleCase {
        row: 7,
        formula: "=N(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 8,
        formula: "=ISNUMBER(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Boolean(false),
    },
    OracleCase {
        row: 9,
        formula: "=COUNT(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 10,
        formula: "=COUNTA(C1)",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 11,
        formula: "=TEXT(C1,\"0\")",
        oracle: "oracle: lo-verified must-not-change",
        expected: Expected::Text("0"),
    },
    OracleCase {
        row: 12,
        formula: "=C1",
        oracle: "oracle: lo-verified must-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 13,
        formula: "=+C1",
        oracle: "oracle: lo-verified must-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 14,
        formula: "=IF(TRUE,C1,5)",
        oracle: "oracle: lo-verified must-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 15,
        formula: "=IFERROR(C1,5)",
        oracle: "oracle: lo-verified must-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 16,
        formula: "=CHOOSE(1,C1)",
        oracle: "oracle: lo-verified must-change",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 17,
        formula: "=C1",
        oracle: "oracle: lo-verified chain producer",
        expected: Expected::Number(0.0),
    },
    OracleCase {
        row: 18,
        formula: "=ISBLANK(A17)",
        oracle: "oracle: lo-verified chain consumer",
        expected: Expected::Boolean(false),
    },
    OracleCase {
        row: 19,
        formula: "=COUNT(A17)",
        oracle: "oracle: lo-verified chain consumer",
        expected: Expected::Number(1.0),
    },
    OracleCase {
        row: 20,
        formula: "=COUNTA(A17)",
        oracle: "oracle: lo-verified chain consumer",
        expected: Expected::Number(1.0),
    },
];

fn expected_literal(expected: Expected) -> LiteralValue {
    match expected {
        Expected::Number(value) => LiteralValue::Number(value),
        Expected::Boolean(value) => LiteralValue::Boolean(value),
        Expected::Text(value) => LiteralValue::Text(value.to_string()),
    }
}

#[test]
fn blank_formula_result_oracle_table() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for case in ORACLE_CASES {
        engine
            .set_cell_formula("Sheet1", case.row, 1, parse(case.formula).unwrap())
            .unwrap();
    }

    engine.evaluate_all().unwrap();

    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 3),
        None,
        "C1 must remain a truly blank stored cell"
    );
    for case in ORACLE_CASES {
        assert_eq!(
            engine.get_cell_value("Sheet1", case.row, 1),
            Some(expected_literal(case.expected)),
            "{} ({})",
            case.formula,
            case.oracle
        );
    }
}

#[derive(Clone, Copy)]
struct PlaneCase {
    col: u32,
    formula: fn(u32) -> String,
    expected: Expected,
}

fn row_formula(template: &str, row: u32) -> String {
    template.replace("{row}", &row.to_string())
}

fn build_plane_engine(mode: FormulaPlaneMode) -> Engine<TestWorkbook> {
    let cases = plane_cases();
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig::default().with_formula_plane_mode(mode),
    );
    let mut records = Vec::new();
    for case in &cases {
        for row in 1..=20 {
            let formula = (case.formula)(row);
            let ast = parse(&formula).unwrap();
            let ast_id = engine.intern_formula_ast(&ast);
            records.push(FormulaIngestRecord::new(
                row,
                case.col,
                ast_id,
                Some(Arc::<str>::from(formula)),
            ));
        }
    }
    engine
        .ingest_formula_batches(vec![FormulaIngestBatch::new("Sheet1", records)])
        .unwrap();
    engine.evaluate_all().unwrap();
    engine
}

fn plane_cases() -> [PlaneCase; 20] {
    [
        PlaneCase {
            col: 4,
            formula: |row| row_formula("=ISBLANK(C{row})", row),
            expected: Expected::Boolean(true),
        },
        PlaneCase {
            col: 5,
            formula: |row| row_formula("=C{row}&\"x\"", row),
            expected: Expected::Text("x"),
        },
        PlaneCase {
            col: 6,
            formula: |row| row_formula("=LEN(C{row})", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 7,
            formula: |row| row_formula("=C{row}=0", row),
            expected: Expected::Boolean(true),
        },
        PlaneCase {
            col: 8,
            formula: |row| row_formula("=C{row}=\"\"", row),
            expected: Expected::Boolean(true),
        },
        PlaneCase {
            col: 9,
            formula: |row| row_formula("=T(C{row})", row),
            expected: Expected::Text(""),
        },
        PlaneCase {
            col: 10,
            formula: |row| row_formula("=N(C{row})", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 11,
            formula: |row| row_formula("=ISNUMBER(C{row})", row),
            expected: Expected::Boolean(false),
        },
        PlaneCase {
            col: 12,
            formula: |row| row_formula("=COUNT(C{row})", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 13,
            formula: |row| row_formula("=COUNTA(C{row})", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 14,
            formula: |row| row_formula("=TEXT(C{row},\"0\")", row),
            expected: Expected::Text("0"),
        },
        PlaneCase {
            col: 15,
            formula: |row| row_formula("=C{row}", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 16,
            formula: |row| row_formula("=+C{row}", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 17,
            formula: |row| row_formula("=IF(TRUE,C{row},5)", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 18,
            formula: |row| row_formula("=IFERROR(C{row},5)", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 19,
            formula: |row| row_formula("=CHOOSE(1,C{row})", row),
            expected: Expected::Number(0.0),
        },
        PlaneCase {
            col: 20,
            formula: |row| row_formula("=ISBLANK(O{row})", row),
            expected: Expected::Boolean(false),
        },
        PlaneCase {
            col: 21,
            formula: |row| row_formula("=COUNT(O{row})", row),
            expected: Expected::Number(1.0),
        },
        PlaneCase {
            col: 22,
            formula: |row| row_formula("=COUNTA(O{row})", row),
            expected: Expected::Number(1.0),
        },
        PlaneCase {
            col: 23,
            formula: |_row| "=$C$1".to_string(),
            expected: Expected::Number(0.0),
        },
    ]
}

#[test]
fn formula_plane_blank_result_values_match_off() {
    let off = build_plane_engine(FormulaPlaneMode::Off);
    let authoritative = build_plane_engine(FormulaPlaneMode::AuthoritativeExperimental);
    assert!(
        authoritative
            .baseline_stats()
            .formula_plane_active_span_count
            > 0
    );

    for case in plane_cases() {
        for row in 1..=20 {
            let expected = Some(expected_literal(case.expected));
            assert_eq!(off.get_cell_value("Sheet1", row, case.col), expected);
            assert_eq!(
                authoritative.get_cell_value("Sheet1", row, case.col),
                off.get_cell_value("Sheet1", row, case.col),
                "FormulaPlane mismatch at ({row}, {})",
                case.col
            );
        }
    }
}

#[test]
fn blank_elements_in_spilled_formula_results_finalize_to_zero() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=C1:C2").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();

    for row in 1..=2 {
        assert_eq!(
            engine.get_cell_value("Sheet1", row, 1),
            Some(LiteralValue::Number(0.0))
        );
        assert_eq!(engine.get_cell_value("Sheet1", row, 3), None);
    }
}

#[test]
fn lookup_blank_targets_stay_empty_until_published() {
    // Excel's VLOOKUP and HLOOKUP return an empty target cell as an empty value:
    // the formula cell shows 0, but VLOOKUP(..)&"" is "", and an array
    // col_index_num leaves the empties out of MEDIAN and COUNT. A real 0 target
    // still counts as 0.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let text = |s: &str| LiteralValue::Text(s.to_string());
    let number = LiteralValue::Number;
    // D1:F3 = A 0.25 0.5 / C <blank> <blank> / Z 0 0; D5:F7 is its transpose.
    for (row, key, b, c) in [
        (1, "A", Some(0.25), Some(0.5)),
        (2, "C", None, None),
        (3, "Z", Some(0.0), Some(0.0)),
    ] {
        engine.set_cell_value("Sheet1", row, 4, text(key)).unwrap();
        engine
            .set_cell_value("Sheet1", 5, 3 + row, text(key))
            .unwrap();
        if let (Some(b), Some(c)) = (b, c) {
            engine.set_cell_value("Sheet1", row, 5, number(b)).unwrap();
            engine.set_cell_value("Sheet1", row, 6, number(c)).unwrap();
            engine
                .set_cell_value("Sheet1", 6, 3 + row, number(b))
                .unwrap();
            engine
                .set_cell_value("Sheet1", 7, 3 + row, number(c))
                .unwrap();
        }
    }
    engine.set_cell_value("Sheet1", 1, 8, number(0.44)).unwrap();

    let cases: &[(&str, Expected)] = &[
        ("=VLOOKUP(\"C\",D1:F3,2,FALSE)", Expected::Number(0.0)),
        ("=VLOOKUP(\"C\",D1:F3,2,FALSE)&\"\"", Expected::Text("")),
        (
            "=VLOOKUP(\"C\",D1:F3,2,FALSE)=\"\"",
            Expected::Boolean(true),
        ),
        ("=VLOOKUP(\"C\",D1:F3,2,FALSE)=0", Expected::Boolean(true)),
        ("=LEN(VLOOKUP(\"C\",D1:F3,2,FALSE))", Expected::Number(0.0)),
        (
            "=MEDIAN(H1,VLOOKUP(\"C\",D1:F3,{2,3},FALSE))",
            Expected::Number(0.44),
        ),
        (
            "=IF(MEDIAN(H1,VLOOKUP(\"C\",D1:F3,{2,3},0))=H1,\"Pass\",\"Fail\")",
            Expected::Text("Pass"),
        ),
        (
            "=COUNT(VLOOKUP(\"C\",D1:F3,{2,3},FALSE))",
            Expected::Number(0.0),
        ),
        ("=HLOOKUP(\"C\",D5:F7,2,FALSE)", Expected::Number(0.0)),
        ("=HLOOKUP(\"C\",D5:F7,3,FALSE)&\"\"", Expected::Text("")),
        (
            "=MEDIAN(H1,HLOOKUP(\"C\",D5:F7,{2;3},FALSE))",
            Expected::Number(0.44),
        ),
        // Unchanged: real zeros and numbers.
        ("=VLOOKUP(\"Z\",D1:F3,2,FALSE)&\"\"", Expected::Text("0")),
        (
            "=MEDIAN(H1,VLOOKUP(\"Z\",D1:F3,{2,3},FALSE))",
            Expected::Number(0.0),
        ),
        (
            "=COUNT(VLOOKUP(\"Z\",D1:F3,{2,3},FALSE))",
            Expected::Number(2.0),
        ),
        (
            "=SUM(VLOOKUP(\"A\",D1:F3,{2,3},FALSE))",
            Expected::Number(0.75),
        ),
        ("=HLOOKUP(\"Z\",D5:F7,2,FALSE)&\"\"", Expected::Text("0")),
    ];
    for (i, (formula, _)) in cases.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", 10 + i as u32, 1, parse(formula).unwrap())
            .unwrap();
    }
    // A spilled lookup of empty targets still publishes zeros.
    engine
        .set_cell_formula(
            "Sheet1",
            40,
            1,
            parse("=VLOOKUP(\"C\",D1:F3,{2,3},FALSE)").unwrap(),
        )
        .unwrap();
    engine.evaluate_all().unwrap();

    for (i, (formula, expected)) in cases.iter().enumerate() {
        assert_eq!(
            engine.get_cell_value("Sheet1", 10 + i as u32, 1),
            Some(expected_literal(*expected)),
            "{formula}"
        );
    }
    for col in 1..=2 {
        assert_eq!(
            engine.get_cell_value("Sheet1", 40, col),
            Some(LiteralValue::Number(0.0))
        );
    }
}

#[test]
fn value_and_numbervalue_read_a_blank_as_zero() {
    // Excel's VALUE of an empty cell is 0, and so is VALUE of the empty target
    // a VLOOKUP returns, while VALUE("") is #VALUE!. Microsoft documents
    // NUMBERVALUE("") as 0 (spaces are ignored); a blank is 0 there too.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let text = |s: &str| LiteralValue::Text(s.to_string());
    engine.set_cell_value("Sheet1", 1, 4, text("A")).unwrap();
    engine
        .set_cell_value("Sheet1", 1, 5, LiteralValue::Number(2.5))
        .unwrap();
    engine.set_cell_value("Sheet1", 2, 4, text("C")).unwrap();
    let value_error = LiteralValue::Error(formualizer_common::ExcelError::new_value());
    let cases = [
        ("=VALUE(C1)", LiteralValue::Number(0.0)),
        (
            "=VALUE(VLOOKUP(\"C\",D1:E2,2,FALSE))",
            LiteralValue::Number(0.0),
        ),
        (
            "=VALUE(HLOOKUP(\"C\",D2:E3,2,FALSE))",
            LiteralValue::Number(0.0),
        ),
        ("=VALUE(\"\")", value_error.clone()),
        ("=VALUE(C1&\"\")", value_error.clone()),
        (
            "=VALUE(VLOOKUP(\"A\",D1:E2,2,FALSE))",
            LiteralValue::Number(2.5),
        ),
        ("=NUMBERVALUE(C1)", LiteralValue::Number(0.0)),
        (
            "=NUMBERVALUE(VLOOKUP(\"C\",D1:E2,2,FALSE))",
            LiteralValue::Number(0.0),
        ),
        ("=NUMBERVALUE(\"\")", LiteralValue::Number(0.0)),
        ("=NUMBERVALUE(\"  \")", LiteralValue::Number(0.0)),
        ("=NUMBERVALUE(\"%\")", value_error.clone()),
        (
            "=ISBLANK(VLOOKUP(\"C\",D1:E2,2,FALSE))",
            LiteralValue::Boolean(true),
        ),
    ];
    for (i, (formula, _)) in cases.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", 10 + i as u32, 1, parse(formula).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    for (i, (formula, expected)) in cases.iter().enumerate() {
        let got = engine.get_cell_value("Sheet1", 10 + i as u32, 1);
        match (expected, &got) {
            (LiteralValue::Error(e), Some(LiteralValue::Error(g))) => {
                assert_eq!(e.kind, g.kind, "{formula}")
            }
            _ => assert_eq!(got.as_ref(), Some(expected), "{formula}"),
        }
    }
}
