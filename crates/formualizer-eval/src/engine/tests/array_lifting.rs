//! Excel evaluates a function once per element when a single-value parameter
//! receives a multi-cell range or array, and IF/IFERROR/IFNA select
//! element-wise over array values.

use crate::engine::named_range::{NameScope, NamedDefinition};
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
fn index_selects_one_value_per_array_row_or_column_number() {
    assert_number("=SUM(INDEX(B1:B3,{1;3}))", 4.0);
    assert_number("=SUM(INDEX({10;20;30},{1;3}))", 40.0);
    assert_number("=ROWS(INDEX(B1:B3,{1;3;2}))", 3.0);
    assert_number("=COLUMNS(INDEX(B1:B3,{1,3}))", 2.0);
    assert_number("=SUM(INDEX(B1:C3,{1;2},1))", 3.0);
    assert_number("=SUM(INDEX(B1:B3,MATCH({\"b\";\"a\"},A1:A3,0)))", 3.0);
    // A row array and a column array broadcast; equal shapes pair up.
    assert_number("=SUM(INDEX(B1:C3,{1;2},{1,2}))", 90103.0);
    assert_number("=COUNT(INDEX(B1:C3,{1,3},{1,2}))", 1.0);
    // Each element stands alone: out of range is #REF!, an error stays.
    assert_number("=SUM(IFERROR(INDEX(B1:B3,{1;4}),100))", 101.0);
    assert_number("=SUM(--ISNA(INDEX(B1:B3,IF({TRUE;FALSE},1,NA()))))", 1.0);
    let mut engine = engine();
    assert_eq!(
        eval(&mut engine, "=INDEX(A1:A3,{2;1})"),
        LiteralValue::Text("b".into())
    );
    match eval(&mut engine, "=INDEX(B1:B3,{4;1})") {
        LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
    // Single-value selections are unchanged.
    assert_number("=INDEX(B1:B3,2)", 2.0);
    assert_number("=SUM(INDEX(B1:C3,0,1))", 6.0);
    match eval(&mut engine, "=INDEX(B1:B3,4)") {
        LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
}

/// The values at `reads` after evaluating each formula at its (row, col)
/// over the A1:C3 grid plus G1:G3 = 3,1,2 and E5:F5 = 2,1, as a workbook file
/// stores them (legacy array semantics): `arrays` lists the (row, col, rows)
/// of formulas entered with the array flag; every other formula is entered
/// without it. Errors keep only their kind.
fn file_values(
    formulas: &[(u32, u32, &str)],
    arrays: &[(u32, u32, u32)],
    reads: &[(u32, u32)],
) -> Vec<Option<LiteralValue>> {
    let mut engine = engine();
    for (row, col, value) in [
        (1, 7, 3.0),
        (2, 7, 1.0),
        (3, 7, 2.0),
        (5, 5, 2.0),
        (5, 6, 1.0),
    ] {
        engine
            .set_cell_value("Sheet1", row, col, LiteralValue::Number(value))
            .unwrap();
    }
    for &(row, col, formula) in formulas {
        engine
            .set_cell_formula("Sheet1", row, col, parse(formula).unwrap())
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    for &(row, col, rows) in arrays {
        engine.declare_array_formula("Sheet1", row, col, rows, 1, false);
    }
    engine.evaluate_all().unwrap();
    reads
        .iter()
        .map(
            |&(row, col)| match engine.get_cell_value("Sheet1", row, col) {
                Some(LiteralValue::Error(error)) => Some(LiteralValue::Error(error.kind.into())),
                other => other,
            },
        )
        .collect()
}

/// The values of formulas entered without the array flag.
fn plain_formula_values(formulas: &[(u32, u32, &str)]) -> Vec<Option<LiteralValue>> {
    let reads: Vec<(u32, u32)> = formulas.iter().map(|&(row, col, _)| (row, col)).collect();
    file_values(formulas, &[], &reads)
}

#[test]
fn index_intersects_a_range_selector_in_a_formula_without_the_array_flag() {
    let number = |n: f64| Some(LiteralValue::Number(n));
    let value_error = || Some(LiteralValue::Error(ExcelErrorKind::Value.into()));
    let text = |t: &str| Some(LiteralValue::Text(t.into()));
    // A column range picks the formula's row and a row range the formula's
    // column; #VALUE! where they do not cross.
    assert_eq!(
        plain_formula_values(&[
            (2, 8, "=INDEX($B$1:$B$3,G1:G3)"),
            (3, 8, "=INDEX($B$1:$B$3,G1:G3)"),
            (10, 8, "=INDEX($B$1:$B$3,G1:G3)"),
            (10, 9, "=IFERROR(INDEX($B$1:$B$3,G1:G3),\"none\")"),
            (10, 5, "=INDEX($B$1:$C$3,1,E5:F5)"),
            (10, 6, "=INDEX($B$1:$C$3,1,E5:F5)"),
            (10, 7, "=INDEX($B$1:$C$3,1,E5:F5)"),
        ]),
        vec![
            number(1.0),
            number(2.0),
            value_error(),
            text("none"),
            number(45000.0),
            number(1.0),
            value_error(),
        ]
    );
    // Inside a reference argument, and when INDEX is used as a reference.
    assert_eq!(
        plain_formula_values(&[
            (2, 8, "=SUM(INDEX($B$1:$B$3,G1:G3))"),
            (10, 8, "=SUM(INDEX($B$1:$B$3,G1:G3))"),
            (3, 9, "=SUM(INDEX($B$1:$B$3,G1:G3):$B$3)"),
        ]),
        vec![number(1.0), value_error(), number(5.0)]
    );
    // A range given to MATCH's lookup value inside row_num intersects too:
    // row 2 looks up G2 = 1, found at 2; row 11 crosses no row of G1:G3.
    assert_eq!(
        plain_formula_values(&[
            (
                2,
                12,
                "=IFERROR(INDEX($B$1:$B$3,MATCH(G1:G3,G1:G3,0)),\"none\")"
            ),
            (
                11,
                8,
                "=IFERROR(INDEX($B$1:$B$3,MATCH(G1:G3,G1:G3,0)),\"none\")"
            ),
        ]),
        vec![number(2.0), text("none")]
    );
    // An array is not a range: it is lifted, and a single cell shows its first
    // element.
    assert_eq!(
        plain_formula_values(&[
            (10, 8, "=INDEX($B$1:$B$3,{3;1;2})"),
            (10, 9, "=SUM(INDEX($B$1:$B$3,{1;3}))"),
        ]),
        vec![number(3.0), number(4.0)]
    );
    // A formula entered with the array flag lifts over the range.
    assert_eq!(
        file_values(
            &[(1, 10, "=INDEX($B$1:$B$3,G1:G3)")],
            &[(1, 10, 3)],
            &[(1, 10), (2, 10), (3, 10)],
        ),
        vec![number(3.0), number(1.0), number(2.0)]
    );
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
    // N reads each reference.
    assert_number("=SUMPRODUCT(N(OFFSET(B1,{0;1;2},0)))", 6.0);
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

fn assert_text(formula: &str, expected: &str) {
    let mut engine = engine();
    match eval(&mut engine, formula) {
        LiteralValue::Text(text) => assert_eq!(text, expected, "{formula}"),
        other => panic!("{formula} = {other:?}, expected {expected:?}"),
    }
}

#[test]
fn offset_past_the_sheet_edge_is_ref_per_element() {
    // Each off-grid reference is #REF! in its own position (100 here); the
    // others still evaluate.
    assert_number(
        "=SUMPRODUCT(IFERROR(SUBTOTAL(9,OFFSET(B1,{0;1048576},0)),100))",
        101.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(SUBTOTAL(9,OFFSET(B1,{0;1},{0;16384})),100))",
        101.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(SUBTOTAL(9,OFFSET(B1,0,0,{1;1048577})),100))",
        101.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(SUBTOTAL(9,OFFSET(B1,0,0,1,{1;16384})),100))",
        101.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(N(OFFSET(B1,{0;1},{0;1E+300})),100))",
        101.0,
    );
    // The scalar path: no wrapped cast, no panic.
    for formula in [
        "=OFFSET(B1,1048576,0)",
        "=OFFSET(B1,0,16384)",
        "=OFFSET(B1,4294967296,0)",
        "=OFFSET(B1,-4294967296,0)",
        "=OFFSET(B1,1E+300,0)",
        "=OFFSET(B1,-1E+300,0)",
        "=SUM(OFFSET(B1,0,0,1048577))",
        "=SUM(OFFSET(B1,0,0,1,16384))",
        "=SUM(OFFSET(B1,0,0,1E+300))",
        "=SUM(OFFSET(B1:B3,1048574,0))",
    ] {
        assert_error(formula, ExcelErrorKind::Ref);
    }
    // The last row and column are still on the sheet.
    assert_number("=ROW(OFFSET(B1,1048575,0))", 1_048_576.0);
    assert_number("=COLUMN(OFFSET(A1,0,16383))", 16_384.0);
    assert_number("=ROWS(OFFSET(B1,0,0,1048576))", 1_048_576.0);
    assert_number("=COLUMNS(OFFSET(A1,0,0,1,16384))", 16_384.0);
}

