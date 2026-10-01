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
    eval_with_unknown_fn_data_at(1, formula, false)
}

/// [`eval_with_unknown_fn_data`] with the formula in row `row` of column C,
/// under the declared array semantics of a workbook file when `legacy` is set.
fn eval_with_unknown_fn_data_at(row: u32, formula: &str, legacy: bool) -> LiteralValue {
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
        .set_cell_formula("Sheet1", row, 3, parse(formula).unwrap())
        .unwrap();
    if legacy {
        engine.use_legacy_array_semantics();
    }
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", row, 3).unwrap()
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

fn assert_error_kind(formula: &str, kind: ExcelErrorKind) {
    match eval_with_unknown_fn_data(formula) {
        LiteralValue::Error(e) => assert_eq!(e.kind, kind, "{formula}"),
        other => panic!("{formula}: expected {kind:?}, got {other:?}"),
    }
}

/// An error passed as a table, vector or range argument is the result: it is not
/// a 1x1 table to look up in or a cell to count, so IFNA/ISNA do not mistake it
/// for a lookup miss.
#[test]
fn error_in_table_or_range_argument_is_the_result() {
    for (formula, kind) in [
        ("=VLOOKUP(1,FOOBARFN(),1,FALSE)", ExcelErrorKind::Name),
        ("=HLOOKUP(1,FOOBARFN(),1,FALSE)", ExcelErrorKind::Name),
        ("=VLOOKUP(1,FOOBARFN(),1,TRUE)", ExcelErrorKind::Name),
        (
            "=IFNA(VLOOKUP(1,FOOBARFN(),1,FALSE),\"x\")",
            ExcelErrorKind::Name,
        ),
        ("=VLOOKUP(1,1/0,1,FALSE)", ExcelErrorKind::Div),
        ("=IFNA(HLOOKUP(1,1/0,1,FALSE),\"x\")", ExcelErrorKind::Div),
        // An error lookup value is the result in both match modes.
        ("=VLOOKUP(FOOBARFN(),A1:B3,2,TRUE)", ExcelErrorKind::Name),
        ("=HLOOKUP(FOOBARFN(),A1:B3,1,TRUE)", ExcelErrorKind::Name),
        ("=LOOKUP(100,A1:A2,FOOBARFN())", ExcelErrorKind::Name),
        // ... whether or not the lookup value is found.
        ("=LOOKUP(1,A1:A2,FOOBARFN())", ExcelErrorKind::Name),
        ("=LOOKUP(100,FOOBARFN())", ExcelErrorKind::Name),
        ("=LOOKUP(100,FOOBARFN(),B1:B2)", ExcelErrorKind::Name),
        ("=LOOKUP(100,A1:A2,NA())", ExcelErrorKind::Na),
        ("=COUNTBLANK(FOOBARFN())", ExcelErrorKind::Name),
        ("=COUNTBLANK(1/0)", ExcelErrorKind::Div),
    ] {
        assert_error_kind(formula, kind);
    }
    for (formula, expected) in [
        (
            "=ISNA(VLOOKUP(1,FOOBARFN(),1,FALSE))",
            LiteralValue::Boolean(false),
        ),
        (
            "=ISNA(LOOKUP(100,A1:A2,FOOBARFN()))",
            LiteralValue::Boolean(false),
        ),
        (
            "=IFERROR(COUNTBLANK(FOOBARFN()),-1)",
            LiteralValue::Number(-1.0),
        ),
        // Tables, vectors and ranges that are not errors still work, and an
        // error cell inside a range is data, not the result.
        ("=VLOOKUP(100,A1:B3,2,FALSE)", LiteralValue::Number(1.0)),
        ("=VLOOKUP(1,{1,2},2,FALSE)", LiteralValue::Number(2.0)),
        ("=HLOOKUP(1,{1,2;3,4},2,FALSE)", LiteralValue::Number(3.0)),
        ("=LOOKUP(100,A1:A2,B1:B2)", LiteralValue::Number(2.0)),
        ("=LOOKUP(2,{1,2,3},{10,20,30})", LiteralValue::Number(20.0)),
        ("=COUNTBLANK(A1:A3)", LiteralValue::Number(0.0)),
        ("=COUNTBLANK(A3)", LiteralValue::Number(0.0)),
        ("=COUNTBLANK(A1:A4)", LiteralValue::Number(1.0)),
    ] {
        assert_eq!(eval_with_unknown_fn_data(formula), expected, "{formula}");
    }
}

/// An error in a scalar argument (`k`, `number`, `quart`, a database or field,
/// `ref_text`) keeps its kind: only a usable value out of range gets the
/// function's own error code.
#[test]
fn error_in_scalar_argument_keeps_its_kind() {
    for (formula, kind) in [
        ("=LARGE(B1:B3,FOOBARFN())", ExcelErrorKind::Name),
        ("=LARGE(B1:B3,NA())", ExcelErrorKind::Na),
        ("=SMALL(B1:B3,NA())", ExcelErrorKind::Na),
        ("=PERCENTILE(B1:B3,NA())", ExcelErrorKind::Na),
        ("=PERCENTILE.INC(B1:B3,FOOBARFN())", ExcelErrorKind::Name),
        ("=PERCENTILE.EXC(B1:B3,NA())", ExcelErrorKind::Na),
        ("=QUARTILE(B1:B3,NA())", ExcelErrorKind::Na),
        ("=QUARTILE.INC(B1:B3,FOOBARFN())", ExcelErrorKind::Name),
        ("=QUARTILE.EXC(B1:B3,NA())", ExcelErrorKind::Na),
        ("=PERCENTRANK(B1:B3,NA())", ExcelErrorKind::Na),
        ("=PERCENTRANK.INC(B1:B3,FOOBARFN())", ExcelErrorKind::Name),
        ("=PERCENTRANK.EXC(B1:B3,NA())", ExcelErrorKind::Na),
        ("=PERCENTRANK.INC(B1:B3,2,NA())", ExcelErrorKind::Na),
        ("=PERCENTRANK.EXC(B1:B3,2,FOOBARFN())", ExcelErrorKind::Name),
        ("=RANK(FOOBARFN(),B1:B3)", ExcelErrorKind::Name),
        ("=RANK.EQ(FOOBARFN(),B1:B3)", ExcelErrorKind::Name),
        ("=RANK.EQ(1/0,B1:B3)", ExcelErrorKind::Div),
        ("=RANK.AVG(FOOBARFN(),B1:B3)", ExcelErrorKind::Name),
        ("=RANK.EQ(2,B1:B3,NA())", ExcelErrorKind::Na),
        ("=RANK.AVG(2,B1:B3,FOOBARFN())", ExcelErrorKind::Name),
        ("=DSUM(FOOBARFN(),1,A1:A2)", ExcelErrorKind::Name),
        ("=DSUM(A1:B2,FOOBARFN(),A1:A2)", ExcelErrorKind::Name),
        ("=DSUM(A1:B2,1,FOOBARFN())", ExcelErrorKind::Name),
        ("=DAVERAGE(NA(),1,A1:A2)", ExcelErrorKind::Na),
        ("=DSTDEV(A1:B2,NA(),A1:A2)", ExcelErrorKind::Na),
        ("=DGET(A1:B2,1,NA())", ExcelErrorKind::Na),
        ("=DCOUNTA(FOOBARFN(),1,A1:A2)", ExcelErrorKind::Name),
        ("=INDIRECT(FOOBARFN())", ExcelErrorKind::Name),
        ("=INDIRECT(NA())", ExcelErrorKind::Na),
        ("=INDIRECT(\"B2\",NA())", ExcelErrorKind::Na),
        ("=SUM(INDIRECT(FOOBARFN()))", ExcelErrorKind::Name),
        ("=FORECAST(FOOBARFN(),B1:B3,B1:B3)", ExcelErrorKind::Name),
        (
            "=FORECAST.LINEAR(FOOBARFN(),B1:B3,B1:B3)",
            ExcelErrorKind::Name,
        ),
        ("=FORECAST(NA(),B1:B3,B1:B3)", ExcelErrorKind::Na),
        ("=FORECAST.LINEAR(1/0,B1:B3,B1:B3)", ExcelErrorKind::Div),
        ("=FORECAST(A3,B1:B3,B1:B3)", ExcelErrorKind::Name),
        // A non-numeric `x` is #VALUE!, as documented.
        ("=FORECAST(\"x\",B1:B3,B1:B3)", ExcelErrorKind::Value),
        // A numeric argument out of range keeps the function's own code.
        ("=LARGE(B1:B3,4)", ExcelErrorKind::Num),
        ("=SMALL(B1:B3,0)", ExcelErrorKind::Num),
        ("=PERCENTILE.INC(B1:B3,2)", ExcelErrorKind::Num),
        ("=QUARTILE.INC(B1:B3,5)", ExcelErrorKind::Num),
        ("=RANK.EQ(3,B1:B3)", ExcelErrorKind::Na),
    ] {
        assert_error_kind(formula, kind);
    }
    for (formula, expected) in [
        (
            "=ERROR.TYPE(LARGE(B1:B3,FOOBARFN()))",
            LiteralValue::Number(5.0),
        ),
        (
            "=ISNA(RANK.EQ(FOOBARFN(),B1:B3))",
            LiteralValue::Boolean(false),
        ),
        (
            "=IFNA(SMALL(B1:B3,NA()),\"x\")",
            LiteralValue::Text("x".into()),
        ),
        ("=LARGE(B1:B3,2)", LiteralValue::Number(2.0)),
        ("=RANK.EQ(4,B1:B3)", LiteralValue::Number(1.0)),
        ("=FORECAST(3,B1:B3,B1:B3)", LiteralValue::Number(3.0)),
        (
            "=IFNA(FORECAST(NA(),B1:B3,B1:B3),\"x\")",
            LiteralValue::Text("x".into()),
        ),
        ("=INDIRECT(\"B2\")", LiteralValue::Number(2.0)),
    ] {
        assert_eq!(eval_with_unknown_fn_data(formula), expected, "{formula}");
    }
}

/// SUBTOTAL and AGGREGATE 1-13 take references. The "ignore error values"
/// options skip error cells inside the referenced ranges; an argument that is
/// itself an error (a value or a failed reference) is the result under every
/// option, and COUNTA does not count it.
#[test]
fn error_argument_to_subtotal_or_aggregate_reference_form_is_the_result() {
    for (formula, kind) in [
        ("=AGGREGATE(9,6,FOOBARFN())", ExcelErrorKind::Name),
        ("=AGGREGATE(4,6,FOOBARFN())", ExcelErrorKind::Name),
        ("=AGGREGATE(1,6,FOOBARFN())", ExcelErrorKind::Name),
        ("=AGGREGATE(9,6,NA())", ExcelErrorKind::Na),
        ("=AGGREGATE(9,2,1/0)", ExcelErrorKind::Div),
        ("=AGGREGATE(5,3,NA())", ExcelErrorKind::Na),
        ("=AGGREGATE(2,7,FOOBARFN())", ExcelErrorKind::Name),
        ("=AGGREGATE(3,6,NA())", ExcelErrorKind::Na),
        ("=AGGREGATE(3,0,NA())", ExcelErrorKind::Na),
        ("=AGGREGATE(12,6,FOOBARFN())", ExcelErrorKind::Name),
        ("=AGGREGATE(13,7,1/0)", ExcelErrorKind::Div),
        ("=AGGREGATE(9,6,B1:B3,FOOBARFN())", ExcelErrorKind::Name),
        ("=AGGREGATE(9,6,INDIRECT(\"zz\"))", ExcelErrorKind::Ref),
        ("=AGGREGATE(9,0,A1:A3)", ExcelErrorKind::Name),
        ("=SUBTOTAL(9,FOOBARFN())", ExcelErrorKind::Name),
        ("=SUBTOTAL(3,FOOBARFN())", ExcelErrorKind::Name),
        ("=SUBTOTAL(103,NA())", ExcelErrorKind::Na),
        ("=SUBTOTAL(3,INDIRECT(\"zz\"))", ExcelErrorKind::Ref),
    ] {
        assert_error_kind(formula, kind);
    }
    for (formula, expected) in [
        (
            "=IFNA(AGGREGATE(9,6,NA()),\"x\")",
            LiteralValue::Text("x".into()),
        ),
        // Error cells inside a referenced range are still skipped.
        ("=AGGREGATE(9,6,A1:A3)", LiteralValue::Number(200.0)),
        ("=AGGREGATE(9,6,A3)", LiteralValue::Number(0.0)),
        ("=AGGREGATE(3,6,A1:A3)", LiteralValue::Number(2.0)),
        ("=AGGREGATE(4,6,A1:A3,B1:B3)", LiteralValue::Number(100.0)),
        ("=AGGREGATE(9,6,B1:B3)", LiteralValue::Number(7.0)),
        ("=SUBTOTAL(3,A1:A3)", LiteralValue::Number(3.0)),
        // The array form (14-19) reads its argument as data, so an error
        // there is an item the option skips.
        ("=AGGREGATE(14,6,A1:A3,1)", LiteralValue::Number(100.0)),
    ] {
        assert_eq!(eval_with_unknown_fn_data(formula), expected, "{formula}");
    }
}

/// In a formula without the array flag, a range in a single-value position is
/// implicitly intersected first; the error that gives (#VALUE! off the range's
/// rows, or an error cell) is the argument, so it is the result too.
#[test]
fn error_arguments_in_formulas_without_the_array_flag() {
    for (row, formula, expected) in [
        (5, "=ISERROR(FOOBARFN(A1:A3))", LiteralValue::Boolean(true)),
        (
            5,
            "=SUMIFS(B1:B2,A1:A2,\"<=\"&EOM(A1:A2,0))",
            LiteralValue::Number(0.0),
        ),
        (
            2,
            "=VLOOKUP(A1:A3,A1:B3,2,FALSE)",
            LiteralValue::Number(1.0),
        ),
        (2, "=LARGE(B1:B3,B1:B3)", LiteralValue::Number(2.0)),
    ] {
        assert_eq!(
            eval_with_unknown_fn_data_at(row, formula, true),
            expected,
            "{formula} in row {row}"
        );
    }
    for (row, formula, kind) in [
        (5, "=COUNTBLANK(FOOBARFN(A1:A3))", ExcelErrorKind::Name),
        (3, "=VLOOKUP(A1:A3,A1:B3,2,FALSE)", ExcelErrorKind::Name),
        (5, "=VLOOKUP(A1:A3,A1:B3,2,FALSE)", ExcelErrorKind::Value),
        (5, "=LARGE(B1:B3,B1:B3)", ExcelErrorKind::Value),
        (5, "=RANK.EQ(B1:B3,B1:B3)", ExcelErrorKind::Value),
        (3, "=RANK.EQ(A1:A3,B1:B3)", ExcelErrorKind::Name),
    ] {
        match eval_with_unknown_fn_data_at(row, formula, true) {
            LiteralValue::Error(e) => assert_eq!(e.kind, kind, "{formula} in row {row}"),
            other => panic!("{formula} in row {row}: expected {kind:?}, got {other:?}"),
        }
    }
}
