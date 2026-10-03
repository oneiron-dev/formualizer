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
fn match_searches_a_one_element_array_bound_to_a_name_or_returned_by_a_lambda() {
    for legacy_file in [true, false] {
        let results = evaluate(
            legacy_file,
            &[
                // A LET name bound to a one-element array, and a calculation
                // over it, is still that array.
                (5, 4, "=LET(x,{1}+0,MATCH(1,x+0,0))"),
                (5, 5, "=LET(x,SEQUENCE(1),MATCH(1,ABS(x),0))"),
                (5, 6, "=LET(x,{1},y,x*1,MATCH(1,y,0))"),
                (5, 7, "=LET(x,{1}+0,LET(y,1,MATCH(1,x*y,0)))"),
                (5, 8, "=MATCH(1,LET(x,{1}+0,x+0),0)"),
                // A LAMBDA returning one, called through a name or in place.
                (5, 9, "=LET(f,LAMBDA(z,{1}+0),MATCH(1,f(0),0))"),
                (5, 10, "=MATCH(1,LET(f,LAMBDA(z,z+0),f({1})),0)"),
                (5, 11, "=MATCH(1,LAMBDA(z,SEQUENCE(z))(1),0)"),
                // The name itself is the array, not a reference.
                (5, 12, "=LET(x,{1},MATCH(1,x,0))"),
                (5, 13, "=LET(x,{1}+0,MATCH(1,x,0))"),
            ],
        );
        assert!(
            results.iter().all(|value| *value == number(1.0)),
            "legacy={legacy_file}: {results:?}"
        );
        assert_eq!(
            evaluate(legacy_file, &[(5, 4, "=LET(x,{1,2},MATCH(2,x,0))")])[0],
            number(2.0),
            "legacy={legacy_file}"
        );
        // Names and LAMBDAs holding single values are single values.
        let results = evaluate(
            legacy_file,
            &[
                (6, 4, "=LET(x,1,MATCH(1,x+0,0))"),
                (6, 5, "=LET(x,SUM({1}),MATCH(1,x,0))"),
                (6, 6, "=LET(f,LAMBDA(z,z+0),MATCH(1,f(1),0))"),
                (6, 7, "=LET(f,LAMBDA(z,{1}+0),MATCH(1,SUM(f(0)),0))"),
                (6, 8, "=LET(f,LAMBDA(z,1),MATCH(1,f(0),0))"),
                (6, 9, "=LET(x,1,MATCH(1,x,0))"),
            ],
        );
        let kinds: Vec<_> = results.into_iter().map(error_kind).collect();
        assert_eq!(
            kinds,
            [
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
            ],
            "legacy={legacy_file}"
        );
    }
}

