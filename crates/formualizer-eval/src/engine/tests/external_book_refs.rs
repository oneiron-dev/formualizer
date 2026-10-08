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

/// A book whose `Failed` sheet Excel could not read at its last refresh
/// (`refreshError`) and whose `Unsaved` sheet has no saved values: every cell
/// they did not save is #REF!. Rows and columns as in probes 1 and 2 of
/// ops/excel-extlinks-probe-20261006.md (Excel for Windows 16.0.20430).
fn engine_with_failed_sheet() -> Engine<TestWorkbook> {
    let mut engine = engine_with_book();
    let mut book = ExternalBook::new();
    let failed = book.add_sheet("Failed");
    failed.set_refresh_error(true);
    failed.set(1, 1, LiteralValue::Number(1.0));
    failed.set(2, 1, LiteralValue::Number(2.0));
    failed.set(4, 1, LiteralValue::Text("x".into()));
    failed.set(5, 1, LiteralValue::Number(5.0));
    failed.set(1, 2, LiteralValue::Empty);
    failed.set(2, 2, LiteralValue::Number(20.0));
    failed.set(1, 3, LiteralValue::Boolean(true));
    let full = book.add_sheet("Full");
    full.set_refresh_error(true);
    for row in 1..=5 {
        full.set(row, 1, LiteralValue::Number(f64::from(row)));
        full.set(
            row,
            2,
            LiteralValue::Text(["a", "b", "c", "d", "e"][row as usize - 1].into()),
        );
    }
    let across = book.add_sheet("Across");
    across.set_refresh_error(true);
    for col in 1..=3 {
        across.set(1, col, LiteralValue::Number(f64::from(col)));
        across.set(
            2,
            col,
            LiteralValue::Text(["p", "q", "r"][col as usize - 1].into()),
        );
    }
    book.add_sheet("Unsaved").set_refresh_error(true);
    engine.set_external_book("[2]", book);
    engine
}

fn assert_ref_error(value: Option<LiteralValue>, formula: &str) {
    match value {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Ref, "{formula}"),
        other => panic!("{formula}: expected #REF!, got {other:?}"),
    }
}

#[test]
fn a_sheet_with_a_refresh_error_reads_unsaved_cells_as_ref_errors() {
    let mut engine = engine_with_failed_sheet();
    for formula in [
        "=[2]Failed!A3",
        "=[2]Failed!A3+1",
        "=SUM([2]Failed!A3)",
        "=SUM([2]Failed!A1:A3)",
        "=[2]Failed!A3:A3",
        "=INDEX([2]Failed!A1:A5,3)",
        "=N([2]Failed!A3)",
        "=VLOOKUP(\"x\",[2]Failed!A1:B5,2,FALSE)",
        "=SUM([2]Failed!B1:B5)",
        "=[2]Failed!A3=\"\"",
        "=[2]Unsaved!A1",
        "=SUM([2]Unsaved!A1:A5)",
    ] {
        assert_ref_error(eval(&mut engine, formula), formula);
    }
    for (formula, expected) in [
        ("=[2]Failed!A1", LiteralValue::Number(1.0)),
        ("=[2]Failed!B1&\"\"", LiteralValue::Text(String::new())),
        ("=[2]Failed!C1", LiteralValue::Boolean(true)),
        ("=ISBLANK([2]Failed!A3)", LiteralValue::Boolean(false)),
        ("=ISERROR([2]Failed!A3)", LiteralValue::Boolean(true)),
        ("=ISREF([2]Failed!A3)", LiteralValue::Boolean(true)),
        (
            "=IFERROR([2]Failed!A3,\"e\")",
            LiteralValue::Text("e".into()),
        ),
        ("=TYPE([2]Failed!A3)", LiteralValue::Number(16.0)),
        ("=COUNTA([2]Failed!A1:A10)", LiteralValue::Number(10.0)),
        ("=COUNT([2]Failed!A1:A10)", LiteralValue::Number(3.0)),
        ("=COUNTA([2]Failed!A3)", LiteralValue::Number(1.0)),
        (
            "=MATCH(\"x\",[2]Failed!A1:A10,0)",
            LiteralValue::Number(4.0),
        ),
        ("=LOOKUP(2,[2]Failed!A1:A2)", LiteralValue::Number(2.0)),
        (
            "=HLOOKUP(1,[2]Failed!A1:C2,2,FALSE)",
            LiteralValue::Number(2.0),
        ),
        ("=ROW([2]Failed!A3)", LiteralValue::Number(3.0)),
        ("=ROWS([2]Failed!A3)", LiteralValue::Number(1.0)),
    ] {
        assert_eq!(
            number_like(eval(&mut engine, formula)),
            Some(expected),
            "{formula}"
        );
    }
}

