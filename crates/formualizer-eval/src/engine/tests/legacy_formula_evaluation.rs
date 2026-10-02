//! Formulas entered without the array flag, under the declared array semantics
//! of a workbook file: Excel evaluates them as values throughout, not only in
//! their final result.
use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// A1:A3 = {1;0;1}, B1:B3 = {10;20;30}; each formula is placed at its
/// (row, col) and evaluated, with `legacy` selecting the file semantics.
fn evaluate(legacy: bool, formulas: &[(u32, u32, &str)]) -> Vec<Option<LiteralValue>> {
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig {
            enable_parallel: false,
            ..Default::default()
        },
    );
    for (row, a, b) in [(1, 1.0, 10.0), (2, 0.0, 20.0), (3, 1.0, 30.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(a))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(b))
            .unwrap();
    }
    for &(row, col, formula) in formulas {
        engine
            .set_cell_formula("Sheet1", row, col, parse(formula).unwrap())
            .unwrap();
    }
    if legacy {
        engine.use_legacy_array_semantics();
    }
    engine.evaluate_all().unwrap();
    formulas
        .iter()
        .map(|&(row, col, _)| engine.get_cell_value("Sheet1", row, col))
        .collect()
}

fn legacy(row: u32, formula: &str) -> Option<LiteralValue> {
    evaluate(true, &[(row, 4, formula)]).remove(0)
}

fn number(n: f64) -> Option<LiteralValue> {
    Some(LiteralValue::Number(n))
}

