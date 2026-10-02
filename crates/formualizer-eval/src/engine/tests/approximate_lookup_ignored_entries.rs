//! Approximate lookup ignores entries it cannot compare against the needle.
//!
//! Oracle: Microsoft Excel 16.105.3 (Microsoft 365 for Mac), synthetic
//! workbook, values recalculated in-app (`oracle: excel-verified`).
//!
//! Excel's legacy approximate match (`MATCH` with `match_type` 1/-1,
//! `VLOOKUP`/`HLOOKUP` with `range_lookup` TRUE) searches only the entries
//! belonging to the needle's value class. Blank cells and entries of another
//! class -- a text header above a numeric column, a stray number inside a
//! text column -- are skipped. They neither make the vector look unsorted nor
//! occupy a matchable position, and the returned index is still the position
//! in the *original* range, not in the compacted one.
//!
//! Error cells are skipped on exactly the same terms (issue #326, oracle rows
//! reproduced in `issue_326_error_skip_oracle.rs`): they are projected out of
//! the search, cannot be returned, never break sortedness, and are not
//! propagated even when a bisection probe lands on one. A range with nothing
//! searchable left yields #N/A.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use chrono::NaiveDate;
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn eval(engine: &mut Engine<TestWorkbook>, formula: &str) -> Option<LiteralValue> {
    engine
        .set_cell_formula("Sheet1", 1, 20, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 20)
}

fn assert_number(value: Option<LiteralValue>, expected: f64, formula: &str) {
    match value {
        Some(LiteralValue::Int(i)) => assert_eq!(i as f64, expected, "{formula}"),
        Some(LiteralValue::Number(n)) => assert!((n - expected).abs() < 1e-9, "{formula} => {n}"),
        other => panic!("{formula}: expected {expected}, got {other:?}"),
    }
}

fn assert_error(value: Option<LiteralValue>, expected: ExcelErrorKind, formula: &str) {
    match value {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, expected, "{formula}"),
        other => panic!("{formula}: expected {expected:?}, got {other:?}"),
    }
}

fn assert_na(value: Option<LiteralValue>, formula: &str) {
    assert_error(value, ExcelErrorKind::Na, formula);
}

/// A: 1..5 in rows 1-5, rows 6-10 never written (blank).
/// C: "Header" in row 1, then 1..5 in rows 2-6.
/// E: 5..1 descending in rows 1-5, rows 6-10 blank.
/// G: 1, 2, blank, 4, 5 -- an interior blank.
/// I: "apple", 1, 2, "zebra" -- both classes interleaved.
/// K: "alpha", "beta", "gamma" in rows 1-3, rows 4-10 blank.
/// M: 3, 1, 5, 2, 4 -- genuinely unsorted control.
fn build_engine() -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    fn num(engine: &mut Engine<TestWorkbook>, row: u32, col: u32, v: i64) {
        engine
            .set_cell_value("Sheet1", row, col, LiteralValue::Int(v))
            .unwrap();
    }
    for i in 1..=5i64 {
        num(&mut engine, i as u32, 1, i);
        num(&mut engine, i as u32, 5, 6 - i);
    }
    // Column C: text header then ascending numbers.
    engine
        .set_cell_value("Sheet1", 1, 3, LiteralValue::Text("Header".into()))
        .unwrap();
    for i in 1..=5i64 {
        num(&mut engine, (i as u32) + 1, 3, i);
    }
    // Column G: interior blank at row 3.
    num(&mut engine, 1, 7, 1);
    num(&mut engine, 2, 7, 2);
    num(&mut engine, 4, 7, 4);
    num(&mut engine, 5, 7, 5);
    // Column I: mixed classes.
    engine
        .set_cell_value("Sheet1", 1, 9, LiteralValue::Text("apple".into()))
        .unwrap();
    num(&mut engine, 2, 9, 1);
    num(&mut engine, 3, 9, 2);
    engine
        .set_cell_value("Sheet1", 4, 9, LiteralValue::Text("zebra".into()))
        .unwrap();
    // Column K: ascending text with a blank tail.
    for (row, word) in [(1u32, "alpha"), (2, "beta"), (3, "gamma")] {
        engine
            .set_cell_value("Sheet1", row, 11, LiteralValue::Text(word.into()))
            .unwrap();
    }
    // Column M: unsorted control.
    for (row, v) in [(1u32, 3i64), (2, 1), (3, 5), (4, 2), (5, 4)] {
        num(&mut engine, row, 13, v);
    }
    // Column B: payload for VLOOKUP against column A.
    for i in 1..=5i64 {
        num(&mut engine, i as u32, 2, i * 10);
    }
    engine
}

/// Excel: `=MATCH(3,A1:A10,1)` => 3. An over-wide range whose tail is blank
/// is still sorted; the blanks are not out-of-order data.
#[test]
fn blank_tail_does_not_make_an_ascending_range_unsorted() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(3,A1:A10,1)"),
        3.0,
        "MATCH(3,A1:A10,1)",
    );
}

