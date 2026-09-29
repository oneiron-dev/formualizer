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