fn error_kind(value: Option<LiteralValue>) -> ExcelErrorKind {
    match value {
        Some(LiteralValue::Error(error)) => error.kind,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn operator_operands_intersect_with_the_formula_cell() {
    assert_eq!(legacy(2, "=A1:A3*2"), number(0.0));
    assert_eq!(legacy(3, "=SUM((A1:A3=1)*B1:B3)"), number(30.0));
    assert_eq!(
        legacy(2, "=IF(A:A=0,\"zero\",\"one\")"),
        Some(LiteralValue::Text("zero".into()))
    );
    assert_eq!(
        error_kind(legacy(5, "=SUM(A1:A3*B1:B3)")),
        ExcelErrorKind::Value
    );
    // A horizontal range intersects the formula's column.
    assert_eq!(
        evaluate(true, &[(1, 5, "=1"), (1, 6, "=2"), (4, 6, "=E1:F1*10")])[2],
        number(20.0)
    );
}

#[test]
fn if_tests_one_value_and_returns_its_whole_range() {
    // Row 2: A2=0, so IF returns all of B1:B3 to MAX.
    assert_eq!(legacy(2, "=MAX(IF(A1:A3=0,B1:B3))"), number(30.0));
    assert_eq!(legacy(3, "=MAX(IF(A1:A3=0,B1:B3))"), number(0.0));
    assert_eq!(
        legacy(2, "=IF(A1:A3,\"yes\",\"no\")"),
        Some(LiteralValue::Text("no".into()))
    );
}

#[test]
fn single_value_arguments_intersect() {
    assert_eq!(legacy(2, "=SUMIF(A1:A3,A1:A3,B1:B3)"), number(20.0));
    // No row of A1:A3 at row 5: the criterion is #VALUE!, which no cell matches.
    assert_eq!(legacy(5, "=SUMIFS(B1:B3,A1:A3,A1:A3)"), number(0.0));
    assert_eq!(
        legacy(3, "=TEXT(B1:B3,\"0.0\")"),
        Some(LiteralValue::Text("30.0".into()))
    );
}

#[test]
fn array_arguments_keep_array_evaluation() {
    assert_eq!(legacy(5, "=SUMPRODUCT((A1:A3=1)*B1:B3)"), number(40.0));
    assert_eq!(legacy(5, "=SUMPRODUCT(--(A1:A3=1))"), number(2.0));
    assert_eq!(legacy(5, "=LOOKUP(2,1/(A1:A3=1),B1:B3)"), number(30.0));
    assert_eq!(
        legacy(5, "=MATCH(1,INDEX((A1:A3=1)*(B1:B3>15),0),0)"),
        number(3.0)
    );
    assert_eq!(legacy(5, "=SUM(COUNTIF(A1:A3,{0,1}))"), number(3.0));
    assert_eq!(legacy(5, "=SUM(B1:B3)"), number(60.0));
}

#[test]
fn if_and_iferror_inside_an_array_argument_test_one_value() {
    assert_eq!(
        error_kind(legacy(5, "=SUMPRODUCT(IF(A1:A3=1,B1:B3,0))")),
        ExcelErrorKind::Value
    );
    assert_eq!(legacy(5, "=SUMPRODUCT(IFERROR(1/A1:A3,0))"), number(0.0));
    assert_eq!(
        error_kind(legacy(5, "=MATCH(1,(A1:A3=1)*(B1:B3>=10),0)")),
        ExcelErrorKind::Value
    );
}

#[test]
fn match_does_not_search_the_single_value_an_intersection_leaves() {
    // Row 2 intersects A2=0 and B2=20: MATCH's lookup_array is one value,
    // which MATCH does not search. Text that is not a number is #VALUE!
    // (the classic INDEX/MATCH on joined columns without array entry).
    assert_eq!(
        error_kind(legacy(2, "=MATCH(\"0x\",A1:A3&\"x\",0)")),
        ExcelErrorKind::Value
    );
    assert_eq!(
        error_kind(legacy(2, "=INDEX(B1:B3,MATCH(\"0x\",A1:A3&\"x\",0))")),
        ExcelErrorKind::Value
    );
    // A number, a logical or numeric text ("020") is #N/A, even when equal.
    assert_eq!(
        error_kind(legacy(2, "=MATCH(1,(A1:A3=0)*(B1:B3=20),0)")),
        ExcelErrorKind::Na
    );
    assert_eq!(
        error_kind(legacy(2, "=MATCH(TRUE,A1:A3=0,0)")),
        ExcelErrorKind::Na
    );
    assert_eq!(
        error_kind(legacy(2, "=MATCH(\"020\",A1:A3&B1:B3,0)")),
        ExcelErrorKind::Na
    );
    // Array entry and an array argument search the whole expression.
    assert_eq!(
        evaluate(false, &[(2, 4, "=MATCH(\"0x\",A1:A3&\"x\",0)")])[0],
        number(2.0)
    );
    assert_eq!(
        legacy(2, "=MATCH(\"0x\",INDEX(A1:A3&\"x\",0),0)"),
        number(2.0)
    );
}

#[test]
fn match_searches_references_and_arrays_but_not_single_values() {
    for legacy_file in [true, false] {
        let results = evaluate(
            legacy_file,
            &[
                (5, 4, "=MATCH(1,1,0)"),
                (5, 5, "=MATCH(TRUE,TRUE,0)"),
                (5, 6, "=MATCH(\"1\",\"1\",0)"),
                (5, 7, "=MATCH(\"a\",\"a\",0)"),
                (5, 8, "=MATCH(0,A2&\"\",0)"),
                (5, 9, "=MATCH(\"0x\",A2&\"x\",0)"),
                (5, 10, "=MATCH(1,1/0,0)"),
                // A one-cell reference and a one-element array, also one an
                // array function returns or an expression computes from one,
                // are searched.
                (5, 11, "=MATCH(0,A2,0)"),
                (5, 12, "=MATCH(\"a\",{\"a\"},0)"),
                (5, 13, "=MATCH(20,B2:B2,0)"),
                (5, 14, "=MATCH(1,SEQUENCE(1),0)"),
                (5, 15, "=MATCH(1,TRANSPOSE(A1),0)"),
                (5, 16, "=MATCH(10,INDEX(B1:B3,1),0)"),
                (5, 17, "=MATCH(1,{1}+0,0)"),
                (5, 18, "=MATCH(1,ABS({-1}),0)"),
                (5, 19, "=MATCH(\"a\",LOWER({\"A\"}),0)"),
                (5, 20, "=MATCH(1,LET(x,SEQUENCE(1),x),0)"),
                (5, 21, "=MATCH(1,IF(TRUE,SEQUENCE(1),0),0)"),
            ],
        );
        let kinds: Vec<_> = results[..7]
            .iter()
            .map(|value| error_kind(value.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Value,
                ExcelErrorKind::Na,
                ExcelErrorKind::Value,
                ExcelErrorKind::Div,
            ],
            "legacy={legacy_file}"
        );
        assert!(
            results[7..].iter().all(|value| *value == number(1.0)),
            "legacy={legacy_file}: {:?}",
            &results[7..]
        );
    }
}

#[test]
fn row_and_column_return_their_first_index_as_values() {
    assert_eq!(legacy(5, "=SUM(ROW(A1:A3))"), number(1.0));
    assert_eq!(legacy(5, "=SUM(COLUMN(A1:C1))"), number(1.0));
    assert_eq!(legacy(5, "=SUMPRODUCT(ROW(A1:A3))"), number(6.0));
    // Row 3 tests A3=1 and ROW gives 1, so SMALL has no second value.
    assert_eq!(
        error_kind(legacy(3, "=SMALL(IF(A1:A3=1,ROW(A1:A3)),2)")),
        ExcelErrorKind::Num
    );
}

#[test]
fn array_formulas_and_undeclared_engines_evaluate_arrays() {
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig {
            enable_parallel: false,
            ..Default::default()
        },
    );
    for (row, a, b) in [(1, 1.0, 10.0), (2, 0.0, 20.0), (3, 1.0, 30.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(a))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(b))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 5, 4, parse("=SUM(A1:A3*B1:B3)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 5, 5, parse("=MAX(IF(A1:A3=1,B1:B3))").unwrap())
        .unwrap();
    engine.use_legacy_array_semantics();
    engine.declare_array_formula("Sheet1", 5, 4, 1, 1, false);
    engine.declare_array_formula("Sheet1", 5, 5, 1, 1, true);
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 5, 4), number(40.0));
    assert_eq!(engine.get_cell_value("Sheet1", 5, 5), number(30.0));

    let values = evaluate(
        false,
        &[
            (5, 4, "=SUM(A1:A3*B1:B3)"),
            (2, 5, "=MAX(IF(A1:A3=0,B1:B3))"),
        ],
    );
    assert_eq!(values, vec![number(40.0), number(20.0)]);
}

