//! Test for SUMIFS with whole column references that have different used regions

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

#[test]
fn sumifs_whole_columns_different_used_regions() {
    // This test verifies that SUMIFS works correctly when whole column references
    // have different amounts of data (different used regions), which was causing
    // "range dims mismatch" errors before the padding fix.

    let wb = TestWorkbook::new();
    let mut engine = Engine::new(
        wb,
        EvalConfig {
            range_expansion_limit: 100_000, // Allow large ranges
            ..Default::default()
        },
    );

    // Set up data similar to the user's scenario:
    // Column P has data up to row 60256
    // Column K has data up to row 50035
    // This simulates the dimension mismatch issue

    // Add some sample data in column P (col 16) - more rows
    engine
        .set_cell_value("Sheet1", 100, 16, LiteralValue::Number(10.0))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 60256, 16, LiteralValue::Number(20.0))
        .unwrap();

    // Add data in column K (col 11) - fewer rows
    engine
        .set_cell_value(
            "Sheet1",
            100,
            11,
            LiteralValue::Text("Malpractice SC0279".into()),
        )
        .unwrap();
    engine
        .set_cell_value("Sheet1", 50035, 11, LiteralValue::Text("Other".into()))
        .unwrap();

    // Add data in column AV (col 48) - some other amount
    engine
        .set_cell_value("Sheet1", 100, 48, LiteralValue::Text("MatchValue".into()))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 55000, 48, LiteralValue::Text("SomeValue".into()))
        .unwrap();

    // Add data in column R (col 18) - dates
    engine
        .set_cell_value("Sheet1", 100, 18, LiteralValue::Number(44562.0))
        .unwrap(); // Some date serial

    // Create a SUMIFS formula similar to the user's case
    // =SUMIFS(P:P, K:K, "Malpractice SC0279", AV:AV, "MatchValue")
    let formula =
        parse("=SUMIFS(P:P, K:K, \"Malpractice SC0279\", AV:AV, \"MatchValue\")").unwrap();

    engine.set_cell_formula("Sheet1", 1, 1, formula).unwrap();

    // This should not error with "range dims mismatch" anymore
    let result = engine.evaluate_cell("Sheet1", 1, 1);

    // Should succeed without dimension mismatch error
    assert!(
        result.is_ok(),
        "SUMIFS with different column lengths should not error"
    );

    // The result should be 10.0 (only row 100 matches both criteria)
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 1).unwrap(),
        LiteralValue::Number(10.0)
    );
}

#[test]
fn sumifs_whole_columns_empty_vs_populated() {
    // Test edge case where one column is completely empty
    let config = EvalConfig::default();
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, config);

    // Column A has data
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Number(100.0))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 1000, 1, LiteralValue::Number(200.0))
        .unwrap();

    // Column B has criteria values
    engine
        .set_cell_value("Sheet1", 1, 2, LiteralValue::Text("Yes".into()))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 1000, 2, LiteralValue::Text("Yes".into()))
        .unwrap();

    // Column C is empty (but referenced in formula)
    // Column D has criteria for column C

    // SUMIFS with empty column reference should still work. Placed in column E
    // so the whole-column references (A:A/B:B/C:C) are not self-inclusive — a
    // SUMIFS *in* one of those columns would be circular per #120.
    let formula = parse("=SUMIFS(A:A, B:B, \"Yes\", C:C, \"\")").unwrap();

    engine.set_cell_formula("Sheet1", 2, 5, formula).unwrap();

    let result = engine.evaluate_cell("Sheet1", 2, 5);
    assert!(result.is_ok(), "SUMIFS with empty column should not error");

    // Result should be 300 (both rows match "Yes" and empty matches empty)
    assert_eq!(
        engine.get_cell_value("Sheet1", 2, 5).unwrap(),
        LiteralValue::Number(300.0)
    );
}

#[test]
fn ifs_ranges_of_different_shapes_are_value_error() {
    // Excel: every range of SUMIFS/COUNTIFS/AVERAGEIFS must have the same rows and
    // columns, whether or not any row matches. A whole column is 1,048,576 rows tall
    // however much of it is used; SUMIF still resizes its sum range.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, tag, amount) in [(1, "A", 10.0), (2, "C", 20.0), (3, "B", 30.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Text(tag.into()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(amount))
            .unwrap();
    }
    for col in 6..=7 {
        engine
            .set_cell_value("Sheet1", 10, col, LiteralValue::Text("A".into()))
            .unwrap();
    }
    let eval = |engine: &mut Engine<TestWorkbook>, formula: &str| {
        engine
            .set_cell_formula("Sheet1", 1, 10, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_cell("Sheet1", 1, 10).unwrap();
        engine.get_cell_value("Sheet1", 1, 10).unwrap()
    };
    for formula in [
        "=SUMIFS(B1:B3,A1:A3,\"A\",F10:G10,\"A\")",
        "=SUMIFS(B1:B3,A1:A3,\"Z\",F10:G10,\"A\")",
        "=SUMIFS(B1:B3,A1:A2,\"A\")",
        "=SUMIFS(B1:B3,F10,\"A\")",
        "=SUMIFS(B1:B3,A:A,\"A\")",
        "=COUNTIFS(A1:A3,\"A\",F10:G10,\"A\")",
        "=COUNTIFS(A1:A3,\"A\",B1:C3,\">0\")",
        "=COUNTIFS(A:A,\"A\",B1:B3,\">0\")",
        "=COUNTIFS(A1:A3,\"<>0\",A1:A4,\"<>0\")",
        "=AVERAGEIFS(B1:B3,A1:A3,\"A\",F10:G10,\"A\")",
    ] {
        match eval(&mut engine, formula) {
            LiteralValue::Error(e) => {
                assert_eq!(
                    e.kind,
                    formualizer_common::ExcelErrorKind::Value,
                    "{formula}"
                )
            }
            other => panic!("{formula}: expected #VALUE!, got {other:?}"),
        }
    }
    for (formula, expected) in [
        ("=SUMIFS(B1:B3,A1:A3,\"A\")", 10.0),
        ("=SUMIFS(B1:B3,A2:A4,\"C\",B1:B3,\">0\")", 10.0),
        ("=SUMIFS(B:B,A:A,\"B\")", 30.0),
        ("=COUNTIFS(F10:G10,\"A\",F10:G10,\"A\")", 2.0),
        // COUNTIFS counts the blank rows past the stored ones in ranges of the
        // same shape, also whole columns used to different heights.
        ("=COUNTIFS(A1:A4,\"<>A\",B1:B4,\"<>0\")", 3.0),
        ("=COUNTIFS(A:A,\"<>A\",F:F,\"<>0\")", 1_048_575.0),
        ("=SUMIF(A1:A3,\"A\",F10:G10)", 0.0),
        ("=SUMIF(A1:A3,\"B\",B1)", 30.0),
    ] {
        assert_eq!(
            eval(&mut engine, formula),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}