#[test]
fn match_does_not_search_a_single_value_computed_from_arrays() {
    for legacy_file in [true, false] {
        let results = evaluate(
            legacy_file,
            &[
                // A function that consumes an array returns a single value.
                (7, 4, "=MATCH(1,SUM({1}),0)"),
                (7, 5, "=MATCH(\"x\",TEXTJOIN(\"\",TRUE,{\"x\"}),0)"),
                (7, 6, "=MATCH(1,ROWS(SEQUENCE(1)),0)"),
                // IF and CHOOSE return the argument they select.
                (7, 7, "=MATCH(1,IF(FALSE,SEQUENCE(1),1),0)"),
                (7, 8, "=MATCH(1,CHOOSE(2,{1},1),0)"),
                (7, 9, "=MATCH(1,IF(A1=1,1,{1}),0)"),
                // LET returns its calculation, whatever else it binds.
                (7, 10, "=MATCH(1,LET(unused,SEQUENCE(1),1),0)"),
            ],
        );
        let kinds: Vec<_> = results.into_iter().map(error_kind).collect();
        assert_eq!(
            kinds,
            [
                ExcelErrorKind::Na,
                ExcelErrorKind::Value,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
                ExcelErrorKind::Na,
            ],
            "legacy={legacy_file}"
        );
        // The selected argument is the array.
        let results = evaluate(
            legacy_file,
            &[
                (8, 4, "=MATCH(1,IF(TRUE,SEQUENCE(1),1),0)"),
                (8, 5, "=MATCH(1,CHOOSE(1,{1},1),0)"),
                (8, 6, "=MATCH(1,IF(A1=1,{1},1),0)"),
                (8, 7, "=MATCH(1,IFERROR({1},1),0)"),
                (8, 8, "=MATCH(1,IF({TRUE},1,0),0)"),
            ],
        );
        assert!(
            results.iter().all(|value| *value == number(1.0)),
            "legacy={legacy_file}: {results:?}"
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
        legacy(2, "=LET(area,A1:A3,IF(area,\"yes\",\"no\"))"),
        legacy(2, "=IF(A1:A3,\"yes\",\"no\")")
    );
    assert_eq!(
        legacy(2, "=LET(area,A1:A3,IF(area,\"yes\",\"no\"))"),
        text("no")
    );
    assert_eq!(
        legacy(3, "=LET(area,A1:A3,IF(area,\"yes\",\"no\"))"),
        text("yes")
    );
    assert_eq!(
        legacy(2, "=LET(area,A1:A3,IF(area=0,\"yes\",\"no\"))"),
        text("yes")
    );
    assert_eq!(
        error_kind(legacy(5, "=LET(area,A1:A3,IF(area,\"yes\",\"no\"))")),
        ExcelErrorKind::Value
    );
    // A reference parameter still takes the whole range.
    assert_eq!(legacy(5, "=LET(area,B1:B3,SUM(area))"), number(60.0));
    assert_eq!(legacy(5, "=LET(area,A1:A3,COUNTIFS(area,1))"), number(2.0));
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

#[test]
fn match_reads_the_shape_of_what_returns_its_lookup_array() {
    for legacy_file in [true, false] {
        // Single values: the value IFERROR or IFNA returns, the branch IF
        // selected with a test on a LET name, a LAMBDA parameter given a
        // value, a value computed from INDEX's reference, XMATCH's position
        // for one lookup value, N and T of a cell.
        let results = evaluate(
            legacy_file,
            &[
                (9, 4, "=MATCH(1,IFERROR(1,{2}),0)"),
                (9, 5, "=MATCH(1,IFNA(1,{2}),0)"),
                (9, 6, "=MATCH(1,LET(x,1,IF(x,1,{2})),0)"),
                (9, 7, "=LAMBDA(x,MATCH(1,x,0))(1)"),
                (9, 8, "=MATCH(1,INDEX(A1:A3,1)+0,0)"),
                (9, 9, "=MATCH(1,XMATCH(7,{7},0),0)"),
                (9, 10, "=MATCH(1,N(A1),0)"),
                (9, 11, "=MATCH(1,CHOOSE(A1,1,{1}),0)"),
                // Calls of equal arguments but another test are told apart.
                (
                    9,
                    12,
                    "=LET(x,{1}+0,y,1,MATCH(1,IF(A2,x,y)+0*SUM(IF(A1,x,y)),0))",
                ),
                // A LAMBDA's result as its body returned it.
                (9, 13, "=MATCH(1,LAMBDA(z,IF(A2,{1}+0,1))(0),0)"),
                (9, 14, "=MATCH(1,LET(f,LAMBDA(z,IF(A2,{1}+0,1)),f(0)),0)"),
                // What one temporary LAMBDA's body recorded is not read for
                // another's.
                (
                    9,
                    18,
                    "=MATCH(2,SUM(LAMBDA(_p,IF(FALSE,1,{1}+0))(0))+LAMBDA(_p,IF(1=1,1,{1}+0))(0),0)",
                ),
                // An empty slot IF selects, read as text.
                (9, 19, "=MATCH(0,--(IF(1=1,,{1}+0)&\"0\"),0)"),
                // The same call written under two LET bindings of f calls
                // two LAMBDAs: each keeps its own result.
                (
                    9,
                    20,
                    "=LET(f,LAMBDA(z,IF(A1,1,{1}+0)),MATCH(1,f(A1)+0*SUM(LET(f,LAMBDA(z,{1}+0),f(A1))),0))",
                ),
                (
                    9,
                    21,
                    "=MATCH(1,LET(f,LAMBDA(z,IF(A1,1,{1}+0)),f(A1))+0*SUM(LET(f,LAMBDA(z,{1}+0),f(A1))),0)",
                ),
                (
                    9,
                    15,
                    "=LET(x,{1}+0,y,1,MATCH(1,IF(A2:A2,x,y)+0*SUM(IF(A1:A1,x,y)),0))",
                ),
                (
                    9,
                    16,
                    "=LET(w,0,x,{1}+0,y,1,f,LAMBDA(a,b,d,z,z),MATCH(1,f(w,w,w,y)+0*SUM(f(w,w,w,x)),0))",
                ),
                (
                    9,
                    17,
                    "=LET(x,{1}+0,f,LAMBDA(z,1),MATCH(1,f(x)+0*SUM(LET(f,LAMBDA(z,z),f(x))),0))",
                ),
            ],
        );
        let kinds: Vec<_> = results.into_iter().map(error_kind).collect();
        assert_eq!(kinds, [ExcelErrorKind::Na; 18], "legacy={legacy_file}");
        // One-element arrays: the replacement IFERROR returns, the branch IF
        // selected, a LAMBDA parameter given one, XMATCH over an array of
        // lookup values, N and T over an array.
        let results = evaluate(
            legacy_file,
            &[
                (10, 4, "=MATCH(2,IFERROR(1/0,{2}+0),0)"),
                (10, 5, "=MATCH(1,LET(x,0,IF(x,1,{1}+0)),0)"),
                (10, 6, "=LAMBDA(x,MATCH(1,x,0))(SEQUENCE(1))"),
                (10, 7, "=MATCH(1,XMATCH({7}+0,{7},0),0)"),
                (10, 8, "=MATCH(1,N({1}),0)"),
                (10, 9, "=MATCH(\"a\",T({\"a\"}),0)"),
                (10, 10, "=MATCH(1,CHOOSE(A1,{1}+0,1),0)"),
                // IFS and SWITCH over a one-element array are element-wise.
                (10, 17, "=MATCH(1,IFS({TRUE}+0,1),0)"),
                (10, 18, "=MATCH(1,SWITCH({1}+0,1,1),0)"),
                (
                    10,
                    11,
                    "=LET(x,{1}+0,y,1,MATCH(1,IF(A1,x,y)+0*SUM(IF(A2,x,y)),0))",
                ),
                (10, 12, "=MATCH(1,LAMBDA(z,IF(A1,{1}+0,1))(0),0)"),
                (10, 13, "=MATCH(1,LET(f,LAMBDA(z,IF(A1,{1}+0,1)),f(0)),0)"),
                (
                    10,
                    14,
                    "=LET(x,{1}+0,y,1,MATCH(1,IF(A1:A1,x,y)+0*SUM(IF(A2:A2,x,y)),0))",
                ),
                (
                    10,
                    15,
                    "=LET(w,0,x,{1}+0,y,1,f,LAMBDA(a,b,d,z,z),MATCH(1,f(w,w,w,x)+0*SUM(f(w,w,w,y)),0))",
                ),
                (
                    10,
                    16,
                    "=LET(x,{1}+0,f,LAMBDA(z,z),MATCH(1,f(x)+0*SUM(LET(f,LAMBDA(z,1),f(x))),0))",
                ),
            ],
        );
        assert!(
            results.iter().all(|value| *value == number(1.0)),
            "legacy={legacy_file}: {results:?}"
        );
    }
}

#[test]
fn reading_the_shape_of_lookup_array_calculates_nothing_again() {
    use crate::args::ArgSchema;
    use crate::function::{FnCaps, Function};
    use crate::traits::{ArgumentHandle, FunctionContext};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// TRUE, counting its calculations.
    #[derive(Debug)]
    struct Tick(Arc<AtomicUsize>);
    impl Function for Tick {
        fn caps(&self) -> FnCaps {
            FnCaps::VOLATILE
        }
        fn name(&self) -> &'static str {
            "TICK"
        }
        fn arg_schema(&self) -> &'static [ArgSchema] {
            &[]
        }
        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<crate::traits::CalcValue<'b>, formualizer_common::ExcelError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Boolean(
                true,
            )))
        }
    }

    // A LAMBDA argument's shape is read without calculating a defined name
    // there again.
    {
        use crate::engine::named_range::{NameScope, NamedDefinition};
        let mut counts = Vec::new();
        for formula in ["=LAMBDA(x,1)(Thing)", "=LAMBDA(x,MATCH(1,x,0))(Thing)"] {
            let count = Arc::new(AtomicUsize::new(0));
            let workbook = TestWorkbook::new().with_function(Arc::new(Tick(count.clone())));
            let mut engine = Engine::new(
                workbook,
                EvalConfig {
                    enable_parallel: false,
                    ..Default::default()
                },
            );
            engine.add_sheet("Sheet1").ok();
            engine
                .define_name(
                    "Thing",
                    NamedDefinition::Formula {
                        ast: parse("=IF(TICK(),{1}+0,1)").unwrap(),
                        dependencies: Vec::new(),
                        range_deps: Vec::new(),
                    },
                    NameScope::Workbook,
                )
                .unwrap();
            engine
                .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
                .unwrap();
            engine.evaluate_all().unwrap();
            counts.push(count.load(Ordering::Relaxed));
        }
        assert_eq!(counts[0], counts[1], "{counts:?}");
    }

    // MATCH tries its lookup_array as a reference before reading its value,
    // so an IF there runs twice whatever its branches are; reading the
    // shape adds no third run.
    for legacy_file in [true, false] {
        for (formula, calls, expected) in [
            ("=LET(x,IF(TICK(),1,{1}),x)", 1, number(1.0)),
            ("=LET(x,IF(TRUE,1,IF(TICK(),1,{1})),x)", 0, number(1.0)),
            ("=MATCH(1,IF(TICK(),1,2),0)", 2, None),
            ("=MATCH(1,IF(TICK(),1,{1}),0)", 2, None),
            ("=MATCH(1,IF(TRUE,1,IF(TICK(),1,{1})),0)", 0, None),
            ("=MATCH(1,IF(TICK(),{1}+0,1),0)", 2, number(1.0)),
            ("=LAMBDA(x,MATCH(1,x,0))(IF(TICK(),1,{1}))", 1, None),
        ] {
            let count = Arc::new(AtomicUsize::new(0));
            let workbook = TestWorkbook::new().with_function(Arc::new(Tick(count.clone())));
            let mut engine = Engine::new(
                workbook,
                EvalConfig {
                    enable_parallel: false,
                    ..Default::default()
                },
            );
            engine
                .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
                .unwrap();
            if legacy_file {
                engine.use_legacy_array_semantics();
            }
            engine.evaluate_all().unwrap();
            assert_eq!(
                count.load(Ordering::Relaxed),
                calls,
                "{formula} legacy={legacy_file}"
            );
            let value = engine.get_cell_value("Sheet1", 1, 1);
            match expected {
                Some(_) => assert_eq!(value, expected, "{formula} legacy={legacy_file}"),
                None => assert_eq!(
                    error_kind(value),
                    ExcelErrorKind::Na,
                    "{formula} legacy={legacy_file}"
                ),
            }
        }
    }
}

