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
