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
fn whole_columns_through_functions_spill_past_the_last_row() {
    // Microsoft's sheet-edge example: =VLOOKUP(A:A,A:C,2,FALSE) in E2 looks
    // up every row of column A, 1,048,576 results that cannot spill from row
    // 2. So do functions lifted over a whole column, IF and CHOOSE selecting
    // one by a constant, IFERROR of one, a 0 row of INDEX, SORT, and
    // TRANSPOSE (16,384 columns at most) even from row 1. Reductions,
    // selections and an unselected branch return single values.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    for r in 1..=3u32 {
        for c in 1..=3u32 {
            engine
                .set_cell_value("Sheet1", r, c, LiteralValue::Int((r * 10 + c) as i64))
                .unwrap();
        }
    }
    let spilling = [
        (2, 5, "=VLOOKUP(A:A,A:C,2,FALSE)"),
        (2, 6, "=IF(TRUE,A:A)"),
        (2, 7, "=INDEX(A:A,0)"),
        (2, 8, "=ABS(A:A)"),
        (2, 9, "=IFERROR(A:A*1,0)"),
        (2, 10, "=CHOOSE(2,1,A:A)"),
        (2, 11, "=SORT(A:A)"),
        (1, 12, "=TRANSPOSE(A:A)"),
        (2, 13, "=INDEX(A:C,0,2)"),
        (2, 14, "=COUNTIF(A:A,A:A)"),
    ];
    let single = [
        (2, 16, "=VLOOKUP(A2,A:C,2,FALSE)", 22.0),
        (2, 17, "=IF(FALSE,A:A,1)", 1.0),
        (2, 18, "=SUM(ABS(A:A))", 63.0),
        (2, 19, "=INDEX(A:A,2)", 21.0),
        (2, 20, "=CHOOSE(1,5,A:A)", 5.0),
    ];
    for (r, c, f) in spilling
        .iter()
        .copied()
        .chain(single.iter().map(|s| (s.0, s.1, s.2)))
    {
        engine
            .set_cell_formula("Sheet1", r, c, parse(f).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    for (r, c, f) in spilling {
        assert_spill(engine.get_cell_value("Sheet1", r, c), f);
        assert_eq!(engine.get_cell_value("Sheet1", r + 1, c), None, "{f}");
    }
    for (r, c, f, v) in single {
        assert_eq!(
            engine.get_cell_value("Sheet1", r, c),
            Some(LiteralValue::Number(v)),
            "{f}"
        );
    }

    // In a workbook file only dynamic arrays spill: an ordinary formula
    // takes the implicit intersection (VLOOKUP looks up A2).
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    for r in 1..=3u32 {
        for c in 1..=3u32 {
            engine
                .set_cell_value("Sheet1", r, c, LiteralValue::Int((r * 10 + c) as i64))
                .unwrap();
        }
    }
    engine
        .set_cell_formula("Sheet1", 2, 5, parse("=VLOOKUP(A:A,A:C,2,FALSE)").unwrap())
        .unwrap();
    engine.use_legacy_array_semantics();
    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 2, 5),
        Some(LiteralValue::Number(22.0))
    );
}

/// Formulas set one per column from `first_col` of `row`, two columns apart.
fn set_formulas(engine: &mut Engine<TestWorkbook>, row: u32, first_col: u32, formulas: &[&str]) {
    for (i, f) in formulas.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", row, first_col + 2 * i as u32, parse(f).unwrap())
            .unwrap();
    }
}

