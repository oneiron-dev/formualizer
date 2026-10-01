//! INDEX/OFFSET over unbounded whole-column/whole-row ranges (issue #162).
//!
//! INDEX (and OFFSET) used to bail with #REF! whenever the array argument had
//! any unbounded dimension (B:B, 2:2, Data!$A:$C, Data!1:2). These tests pin
//! the fixed behavior: unbounded dimensions are clamped to the used region via
//! `resolve_range_view`, exactly like MATCH/VLOOKUP.

use crate::engine::{Engine, EvalConfig, FormulaPlaneMode};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn new_engine() -> Engine<TestWorkbook> {
    Engine::new(TestWorkbook::new(), EvalConfig::default())
}

fn assert_number(engine: &Engine<TestWorkbook>, sheet: &str, row: u32, col: u32, expected: f64) {
    match engine.get_cell_value(sheet, row, col) {
        Some(LiteralValue::Number(n)) => {
            assert!(
                (n - expected).abs() < 1e-9,
                "{sheet}!R{row}C{col}: expected {expected}, got {n}"
            )
        }
        Some(LiteralValue::Int(i)) => {
            assert_eq!(i as f64, expected, "{sheet}!R{row}C{col}")
        }
        other => panic!("{sheet}!R{row}C{col}: expected {expected}, got {other:?}"),
    }
}

#[test]
fn index_whole_column_same_sheet() {
    let mut engine = new_engine();
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(42))
        .unwrap();
    // Formula placed outside column B so the whole-column reference is not
    // self-inclusive.
    engine
        .set_cell_formula("Sheet1", 1, 4, parse("=INDEX(B:B,2,1)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 4, 42.0);
}

#[test]
fn index_whole_row_same_sheet() {
    let mut engine = new_engine();
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(42))
        .unwrap();
    // Formula placed outside row 2 so the whole-row reference is not
    // self-inclusive.
    engine
        .set_cell_formula("Sheet1", 5, 4, parse("=INDEX(2:2,1,2)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 5, 4, 42.0);
}

#[test]
fn index_whole_column_cross_sheet() {
    let mut engine = new_engine();
    engine.add_sheet("Data").unwrap();
    engine
        .set_cell_value("Data", 2, 2, LiteralValue::Int(42))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=INDEX(Data!B:B,2,1)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 1, 42.0);
}

#[test]
fn index_multi_whole_row_cross_sheet() {
    let mut engine = new_engine();
    engine.add_sheet("Data").unwrap();
    engine
        .set_cell_value("Data", 2, 2, LiteralValue::Int(42))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=INDEX(Data!1:2,2,2)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 1, 42.0);
}

#[test]
fn index_unbounded_with_match_row_and_col() {
    // The exact shape from issue #162:
    // =INDEX(Data!$A:$C, MATCH("row",Data!$A:$A,0), MATCH("col",Data!$1:$1,0))
    let mut engine = new_engine();
    engine.add_sheet("Data").unwrap();
    engine
        .set_cell_value("Data", 1, 2, LiteralValue::Text("col".into()))
        .unwrap();
    engine
        .set_cell_value("Data", 2, 1, LiteralValue::Text("row".into()))
        .unwrap();
    engine
        .set_cell_value("Data", 2, 2, LiteralValue::Int(42))
        .unwrap();
    engine
        .set_cell_formula(
            "Sheet1",
            1,
            1,
            parse("=INDEX(Data!$A:$C, MATCH(\"row\",Data!$A:$A,0), MATCH(\"col\",Data!$1:$1,0))")
                .unwrap(),
        )
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 1, 42.0);
}

#[test]
fn index_whole_column_zero_row_returns_entire_used_column() {
    // Interaction with INDEX(range, 0, c) from PR #156: row_num == 0 over an
    // unbounded column yields the clamped whole column.
    let mut engine = new_engine();
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(42))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 3, 2, LiteralValue::Int(8))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 4, parse("=SUM(INDEX(B:B,0,1))").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 4, 50.0);
}

