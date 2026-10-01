//! Excel has no negative zero: `-A1` or `-1*(A1-5)` with a zero operand is a
//! plain 0, so it prints as "0" and matches a 0 criterion in COUNTIF(S).

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

fn engine_with(cells: &[(u32, u32, &str)]) -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for &(row, col, content) in cells {
        if content.starts_with('=') {
            engine
                .set_cell_formula("Sheet1", row, col, parse(content).unwrap())
                .unwrap();
        } else {
            engine
                .set_cell_value(
                    "Sheet1",
                    row,
                    col,
                    LiteralValue::Number(content.parse().unwrap()),
                )
                .unwrap();
        }
    }
    engine.evaluate_all().unwrap();
    engine
}

fn value(engine: &Engine<TestWorkbook>, row: u32, col: u32) -> LiteralValue {
    engine.get_cell_value("Sheet1", row, col).unwrap()
}

fn positive_zero(v: LiteralValue) -> bool {
    matches!(v, LiteralValue::Number(n) if n == 0.0 && n.is_sign_positive())
}

#[test]
fn zero_results_of_operators_are_positive_zero() {
    // A1 = 0, A2 = 5
    let engine = engine_with(&[
        (1, 1, "0"),
        (2, 1, "5"),
        (1, 2, "=-A1"),
        (2, 2, "=-1*(A2-5)"),
        (3, 2, "=0/-A2"),
        (4, 2, "=-A2"),
        (5, 2, "=-A2*1E-300"),
        (6, 2, "=ROUND(-0.4,0)"),
    ]);
    assert!(positive_zero(value(&engine, 1, 2)));
    assert!(positive_zero(value(&engine, 2, 2)));
    assert!(positive_zero(value(&engine, 3, 2)));
    assert!(positive_zero(value(&engine, 6, 2)));
    // Nonzero negatives keep their sign, however small.
    assert_eq!(value(&engine, 4, 2), LiteralValue::Number(-5.0));
    assert_eq!(value(&engine, 5, 2), LiteralValue::Number(-5e-300));
}

#[test]
fn zero_converts_to_text_without_a_minus_sign() {
    let engine = engine_with(&[
        (1, 1, "0"),
        (2, 1, "5"),
        (1, 2, "=-1*(A2-5)"),
        (1, 3, "=B1&\"\""),
        (2, 3, "=CONCATENATE(B1,\",\",-A1,\",\",-A2)"),
        (3, 3, "=ROUND(-0.4,0)&\"|\""),
    ]);
    assert_eq!(value(&engine, 1, 3), LiteralValue::Text("0".into()));
    assert_eq!(value(&engine, 2, 3), LiteralValue::Text("0,0,-5".into()));
    assert_eq!(value(&engine, 3, 3), LiteralValue::Text("0|".into()));
}

#[test]
fn countif_matches_negated_zero_as_zero() {
    // A1:A3 = 0, 0, 5 (numbers only); B1:B3 = -A1, 0*-1, 5 (formula results).
    let engine = engine_with(&[
        (1, 1, "0"),
        (2, 1, "0"),
        (3, 1, "5"),
        (1, 2, "=-A1"),
        (2, 2, "=0*-1"),
        (3, 2, "=A3"),
        (1, 4, "=COUNTIF(A1:A3,-A1)"),
        (2, 4, "=COUNTIF(A1:A3,0*-1)"),
        (3, 4, "=COUNTIFS(B1:B3,0)"),
        (4, 4, "=COUNTIFS(B1:B3,\"<>0\")"),
        (5, 4, "=COUNTIF(A1:A3,\"-0\")"),
        (6, 4, "=COUNTIF(A1:A3,-A3)"),
    ]);
    assert_eq!(value(&engine, 1, 4), LiteralValue::Number(2.0));
    assert_eq!(value(&engine, 2, 4), LiteralValue::Number(2.0));
    assert_eq!(value(&engine, 3, 4), LiteralValue::Number(2.0));
    assert_eq!(value(&engine, 4, 4), LiteralValue::Number(1.0));
    assert_eq!(value(&engine, 5, 4), LiteralValue::Number(2.0));
    assert_eq!(value(&engine, 6, 4), LiteralValue::Number(0.0));
}