#[test]
fn reading_the_shape_of_a_lambda_call_types_only_what_ran() {
    // Twenty LAMBDAs, each calling the one before twice in a branch never
    // taken: their results are read without typing those branches.
    let mut bindings = vec!["_f0,LAMBDA(z,1)".to_string()];
    for i in 1..=20 {
        bindings.push(format!(
            "_f{i},LAMBDA(z,IF(TRUE,1,_f{p}(z)+_f{p}(z)))",
            p = i - 1
        ));
    }
    let formula = format!("=LET({},LAMBDA(z,z)(_f20(0)))", bindings.join(","));
    for legacy_file in [true, false] {
        assert_eq!(
            evaluate(legacy_file, &[(5, 4, formula.as_str())])[0],
            number(1.0),
            "legacy={legacy_file}"
        );
        let formula = format!("=LET({},MATCH(1,_f20(0),0))", bindings.join(","));
        assert_eq!(
            error_kind(evaluate(legacy_file, &[(5, 4, formula.as_str())]).remove(0)),
            ExcelErrorKind::Na,
            "legacy={legacy_file}"
        );
    }
    // Each LAMBDA calling the one before twice, both calls taken: each body
    // is read once per distinct call, and the result is a single value.
    let mut bindings = vec!["_f0,LAMBDA(z,1)".to_string()];
    for i in 1..=12 {
        bindings.push(format!("_f{i},LAMBDA(z,_f{p}(z)+_f{p}(z))", p = i - 1));
    }
    let formula = format!("=LET({},MATCH(4096,_f12(0),0))", bindings.join(","));
    for legacy_file in [true, false] {
        assert_eq!(
            error_kind(evaluate(legacy_file, &[(5, 4, formula.as_str())]).remove(0)),
            ExcelErrorKind::Na,
            "legacy={legacy_file}"
        );
    }
    // Twelve-parameter LAMBDAs called with every mix of shapes, in an IFS
    // value never returned: not typed.
    let names = ["a", "b", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m"];
    let params = names.join(",");
    let mut bindings = vec![format!("_f0,LAMBDA({params},1)")];
    for i in 1..=12 {
        let (mut left, mut right) = (names.to_vec(), names.to_vec());
        left[i - 1] = "1";
        right[i - 1] = "{1}";
        bindings.push(format!(
            "_f{i},LAMBDA({params},_f{p}({})+_f{p}({}))",
            left.join(","),
            right.join(","),
            p = i - 1
        ));
    }
    let formula = format!(
        "=MATCH(1,LET({},IFS(TRUE,1,FALSE,_f12({}))),0)",
        bindings.join(","),
        ["1"; 12].join(",")
    );
    for legacy_file in [true, false] {
        assert_eq!(
            error_kind(evaluate(legacy_file, &[(5, 4, formula.as_str())]).remove(0)),
            ExcelErrorKind::Na,
            "legacy={legacy_file}"
        );
    }
    // Two LAMBDAs of one body, `_x`, one returning its parameter (an
    // array), the other a name it sees (a single value).
    let formula = "=MATCH(2,LET(_arr,{1},LET(_x,1,_f,LAMBDA(_y,_x),_f(_arr))+LET(_x,1,_f,LAMBDA(_x,_x),_f(_arr))),0)";
    for legacy_file in [true, false] {
        assert_eq!(
            evaluate(legacy_file, &[(5, 4, formula)])[0],
            number(1.0),
            "legacy={legacy_file}"
        );
    }
}

#[test]
fn a_lambda_result_computed_from_a_defined_reference_is_a_single_value() {
    use crate::engine::named_range::{NameScope, NamedDefinition};
    use crate::reference::{CellRef, Coord};
    for legacy_file in [true, false] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig {
                enable_parallel: false,
                ..Default::default()
            },
        );
        engine.add_sheet("Sheet1").ok();
        let sheet = engine.sheet_id("Sheet1").unwrap();
        engine
            .set_cell_value("Sheet1", 1, 1, LiteralValue::Number(1.0))
            .unwrap();
        engine
            .define_name(
                "Thing",
                NamedDefinition::Cell(CellRef::new(sheet, Coord::from_excel(1, 1, true, true))),
                NameScope::Workbook,
            )
            .unwrap();
        engine
            .set_cell_formula(
                "Sheet1",
                2,
                2,
                parse("=MATCH(1,LAMBDA(z,Thing+0)(0),0)").unwrap(),
            )
            .unwrap();
        if legacy_file {
            engine.use_legacy_array_semantics();
        }
        engine.evaluate_all().unwrap();
        assert_eq!(
            error_kind(engine.get_cell_value("Sheet1", 2, 2)),
            ExcelErrorKind::Na,
            "legacy={legacy_file}"
        );
    }
}

