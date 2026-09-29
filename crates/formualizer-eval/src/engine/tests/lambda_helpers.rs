//! LAMBDA invocation and the LAMBDA helper functions evaluated through the
//! engine, so formulas go through arena storage and dynamic-array spilling.

use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

use crate::engine::{Engine, EvalConfig, FormulaPlaneMode};
use crate::test_workbook::TestWorkbook;

fn n(value: f64) -> LiteralValue {
    LiteralValue::Number(value)
}

/// A1:B2 = 1 2 / 3 4; D1:D3 = 1 2 3.
fn build(mode: FormulaPlaneMode, formula: &str) -> Engine<TestWorkbook> {
    let mut engine = Engine::new(
        TestWorkbook::default(),
        EvalConfig::default().with_formula_plane_mode(mode),
    );
    for (row, col, value) in [(1, 1, 1.0), (1, 2, 2.0), (2, 1, 3.0), (2, 2, 4.0)] {
        engine.set_cell_value("Sheet1", row, col, n(value)).unwrap();
    }
    for row in 1..=3 {
        engine
            .set_cell_value("Sheet1", row, 4, n(row as f64))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 10, 10, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine
}

/// Evaluates `formula` at J10 and reads back a `rows x cols` block.
fn spill(formula: &str, rows: u32, cols: u32) -> Vec<Vec<LiteralValue>> {
    let mut first = None;
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let engine = build(mode, formula);
        let block: Vec<Vec<LiteralValue>> = (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| {
                        engine
                            .get_cell_value("Sheet1", 10 + r, 10 + c)
                            .unwrap_or(LiteralValue::Empty)
                    })
                    .collect()
            })
            .collect();
        match &first {
            None => first = Some(block),
            Some(prev) => assert_eq!(prev, &block, "{formula} differs across modes"),
        }
    }
    first.unwrap()
}

fn single(formula: &str) -> LiteralValue {
    spill(formula, 1, 1).remove(0).remove(0)
}

fn error_kind(value: LiteralValue) -> ExcelErrorKind {
    match value {
        LiteralValue::Error(e) => e.kind,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn lambda_immediate_invocation() {
    assert_eq!(single("=LAMBDA(x,x*2)(5)"), n(10.0));
    assert_eq!(single("=LAMBDA(x,y,x+y)(3,4)"), n(7.0));
    assert_eq!(single("=LAMBDA(r,SUM(r))(A1:B2)"), n(10.0));
    assert_eq!(single("=LET(k,10,LAMBDA(x,x+k)(1))"), n(11.0));
}

#[test]
fn lambda_invocation_with_wrong_argument_count_is_value_error() {
    assert_eq!(
        error_kind(single("=LAMBDA(x,y,x+y)(3)")),
        ExcelErrorKind::Value
    );
    assert_eq!(
        error_kind(single("=LAMBDA(x,x)(1,2)")),
        ExcelErrorKind::Value
    );
}

#[test]
fn map_applies_lambda_per_element() {
    assert_eq!(
        spill("=MAP(D1:D3,LAMBDA(x,x*2))", 3, 1),
        vec![vec![n(2.0)], vec![n(4.0)], vec![n(6.0)]]
    );
    assert_eq!(
        spill("=MAP(A1:B2,A1:B2,LAMBDA(a,b,a*b))", 2, 2),
        vec![vec![n(1.0), n(4.0)], vec![n(9.0), n(16.0)]]
    );
    assert_eq!(
        error_kind(single("=MAP(D1:D3,LAMBDA(a,b,a+b))")),
        ExcelErrorKind::Value
    );
}

#[test]
fn reduce_and_scan_fold_in_row_major_order() {
    assert_eq!(single("=REDUCE(0,D1:D3,LAMBDA(a,b,a+b))"), n(6.0));
    assert_eq!(
        single("=REDUCE(,A1:B2,LAMBDA(a,b,a&b))"),
        LiteralValue::Text("1234".into())
    );
    assert_eq!(
        spill("=SCAN(0,D1:D3,LAMBDA(a,b,a+b))", 3, 1),
        vec![vec![n(1.0)], vec![n(3.0)], vec![n(6.0)]]
    );
    assert_eq!(
        spill("=SCAN(1,A1:B2,LAMBDA(a,b,a*b))", 2, 2),
        vec![vec![n(1.0), n(2.0)], vec![n(6.0), n(24.0)]]
    );
}

#[test]
fn reduce_may_return_an_array() {
    assert_eq!(
        spill("=REDUCE(0,D1:D2,LAMBDA(a,b,a+{1;10}*b))", 2, 1),
        vec![vec![n(3.0)], vec![n(30.0)]]
    );
}

#[test]
fn byrow_and_bycol_reduce_each_line() {
    assert_eq!(
        spill("=BYROW(A1:B2,LAMBDA(r,SUM(r)))", 2, 1),
        vec![vec![n(3.0)], vec![n(7.0)]]
    );
    assert_eq!(
        spill("=BYCOL(A1:B2,LAMBDA(c,SUM(c)))", 1, 2),
        vec![vec![n(4.0), n(6.0)]]
    );
    assert_eq!(
        error_kind(single("=BYROW(A1:B2,LAMBDA(r,r*2))")),
        ExcelErrorKind::Calc
    );
}

#[test]
fn makearray_builds_from_indices() {
    assert_eq!(
        spill("=MAKEARRAY(2,2,LAMBDA(r,c,r*c))", 2, 2),
        vec![vec![n(1.0), n(2.0)], vec![n(2.0), n(4.0)]]
    );
    assert_eq!(
        spill("=MAKEARRAY(1,3,LAMBDA(r,c,c))", 1, 3),
        vec![vec![n(1.0), n(2.0), n(3.0)]]
    );
    assert_eq!(
        error_kind(single("=MAKEARRAY(0,2,LAMBDA(r,c,r))")),
        ExcelErrorKind::Value
    );
}

#[test]
fn helper_without_lambda_is_value_error() {
    assert_eq!(error_kind(single("=MAP(D1:D3,5)")), ExcelErrorKind::Value);
}
