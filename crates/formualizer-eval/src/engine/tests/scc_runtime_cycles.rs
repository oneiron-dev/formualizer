//! Stage 2 — SCC evaluation under `CycleDetection::Runtime` (RFC #112).
//!
//! Test inventory from `formualizer-stage2-scc-evaluation-design.md` §5 and
//! the spec's §7 Error-policy subset: phantom (guarded) cycles produce
//! values, live cycles produce `#CIRC!` with live-cycle-only blast radius,
//! and `CycleDetection::Static` (the default) stays byte-for-byte today's
//! behavior.

use crate::engine::graph::editor::undo_engine::UndoEngine;
use crate::engine::named_range::{NameScope, NamedDefinition};
use crate::engine::{CycleConfig, CycleDetection, CyclePolicy, Engine, EvalConfig};
use crate::reference::{CellRef, Coord, RangeRef};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue, PackedSheetCell};
use formualizer_parse::parser::parse;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn runtime_cycle() -> CycleConfig {
    CycleConfig {
        detection: CycleDetection::Runtime,
        policy: CyclePolicy::Error,
    }
}

fn runtime_cfg() -> EvalConfig {
    EvalConfig::default()
        .with_cycle(runtime_cycle())
        .with_virtual_dep_telemetry(true)
}

fn runtime_engine() -> Engine<TestWorkbook> {
    Engine::new(TestWorkbook::new(), runtime_cfg())
}

fn static_engine() -> Engine<TestWorkbook> {
    Engine::new(TestWorkbook::new(), EvalConfig::default())
}

fn set_formula(engine: &mut Engine<TestWorkbook>, sheet: &str, row: u32, col: u32, f: &str) {
    engine
        .set_cell_formula(sheet, row, col, parse(f).expect("parse"))
        .expect("set formula");
}

fn set_value(engine: &mut Engine<TestWorkbook>, sheet: &str, row: u32, col: u32, v: LiteralValue) {
    engine
        .set_cell_value(sheet, row, col, v)
        .expect("set value");
}

fn num(engine: &Engine<TestWorkbook>, sheet: &str, row: u32, col: u32) -> f64 {
    match engine.get_cell_value(sheet, row, col) {
        Some(LiteralValue::Number(n)) => n,
        Some(LiteralValue::Int(i)) => i as f64,
        other => panic!("expected number at {sheet} r{row}c{col}, got {other:?}"),
    }
}

fn is_circ(engine: &Engine<TestWorkbook>, sheet: &str, row: u32, col: u32) -> bool {
    matches!(
        engine.get_cell_value(sheet, row, col),
        Some(LiteralValue::Error(e)) if e.kind == ExcelErrorKind::Circ
    )
}

/// Build the discussion-#99 guarded pair: A1 guard, A2/A3 the static SCC.
fn build_99_pair(engine: &mut Engine<TestWorkbook>, guard: bool) {
    set_value(engine, "Sheet1", 1, 1, LiteralValue::Boolean(guard));
    set_formula(engine, "Sheet1", 2, 1, "=IF(A1,555,A3)");
    set_formula(engine, "Sheet1", 3, 1, "=IF(A1,A2,999)");
}

/* ───────────────────────── 7.1 self-reference ───────────────────────── */

/// Engine rule that PRE-EMPTS spec §7.1's eval-time `#CIRC!`: a direct
/// self-reference (`=A1+1` in A1, or an expanded range containing the cell)
/// is rejected when the formula is SET ("Self-reference detected"). Runtime
/// mode must not change that edit-time rule. Eval-time self-loop handling is
/// still exercised through dynamic refs (see the INDIRECT tests) and the
/// `live_graph` unit tests.
#[test]
fn direct_self_reference_rejected_at_ingest_in_both_modes() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        let err = engine
            .set_cell_formula("Sheet1", 1, 1, parse("=A1+1").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, ExcelErrorKind::Circ);
        // Dense range covering the cell expands to direct deps → same rule.
        let err = engine
            .set_cell_formula("Sheet1", 5, 1, parse("=SUM(A1:A10)").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, ExcelErrorKind::Circ);
    }
}

#[test]
fn live_two_cycle_is_circ_via_evaluate_all_and_evaluate_cell() {
    let mut engine = runtime_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=B1+1");
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1");
    let res = engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 1));
    assert!(is_circ(&engine, "Sheet1", 1, 2));
    assert_eq!(res.cycle_errors, 1);
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.static_sccs, 1);
    assert_eq!(t.phantom_sccs, 0);
    assert_eq!(t.live_cycles_witnessed, 1);
    assert_eq!(t.circ_cells_stamped, 2);

    let mut engine = runtime_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=B1+1");
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1");
    let v = engine.evaluate_cell("Sheet1", 1, 1).unwrap();
    assert!(
        matches!(v, Some(LiteralValue::Error(ref e)) if e.kind == ExcelErrorKind::Circ),
        "got {v:?}"
    );
}

/* ──────────────── 7.2 #99 guarded pair — both polarities ─────────────── */

#[test]
fn guarded_pair_99_evaluates_to_values_under_runtime() {
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    let res = engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);
    assert_eq!(res.cycle_errors, 0, "phantom SCC must not count as a cycle");
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.static_sccs, 1);
    assert_eq!(t.phantom_sccs, 1);
    assert_eq!(t.live_cycles_witnessed, 0);
    assert_eq!(t.circ_cells_stamped, 0);
    // A2 is ordered first and A3 reads it fresh: exactly one pass.
    assert_eq!(t.settle_passes_total, 1);
}

#[test]
fn guarded_pair_99_opposite_polarity_settles_to_values() {
    // Same pair but the live edge points at the *later*-ordered member:
    // A2 = IF(A1, A3, 999), A3 = IF(A1, 555, A2). With A1=TRUE the live
    // edge is A2→A3 and A2 runs first, so one settle re-eval is needed.
    let mut engine = runtime_engine();
    set_value(&mut engine, "Sheet1", 1, 1, LiteralValue::Boolean(true));
    set_formula(&mut engine, "Sheet1", 2, 1, "=IF(A1,A3,999)");
    set_formula(&mut engine, "Sheet1", 3, 1, "=IF(A1,555,A2)");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.phantom_sccs, 1);
    assert_eq!(t.settle_passes_total, 2, "polarity flip costs one settle");
}

#[test]
fn guarded_pair_99_guard_flip_between_recalcs_reverses_values() {
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);

    // Flip the guard: live edges reverse, values follow.
    set_value(&mut engine, "Sheet1", 1, 1, LiteralValue::Boolean(false));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 999.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 999.0);
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);

    // And back.
    set_value(&mut engine, "Sheet1", 1, 1, LiteralValue::Boolean(true));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);
}

#[test]
fn guarded_pair_99_via_evaluate_cell_both_polarities() {
    // Demand path (evaluate_until under the hood): the demand closure pulls
    // the whole SCC via static deps and evaluates it as one task.
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    let v = engine.evaluate_cell("Sheet1", 2, 1).unwrap();
    assert_eq!(v, Some(LiteralValue::Number(555.0)));
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);

    let mut engine = runtime_engine();
    build_99_pair(&mut engine, false);
    let v = engine.evaluate_cell("Sheet1", 3, 1).unwrap();
    assert_eq!(v, Some(LiteralValue::Number(999.0)));
    assert_eq!(num(&engine, "Sheet1", 2, 1), 999.0);
}

#[test]
fn guarded_chains_three_and_five_cells_mixed_polarity() {
    // 3-cell guarded ring: every static edge exists, live edges form a chain.
    let mut engine = runtime_engine();
    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(true)); // G1
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(G1,100,A2)");
    set_formula(&mut engine, "Sheet1", 2, 1, "=IF(G1,A1,A3)");
    set_formula(&mut engine, "Sheet1", 3, 1, "=IF(G1,A2,100)");
    engine.evaluate_all().unwrap();
    for r in 1..=3 {
        assert_eq!(num(&engine, "Sheet1", r, 1), 100.0, "row {r}");
    }
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);

    // Flip: live edges reverse direction (reads now go down the column,
    // against member order → settling required).
    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(false));
    engine.evaluate_all().unwrap();
    for r in 1..=3 {
        assert_eq!(num(&engine, "Sheet1", r, 1), 100.0, "row {r} after flip");
    }

    // 5-cell ring with mixed polarity guards.
    let mut engine = runtime_engine();
    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(true));
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(G1,42,B5)");
    set_formula(&mut engine, "Sheet1", 2, 1, "=IF(G1,A1,B1)"); // reads up (fresh)
    set_formula(&mut engine, "Sheet1", 3, 1, "=IF(NOT(G1),B1,A2)"); // reads up (fresh)
    set_formula(&mut engine, "Sheet1", 4, 1, "=IF(G1,A5,A3)"); // reads DOWN (stale)
    set_formula(&mut engine, "Sheet1", 5, 1, "=IF(G1,A3,A4)"); // reads up
    // Static ring closure: B5 referenced by A1's untaken branch.
    set_formula(&mut engine, "Sheet1", 5, 2, "=A1");
    engine.evaluate_all().unwrap();
    for r in 1..=5 {
        assert_eq!(num(&engine, "Sheet1", r, 1), 42.0, "row {r}");
    }
}