#[test]
fn intersected_cells_keep_blanks_and_single_cells_need_no_intersection() {
    // A4 is blank: it compares equal to "" and concatenates as "".
    assert_eq!(
        legacy(4, "=A1:A4&\"x\""),
        Some(LiteralValue::Text("x".into()))
    );
    assert_eq!(
        legacy(4, "=IF(A1:A4=\"\",\"blank\",\"value\")"),
        Some(LiteralValue::Text("blank".into()))
    );
    assert_eq!(legacy(5, "=B3:B3+1"), number(31.0));
}

#[test]
fn ranges_returned_by_functions_intersect_in_single_value_positions() {
    assert_eq!(
        legacy(2, "=IF(IF(TRUE,A1:A3),\"yes\",\"no\")"),
        Some(LiteralValue::Text("no".into()))
    );
    assert_eq!(legacy(3, "=INDEX(B1:B3,0)+1"), number(31.0));
    assert_eq!(legacy(5, "=SUM(INDEX(B1:B3,0))"), number(60.0));
}

#[test]
fn reference_parameters_lift_over_arrays_of_references_next_to_intersections() {
    // Inside SUMPRODUCT, ROW(A2:A4)-1 is {1;2;3}: OFFSET gives the array of
    // references {A2;A3;A4} and SUBTOTAL counts each one (A4 is blank).
    assert_eq!(
        legacy(5, "=SUMPRODUCT(SUBTOTAL(3,OFFSET(A1,ROW(A2:A4)-1,0)))"),
        number(2.0)
    );
    assert_eq!(
        legacy(
            5,
            "=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0))*{1;10;100})"
        ),
        number(3210.0)
    );
    // The same formula intersects A1:A3 in the operand next to SUMPRODUCT
    // (row 3: A3 = 1) and keeps lifting inside it.
    assert_eq!(
        legacy(
            3,
            "=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))+A1:A3*100"
        ),
        number(160.0)
    );
    // Outside an array argument ROW gives its first row, so there is one
    // reference; an array constant still makes an array of references.
    assert_eq!(
        legacy(5, "=SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))"),
        number(10.0)
    );
    assert_eq!(
        legacy(5, "=SUM(SUBTOTAL(9,OFFSET(B1,{0;1;2},0)))"),
        number(60.0)
    );
    assert_eq!(
        legacy(5, "=SUM(COUNTIF(OFFSET(A1,{0;1;2},0),1))"),
        number(2.0)
    );
    // OFFSET's rows is a single value: a range there is intersected with the
    // formula cell (row 2: A2 = 0, row 3: A3 = 1, row 5: #VALUE!), as when
    // OFFSET is evaluated on its own, and kept whole inside SUMPRODUCT.
    assert_eq!(legacy(2, "=SUBTOTAL(9,OFFSET(B1,A1:A3,0))"), number(10.0));
    assert_eq!(legacy(3, "=SUBTOTAL(9,OFFSET(B1,A1:A3,0))"), number(20.0));
    assert_eq!(
        error_kind(legacy(5, "=SUBTOTAL(9,OFFSET(B1,A1:A3,0))")),
        ExcelErrorKind::Value
    );
    assert_eq!(
        legacy(5, "=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,A1:A3,0)))"),
        number(50.0)
    );
    // IF tests one value (row 3: A3 = 1) and passes the selected array of
    // references on; row 2 selects B1.
    let branch = "=SUMPRODUCT(SUBTOTAL(9,IF(A1:A3,OFFSET(B1,{0;1;2},0),B1)))";
    assert_eq!(legacy(3, branch), number(60.0));
    assert_eq!(legacy(2, branch), number(10.0));
    // N reads each reference's first cell, and a range by its top-left cell
    // rather than by intersection.
    assert_eq!(
        legacy(5, "=SUMPRODUCT(N(OFFSET(B1,ROW(B1:B3)-1,0)))"),
        number(60.0)
    );
    assert_eq!(legacy(3, "=N(B2:B3)"), number(20.0));
    // Array formulas and engines without the declared semantics are unchanged.
    assert_eq!(
        evaluate(
            false,
            &[
                (5, 4, "=SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))"),
                (5, 5, "=SUMPRODUCT(SUBTOTAL(9,OFFSET(B1,A1:A3,0)))"),
            ],
        ),
        vec![number(60.0), number(50.0)]
    );
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig {
            enable_parallel: false,
            ..Default::default()
        },
    );
    for (row, a, b) in [(1, 1.0, 10.0), (2, 0.0, 20.0), (3, 1.0, 30.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(a))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(b))
            .unwrap();
    }
    for (col, formula) in [
        (4, "=SUM(SUBTOTAL(9,OFFSET(B1,ROW(B1:B3)-1,0)))"),
        (5, "=SUM(SUBTOTAL(9,OFFSET(B1,A1:A3,0)))"),
    ] {
        engine
            .set_cell_formula("Sheet1", 5, col, parse(formula).unwrap())
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    engine.declare_array_formula("Sheet1", 5, 4, 1, 1, false);
    engine.declare_array_formula("Sheet1", 5, 5, 1, 1, true);
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 5, 4), number(60.0));
    assert_eq!(engine.get_cell_value("Sheet1", 5, 5), number(50.0));
}