/// Excel: `=MATCH(3.5,A1:A10,1)` => 3. A needle between keys still selects
/// the largest key not greater than it, not a trailing blank.
#[test]
fn blank_tail_is_never_the_selected_approximate_position() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(3.5,A1:A10,1)"),
        3.0,
        "MATCH(3.5,A1:A10,1)",
    );
}

/// Excel: `=MATCH(6,A1:A10,1)` => 5. A needle past the last key selects the
/// last populated row, not the last row of the range.
#[test]
fn needle_past_the_last_key_selects_the_last_populated_row() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(6,A1:A10,1)"),
        5.0,
        "MATCH(6,A1:A10,1)",
    );
}

/// Excel: `=VLOOKUP(3,A1:B10,2,TRUE)` => 30.
#[test]
fn vlookup_approximate_tolerates_a_blank_tail() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=VLOOKUP(3,A1:B10,2,TRUE)"),
        30.0,
        "VLOOKUP(3,A1:B10,2,TRUE)",
    );
}

/// Excel: `=MATCH(4,G1:G5,1)` => 4 and `=MATCH(3,G1:G5,1)` => 2, where G3 is
/// blank. Interior blanks are skipped, and the result is the position in the
/// original range.
#[test]
fn interior_blank_is_skipped_and_positions_stay_original() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(4,G1:G5,1)"),
        4.0,
        "MATCH(4,G1:G5,1)",
    );
    assert_number(
        eval(&mut engine, "=MATCH(3,G1:G5,1)"),
        2.0,
        "MATCH(3,G1:G5,1)",
    );
}

/// Excel: `=MATCH(3,C1:C6,1)` => 4, where C1 is the text "Header". The header
/// is skipped rather than treated as out-of-order data, and the answer is
/// still counted from the top of the range.
#[test]
fn text_header_above_a_numeric_column_is_skipped() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(3,C1:C6,1)"),
        4.0,
        "MATCH(3,C1:C6,1)",
    );
}

/// Excel: `=MATCH(3.5,C1:C6,1)` => 4.
#[test]
fn text_header_is_skipped_for_a_between_keys_needle() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(3.5,C1:C6,1)"),
        4.0,
        "MATCH(3.5,C1:C6,1)",
    );
}

/// Excel: `=MATCH(2,I1:I4,1)` => 3, where I1 and I4 are text. Numbers are the
/// needle's class; the surrounding text is skipped in both directions.
#[test]
fn numeric_needle_skips_text_entries_on_both_sides() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(2,I1:I4,1)"),
        3.0,
        "MATCH(2,I1:I4,1)",
    );
}

/// Excel: `=MATCH("m",I1:I4,1)` => 1. The mirror direction: a text needle
/// searches only the text entries.
#[test]
fn text_needle_skips_numeric_entries() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(\"m\",I1:I4,1)"),
        1.0,
        "MATCH(\"m\",I1:I4,1)",
    );
}

/// Excel: `=MATCH("beta",K1:K10,1)` => 2. Text vectors get the same blank
/// tolerance as numeric ones.
#[test]
fn text_vector_with_a_blank_tail_matches() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(\"beta\",K1:K10,1)"),
        2.0,
        "MATCH(\"beta\",K1:K10,1)",
    );
}

/// Excel: `=MATCH(3,E1:E10,-1)` => 3 on a descending column with a blank
/// tail. Descending mode gets the same treatment as ascending.
#[test]
fn blank_tail_does_not_make_a_descending_range_unsorted() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(3,E1:E10,-1)"),
        3.0,
        "MATCH(3,E1:E10,-1)",
    );
}

/// Control: Excel bisects unsorted data rather than rejecting it, and here the
/// probes reach 5 and then 3, both above the needle, so the answer is `#N/A`.
/// Excel: `=MATCH(2,M1:M5,1)` => `#N/A`.
#[test]
fn genuinely_unsorted_data_is_still_na() {
    let mut engine = build_engine();
    assert_na(eval(&mut engine, "=MATCH(2,M1:M5,1)"), "MATCH(2,M1:M5,1)");
}

/// Control: a needle below every key is still `#N/A`.
/// Excel: `=MATCH(0.5,A1:A10,1)` => `#N/A`.
#[test]
fn needle_below_every_key_is_still_na() {
    let mut engine = build_engine();
    assert_na(
        eval(&mut engine, "=MATCH(0.5,A1:A10,1)"),
        "MATCH(0.5,A1:A10,1)",
    );
}

/// Control: exact match over the same blank-tailed range is unaffected.
/// Excel: `=MATCH(3,A1:A10,0)` => 3.
///
/// Note: `=MATCH(0,A1:A10,0)` is `#N/A` in Excel but returns 6 here, because
/// the exact path coerces a blank cell to numeric zero. That is a separate
/// defect on a separate code path and is filed on its own; this PR neither
/// fixes nor worsens it.
#[test]
fn exact_match_over_a_blank_tail_is_unaffected() {
    let mut engine = build_engine();
    assert_number(
        eval(&mut engine, "=MATCH(3,A1:A10,0)"),
        3.0,
        "MATCH(3,A1:A10,0)",
    );
}