/* ─────────────── 7.3 guard inside the cycle (always live) ────────────── */

#[test]
fn guard_reading_cycle_member_is_a_live_cycle() {
    // Spec §7.3's literal example (`=IF(A1, A2+1, 5)` in A2) contains a
    // direct self-reference and is rejected at ingest; the same semantics —
    // a guard read that is itself a live edge into the cycle — is built with
    // two cells: A1's GUARD always reads A2, and A2 always reads A1, so the
    // live subgraph is cyclic in every guard state.
    for guard_seed in [0.0, 9.0] {
        let mut engine = runtime_engine();
        set_value(
            &mut engine,
            "Sheet1",
            1,
            2,
            LiteralValue::Number(guard_seed),
        );
        set_formula(&mut engine, "Sheet1", 1, 1, "=IF(A2>0,A2+1,5)");
        set_formula(&mut engine, "Sheet1", 2, 1, "=A1+B1");
        engine.evaluate_all().unwrap();
        assert!(is_circ(&engine, "Sheet1", 1, 1), "seed {guard_seed}");
        assert!(is_circ(&engine, "Sheet1", 2, 1), "seed {guard_seed}");
        assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
    }
}

/* ─────────────── 7.4 arithmetic routing — always live ────────────────── */

#[test]
fn arithmetic_routing_stays_circ() {
    // Both operands of `+`/`*` always evaluate: the reads genuinely occur,
    // so the cycle is live regardless of the mixing weight g (A1).
    let mut engine = runtime_engine();
    set_value(&mut engine, "Sheet1", 1, 1, LiteralValue::Number(0.0)); // g
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1*99+(1-A1)*C1"); // B1
    set_formula(&mut engine, "Sheet1", 1, 3, "=A1*B1+(1-A1)*7"); // C1
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 2));
    assert!(is_circ(&engine, "Sheet1", 1, 3));
}

/* ─────────────────────── 7.8 range self-inclusion ────────────────────── */

#[test]
fn range_mediated_live_cycle_is_circ() {
    // A5 ranges over B1:B10 which contains B5 = A5: the rect read records a
    // live edge into B5 and the cycle is witnessed. (Direct self-inclusion —
    // `=SUM(A1:A10)` in A5 — is rejected at ingest; see the §7.1 test.)
    let mut engine = runtime_engine();
    for r in 1..=10u32 {
        if r != 5 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(r as f64));
        }
    }
    set_formula(&mut engine, "Sheet1", 5, 1, "=SUM(B1:B10)");
    set_formula(&mut engine, "Sheet1", 5, 2, "=A5");
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 5, 1));
    assert!(is_circ(&engine, "Sheet1", 5, 2));
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
}

#[test]
fn range_mediated_guarded_cycle_is_phantom_until_guard_flips() {
    // Same shape but the range read sits in a guarded branch: phantom while
    // the guard holds (Static would stamp #CIRC — documented diff), live
    // cycle when it flips.
    let mut engine = runtime_engine();
    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(true)); // G1
    for r in 1..=10u32 {
        if r != 5 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(1.0));
        }
    }
    set_formula(&mut engine, "Sheet1", 5, 1, "=IF(G1,5,SUM(B1:B10))");
    set_formula(&mut engine, "Sheet1", 5, 2, "=A5");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 5, 2), 5.0);
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);

    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(false));
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 5, 1));
    assert!(is_circ(&engine, "Sheet1", 5, 2));
}

#[test]
fn named_range_covering_the_cell_rejected_at_ingest_in_both_modes() {
    // A name whose region covers the formula's own cell is rejected when the
    // formula is set ("Circular reference through named range") — existing
    // engine rule, identical under Runtime.
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        let sheet_id = engine.sheet_id("Sheet1").unwrap();
        let nr = RangeRef::new(
            CellRef::new(sheet_id, Coord::from_excel(1, 1, true, true)),
            CellRef::new(sheet_id, Coord::from_excel(10, 1, true, true)),
        );
        engine
            .define_name("COVER", NamedDefinition::Range(nr), NameScope::Workbook)
            .unwrap();
        let err = engine
            .set_cell_formula("Sheet1", 5, 1, parse("=SUM(COVER)").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, ExcelErrorKind::Circ);
    }
}

/// Spec §7.8 stripe-path self-inclusion (#120): whole-column `=SUM(B:B)` in
/// B1 references column B, which contains B1 itself. At ingest the stripe
/// region is detected to cover the formula's own cell, so a SELF-LOOP edge is
/// recorded; Tarjan then sees a single-vertex SCC with a self-loop and
/// `separate_cycles` classifies it as a cycle. Both modes resolve to `#CIRC!`
/// under `CyclePolicy::Error` and Runtime still matches Static exactly.
///
/// This pin previously documented the *incorrect* pre-#120 value (silent
/// `9.0`, 0 cycle errors); it was consciously updated when the detection gap
/// was closed.
#[test]
fn whole_column_self_inclusion_gap_runtime_matches_static() {
    let run = |cfg: EvalConfig| {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        for r in 2..=4u32 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(r as f64));
        }
        set_formula(&mut engine, "Sheet1", 1, 2, "=SUM(B:B)");
        let res = engine.evaluate_all().unwrap();
        (engine.get_cell_value("Sheet1", 1, 2), res.cycle_errors)
    };
    let static_out = run(EvalConfig::default());
    let runtime_out = run(runtime_cfg());
    assert_eq!(static_out, runtime_out);
    // Whole-column self-inclusion is now a detected cycle in both modes.
    assert!(
        matches!(
            static_out.0,
            Some(LiteralValue::Error(ref e)) if e.kind == ExcelErrorKind::Circ
        ),
        "expected #CIRC, got {:?}",
        static_out.0
    );
    assert_eq!(static_out.1, 1);
}

/// #120: whole-column self-inclusion `=SUM(B:B)` placed in column B is a
/// circular reference (Excel flags it). The stripe region covers the
/// formula's own cell, so a self-loop is recorded at ingest and the cell
/// resolves to `#CIRC!` under both detection modes.
#[test]
fn whole_column_in_column_self_inclusion_is_circ_both_modes() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        for r in 2..=4u32 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(r as f64));
        }
        // B1 = SUM(B:B) ⇒ B1 ∈ column B ⇒ self-loop.
        set_formula(&mut engine, "Sheet1", 1, 2, "=SUM(B:B)");
        engine.evaluate_all().unwrap();
        assert!(
            is_circ(&engine, "Sheet1", 1, 2),
            "B1=SUM(B:B) must be #CIRC, got {:?}",
            engine.get_cell_value("Sheet1", 1, 2)
        );
    }
}

/// #120 symmetric whole-row case: `=SUM(2:2)` placed in row 2 is circular.
#[test]
fn whole_row_in_row_self_inclusion_is_circ_both_modes() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        for c in 3..=5u32 {
            set_value(&mut engine, "Sheet1", 2, c, LiteralValue::Number(c as f64));
        }
        // B2 = SUM(2:2) ⇒ B2 ∈ row 2 ⇒ self-loop.
        set_formula(&mut engine, "Sheet1", 2, 2, "=SUM(2:2)");
        engine.evaluate_all().unwrap();
        assert!(
            is_circ(&engine, "Sheet1", 2, 2),
            "B2=SUM(2:2) must be #CIRC, got {:?}",
            engine.get_cell_value("Sheet1", 2, 2)
        );
    }
}

/// #120 large bounded range over the expansion limit: `=SUM(B1:B100000)` in
/// B5 is stripe-compressed (not expanded to explicit cell edges), so the
/// pre-#120 ingest self-reference check never saw B5. The stripe region now
/// covers B5 ⇒ self-loop ⇒ `#CIRC!`.
#[test]
fn large_bounded_range_self_inclusion_is_circ_both_modes() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        for r in 1..=4u32 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(r as f64));
        }
        set_formula(&mut engine, "Sheet1", 5, 2, "=SUM(B1:B100000)");
        engine.evaluate_all().unwrap();
        assert!(
            is_circ(&engine, "Sheet1", 5, 2),
            "B5=SUM(B1:B100000) must be #CIRC, got {:?}",
            engine.get_cell_value("Sheet1", 5, 2)
        );
    }
}

