//! A function that fails returns its error as a value: the formula around it
//! keeps evaluating, so IS-functions and IF see the error like any value.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn eval(formula: &str) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Number(5.0))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 1, parse("=INDEX(#REF!,1)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 3, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 3).unwrap()
}

#[test]
fn is_functions_see_errors_from_inner_calls() {
    assert_eq!(
        eval("=ISNUMBER(SEARCH(\"Yes\",A2))"),
        LiteralValue::Boolean(false)
    );
    assert_eq!(eval("=ISTEXT(LEFT(A2,1))"), LiteralValue::Boolean(false));
    assert_eq!(
        eval("=ISERROR(FIND(\"x\",A2))"),
        LiteralValue::Boolean(true)
    );
    assert_eq!(
        eval("=IF(ISNUMBER(SEARCH(\"Yes\",A2)),A1,\"\")"),
        LiteralValue::Text(String::new())
    );
    assert_eq!(
        eval("=IF(ISNUMBER(SEARCH(\"Yes\",A2)),A2,\"\")"),
        LiteralValue::Text(String::new())
    );
    assert_eq!(
        eval("=IF(FALSE,A2,\"\")"),
        LiteralValue::Text(String::new())
    );
    match eval("=SEARCH(\"Yes\",A2)") {
        LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
}

#[test]
fn sequence_defaults_and_date_range() {
    assert_eq!(eval("=SUM(SEQUENCE(3,,10))"), LiteralValue::Number(33.0));
    assert_eq!(eval("=SUM(SEQUENCE(2,2))"), LiteralValue::Number(10.0));
    for formula in ["=DATE(21,202021,0)", "=DATE(10000,1,1)", "=DATE(1900,1,-1)"] {
        match eval(formula) {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Num, "{formula}"),
            other => panic!("{formula}: expected #NUM!, got {other:?}"),
        }
    }
    assert_eq!(eval("=DATE(9999,12,31)*1"), LiteralValue::Number(2958465.0));
}
