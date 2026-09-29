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
