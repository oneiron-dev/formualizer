//! Excel evaluates a function once per element when a single-value parameter
//! receives a multi-cell range or array, and IF/IFERROR/IFNA select
//! element-wise over array values.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// A1:A3 = "a","b","a" ; B1:B3 = 1,2,3 ; C1:C3 = 45000,45100,"x"
fn engine() -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let cells: [(u32, u32, LiteralValue); 9] = [
        (1, 1, LiteralValue::Text("a".into())),
        (2, 1, LiteralValue::Text("b".into())),
        (3, 1, LiteralValue::Text("a".into())),
        (1, 2, LiteralValue::Number(1.0)),
        (2, 2, LiteralValue::Number(2.0)),
        (3, 2, LiteralValue::Number(3.0)),
        (1, 3, LiteralValue::Number(45000.0)),
        (2, 3, LiteralValue::Number(45100.0)),
        (3, 3, LiteralValue::Text("x".into())),
    ];
    for (row, col, value) in cells {
        engine.set_cell_value("Sheet1", row, col, value).unwrap();
    }
    engine
}

fn eval(engine: &mut Engine<TestWorkbook>, formula: &str) -> LiteralValue {
    engine
        .set_cell_formula("Sheet1", 1, 10, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_cell("Sheet1", 1, 10).unwrap();
    engine.get_cell_value("Sheet1", 1, 10).unwrap()
}

fn assert_number(formula: &str, expected: f64) {
    let mut engine = engine();
    match eval(&mut engine, formula) {
        LiteralValue::Number(n) => assert!((n - expected).abs() < 1e-9, "{formula} = {n}"),
        LiteralValue::Int(i) => assert_eq!(i as f64, expected, "{formula}"),
        other => panic!("{formula} = {other:?}, expected {expected}"),
    }
}

#[test]
fn scalar_text_date_and_info_functions_lift_over_ranges() {
    assert_number("=SUMPRODUCT(LEN(A1:A3))", 3.0);
    assert_number("=SUM(--ISNUMBER(B1:C3))", 5.0);
    assert_number("=SUMPRODUCT(--(MONTH(C1:C2)=3))", 1.0);
    assert_number("=SUMPRODUCT(--(LEFT(A1:A3,1)=\"a\"))", 2.0);
    assert_number("=SUMPRODUCT(--(TEXT(C1:C2,\"yyyy-mm\")=\"2023-06\"))", 1.0);
    assert_number("=SUM(ROUND(B1:B3/2,0))", 4.0);
}

#[test]
fn lookup_and_criteria_values_lift_but_tables_and_ranges_do_not() {
    assert_number("=SUM(VLOOKUP(A1:A3,A1:B3,2,0))", 4.0);
    assert_number("=SUM(MATCH(A1:A3,A1:A3,0))", 4.0);
    assert_number("=SUM(COUNTIF(A1:A3,A1:A3))", 5.0);
    assert_number("=SUM(1/COUNTIF(A1:A3,A1:A3))", 2.0);
    assert_number("=SUM(SUMIFS(B1:B3,A1:A3,{\"a\",\"b\"}))", 6.0);
    assert_number("=SUM(LARGE(B1:B3,{1,2}))", 5.0);
}

#[test]
fn if_selects_element_wise_over_array_conditions() {
    assert_number("=AVERAGE(IF(A1:A3=\"a\",B1:B3))", 2.0);
    assert_number("=SUM(IF(A1:A3=\"a\",1,0))", 2.0);
    assert_number("=MAX(IF(A1:A3=\"b\",B1:B3))", 2.0);
    assert_number("=SUM(IF((A1:A3=\"a\")*(B1:B3>1),B1:B3,0))", 3.0);
    // A missing FALSE branch is FALSE, which SUM ignores.
    assert_number("=SUM(IF(B1:B3>1,B1:B3))", 5.0);
    // Text "TRUE"/"FALSE" conditions are logical values.
    assert_number("=IF(\"true\",1,2)", 1.0);
    let mut engine = engine();
    assert!(matches!(
        eval(&mut engine, "=IF(\"yes\",1,2)"),
        LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value
    ));
}

#[test]
fn iferror_and_ifna_replace_error_elements() {
    assert_number("=SUM(IFERROR(1/(B1:B3-2),0))", 0.0);
    assert_number("=SUM(IFERROR(--C1:C3,100))", 90200.0);
    assert_number("=SUM(IFNA(MATCH({\"a\",\"z\"},A1:A3,0),10))", 11.0);
    // IFNA leaves other errors in place.
    let mut engine = engine();
    assert!(matches!(
        eval(&mut engine, "=SUM(IFNA(1/(B1:B3-2),0))"),
        LiteralValue::Error(e) if e.kind == ExcelErrorKind::Div
    ));
}

#[test]
fn mismatched_lifted_shapes_pad_with_na() {
    let mut engine = engine();
    assert!(matches!(
        eval(&mut engine, "=SUM(ROUND(B1:B3,{0;1}))"),
        LiteralValue::Error(e) if e.kind == ExcelErrorKind::Na
    ));
    assert_number("=SUM(ROUND(B1:B3,{0}))", 6.0);
}

#[test]
fn concatenation_is_element_wise_over_arrays() {
    // A1:A3 & B1:B3 = "a1","b2","a3"
    assert_number("=MATCH(\"b2\",A1:A3&B1:B3,0)", 2.0);
    assert_number("=ROWS(A1:A3&\"-\")", 3.0);
    assert_number("=SUM(--(A1:A3&B1:B3=\"a3\"))", 1.0);
    let mut engine = engine();
    assert_eq!(
        eval(&mut engine, "=INDEX(A1:A3&B1:B3,3)"),
        LiteralValue::Text("a3".into())
    );
    // An error element stays an error in its own position only.
    assert_number("=SUM(--ISERROR({1,\"x\"}&IF({TRUE,FALSE},NA(),1)))", 1.0);
}

#[test]
fn operators_pad_the_shorter_array_with_na() {
    // {1,2,3}+{1,2} = {2,4,#N/A}
    assert_number("=SUM(IFERROR({1,2,3}+{1,2},0))", 6.0);
    // ROW(B1:B3)/(A1:A2="a") = {1,#DIV/0!,#N/A}; AGGREGATE option 6 skips errors.
    assert_number("=_xlfn.AGGREGATE(15,6,ROW(B1:B3)/(A1:A2=\"a\"),1)", 1.0);
    assert_number("=ROWS(B1:B3*{1;2})", 3.0);
    assert_number("=SUM(IFERROR(B1:B3*{1;2},0))", 5.0);
    // A single row or column still repeats.
    assert_number("=SUM({1;2;3}*{1,2})", 18.0);
    let mut engine = engine();
    match eval(&mut engine, "=INDEX(B1:B3+{1;2},3)") {
        LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Na),
        other => panic!("expected #N/A, got {other:?}"),
    }
}

