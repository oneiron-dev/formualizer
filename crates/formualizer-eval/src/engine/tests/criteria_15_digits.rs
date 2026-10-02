//! Criteria compare numbers to 15 significant digits, for every operator,
//! as Excel's comparison operators do: a cell holding 8:30
//! (0.35416666666666669) meets `">="&A1` with A1 8:30, whose text makes the
//! criterion `">=0.354166666666667"` (0.35416666666666702). The same rule holds
//! in COUNTIF(S), SUMIF(S), AVERAGEIF(S), MAXIFS, MINIFS and the D functions,
//! in the cached criteria masks over numeric lanes and in the scalar matcher.

use super::common::arrow_eval_config;
use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

fn eval(engine: &mut Engine<TestWorkbook>, formula: &str) -> LiteralValue {
    engine
        .set_cell_formula("Sheet1", 1, 20, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_cell("Sheet1", 1, 20).unwrap();
    engine.get_cell_value("Sheet1", 1, 20).unwrap()
}

/// A1 = 8:30 (8.5/24), A2 = 0.1+0.2, A3 = 1.00000000000001, A4 = 5 with
/// B1..B4 = 1, 2, 4, 8, plus A5 = "x", B5 = 16 when `text` (a text cell sends
/// numeric equality to the scalar matcher). `bulk` stores the data in base
/// lanes over two chunks; otherwise it lands in overlays.
fn engine(bulk: bool, text: bool) -> Engine<TestWorkbook> {
    let mut a = vec![
        LiteralValue::Number(8.5 / 24.0),
        LiteralValue::Number(0.1 + 0.2),
        LiteralValue::Number(1.00000000000001),
        LiteralValue::Number(5.0),
    ];
    let mut b = vec![1.0, 2.0, 4.0, 8.0];
    if text {
        a.push(LiteralValue::Text("x".into()));
        b.push(16.0);
    }
    if bulk {
        let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
        let mut ab = engine.begin_bulk_ingest_arrow();
        ab.add_sheet("Sheet1", 2, 2);
        for (a, b) in a.iter().zip(b) {
            ab.append_row("Sheet1", &[a.clone(), LiteralValue::Number(b)])
                .unwrap();
        }
        ab.finish().unwrap();
        engine
    } else {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        for (i, (a, b)) in a.iter().zip(b).enumerate() {
            let row = i as u32 + 1;
            engine.set_cell_value("Sheet1", row, 1, a.clone()).unwrap();
            engine
                .set_cell_value("Sheet1", row, 2, LiteralValue::Number(b))
                .unwrap();
        }
        engine
    }
}

#[test]
fn numeric_criteria_compare_to_15_digits_with_every_operator() {
    for text in [false, true] {
        for bulk in [false, true] {
            let mut engine = engine(bulk, text);
            // The text row (A5="x", B5=16) meets only `<>n`.
            let (t, ts) = if text { (1.0, 16.0) } else { (0.0, 0.0) };
            let r = if text { "A1:A5" } else { "A1:A4" };
            let s = if text { "B1:B5" } else { "B1:B4" };
            let cases = [
                // A1 is 8:30 to 15 digits: = and <> see it, >= takes it and <
                // leaves it, as > and <= already did.
                (format!("=COUNTIF({r}, \"=0.354166666666667\")"), 1.0),
                (format!("=COUNTIF({r}, 0.354166666666667)"), 1.0),
                (format!("=COUNTIF({r}, \"0.354166666666667\")"), 1.0),
                (format!("=COUNTIF({r}, \"<>0.354166666666667\")"), 3.0 + t),
                (format!("=COUNTIF({r}, \">=0.354166666666667\")"), 3.0),
                (format!("=COUNTIF({r}, \"<0.354166666666667\")"), 1.0),
                (format!("=COUNTIF({r}, \">0.354166666666667\")"), 2.0),
                (format!("=COUNTIF({r}, \"<=0.354166666666667\")"), 2.0),
                // 0.1+0.2 is 0.3.
                (format!("=COUNTIF({r}, \"=0.3\")"), 1.0),
                (format!("=COUNTIF({r}, \">0.3\")"), 3.0),
                (format!("=COUNTIF({r}, \"<=0.3\")"), 1.0),
                (format!("=COUNTIF({r}, \"<>0.3\")"), 3.0 + t),
                // 1.00000000000001 differs from 1 in the 15th digit.
                (format!("=COUNTIF({r}, 1)"), 0.0),
                (format!("=COUNTIF({r}, \">1\")"), 2.0),
                (format!("=COUNTIF({r}, \"<=1\")"), 2.0),
                (format!("=COUNTIF({r}, \"=1.00000000000001\")"), 1.0),
                (
                    format!("=COUNTIFS({r}, \">=0.354166666666667\", {r}, \"<0.354166666666667\")"),
                    0.0,
                ),
                (format!("=SUMIF({r}, \">=0.354166666666667\", {s})"), 13.0),
                (
                    format!("=SUMIF({r}, \"<>0.354166666666667\", {s})"),
                    14.0 + ts,
                ),
                (
                    format!("=SUMIFS({s}, {r}, \">=0.354166666666667\", {r}, \"<1\")"),
                    1.0,
                ),
                (
                    format!("=AVERAGEIF({r}, \"<=0.354166666666667\", {s})"),
                    1.5,
                ),
                (format!("=AVERAGEIFS({s}, {r}, \"=0.3\")"), 2.0),
                (format!("=MAXIFS({s}, {r}, \"<=0.354166666666667\")"), 2.0),
                (format!("=MINIFS({s}, {r}, \">=0.354166666666667\")"), 1.0),
                (format!("=MINIFS({s}, {r}, \"<0.354166666666667\")"), 2.0),
            ];
            for (formula, expected) in cases {
                assert_eq!(
                    eval(&mut engine, &formula),
                    LiteralValue::Number(expected),
                    "bulk={bulk} text={text}: {formula}"
                );
            }
        }
    }
}

#[test]
fn criteria_built_from_a_number_meet_that_number() {
    // `">="&C1` writes C1 as Excel's text, 15 significant digits: 8:30 reads
    // ">=0.354166666666667" and 0.1+0.2 reads "=0.3". The cell holding the
    // same number still meets the criterion, for every operator.
    for bulk in [false, true] {
        let mut engine = engine(bulk, false);
        for (row, formula) in [(1, "=TIME(8,30,0)"), (2, "=0.1+0.2"), (3, "=8.5/24")] {
            engine
                .set_cell_formula("Sheet1", row, 3, parse(formula).unwrap())
                .unwrap();
        }
        let cases = [
            ("=COUNTIF(A1:A4, \">=\"&C1)", 3.0),
            ("=COUNTIF(A1:A4, \"<\"&C1)", 1.0),
            ("=COUNTIF(A1:A4, \"=\"&C1)", 1.0),
            ("=COUNTIF(A1:A4, \"<>\"&C1)", 3.0),
            ("=COUNTIF(A1:A4, \"<=\"&C3)", 2.0),
            ("=COUNTIF(A1:A4, \">\"&C3)", 2.0),
            ("=COUNTIFS(A1:A4, \">=\"&C3, A1:A4, \"<\"&C2*2)", 1.0),
            ("=SUMIFS(B1:B4, A1:A4, \">=\"&C1, A1:A4, \"<1\")", 1.0),
            ("=SUMIF(A1:A4, \"=\"&C2, B1:B4)", 2.0),
            ("=COUNTIF(A1:A4, C2&\"\")", 1.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                eval(&mut engine, formula),
                LiteralValue::Number(expected),
                "bulk={bulk}: {formula}"
            );
        }
    }
}

#[test]
fn database_criteria_compare_to_15_digits() {
    // DCOUNT/DSUM read their criteria like COUNTIFS: E1:F5 is the database
    // (t = A's values, v = B's), G1:G2 and H1:H2 criteria on t.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let t = [8.5 / 24.0, 0.1 + 0.2, 1.00000000000001, 5.0];
    let v = [1.0, 2.0, 4.0, 8.0];
    let text = |s: &str| LiteralValue::Text(s.into());
    engine.set_cell_value("Sheet1", 1, 5, text("t")).unwrap();
    engine.set_cell_value("Sheet1", 1, 6, text("v")).unwrap();
    for (i, (t, v)) in t.iter().zip(v).enumerate() {
        let row = i as u32 + 2;
        engine
            .set_cell_value("Sheet1", row, 5, LiteralValue::Number(*t))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 6, LiteralValue::Number(v))
            .unwrap();
    }
    for (col, criterion) in [
        (7, ">=0.354166666666667"),
        (8, "<0.354166666666667"),
        (9, "=0.3"),
    ] {
        engine.set_cell_value("Sheet1", 1, col, text("t")).unwrap();
        engine
            .set_cell_value("Sheet1", 2, col, text(criterion))
            .unwrap();
    }
    let cases = [
        ("=DCOUNT(E1:F5, \"v\", G1:G2)", 3.0),
        ("=DSUM(E1:F5, \"v\", G1:G2)", 13.0),
        ("=DCOUNT(E1:F5, \"v\", H1:H2)", 1.0),
        ("=DSUM(E1:F5, \"v\", I1:I2)", 2.0),
    ];
    for (formula, expected) in cases {
        assert_eq!(
            eval(&mut engine, formula),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}

#[test]
fn negative_zero_criterion_is_zero() {
    // Excel has no negative zero: COUNTIF(H:H,-H12) with H12 = 0 counts the
    // zeros. The numeric lanes compare like the comparison operators, not in
    // Arrow's total order, where -0 sorts below 0.
    let a = [0.0, 5.0, 0.0, 7.0];
    for bulk in [false, true] {
        let mut engine = if bulk {
            let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
            let mut ab = engine.begin_bulk_ingest_arrow();
            ab.add_sheet("Sheet1", 1, 2);
            for a in a {
                ab.append_row("Sheet1", &[LiteralValue::Number(a)]).unwrap();
            }
            ab.finish().unwrap();
            engine
        } else {
            let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
            for (i, a) in a.into_iter().enumerate() {
                engine
                    .set_cell_value("Sheet1", i as u32 + 1, 1, LiteralValue::Number(a))
                    .unwrap();
            }
            engine
        };
        engine
            .set_cell_value("Sheet1", 1, 3, LiteralValue::Number(0.0))
            .unwrap();
        let cases = [
            ("=COUNTIF(A1:A4, -C1)", 2.0),
            ("=COUNTIF(A1:A4, \"<>\"&-C1)", 2.0),
            ("=COUNTIF(A1:A4, \"<=\"&-C1)", 2.0),
            ("=COUNTIF(A1:A4, \"<\"&-C1)", 0.0),
            ("=COUNTIFS(A1:A4, -C1, A1:A4, \">=\"&-C1)", 2.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                eval(&mut engine, formula),
                LiteralValue::Number(expected),
                "bulk={bulk}: {formula}"
            );
        }
    }
}

#[test]
fn numeric_text_cells_keep_15_significant_digits() {
    // Text reads as a number to its first 15 significant digits, the rest
    // zeros, as Excel reads typed numbers: "1000000000000005" is
    // 1000000000000000, not a number that rounds to 1000000000000010, so it
    // meets "<>1000000000000010" and not "=1000000000000010".
    let a = [
        LiteralValue::Text("1000000000000005".into()),
        LiteralValue::Text("1000000000000012".into()),
    ];
    let b = [10.0, 20.0];
    for bulk in [false, true] {
        let mut engine = if bulk {
            let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
            let mut ab = engine.begin_bulk_ingest_arrow();
            ab.add_sheet("Sheet1", 2, 2);
            for (a, b) in a.iter().zip(b) {
                ab.append_row("Sheet1", &[a.clone(), LiteralValue::Number(b)])
                    .unwrap();
            }
            ab.finish().unwrap();
            engine
        } else {
            let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                let row = i as u32 + 1;
                engine.set_cell_value("Sheet1", row, 1, a.clone()).unwrap();
                engine
                    .set_cell_value("Sheet1", row, 2, LiteralValue::Number(b))
                    .unwrap();
            }
            engine
        };
        let cases = [
            ("=COUNTIF(A1:A2, \"<>1000000000000010\")", 1.0),
            ("=SUMIF(A1:A2, \"<>1000000000000010\", B1:B2)", 10.0),
            ("=COUNTIF(A1:A2, \"=1000000000000010\")", 1.0),
            ("=SUMIF(A1:A2, 1000000000000000, B1:B2)", 10.0),
            // The criterion's text keeps 15 digits too.
            ("=SUMIF(A1:A2, \"1000000000000019\", B1:B2)", 20.0),
            ("=VALUE(\"1000000000000005\")", 1000000000000000.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                eval(&mut engine, formula),
                LiteralValue::Number(expected),
                "bulk={bulk}: {formula}"
            );
        }
    }
}

#[test]
fn database_criteria_read_numeric_text_records_to_15_digits() {
    // E1:F2 is the database (n = "1000000000000005" as text, v = 10); the
    // record reads as 1000000000000000, so neither "=1000000000000010" nor
    // ">=1000000000000010" selects it.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let text = |s: &str| LiteralValue::Text(s.into());
    engine.set_cell_value("Sheet1", 1, 5, text("n")).unwrap();
    engine.set_cell_value("Sheet1", 1, 6, text("v")).unwrap();
    engine
        .set_cell_value("Sheet1", 2, 5, text("1000000000000005"))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 6, LiteralValue::Number(10.0))
        .unwrap();
    for (col, criterion) in [
        (7, "=1000000000000010"),
        (8, ">=1000000000000010"),
        (9, "=1000000000000000"),
    ] {
        engine.set_cell_value("Sheet1", 1, col, text("n")).unwrap();
        engine
            .set_cell_value("Sheet1", 2, col, text(criterion))
            .unwrap();
    }
    let cases = [
        ("=DSUM(E1:F2, \"v\", G1:G2)", 0.0),
        ("=DCOUNT(E1:F2, \"v\", H1:H2)", 0.0),
        ("=DSUM(E1:F2, \"v\", I1:I2)", 10.0),
    ];
    for (formula, expected) in cases {
        assert_eq!(
            eval(&mut engine, formula),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}