#[test]
fn match_reads_the_selection_its_value_came_from() {
    use crate::args::ArgSchema;
    use crate::function::{FnCaps, Function};
    use crate::traits::{ArgumentHandle, FunctionContext};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// TRUE, FALSE, TRUE, ... on successive calculations.
    #[derive(Debug)]
    struct Toggle(Arc<AtomicUsize>);
    impl Function for Toggle {
        fn caps(&self) -> FnCaps {
            FnCaps::VOLATILE
        }
        fn name(&self) -> &'static str {
            "TOGGLE"
        }
        fn arg_schema(&self) -> &'static [ArgSchema] {
            &[]
        }
        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<crate::traits::CalcValue<'b>, formualizer_common::ExcelError> {
            let calls = self.0.fetch_add(1, Ordering::Relaxed);
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Boolean(
                calls.is_multiple_of(2),
            )))
        }
    }

    // MATCH tries lookup_array as a reference first (IF and CHOOSE resolve
    // their selected argument as one too): what that runs is discarded and
    // records nothing.
    for formula in [
        "=MATCH(1,IF(TOGGLE(),1,{2}+0),0)",
        "=MATCH(1,IF(TRUE,IF(TOGGLE(),1,{2}+0),0),0)",
        "=MATCH(1,CHOOSE(1,IF(TOGGLE(),1,{2}+0)),0)",
    ] {
        for legacy_file in [true, false] {
            let count = Arc::new(AtomicUsize::new(0));
            let workbook = TestWorkbook::new().with_function(Arc::new(Toggle(count)));
            let mut engine = Engine::new(
                workbook,
                EvalConfig {
                    enable_parallel: false,
                    ..Default::default()
                },
            );
            engine
                .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
                .unwrap();
            if legacy_file {
                engine.use_legacy_array_semantics();
            }
            engine.evaluate_all().unwrap();
            assert_eq!(
                error_kind(engine.get_cell_value("Sheet1", 1, 1)),
                ExcelErrorKind::Na,
                "{formula} legacy={legacy_file}"
            );
        }
    }
}