fn assert_error(formula: &str, kind: ExcelErrorKind) {
    let mut engine = engine();
    match eval(&mut engine, formula) {
        LiteralValue::Error(e) => assert_eq!(e.kind, kind, "{formula}"),
        other => panic!("{formula} = {other:?}, expected {kind:?}"),
    }
}

#[test]
fn reference_parameters_lift_over_offset_with_array_offsets() {
    // OFFSET(B1,{0;1;2},0) is the array of references {B1;B2;B3}.
    assert_number("=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,{0;1;2},0)))", 6.0);
    assert_number("=MAX(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))", 3.0);
    assert_number(
        "=SUMPRODUCT(SUBTOTAL(9,OFFSET(B$1,ROW(B$1:B$3)-ROW(B$1),0)),--(A1:A3=\"a\"))",
        4.0,
    );
    // Each reference may be a range: per-column maxima of B1:C3 (text skipped).
    assert_number("=SUM(SUBTOTAL(4,OFFSET(B1:B3,,{0,1},)))", 45103.0);
    assert_number(
        "=SUMPRODUCT(_xlfn.AGGREGATE(9,6,OFFSET(B1,{0;1;2},0)))",
        6.0,
    );
    // Criteria ranges, sum ranges and an array width.
    assert_number("=SUM(SUMIF(A1:A3,\"a\",OFFSET(A1:A3,,{1,2})))", 45004.0);
    assert_number("=SUMPRODUCT(COUNTIF(OFFSET(A1,{0;1;2},0),\"a\"))", 2.0);
    assert_number("=SUM(SUMIF(OFFSET(B1,,,{1,2,3}),\"<>\"))", 10.0);
    assert_number("=SUM(SUMIFS(OFFSET(B1:B3,0,{0,1}),A1:A3,\"a\"))", 45004.0);
    assert_number(
        "=SUM(COUNTIFS(OFFSET(A1,{0;1;2},0),\"a\",B1:B1,\">1\"))",
        0.0,
    );
    assert_number("=SUM(COUNTIFS(OFFSET(A1,{0;1;2},0),\"a\"))", 2.0);
    // A single-value parameter reads each reference.
    assert_number("=SUMPRODUCT(N(OFFSET(B1,{0;1;2},0)))", 6.0);
    assert_number("=SUMPRODUCT(--ISNUMBER(OFFSET(B1,{0;1},{0,1,2})))", 4.0);
    // INDIRECT with an array of addresses is an array of references too.
    assert_number("=SUMPRODUCT(N(INDIRECT(\"B\"&{1,2,3})))", 6.0);
    assert_number("=SUM(COUNTIF(INDIRECT({\"A1:A3\",\"A2\"}),\"a\"))", 2.0);
}

#[test]
fn array_of_references_elements_broadcast_and_keep_their_errors() {
    // {0;1;2} against {0;1}: the third reference is #N/A.
    assert_error(
        "=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,{0;1;2},{0;1})))",
        ExcelErrorKind::Na,
    );
    // B0 is #REF! in its own position only.
    assert_number(
        "=SUMPRODUCT(--ISERROR(SUBTOTAL(9,OFFSET(B1,{-1;0;1},0))))",
        1.0,
    );
    // An array of references has no value of its own.
    assert_error("=OFFSET(B1,{0;1},0)", ExcelErrorKind::Value);
}

#[test]
fn single_offset_references_are_not_lifted() {
    assert_number("=SUBTOTAL(9,OFFSET(B1,1,0,2))", 5.0);
    assert_number("=SUMIF(A1:A3,\"a\",OFFSET(A1:A3,0,1))", 4.0);
    assert_number("=COUNTIF(OFFSET(A1,0,0,3),\"a\")", 2.0);
    assert_number("=SUM(OFFSET(B1,0,0,3))", 6.0);
    assert_number("=ROWS(OFFSET(B1,0,0,3,2))", 3.0);
    assert_number("=N(OFFSET(B1,2,0))", 3.0);
    assert_error("=OFFSET(B1,-1,0)", ExcelErrorKind::Ref);
    assert_error("=OFFSET(B1,\"x\",0)", ExcelErrorKind::Value);
    assert_error("=OFFSET(B1,NA(),0)", ExcelErrorKind::Na);
}