/// #120 NEGATIVE control: a whole-column reference consumed from OUTSIDE the
/// column stays acyclic and computes normally.
#[test]
fn whole_column_referenced_from_outside_stays_acyclic() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        for r in 1..=3u32 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(r as f64));
        }
        // C1 = SUM(B:B): column C is not referenced ⇒ no self-loop.
        set_formula(&mut engine, "Sheet1", 1, 3, "=SUM(B:B)");
        let res = engine.evaluate_all().unwrap();
        assert_eq!(num(&engine, "Sheet1", 1, 3), 6.0);
        assert_eq!(res.cycle_errors, 0);
    }
}

/// #120 NEGATIVE control: a large bounded range NOT intersecting the
/// formula's own cell is unchanged (computes, no cycle).
#[test]
fn large_bounded_range_non_intersecting_stays_acyclic() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        for r in 1..=4u32 {
            set_value(&mut engine, "Sheet1", r, 1, LiteralValue::Number(r as f64));
        }
        // B5 = SUM(A1:A100000): own cell B5 is in column B, range is column A.
        set_formula(&mut engine, "Sheet1", 5, 2, "=SUM(A1:A100000)");
        let res = engine.evaluate_all().unwrap();
        assert_eq!(num(&engine, "Sheet1", 5, 2), 10.0);
        assert_eq!(res.cycle_errors, 0);
    }
}

/* ──────────────────── 7.9 spill anchor inside an SCC ─────────────────── */

#[test]
fn spill_anchor_in_scc_is_circ_and_region_freed() {
    let mut engine = runtime_engine();
    // Prime: B1 spills SEQUENCE(C1) with C1 = 3 → B1:B3.
    set_value(&mut engine, "Sheet1", 1, 3, LiteralValue::Number(3.0));
    set_formula(&mut engine, "Sheet1", 1, 2, "=SEQUENCE(C1)");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 2), 2.0);
    assert_eq!(num(&engine, "Sheet1", 3, 2), 3.0);

    // Introduce the cycle: C1 = B1 + 1 ⇒ static SCC {B1, C1}.
    set_formula(&mut engine, "Sheet1", 1, 3, "=B1+1");
    engine.evaluate_all().unwrap();

    // Anchor pre-stamped #CIRC with spill teardown (spec §7.9, #115).
    assert!(is_circ(&engine, "Sheet1", 1, 2));
    for r in 2..=3 {
        assert!(
            matches!(
                engine.get_cell_value("Sheet1", r, 2),
                None | Some(LiteralValue::Empty)
            ),
            "spilled B{r} must be cleared, got {:?}",
            engine.get_cell_value("Sheet1", r, 2)
        );
    }
    // C1 reads the stamped anchor: #CIRC propagates through arithmetic.
    assert!(is_circ(&engine, "Sheet1", 1, 3));

    // Region freed: a new spill into the former region succeeds.
    set_formula(&mut engine, "Sheet1", 2, 2, "=SEQUENCE(2)");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 2), 1.0);
    assert_eq!(num(&engine, "Sheet1", 3, 2), 2.0);
}

#[test]
fn array_result_first_produced_inside_scc_is_stamped() {
    // A member that would *become* a spill anchor during the SCC task gets
    // the conservative §7.9 verdict instead of spilling.
    let mut engine = runtime_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=SEQUENCE(2)+0*A2");
    set_formula(&mut engine, "Sheet1", 2, 1, "=A1");
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 1));
    assert!(
        is_circ(&engine, "Sheet1", 2, 1),
        "readers see propagated #CIRC"
    );
}

/* ───────────── 7.13 cross-sheet and named-formula members ────────────── */

#[test]
fn cross_sheet_phantom_scc_produces_values() {
    let mut engine = runtime_engine();
    engine.add_sheet("Sheet2").unwrap();
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,5,Sheet2!A1)");
    set_formula(&mut engine, "Sheet2", 1, 1, "=Sheet1!A1+1");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
    assert_eq!(num(&engine, "Sheet2", 1, 1), 6.0);
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);
}

/// Entering a cell formula that reads a name already depending on that cell
/// is rejected at ingest in both modes (existing rule).
#[test]
fn named_formula_cycle_rejected_at_ingest_in_both_modes() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        engine
            .define_name(
                "N",
                NamedDefinition::Formula {
                    ast: parse("=A1+1").unwrap(),
                    dependencies: Vec::new(),
                    range_deps: Vec::new(),
                },
                NameScope::Workbook,
            )
            .unwrap();
        let err = engine
            .set_cell_formula("Sheet1", 1, 1, parse("=N").unwrap())
            .unwrap_err();
        assert_eq!(err.kind, ExcelErrorKind::Circ);
    }
}

/// Direct SCC-task test for name-vertex members (spec §7.13): the scheduler
/// cannot currently produce a {cell, name} SCC (no static name-cycle edges),
/// so drive `evaluate_scc_unit` with the membership directly. Pass 1 records
/// the cell's read OF the name (by folded key) and the name's read of the
/// cell — a witnessed live cycle stamping both.
#[test]
fn named_formula_member_live_cycle_is_circ_in_scc_task() {
    let mut engine = runtime_engine();
    engine
        .define_name(
            "N",
            NamedDefinition::Literal(LiteralValue::Number(1.0)),
            NameScope::Workbook,
        )
        .unwrap();
    set_formula(&mut engine, "Sheet1", 1, 1, "=N");
    engine.evaluate_all().unwrap();
    // Re-point the name at a formula reading A1 (allowed via update path).
    engine
        .graph
        .update_name(
            "N",
            NamedDefinition::Formula {
                ast: parse("=A1+1").unwrap(),
                dependencies: Vec::new(),
                range_deps: Vec::new(),
            },
            NameScope::Workbook,
        )
        .unwrap();

    let sheet_id = engine.sheet_id("Sheet1").unwrap();
    let a1 = *engine
        .graph
        .get_vertex_id_for_address(&CellRef::new(sheet_id, Coord::from_excel(1, 1, true, true)))
        .unwrap();
    let n = engine
        .graph
        .resolve_name_entry("N", sheet_id)
        .expect("name entry")
        .vertex;

    let stamped = engine.evaluate_scc_unit(&[a1, n], None, None).unwrap();
    assert_eq!(stamped, 2, "both the cell and the name member are stamped");
    assert!(is_circ(&engine, "Sheet1", 1, 1));
    assert!(matches!(
        engine.graph.get_value(n),
        Some(LiteralValue::Error(ref e)) if e.kind == ExcelErrorKind::Circ
    ));
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.live_cycles_witnessed, 1);
    assert_eq!(t.circ_cells_stamped, 2);
}

/// Phantom counterpart: the cell's guarded branch never reads the name, so
/// the SCC task produces values for both members.
#[test]
fn named_formula_member_phantom_produces_values_in_scc_task() {
    let mut engine = runtime_engine();
    engine
        .define_name(
            "N",
            NamedDefinition::Literal(LiteralValue::Number(1.0)),
            NameScope::Workbook,
        )
        .unwrap();
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,2,N)");
    engine.evaluate_all().unwrap();
    engine
        .graph
        .update_name(
            "N",
            NamedDefinition::Formula {
                ast: parse("=A1+1").unwrap(),
                dependencies: Vec::new(),
                range_deps: Vec::new(),
            },
            NameScope::Workbook,
        )
        .unwrap();

    let sheet_id = engine.sheet_id("Sheet1").unwrap();
    let a1 = *engine
        .graph
        .get_vertex_id_for_address(&CellRef::new(sheet_id, Coord::from_excel(1, 1, true, true)))
        .unwrap();
    let n = engine
        .graph
        .resolve_name_entry("N", sheet_id)
        .expect("name entry")
        .vertex;

    let stamped = engine.evaluate_scc_unit(&[a1, n], None, None).unwrap();
    assert_eq!(stamped, 0);
    assert_eq!(num(&engine, "Sheet1", 1, 1), 2.0);
    assert_eq!(engine.graph.get_value(n), Some(LiteralValue::Number(3.0)));
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);
}

/* ───────────── 7.14 user value over a formula member ─────────────────── */