#[test]
fn approximate_lookups_skip_error_entries_in_any_lookup_position() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let error = LiteralValue::Error(ExcelError::new(ExcelErrorKind::Div));

    // A = 1, 2, #DIV/0!, 9 with B = 10, 20, 30, 40.
    for (row, value) in [
        LiteralValue::Int(1),
        LiteralValue::Int(2),
        error.clone(),
        LiteralValue::Int(9),
    ]
    .into_iter()
    .enumerate()
    {
        engine
            .set_cell_value("Sheet1", row as u32 + 1, 1, value)
            .unwrap();
        engine
            .set_cell_value(
                "Sheet1",
                row as u32 + 1,
                2,
                LiteralValue::Int((row as i64 + 1) * 10),
            )
            .unwrap();
    }
    // The error sits where the search decides. Excel projects it out and
    // answers from the surviving entries: the largest value <= 5 is the 2 in
    // row 2, counted in the original range.
    assert_number(
        eval(&mut engine, "=MATCH(5,A1:A4,1)"),
        2.0,
        "MATCH ascending skips a deciding error",
    );
    assert_number(
        eval(&mut engine, "=VLOOKUP(5,A1:B4,2,TRUE)"),
        20.0,
        "VLOOKUP skips a deciding error",
    );

    // Move the error past a key that already disqualifies the final row: the
    // answer is unchanged, so skipping is not search-path dependent.
    engine
        .set_cell_value("Sheet1", 3, 1, LiteralValue::Int(9))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 4, 1, error.clone())
        .unwrap();
    assert_number(
        eval(&mut engine, "=MATCH(5,A1:A4,1)"),
        2.0,
        "MATCH ascending skips a non-deciding error",
    );
    assert_number(
        eval(&mut engine, "=VLOOKUP(5,A1:B4,2,TRUE)"),
        20.0,
        "VLOOKUP skips a non-deciding error",
    );

    for (col, value) in [
        LiteralValue::Int(1),
        LiteralValue::Int(2),
        LiteralValue::Int(9),
        error.clone(),
    ]
    .into_iter()
    .enumerate()
    {
        engine
            .set_cell_value("Sheet1", 10, col as u32 + 1, value)
            .unwrap();
        engine
            .set_cell_value(
                "Sheet1",
                11,
                col as u32 + 1,
                LiteralValue::Int((col as i64 + 1) * 10),
            )
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=HLOOKUP(5,A10:D11,2,TRUE)"),
        20.0,
        "HLOOKUP skips a non-deciding error",
    );

    // Descending: 9, #DIV/0!, 2, 1. The smallest entry >= 5 is the 9 in row 1.
    for (row, value) in [
        LiteralValue::Int(9),
        error,
        LiteralValue::Int(2),
        LiteralValue::Int(1),
    ]
    .into_iter()
    .enumerate()
    {
        engine
            .set_cell_value("Sheet1", row as u32 + 1, 5, value)
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(5,E1:E4,-1)"),
        1.0,
        "MATCH descending skips a deciding error",
    );

    // An all-error range has nothing searchable: #N/A, not the error.
    for row in 1..=3u32 {
        engine
            .set_cell_value(
                "Sheet1",
                row,
                6,
                LiteralValue::Error(ExcelError::new(ExcelErrorKind::Div)),
            )
            .unwrap();
    }
    assert_na(eval(&mut engine, "=MATCH(5,F1:F3,1)"), "all-error range");

    // Exact mode never matches an error cell.
    assert_na(
        eval(&mut engine, "=MATCH(5,F1:F3,0)"),
        "exact mode over errors",
    );
}

#[test]
fn approximate_match_skips_materialized_pre_1900_date_error() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, date) in [(1899, 1, 1), (1899, 6, 1), (1900, 3, 1), (2024, 1, 1)]
        .into_iter()
        .enumerate()
    {
        engine
            .set_cell_value(
                "Sheet1",
                row as u32 + 1,
                1,
                LiteralValue::Date(NaiveDate::from_ymd_opt(date.0, date.1, date.2).unwrap()),
            )
            .unwrap();
    }
    // The three pre-1900 dates materialize as #NUM!. Excel projects error cells
    // out of an approximate search, so the only searchable entry is the 2024
    // date in row 4, and the needle lands on it.
    assert_number(
        eval(&mut engine, "=MATCH(50000,A1:A4,1)"),
        4.0,
        "MATCH skips pre-1900 date errors",
    );
}

#[test]
fn ascending_duplicates_are_sorted_and_return_the_last_original_position() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in [(1, 1), (2, 2), (4, 2), (6, 2), (8, 5)] {
        engine
            .set_cell_value("Sheet1", row, 8, LiteralValue::Int(value))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(3,H1:H8,1)"),
        6.0,
        "MATCH duplicate ascending keys",
    );
}