#[test]
fn lambda_bodies_keep_array_evaluation() {
    assert_eq!(
        legacy(5, "=LAMBDA(x,SUMPRODUCT(ROW(A1:A3)))(0)"),
        number(6.0)
    );
}

#[test]
fn let_names_bound_to_ranges_intersect_like_the_range() {
    // A LET name bound to a range is that range: in a single-value position
    // (IF's test) or as an operand it intersects with the formula cell, as
    // A1:A3 written there does (row 2: A2 = 0; row 3: A3 = 1).
    assert_eq!(
        legacy(2, "=LET(r,A1:A3,IF(r,\"yes\",\"no\"))"),
        legacy(2, "=IF(A1:A3,\"yes\",\"no\")")
    );
    assert_eq!(legacy(2, "=LET(r,A1:A3,IF(r,\"yes\",\"no\"))"), text("no"));
    assert_eq!(legacy(3, "=LET(r,A1:A3,IF(r,\"yes\",\"no\"))"), text("yes"));
    assert_eq!(
        legacy(2, "=LET(r,A1:A3,IF(r=0,\"yes\",\"no\"))"),
        text("yes")
    );
    assert_eq!(
        error_kind(legacy(5, "=LET(r,A1:A3,IF(r,\"yes\",\"no\"))")),
        ExcelErrorKind::Value
    );
    // A reference parameter still takes the whole range.
    assert_eq!(legacy(5, "=LET(r,B1:B3,SUM(r))"), number(60.0));
    assert_eq!(legacy(5, "=LET(r,A1:A3,COUNTIFS(r,1))"), number(2.0));
}