#[test]
fn user_value_over_formula_removes_it_from_the_scc() {
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    engine.evaluate_all().unwrap();
    assert_eq!(engine.last_cycle_telemetry().static_sccs, 1);

    // Overwrite A3 with a literal: its formula is removed (engine rule), so
    // no static SCC remains.
    set_value(&mut engine, "Sheet1", 3, 1, LiteralValue::Number(777.0));
    let res = engine.evaluate_all().unwrap();
    assert_eq!(res.cycle_errors, 0);
    assert_eq!(engine.last_cycle_telemetry().static_sccs, 0);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 777.0);
}

/* ─────────────────────────── blast radius ────────────────────────────── */

#[test]
fn blast_radius_only_live_cycle_members_are_stamped() {
    // 10-member static ring C1..C10 (each row r references row r+1 in an
    // untaken branch; C10 closes the ring to C1). Only C5↔C6 is live.
    let mut engine = runtime_engine();
    for r in 1..=10u32 {
        let f = match r {
            5 => "=IF(TRUE,C6,C6)".to_string(), // live edge C5→C6 (both arms)
            6 => "=IF(TRUE,C5,C7)".to_string(), // live edge C6→C5
            10 => "=IF(TRUE,100,C1)".to_string(),
            _ => format!("=IF(TRUE,{},C{})", r * 10, r + 1),
        };
        set_formula(&mut engine, "Sheet1", r, 3, &f);
    }
    let res = engine.evaluate_all().unwrap();

    assert!(is_circ(&engine, "Sheet1", 5, 3), "C5 on the live cycle");
    assert!(is_circ(&engine, "Sheet1", 6, 3), "C6 on the live cycle");
    for r in [1u32, 2, 3, 4, 7, 8, 9] {
        assert_eq!(num(&engine, "Sheet1", r, 3), (r * 10) as f64, "C{r}");
    }
    assert_eq!(num(&engine, "Sheet1", 10, 3), 100.0);

    assert_eq!(res.cycle_errors, 1);
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.static_sccs, 1);
    assert_eq!(t.live_cycles_witnessed, 1);
    assert_eq!(t.circ_cells_stamped, 2, "exactly the live-cycle members");
    assert_eq!(t.phantom_sccs, 0);
}

/* ─────────────────────────── settle mechanics ────────────────────────── */

#[test]
fn engineered_stale_reader_settles_to_exact_values() {
    // A1 (ordered first) live-reads A2 (ordered later): pass 1 sees A2's
    // pre-task value, the settle pass fixes it.
    let mut engine = runtime_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,A2,0)");
    set_formula(&mut engine, "Sheet1", 2, 1, "=IF(TRUE,7,A1)");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 7.0);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 7.0);
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.phantom_sccs, 1);
    assert_eq!(t.settle_passes_total, 2);
    assert_eq!(t.max_passes_single_scc, 2);
}

#[test]
fn branch_flip_during_settle_creating_live_cycle_is_circ() {
    // Pass 1 is acyclic; C1's settle re-eval flips its branch onto C3,
    // closing a live cycle C1↔C3 that only classification-after-settle sees.
    let mut engine = runtime_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(A2=999,A3,7)");
    set_formula(&mut engine, "Sheet1", 2, 1, "=IF(TRUE,999,A1)");
    set_formula(&mut engine, "Sheet1", 3, 1, "=IF(TRUE,A1,8)");
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 1), "A1 joins the live cycle");
    assert!(is_circ(&engine, "Sheet1", 3, 1), "A3 joins the live cycle");
    assert_eq!(num(&engine, "Sheet1", 2, 1), 999.0, "A2 keeps its value");
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.live_cycles_witnessed, 1);
    assert_eq!(t.circ_cells_stamped, 2);
    assert_eq!(t.capped_sccs, 0);
}

/* ───────────────────────────── side effects ──────────────────────────── */

#[test]
fn one_delta_per_member_per_recalc() {
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    let (_res, delta) = engine.evaluate_all_with_delta().unwrap();
    let sheet_id = engine.sheet_id("Sheet1").unwrap();
    let mut expected = vec![
        PackedSheetCell::try_new(sheet_id, 1, 0).unwrap(), // A2 (0-based r1c0)
        PackedSheetCell::try_new(sheet_id, 2, 0).unwrap(), // A3
    ];
    expected.sort_unstable();
    assert_eq!(delta.changed_cells, expected);

    // No changes → no deltas (values persist, the SCC isn't re-stamped).
    let (_res, delta) = engine.evaluate_all_with_delta().unwrap();
    assert!(
        delta.changed_cells.is_empty(),
        "got {:?}",
        delta.changed_cells
    );

    // Guard flip: exactly one delta per member again.
    set_value(&mut engine, "Sheet1", 1, 1, LiteralValue::Boolean(false));
    let (_res, delta) = engine.evaluate_all_with_delta().unwrap();
    assert_eq!(delta.changed_cells, expected);
}

#[test]
fn evaluation_writes_do_not_hit_the_changelog() {
    // G11 confirm: `evaluate_all_logged` records only spill events for
    // computed results; Runtime SCC commits must not add anything either.
    use crate::engine::ChangeLog;
    use crate::engine::graph::editor::change_log::ChangeEvent;

    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    let mut log = ChangeLog::new();
    engine.evaluate_all_logged(&mut log).unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    for ev in log.events() {
        assert!(
            matches!(
                ev,
                ChangeEvent::CompoundStart { .. } | ChangeEvent::CompoundEnd { .. }
            ),
            "unexpected changelog event from SCC evaluation: {ev:?}"
        );
    }
}

#[test]
fn undo_of_the_triggering_edit_restores_pre_recalc_values() {
    let mut engine = runtime_engine();
    let mut undo = UndoEngine::new();

    let (_v, journal) = engine
        .action_atomic_journal("seed".to_string(), |tx| {
            tx.set_cell_value("Sheet1", 1, 1, LiteralValue::Boolean(true))?;
            tx.set_cell_formula("Sheet1", 2, 1, parse("=IF(A1,555,A3)").unwrap())?;
            tx.set_cell_formula("Sheet1", 3, 1, parse("=IF(A1,A2,999)").unwrap())?;
            Ok(())
        })
        .unwrap();
    undo.push_action(journal);
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);

    let (_v, journal) = engine
        .action_atomic_journal("flip".to_string(), |tx| {
            tx.set_cell_value("Sheet1", 1, 1, LiteralValue::Boolean(false))?;
            Ok(())
        })
        .unwrap();
    undo.push_action(journal);
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 999.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 999.0);

    // Undo the flip and recalc: pre-flip values come back.
    engine.undo_action(&mut undo).unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);
}

#[test]
fn cancellation_is_honored_at_settle_pass_boundaries() {
    use crate::args::ArgSchema;
    use crate::function::{FnCaps, Function};
    use crate::traits::{ArgumentHandle, FunctionContext};

    // TRIP() sets the cancel flag as a side effect of pass 1; the SCC task's
    // per-settle-pass check must observe it before re-evaluating.
    #[derive(Debug)]
    struct TripFn(Arc<AtomicBool>);
    impl Function for TripFn {
        fn caps(&self) -> FnCaps {
            FnCaps::empty()
        }
        fn name(&self) -> &'static str {
            "TRIPCANCEL"
        }
        fn arg_schema(&self) -> &'static [ArgSchema] {
            &[]
        }
        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<crate::traits::CalcValue<'b>, formualizer_common::ExcelError> {
            self.0.store(true, Ordering::Relaxed);
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(0)))
        }
    }

    let flag = Arc::new(AtomicBool::new(false));
    let wb = TestWorkbook::new().with_function(Arc::new(TripFn(flag.clone())));
    let mut engine = Engine::new(wb, runtime_cfg());
    // Stale-reader shape: A1 needs a settle pass, A2's pass-1 evaluation
    // trips the flag.
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,A2,0)");
    set_formula(&mut engine, "Sheet1", 2, 1, "=IF(TRUE,7+TRIPCANCEL(),A1)");
    let err = engine
        .evaluate_all_cancellable(crate::engine::CancelToken::from_flag(flag))
        .unwrap_err();
    assert_eq!(err.kind, ExcelErrorKind::Cancelled);
    assert!(
        err.message.as_deref().unwrap_or("").contains("SCC"),
        "cancellation must come from the SCC pass boundary, got {err:?}"
    );
}

/* ───────────────────── G12: INDIRECT inside an SCC ───────────────────── */