#[test]
fn vlookup_maps_a_projected_match_back_to_the_original_row() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Text("Key".into()))
        .unwrap();
    for value in 1..=5i64 {
        engine
            .set_cell_value("Sheet1", value as u32 + 1, 1, LiteralValue::Int(value))
            .unwrap();
        engine
            .set_cell_value("Sheet1", value as u32 + 1, 2, LiteralValue::Int(value * 10))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=VLOOKUP(3.5,A1:B6,2,TRUE)"),
        30.0,
        "VLOOKUP original-position remap",
    );
}

#[test]
fn searchable_count_selects_the_small_descending_linear_path() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in (1..=5i64).rev().enumerate() {
        engine
            .set_cell_value("Sheet1", row as u32 + 1, 5, LiteralValue::Int(value))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(3.5,E1:E10,-1)"),
        2.0,
        "MATCH threshold uses searchable count",
    );
}

#[test]
fn descending_binary_search_returns_the_last_qualifying_position() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in (1..=10i64).rev().enumerate() {
        engine
            .set_cell_value("Sheet1", row as u32 + 1, 1, LiteralValue::Int(value))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(3.5,A1:A10,-1)"),
        7.0,
        "MATCH descending binary boundary",
    );
    assert_number(
        eval(&mut engine, "=MATCH(3,A1:A10,-1)"),
        8.0,
        "MATCH descending exact-hit masking control",
    );
}

#[test]
fn descending_binary_search_skips_interior_blanks_without_shifting_the_answer() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in [
        (1, 9),
        (2, 8),
        (3, 7),
        (5, 6),
        (6, 5),
        (7, 4),
        (9, 3),
        (10, 2),
        (11, 1),
    ] {
        engine
            .set_cell_value("Sheet1", row, 7, LiteralValue::Int(value))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(5.5,G1:G11,-1)"),
        5.0,
        "MATCH descending with interior blanks",
    );
}

#[test]
fn searchable_count_keeps_duplicate_tie_break_on_small_descending_data() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in [(1, 5), (2, 4), (3, 4), (4, 3), (5, 2)] {
        engine
            .set_cell_value("Sheet1", row, 5, LiteralValue::Int(value))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(4,E1:E10,-1)"),
        2.0,
        "MATCH threshold preserves descending duplicate tie-break",
    );
}

#[test]
fn searchable_count_not_range_extent_controls_descending_tie_break() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, value) in [(1, 5), (3, 4), (4, 4), (6, 3), (8, 2)] {
        engine
            .set_cell_value("Sheet1", row, 5, LiteralValue::Int(value))
            .unwrap();
    }
    for row in [2, 5, 7, 9, 10] {
        engine
            .set_cell_value("Sheet1", row, 5, LiteralValue::Text("skip".into()))
            .unwrap();
    }
    assert_number(
        eval(&mut engine, "=MATCH(4,E1:E10,-1)"),
        3.0,
        "MATCH threshold uses projected length, not range extent",
    );
}

/// An omitted IF false branch leaves FALSE in a numeric lookup vector. To an
/// approximate search a logical is not the number 0: Excel skips it like a
/// blank, and the answer is still counted in the full vector.
/// Excel: `=MATCH(35,IF(B1:B4="x",A1:A4),1)` => 3 with A = 10, 20, 30, 40 and
/// B = x, y, x, y.
#[test]
fn logical_entries_are_not_numeric_candidates() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, (value, group)) in [(10, "x"), (20, "y"), (30, "x"), (40, "y")]
        .into_iter()
        .enumerate()
    {
        let row = row as u32 + 1;
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Int(value))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Text(group.into()))
            .unwrap();
    }
    for (formula, expected) in [
        ("=MATCH(35,IF(B1:B4=\"x\",A1:A4),1)", 3.0),
        ("=MATCH(35,IF(B1:B4=\"y\",A1:A4),1)", 2.0),
        ("=MATCH(25,{10;20;FALSE;30},1)", 2.0),
        ("=MATCH(25,{FALSE;10;20;30},1)", 3.0),
        ("=LOOKUP(25,{10;TRUE;20},{1;2;3})", 3.0),
        // Control: a vector with no logicals is unchanged.
        ("=MATCH(25,{10;20;30},1)", 2.0),
    ] {
        assert_number(eval(&mut engine, formula), expected, formula);
    }
    assert_eq!(
        eval(
            &mut engine,
            "=VLOOKUP(25,{10,\"a\";TRUE,\"b\";20,\"c\"},2,TRUE)"
        ),
        Some(LiteralValue::Text("c".into()))
    );
    // FALSE is not a 0 below the needle, so nothing qualifies.
    assert_na(
        eval(&mut engine, "=MATCH(5,{FALSE;10;20},1)"),
        "MATCH(5,{FALSE;10;20},1)",
    );
}

