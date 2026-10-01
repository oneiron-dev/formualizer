//! Excel-parity semantics for XLOOKUP no-match and exact-match class rules.
//!
//! - An omitted-in-place `if_not_found` slot (`XLOOKUP(v,l,r,,mode)`) is not a
//!   supplied argument: a no-match returns #N/A, never the slot's implicit 0.
//! - Exact match never crosses value classes: a text needle does not find a
//!   number, whichever search direction or storage path is used.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

fn numeric_grid_engine() -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, v) in [(1u32, 10.0), (2, 20.0), (3, 30.0)] {
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(v))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 3, LiteralValue::Number(v * 10.0))
            .unwrap();
    }
    engine
}

fn assert_na(engine: &Engine<TestWorkbook>, row: u32, col: u32) {
    match engine.get_cell_value("Sheet1", row, col) {
        Some(LiteralValue::Error(e)) => assert_eq!(e.kind, ExcelErrorKind::Na),
        other => panic!("expected #N/A, got {other:?}"),
    }
}

#[test]
fn xlookup_omitted_if_not_found_returns_na_on_approximate_no_match() {
    let mut engine = numeric_grid_engine();
    engine
        .set_cell_formula(
            "Sheet1",
            10,
            1,
            parse("=XLOOKUP(5,B1:B3,C1:C3,,-1)").unwrap(),
        )
        .unwrap();
    engine
        .set_cell_formula(
            "Sheet1",
            11,
            1,
            parse("=XLOOKUP(35,B1:B3,C1:C3,,1)").unwrap(),
        )
        .unwrap();
    engine
        .set_cell_formula(
            "Sheet1",
            12,
            1,
            parse("=XLOOKUP(35,B1:B3,C1:C3,\"none\",1)").unwrap(),
        )
        .unwrap();

    engine.evaluate_all().unwrap();

    assert_na(&engine, 10, 1);
    assert_na(&engine, 11, 1);
    assert_eq!(
        engine.get_cell_value("Sheet1", 12, 1),
        Some(LiteralValue::Text("none".into()))
    );
}

#[test]
fn xlookup_text_needle_does_not_coerce_to_number() {
    let mut engine = numeric_grid_engine();
    engine
        .set_cell_formula(
            "Sheet1",
            10,
            1,
            parse("=XLOOKUP(\"20\",B1:B3,C1:C3)").unwrap(),
        )
        .unwrap();
    engine
        .set_cell_formula(
            "Sheet1",
            11,
            1,
            parse("=XLOOKUP(\"20\",B1:B3,C1:C3,,0,-1)").unwrap(),
        )
        .unwrap();
    // Control: a real numeric needle still matches.
    engine
        .set_cell_formula("Sheet1", 12, 1, parse("=XLOOKUP(20,B1:B3,C1:C3)").unwrap())
        .unwrap();

    engine.evaluate_all().unwrap();

    assert_na(&engine, 10, 1);
    assert_na(&engine, 11, 1);
    assert_eq!(
        engine.get_cell_value("Sheet1", 12, 1),
        Some(LiteralValue::Number(200.0))
    );
}

fn eval_formula(engine: &mut Engine<TestWorkbook>, formula: &str) -> Option<LiteralValue> {
    engine
        .set_cell_formula("Sheet1", 30, 10, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 30, 10)
}

fn number(value: Option<LiteralValue>) -> f64 {
    match value {
        Some(LiteralValue::Number(n)) => n,
        Some(LiteralValue::Int(i)) => i as f64,
        other => panic!("expected a number, got {other:?}"),
    }
}