#[test]
fn count_does_not_count_an_intersected_error() {
    // COUNT does not count error values, so an operand intersected to one
    // error counts 0 instead of returning the error; the
    // IF(COUNT(SEARCH(..)),..) idiom depends on this.
    assert_eq!(legacy(2, "=COUNT(SEARCH(\"q\",B1:B3))"), number(0.0));
    assert_eq!(
        legacy(2, "=IF(COUNT(MATCH(A1:A3,{5},0)),\"hit\",\"miss\")"),
        Some(LiteralValue::Text("miss".into()))
    );
    assert_eq!(legacy(2, "=COUNT(A1:A3/0)"), number(0.0));
    // No row of A1:A3 at row 5: the operand is #VALUE!, which is not counted.
    assert_eq!(legacy(5, "=COUNT(A1:A3*1)"), number(0.0));
    assert_eq!(legacy(2, "=COUNT(A1:A3*1)"), number(1.0));
    // COUNTA does count the error value.
    assert_eq!(legacy(5, "=COUNTA(A1:A3*1)"), number(1.0));
}

fn text(s: &str) -> Option<LiteralValue> {
    Some(LiteralValue::Text(s.into()))
}

#[test]
fn single_value_parameters_of_dynamic_array_functions_intersect() {
    // Dynamic-array Excel shows `@` before these ranges in a formula saved
    // without the array flag: the looked-up value intersects (row 2: A2 = 0).
    assert_eq!(legacy(2, "=XLOOKUP(A1:A3,A1:A3,B1:B3)"), number(20.0));
    assert_eq!(legacy(2, "=SUM(XLOOKUP(A1:A3,A1:A3,B1:B3))"), number(20.0));
    assert_eq!(legacy(2, "=SUM(XMATCH(A1:A3,A1:A3))"), number(2.0));
    assert_eq!(
        legacy(3, "=TEXTBEFORE(B1:B3&\"-x\",\"-\")&\"!\""),
        text("30!")
    );
    assert_eq!(
        error_kind(legacy(5, "=XLOOKUP(A1:A3,A1:A3,B1:B3)")),
        ExcelErrorKind::Value
    );
    // The lookup and return arrays stay arrays, and inside an array argument
    // the looked-up value lifts.
    assert_eq!(
        legacy(5, "=XLOOKUP(1,(A1:A3=0)*(B1:B3>10),B1:B3)"),
        number(20.0)
    );
    assert_eq!(
        legacy(5, "=SUMPRODUCT(XLOOKUP(A1:A3,A1:A3,B1:B3))"),
        number(40.0)
    );
    assert_eq!(
        evaluate(false, &[(2, 4, "=SUM(XLOOKUP(A1:A3,A1:A3,B1:B3))")]),
        vec![number(40.0)]
    );
}

#[test]
fn scalar_parameters_follow_the_argument_schema() {
    // Single-value parameters that `lift_spec` does not list intersect too.
    assert_eq!(
        legacy(2, "=SUM(T.DIST(B1:B3,5,TRUE))"),
        legacy(2, "=T.DIST(B2,5,TRUE)")
    );
    assert_eq!(
        error_kind(legacy(5, "=T.DIST(B1:B3,5,TRUE)")),
        ExcelErrorKind::Value
    );
    assert_eq!(legacy(2, "=IFS(A1:A3,\"t\",TRUE,\"f\")"), text("f"));
    assert_eq!(legacy(3, "=IFS(A1:A3,\"t\",TRUE,\"f\")"), text("t"));
    assert_eq!(legacy(2, "=SUM(SWITCH(A1:A3,0,100,1,1))"), number(100.0));
    assert_eq!(legacy(2, "=TRIMMEAN(B1:B3,A1:A3)"), number(20.0));
    // Parameters that take ranges, arrays or references keep them whole.
    assert_eq!(legacy(5, "=COUNTA(A1:A3)"), number(3.0));
    assert_eq!(legacy(5, "=AND(A1:A3)"), Some(LiteralValue::Boolean(false)));
    assert_eq!(legacy(5, "=OR(A1:A3)"), Some(LiteralValue::Boolean(true)));
    assert_eq!(
        legacy(5, "=ISREF(A1:A3)"),
        Some(LiteralValue::Boolean(true))
    );
    assert_eq!(legacy(5, "=TEXTJOIN(\"-\",TRUE,B1:B3)"), text("10-20-30"));
    assert_eq!(legacy(5, "=SUMIF(A1:A3,1,B1:B3)"), number(40.0));
    assert_eq!(legacy(5, "=SUM(B1:B3)"), number(60.0));
}