#[test]
fn offset_sizes_and_offsets_are_numeric_parameters() {
    // D1:D3 are blank: a blank offset is 0.
    assert_number("=OFFSET(B1,D1,0)", 1.0);
    assert_number("=OFFSET(B1,0,D1)", 1.0);
    assert_number("=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,D1:D3,0)))", 3.0);
    // Logicals and numeric text convert; a fraction truncates.
    assert_number("=OFFSET(B1,TRUE,0)", 2.0);
    assert_number("=OFFSET(B1,\"1\",0)", 2.0);
    assert_number("=OFFSET(B1,1.9,0)", 2.0);
    assert_number("=SUM(OFFSET(B1,0,0,\"2\"))", 3.0);
    assert_number("=SUM(OFFSET(B1,0,0,TRUE))", 1.0);
    assert_number("=SUM(OFFSET(B1,0,0,3,FALSE+1))", 6.0);
    // Other text is #VALUE!, from a literal or a cell (C3 is "x").
    assert_error("=OFFSET(B1,\"x\",0)", ExcelErrorKind::Value);
    assert_error("=OFFSET(B1,C3,0)", ExcelErrorKind::Value);
    assert_error("=SUM(OFFSET(B1,0,0,C3))", ExcelErrorKind::Value);
    // An omitted height or width keeps the reference's size; 0 is #REF!.
    assert_number("=SUM(OFFSET(B1:B2,1,0))", 5.0);
    assert_number("=SUM(OFFSET(B1:B2,1,0,,))", 5.0);
    assert_error("=SUM(OFFSET(B1,0,0,0))", ExcelErrorKind::Ref);
    assert_error("=SUM(OFFSET(B1,0,0,1,0))", ExcelErrorKind::Ref);
    assert_error("=SUM(OFFSET(B1,0,0,D1))", ExcelErrorKind::Ref);
}