#[test]
fn an_open_reference_into_a_sheet_with_a_refresh_error_reaches_a_ref_error() {
    // Past the saved cells every cell of A:A (or 1:1) is #REF!: anything
    // that reads them all is #REF!, lookups skip them.
    let mut engine = engine_with_failed_sheet();
    for formula in [
        "=SUM([2]Full!A:A)",
        "=INDEX([2]Full!A:A,6)",
        "=INDEX([2]Full!A:A,1000)",
        "=SUM([2]Across!1:1)",
    ] {
        assert_ref_error(eval(&mut engine, formula), formula);
    }
    for (formula, expected) in [
        ("=COUNT([2]Full!A:A)", LiteralValue::Number(5.0)),
        ("=MATCH(4,[2]Full!A:A,0)", LiteralValue::Number(4.0)),
        ("=MATCH(4,[2]Full!A:A,1)", LiteralValue::Number(4.0)),
        ("=MATCH(99,[2]Full!A:A,1)", LiteralValue::Number(5.0)),
        ("=MATCH(4.5,[2]Full!A:A)", LiteralValue::Number(4.0)),
        (
            "=VLOOKUP(3,[2]Full!A:B,2,FALSE)",
            LiteralValue::Text("c".into()),
        ),
        (
            "=VLOOKUP(3.5,[2]Full!A:B,2,TRUE)",
            LiteralValue::Text("c".into()),
        ),
        ("=VLOOKUP(99,[2]Full!A:B,2)", LiteralValue::Text("e".into())),
        (
            "=LOOKUP(3.5,[2]Full!A:A,[2]Full!B:B)",
            LiteralValue::Text("c".into()),
        ),
        ("=LOOKUP(99,[2]Full!A:A)", LiteralValue::Number(5.0)),
        (
            "=XLOOKUP(3,[2]Full!A:A,[2]Full!B:B)",
            LiteralValue::Text("c".into()),
        ),
        (
            "=XLOOKUP(9,[2]Full!A:A,[2]Full!B:B,\"none\")",
            LiteralValue::Text("none".into()),
        ),
        (
            "=XLOOKUP(3.5,[2]Full!A:A,[2]Full!B:B,,-1)",
            LiteralValue::Text("c".into()),
        ),
        ("=XMATCH(3,[2]Full!A:A,0,2)", LiteralValue::Number(3.0)),
        ("=INDEX([2]Full!A:A,5)", LiteralValue::Number(5.0)),
        ("=MATCH(3,[2]Across!1:1,0)", LiteralValue::Number(3.0)),
        (
            "=HLOOKUP(2,[2]Across!1:2,2,FALSE)",
            LiteralValue::Text("q".into()),
        ),
        ("=ROWS([2]Full!A:A)", LiteralValue::Number(1_048_576.0)),
        ("=COLUMNS([2]Across!1:1)", LiteralValue::Number(16_384.0)),
    ] {
        assert_eq!(
            number_like(eval(&mut engine, formula)),
            Some(expected),
            "{formula}"
        );
    }
}

#[test]
fn row_and_column_give_the_position_written() {
    // ROW([1]DATI!$K$2:$K$999) is {2;...;999}, as for a local range, though the
    // reference reads the values saved with the link (SpreadsheetBench 55965).
    let mut engine = engine_with_book();
    for (formula, expected) in [
        ("=ROW([1]Rates!B2)", 2.0),
        ("=INDEX(ROW([1]Rates!A2:A4),2)", 3.0),
        ("=COLUMN([1]Rates!C3)", 3.0),
        ("=INDEX(COLUMN([1]Rates!B2:D2),1,3)", 4.0),
        ("=ROWS([1]Rates!A:A)", 1_048_576.0),
        ("=ROWS([1]Rates!A1:A10)", 10.0),
        ("=ROW(INDEX([1]Rates!A1:A5,3))", 3.0),
        ("=COLUMN(INDEX([1]Rates!A1:B5,1,2))", 2.0),
        (
            "=LARGE(INDEX(ROW([1]Rates!$B$1:$B$9)*([1]Rates!$B$1:$B$9>4),),1)",
            4.0,
        ),
        (
            "=LARGE(INDEX(ROW([1]Rates!$B$1:$B$9)*([1]Rates!$B$1:$B$9>4),),2)",
            2.0,
        ),
    ] {
        assert_eq!(
            number_like(eval(&mut engine, formula)),
            Some(LiteralValue::Number(expected)),
            "{formula}"
        );
    }
    assert_eq!(
        eval(
            &mut engine,
            "=INDEX([1]Rates!$A$1:$A$9,LARGE(INDEX(ROW([1]Rates!$B$1:$B$9)*([1]Rates!$B$1:$B$9>4),),2))"
        ),
        Some(LiteralValue::Text("pear".into()))
    );
}