#[test]
fn indirect_cycle_through_replan_then_target_change_breaks_it() {
    // A1 = INDIRECT(D1)+1 with D1 → "B1" and B1 = A1+1: the virtual edge
    // closes a 2-vertex SCC (tarjan_scc_with_virtual); both members are
    // live (arithmetic) → #CIRC. (A single-vertex virtual self-edge forms a
    // cycle unit too under Runtime; see
    // `dynamic_self_reference_is_a_circular_reference`.)
    let mut engine = runtime_engine();
    set_value(
        &mut engine,
        "Sheet1",
        1,
        4,
        LiteralValue::Text("B1".to_string()),
    );
    set_formula(&mut engine, "Sheet1", 1, 1, "=INDIRECT(D1)+1");
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1");
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 1));
    assert!(is_circ(&engine, "Sheet1", 1, 2));

    // Re-point the dynamic ref: the outer replan loop drops the virtual
    // edge and both cells evaluate normally.
    set_value(&mut engine, "Sheet1", 3, 1, LiteralValue::Number(10.0)); // A3
    set_value(
        &mut engine,
        "Sheet1",
        1,
        4,
        LiteralValue::Text("A3".to_string()),
    );
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 11.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 12.0);
}

#[test]
fn indirect_in_untaken_branch_is_phantom() {
    // The virtual-dep builder registers INDIRECT targets statically, so the
    // self-edge exists in the schedule; at runtime the branch never executes
    // and the SCC is phantom.
    let mut engine = runtime_engine();
    set_value(
        &mut engine,
        "Sheet1",
        1,
        4,
        LiteralValue::Text("A1".to_string()),
    );
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,1,INDIRECT(D1))");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 1.0);
}

/* ───────────────── recalc-plan dirty quirk under Runtime ─────────────── */

#[test]
fn recalc_plan_skips_clean_cycles_and_evaluates_dirty_ones_whole() {
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    set_formula(&mut engine, "Sheet1", 1, 5, "=1+1"); // unrelated formula E1

    let mut plan = engine.build_recalc_plan().unwrap();
    engine.evaluate_recalc_plan(&plan).unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(engine.last_cycle_telemetry().static_sccs, 1);

    // Dirty only the unrelated formula: the clean SCC must be skipped
    // entirely (values stand, no task runs).
    set_formula(&mut engine, "Sheet1", 1, 5, "=2+2");
    let stale = engine.evaluate_recalc_plan(&plan).unwrap_err();
    assert!(matches!(
        stale.extra,
        formualizer_common::ExcelErrorExtra::PlanStale {
            reason: formualizer_common::PlanStaleReason::Graph
        }
    ));
    plan = engine.build_recalc_plan().unwrap();
    engine.evaluate_recalc_plan(&plan).unwrap();
    assert_eq!(engine.last_cycle_telemetry().static_sccs, 0);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);

    // Dirty a cycle member (via its guard): the whole SCC evaluates.
    set_value(&mut engine, "Sheet1", 1, 1, LiteralValue::Boolean(false));
    engine.evaluate_recalc_plan(&plan).unwrap();
    assert_eq!(engine.last_cycle_telemetry().static_sccs, 1);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 999.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 999.0);
}

/* ──────────────────────── compat & determinism ───────────────────────── */

#[test]
fn static_default_keeps_stamping_the_99_pair() {
    // Golden dual-mode pin: the only documented diff (spec §8) is values vs
    // #CIRC for the guarded phantom pair.
    let mut engine = static_engine();
    build_99_pair(&mut engine, true);
    let res = engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 2, 1));
    assert!(is_circ(&engine, "Sheet1", 3, 1));
    assert_eq!(res.cycle_errors, 1);
    // Telemetry stays default-zero in Static mode.
    assert_eq!(
        engine.last_cycle_telemetry(),
        &crate::engine::CycleTelemetry::default()
    );
}

#[test]
fn dual_mode_corpus_documented_diffs_only() {
    // For each scenario: build identically under both modes; live cycles
    // must be #CIRC in both, phantoms differ exactly as documented.
    type Builder = fn(&mut Engine<TestWorkbook>);
    type Scenario = (Builder, &'static [(u32, u32)], &'static [(u32, u32, f64)]);
    let scenarios: Vec<Scenario> = vec![
        // (build, circ-in-both, runtime-values)
        (
            |e| {
                set_value(e, "Sheet1", 1, 7, LiteralValue::Boolean(true));
                set_value(e, "Sheet1", 1, 2, LiteralValue::Number(1.0));
                set_formula(e, "Sheet1", 5, 1, "=IF(G1,5,SUM(B1:B10))");
                set_formula(e, "Sheet1", 5, 2, "=A5");
            },
            &[][..],
            &[(5, 1, 5.0), (5, 2, 5.0)][..],
        ),
        (
            |e| build_99_pair(e, true),
            &[][..],
            &[(2, 1, 555.0), (3, 1, 555.0)][..],
        ),
        (
            |e| {
                set_formula(e, "Sheet1", 1, 1, "=B1+1");
                set_formula(e, "Sheet1", 1, 2, "=A1+1");
            },
            &[(1, 1), (1, 2)][..],
            &[][..],
        ),
    ];

    for (build, circ_both, runtime_values) in scenarios {
        let mut st = static_engine();
        build(&mut st);
        st.evaluate_all().unwrap();
        let mut rt = runtime_engine();
        build(&mut rt);
        rt.evaluate_all().unwrap();

        for &(r, c) in circ_both {
            assert!(is_circ(&st, "Sheet1", r, c), "static r{r}c{c}");
            assert!(is_circ(&rt, "Sheet1", r, c), "runtime r{r}c{c}");
        }
        for &(r, c, v) in runtime_values {
            assert!(
                is_circ(&st, "Sheet1", r, c),
                "static stamps phantoms r{r}c{c}"
            );
            assert_eq!(num(&rt, "Sheet1", r, c), v, "runtime value r{r}c{c}");
        }
    }
}

#[test]
fn deterministic_across_thread_counts_and_repeats() {
    fn build_and_run(threads: usize) -> (Vec<Option<LiteralValue>>, crate::engine::CycleTelemetry) {
        let cfg = EvalConfig {
            max_threads: Some(threads),
            enable_parallel: threads > 1,
            ..runtime_cfg()
        };
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        // Mixed workbook: phantom pair + live pair + blast-radius ring +
        // downstream readers.
        build_99_pair(&mut engine, true);
        set_formula(&mut engine, "Sheet1", 1, 2, "=B2+1");
        set_formula(&mut engine, "Sheet1", 2, 2, "=B1+1");
        for r in 1..=10u32 {
            let f = match r {
                5 => "=IF(TRUE,C6,C6)".to_string(),
                6 => "=IF(TRUE,C5,C7)".to_string(),
                10 => "=IF(TRUE,100,C1)".to_string(),
                _ => format!("=IF(TRUE,{},C{})", r * 10, r + 1),
            };
            set_formula(&mut engine, "Sheet1", r, 3, &f);
        }
        set_formula(&mut engine, "Sheet1", 1, 4, "=A2+C1"); // downstream
        engine.evaluate_all().unwrap();

        let mut values = Vec::new();
        for r in 1..=10u32 {
            for c in 1..=4u32 {
                values.push(engine.get_cell_value("Sheet1", r, c));
            }
        }
        let mut telemetry = engine.last_cycle_telemetry().clone();
        telemetry.elapsed_ms = 0; // wall clock is the only nondeterministic field
        (values, telemetry)
    }

    let baseline = build_and_run(1);
    for threads in [1usize, 2, 8] {
        for run in 0..2 {
            let out = build_and_run(threads);
            assert_eq!(out, baseline, "threads={threads} run={run}");
        }
    }
}

/* ──────────────── journaled-effects site under Runtime ───────────────── */

#[test]
fn evaluate_all_logged_handles_runtime_cycles_directly() {
    use crate::engine::ChangeLog;

    // Phantom and live cycles through the ChangeLog-threaded path produce
    // identical results to plain evaluate_all (the journal only ever records
    // spill events, which SCC tasks never produce).
    let mut engine = runtime_engine();
    build_99_pair(&mut engine, true);
    set_formula(&mut engine, "Sheet1", 1, 2, "=B2+1");
    set_formula(&mut engine, "Sheet1", 2, 2, "=B1+1");
    let mut log = ChangeLog::new();
    let res = engine.evaluate_all_logged(&mut log).unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 1), 555.0);
    assert_eq!(num(&engine, "Sheet1", 3, 1), 555.0);
    assert!(is_circ(&engine, "Sheet1", 1, 2));
    assert!(is_circ(&engine, "Sheet1", 2, 2));
    assert_eq!(res.cycle_errors, 1);
}

/* ───────── Excel with iteration off: CyclePolicy::RetainLastValue ───────── */