/// The same type rule from the other sides: numeric-looking text is text, so a
/// numeric needle skips it and a text needle orders it as text ("10" < "9"),
/// and a logical needle searches only the logicals.
#[test]
fn numeric_text_and_logical_needles_search_their_own_type() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    assert_na(
        eval(&mut engine, "=MATCH(15,{\"10\";20;30},1)"),
        "MATCH(15,{\"10\";20;30},1)",
    );
    for (formula, expected) in [
        ("=MATCH(25,{\"10\";20;30},1)", 2.0),
        ("=MATCH(\"9\",{\"10\";\"20\";\"30\"},1)", 3.0),
        ("=MATCH(TRUE,{1;FALSE;TRUE},1)", 3.0),
        ("=MATCH(FALSE,{0;FALSE;TRUE},1)", 2.0),
    ] {
        assert_number(eval(&mut engine, formula), expected, formula);
    }
}

/// Text keys stored in numeric order ("1".."10") are out of text order, since
/// Excel compares text with text as text ("10" < "2"). Excel never checks the
/// order of an approximate search: it bisects, and for "5" every probe lands
/// on "3".."8" before settling on "5", so MATCH gives 5 and VLOOKUP the value
/// beside it. The engine bisects like Excel instead of answering #N/A. The
/// whole column D:D is bisected over its full height, and its blank tail
/// leads the probes down to the same "5".
#[test]
fn text_keys_out_of_text_order_are_bisected_not_rejected() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for i in 1..=10u32 {
        engine
            .set_cell_value("Sheet1", i, 4, LiteralValue::Text(i.to_string()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", i, 5, LiteralValue::Int(i as i64 * 100))
            .unwrap();
    }
    let keys = (1..=10).map(|i| format!("\"{i}\"")).collect::<Vec<_>>();
    let column = keys.join(";");
    let table_row = format!(
        "{};{}",
        keys.join(","),
        (1..=10)
            .map(|i| (i * 100).to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    for (formula, expected) in [
        ("=MATCH(\"5\",D1:D10,1)".to_string(), 5.0),
        ("=MATCH(\"5\",D1:D10)".to_string(), 5.0),
        ("=MATCH(\"5\",D:D,1)".to_string(), 5.0),
        (format!("=MATCH(\"5\",{{{column}}},1)"), 5.0),
        ("=VLOOKUP(\"5\",D1:E10,2)".to_string(), 500.0),
        ("=VLOOKUP(\"5\",D:E,2,TRUE)".to_string(), 500.0),
        (format!("=HLOOKUP(\"5\",{{{table_row}}},2,TRUE)"), 500.0),
        // Descending search over the same keys: "10" sorts below "5" as text
        // but is never probed; the run of qualifying entries ends at "5".
        (
            "=MATCH(\"5\",{\"10\";\"9\";\"8\";\"7\";\"6\";\"5\";\"4\";\"3\";\"2\";\"1\"},-1)"
                .to_string(),
            6.0,
        ),
        // A short vector is bisected too, not scanned for the last entry
        // <= "10": the first probe "2" is above "10", so the answer is "1".
        ("=MATCH(\"10\",{\"1\";\"2\";\"10\"},1)".to_string(), 1.0),
        // Keys in text order are unchanged.
        ("=MATCH(\"10\",{\"1\";\"10\";\"2\"},1)".to_string(), 2.0),
    ] {
        assert_number(eval(&mut engine, &formula), expected, &formula);
    }
    // A text needle below every probed key still finds nothing.
    assert_na(
        eval(&mut engine, "=MATCH(\"0\",D1:D10,1)"),
        "MATCH(\"0\",D1:D10,1)",
    );
}
fn eval_at(engine: &mut Engine<TestWorkbook>, formula: &str) -> Option<LiteralValue> {
    engine
        .set_cell_formula("Sheet1", 30, 30, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 30, 30)
}

/// Excel's approximate MATCH, VLOOKUP and HLOOKUP bisect the lookup vector
/// without checking its order: inclusive bounds, a floor midpoint, a probe on
/// a skipped entry moving on to the next searched one. On unsorted data the
/// answer is wherever the probes lead.
#[test]
fn unsorted_data_is_bisected_not_rejected() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // A = 1, 2, 3, 10, 4, 4 and B = 10..60.
    for (row, value) in [1, 2, 3, 10, 4, 4].into_iter().enumerate() {
        let row = row as u32 + 1;
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Int(value))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Int(row as i64 * 10))
            .unwrap();
    }
    // Probes 3, 4, 4: the last entry not above 5 is row 6.
    for (formula, expected) in [
        ("=MATCH(5,A1:A6,1)", 6.0),
        ("=VLOOKUP(5,A1:B6,2,TRUE)", 60.0),
        ("=MATCH(30,{10,30,20,40,50},1)", 3.0),
        ("=MATCH(45,{30,10,50,20,40},1)", 2.0),
        ("=MATCH(30,{50,30,40,20,10},-1)", 3.0),
    ] {
        assert_number(eval_at(&mut engine, formula), expected, formula);
    }
    // An exact probe ends the search.
    let titles = "{\"Winter Guard\";\"Iron man\";\"Infinity War\";\"Deadpool AGAIN\";\"Deadpool\";\"Black Cat\"}";
    assert_number(
        eval_at(&mut engine, &format!("=MATCH(\"Infinity War\",{titles},1)")),
        3.0,
        "exact probe on unsorted text",
    );
    assert_na(
        eval_at(&mut engine, &format!("=MATCH(\"Black Cat\",{titles},1)")),
        "probes above the needle on unsorted text",
    );
}

