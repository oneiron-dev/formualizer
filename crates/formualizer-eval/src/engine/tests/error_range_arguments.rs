//! An error value where a function expects a range is the function's result;
//! an error value as a criterion counts the cells holding that error.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn eval(formula: &str) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Number(1.0))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 1, parse("=NA()").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 3, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 3).unwrap()
}

#[test]
fn error_in_place_of_a_range_is_the_result() {
    for formula in [
        "=COUNTIF(#REF!,1)",
        "=SUMIF(#REF!,1)",
        "=SUMIF(A1:A2,\">0\",#REF!)",
        "=COUNTIFS(#REF!,1)",
        "=MATCH(1,#REF!,0)",
        "=INDEX(#REF!,MATCH(1,#REF!,0))",
        "=ROWS(#REF!)",
        "=COLUMNS(#REF!)",
    ] {
        match eval(formula) {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Ref, "{formula}"),
            other => panic!("{formula}: expected #REF!, got {other:?}"),
        }
    }
}

#[test]
fn error_criterion_counts_matching_errors() {
    assert_eq!(eval("=COUNTIF(A1:A2,#N/A)"), LiteralValue::Number(1.0));
    assert_eq!(eval("=COUNTIF(A1:A2,A2)"), LiteralValue::Number(1.0));
    assert_eq!(eval("=COUNTIF(A1:A2,#DIV/0!)"), LiteralValue::Number(0.0));
}

#[test]
fn error_in_a_summed_matching_cell_is_the_result() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, key, value) in [(1, "a", "=1"), (2, "b", "=NA()"), (3, "a", "=2")] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Text(key.into()))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", row, 2, parse(value).unwrap())
            .unwrap();
    }
    let formulas = [
        "=SUMIF(A1:A3,\"a\",B1:B3)",
        "=SUMIF(A1:A3,\"b\",B1:B3)",
        "=SUMIFS(B1:B3,A1:A3,\"<>c\")",
        "=AVERAGEIF(A1:A3,\"b\",B1:B3)",
        "=SUMIF(B1:B3,\"<>#N/A\")",
    ];
    for (i, formula) in formulas.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", 1 + i as u32, 4, parse(formula).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    let value = |row| engine.get_cell_value("Sheet1", row, 4).unwrap();
    assert_eq!(value(1), LiteralValue::Number(3.0));
    for row in 2..=4 {
        match value(row) {
            LiteralValue::Error(e) => {
                assert_eq!(e.kind, ExcelErrorKind::Na, "{}", formulas[row as usize - 1])
            }
            other => panic!(
                "{}: expected #N/A, got {other:?}",
                formulas[row as usize - 1]
            ),
        }
    }
    assert_eq!(value(5), LiteralValue::Number(3.0));
}

#[test]
fn error_text_and_empty_criteria() {
    assert_eq!(eval("=COUNTIF(A1:A2,\"#N/A\")"), LiteralValue::Number(1.0));
    assert_eq!(
        eval("=COUNTIF(A1:A2,\"<>#N/A\")"),
        LiteralValue::Number(1.0)
    );
    // An empty criteria cell is the criterion 0, which blanks do not meet.
    assert_eq!(eval("=COUNTIF(A1:A5,B9)"), LiteralValue::Number(0.0));
    assert_eq!(eval("=COUNTIF(A1:A5,\"\")"), LiteralValue::Number(3.0));
}
