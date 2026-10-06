//! A defined name evaluates for the formula that uses it, as Excel evaluates
//! it: relative R1C1 text and ROW() in the name read the calling cell, and a
//! random call in the name is one more draw of the calling formula, the same
//! on every run with the same seed. INDIRECT text that names a workbook is
//! recorded for the host.

use crate::engine::named_range::{NameScope, NamedDefinition};
use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn engine(seed: u64) -> Engine<TestWorkbook> {
    let config = EvalConfig {
        workbook_seed: seed,
        ..EvalConfig::default()
    };
    Engine::new(TestWorkbook::new(), config)
}

fn define(engine: &mut Engine<TestWorkbook>, name: &str, formula: &str) {
    engine
        .define_name(
            name,
            NamedDefinition::Formula {
                ast: parse(formula).unwrap(),
                dependencies: Vec::new(),
                range_deps: Vec::new(),
            },
            NameScope::Workbook,
        )
        .unwrap();
}

fn formula(engine: &mut Engine<TestWorkbook>, sheet: &str, row: u32, col: u32, text: &str) {
    engine
        .set_cell_formula(sheet, row, col, parse(text).unwrap())
        .unwrap_or_else(|e| panic!("{text}: {e:?}"));
}

fn value(engine: &Engine<TestWorkbook>, sheet: &str, row: u32, col: u32) -> LiteralValue {
    engine
        .get_cell_value(sheet, row, col)
        .unwrap_or(LiteralValue::Empty)
}

fn number(engine: &Engine<TestWorkbook>, sheet: &str, row: u32, col: u32) -> f64 {
    match value(engine, sheet, row, col) {
        LiteralValue::Number(n) => n,
        other => panic!("{sheet}!R{row}C{col}: expected a number, got {other:?}"),
    }
}

#[test]
fn relative_r1c1_text_in_a_name_reads_the_calling_cell() {
    // Prev = INDIRECT("RC[-1]",FALSE): in B2 it is A2 (review of PR #1295:
    // the name was read from A1, whose previous column wraps to XFD1).
    let mut engine = engine(1);
    define(&mut engine, "Prev", "=INDIRECT(\"RC[-1]\",FALSE)");
    define(&mut engine, "Here", "=ROW()");
    engine
        .set_cell_value("Sheet1", 2, 1, LiteralValue::Number(10.0))
        .unwrap();
    formula(&mut engine, "Sheet1", 2, 2, "=Prev");
    formula(&mut engine, "Sheet1", 3, 3, "=ROW(Prev)");
    formula(&mut engine, "Sheet1", 2, 4, "=COLUMN(Prev)");
    // The cell the name reads is a formula evaluated first.
    formula(&mut engine, "Sheet1", 5, 1, "=2*21");
    formula(&mut engine, "Sheet1", 5, 2, "=Prev+0");
    formula(&mut engine, "Sheet1", 7, 5, "=Here");
    engine.evaluate_all().unwrap();
    assert_eq!(value(&engine, "Sheet1", 2, 2), LiteralValue::Number(10.0));
    assert_eq!(value(&engine, "Sheet1", 3, 3), LiteralValue::Number(3.0));
    assert_eq!(value(&engine, "Sheet1", 2, 4), LiteralValue::Number(3.0));
    assert_eq!(value(&engine, "Sheet1", 5, 2), LiteralValue::Number(42.0));
    assert_eq!(value(&engine, "Sheet1", 7, 5), LiteralValue::Number(7.0));
}

#[test]
fn resolving_a_name_does_not_restart_the_cells_draws() {
    // RAND()+ROW(Anchor)-RAND() was exactly 1 for every seed: resolving Anchor
    // restarted A1's draws, so the second RAND repeated the first. It draws
    // what RAND()+ROW($C$1)-RAND() draws in the same cell.
    for seed in [7, 8, 9] {
        let mut named = engine(seed);
        define(&mut named, "Anchor", "=OFFSET(Sheet1!$C$1,0,0)");
        formula(&mut named, "Sheet1", 1, 1, "=RAND()+ROW(Anchor)-RAND()");
        named.evaluate_all().unwrap();
        let mut plain = engine(seed);
        formula(
            &mut plain,
            "Sheet1",
            1,
            1,
            "=RAND()+ROW(Sheet1!$C$1)-RAND()",
        );
        plain.evaluate_all().unwrap();
        let value = number(&named, "Sheet1", 1, 1);
        assert_ne!(value, 1.0, "seed {seed}");
        assert_eq!(value, number(&plain, "Sheet1", 1, 1), "seed {seed}");
    }
}