#[test]
fn index_whole_column_past_the_data_is_a_blank_cell() {
    let mut engine = new_engine();
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(42))
        .unwrap();
    // B:B is rows 1-1048576 whatever its data: row 5 is a blank cell.
    engine
        .set_cell_formula("Sheet1", 1, 4, parse("=INDEX(B:B,5,1)&\"\"").unwrap())
        .unwrap();
    // Beyond the grid and negative indexes are #REF!.
    engine
        .set_cell_formula("Sheet1", 2, 4, parse("=INDEX(B:B,-1,1)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 3, 4, parse("=INDEX(B:B,1048577,1)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 4),
        Some(LiteralValue::Text(String::new()))
    );
    for row in [2u32, 3u32] {
        match engine.get_cell_value("Sheet1", row, 4) {
            Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Ref),
            other => panic!("Sheet1!R{row}C4: expected #REF!, got {other:?}"),
        }
    }
}

#[test]
fn dynamic_index_range_self_loop_uses_selected_reference_in_every_mode() {
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        engine
            .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(42))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 2, 1, parse("=INDEX(2:2,1,2)").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 3, 1, parse("=INDEX(3:3,1,1)").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 2, 5, parse("=INDEX(E:E,2)").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 8, 8, parse("=INDEX(8:8,8)").unwrap())
            .unwrap();
        engine.evaluate_all().unwrap();
        assert_number(&engine, "Sheet1", 2, 1, 42.0);
        for (row, col) in [(3, 1), (2, 5), (8, 8)] {
            match engine.get_cell_value("Sheet1", row, col) {
                Some(LiteralValue::Error(error)) => {
                    assert_eq!(error.kind, ExcelErrorKind::Circ, "{mode:?}")
                }
                other => panic!("{mode:?} Sheet1!R{row}C{col}: expected #CIRC!, got {other:?}"),
            }
        }
    }
}

#[test]
fn static_index_self_loop_classification_matches_index_reference_semantics() {
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        engine
            .set_cell_value("Sheet1", 1, 1, LiteralValue::Int(42))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 100, 1, parse("=INDEX(A1:A100,1)").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 2, 2, parse("=INDEX(B1:B100,2)").unwrap())
            .unwrap();
        engine
            .set_cell_value("Sheet1", 2, 3, LiteralValue::Int(42))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 2, 1, parse("=SUM(INDEX(2:2,0,3))").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 3, 1, parse("=SUM(INDEX(3:3,1,0))").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 4, 1, parse("=INDEX(4:4,-1,1)").unwrap())
            .unwrap();
        engine
            .set_cell_value("Sheet1", 5, 2, LiteralValue::Int(1))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 5, 1, parse("=INDEX(5:5,1,2)+SUM(5:5)").unwrap())
            .unwrap();

        engine.evaluate_all().unwrap();
        assert_number(&engine, "Sheet1", 100, 1, 42.0);
        assert_number(&engine, "Sheet1", 2, 1, 42.0);
        for (row, kind) in [
            (2, ExcelErrorKind::Circ),
            (3, ExcelErrorKind::Circ),
            (4, ExcelErrorKind::Ref),
            (5, ExcelErrorKind::Circ),
        ] {
            let col = if row == 2 { 2 } else { 1 };
            match engine.get_cell_value("Sheet1", row, col) {
                Some(LiteralValue::Error(error)) => assert_eq!(error.kind, kind, "{mode:?}"),
                other => panic!("{mode:?} Sheet1!R{row}C{col}: expected {kind:?}, got {other:?}"),
            }
        }
    }
}

#[test]
fn static_index_self_loop_omitted_column_selects_entire_row() {
    // INDEX(range, r) on a multi-column range selects the entire row r, so the
    // row containing the formula is a self-loop and any other row is not.
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        for col in 5..=7 {
            engine
                .set_cell_value("Sheet1", 2, col, LiteralValue::Int(col as i64))
                .unwrap();
        }
        engine
            .set_cell_formula("Sheet1", 1, 3, parse("=SUM(INDEX(A1:C100,1))").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 3, 6, parse("=SUM(INDEX(E1:G100,2))").unwrap())
            .unwrap();

        engine.evaluate_all().unwrap();
        assert_number(&engine, "Sheet1", 3, 6, 18.0);
        match engine.get_cell_value("Sheet1", 1, 3) {
            Some(LiteralValue::Error(error)) => {
                assert_eq!(error.kind, ExcelErrorKind::Circ, "{mode:?}")
            }
            other => panic!("{mode:?} Sheet1!C1: expected #CIRC!, got {other:?}"),
        }
    }
}