/// A whole column or row is bisected over its full height or width, as
/// written, not over the used cells: the blank tail moves the early probes
/// left and changes where they land on unsorted data. A bounded range is
/// bisected over its own length.
#[test]
fn whole_column_is_bisected_over_its_full_height() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // A = 3, "h", 5, 6, 7, 8, 1, 1 with B = 10..80; row 20 holds the same
    // keys across J20:Q20 with the payload in row 21. D = 9, 8, 1, 7, 6.
    let keys = [
        LiteralValue::Int(3),
        LiteralValue::Text("h".into()),
        LiteralValue::Int(5),
        LiteralValue::Int(6),
        LiteralValue::Int(7),
        LiteralValue::Int(8),
        LiteralValue::Int(1),
        LiteralValue::Int(1),
    ];
    for (i, key) in keys.into_iter().enumerate() {
        let i = i as u32 + 1;
        engine.set_cell_value("Sheet1", i, 1, key.clone()).unwrap();
        engine
            .set_cell_value("Sheet1", i, 2, LiteralValue::Int(i as i64 * 10))
            .unwrap();
        engine.set_cell_value("Sheet1", 20, i + 9, key).unwrap();
        engine
            .set_cell_value("Sheet1", 21, i + 9, LiteralValue::Int(i as i64 * 10))
            .unwrap();
    }
    for (row, value) in [9, 8, 1, 7, 6].into_iter().enumerate() {
        engine
            .set_cell_value("Sheet1", row as u32 + 1, 4, LiteralValue::Int(value))
            .unwrap();
    }
    for (formula, expected) in [
        ("=MATCH(6.5,A1:A8,1)", 4.0),
        ("=MATCH(6.5,A:A,1)", 8.0),
        ("=VLOOKUP(6.5,A1:B8,2,TRUE)", 40.0),
        ("=VLOOKUP(6.5,A:B,2,TRUE)", 80.0),
        ("=HLOOKUP(6.5,J20:Q21,2,TRUE)", 40.0),
        ("=HLOOKUP(6.5,20:21,2,TRUE)", 80.0),
        ("=MATCH(6.5,D1:D5,-1)", 2.0),
        ("=MATCH(6.5,D:D,-1)", 4.0),
    ] {
        assert_number(eval_at(&mut engine, formula), expected, formula);
    }
}

/// An exact hit ends Excel's bisection: Excel walks from it through the run
/// of equal entries next to it and returns the run's last entry (the first,
/// for a descending search), not a later equal entry beyond a smaller one.
/// Excel (Microsoft Q&A "Binary search explain in Lookup with duplicates"):
/// `=MATCH(1,{1,1,1,1,1,0,1},1)` => 5, `=MATCH(1,{1,1,1,1,1,0,1,-1,1,1,1,1,1,0,1})`
/// => 13, `=MATCH(1,{1,1,1,1,1,0,1,2,1,1,1,1,1,0,1})` => 5,
/// `=LOOKUP(1,{1;1;1;1;1},{"a";"b";"c";"d";"e"})` => "e" and
/// `=LOOKUP(3,{5;3;1;2;4},{"a";"b";"c";"d";"e"})` => "d".
#[test]
fn exact_hit_ends_the_bisection_at_the_end_of_its_run() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // A = 1, 1, 1, 1, 1, 0, 1 with B = 10..70.
    for (row, value) in [1, 1, 1, 1, 1, 0, 1].into_iter().enumerate() {
        let row = row as u32 + 1;
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Int(value))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Int(row as i64 * 10))
            .unwrap();
    }
    for (formula, expected) in [
        ("=MATCH(1,{1,1,1,1,1,0,1},1)", 5.0),
        ("=MATCH(1,{1,1,1,1,1,0,1,-1,1,1,1,1,1,0,1})", 13.0),
        ("=MATCH(1,{1,1,1,1,1,0,1,2,1,1,1,1,1,0,1})", 5.0),
        ("=MATCH(1,A1:A7,1)", 5.0),
        ("=VLOOKUP(1,A1:B7,2,TRUE)", 50.0),
        ("=LOOKUP(1,A1:A7,B1:B7)", 50.0),
        // The mirror for a descending search: the first entry of the run.
        ("=MATCH(4,{9,4,9,4,4,4,4},-1)", 4.0),
    ] {
        assert_number(eval_at(&mut engine, formula), expected, formula);
    }
    for (formula, expected) in [
        (
            "=LOOKUP(1,{1;1;1;1;1},{\"a\";\"b\";\"c\";\"d\";\"e\"})",
            "e",
        ),
        (
            "=LOOKUP(3,{5;3;1;2;4},{\"a\";\"b\";\"c\";\"d\";\"e\"})",
            "d",
        ),
    ] {
        assert_eq!(
            eval_at(&mut engine, formula),
            Some(LiteralValue::Text(expected.into())),
            "{formula}"
        );
    }
}