/// XLOOKUP and XMATCH with match_mode -1/1 and a linear search_mode scan every
/// entry: the lookup array need not be sorted, the nearest entry on the
/// requested side wins, an exact match wins outright, and a miss reaches
/// if_not_found. Only binary search modes (2/-2) assume sorted data.
#[test]
fn approximate_match_modes_scan_unsorted_data() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // Row 1: headers; row 2: unsorted keys with a blank in F2.
    for (col, (header, key)) in [
        (10.0, Some(0.065)),
        (20.0, Some(0.344)),
        (30.0, Some(0.109)),
        (40.0, Some(0.436)),
        (50.0, Some(0.2)),
        (60.0, None),
    ]
    .into_iter()
    .enumerate()
    {
        let col = col as u32 + 1;
        engine
            .set_cell_value("Sheet1", 1, col, LiteralValue::Number(header))
            .unwrap();
        if let Some(key) = key {
            engine
                .set_cell_value("Sheet1", 2, col, LiteralValue::Number(key))
                .unwrap();
        }
    }
    for (formula, expected) in [
        ("=XLOOKUP(0.3,A2:F2,A1:F1,\"\",1)", 20.0),
        ("=XLOOKUP(0.3,A2:F2,A1:F1,\"\",-1)", 50.0),
        ("=XLOOKUP(0.1,A2:F2,A1:F1,\"\",1)", 30.0),
        ("=XLOOKUP(0.2,A2:F2,A1:F1,\"\",1)", 50.0),
        ("=XLOOKUP(0.3,{0.1,0.4,0.2},{1,2,3},\"\",1)", 2.0),
        ("=XLOOKUP(0.3,{0.1,0.4,0.2},{1,2,3},\"\",-1)", 3.0),
        ("=XMATCH(0.3,{0.1,0.4,0.2},1)", 2.0),
        // Equal nearest entries: the first met in search order.
        ("=XLOOKUP(4,{5,1,5},{1,2,3},,1)", 1.0),
        ("=XLOOKUP(4,{5,1,5},{1,2,3},,1,-1)", 3.0),
        ("=XMATCH(4,{5,1,5},1,-1)", 3.0),
        // Control: sorted data answers as before.
        ("=XLOOKUP(25,{10,20,30},{1,2,3},,-1)", 2.0),
        ("=XLOOKUP(25,{10,20,30},{1,2,3},,1)", 3.0),
    ] {
        assert_eq!(
            number(eval_formula(&mut engine, formula)),
            expected,
            "{formula}"
        );
    }
    assert_eq!(
        eval_formula(
            &mut engine,
            "=XLOOKUP(0.5,{0.3,0.1,0.2},{1,2,3},\"none\",1)"
        ),
        Some(LiteralValue::Text("none".into()))
    );
    // A text lookup value is ordered against the text entries.
    assert_eq!(
        number(eval_formula(
            &mut engine,
            "=XLOOKUP(\"m\",{\"z\",\"a\",\"p\"},{1,2,3},,1)"
        )),
        3.0
    );
}

/// In XLOOKUP's exact-or-next-larger scan an empty entry ranks above every
/// number: a qualifying number still wins, but when no number is at or above
/// the lookup value the first blank met is returned instead of if_not_found.
/// Excel: `=XLOOKUP(0.5,FILTER(B2:F3,A2:A3=2),B1:F1,"",1)` => 30, the header
/// of the first blank in the filtered row.
#[test]
fn next_larger_ranks_a_blank_above_every_number() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let rows: [(i64, [Option<f64>; 5]); 2] = [
        (1, [Some(0.1), None, Some(0.3), None, Some(0.2)]),
        (2, [Some(0.065), Some(0.109), None, None, Some(0.436)]),
    ];
    for col in 0..5u32 {
        engine
            .set_cell_value(
                "Sheet1",
                1,
                col + 2,
                LiteralValue::Int((col as i64 + 1) * 10),
            )
            .unwrap();
    }
    for (row, (id, keys)) in rows.into_iter().enumerate() {
        let row = row as u32 + 2;
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Int(id))
            .unwrap();
        for (col, key) in keys.into_iter().enumerate() {
            if let Some(key) = key {
                engine
                    .set_cell_value("Sheet1", row, col as u32 + 2, LiteralValue::Number(key))
                    .unwrap();
            }
        }
    }
    for (formula, expected) in [
        ("=XLOOKUP(0.5,FILTER(B2:F3,A2:A3=2),B1:F1,\"\",1)", 30.0),
        ("=XLOOKUP(0.3,FILTER(B2:F3,A2:A3=2),B1:F1,\"\",1)", 50.0),
        ("=XLOOKUP(0.5,B2:F2,B1:F1,\"\",1)", 20.0),
        ("=XLOOKUP(0.5,B2:F2,B1:F1,\"\",1,-1)", 40.0),
        ("=XMATCH(0.5,B2:F2,1)", 2.0),
        // Control: a qualifying number outranks the blanks.
        ("=XLOOKUP(0.25,B2:F2,B1:F1,\"\",1)", 30.0),
    ] {
        assert_eq!(
            number(eval_formula(&mut engine, formula)),
            expected,
            "{formula}"
        );
    }
    // A blank is not a 0 below the lookup value.
    assert_eq!(
        eval_formula(&mut engine, "=XLOOKUP(0.05,B2:F2,B1:F1,\"none\",-1)"),
        Some(LiteralValue::Text("none".into()))
    );
}
