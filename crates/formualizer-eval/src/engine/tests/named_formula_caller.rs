//! A defined name evaluates for the formula that uses it, as Excel evaluates
//! it: relative R1C1 text and ROW() in the name read the calling cell, and a
//! random call in the name is one more draw of the calling formula, the same
//! on every run with the same seed. INDIRECT text that names a workbook is
//! recorded for the host.

use crate::engine::named_range::{NameScope, NamedDefinition};
use crate::engine::{CycleConfig, CycleDetection, CyclePolicy, Engine, EvalConfig};
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

/// Excel with iterative calculation off, as the cache writer runs it.
fn retain_engine() -> Engine<TestWorkbook> {
    Engine::new(
        TestWorkbook::new(),
        EvalConfig::default().with_cycle(CycleConfig {
            detection: CycleDetection::Runtime,
            policy: CyclePolicy::RetainLastValue,
        }),
    )
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

#[test]
fn a_name_reading_its_callers_cell_is_a_circular_reference() {
    // Review of oneiron #1295: Loop = INDIRECT("RC",FALSE)+1 in B2 reads B2.
    // The read was no dependency of B2, so B2 was scheduled as acyclic and
    // cached 1 (B2 read as blank). Like =INDIRECT("RC",FALSE)+1 written in
    // B2, it is circular, and with iteration off B2 keeps its last calculated
    // value (17, the file's cache).
    for written in ["=Loop", "=INDIRECT(\"RC\",FALSE)+1"] {
        let mut engine = retain_engine();
        define(&mut engine, "Loop", "=INDIRECT(\"RC\",FALSE)+1");
        formula(&mut engine, "Sheet1", 2, 2, written);
        engine.set_last_calculated_value("Sheet1", 2, 2, LiteralValue::Number(17.0));
        engine.evaluate_all().unwrap();
        assert_eq!(number(&engine, "Sheet1", 2, 2), 17.0, "{written}");
        assert_eq!(
            engine.last_cycle_telemetry().live_cycles_witnessed,
            1,
            "{written}"
        );
    }
}

#[test]
fn a_names_reads_are_its_callers_dependencies() {
    let mut engine = retain_engine();
    define(&mut engine, "Next", "=INDIRECT(\"RC[1]\",FALSE)");
    define(&mut engine, "Loop", "=INDIRECT(\"RC\",FALSE)+1");
    define(&mut engine, "Again", "=Loop*1");
    define(&mut engine, "Fixed", "=INDIRECT(\"Sheet1!B8\")+1");
    define(&mut engine, "Prev", "=INDIRECT(\"RC[-1]\",FALSE)");
    // B4 reads C4 through Next, and C4 reads B4: a cycle through the name.
    formula(&mut engine, "Sheet1", 4, 2, "=Next+1");
    formula(&mut engine, "Sheet1", 4, 3, "=B4*2");
    engine.set_last_calculated_value("Sheet1", 4, 2, LiteralValue::Number(3.0));
    engine.set_last_calculated_value("Sheet1", 4, 3, LiteralValue::Number(6.0));
    // Through a name that uses the name.
    formula(&mut engine, "Sheet1", 6, 2, "=Again");
    engine.set_last_calculated_value("Sheet1", 6, 2, LiteralValue::Number(5.0));
    // A name whose INDIRECT text is fixed reads its target for every caller.
    formula(&mut engine, "Sheet1", 8, 2, "=Fixed");
    engine.set_last_calculated_value("Sheet1", 8, 2, LiteralValue::Number(9.0));
    // In a branch IF does not take, the name is not read: no circularity.
    formula(&mut engine, "Sheet1", 9, 2, "=IF(TRUE,5,Loop)");
    // B10 reads A10 through Prev; A10 is a formula over C12, both entered
    // after B10, so only the read through the name orders them.
    formula(&mut engine, "Sheet1", 10, 2, "=Prev+1");
    formula(&mut engine, "Sheet1", 10, 1, "=C12*2");
    engine
        .set_cell_value("Sheet1", 12, 3, LiteralValue::Number(21.0))
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(number(&engine, "Sheet1", 4, 2), 3.0);
    assert_eq!(number(&engine, "Sheet1", 4, 3), 6.0);
    assert_eq!(number(&engine, "Sheet1", 6, 2), 5.0);
    assert_eq!(number(&engine, "Sheet1", 8, 2), 9.0);
    assert_eq!(number(&engine, "Sheet1", 9, 2), 5.0);
    assert_eq!(number(&engine, "Sheet1", 10, 1), 42.0);
    assert_eq!(number(&engine, "Sheet1", 10, 2), 43.0);
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 3);
}

#[test]
fn a_name_resolved_as_a_reference_draws_once() {
    // Review of oneiron #1295: Pick = OFFSET(Sheet1!$C$1,RANDBETWEEN(0,1),0)
    // in B1 = Pick+0 was resolved as a reference (drawing 0.508..., row
    // offset 1), discarded because it is one cell, and evaluated again
    // (drawing 0.114..., offset 0): 10 where the inline formula gives 20 for
    // seed 7. A name evaluates once where it is used, drawing what the same
    // formula written there draws.
    let run = |seed: u64, written: &str| {
        let mut engine = engine(seed);
        define(
            &mut engine,
            "Pick",
            "=OFFSET(Sheet1!$C$1,RANDBETWEEN(0,1),0)",
        );
        engine
            .set_cell_value("Sheet1", 1, 3, LiteralValue::Number(10.0))
            .unwrap();
        engine
            .set_cell_value("Sheet1", 2, 3, LiteralValue::Number(20.0))
            .unwrap();
        formula(&mut engine, "Sheet1", 1, 2, written);
        // A workbook's ordinary formulas are not array formulas.
        engine.use_legacy_array_semantics();
        engine.evaluate_all().unwrap();
        value(&engine, "Sheet1", 1, 2)
    };
    let inline = "OFFSET(Sheet1!$C$1,RANDBETWEEN(0,1),0)";
    assert_eq!(run(7, "=Pick+0"), LiteralValue::Number(20.0));
    assert_eq!(run(7, &format!("={inline}+0")), LiteralValue::Number(20.0));
    for form in [
        "=Pick+0",
        "=-Pick",
        "=ABS(Pick)",
        "=Pick&\"\"",
        "=N(Pick)",
        "=SUM(Pick)",
        "=Pick+Pick",
        "=ROW(Pick)+RAND()",
        "=Pick*RAND()",
        "=INDEX(Pick,1)+RAND()",
        "=IF(TRUE,Pick)+RAND()",
    ] {
        let inline_form = form.replace("Pick", inline);
        for seed in 1..=12 {
            assert_eq!(
                run(seed, form),
                run(seed, &inline_form),
                "{form} seed {seed}"
            );
        }
    }
}