/// Evaluates `formula` on Sheet2 against a Sheet1 holding only A1 = 10 and
/// B1 = 100, so every other cell of a whole row or column of Sheet1 lies past
/// its used range.
fn eval_against_sparse_sheet(formula: &str) -> Option<LiteralValue> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine.graph.add_sheet("Sheet2").unwrap();
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Int(10))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 1, 2, LiteralValue::Int(100))
        .unwrap();
    engine
        .set_cell_formula("Sheet2", 1, 1, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet2", 1, 1)
}

/// VLOOKUP and HLOOKUP return #REF! only for an index past table_array as
/// written: whole rows 1:2 span 16,384 columns and whole columns A:B
/// 1,048,576 rows, so an index past the used cells reads a blank cell (0).
#[test]
fn return_index_is_bounded_by_the_table_as_written() {
    for (formula, expected) in [
        ("=VLOOKUP(10,Sheet1!1:2,100,TRUE)", 0.0),
        ("=HLOOKUP(10,Sheet1!A:B,100,TRUE)", 0.0),
        ("=VLOOKUP(10,Sheet1!1:2,100,FALSE)", 0.0),
        ("=HLOOKUP(10,Sheet1!A:B,100,FALSE)", 0.0),
        ("=VLOOKUP(10,Sheet1!1:2,16384,TRUE)", 0.0),
        ("=HLOOKUP(10,Sheet1!A:B,1048576,TRUE)", 0.0),
        // Control: a used return cell.
        ("=VLOOKUP(10,Sheet1!1:2,2,TRUE)", 100.0),
    ] {
        assert_number(eval_against_sparse_sheet(formula), expected, formula);
    }
    for formula in [
        "=VLOOKUP(10,Sheet1!A:B,3,TRUE)",
        "=HLOOKUP(10,Sheet1!1:2,3,TRUE)",
        "=VLOOKUP(10,Sheet1!A1:B5,3,FALSE)",
    ] {
        assert_error(
            eval_against_sparse_sheet(formula),
            ExcelErrorKind::Ref,
            formula,
        );
    }
}

/// MATCH's lookup array must be one row or one column (MS-OI29500 2.1.990,
/// "MATCH"): a two-dimensional array or reference gives #N/A in every match
/// type and is never flattened into a vector.
#[test]
fn match_rejects_a_two_dimensional_lookup_array() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // X1:Y2 = 1, 2; 3, 4.
    for (row, col, value) in [(1, 24, 1), (1, 25, 2), (2, 24, 3), (2, 25, 4)] {
        engine
            .set_cell_value("Sheet1", row, col, LiteralValue::Int(value))
            .unwrap();
    }
    for formula in [
        "=MATCH(3,{1,2;3,4},1)",
        "=MATCH(3,{1,2;3,4},0)",
        "=MATCH(3,{4,3;2,1},-1)",
        "=MATCH(3,X1:Y2,1)",
        "=MATCH(3,X1:Y2,0)",
        "=MATCH(3,X1:Y2,-1)",
        "=MATCH(3,X:Y,1)",
    ] {
        assert_na(eval_at(&mut engine, formula), formula);
    }
    // Controls: one row and one column.
    for (formula, expected) in [
        ("=MATCH(3,{1,2,3,4},1)", 3.0),
        ("=MATCH(3,{1;2;3;4},1)", 3.0),
        ("=MATCH(3,X1:X2,1)", 2.0),
        ("=MATCH(2,X1:Y1,1)", 2.0),
    ] {
        assert_number(eval_at(&mut engine, formula), expected, formula);
    }
}