#[test]
fn index_selects_a_reference_into_the_linked_workbook() {
    let mut engine = engine_with_book();
    for (formula, expected) in [
        (
            "=ISREF(INDEX([1]Rates!A1:A5,3))",
            LiteralValue::Boolean(true),
        ),
        (
            "=ISBLANK(INDEX([1]Rates!A:A,100))",
            LiteralValue::Boolean(true),
        ),
        (
            "=ISBLANK(INDEX([1]Rates!A1:A9,8))",
            LiteralValue::Boolean(true),
        ),
        ("=INDEX([1]Rates!A:B,2,2)", LiteralValue::Number(5.0)),
    ] {
        assert_eq!(
            number_like(eval(&mut engine, formula)),
            Some(expected),
            "{formula}"
        );
    }
}

#[test]
fn legacy_formulas_intersect_linked_ranges_with_the_formula_cell() {
    // Without the array flag a range operand of a single-value position is
    // the cell in the formula's row (column): #VALUE! outside it.
    let mut engine = engine_with_book();
    for (row, formula) in [
        (2, "=SUM([1]Rates!B1:B4*2)"),
        (9, "=SUM([1]Rates!B1:B4*2)"),
        (4, "=IF([1]Rates!B:B>6,\"big\",\"small\")"),
        (2, "=IF([1]Rates!B:B>6,\"big\",\"small\")"),
    ] {
        engine
            .set_cell_formula(
                "Sheet1",
                row,
                3 + u32::from(row == 2 && formula.starts_with("=IF")),
                parse(formula).unwrap(),
            )
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    engine.evaluate_all().unwrap();
    assert_eq!(
        number_like(engine.get_cell_value("Sheet1", 2, 3)),
        Some(LiteralValue::Number(10.0))
    );
    match engine.get_cell_value("Sheet1", 9, 3) {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Value),
        other => panic!("expected #VALUE!, got {other:?}"),
    }
    assert_eq!(
        engine.get_cell_value("Sheet1", 4, 3),
        Some(LiteralValue::Text("big".into()))
    );
    assert_eq!(
        engine.get_cell_value("Sheet1", 2, 4),
        Some(LiteralValue::Text("small".into()))
    );
}

/// A range of a closed linked workbook that a function returns (INDEX at row
/// 0, here computed, or through IF and IFERROR) intersects a legacy formula's
/// cell by the rows and columns it spans in the linked sheet, as a written
/// one does, where a cell the link did not save is blank or #REF!: Excel for
/// Windows 16.0.20430, job probe-w2-links2-1
/// (ops/excel-links2-probe-20261008.md), where the fork took the top-left
/// value.
#[test]
fn a_linked_range_a_function_returns_intersects_a_legacy_formula() {
    let mut engine = engine_with_failed_sheet();
    let cases = [
        (1, 3, "=IFERROR(INDEX([1]Rates!B1:B4,SMALL({0,1},1)),\"\")"),
        (2, 3, "=INDEX([1]Rates!B1:B4,SMALL({0,1},1))*2"),
        (4, 3, "=IF(TRUE,INDEX([1]Rates!B1:B4,SMALL({0,1},1)))"),
        (9, 3, "=INDEX([1]Rates!B1:B4,SMALL({0,1},1))*2"),
        (5, 2, "=INDEX([1]Rates!A2:C2,SMALL({0,1},1))+1"),
        (4, 4, "=INDEX([2]Failed!A:A,FALSE)"),
        (2, 4, "=INDEX([2]Failed!A:A,0)+1"),
        (9, 4, "=INDEX([2]Failed!A1:B9,0,SMALL({1,2},1))"),
    ];
    for (row, col, formula) in cases {
        engine
            .set_cell_formula("Sheet1", row, col, parse(formula).unwrap())
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    engine.evaluate_all().unwrap();
    let value = |row, col| number_like(engine.get_cell_value("Sheet1", row, col));
    assert_eq!(value(1, 3), Some(LiteralValue::Number(3.0)));
    assert_eq!(value(2, 3), Some(LiteralValue::Number(10.0)));
    assert_eq!(value(4, 3), Some(LiteralValue::Number(7.0)));
    match value(9, 3) {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Value),
        other => panic!("row 9, outside B1:B4: expected #VALUE!, got {other:?}"),
    }
    // A row intersects by the formula's column: B2 is 5.
    assert_eq!(value(5, 2), Some(LiteralValue::Number(6.0)));
    // Failed!A4 is the text saved there; A2 is 2; A9 was not saved.
    assert_eq!(value(4, 4), Some(LiteralValue::Text("x".into())));
    assert_eq!(value(2, 4), Some(LiteralValue::Number(3.0)));
    assert_ref_error(value(9, 4), "=INDEX([2]Failed!A1:B9,0,1) in row 9");
}

