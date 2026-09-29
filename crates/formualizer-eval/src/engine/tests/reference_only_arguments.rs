//! Arguments that Excel reads only as references (ROW, ROWS, COLUMN, COLUMNS,
//! AREAS, ISREF, SHEET, and CELL's non-contents info types) are not value
//! dependencies, so a formula may name its own cell there without a cycle.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn eval_all(formulas: &[(u32, u32, &str)]) -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_value("Sheet1", 1, 10, LiteralValue::Number(100.0))
        .unwrap();
    for &(row, col, formula) in formulas {
        engine
            .set_cell_formula("Sheet1", row, col, parse(formula).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    engine
}

#[test]
fn position_and_shape_functions_may_reference_their_own_cell() {
    let engine = eval_all(&[
        (6, 1, "=IF(ROWS(A$6:A6)>2,\"\",$J$1+ROWS(A$6:A6))"),
        (7, 1, "=IF(ROWS(A$6:A7)>2,\"\",$J$1+ROWS(A$6:A7))"),
        (2, 2, "=ROW(B2)*10"),
        (1, 3, "=COLUMNS($A1:C1)"),
        (5, 4, "=CELL(\"row\",D5)"),
        (3, 5, "=ISREF(E3)"),
    ]);
    let value = |row, col| engine.get_cell_value("Sheet1", row, col);
    assert_eq!(value(6, 1), Some(LiteralValue::Number(101.0)));
    assert_eq!(value(7, 1), Some(LiteralValue::Number(102.0)));
    assert_eq!(value(2, 2), Some(LiteralValue::Number(20.0)));
    assert_eq!(value(1, 3), Some(LiteralValue::Number(3.0)));
    assert_eq!(value(5, 4), Some(LiteralValue::Number(5.0)));
    assert_eq!(value(3, 5), Some(LiteralValue::Boolean(true)));
}

#[test]
fn value_reading_forms_still_see_the_cycle() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for formula in ["=CELL(\"contents\",A1)", "=SUM(A1)+ROWS(A1)"] {
        let error = engine
            .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
            .unwrap_err();
        assert_eq!(error.kind, ExcelErrorKind::Circ, "{formula}");
    }
}