#[test]
fn sheet_edge_applies_to_returned_arrays_not_failed_selections() {
    // INDEX(A:A,0,2) is #REF! (A:A has one column), so IFERROR returns its
    // fallback and nothing spills from a whole column; an error is the
    // formula's value. A selection that cannot fail, or did not, still
    // returns the whole column.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    for r in 1..=3u32 {
        engine
            .set_cell_value("Sheet1", r, 1, LiteralValue::Int((r * 10 + 1) as i64))
            .unwrap();
    }
    engine
        .set_cell_value("Sheet1", 1, 3, LiteralValue::Int(1))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 3, LiteralValue::Int(2))
        .unwrap();
    let kept = [
        "=IFERROR(INDEX(A:A,0,2),99)",
        "=INDEX(A:A,0,2)",
        "=IFERROR(INDEX(A:A,0,2),{1,2})",
        "=IFERROR(SORT(A:A,2),0)",
        "=INDEX(A:A,0,C2)",
        "=INDEX(A:A,0,2)+{1,2}",
        "=IFERROR(INDEX(A:A,0,C2),SEQUENCE(2))",
    ];
    let kept_too = [
        ("=N(INDEX(A:A,0,1))", Some(11.0)),
        ("=N(IF(TRUE,A:A,0))", Some(11.0)),
        ("=T(INDEX(A:A,0,1))", None),
        ("=INDEX(A:A,0,{1,1})", None),
    ];
    let spilling = [
        "=INDEX(A:A,0,C1)",
        "=INDEX(A:A,0,C1)*2",
        "=IFERROR(SORT(A:A,1,-1),0)",
        "=IFERROR(INDEX(A:C,0,2),0)",
        "=-INDEX(A:A,0,1)",
    ];
    set_formulas(&mut engine, 2, 5, &kept);
    set_formulas(&mut engine, 2, 5 + 2 * kept.len() as u32, &spilling);
    // N and T read the first cell of the column INDEX or IF returns; INDEX
    // lifted over an array of columns gives one value per element.
    for (i, (f, _)) in kept_too.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", 6, 5 + 3 * i as u32, parse(f).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    for (i, (f, value)) in kept_too.iter().enumerate() {
        let got = engine.get_cell_value("Sheet1", 6, 5 + 3 * i as u32);
        match value {
            Some(v) => assert_eq!(got, Some(LiteralValue::Number(*v)), "{f}"),
            None => assert!(
                !matches!(&got, Some(LiteralValue::Error(e)) if e.kind == formualizer_common::ExcelErrorKind::Spill),
                "{f}: {got:?}"
            ),
        }
    }
    let at = |i: usize, row: u32| engine.get_cell_value("Sheet1", row, 5 + 2 * i as u32);
    let n = |v: f64| Some(LiteralValue::Number(v));
    let is_ref = |v: Option<LiteralValue>| matches!(v, Some(LiteralValue::Error(e)) if e.kind == formualizer_common::ExcelErrorKind::Ref);
    assert_eq!(at(0, 2), n(99.0));
    assert!(is_ref(at(1, 2)), "{:?}", at(1, 2));
    assert_eq!(
        (at(2, 2), engine.get_cell_value("Sheet1", 2, 10)),
        (n(1.0), n(2.0))
    );
    assert_eq!(at(3, 2), n(0.0));
    assert!(is_ref(at(4, 2)), "{:?}", at(4, 2));
    assert!(is_ref(at(5, 2)) && is_ref(engine.get_cell_value("Sheet1", 2, 16)));
    assert_eq!((at(6, 2), at(6, 3)), (n(1.0), n(2.0)));
    for (i, f) in spilling.iter().enumerate() {
        assert_spill(at(kept.len() + i, 2), f);
    }

    // A whole column read over a used part of one row (on another sheet)
    // is still a whole column.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    engine
        .set_cell_value("Data", 1, 1, LiteralValue::Int(5))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 2, parse("=Data!A:A*2").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_spill(engine.get_cell_value("Sheet1", 2, 2), "=Data!A:A*2");
}

