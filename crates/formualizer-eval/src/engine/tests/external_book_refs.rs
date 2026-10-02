//! References into linked workbooks evaluate from their saved values.

use crate::engine::external_book::ExternalBook;
use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn engine_with_book() -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let mut book = ExternalBook::new();
    book.add_sheet("Empty");
    let rates = book.add_sheet("Rates");
    rates.set(1, 1, LiteralValue::Text("apple".into()));
    rates.set(1, 2, LiteralValue::Number(3.0));
    rates.set(2, 1, LiteralValue::Text("pear".into()));
    rates.set(2, 2, LiteralValue::Number(5.0));
    rates.set(4, 2, LiteralValue::Number(7.0));
    engine.set_external_book("[1]", book);
    engine
}

fn eval(engine: &mut Engine<TestWorkbook>, formula: &str) -> Option<LiteralValue> {
    engine
        .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 1)
}

#[test]
fn cell_reference_reads_saved_value() {
    let mut engine = engine_with_book();
    assert_eq!(
        eval(&mut engine, "=[1]Rates!$B$2*2"),
        Some(LiteralValue::Number(10.0))
    );
    // Sheet names match case-insensitively; quoted sheet segments work too.
    assert_eq!(
        eval(&mut engine, "='[1]rates'!A1"),
        Some(LiteralValue::Text("apple".into()))
    );
}

#[test]
fn unsaved_cell_is_blank() {
    let mut engine = engine_with_book();
    assert_eq!(
        eval(&mut engine, "=[1]Rates!C9+1"),
        Some(LiteralValue::Number(1.0))
    );
    assert_eq!(
        eval(&mut engine, "=ISBLANK([1]Rates!B3)"),
        Some(LiteralValue::Boolean(true))
    );
}

#[test]
fn ranges_feed_lookups_and_aggregates() {
    let mut engine = engine_with_book();
    assert_eq!(
        eval(&mut engine, "=VLOOKUP(\"pear\",[1]Rates!A1:B4,2,FALSE)"),
        Some(LiteralValue::Number(5.0))
    );
    assert_eq!(
        eval(&mut engine, "=SUM([1]Rates!B:B)"),
        Some(LiteralValue::Number(15.0))
    );
    assert_eq!(
        eval(&mut engine, "=ROWS([1]Rates!A1:B4)"),
        Some(LiteralValue::Number(4.0))
    );
}

/// LOOKUP's array form takes its shape from the range as written, though a
/// linked range too large to read whole stops at the last saved cell:
/// A1:ZZ20000 is taller than wide (search column A, return from ZZ) and
/// A1:XFD300 is wider than tall (search row 1, return from row 300). The cells
/// returned lie past the saved ones, so they are blank.
#[test]
fn lookup_array_form_reads_a_cropped_range_as_written() {
    let mut engine = engine_with_book();
    assert_eq!(
        eval(&mut engine, "=LOOKUP(\"pear\",[1]Rates!A1:ZZ20000)"),
        Some(LiteralValue::Number(0.0))
    );
    assert_eq!(
        eval(&mut engine, "=LOOKUP(5,[1]Rates!A1:XFD300)"),
        Some(LiteralValue::Number(0.0))
    );
    // Within the saved cells the array form is unchanged.
    assert_eq!(
        eval(&mut engine, "=LOOKUP(\"pear\",[1]Rates!A1:B4)"),
        Some(LiteralValue::Number(5.0))
    );
}

#[test]
fn sheet_missing_from_link_is_ref_error() {
    let mut engine = engine_with_book();
    match eval(&mut engine, "=[1]Other!A1") {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
    assert_eq!(
        eval(&mut engine, "=IFERROR(VLOOKUP(1,[1]Other!A1:B4,2,0),\"-\")"),
        Some(LiteralValue::Text("-".into()))
    );
}

#[test]
fn sheet_with_nothing_saved_reads_blank() {
    let mut engine = engine_with_book();
    assert_eq!(
        eval(&mut engine, "=COUNTA([1]Empty!A1:C3)"),
        Some(LiteralValue::Number(0.0))
    );
}

#[test]
fn index_match_read_saved_values() {
    let mut engine = engine_with_book();
    assert_eq!(
        eval(
            &mut engine,
            "=INDEX([1]Rates!$B$1:$B$4,MATCH(\"pear\",[1]Rates!$A$1:$A$4,0))"
        ),
        Some(LiteralValue::Number(5.0))
    );
}

#[test]
fn range_only_parameters_reject_closed_workbook() {
    let mut engine = engine_with_book();
    for formula in [
        "=SUMIF([1]Rates!A1:A4,\"pear\",[1]Rates!B1:B4)",
        "=COUNTIF([1]Rates!A1:A4,\"pear\")",
        "=SUMIFS([1]Rates!B1:B4,[1]Rates!A1:A4,\"pear\")",
        "=COUNTIFS([1]Rates!A1:A4,\"pear\")",
        "=AVERAGEIF([1]Rates!A1:A4,\"pear\",[1]Rates!B1:B4)",
        "=MAXIFS([1]Rates!B1:B4,[1]Rates!A1:A4,\"pear\")",
        "=COUNTBLANK([1]Rates!A1:A4)",
    ] {
        match eval(&mut engine, formula) {
            Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Value, "{formula}"),
            other => panic!("{formula}: expected #VALUE!, got {other:?}"),
        }
    }
    // A criterion read from the linked workbook is an ordinary value.
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Text("pear".into()))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 3, LiteralValue::Number(4.0))
        .unwrap();
    assert_eq!(
        eval(&mut engine, "=SUMIF(B2,[1]Rates!A2,C2)"),
        Some(LiteralValue::Number(4.0))
    );
}

#[test]
fn linked_table_is_ref_error() {
    let mut engine = engine_with_book();
    match eval(&mut engine, "=VLOOKUP(\"pear\",[1]!Prices[#Data],2,FALSE)") {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
    assert_eq!(
        eval(&mut engine, "=IFERROR(SUM([1]!Prices[Qty]),-1)"),
        Some(LiteralValue::Number(-1.0))
    );
}

#[test]
fn link_without_saved_values_is_ref_error() {
    let mut engine = engine_with_book();
    match eval(&mut engine, "=SUM([7]Sheet1!$B:$B)") {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
    assert_eq!(
        eval(&mut engine, "=IFERROR([7]Sheet1!A1,0)"),
        Some(LiteralValue::Number(0.0))
    );
}