fn retain_engine() -> Engine<TestWorkbook> {
    Engine::new(
        TestWorkbook::new(),
        EvalConfig::default().with_cycle(CycleConfig {
            detection: CycleDetection::Runtime,
            policy: CyclePolicy::RetainLastValue,
        }),
    )
}

#[test]
fn retain_last_value_requires_runtime_detection() {
    let config = CycleConfig {
        detection: CycleDetection::Static,
        policy: CyclePolicy::RetainLastValue,
    };
    assert!(config.validate().is_err());
}

/// Excel cannot calculate a circular formula with iteration off; the cell
/// shows "either a zero or the last calculated value". Members of a live
/// cycle that never calculated hold 0, and formulas reading them calculate
/// from that value.
#[test]
fn retain_last_value_live_cycle_without_prior_values_holds_zero() {
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=B1+1");
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1");
    set_formula(&mut engine, "Sheet1", 1, 3, "=A1+B1+1");
    let res = engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 1.0);
    assert_eq!(res.cycle_errors, 0);
    let t = engine.last_cycle_telemetry();
    assert_eq!(t.live_cycles_witnessed, 1);
    assert_eq!(t.circ_cells_stamped, 0);
}

/// Last calculated values supplied for cells the engine has not calculated
/// (an XLSX file's cached results) are what live-cycle members keep; the
/// rest of the SCC and its dependents calculate from them.
#[test]
fn retain_last_value_keeps_supplied_last_calculated_values() {
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 4, LiteralValue::Number(1.0)); // D1
    set_formula(&mut engine, "Sheet1", 1, 1, "=B1+D1"); // A1
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1"); // B1
    set_formula(&mut engine, "Sheet1", 1, 3, "=A1+B1"); // C1, downstream
    engine.set_last_calculated_value("Sheet1", 1, 1, LiteralValue::Number(5.0));
    engine.set_last_calculated_value("Sheet1", 1, 2, LiteralValue::Number(6.0));
    engine.set_last_calculated_value("Sheet1", 1, 3, LiteralValue::Number(0.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 6.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 11.0);

    // A later recalculation of the cycle keeps the values it holds now.
    set_value(&mut engine, "Sheet1", 1, 4, LiteralValue::Number(2.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 6.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 11.0);
}

/// Microsoft's example: an IF formula calculates until an argument makes it
/// calculate itself, then "Excel retains the value from the last successful
/// calculation". Supplied values only describe the state before the first
/// recalculation, so the formula keeps its own result, not the file's.
#[test]
fn retain_last_value_keeps_the_last_successful_result_when_a_guard_flips() {
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(true)); // G1
    for r in 1..=10u32 {
        if r != 5 {
            set_value(&mut engine, "Sheet1", r, 2, LiteralValue::Number(1.0));
        }
    }
    set_formula(&mut engine, "Sheet1", 5, 1, "=IF(G1,5,SUM(B1:B10))"); // A5
    set_formula(&mut engine, "Sheet1", 5, 2, "=A5"); // B5
    set_formula(&mut engine, "Sheet1", 6, 1, "=A5*10"); // A6, downstream
    engine.set_last_calculated_value("Sheet1", 5, 1, LiteralValue::Number(99.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 1), 5.0, "phantom SCC calculates");
    assert_eq!(num(&engine, "Sheet1", 5, 2), 5.0);
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);

    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(false));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 5, 2), 5.0);
    assert_eq!(num(&engine, "Sheet1", 6, 1), 50.0);
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
}

/// INDEX reads only the cell it selects: `INDEX(B:B,MATCH(A4,A:A,0))` in
/// column B, picking an earlier row, is not a circular reference, so with
/// iteration off it calculates and does not keep a supplied last calculated
/// value. A formula that selects its own cell is circular and keeps it.
#[test]
fn retain_last_value_index_selecting_another_cell_of_its_column_calculates() {
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 2, LiteralValue::Text("ID".into()));
    // Each product gets the next ID on first sight and its first ID after.
    for (row, product) in [(2u32, "p"), (3, "q"), (4, "p"), (5, "r"), (6, "q")] {
        set_value(
            &mut engine,
            "Sheet1",
            row,
            1,
            LiteralValue::Text(product.into()),
        );
        set_formula(
            &mut engine,
            "Sheet1",
            row,
            2,
            &format!(
                "=IF(COUNTIF(A$2:A{row},A{row})=1,MAX(B$1:B{})+1,INDEX(B:B,MATCH(A{row},A:A,0)))",
                row - 1
            ),
        );
        engine.set_last_calculated_value("Sheet1", row, 2, LiteralValue::Number(99.0));
    }
    set_formula(&mut engine, "Sheet1", 7, 2, "=INDEX(B:B,ROW())+1"); // B7, itself
    engine.set_last_calculated_value("Sheet1", 7, 2, LiteralValue::Number(42.0));
    engine.evaluate_all().unwrap();
    let ids: Vec<f64> = (2..=6).map(|row| num(&engine, "Sheet1", row, 2)).collect();
    assert_eq!(ids, vec![1.0, 2.0, 1.0, 3.0, 2.0]);
    assert_eq!(num(&engine, "Sheet1", 7, 2), 42.0);
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
    assert!(engine.kept_last_calculated_value("Sheet1", 7, 2));
    assert!((2..=6).all(|row| !engine.kept_last_calculated_value("Sheet1", row, 2)));

    // Calculated again (the cycle broken), B7 no longer keeps a value.
    set_formula(&mut engine, "Sheet1", 7, 2, "=INDEX(B:B,ROW()-1)+1");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 7, 2), 3.0);
    assert!(!engine.kept_last_calculated_value("Sheet1", 7, 2));
}

/// With iteration off Excel accepts a formula that refers to its own cell
/// (it warns and leaves it uncalculated): the edit is not rejected and the
/// new formula, which has no earlier result, holds 0.
#[test]
fn retain_last_value_accepts_a_direct_self_reference() {
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=A1+1");
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+2");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 2.0);
}

/// A bulk-loaded formula whose range covers its own cell
/// (`MAX(B1:B100)+1` in B6) is a self-reference, as it is when the formula
/// is set interactively (#120): `#CIRC!` under the default policy, the last
/// calculated value with iteration off, and an ordinary value while the
/// read sits in an untaken branch.
#[test]
fn bulk_loaded_range_covering_its_own_cell_is_a_self_reference() {
    use crate::engine::{FormulaIngestBatch, FormulaIngestRecord};

    fn load(engine: &mut Engine<TestWorkbook>, formulas: &[(u32, u32, &str)]) {
        engine.add_sheet("Sheet1").ok();
        for r in 1..=5u32 {
            set_value(engine, "Sheet1", r, 2, LiteralValue::Number(f64::from(r)));
        }
        let records = formulas
            .iter()
            .map(|(row, col, formula)| {
                let ast_id = engine.intern_formula_ast(&parse(formula).unwrap());
                FormulaIngestRecord::new(*row, *col, ast_id, Some(Arc::<str>::from(*formula)))
            })
            .collect();
        engine
            .ingest_formula_batches(vec![FormulaIngestBatch::new("Sheet1", records)])
            .unwrap();
    }
    let formulas = [(6, 2, "=MAX(B1:B100)+1"), (1, 3, "=B6*2")];

    let mut engine = static_engine();
    load(&mut engine, &formulas);
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 6, 2));

    let mut engine = runtime_engine();
    load(&mut engine, &formulas);
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 6, 2));

    let mut engine = retain_engine();
    load(&mut engine, &formulas);
    engine.set_last_calculated_value("Sheet1", 6, 2, LiteralValue::Number(7.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 6, 2), 7.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 14.0);

    let mut engine = retain_engine();
    load(
        &mut engine,
        &[(6, 1, "=\"E\""), (6, 2, "=IF(A6=\"E\",0,MAX(B1:B100)+1)")],
    );
    engine.set_last_calculated_value("Sheet1", 6, 2, LiteralValue::Number(7.0));
    engine.evaluate_all().unwrap();
    assert_eq!(
        num(&engine, "Sheet1", 6, 2),
        0.0,
        "untaken branch: not circular"
    );
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);
}

/// A retained value can flip a branch of another SCC member: the members
/// left to calculate settle from the retained values in their new read
/// order (C1 reads D1 once A1 holds 0, so D1 must calculate first).
#[test]
fn retain_last_value_settles_members_whose_branches_flip() {
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,B1+1,C1)"); // A1
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1"); // B1
    set_formula(&mut engine, "Sheet1", 1, 3, "=IF(A1=0,D1,5)"); // C1
    set_formula(&mut engine, "Sheet1", 1, 4, "=IF(A1=0,9,C1)"); // D1
    set_formula(&mut engine, "Sheet1", 1, 5, "=C1+D1"); // E1, downstream
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 9.0);
    assert_eq!(num(&engine, "Sheet1", 1, 4), 9.0);
    assert_eq!(num(&engine, "Sheet1", 1, 5), 18.0);
}