#[test]
fn a_random_name_draws_for_its_caller_the_same_on_every_run() {
    // RandomDraw = RAND() read by many cells gave different caches for the
    // same seed: every reading drew from A1's shared count. Each reading is
    // its caller's next draw, so B<n> = RandomDraw is B<n> = RAND(), and two
    // readings in one formula differ.
    const ROWS: u32 = 60;
    let run = |named: bool| {
        let mut engine = engine(11);
        define(&mut engine, "RandomDraw", "=RAND()");
        for row in 1..=ROWS {
            formula(&mut engine, "Sheet1", row, 1, "=RAND()+RAND()");
            if named {
                formula(&mut engine, "Sheet1", row, 2, "=RandomDraw");
                formula(&mut engine, "Sheet1", row, 3, "=RandomDraw-RandomDraw");
            } else {
                formula(&mut engine, "Sheet1", row, 2, "=RAND()");
                formula(&mut engine, "Sheet1", row, 3, "=RAND()-RAND()");
            }
        }
        engine.evaluate_all().unwrap();
        (1..=ROWS)
            .flat_map(|row| (1..=3).map(move |col| (row, col)))
            .map(|(row, col)| number(&engine, "Sheet1", row, col))
            .collect::<Vec<_>>()
    };
    let first = run(true);
    for _ in 0..4 {
        assert_eq!(run(true), first);
    }
    assert_eq!(run(false), first);
    assert!(first.chunks(3).all(|row| row[2] != 0.0));
}

#[test]
fn indirect_text_naming_a_workbook_is_recorded() {
    // Excel reads '[self-bookref.xlsx]Input'!A1 from this workbook when that
    // is its name (42), from another open one otherwise; the engine gives the
    // closed workbook's #REF! and records the read for the host.
    let reads_a_workbook = |text: &str| {
        let mut engine = engine(1);
        engine.add_sheet("Input").unwrap();
        engine.add_sheet("Data.xlsx").unwrap();
        engine
            .set_cell_value("Input", 1, 1, LiteralValue::Number(42.0))
            .unwrap();
        engine
            .set_cell_value("Data.xlsx", 1, 1, LiteralValue::Number(7.0))
            .unwrap();
        engine
            .set_cell_value(
                "Sheet1",
                1,
                3,
                LiteralValue::Text("'[self-bookref.xlsx]Input'!A1".into()),
            )
            .unwrap();
        formula(&mut engine, "Sheet1", 1, 1, text);
        engine.evaluate_all().unwrap();
        (engine.text_named_workbook(), value(&engine, "Sheet1", 1, 1))
    };
    let reference_error = LiteralValue::Error(ExcelErrorKind::Ref.into());
    for text in [
        "=INDIRECT(\"'[self-bookref.xlsx]Input'!A1\")",
        "=INDIRECT(\"[self-bookref.xlsx]Input!A1\")",
        "=INDIRECT(\"'C:\\Data\\[self-bookref.xlsx]Input'!A1\")",
        "=INDIRECT(\"'[self-bookref.xlsx]Input'!R1C1\",FALSE)",
        "=INDIRECT(\"self-bookref.xlsx!Total\")",
        "=INDIRECT(C1)",
    ] {
        assert_eq!(
            reads_a_workbook(text),
            (true, reference_error.clone()),
            "{text}"
        );
    }
    // Behind IFERROR the value is 0, and the read is still recorded.
    assert_eq!(
        reads_a_workbook("=IFERROR(INDIRECT(\"[self-bookref.xlsx]Input!\"&\"A1\"),0)"),
        (true, LiteralValue::Number(0.0))
    );
    for (text, expected) in [
        ("=INDIRECT(\"Input!A1\")", LiteralValue::Number(42.0)),
        (
            "=INDIRECT(\"'Input'!R1C1\",FALSE)",
            LiteralValue::Number(42.0),
        ),
        // A sheet named like a workbook is that sheet.
        ("=INDIRECT(\"'Data.xlsx'!A1\")", LiteralValue::Number(7.0)),
        // A table's column may hold `[` and `!`.
        (
            "=ISREF(INDIRECT(\"Table1[a!b]\"))",
            LiteralValue::Boolean(false),
        ),
        (
            "=ISREF(INDIRECT(\"Missing!A1\"))",
            LiteralValue::Boolean(false),
        ),
    ] {
        assert_eq!(reads_a_workbook(text), (false, expected), "{text}");
    }
}
