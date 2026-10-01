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

fn number(v: LiteralValue) -> f64 {
    match v {
        LiteralValue::Number(n) => n,
        LiteralValue::Int(i) => i as f64,
        other => panic!("expected a number, got {other:?}"),
    }
}

fn text(s: &str) -> LiteralValue {
    LiteralValue::Text(s.into())
}

#[test]
fn function_zero_results_reach_text_functions_as_zero() {
    // ROUND(-0.4,0), TRUNC(-0.5), QUOTIENT(-1,2) and PRODUCT(-1,0) are 0, so
    // a text function that takes one directly sees "0", never "-0".
    let engine = engine_with(&[
        (1, 1, "=LEN(ROUND(-0.4,0))"),
        (2, 1, "=LEN(TRUNC(-0.5))"),
        (3, 1, "=LEFT(ROUND(-0.4,0),1)"),
        (4, 1, "=RIGHT(QUOTIENT(-1,2))"),
        (5, 1, "=MID(ROUND(-0.4,0),1,1)"),
        (6, 1, "=FIND(\"0\",ROUND(-0.4,0))"),
        (7, 1, "=SEARCH(\"-\",ROUND(-0.4,0))"),
        (8, 1, "=EXACT(ROUND(-0.4,0),\"0\")"),
        (9, 1, "=SUBSTITUTE(ROUND(-0.4,0),\"0\",\"x\")"),
        (10, 1, "=REPLACE(ROUND(-0.4,0),1,0,\"x\")"),
        (11, 1, "=REPT(ROUND(-0.4,0),2)"),
        (
            12,
            1,
            "=TEXTJOIN(\",\",TRUE,ROUND({-0.4,0.4},0),PRODUCT(-1,0))",
        ),
        (13, 1, "=CLEAN(ROUND(-0.4,0))"),
        (14, 1, "=UNICODE(ROUND(-0.4,0))"),
        (15, 1, "=CODE(TRUNC(-0.5))"),
        (16, 1, "=TEXTBEFORE(ROUND(-0.4,0),\"0\")"),
        (17, 1, "=ARRAYTOTEXT(ROUND(-0.4,0))"),
        (18, 1, "=VALUETOTEXT(ROUND(-0.4,0))"),
        (19, 1, "=TEXT(5,ROUND(-0.4,0))"),
        (20, 1, "=UPPER(ROUND(-0.4,0))"),
        (1, 3, "=TEXTSPLIT(ROUND(-0.4,0),\",\")"),
    ]);
    assert_eq!(number(value(&engine, 1, 1)), 1.0);
    assert_eq!(number(value(&engine, 2, 1)), 1.0);
    assert_eq!(value(&engine, 3, 1), text("0"));
    assert_eq!(value(&engine, 4, 1), text("0"));
    assert_eq!(value(&engine, 5, 1), text("0"));
    assert_eq!(number(value(&engine, 6, 1)), 1.0);
    assert!(matches!(
        value(&engine, 7, 1),
        LiteralValue::Error(e) if e.kind == formualizer_common::ExcelErrorKind::Value
    ));
    assert_eq!(value(&engine, 8, 1), LiteralValue::Boolean(true));
    assert_eq!(value(&engine, 9, 1), text("x"));
    assert_eq!(value(&engine, 10, 1), text("x0"));
    assert_eq!(value(&engine, 11, 1), text("00"));
    assert_eq!(value(&engine, 12, 1), text("0,0,0"));
    assert_eq!(value(&engine, 13, 1), text("0"));
    assert_eq!(number(value(&engine, 14, 1)), 48.0);
    assert_eq!(number(value(&engine, 15, 1)), 48.0);
    assert_eq!(value(&engine, 16, 1), text(""));
    assert_eq!(value(&engine, 17, 1), text("0"));
    assert_eq!(value(&engine, 18, 1), text("0"));
    assert_eq!(value(&engine, 19, 1), text("5"));
    assert_eq!(value(&engine, 20, 1), text("0"));
    assert_eq!(value(&engine, 1, 3), text("0"));
}

#[test]
fn nonzero_negative_function_results_keep_their_sign_in_text() {
    let engine = engine_with(&[
        (1, 1, "=LEN(ROUND(-1.4,0))"),
        (2, 1, "=LEFT(TRUNC(-2.5),1)"),
        (3, 1, "=TEXTJOIN(\",\",TRUE,ROUND({-1.4,0.4},0))"),
        (4, 1, "=SUBSTITUTE(-0.5*1,\"5\",\"x\")"),
    ]);
    assert_eq!(number(value(&engine, 1, 1)), 2.0);
    assert_eq!(value(&engine, 2, 1), text("-"));
    assert_eq!(value(&engine, 3, 1), text("-1,0"));
    assert_eq!(value(&engine, 4, 1), text("-0.x"));
}