/// A retained value is the cell's value, not a recomputable cache: moving
/// the cell by a row insertion keeps it. Supplied values move with the cells
/// they were supplied for, so an insertion before the first recalculation
/// hands each to its own formula, not to the cell now at its old address.
#[test]
fn retain_last_value_survives_row_insertion() {
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 2, "=B1+1");
    set_formula(&mut engine, "Sheet1", 1, 3, "=B1*2");
    engine.set_last_calculated_value("Sheet1", 1, 2, LiteralValue::Number(7.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 2), 7.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 14.0);
    engine.insert_rows("Sheet1", 1, 1).unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 2), 7.0);
    assert_eq!(num(&engine, "Sheet1", 2, 3), 14.0);

    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 2, "=B1+1");
    set_formula(&mut engine, "Sheet1", 2, 2, "=B2+1");
    engine.set_last_calculated_value("Sheet1", 1, 2, LiteralValue::Number(7.0));
    engine.set_last_calculated_value("Sheet1", 2, 2, LiteralValue::Number(9.0));
    engine.insert_rows("Sheet1", 1, 1).unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 2, 2), 7.0);
    assert_eq!(num(&engine, "Sheet1", 3, 2), 9.0);
}

/// The last calculated value of a formula is its latest result, whichever
/// calculation produced it: a phantom SCC's ordinary values are what the
/// members keep when the cycle later becomes live, and moving the cells by a
/// row insertion in between neither erases nor replaces that history.
#[test]
fn retain_last_value_keeps_the_latest_result_across_row_insertion() {
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 7, LiteralValue::Boolean(true)); // G1
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(G1,5,B1)"); // A1
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1"); // B1
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 6.0);
    assert_eq!(engine.last_cycle_telemetry().phantom_sccs, 1);

    engine.insert_rows("Sheet1", 1, 1).unwrap();
    set_value(&mut engine, "Sheet1", 2, 7, LiteralValue::Boolean(false)); // G2
    engine.evaluate_all().unwrap();
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 2, 2), 6.0);

    // Columns too, and a later insertion keeps what the first one carried.
    engine.insert_columns("Sheet1", 1, 1).unwrap();
    engine.insert_rows("Sheet1", 1, 1).unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 3, 2), 5.0);
    assert_eq!(num(&engine, "Sheet1", 3, 3), 6.0);
}

/// A cycle that dissolves calculates ordinary results; those, not the values
/// it kept while circular, are what it keeps when it becomes circular again
/// after its cells moved.
#[test]
fn retain_last_value_does_not_revive_values_kept_before_a_later_result() {
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 4, LiteralValue::Text("B1".into())); // D1
    set_value(&mut engine, "Sheet1", 1, 3, LiteralValue::Number(10.0)); // C1
    set_formula(&mut engine, "Sheet1", 1, 1, "=INDIRECT(D1)+1"); // A1
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1"); // B1
    engine.set_last_calculated_value("Sheet1", 1, 1, LiteralValue::Number(7.0));
    engine.set_last_calculated_value("Sheet1", 1, 2, LiteralValue::Number(8.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 7.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 8.0);

    set_value(&mut engine, "Sheet1", 1, 4, LiteralValue::Text("C1".into()));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 11.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 12.0);

    engine.insert_rows("Sheet1", 1, 1).unwrap();
    set_value(&mut engine, "Sheet1", 2, 4, LiteralValue::Text("B2".into())); // D2
    engine.evaluate_all().unwrap();
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 11.0);
    assert_eq!(num(&engine, "Sheet1", 2, 2), 12.0);
}

/// Supplied last calculated values move with their cells on row and column
/// edits (a deleted cell's value goes with it).
#[test]
fn retain_last_value_supplied_values_move_with_their_cells() {
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 3, 2, "=B3+1"); // B3
    set_formula(&mut engine, "Sheet1", 4, 2, "=B4+1"); // B4
    set_formula(&mut engine, "Sheet1", 1, 1, "=A1+1"); // A1, above the edit
    engine.set_last_calculated_value("Sheet1", 3, 2, LiteralValue::Number(7.0));
    engine.set_last_calculated_value("Sheet1", 4, 2, LiteralValue::Number(9.0));
    engine.set_last_calculated_value("Sheet1", 1, 1, LiteralValue::Number(4.0));
    engine.insert_rows("Sheet1", 2, 2).unwrap(); // B3 -> B5, B4 -> B6
    engine.delete_rows("Sheet1", 6, 1).unwrap(); // B6 (was B4) goes
    engine.insert_columns("Sheet1", 1, 1).unwrap(); // B5 -> C5, A1 -> B1
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 3), 7.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 4.0);
    assert!(matches!(
        engine.get_cell_value("Sheet1", 6, 3),
        None | Some(LiteralValue::Empty)
    ));
}

/// Calculating an unrelated cell does not calculate a circular formula, so
/// the value supplied for it still applies to the next full calculation.
#[test]
fn retain_last_value_targeted_calculation_keeps_untouched_supplied_values() {
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=A1+1"); // A1
    set_formula(&mut engine, "Sheet1", 1, 2, "=2"); // B1
    set_formula(&mut engine, "Sheet1", 1, 3, "=A1+1"); // C1
    engine.set_last_calculated_value("Sheet1", 1, 1, LiteralValue::Number(7.0));
    engine.evaluate_cell("Sheet1", 1, 2).unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 7.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 8.0);
}

/// A name whose range covers the formula's own cell is the same circular
/// reference reached through a name: with iteration off it is accepted
/// (Excel warns), the formula keeps its last calculated value (0 here), and
/// a read in an untaken branch is not circular.
#[test]
fn retain_last_value_accepts_named_self_references() {
    fn cover(engine: &mut Engine<TestWorkbook>) {
        let sheet_id = engine.sheet_id("Sheet1").unwrap();
        let nr = RangeRef::new(
            CellRef::new(sheet_id, Coord::from_excel(1, 1, true, true)),
            CellRef::new(sheet_id, Coord::from_excel(10, 1, true, true)),
        );
        engine
            .define_name("COVER", NamedDefinition::Range(nr), NameScope::Workbook)
            .unwrap();
        for r in 1..=4u32 {
            set_value(engine, "Sheet1", r, 1, LiteralValue::Number(1.0));
        }
    }
    let mut engine = retain_engine();
    cover(&mut engine);
    set_formula(&mut engine, "Sheet1", 5, 1, "=SUM(COVER)");
    set_formula(&mut engine, "Sheet1", 5, 2, "=A5+1");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 5, 2), 1.0);
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);

    let mut engine = retain_engine();
    cover(&mut engine);
    set_formula(&mut engine, "Sheet1", 5, 1, "=IF(TRUE,7,SUM(COVER))");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 1), 7.0);

    // A name defined after the formula follows the same rule.
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 5, 1, "=IF(TRUE,7,SUM(COVER))");
    cover(&mut engine);
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 1), 7.0);
}

/// Circularity includes references found while calculating: `INDIRECT` of
/// the formula's own cell is a self-reference like `=A1+1`.
#[test]
fn dynamic_self_reference_is_a_circular_reference() {
    let mut engine = runtime_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=INDIRECT(\"A1\")+1");
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 1));
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);

    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=INDIRECT(\"A1\")+1");
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 1.0);
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);

    // Read only in an untaken branch, it is not circular.
    let mut engine = retain_engine();
    set_formula(&mut engine, "Sheet1", 1, 1, "=IF(TRUE,5,INDIRECT(\"A1\"))");
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
}

