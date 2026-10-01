use crate::engine::{EvalConfig, eval::Engine};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

fn serial_eval_config() -> EvalConfig {
    EvalConfig {
        enable_parallel: false,
        ..Default::default()
    }
}

#[test]
fn spill_exceeds_sheet_bounds() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // Anchor at last allowed column (1-based max 16384); spilling 1x2 exceeds bounds
    engine
        .set_cell_value("Sheet1", 1, 16384, LiteralValue::Int(0))
        .unwrap();
    // Array that would require col 16385 (out of bounds)
    engine
        .set_cell_formula("Sheet1", 1, 16384, parse("={1,2}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();
    match engine.get_cell_value("Sheet1", 1, 16384) {
        Some(LiteralValue::Error(e)) => {
            assert_eq!(e, "#SPILL!");
            if let formualizer_common::ExcelErrorExtra::Spill {
                expected_rows,
                expected_cols,
            } = &e.extra
            {
                assert_eq!((*expected_rows, *expected_cols), (1, 2));
            }
        }
        v => panic!("expected #SPILL!, got {v:?}"),
    }
}

#[test]
fn spill_exceeds_sheet_bounds_rows() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // Anchor at last allowed row (1-based max 1_048_576); spilling 2 rows exceeds bounds
    engine
        .set_cell_value("Sheet1", 1_048_576, 1, LiteralValue::Int(0))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1_048_576, 1, parse("={1;2}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();
    match engine.get_cell_value("Sheet1", 1_048_576, 1) {
        Some(LiteralValue::Error(e)) => assert_eq!(e, "#SPILL!"),
        v => panic!("expected #SPILL!, got {v:?}"),
    }
}

fn assert_spill(value: Option<LiteralValue>, what: &str) {
    match value {
        Some(LiteralValue::Error(e)) => assert_eq!(e, "#SPILL!", "{what}"),
        v => panic!("{what}: expected #SPILL!, got {v:?}"),
    }
}

#[test]
fn whole_column_arrays_spill_past_the_last_row() {
    // A whole column holds all 1,048,576 rows, so an element-wise result over
    // it cannot spill from below row 1, however few of its rows hold data.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    for r in 1..=3u32 {
        for c in [1, 8, 9, 10] {
            engine
                .set_cell_value("Sheet1", r, c, LiteralValue::Int(r as i64))
                .unwrap();
        }
    }
    let formulas = [
        (2, 3, "=A:A*2"),
        (2, 4, "=SUMPRODUCT(--(A:A=1))*H:J"),
        (2, 5, "=A:A"),
        (2, 6, "=-(H:J>1)"),
        (5, 2, "=1:1&\"\""),
        (2, 12, "=SUM(A:A*2)"),
        (2, 13, "=INDEX(A:A,3)"),
        (2, 14, "=A1:A3*2"),
    ];
    for (r, c, f) in formulas {
        engine
            .set_cell_formula("Sheet1", r, c, parse(f).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    for (r, c, f) in &formulas[..5] {
        assert_spill(engine.get_cell_value("Sheet1", *r, *c), f);
    }
    let n = |v: f64| Some(LiteralValue::Number(v));
    // Reductions and selections are not spilled arrays.
    assert_eq!(engine.get_cell_value("Sheet1", 2, 12), n(12.0));
    assert_eq!(engine.get_cell_value("Sheet1", 2, 13), n(3.0));
    // A bounded range spills.
    assert_eq!(engine.get_cell_value("Sheet1", 4, 14), n(6.0));

    // A whole column fits from row 1 and a whole row from column A.
    for ((r, c, f), spilled) in [((1, 3, "=A:A*2"), (2, 3)), ((4, 1, "=1:1*2"), (4, 2))] {
        let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
        for (r, c) in [(1, 1), (2, 1), (1, 2)] {
            engine
                .set_cell_value("Sheet1", r, c, LiteralValue::Int(5))
                .unwrap();
        }
        engine
            .set_cell_formula("Sheet1", r, c, parse(f).unwrap())
            .unwrap();
        engine.evaluate_all().unwrap();
        assert_eq!(
            engine.get_cell_value("Sheet1", spilled.0, spilled.1),
            n(10.0),
            "{f}"
        );
    }
}

#[test]
fn whole_column_arrays_keep_legacy_formula_semantics() {
    // In a workbook file only dynamic arrays spill: an ordinary formula takes
    // the implicit intersection and a CSE array fills its own cells.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    for r in 1..=3u32 {
        engine
            .set_cell_value("Sheet1", r, 1, LiteralValue::Int(r as i64))
            .unwrap();
    }
    for c in 2..=4 {
        engine
            .set_cell_formula("Sheet1", 2, c, parse("=A:A*2").unwrap())
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    engine.declare_array_formula("Sheet1", 2, 3, 2, 1, false);
    engine.declare_array_formula("Sheet1", 2, 4, 1, 1, true);
    engine.evaluate_all().unwrap();
    let n = |v: f64| Some(LiteralValue::Number(v));
    assert!(matches!(
        engine.get_cell_value("Sheet1", 2, 2),
        Some(LiteralValue::Number(_))
    ));
    assert_eq!(engine.get_cell_value("Sheet1", 2, 3), n(2.0));
    assert_eq!(engine.get_cell_value("Sheet1", 3, 3), n(4.0));
    assert_spill(engine.get_cell_value("Sheet1", 2, 4), "dynamic =A:A*2");
}

#[test]
fn spill_values_update_dependents() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // A1 spills 2x2
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={1,2;3,4}").unwrap())
        .unwrap();
    // C1 reads B2 (spilled bottom-right of 2x2)
    engine
        .set_cell_formula("Sheet1", 1, 3, parse("=B2").unwrap())
        .unwrap();
    // Two-pass: first pass materializes spill cells; second pass updates dependents
    let _ = engine.evaluate_all().unwrap();
    // Demand-driven compute of C1 after spill is materialized
    let _ = engine.evaluate_until(&[("Sheet1", 1, 3)]).unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 3),
        Some(LiteralValue::Number(4.0))
    );

    // Change anchor to {5,6;7,8}, B2 becomes 8; C1 should update to 8
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={5,6;7,8}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();
    let _ = engine.evaluate_until(&[("Sheet1", 1, 3)]).unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 3),
        Some(LiteralValue::Number(8.0))
    );
}