#[test]
fn static_index_self_loop_whole_columns_used_in_one_row_select_entire_row() {
    // INDEX(A:C,1) is the entire row A1:C1, like INDEX(A:C,1,0): a whole column
    // spans the full grid, so it is never a single row, even when every used
    // cell of A:C sits in row 1. A formula in C1 reading it is circular.
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        for (col, value) in [(1, 1), (2, 2), (5, 1), (6, 2), (9, 1), (10, 2)] {
            engine
                .set_cell_value("Sheet1", 1, col, LiteralValue::Int(value))
                .unwrap();
        }
        engine
            .set_cell_formula("Sheet1", 1, 3, parse("=SUM(INDEX(A:C,1))").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 1, 7, parse("=SUM(INDEX(E:G,1,0))").unwrap())
            .unwrap();
        // A row of the whole columns that does not hold the formula is no loop.
        engine
            .set_cell_formula("Sheet1", 2, 11, parse("=SUM(INDEX(I:K,1))").unwrap())
            .unwrap();
        // A lone index on a single whole row still selects a column.
        engine
            .set_cell_value("Sheet1", 5, 2, LiteralValue::Int(7))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 5, 1, parse("=INDEX(5:5,2)").unwrap())
            .unwrap();

        engine.evaluate_all().unwrap();
        assert_number(&engine, "Sheet1", 2, 11, 3.0);
        assert_number(&engine, "Sheet1", 5, 1, 7.0);
        for (row, col) in [(1, 3), (1, 7)] {
            match engine.get_cell_value("Sheet1", row, col) {
                Some(LiteralValue::Error(error)) => {
                    assert_eq!(error.kind, ExcelErrorKind::Circ, "{mode:?}")
                }
                other => panic!("{mode:?} Sheet1!R{row}C{col}: expected #CIRC!, got {other:?}"),
            }
        }
    }
}