/// An argument a function reads only as a reference (ROWS's, COLUMNS's,
/// CELL("row",...)'s) is not a read of its cells, also in a formula with a
/// dynamic reference: `=INDIRECT("B1")+ROWS(B$1:B5)` in B5 is not circular,
/// so it calculates instead of keeping its last calculated value. A read of
/// the cell's contents is circular.
#[test]
fn reference_only_arguments_of_a_dynamic_formula_are_not_circular() {
    for (formula, expected) in [
        ("=INDIRECT(\"B1\")+ROWS(B$1:B5)", 12.0),
        ("=OFFSET($B$1,0,0)+COLUMNS(A5:B5)", 9.0),
        ("=INDIRECT(\"B1\")+CELL(\"row\",B5)", 12.0),
    ] {
        let mut engine = runtime_engine();
        set_value(&mut engine, "Sheet1", 1, 2, LiteralValue::Number(7.0));
        set_formula(&mut engine, "Sheet1", 5, 2, formula);
        engine.evaluate_all().unwrap();
        assert_eq!(num(&engine, "Sheet1", 5, 2), expected, "{formula}");

        let mut engine = retain_engine();
        set_value(&mut engine, "Sheet1", 1, 2, LiteralValue::Number(7.0));
        set_formula(&mut engine, "Sheet1", 5, 2, formula);
        engine.set_last_calculated_value("Sheet1", 5, 2, LiteralValue::Number(-1.0));
        engine.evaluate_all().unwrap();
        assert_eq!(num(&engine, "Sheet1", 5, 2), expected, "{formula}");
        assert_eq!(
            engine.last_cycle_telemetry().live_cycles_witnessed,
            0,
            "{formula}"
        );
    }

    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 2, LiteralValue::Number(7.0));
    set_formula(&mut engine, "Sheet1", 5, 2, "=INDIRECT(\"B1\")+SUM(B$1:B5)");
    engine.set_last_calculated_value("Sheet1", 5, 2, LiteralValue::Number(-1.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 5, 2), -1.0);
}

/// A circular reference whose `INDIRECT` text is itself calculated is only
/// found once that text is known, after the formulas on it calculated once
/// without knowing they are circular. That one-pass result is not a
/// successful calculation: the members keep their last calculated values
/// (0 for formulas that never calculated, else the supplied values).
#[test]
fn circular_reference_found_through_a_calculated_reference_keeps_last_values() {
    fn build(engine: &mut Engine<TestWorkbook>) {
        set_formula(engine, "Sheet1", 1, 4, "=\"B1\""); // D1
        set_formula(engine, "Sheet1", 1, 1, "=INDIRECT(D1)+1"); // A1
        set_formula(engine, "Sheet1", 1, 2, "=A1+1"); // B1
        set_formula(engine, "Sheet1", 1, 3, "=B1*10"); // C1, downstream
    }
    let mut engine = retain_engine();
    build(&mut engine);
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 0.0);
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);

    let mut engine = retain_engine();
    build(&mut engine);
    engine.set_last_calculated_value("Sheet1", 1, 1, LiteralValue::Number(5.0));
    engine.set_last_calculated_value("Sheet1", 1, 2, LiteralValue::Number(6.0));
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 6.0);
    assert_eq!(num(&engine, "Sheet1", 1, 3), 60.0);

    // A formula that calculated before keeps that result when an edit
    // makes it circular through a calculated reference.
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 5, LiteralValue::Text("C5".into())); // E1
    set_value(&mut engine, "Sheet1", 5, 3, LiteralValue::Number(10.0)); // C5
    set_formula(&mut engine, "Sheet1", 1, 4, "=E1&\"\""); // D1
    set_formula(&mut engine, "Sheet1", 1, 1, "=INDIRECT(D1)+1"); // A1
    set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1"); // B1
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 11.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 12.0);
    set_value(&mut engine, "Sheet1", 1, 5, LiteralValue::Text("B1".into()));
    engine.evaluate_all().unwrap();
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
    assert_eq!(num(&engine, "Sheet1", 1, 1), 11.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 12.0);

    // Under the default policy the same cycle is #CIRC!.
    let mut engine = runtime_engine();
    build(&mut engine);
    engine.evaluate_all().unwrap();
    assert!(is_circ(&engine, "Sheet1", 1, 1));
    assert!(is_circ(&engine, "Sheet1", 1, 2));
}

/// When a dynamic reference resolves only after its formula first
/// calculated (its target calculates later in the same request), the
/// formula calculates again — and so does every formula that reads it.
#[test]
fn formulas_reading_a_recalculated_dynamic_reference_calculate_again() {
    for cfg in [EvalConfig::default(), runtime_cfg()] {
        let mut engine = Engine::new(TestWorkbook::new(), cfg);
        set_value(&mut engine, "Sheet1", 1, 24, LiteralValue::Number(1.0)); // X1
        set_formula(&mut engine, "Sheet1", 2, 24, "=X1+1"); // X2
        set_formula(&mut engine, "Sheet1", 3, 24, "=X2+1"); // X3
        set_formula(&mut engine, "Sheet1", 5, 3, "=X3+7"); // C5 = 10
        set_formula(&mut engine, "Sheet1", 1, 4, "=\"C5\""); // D1
        set_formula(&mut engine, "Sheet1", 1, 1, "=INDIRECT(D1)+1"); // A1
        set_formula(&mut engine, "Sheet1", 1, 2, "=A1+1"); // B1
        engine.evaluate_all().unwrap();
        assert_eq!(num(&engine, "Sheet1", 1, 1), 11.0);
        assert_eq!(num(&engine, "Sheet1", 1, 2), 12.0);
    }
}

/// Bulk-load `formulas` (with the range self-loops bulk ingest records) and
/// declare the legacy (CSE) array formulas `arrays` as `(row, col, rows)`.
fn load_with_arrays(
    engine: &mut Engine<TestWorkbook>,
    formulas: &[(u32, u32, &str)],
    arrays: &[(u32, u32, u32)],
) {
    use crate::engine::{FormulaIngestBatch, FormulaIngestRecord};
    engine.add_sheet("Sheet1").ok();
    let records = formulas
        .iter()
        .map(|(row, col, formula)| {
            let ast_id = engine.intern_formula_ast(&parse(formula).unwrap());
            FormulaIngestRecord::new(*row, *col, ast_id, Some(Arc::<str>::from(*formula)))
        })
        .collect();
    engine
        .ingest_formula_batches(vec![FormulaIngestBatch::new("Sheet1", records)])
        .unwrap();
    engine.use_legacy_array_semantics();
    for &(row, col, rows) in arrays {
        engine.declare_array_formula("Sheet1", row, col, rows, 1, false);
    }
}

/// An array result is not by itself a circular reference: an array formula
/// whose range covers its own cells only in an untaken branch calculates in
/// every cell of its area, as Excel does.
#[test]
fn retain_last_value_array_formula_with_untaken_self_read_calculates() {
    let mut engine = retain_engine();
    load_with_arrays(
        &mut engine,
        &[(1, 1, "=IF(TRUE,7,SUM(A1:A100))"), (1, 2, "=A2*10")],
        &[(1, 1, 2)],
    );
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 7.0);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 7.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 70.0);
    assert_eq!(engine.last_cycle_telemetry().circ_cells_stamped, 0);
}

/// An array formula on a live circular reference keeps the last calculated
/// values of all its cells (0 where it never calculated), and formulas that
/// read them calculate from those values.
#[test]
fn retain_last_value_circular_array_formula_keeps_its_cells_values() {
    let formulas = [(1, 1, "={1;2}+SUM(A1:A100)"), (1, 2, "=A2*10")];
    let mut engine = retain_engine();
    load_with_arrays(&mut engine, &formulas, &[(1, 1, 2)]);
    engine.set_last_calculated_value("Sheet1", 1, 1, LiteralValue::Number(5.0));
    engine.set_last_calculated_value("Sheet1", 2, 1, LiteralValue::Number(6.0));
    engine.evaluate_all().unwrap();
    assert!(engine.last_cycle_telemetry().live_cycles_witnessed >= 1);
    assert_eq!(engine.last_cycle_telemetry().circ_cells_stamped, 0);
    assert_eq!(num(&engine, "Sheet1", 1, 1), 5.0);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 6.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 60.0);

    let mut engine = retain_engine();
    load_with_arrays(&mut engine, &formulas, &[(1, 1, 2)]);
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 1, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 2, 1), 0.0);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 0.0);
}

/// A dynamic array that spilled keeps its spilled values when a later edit
/// makes it circular.
#[test]
fn retain_last_value_spilled_array_keeps_its_spill_when_it_becomes_circular() {
    let mut engine = retain_engine();
    set_value(&mut engine, "Sheet1", 1, 3, LiteralValue::Number(3.0)); // C1
    set_formula(&mut engine, "Sheet1", 1, 2, "=SEQUENCE(C1)"); // B1:B3
    engine.evaluate_all().unwrap();
    assert_eq!(num(&engine, "Sheet1", 3, 2), 3.0);

    set_formula(&mut engine, "Sheet1", 1, 3, "=B1+2"); // C1 closes the cycle
    engine.evaluate_all().unwrap();
    assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 1);
    assert_eq!(num(&engine, "Sheet1", 1, 2), 1.0);
    assert_eq!(num(&engine, "Sheet1", 2, 2), 2.0);
    assert_eq!(num(&engine, "Sheet1", 3, 2), 3.0);
}