/// Whole numbers as numbers, whatever integer type the engine keeps.
fn number_like(value: Option<LiteralValue>) -> Option<LiteralValue> {
    match value {
        Some(LiteralValue::Int(n)) => Some(LiteralValue::Number(n as f64)),
        other => other,
    }
}

#[test]
fn iferror_and_ifna_catch_the_error_a_linked_cell_saves() {
    let mut engine = engine_with_failed_sheet();
    let mut book = ExternalBook::new();
    let saved = book.add_sheet("Saved");
    saved.set(
        1,
        1,
        LiteralValue::Error(formualizer_common::ExcelError::new(ExcelErrorKind::Na)),
    );
    engine.set_external_book("[3]", book);
    for (formula, expected) in [
        (
            "=IFERROR([2]Failed!A3,\"e\")",
            LiteralValue::Text("e".into()),
        ),
        ("=IFERROR([2]Failed!A1,\"e\")", LiteralValue::Number(1.0)),
        ("=IFNA([3]Saved!A1,\"na\")", LiteralValue::Text("na".into())),
        (
            "=IFERROR([3]Saved!A1:A1,\"e\")",
            LiteralValue::Text("e".into()),
        ),
    ] {
        assert_eq!(
            number_like(eval(&mut engine, formula)),
            Some(expected),
            "{formula}"
        );
    }
    match eval(&mut engine, "=IFNA([2]Failed!A3,\"na\")") {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Ref),
        other => panic!("expected #REF!, got {other:?}"),
    }
}

#[test]
fn a_closed_linked_workbook_has_no_formula_or_sheet_to_inspect() {
    let mut engine = engine_with_book();
    for formula in [
        "=ISFORMULA([1]Rates!A1)",
        "=FORMULATEXT([1]Rates!A1)",
        "=SHEET([1]Rates!A1)",
        "=SHEETS([1]Rates!A1)",
    ] {
        match eval(&mut engine, formula) {
            Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Na, "{formula}"),
            other => panic!("{formula}: expected #N/A, got {other:?}"),
        }
    }
    assert_eq!(
        number_like(eval(&mut engine, "=SUBTOTAL(9,[1]Rates!B1:B4)")),
        Some(LiteralValue::Number(15.0))
    );
    assert_eq!(
        number_like(eval(&mut engine, "=AGGREGATE(14,6,[1]Rates!B1:B4,2)")),
        Some(LiteralValue::Number(5.0))
    );
}

#[test]
fn a_reversed_linked_range_spans_the_same_cells() {
    // Excel stores `[1]Ok!A5:A1` as `[1]Ok!A1:A5` (probe 4 of
    // ops/excel-extlinks-probe-20261006.md): INDEX, ROW and ROWS read it in
    // order, and a large reversed range still reaches the unsaved #REF!.
    let mut engine = engine_with_failed_sheet();
    for (formula, expected) in [
        (
            "=INDEX([1]Rates!A4:A1,2)",
            LiteralValue::Text("pear".into()),
        ),
        ("=ROWS([1]Rates!B4:B1)", LiteralValue::Number(4.0)),
        ("=ROW([1]Rates!A4:A2)", LiteralValue::Number(2.0)),
        ("=SUM([1]Rates!B4:B1)", LiteralValue::Number(15.0)),
    ] {
        assert_eq!(
            number_like(eval(&mut engine, formula)),
            Some(expected),
            "{formula}"
        );
    }
    assert_ref_error(
        eval(&mut engine, "=SUM([2]Full!E1000000:A1)"),
        "=SUM([2]Full!E1000000:A1)",
    );
}