/// Lookups compare numbers by their exact values, with no tolerance: 5E-13
/// and 1E-13 are above 0, so an ascending approximate lookup for 0 has
/// nothing at or below it (the logicals beside 1E-13 are skipped, not taken
/// for 0), exact-or-next-smaller misses, and an exact lookup finds no 0.
/// 0.1+0.2 is 0.30000000000000004: above 0.3 to an approximate search and not
/// 0.3 to an exact one, though `(0.1+0.2)=0.3` is TRUE, the `=` operator
/// rounding both sides to 15 significant digits.
#[test]
fn lookups_compare_numbers_exactly() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // X1:X2 = 5E-13, 1 with Y1:Y2 = 10, 20; X3:X4 = 0.1, =0.1+0.2 with
    // Y3:Y4 = 30, 40.
    for (row, key, payload) in [(1, 0.0000000000005, 10), (2, 1.0, 20), (3, 0.1, 30)] {
        engine
            .set_cell_value("Sheet1", row, 24, LiteralValue::Number(key))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 25, LiteralValue::Int(payload))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 4, 24, parse("=0.1+0.2").unwrap())
        .unwrap();
    engine
        .set_cell_value("Sheet1", 4, 25, LiteralValue::Int(40))
        .unwrap();
    for formula in [
        "=MATCH(0,{0.0000000000005,1},1)",
        "=LOOKUP(0,{0.0000000000005,1},{10,20})",
        "=VLOOKUP(0,{0.0000000000005,10;1,20},2,TRUE)",
        "=MATCH(0,X1:X2,1)",
        "=VLOOKUP(0,X1:Y2,2,TRUE)",
        "=MATCH(0,{0.0000000000005,1},0)",
        "=MATCH(0,X1:X2,0)",
        "=VLOOKUP(0,X1:Y2,2,FALSE)",
        "=XMATCH(0,X1:X2)",
        "=MATCH(0,{1E-13},1)",
        "=MATCH(0,{1E-13;FALSE;TRUE},1)",
        "=LOOKUP(0,{1E-13;FALSE;TRUE},{\"number\";\"false\";\"true\"})",
        "=VLOOKUP(0,{1E-13,\"number\";FALSE,\"false\";TRUE,\"true\"},2,TRUE)",
        "=HLOOKUP(0,{1E-13,FALSE,TRUE;\"number\",\"false\",\"true\"},2,TRUE)",
        "=MATCH(0.3,X4,1)",
        "=MATCH(0.3,X4,0)",
        "=MATCH(0.1+0.2,{0.1,0.3},0)",
        "=VLOOKUP(0.3,X4:Y4,2,FALSE)",
        "=XMATCH(0.3,X3:X4)",
    ] {
        assert_na(eval_at(&mut engine, formula), formula);
    }
    for formula in [
        "=XLOOKUP(0,{0.0000000000005,1},{10,20},\"NF\",-1)",
        "=XLOOKUP(0,{1E-13},{10},\"NF\",-1)",
    ] {
        assert_eq!(
            eval_at(&mut engine, formula),
            Some(LiteralValue::Text("NF".into())),
            "{formula}"
        );
    }
    for (formula, expected) in [
        ("=XLOOKUP(0,{0.0000000000005,1},{10,20},\"NF\",1)", 10.0),
        ("=MATCH(0.0000000000005,X1:X2,0)", 1.0),
        ("=MATCH(0.0000000000005,X1:X2,1)", 1.0),
        // 0.30000000000000004 is above 0.3: the last entry at or below 0.3 is
        // 0.1, and it is the next larger entry, not an exact match.
        ("=MATCH(0.3,X3:X4,1)", 1.0),
        ("=VLOOKUP(0.3,X3:Y4,2,TRUE)", 30.0),
        ("=LOOKUP(0.3,X3:X4,Y3:Y4)", 30.0),
        ("=XLOOKUP(0.3,X3:X4,Y3:Y4,\"NF\",-1)", 30.0),
        ("=XLOOKUP(0.3,X3:X4,Y3:Y4,\"NF\",1)", 40.0),
        // The same computation finds itself.
        ("=MATCH(0.1+0.2,X3:X4,0)", 2.0),
        ("=MATCH(0.1+0.2,X3:X4,1)", 2.0),
        ("=XLOOKUP(0.1+0.2,X3:X4,Y3:Y4,\"NF\",-1)", 40.0),
    ] {
        assert_number(eval_at(&mut engine, formula), expected, formula);
    }
    // Control: the = operator still compares to 15 significant digits.
    assert_eq!(
        eval_at(&mut engine, "=(0.1+0.2)=0.3"),
        Some(LiteralValue::Boolean(true))
    );
}

/// An exact lookup over a column long enough for the engine's lookup index
/// still compares exactly: the number one step above 3 is not 3, though the
/// index files near-integers under the integer.
#[test]
fn exact_lookup_over_a_long_column_compares_exactly() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let above_three = f64::from_bits(3.0f64.to_bits() + 1);
    for row in 1..=200u32 {
        let key = match row {
            5 => LiteralValue::Number(above_three),
            150 => LiteralValue::Int(3),
            _ => LiteralValue::Int(1000 + row as i64),
        };
        engine.set_cell_value("Sheet1", row, 24, key).unwrap();
        engine
            .set_cell_value("Sheet1", row, 25, LiteralValue::Int(row as i64))
            .unwrap();
    }
    for (formula, expected) in [
        ("=MATCH(3,X1:X200,0)", 150.0),
        ("=VLOOKUP(3,X1:Y200,2,FALSE)", 150.0),
        ("=XLOOKUP(3,X1:X200,Y1:Y200)", 150.0),
        ("=XMATCH(3,X:X)", 150.0),
    ] {
        for _ in 0..3 {
            assert_number(eval_at(&mut engine, formula), expected, formula);
        }
    }
}