#[test]
fn whole_columns_returned_through_functions_spill_past_the_last_row() {
    // IF, IFERROR, CHOOSE, SWITCH, XLOOKUP and LET return the whole column
    // they select, whatever selects it, and INDIRECT the whole column its
    // text names: 1,048,576 rows that cannot spill from row 2 (a whole row,
    // 16,384 columns, not from column B).
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Int(11))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 1, LiteralValue::Int(22))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 1, 2, LiteralValue::Boolean(true))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 1, 3, LiteralValue::Int(1))
        .unwrap();
    let spilling = [
        "=IF(B1,A:A,0)",
        "=IF(1=1,A:A,99)",
        "=IFERROR(1/0,A:A)",
        "=INDIRECT(\"A:A\")",
        "=LET(x,A:A,x)",
        "=CHOOSE(C1,A:A,1)",
        "=SWITCH(1,1,A:A)",
        "=XLOOKUP(9,A:A,A:A,A:A)",
    ];
    let fitting = [
        ("=IF(NOT(B1),A:A,0)", 0.0, None),
        ("=TAKE(A:A,2)", 11.0, Some(22.0)),
        ("=IF(B1,A1:A2,0)", 11.0, Some(22.0)),
        ("=INDEX(A:A,C1+1)", 22.0, None),
    ];
    set_formulas(&mut engine, 2, 5, &spilling);
    let first = 5 + 2 * spilling.len() as u32;
    set_formulas(
        &mut engine,
        2,
        first,
        &fitting.iter().map(|f| f.0).collect::<Vec<_>>(),
    );
    engine
        .set_cell_formula("Sheet1", 5, 2, parse("=INDIRECT(\"1:1\")").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    for (i, f) in spilling.iter().enumerate() {
        let col = 5 + 2 * i as u32;
        assert_spill(engine.get_cell_value("Sheet1", 2, col), f);
        assert_eq!(engine.get_cell_value("Sheet1", 3, col), None, "{f}");
    }
    assert_spill(engine.get_cell_value("Sheet1", 5, 2), "=INDIRECT(\"1:1\")");
    for (i, (f, anchor, below)) in fitting.iter().enumerate() {
        let col = first + 2 * i as u32;
        assert_eq!(
            engine.get_cell_value("Sheet1", 2, col),
            Some(LiteralValue::Number(*anchor)),
            "{f}"
        );
        if let Some(below) = below {
            assert_eq!(
                engine.get_cell_value("Sheet1", 3, col),
                Some(LiteralValue::Number(*below)),
                "{f}"
            );
        }
    }

    // From row 1 the whole column fits.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Int(11))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 1, LiteralValue::Int(22))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 3, parse("=IF(A1>0,A:A,0)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 2, 3),
        Some(LiteralValue::Number(22.0))
    );
}

#[test]
fn blocked_spill_extent_is_the_result_that_could_not_spill() {
    // A1 =SEQUENCE(C1) is blocked by A2; E2 =A:A runs past the sheet's edge.
    let mut engine = Engine::new(TestWorkbook::new(), serial_eval_config());
    engine
        .set_cell_value("Sheet1", 1, 3, LiteralValue::Int(3))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 1, LiteralValue::Text("x".into()))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=SEQUENCE(C1)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 5, parse("=A:A").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_spill(engine.get_cell_value("Sheet1", 1, 1), "blocked");
    assert_eq!(engine.blocked_spill_extent("Sheet1", 1, 1), Some((3, 1)));
    assert_spill(engine.get_cell_value("Sheet1", 2, 5), "sheet edge");
    assert_eq!(engine.blocked_spill_extent("Sheet1", 2, 5), None);
    // Still blocked, with a two-row result.
    engine
        .set_cell_value("Sheet1", 1, 3, LiteralValue::Int(2))
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(engine.blocked_spill_extent("Sheet1", 1, 1), Some((2, 1)));
    // Without the blocker it spills.
    engine
        .set_cell_value("Sheet1", 2, 1, LiteralValue::Empty)
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=SEQUENCE(C1)").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 2, 1),
        Some(LiteralValue::Number(2.0))
    );
    assert_eq!(engine.blocked_spill_extent("Sheet1", 1, 1), None);
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