#[test]
fn index_whole_columns_omitted_column_selects_entire_row() {
    let mut engine = new_engine();
    for col in 1..=3 {
        engine
            .set_cell_value("Sheet1", 2, col, LiteralValue::Int(col as i64 * 10))
            .unwrap();
    }
    // A dynamic-array formula spills the whole row.
    engine
        .set_cell_formula("Sheet1", 5, 5, parse("=INDEX(A:C,2)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 6, 5, parse("=SUM(INDEX(A:C,2))").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    for (col, expected) in [(5, 10.0), (6, 20.0), (7, 30.0)] {
        assert_number(&engine, "Sheet1", 5, col, expected);
    }
    assert_number(&engine, "Sheet1", 6, 5, 60.0);
}

#[test]
fn index_omitted_column_row_under_legacy_array_semantics() {
    let mut engine = new_engine();
    for col in 1..=3 {
        engine
            .set_cell_value("Sheet1", 2, col, LiteralValue::Int(col as i64 * 10))
            .unwrap();
    }
    // A legacy (CSE) array over E8:G8 fills with the selected row.
    engine
        .set_cell_formula("Sheet1", 8, 5, parse("=INDEX(A:C,2)").unwrap())
        .unwrap();
    // Ordinary formulas take the implicit intersection of the row reference:
    // the formula's own column inside A:C, #VALUE! outside it.
    engine
        .set_cell_formula("Sheet1", 9, 2, parse("=INDEX(A1:C3,2)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 9, 5, parse("=INDEX(A1:C3,2)").unwrap())
        .unwrap();
    engine.use_legacy_array_semantics();
    engine.declare_array_formula("Sheet1", 8, 5, 1, 3, false);
    engine.evaluate_all().unwrap();

    for (col, expected) in [(5, 10.0), (6, 20.0), (7, 30.0)] {
        assert_number(&engine, "Sheet1", 8, col, expected);
    }
    assert_number(&engine, "Sheet1", 9, 2, 20.0);
    match engine.get_cell_value("Sheet1", 9, 5) {
        Some(LiteralValue::Error(error)) => assert_eq!(error.kind, ExcelErrorKind::Value),
        other => panic!("Sheet1!E9: expected #VALUE!, got {other:?}"),
    }
}

/// A1:C3 = 1 2 3 / 10 20 30 / 7 8 9, A5:C5 = "M10" "M20" "M30" and
/// M1:M40 = 1..40.
fn single_value_argument_engine() -> Engine<TestWorkbook> {
    let mut engine = new_engine();
    for (row, values) in [(1, [1, 2, 3]), (2, [10, 20, 30]), (3, [7, 8, 9])] {
        for (col, value) in (1..).zip(values) {
            engine
                .set_cell_value("Sheet1", row, col, LiteralValue::Int(value))
                .unwrap();
        }
    }
    for (col, text) in (1..).zip(["M10", "M20", "M30"]) {
        engine
            .set_cell_value("Sheet1", 5, col, LiteralValue::Text(text.into()))
            .unwrap();
    }
    for row in 1..=40 {
        engine
            .set_cell_value("Sheet1", row, 13, LiteralValue::Int(row as i64))
            .unwrap();
    }
    engine
}

fn assert_text(engine: &Engine<TestWorkbook>, row: u32, col: u32, expected: &str) {
    match engine.get_cell_value("Sheet1", row, col) {
        Some(LiteralValue::Text(text)) => assert_eq!(text, expected, "Sheet1!R{row}C{col}"),
        other => panic!("Sheet1!R{row}C{col}: expected {expected:?}, got {other:?}"),
    }
}

fn assert_value_error(engine: &Engine<TestWorkbook>, row: u32, col: u32) {
    match engine.get_cell_value("Sheet1", row, col) {
        Some(LiteralValue::Error(error)) => {
            assert_eq!(error.kind, ExcelErrorKind::Value, "Sheet1!R{row}C{col}")
        }
        other => panic!("Sheet1!R{row}C{col}: expected #VALUE!, got {other:?}"),
    }
}

#[test]
fn single_value_arguments_intersect_a_row_in_legacy_formulas() {
    // In an ordinary legacy formula a multi-cell reference passed to a
    // single-value parameter (INDEX's row_num/column_num, OFFSET's rows,
    // CHOOSE's index_num, INDIRECT's ref_text) or used as an operand takes the
    // implicit intersection with the formula cell: its column of a row,
    // #VALUE! when the formula sits outside the row's columns.
    let mut engine = single_value_argument_engine();
    let formulas = [
        "=INDEX($M$1:$M$40,INDEX($A$1:$C$3,2))",
        "=INDEX($M$1:$M$40,INDEX($A$1:$C$3,2),1)",
        "=OFFSET($M$1,INDEX($A$1:$C$3,2),0)",
        "=SUM(OFFSET($M$1,INDEX($A$1:$C$3,2),0,2))",
        "=CHOOSE(INDEX($A$1:$C$3,1),\"p\",\"q\",\"r\")",
        "=INDIRECT($A$5:$C$5)",
        "=INDEX($M$1:$M$40,$A$1:$C$1+0)",
        // A single-cell range is its own value wherever the formula is.
        "=INDEX($M$1:$M$40,$A$2:$A$2)",
        "=CHOOSE(INDEX($A$1:$C$3,3)-6,\"p\",\"q\",\"r\")",
        "=INDIRECT(\"M\"&INDEX($A$1:$C$3,2))",
    ];
    for (row, formula) in (60..).zip(formulas) {
        for col in [1, 2, 5] {
            engine
                .set_cell_formula("Sheet1", row, col, parse(formula).unwrap())
                .unwrap();
        }
    }
    engine.use_legacy_array_semantics();
    engine.evaluate_all().unwrap();

    for row in [60, 61] {
        assert_number(&engine, "Sheet1", row, 1, 10.0);
        assert_number(&engine, "Sheet1", row, 2, 20.0);
        assert_value_error(&engine, row, 5);
    }
    assert_number(&engine, "Sheet1", 62, 1, 11.0);
    assert_number(&engine, "Sheet1", 62, 2, 21.0);
    assert_value_error(&engine, 62, 5);
    assert_number(&engine, "Sheet1", 63, 1, 23.0);
    assert_number(&engine, "Sheet1", 63, 2, 43.0);
    assert_value_error(&engine, 63, 5);
    assert_text(&engine, 64, 1, "p");
    assert_text(&engine, 64, 2, "q");
    assert_value_error(&engine, 64, 5);
    assert_number(&engine, "Sheet1", 65, 1, 10.0);
    assert_number(&engine, "Sheet1", 65, 2, 20.0);
    assert_value_error(&engine, 65, 5);
    for col in [1, 2, 5] {
        assert_number(&engine, "Sheet1", 67, col, 10.0);
    }
    // An operator intersects its reference operand first.
    assert_number(&engine, "Sheet1", 66, 1, 1.0);
    assert_number(&engine, "Sheet1", 66, 2, 2.0);
    assert_value_error(&engine, 66, 5);
    assert_text(&engine, 68, 1, "p");
    assert_text(&engine, 68, 2, "q");
    assert_value_error(&engine, 68, 5);
    assert_number(&engine, "Sheet1", 69, 1, 10.0);
    assert_number(&engine, "Sheet1", 69, 2, 20.0);
    assert_value_error(&engine, 69, 5);
}

#[test]
fn single_value_arguments_lift_over_a_row_in_legacy_array_formulas() {
    // OFFSET and INDIRECT over the row give an array of references, which N
    // reads one reference at a time.
    let mut engine = single_value_argument_engine();
    let formulas = [
        "=INDEX($M$1:$M$40,INDEX($A$1:$C$3,2))",
        "=N(OFFSET($M$1,INDEX($A$1:$C$3,2),0))",
        "=CHOOSE(INDEX($A$1:$C$3,3)-6,\"p\",\"q\",\"r\")",
        "=N(INDIRECT($A$5:$C$5))",
    ];
    for (row, formula) in (60..).zip(formulas) {
        engine
            .set_cell_formula("Sheet1", row, 5, parse(formula).unwrap())
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    for row in 60..64 {
        engine.declare_array_formula("Sheet1", row, 5, 1, 3, false);
    }
    engine.evaluate_all().unwrap();

    for (col, row_value, offset_value, choice) in [
        (5, 10.0, 11.0, "p"),
        (6, 20.0, 21.0, "q"),
        (7, 30.0, 31.0, "r"),
    ] {
        assert_number(&engine, "Sheet1", 60, col, row_value);
        assert_number(&engine, "Sheet1", 61, col, offset_value);
        assert_text(&engine, 62, col, choice);
        assert_number(&engine, "Sheet1", 63, col, row_value);
    }
}

#[test]
fn single_value_arguments_lift_over_a_row_with_dynamic_arrays() {
    let mut engine = single_value_argument_engine();
    let formulas = [
        (60, "=INDEX($M$1:$M$40,INDEX($A$1:$C$3,2))"),
        (62, "=N(OFFSET($M$1,INDEX($A$1:$C$3,2),0))"),
        (64, "=CHOOSE(INDEX($A$1:$C$3,3)-6,\"p\",\"q\",\"r\")"),
        (66, "=N(INDIRECT(\"M\"&INDEX($A$1:$C$3,2)))"),
        (68, "=SUM(INDEX($M$1:$M$40,INDEX($A$1:$C$3,2)))"),
        // Several indexes select element-wise, each choice broadcast.
        (70, "=CHOOSE({1,2},$A$1:$A$3,$B$1:$B$3)"),
        (74, "=VLOOKUP(20,CHOOSE({1,2},$B$1:$B$3,$A$1:$A$3),2,FALSE)"),
        // An array of references has no value of its own.
        (76, "=OFFSET($M$1,INDEX($A$1:$C$3,2),0)"),
        (78, "=INDIRECT(\"M\"&INDEX($A$1:$C$3,2))"),
    ];
    for (row, formula) in formulas {
        engine
            .set_cell_formula("Sheet1", row, 5, parse(formula).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();

    for (col, row_value, offset_value, choice) in [
        (5, 10.0, 11.0, "p"),
        (6, 20.0, 21.0, "q"),
        (7, 30.0, 31.0, "r"),
    ] {
        assert_number(&engine, "Sheet1", 60, col, row_value);
        assert_number(&engine, "Sheet1", 62, col, offset_value);
        assert_text(&engine, 64, col, choice);
        assert_number(&engine, "Sheet1", 66, col, row_value);
    }
    assert_number(&engine, "Sheet1", 68, 5, 60.0);
    for (row, first, second) in [(70, 1.0, 2.0), (71, 10.0, 20.0), (72, 7.0, 8.0)] {
        assert_number(&engine, "Sheet1", row, 5, first);
        assert_number(&engine, "Sheet1", row, 6, second);
    }
    assert_number(&engine, "Sheet1", 74, 5, 10.0);
    assert_value_error(&engine, 76, 5);
    assert_value_error(&engine, 78, 5);
}

#[test]
fn static_index_self_loop_classification_reads_area_num() {
    // An omitted area_num is area 1, so INDEX(r,i,j,1) selects like
    // INDEX(r,i,j): selecting another cell of a range that contains the
    // formula is not circular, selecting the formula's own cell is. Any other
    // area is an error that never reads the range, so it is not circular.
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        engine
            .set_cell_value("Sheet1", 1, 2, LiteralValue::Int(42))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 100, 2, parse("=INDEX(B1:B100,1,1,1)").unwrap())
            .unwrap();
        for (row, formula) in [
            (2, "=INDEX(2:2,1,5,1)"),
            (3, "=INDEX(3:3,1,5,)"),
            (4, "=INDEX(4:4,1,5,1.9)"),
            (5, "=SUM(INDEX(5:5,0,5,1))"),
            (6, "=INDEX(6:6,1,1,1)"),
            (7, "=INDEX(7:7,1,1,2)"),
            (8, "=INDEX(8:8,1,1,0)"),
            (9, "=INDEX(9:9,1,1,-1)"),
        ] {
            engine
                .set_cell_value("Sheet1", row, 5, LiteralValue::Int(7))
                .unwrap();
            engine
                .set_cell_formula("Sheet1", row, 1, parse(formula).unwrap())
                .unwrap();
        }

        engine.evaluate_all().unwrap();
        assert_number(&engine, "Sheet1", 100, 2, 42.0);
        for row in 2..=5 {
            assert_number(&engine, "Sheet1", row, 1, 7.0);
        }
        for (row, kind) in [
            (6, ExcelErrorKind::Circ),
            (7, ExcelErrorKind::Ref),
            (8, ExcelErrorKind::Value),
            (9, ExcelErrorKind::Value),
        ] {
            match engine.get_cell_value("Sheet1", row, 1) {
                Some(LiteralValue::Error(error)) => {
                    assert_eq!(error.kind, kind, "{mode:?} row {row}")
                }
                other => panic!("{mode:?} Sheet1!R{row}C1: expected {kind:?}, got {other:?}"),
            }
        }
    }
}

#[test]
fn index_area_num_reads_blank_and_numeric_text_cells() {
    // A blank area_num cell is area 0 (#VALUE!); a cell holding the text "1"
    // converts to area 1 and "2" to area 2 (#REF!).
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        engine
            .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(20))
            .unwrap();
        engine
            .set_cell_value("Sheet1", 2, 3, LiteralValue::Text("1".into()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", 3, 3, LiteralValue::Text("2".into()))
            .unwrap();
        for (row, formula) in [
            (1, "=INDEX(A1:B3,2,2,C1)"),
            (2, "=INDEX(A1:B3,2,2,C2)"),
            (3, "=INDEX(A1:B3,2,2,C3)"),
            (4, "=SUM(INDEX(A1:B3,0,2,C2))"),
        ] {
            engine
                .set_cell_formula("Sheet1", row, 5, parse(formula).unwrap())
                .unwrap();
        }

        engine.evaluate_all().unwrap();
        assert_number(&engine, "Sheet1", 2, 5, 20.0);
        assert_number(&engine, "Sheet1", 4, 5, 20.0);
        for (row, kind) in [(1, ExcelErrorKind::Value), (3, ExcelErrorKind::Ref)] {
            match engine.get_cell_value("Sheet1", row, 5) {
                Some(LiteralValue::Error(error)) => {
                    assert_eq!(error.kind, kind, "{mode:?} row {row}")
                }
                other => panic!("{mode:?} Sheet1!R{row}C5: expected {kind:?}, got {other:?}"),
            }
        }
    }
}

#[test]
fn offset_whole_column_and_row_clamped() {
    let mut engine = new_engine();
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(42))
        .unwrap();
    // OFFSET(B:B,1,0,1,1) -> B2
    engine
        .set_cell_formula("Sheet1", 1, 4, parse("=OFFSET(B:B,1,0,1,1)").unwrap())
        .unwrap();
    // OFFSET(2:2,0,1,1,1) -> B2
    engine
        .set_cell_formula("Sheet1", 5, 4, parse("=OFFSET(2:2,0,1,1,1)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 4, 42.0);
    assert_number(&engine, "Sheet1", 5, 4, 42.0);
}

#[test]
fn offset_whole_column_default_size_sums_used_region() {
    let mut engine = new_engine();
    engine
        .set_cell_value("Sheet1", 1, 2, LiteralValue::Int(1))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 2, LiteralValue::Int(41))
        .unwrap();
    // Height defaults to the clamped used height of B:B (rows 1..2), shifted
    // one column right onto C. C1:C2 holds 2 and 40.
    engine
        .set_cell_value("Sheet1", 1, 3, LiteralValue::Int(2))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 3, LiteralValue::Int(40))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 5, parse("=SUM(OFFSET(B:B,0,1))").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_number(&engine, "Sheet1", 1, 5, 42.0);
}