#[test]
fn n_and_t_read_the_first_cell_of_each_reference() {
    // N(OFFSET(B1,{0;1},0,2)) is {N(B1:B2);N(B2:B3)} = {1;2}.
    assert_number("=SUMPRODUCT(N(OFFSET(B1,{0;1},0,2)))", 3.0);
    assert_number("=SUMPRODUCT(--(T(OFFSET(A2,{0;1},0,2))=\"b\"))", 1.0);
    // T(A1) is "a", T(A2:A3) is T(A2), "b".
    assert_number(
        "=SUMPRODUCT(--(T(INDIRECT({\"A1\";\"A2:A3\"}))=\"b\"))",
        1.0,
    );
    // A range reference is read whole: N(B1:B2) is N(B1).
    assert_number("=N(B2:B3)", 2.0);
    assert_number("=SUMPRODUCT(N(B1:B2))", 1.0);
    assert_number("=SUMPRODUCT(N(B1:C3))", 1.0);
    assert_text("=T(A2:A3)", "b");
    assert_number("=SUMPRODUCT(LEN(T(A1:A3)))", 1.0);
    // An array value is still lifted.
    assert_number("=SUMPRODUCT(N(B1:B3>1))", 2.0);
    assert_number("=SUMPRODUCT(N({1;2;3}))", 6.0);
    assert_number("=SUMPRODUCT(LEN(T({\"ab\",1,\"c\"})))", 3.0);
}

