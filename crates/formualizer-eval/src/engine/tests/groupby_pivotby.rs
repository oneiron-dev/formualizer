//! GROUPBY and PIVOTBY through the engine, with LAMBDA and eta-reduced
//! (bare function name) aggregations.

use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;

fn n(v: f64) -> LiteralValue {
    LiteralValue::Number(v)
}

fn t(s: &str) -> LiteralValue {
    LiteralValue::Text(s.into())
}

/// A1:A4 = b a b a, B1:B4 = x x y y, C1:C4 = 1 2 3 4.
fn spill(formula: &str, rows: u32, cols: u32) -> Vec<Vec<LiteralValue>> {
    let mut engine = Engine::new(TestWorkbook::default(), EvalConfig::default());
    for (row, (a, b, c)) in [
        ("b", "x", 1.0),
        ("a", "x", 2.0),
        ("b", "y", 3.0),
        ("a", "y", 4.0),
    ]
    .into_iter()
    .enumerate()
    {
        let row = row as u32 + 1;
        engine.set_cell_value("Sheet1", row, 1, t(a)).unwrap();
        engine.set_cell_value("Sheet1", row, 2, t(b)).unwrap();
        engine.set_cell_value("Sheet1", row, 3, n(c)).unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 10, 10, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    engine
                        .get_cell_value("Sheet1", 10 + r, 10 + c)
                        .unwrap_or(LiteralValue::Empty)
                })
                .collect()
        })
        .collect()
}

#[test]
fn groupby_sorts_groups_and_adds_a_total_row() {
    assert_eq!(
        spill("=GROUPBY(A1:A4,C1:C4,SUM)", 3, 2),
        vec![
            vec![t("a"), n(6.0)],
            vec![t("b"), n(4.0)],
            vec![t("Total"), n(10.0)],
        ]
    );
}

#[test]
fn groupby_accepts_a_lambda_and_options() {
    assert_eq!(
        spill("=GROUPBY(A1:A4,C1:C4,LAMBDA(v,MAX(v)),0,0,-1)", 2, 2),
        vec![vec![t("b"), n(3.0)], vec![t("a"), n(4.0)]]
    );
    assert_eq!(
        spill("=GROUPBY(A1:A4,C1:C4,SUM,,0,-2)", 2, 2),
        vec![vec![t("a"), n(6.0)], vec![t("b"), n(4.0)]]
    );
    assert_eq!(
        spill("=GROUPBY(A1:A4,C1:C4,SUM,,-1,,C1:C4>1)", 3, 2),
        vec![
            vec![t("Total"), n(9.0)],
            vec![t("a"), n(6.0)],
            vec![t("b"), n(3.0)],
        ]
    );
}

#[test]
fn groupby_with_two_key_columns_adds_subtotals() {
    assert_eq!(
        spill("=GROUPBY(A1:B4,C1:C4,SUM)", 7, 3),
        vec![
            vec![t("a"), t("x"), n(2.0)],
            vec![t("a"), t("y"), n(4.0)],
            vec![t("a"), t(""), n(6.0)],
            vec![t("b"), t("x"), n(1.0)],
            vec![t("b"), t("y"), n(3.0)],
            vec![t("b"), t(""), n(4.0)],
            vec![t("Total"), t(""), n(10.0)],
        ]
    );
}

#[test]
fn pivotby_crosses_row_and_column_keys() {
    assert_eq!(
        spill("=PIVOTBY(A1:A4,B1:B4,C1:C4,SUM)", 4, 4),
        vec![
            vec![t(""), t("x"), t("y"), t("Total")],
            vec![t("a"), n(2.0), n(4.0), n(6.0)],
            vec![t("b"), n(1.0), n(3.0), n(4.0)],
            vec![t("Total"), n(3.0), n(7.0), n(10.0)],
        ]
    );
    assert_eq!(
        spill("=PIVOTBY(A1:A4,B1:B4,C1:C4,COUNT,,0,,0,,C1:C4<>2)", 3, 3),
        vec![
            vec![t(""), t("x"), t("y")],
            vec![t("a"), t(""), n(1.0)],
            vec![t("b"), n(1.0), n(1.0)],
        ]
    );
}

#[test]
fn text_function_names_are_not_functions() {
    match spill("=GROUPBY(A1:A4,C1:C4,\"SUM\")", 1, 1)
        .remove(0)
        .remove(0)
    {
        LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Value),
        other => panic!("expected #VALUE!, got {other:?}"),
    }
}

#[test]
fn eta_reduced_functions_work_in_lambda_helpers() {
    assert_eq!(
        spill("=BYROW(C1:C4,SUM)", 4, 1),
        vec![vec![n(1.0)], vec![n(2.0)], vec![n(3.0)], vec![n(4.0)]]
    );
    assert_eq!(spill("=REDUCE(0,C1:C4,SUM)", 1, 1), vec![vec![n(10.0)]]);
}