#[test]
fn lookup_values_of_vlookup_and_hlookup_are_single_values_inside_arrays() {
    // Like INDEX's row and column, VLOOKUP's looked-up value is a single value
    // even inside SUMPRODUCT: an array of them needs array entry.
    assert_eq!(
        error_kind(legacy(5, "=SUMPRODUCT(VLOOKUP(A1:A3,A1:B3,2,0))")),
        ExcelErrorKind::Value
    );
    assert_eq!(
        legacy(2, "=SUMPRODUCT(VLOOKUP(A1:A3,A1:B3,2,0))"),
        number(20.0)
    );
    // MATCH's looked-up value still lifts there, and array formulas and
    // engines without the declared semantics are unchanged.
    assert_eq!(legacy(5, "=SUMPRODUCT(MATCH(A1:A3,A1:A3,0))"), number(4.0));
    assert_eq!(
        evaluate(false, &[(5, 4, "=SUMPRODUCT(VLOOKUP(A1:A3,A1:B3,2,0))")]),
        vec![number(40.0)]
    );
}

#[test]
fn rows_and_columns_take_a_reference() {
    // A range passes whole; an operand intersects (row 2).
    assert_eq!(legacy(5, "=ROWS(A1:A3)"), number(3.0));
    assert_eq!(legacy(5, "=COLUMNS(A1:B3)"), number(2.0));
    assert_eq!(legacy(2, "=ROWS(A1:A3*1)"), number(1.0));
    assert_eq!(legacy(5, "=ROWS(INDEX(A1:B3,0,1))"), number(3.0));
}

#[test]
fn intersecting_a_single_blank_cell_keeps_it_blank() {
    assert_eq!(legacy(4, "=@A4&\"x\""), text("x"));
    assert_eq!(legacy(4, "=@A1:A4&\"x\""), text("x"));
    assert_eq!(legacy(4, "=@A3&\"x\""), text("1x"));
    assert_eq!(legacy(4, "=@A4+1"), number(1.0));
    assert_eq!(legacy(4, "=@A4"), legacy(4, "=A4"));
    assert_eq!(evaluate(false, &[(4, 4, "=@A4&\"x\"")]), vec![text("x")]);
}

#[test]
fn an_empty_if_slot_is_empty_text_to_ampersand_with_an_intersected_test() {
    // IF's test intersects as it does for IF itself (A1=1 at row 1, A2=0 at
    // row 2), and the empty slot it selects reads as "" to `&`.
    assert_eq!(legacy(1, "=IF(A1:A3=0,\"not \",)&\"ok\""), text("ok"));
    assert_eq!(legacy(2, "=IF(A1:A3=0,\"not \",)&\"ok\""), text("not ok"));
    assert_eq!(legacy(1, "=IF(A1:A3,,\"x\")&\"ok\""), text("ok"));
    assert_eq!(legacy(2, "=IF(A1:A3,,\"x\")&\"ok\""), text("xok"));
    assert_eq!(legacy(1, "=LEN(IF(A1:A3,,\"x\"))"), number(0.0));
    // A range IF selects intersects as an operand of `&`.
    assert_eq!(legacy(2, "=IF(TRUE,B1:B3,)&\"x\""), text("20x"));
    // Inside an array argument IF still tests one value.
    assert_eq!(
        legacy(2, "=SUMPRODUCT(LEN(IF(A1:A3=1,\"ab\",)&\"c\"))"),
        number(1.0)
    );
    // Outside text the slot stays IF's 0.
    assert_eq!(legacy(1, "=IF(A1:A3=0,1,)+5"), number(5.0));
    assert_eq!(legacy(1, "=COUNT(IF(A1:A3=0,1,))"), number(1.0));
}