#[test]
fn indirect_of_an_invalid_reference_is_ref_everywhere() {
    // ERROR.TYPE: #REF! is 4, #NAME? would be 5.
    for formula in [
        "=ERROR.TYPE(INDIRECT(\"nosuchname\"))",
        "=ERROR.TYPE(INDIRECT(\"XFE1\"))",
        "=ERROR.TYPE(INDIRECT(\"B1048577\"))",
        "=ERROR.TYPE(N(INDIRECT(\"nosuchname\")))",
        "=ERROR.TYPE(SUBTOTAL(9,INDIRECT(\"nosuchname\")))",
        "=ERROR.TYPE(SUBTOTAL(9,INDIRECT(\"XFE1\")))",
        "=ERROR.TYPE(COUNTIF(INDIRECT(\"nosuchname\"),1))",
        "=ERROR.TYPE(SUMIF(INDIRECT(\"B1048577\"),1))",
        "=ERROR.TYPE(SUM(INDIRECT(\"nosuchname\")))",
    ] {
        assert_number(formula, 4.0);
    }
    // Lifted: value and reference positions alike.
    assert_number(
        "=SUMPRODUCT(ERROR.TYPE(N(INDIRECT({\"nosuchname\";\"XFE1\";\"B1048577\"}))))",
        12.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(ERROR.TYPE(SUBTOTAL(9,INDIRECT({\"B1\";\"nosuchname\"}))),0))",
        4.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(ERROR.TYPE(COUNTIF(INDIRECT({\"A1:A3\";\"XFE1\"}),\"a\")),0))",
        4.0,
    );
    // A defined name still resolves.
    let mut engine = engine();
    engine
        .define_name(
            "Amounts",
            NamedDefinition::Formula {
                ast: parse("=Sheet1!$B$1:$B$3").unwrap(),
                dependencies: Vec::new(),
                range_deps: Vec::new(),
            },
            NameScope::Workbook,
        )
        .unwrap();
    assert_eq!(
        eval(&mut engine, "=SUBTOTAL(9,INDIRECT(\"Amounts\"))"),
        LiteralValue::Number(6.0)
    );
}

#[test]
fn each_indirect_array_element_reads_its_ref_text_like_a_single_call() {
    // ERROR.TYPE: #DIV/0! is 2, #REF! is 4, #N/A is 7. An error element stays
    // that error, a number or logical names no reference, text resolves.
    assert_number(
        "=SUMPRODUCT(IFERROR(ERROR.TYPE(N(INDIRECT({\"B1\";#N/A;5;TRUE;#DIV/0!}))),0))",
        17.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(N(INDIRECT({\"B1\";#N/A;5;\"B3\"})),0))",
        4.0,
    );
    // The #N/A beyond a shorter array, in ref_text or in a1, is returned.
    assert_number(
        "=SUMPRODUCT(IFERROR(ERROR.TYPE(N(INDIRECT({\"B1\",\"B2\"},{TRUE,TRUE,TRUE}))),0))",
        7.0,
    );
    assert_number(
        "=SUMPRODUCT(IFERROR(ERROR.TYPE(N(INDIRECT({\"B1\",\"B2\",\"B3\"},{TRUE,TRUE}))),0))",
        7.0,
    );
}

#[test]
fn only_n_and_t_read_an_array_of_references_as_values() {
    // An array of references has no value: other single-value parameters
    // see #VALUE!, as operators do.
    for formula in [
        "=SUMPRODUCT(ABS(OFFSET(B1,{0;1;2},0)))",
        "=SUMPRODUCT(LEN(OFFSET(A1,{0;1;2},0)))",
        "=SUM(VLOOKUP(OFFSET(A1,{0;1},0),A1:B3,2,0))",
        "=SUMPRODUCT(-OFFSET(B1,{0;1;2},0))",
        "=SUMPRODUCT(OFFSET(B1,{0;1;2},0)+0)",
    ] {
        assert_error(formula, ExcelErrorKind::Value);
    }
    assert_number("=SUMPRODUCT(--ISNUMBER(OFFSET(B1,{0;1},{0,1,2})))", 0.0);
    assert_number("=SUMPRODUCT(--ISERROR(OFFSET(B1,{0;1},0)))", 1.0);
    // Reference parameters still evaluate once per reference.
    assert_number("=SUMPRODUCT(COUNTIF(OFFSET(A1,{0;1;2},0),\"a\"))", 2.0);
}

#[test]
fn arrays_of_references_are_recognised_by_value() {
    // OFFSET's own reference parameter.
    assert_number(
        "=SUMPRODUCT(SUBTOTAL(9,OFFSET(OFFSET(B1,{0;1},0),0,0)))",
        3.0,
    );
    assert_number("=SUMPRODUCT(N(OFFSET(OFFSET(B1,{0;1},0),1,0)))", 5.0);
    // The branch IF or CHOOSE selects.
    assert_number("=SUMPRODUCT(SUBTOTAL(9,IF(1,OFFSET(B1,{0;1;2},0))))", 6.0);
    assert_number("=SUMPRODUCT(SUBTOTAL(9,IF(0,B1,OFFSET(B1,{0;1},0))))", 3.0);
    assert_number(
        "=SUMPRODUCT(SUBTOTAL(9,CHOOSE(2,B1,OFFSET(B1,{0;1;2},0))))",
        6.0,
    );
    assert_number("=SUMPRODUCT(N(IF(TRUE,OFFSET(B1,{0;1},0))))", 3.0);
    // A LET binding.
    assert_number(
        "=LET(r,OFFSET(B1,{0;1;2},0),SUMPRODUCT(SUBTOTAL(9,r)))",
        6.0,
    );
    assert_number("=LET(r,OFFSET(B1,{0;1;2},0),SUMPRODUCT(N(r)))", 6.0);
    assert_error("=LET(r,OFFSET(B1,{0;1;2},0),r)", ExcelErrorKind::Value);
    assert_number("=LET(r,B1:B3,SUM(r))", 6.0);
    // A defined name whose formula is a lifted OFFSET or INDIRECT.
    let mut engine = engine();
    for (name, formula) in [
        ("Refs", "=OFFSET(Sheet1!$B$1,{0;1;2},0)"),
        ("Cells", "=INDIRECT(\"Sheet1!B\"&{1;2})"),
    ] {
        engine
            .define_name(
                name,
                NamedDefinition::Formula {
                    ast: parse(formula).unwrap(),
                    dependencies: Vec::new(),
                    range_deps: Vec::new(),
                },
                NameScope::Workbook,
            )
            .unwrap();
    }
    for (formula, expected) in [
        ("=SUMPRODUCT(SUBTOTAL(9,Refs))", 6.0),
        ("=SUMPRODUCT(N(Refs))", 6.0),
        ("=SUM(COUNTIF(Refs,\">1\"))", 2.0),
        ("=SUMPRODUCT(N(Cells))", 3.0),
    ] {
        assert_eq!(
            eval(&mut engine, formula),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}

#[test]
fn subtotal_aggregate_and_rank_parameters_lift() {
    // The function number lifts like any single-value parameter.
    assert_number("=SUM(SUBTOTAL({9,4},B1:B3))", 9.0);
    assert_number("=SUM(SUBTOTAL({9;4},OFFSET(B1,{0,1},0)))", 6.0);
    assert_number("=SUM(_xlfn.AGGREGATE({9,4},6,B1:B3))", 9.0);
    assert_number("=SUM(_xlfn.AGGREGATE(9,{4,6},B1:B3))", 12.0);
    // k of the array form; in the reference form the 4th argument is a ref.
    assert_number("=SUM(_xlfn.AGGREGATE(14,6,B1:B3,{1,2}))", 5.0);
    assert_number("=SUM(_xlfn.AGGREGATE({14,15},6,B1:B3,{1,1}))", 4.0);
    assert_number("=_xlfn.AGGREGATE(9,4,B1:B3,B1:B2)", 9.0);
    // RANK's ref is a reference parameter.
    assert_number("=SUM(RANK(B1,OFFSET(B1,0,0,{2,3})))", 5.0);
    assert_number("=SUM(_xlfn.RANK.EQ(B2,OFFSET(B1,0,0,{2,3})))", 3.0);
    assert_number("=SUM(_xlfn.RANK.AVG(B2,OFFSET(B1,0,0,{2,3}),1))", 4.0);
}

#[test]
fn row_and_column_of_a_computed_array_keep_errors_and_give_value_otherwise() {
    // Excel (SpreadsheetBench 42354): COLUMN of the computed array
    // IF(D1:F1<>"~N/A",D1:F1) is {#N/A,#VALUE!,#N/A} for {#N/A,44635,#N/A}, so
    // MATCH(3,ERROR.TYPE(...),0) finds the first non-error cell.
    let mut engine = engine();
    let na = LiteralValue::Error(formualizer_common::ExcelError::new_na());
    engine.set_cell_value("Sheet1", 1, 4, na.clone()).unwrap();
    engine
        .set_cell_value("Sheet1", 1, 5, LiteralValue::Number(44635.0))
        .unwrap();
    engine.set_cell_value("Sheet1", 1, 6, na).unwrap();
    let column_types = "ERROR.TYPE(COLUMN(IF(D1:F1<>\"~N/A\",D1:F1)))";
    assert_eq!(
        eval(
            &mut engine,
            &format!("=SUMPRODUCT({column_types}*{{1,10,100}})")
        ),
        LiteralValue::Number(737.0)
    );
    assert_eq!(
        eval(
            &mut engine,
            &format!("=INDEX(D1:F1,MATCH(3,{column_types},0))")
        ),
        LiteralValue::Number(44635.0)
    );
    // ROW lifts the same way over a vertical array: {#N/A;#VALUE!;#VALUE!}.
    assert_number("=SUM(ERROR.TYPE(ROW(IF(B1:B3>1,B1:B3,NA()))))", 13.0);
    // A computed scalar: an error passes through, any other value is #VALUE!.
    assert_number("=ERROR.TYPE(COLUMN(1+1))", 3.0);
    assert_number("=ERROR.TYPE(ROW(1/0))", 2.0);

    // References keep their positions.
    assert_number("=SUM(COLUMN(D1:F1))", 15.0);
    assert_number("=SUM(ROW(IF(TRUE,B1:B3)))", 6.0);
    assert_number("=COLUMN(INDEX(A1:C3,1,2))", 2.0);
    assert_number("=COLUMN(A1:C3 B1:B3)", 2.0);
    assert_number("=ROWS(IF(B1:B3>1,B1:B3))", 3.0);
}

/// The array `formula` spills from J1, `rows` by `cols` cells of it, written
/// as an array constant (`{1,2;3,4}`).
fn spill_in(engine: &mut Engine<TestWorkbook>, formula: &str, rows: u32, cols: u32) -> String {
    eval(engine, formula);
    let text = |value: Option<LiteralValue>| match value {
        Some(LiteralValue::Number(n)) => n.to_string(),
        Some(LiteralValue::Int(i)) => i.to_string(),
        Some(LiteralValue::Error(error)) => error.kind.to_string(),
        other => format!("{other:?}"),
    };
    let rows: Vec<String> = (1..=rows)
        .map(|row| {
            (10..10 + cols)
                .map(|col| text(engine.get_cell_value("Sheet1", row, col)))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    format!("{{{}}}", rows.join(";"))
}

fn spill(formula: &str, rows: u32, cols: u32) -> String {
    spill_in(&mut engine(), formula, rows, cols)
}

/// The test engine with names that hold values (`Konst`, `Calc`, `Konst1`)
/// and one that names a range (`Amounts`).
fn engine_with_value_names() -> Engine<TestWorkbook> {
    let mut engine = engine();
    for (name, formula) in [
        ("Konst", "={1,2,3}"),
        ("Calc", "=Sheet1!$B$1:$B$3*2"),
        ("Konst1", "=5"),
        ("Amounts", "=Sheet1!$B$1:$B$3"),
    ] {
        engine
            .define_name(
                name,
                NamedDefinition::Formula {
                    ast: parse(formula).unwrap(),
                    dependencies: Vec::new(),
                    range_deps: Vec::new(),
                },
                NameScope::Workbook,
            )
            .unwrap();
    }
    engine
}

#[test]
fn row_and_column_of_a_name_holding_a_value_lift_like_a_computed_array() {
    // A name whose formula yields a value, not a reference, is a computed
    // array: not a reference anchored at A1.
    let spill_named = |formula: &str, rows: u32, cols: u32| {
        spill_in(&mut engine_with_value_names(), formula, rows, cols)
    };
    assert_eq!(
        spill_named("=COLUMN(Konst)", 1, 3),
        "{#VALUE!,#VALUE!,#VALUE!}"
    );
    assert_eq!(spill_named("=ROW(Calc)", 3, 1), "{#VALUE!;#VALUE!;#VALUE!}");
    assert_eq!(spill_named("=COLUMN(Konst1)", 1, 1), "{#VALUE!}");
    // A name for a range is that reference.
    assert_eq!(spill_named("=ROW(Amounts)", 3, 1), "{1;2;3}");
    // Functions that take a reference or a value read the name's value.
    let mut engine = engine_with_value_names();
    assert_eq!(
        eval(&mut engine, "=COLUMNS(Konst)"),
        LiteralValue::Number(3.0)
    );
    assert_eq!(
        eval(&mut engine, "=INDEX(Konst,2)"),
        LiteralValue::Number(2.0)
    );
    assert_eq!(
        eval(&mut engine, "=ISREF(Konst)"),
        LiteralValue::Boolean(false)
    );
}

#[test]
fn let_and_lambda_keep_references_for_row_and_column() {
    // LET and LAMBDA keep references, so ROW/COLUMN of a bound range give its
    // row and column numbers.
    assert_eq!(spill("=LET(r,A1:C1,COLUMN(r))", 1, 3), "{1,2,3}");
    assert_number("=LET(r,B2:B3,SUM(ROW(r)))", 5.0);
    assert_eq!(spill("=LET(d,B1:B3,ROW(d)-MIN(ROW(d))+1)", 3, 1), "{1;2;3}");
    assert_eq!(spill("=LET(r,A1:C1,s,r,COLUMN(s))", 1, 3), "{1,2,3}");
    assert_eq!(spill("=LET(r,IF(TRUE,B2:B3),ROW(r))", 2, 1), "{2;3}");
    assert_eq!(spill("=LAMBDA(r,COLUMN(r))(A1:C1)", 1, 3), "{1,2,3}");
    assert_eq!(spill("=LET(f,LAMBDA(r,ROW(r)),f(B2:B3))", 2, 1), "{2;3}");
    // MAP, BYROW and BYCOL give the LAMBDA each cell, row or column of a range
    // as a reference.
    assert_eq!(spill("=BYROW(A1:C3,LAMBDA(r,ROW(r)))", 3, 1), "{1;2;3}");
    assert_eq!(spill("=BYCOL(A1:C3,LAMBDA(c,COLUMN(c)))", 1, 3), "{1,2,3}");
    assert_eq!(spill("=MAP(B2:B3,LAMBDA(x,ROW(x)))", 2, 1), "{2;3}");
    // An eta-reduced function receives the references the same way.
    assert_eq!(spill("=MAP(B2:B3,ROW)", 2, 1), "{2;3}");
    assert_eq!(spill("=BYCOL(A1:C3,COLUMN)", 1, 3), "{1,2,3}");
    // The bound values are the cells' values.
    assert_number("=LET(r,B1:B3,SUM(r))", 6.0);
    assert_number("=SUM(BYROW(B1:C2,LAMBDA(r,SUM(r))))", 90103.0);
    assert_number("=SUM(MAP(B1:B3,LAMBDA(x,x*2)))", 12.0);
    assert_number("=SUM(BYROW({1,2;3,4},LAMBDA(r,SUM(r))))", 10.0);
    // A local bound to a value is a computed array, and functions that take a
    // reference or a value read its value.
    assert_eq!(
        spill("=LET(a,{1,2,3},COLUMN(a))", 1, 3),
        "{#VALUE!,#VALUE!,#VALUE!}"
    );
    assert_number("=LET(a,{1,2;3,4},VLOOKUP(3,a,2,0))", 4.0);
    assert_number("=LET(a,{1,2,3},MATCH(2,a,0))", 2.0);
    assert_number("=LET(r,B1:B3,MATCH(2,r,0))", 2.0);
    assert_number("=LET(r,B1:B3,ROWS(r))", 3.0);
}

#[test]
fn let_and_lambda_keep_references_when_evaluated_from_the_formula_tree() {
    crate::builtins::load_builtins();
    let mut wb = TestWorkbook::new();
    for row in 1..=3 {
        for col in 1..=3 {
            wb = wb.with_cell("Sheet1", row, col, LiteralValue::Number((row * col) as f64));
        }
    }
    let interpreter = wb.interpreter();
    for (formula, expected) in [
        ("=SUM(LET(r,B2:D2,COLUMN(r)))", 9.0),
        ("=SUM(LAMBDA(r,ROW(r))(B2:B3))", 5.0),
        ("=LET(f,LAMBDA(r,COLUMN(r)),SUM(f(B1:C1)))", 5.0),
        ("=SUM(BYROW(A2:C3,LAMBDA(r,ROW(r))))", 5.0),
        ("=SUM(MAP(B2:B3,LAMBDA(x,ROW(x))))", 5.0),
        ("=SUM(LET(r,B2:B3,r))", 10.0),
    ] {
        let ast = parse(formula).unwrap();
        assert_eq!(
            interpreter.evaluate_ast(&ast).unwrap().into_literal(),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}

#[test]
fn row_and_column_of_an_empty_array_are_calc() {
    // Excel has no empty array (an empty result is #CALC!), so an
    // engine-internal 0x0 array gives #CALC!, not a cell with no value.
    use formualizer_parse::parser::{ASTNode, ASTNodeType};
    crate::builtins::load_builtins();
    let wb = TestWorkbook::new();
    let interpreter = wb.interpreter();
    for name in ["ROW", "COLUMN"] {
        let empty = ASTNode::new(ASTNodeType::Literal(LiteralValue::Array(Vec::new())), None);
        let call = ASTNode::new(
            ASTNodeType::Function {
                name: name.to_string(),
                args: vec![empty],
            },
            None,
        );
        let value = interpreter.evaluate_ast(&call).unwrap().into_literal();
        assert!(
            matches!(&value, LiteralValue::Error(error) if error.kind == ExcelErrorKind::Calc),
            "{name}: {value:?}"
        );
    }
}

#[test]
fn row_and_column_of_an_array_of_references_give_one_result_per_reference() {
    // OFFSET and INDIRECT lifted over an array return an array of references;
    // ROW and COLUMN apply to each reference, and an element that is no
    // reference (an offset above row 1) keeps its #REF!.
    assert_eq!(spill("=COLUMN(OFFSET(A1,0,{0,1,2}))", 1, 3), "{1,2,3}");
    assert_eq!(spill("=ROW(INDIRECT({\"A1\";\"C3\"}))", 2, 1), "{1;3}");
    assert_eq!(spill("=ROW(OFFSET(A1,{-1;0;1},0))", 3, 1), "{#REF!;1;2}");
    assert_eq!(spill("=COLUMN(OFFSET(C1,0,{-3,-1}))", 1, 2), "{#REF!,2}");
    assert_number("=SUM(ROW(OFFSET(A1,{1;2;3},0)))", 9.0);
    // The same array of references through IF, a LET name or a defined name.
    assert_eq!(spill("=ROW(IF(TRUE,OFFSET(A1,{0;2},0)))", 2, 1), "{1;3}");
    assert_eq!(spill("=LET(r,OFFSET(A1,{1;2},0),ROW(r))", 2, 1), "{2;3}");
    let mut engine = engine();
    engine
        .define_name(
            "Steps",
            NamedDefinition::Formula {
                ast: parse("=OFFSET(Sheet1!$A$1,0,{1,2})").unwrap(),
                dependencies: Vec::new(),
                range_deps: Vec::new(),
            },
            NameScope::Workbook,
        )
        .unwrap();
    assert_eq!(spill_in(&mut engine, "=COLUMN(Steps)", 1, 2), "{2,3}");
}

/// `engine_with_value_names` plus names that alias a name or choose with IF.
fn engine_with_aliased_names() -> Engine<TestWorkbook> {
    let mut engine = engine_with_value_names();
    for (name, formula) in [
        ("AliasValue", "=Konst"),
        ("TrueValue", "=IF(TRUE,{1,2,3})"),
        ("ChoiceScalar", "=IF(TRUE,42)"),
        ("AliasRange", "=Amounts"),
        ("TrueRange", "=IF(TRUE,Sheet1!$B$2:$B$3)"),
        ("IndirectRange", "=INDIRECT(\"Amounts\")"),
    ] {
        engine
            .define_name(
                name,
                NamedDefinition::Formula {
                    ast: parse(formula).unwrap(),
                    dependencies: Vec::new(),
                    range_deps: Vec::new(),
                },
                NameScope::Workbook,
            )
            .unwrap();
    }
    engine
}

#[test]
fn a_name_holds_a_value_unless_its_formula_evaluates_to_a_reference() {
    // Aliasing a name that holds a value, or producing the value through IF,
    // does not give it worksheet coordinates.
    let spill_named = |formula: &str, rows: u32, cols: u32| {
        spill_in(&mut engine_with_aliased_names(), formula, rows, cols)
    };
    assert_eq!(
        spill_named("=COLUMN(AliasValue)", 1, 3),
        "{#VALUE!,#VALUE!,#VALUE!}"
    );
    assert_eq!(
        spill_named("=COLUMN(TrueValue)", 1, 3),
        "{#VALUE!,#VALUE!,#VALUE!}"
    );
    assert_eq!(spill_named("=COLUMN(ChoiceScalar)", 1, 1), "{#VALUE!}");
    // A name whose formula evaluates to a reference is that reference.
    assert_eq!(spill_named("=ROW(AliasRange)", 3, 1), "{1;2;3}");
    assert_eq!(spill_named("=ROW(TrueRange)", 2, 1), "{2;3}");
    assert_eq!(spill_named("=ROW(IndirectRange)", 3, 1), "{1;2;3}");
    // Their values read as before.
    let mut engine = engine_with_aliased_names();
    for (formula, expected) in [
        ("=SUM(AliasValue)", LiteralValue::Number(6.0)),
        ("=SUM(TrueValue)", LiteralValue::Number(6.0)),
        ("=ChoiceScalar+1", LiteralValue::Number(43.0)),
        ("=SUM(AliasRange)", LiteralValue::Number(6.0)),
        ("=SUM(TrueRange)", LiteralValue::Number(5.0)),
        ("=ISREF(AliasValue)", LiteralValue::Boolean(false)),
        ("=ISREF(ChoiceScalar)", LiteralValue::Boolean(false)),
        ("=ISREF(AliasRange)", LiteralValue::Boolean(true)),
    ] {
        assert_eq!(eval(&mut engine, formula), expected, "{formula}");
    }
}

#[test]
fn row_and_column_propagate_cancellation() {
    // Cancellation aborts the formula; it is not a worksheet error of ROW or
    // COLUMN's argument.
    #[derive(Debug)]
    struct Abort;
    impl crate::function::Function for Abort {
        fn name(&self) -> &'static str {
            "ABORT"
        }
        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [crate::traits::ArgumentHandle<'a, 'b>],
            _ctx: &dyn crate::traits::FunctionContext<'b>,
        ) -> Result<crate::traits::CalcValue<'b>, formualizer_common::ExcelError> {
            Err(formualizer_common::ExcelError::new(
                ExcelErrorKind::Cancelled,
            ))
        }
    }
    crate::builtins::load_builtins();
    let wb = TestWorkbook::new().with_function(std::sync::Arc::new(Abort));
    let interpreter = wb.interpreter();
    for formula in [
        "=ABORT()",
        "=COLUMN(ABORT())",
        "=ROW(ABORT())",
        "=ROW(OFFSET(A1,ABORT(),0))",
        "=COLUMN(IF(TRUE,ABORT()))",
    ] {
        let result = interpreter.evaluate_ast(&parse(formula).unwrap());
        assert!(
            matches!(&result, Err(error) if error.kind == ExcelErrorKind::Cancelled),
            "{formula}: {:?}",
            result.map(|value| value.into_literal())
        );
    }
}
