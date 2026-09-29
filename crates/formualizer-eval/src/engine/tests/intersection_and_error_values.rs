//! The space (intersection) operator, and functions that inspect error
//! values rather than propagating them.

use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;

/// A1:D4 hold 1..16 row by row; each formula goes in F1, F2, ...
fn eval_all(formulas: &[&str]) -> Vec<LiteralValue> {
    let mut engine = Engine::new(TestWorkbook::default(), EvalConfig::default());
    for r in 1..=4u32 {
        for c in 1..=4u32 {
            engine
                .set_cell_value(
                    "Sheet1",
                    r,
                    c,
                    LiteralValue::Number(((r - 1) * 4 + c) as f64),
                )
                .unwrap();
        }
    }
    for (i, f) in formulas.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", i as u32 + 1, 6, parse(f).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    (0..formulas.len())
        .map(|i| engine.get_cell_value("Sheet1", i as u32 + 1, 6).unwrap())
        .collect()
}

fn n(v: f64) -> LiteralValue {
    LiteralValue::Number(v)
}

fn kind(v: &LiteralValue) -> Option<ExcelErrorKind> {
    match v {
        LiteralValue::Error(e) => Some(e.kind),
        _ => None,
    }
}

#[test]
fn space_operator_intersects_references() {
    let got = eval_all(&[
        "=SUM(A1:C3 B2:D4)",
        "=A1:D1 B1:B4",
        "=ROWS(A1:C3 B2:D4)",
        "=SUM(A:A 2:2)",
    ]);
    assert_eq!(got[0], n(6.0 + 7.0 + 10.0 + 11.0));
    assert_eq!(got[1], n(2.0));
    assert_eq!(got[2], n(2.0));
    assert_eq!(got[3], n(5.0));
}

#[test]
fn disjoint_intersection_is_null() {
    let got = eval_all(&["=A1:A2 C1:C2", "=ERROR.TYPE(A1:A2 C1:C2)"]);
    assert_eq!(kind(&got[0]), Some(ExcelErrorKind::Null));
    assert_eq!(got[1], n(1.0));
}

#[test]
fn error_inspecting_functions_see_errors() {
    let got = eval_all(&[
        "=ERROR.TYPE(qwertyzz)",
        "=TYPE(1/0)",
        "=TYPE(qwertyzz)",
        "=ISERROR(qwertyzz)",
        "=IFERROR(qwertyzz,7)",
        "=TYPE(\"a\")",
    ]);
    assert_eq!(got[0], n(5.0));
    assert_eq!(got[1], n(16.0));
    assert_eq!(got[2], n(16.0));
    assert_eq!(got[3], LiteralValue::Boolean(true));
    assert_eq!(got[4], n(7.0));
    assert_eq!(got[5], n(2.0));
}

#[test]
fn intersection_formulas_parse() {
    for f in [
        "=SUM(A1:C3 B2:D4)",
        "=A1:D1 B1:B4",
        "=ROWS(A1:C3 B2:D4)",
        "=SUM(A:A 2:2)",
    ] {
        assert!(parse(f).is_ok(), "{f}: {:?}", parse(f).err());
    }
}
