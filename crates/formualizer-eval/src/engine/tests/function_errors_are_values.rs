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

/// Evaluates `formula` in C1 after A1:A3 = 100, 100, =FOOBARFN() and
/// B1:B3 = 1, 2, 4, so A3 holds the #NAME? an unknown function produces.
fn eval_with_unknown_fn_data(formula: &str) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in [(1, 100.0), (2, 100.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(value))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 3, 1, parse("=FOOBARFN()").unwrap())
        .unwrap();
    for (row, value) in [(1, 1.0), (2, 2.0), (3, 4.0)] {
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(value))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 1, 3, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 3).unwrap()
}

fn assert_name_error(formula: &str) {
    match eval_with_unknown_fn_data(formula) {
        LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Name, "{formula}"),
        other => panic!("{formula}: expected #NAME?, got {other:?}"),
    }
}

#[test]
fn unknown_function_is_a_name_value_for_enclosing_functions() {
    for (formula, expected) in [
        ("=ISERROR(FOOBARFN(1))", LiteralValue::Boolean(true)),
        ("=ISERR(FOOBARFN(1))", LiteralValue::Boolean(true)),
        ("=ISNA(FOOBARFN(1))", LiteralValue::Boolean(false)),
        ("=ISERROR(_xludf.FOOBARFN(1))", LiteralValue::Boolean(true)),
        ("=ERROR.TYPE(EOM(A1,0))", LiteralValue::Number(5.0)),
        ("=IFERROR(EOM(A1,0),7)", LiteralValue::Number(7.0)),
        (
            "=IF(ISERROR(_xlfn.NOSUCHFN(\"<a><b>1</b></a>\",\"//b\")),\"none\",\"some\")",
            LiteralValue::Text("none".into()),
        ),
        // A #NAME? criterion matches only cells holding #NAME?.
        (
            "=SUMIFS(B1:B2,A1:A2,\"<=\"&EOM(A1,0))",
            LiteralValue::Number(0.0),
        ),
        (
            "=SUMIFS(B1:B2,A1:A2,\">=\"&A1,A1:A2,\"<=\"&EOM(A1,0))",
            LiteralValue::Number(0.0),
        ),
        (
            "=SUM(SUMIFS(B1:B2,A1:A2,_xlfn.NOSUCHFN(\"<k><m>100</m></k>\",\"//m\")))",
            LiteralValue::Number(0.0),
        ),
        ("=SUMIF(A1:A3,FOOBARFN(1),B1:B3)", LiteralValue::Number(4.0)),
        ("=COUNTIF(A1:A3,FOOBARFN(1))", LiteralValue::Number(1.0)),
        // Arguments that are not evaluated never call the unknown function.
        (
            "=IF(FALSE,FOOBARFN(1),\"x\")",
            LiteralValue::Text("x".into()),
        ),
        (
            "=CHOOSE(2,FOOBARFN(1),\"x\")",
            LiteralValue::Text("x".into()),
        ),
    ] {
        assert_eq!(eval_with_unknown_fn_data(formula), expected, "{formula}");
    }
}

#[test]
fn unknown_function_still_yields_name_error() {
    for formula in [
        "=FOOBARFN(1)",
        "=EOM(A1,0)",
        "=\"<=\"&EOM(A1,0)",
        "=1+FOOBARFN()",
        "=SUM(B1,FOOBARFN(1))",
        "=IF(TRUE,FOOBARFN(1),\"x\")",
        "=IFERROR(FOOBARFN(1),FOOBARFN(2))",
        "=_xludf.FOOBARFN(1)",
        // WEBSERVICE and ENCODEURL are not implemented (no corpus use).
        "=_xlfn.WEBSERVICE(\"http://example.com\")",
        "=WEBSERVICE(\"http://example.com\")",
        "=_xlfn.ENCODEURL(\"a b\")",
        "=ENCODEURL(\"a b\")",
    ] {
        assert_name_error(formula);
    }
}