#[test]
fn scalar_after_array_clears_spill() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={1,2;3,4}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    // Switch to scalar
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=42").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 1),
        Some(LiteralValue::Number(42.0))
    );
    // Previously spilled cells cleared
    assert_eq!(engine.get_cell_value("Sheet1", 1, 2), None);
    assert_eq!(engine.get_cell_value("Sheet1", 2, 1), None);
    assert_eq!(engine.get_cell_value("Sheet1", 2, 2), None);
}

#[test]
fn empty_cells_do_not_block_spill() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // Pre-fill B1 with Empty explicitly
    engine
        .set_cell_value("Sheet1", 1, 2, LiteralValue::Empty)
        .unwrap();
    // A1 spills into A1:B1
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={10,20}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 1),
        Some(LiteralValue::Number(10.0))
    );
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 2),
        Some(LiteralValue::Number(20.0))
    );
}

#[test]
fn non_empty_values_block_spill() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // Pre-fill B1 with a non-empty value
    engine
        .set_cell_value("Sheet1", 1, 2, LiteralValue::Number(99.0))
        .unwrap();
    // A1 tries to spill 1x2 into A1:B1; B1 contains a value → #SPILL!
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={10,20}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();
    match engine.get_cell_value("Sheet1", 1, 1) {
        Some(LiteralValue::Error(e)) => assert_eq!(e, "#SPILL!"),
        v => panic!("expected #SPILL!, got {v:?}"),
    }
}

#[test]
fn overlapping_spills_conflict() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // A1 and A2 both try to spill 2x2 overlapping on A2:B3
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={1,2;3,4}").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 1, parse("={5,6;7,8}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    let a1 = engine.get_cell_value("Sheet1", 1, 1).unwrap();
    let a2 = engine.get_cell_value("Sheet1", 2, 1).unwrap();
    let is_spill = |v: &LiteralValue| matches!(v, LiteralValue::Error(e) if e.kind == formualizer_common::ExcelErrorKind::Spill);
    assert!(
        is_spill(&a1) || is_spill(&a2),
        "expected at least one anchor to be #SPILL!, got A1={a1:?}, A2={a2:?}"
    );
}

#[test]
fn formula_cells_block_spill() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // Put a scalar formula in B1
    engine
        .set_cell_formula("Sheet1", 1, 2, parse("=42").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    // A1 tries to spill 1x2 into A1:B1; B1 is occupied by a formula → #SPILL!
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={1,2}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();
    match engine.get_cell_value("Sheet1", 1, 1) {
        Some(LiteralValue::Error(e)) => assert_eq!(e, "#SPILL!"),
        v => panic!("expected #SPILL!, got {v:?}"),
    }
}

#[test]
fn overlapping_spills_firstwins_is_deterministic_sequential() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());

    // Evaluate A1 first, then A2; A2 should conflict and show #SPILL! (FirstWins)
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={1,2;3,4}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    engine
        .set_cell_formula("Sheet1", 2, 1, parse("={5,6;7,8}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    let a1 = engine.get_cell_value("Sheet1", 1, 1).unwrap();
    let a2 = engine.get_cell_value("Sheet1", 2, 1).unwrap();
    match a2 {
        LiteralValue::Error(e) => assert_eq!(e, "#SPILL!"),
        v => panic!("expected #SPILL! at A2, got {v:?} (A1={a1:?})"),
    }
}

#[test]
fn spills_on_different_sheets_do_not_conflict() {
    let wb = TestWorkbook::new();
    let mut engine = Engine::new(wb, serial_eval_config());
    // Add Sheet2
    engine.graph.add_sheet("Sheet2").unwrap();

    engine
        .set_cell_formula("Sheet1", 1, 1, parse("={1,2}").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet2", 1, 1, parse("={3,4}").unwrap())
        .unwrap();
    let _ = engine.evaluate_all().unwrap();

    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 1),
        Some(LiteralValue::Number(1.0))
    );
    assert_eq!(
        engine.get_cell_value("Sheet2", 1, 1),
        Some(LiteralValue::Number(3.0))
    );
}
