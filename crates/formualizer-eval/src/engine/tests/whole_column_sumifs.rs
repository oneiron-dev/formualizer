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

#[test]
fn ifs_whole_axis_ranges_from_let_lambda_and_xlookup_keep_full_size() {
    // Excel: a LET name or LAMBDA parameter bound to a reference stays that
    // reference, and XLOOKUP returns a reference into its return range. A whole
    // column reached that way is still 1,048,576 rows (a whole row 16,384
    // columns) however much of it is used, so it matches another whole column
    // and not a bounded range of its used height.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let data = [("A", 10.0), ("C", 20.0), ("B", 30.0)];
    for (sheet, first_row) in [("Sheet1", 1), ("Sheet3", 2), ("Sheet4", 1)] {
        for (offset, (tag, amount)) in data.iter().enumerate() {
            let row = first_row + offset as u32;
            engine
                .set_cell_value(sheet, row, 1, LiteralValue::Text((*tag).into()))
                .unwrap();
            engine
                .set_cell_value(sheet, row, 2, LiteralValue::Number(*amount))
                .unwrap();
        }
    }
    for (col, header) in [(1, "Cat"), (2, "Amt")] {
        engine
            .set_cell_value("Sheet3", 1, col, LiteralValue::Text(header.into()))
            .unwrap();
    }
    let eval = |engine: &mut Engine<TestWorkbook>, formula: &str| {
        engine
            .set_cell_formula("Sheet1", 1, 10, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_cell("Sheet1", 1, 10).unwrap();
        engine.get_cell_value("Sheet1", 1, 10).unwrap()
    };
    for (formula, expected) in [
        ("=LET(col,A:A,SUMIFS(B:B,col,\"A\"))", 10.0),
        ("=LET(col,A:A,AVERAGEIFS(B:B,col,\"A\"))", 10.0),
        ("=LET(col,A:A,COUNTIFS(col,\"A\",B:B,\">0\"))", 1.0),
        ("=LET(col,A:A,d,col,SUMIFS(B:B,d,\"B\"))", 30.0),
        ("=LAMBDA(area,SUMIFS(B:B,area,\"A\"))(A:A)", 10.0),
        ("=LET(f,LAMBDA(area,SUMIFS(B:B,area,\"C\")),f(A:A))", 20.0),
        ("=LET(area,Sheet3!2:2,SUMIFS(Sheet3!3:3,area,\"A\"))", 0.0),
        (
            "=SUMIFS(XLOOKUP(\"Amt\",Sheet3!A1:B1,Sheet3!A:B),Sheet3!A:A,\"A\")",
            10.0,
        ),
        (
            "=SUMIFS(Sheet3!B:B,XLOOKUP(\"Cat\",Sheet3!A1:B1,Sheet3!A:B),\"A\")",
            10.0,
        ),
        // Bounded ranges through the same paths keep their own size.
        ("=LET(col,A1:A3,SUMIFS(B1:B3,col,\"B\"))", 30.0),
        ("=LAMBDA(area,SUMIFS(B1:B3,area,\"B\"))(A1:A3)", 30.0),
    ] {
        assert_eq!(
            eval(&mut engine, formula),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
    for formula in [
        // Sheet4 uses only rows 1-3, but its whole column A is not B1:B3's shape.
        "=LET(col,Sheet4!A:A,SUMIFS(Sheet4!B1:B3,col,\"A\"))",
        "=LAMBDA(area,SUMIFS(Sheet4!B1:B3,area,\"A\"))(Sheet4!A:A)",
        // XLOOKUP gives Sheet3!A:A, not the four rows Sheet3 uses.
        "=SUMIFS(Sheet3!B1:B4,XLOOKUP(\"Cat\",Sheet3!A1:B1,Sheet3!A:B),\"A\")",
        "=LET(col,A1:A2,COUNTIFS(col,\"A\",B1:B3,\">0\"))",
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
}

#[test]
fn let_lambda_and_xlookup_results_are_references() {
    // The same rule outside the IFS functions: a bound name or an XLOOKUP
    // result works wherever Excel takes a reference.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, tag, amount) in [(1, "A", 10.0), (2, "C", 20.0), (3, "B", 30.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Text(tag.into()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(amount))
            .unwrap();
    }
    let eval = |engine: &mut Engine<TestWorkbook>, formula: &str| {
        engine
            .set_cell_formula("Sheet1", 1, 10, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_cell("Sheet1", 1, 10).unwrap();
        engine.get_cell_value("Sheet1", 1, 10).unwrap()
    };
    for (formula, expected) in [
        ("=LET(col,A:A,ROWS(col))", LiteralValue::Number(1_048_576.0)),
        ("=LAMBDA(area,ROWS(area))(2:2)", LiteralValue::Number(1.0)),
        (
            "=LAMBDA(area,COLUMNS(area))(2:2)",
            LiteralValue::Number(16_384.0),
        ),
        ("=LET(col,B2,ISREF(col))", LiteralValue::Boolean(true)),
        ("=LET(col,B2,ROW(col))", LiteralValue::Number(2.0)),
        ("=LAMBDA(area,ROW(area))(B3)", LiteralValue::Number(3.0)),
        (
            "=LET(col,B1:B3,SUM(OFFSET(col,1,0,2)))",
            LiteralValue::Number(50.0),
        ),
        (
            "=LET(col,A1:A3,SUMIF(col,\"B\",B1:B3))",
            LiteralValue::Number(30.0),
        ),
        ("=LET(x,5,x*2)", LiteralValue::Number(10.0)),
        (
            "=ISREF(XLOOKUP(\"C\",A1:A3,B1:B3))",
            LiteralValue::Boolean(true),
        ),
        (
            "=ROW(XLOOKUP(\"B\",A1:A3,B1:B3))",
            LiteralValue::Number(3.0),
        ),
        // Microsoft's XLOOKUP example builds a range from two XLOOKUPs.
        (
            "=SUM(XLOOKUP(\"C\",A1:A3,B1:B3):XLOOKUP(\"B\",A1:A3,B1:B3))",
            LiteralValue::Number(50.0),
        ),
        ("=XLOOKUP(\"C\",A1:A3,B1:B3)", LiteralValue::Number(20.0)),
        (
            "=XLOOKUP(\"Z\",A1:A3,B1:B3,\"none\")",
            LiteralValue::Text("none".into()),
        ),
    ] {
        let got = eval(&mut engine, formula);
        let number = |value: &LiteralValue| match value {
            LiteralValue::Int(i) => Some(*i as f64),
            LiteralValue::Number(n) => Some(*n),
            _ => None,
        };
        match (number(&got), number(&expected)) {
            (Some(got), Some(expected)) => assert_eq!(got, expected, "{formula}"),
            _ => assert_eq!(got, expected, "{formula}"),
        }
    }
}
