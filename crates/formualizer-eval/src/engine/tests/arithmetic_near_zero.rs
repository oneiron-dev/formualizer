//! Excel's arithmetic near zero (Microsoft, "Floating-point arithmetic may
//! give inaccurate results in Excel"): a formula's last addition or
//! subtraction that lands within binary conversion error of zero is exactly 0
//! ("Example when a value reaches zero"), and results too small for a normal
//! double underflow to 0, as Excel has no denormalized numbers.

use std::sync::Arc;

use crate::engine::{
    Engine, EvalConfig, FormulaIngestBatch, FormulaIngestRecord, FormulaPlaneMode,
};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

/// Sets `(row, col, content)` cells (a leading `=` makes a formula) and
/// evaluates the sheet.
fn engine_with(config: EvalConfig, cells: &[(u32, u32, &str)]) -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), config);
    for &(row, col, content) in cells {
        if content.starts_with('=') {
            engine
                .set_cell_formula("Sheet1", row, col, parse(content).unwrap())
                .unwrap();
        } else {
            engine
                .set_cell_value(
                    "Sheet1",
                    row,
                    col,
                    LiteralValue::Number(content.parse().unwrap()),
                )
                .unwrap();
        }
    }
    engine.evaluate_all().unwrap();
    engine
}

fn number(engine: &Engine<TestWorkbook>, row: u32, col: u32) -> f64 {
    match engine.get_cell_value("Sheet1", row, col) {
        Some(LiteralValue::Number(n)) => n,
        other => panic!("R{row}C{col}: expected a number, got {other:?}"),
    }
}

fn configs() -> [EvalConfig; 2] {
    [EvalConfig::default(), super::common::arrow_eval_config()]
}

#[test]
fn final_add_or_subtract_at_zero_is_exactly_zero() {
    for config in configs() {
        let engine = engine_with(
            config,
            &[
                (1, 1, "-40411178.260000005"),
                (2, 1, "40411178.26"),
                (1, 2, "=1.333+1.225-1.333-1.225"),
                (2, 2, "=0.5-0.4-0.1"),
                (3, 2, "=0.1+0.2-0.3"),
                (4, 2, "=A1+A2"),
                (5, 2, "=-A2-A1"),
                (6, 2, "=(4/3-1)*3-1"),
                (7, 2, "=SUM(1.333,1.225)-1.333-1.225"),
            ],
        );
        for row in 1..=7 {
            let n = number(&engine, row, 2);
            assert_eq!(n, 0.0, "row {row}");
            assert!(n.is_sign_positive(), "row {row}");
        }
    }
}

#[test]
fn intermediate_and_larger_residues_are_kept() {
    for config in configs() {
        let engine = engine_with(
            config,
            &[
                (1, 1, "14.860000000000014"),
                (2, 1, "14.86"),
                (1, 2, "=1*(0.5-0.4-0.1)"),
                (2, 2, "=0.5-0.4-0.1+0"),
                (3, 2, "=IF(TRUE,0.5-0.4-0.1)"),
                (4, 2, "=A1-A2"),
                (5, 2, "=1-0.999"),
                (6, 2, "=2E-20-1E-20"),
                (7, 2, "=(43.1-43.2)+1"),
            ],
        );
        let residue = 0.5 - 0.4 - 0.1;
        assert_ne!(residue, 0.0);
        assert_eq!(number(&engine, 1, 2), residue);
        assert_eq!(number(&engine, 2, 2), residue);
        assert_eq!(number(&engine, 3, 2), residue);
        // An eight-unit residue is more than conversion error.
        assert_eq!(number(&engine, 4, 2), 14.860000000000014 - 14.86);
        assert_eq!(number(&engine, 5, 2), 1.0 - 0.999);
        assert_eq!(number(&engine, 6, 2), 1e-20);
        // Microsoft's "Adding a negative number": 0.899999999999999.
        assert_eq!(number(&engine, 7, 2), (43.1 - 43.2) + 1.0);
    }
}

#[test]
fn formula_plane_spans_compensate_the_final_operation() {
    let cfg =
        EvalConfig::default().with_formula_plane_mode(FormulaPlaneMode::AuthoritativeExperimental);
    let mut engine = Engine::new(TestWorkbook::default(), cfg);
    for row in 1..=120 {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(0.5 - 0.4))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(0.1))
            .unwrap();
    }
    let mut formulas = Vec::new();
    for (col, template) in [(3, "=A{row}-B{row}"), (4, "=2*(A{row}-B{row})")] {
        for row in 1..=120 {
            let formula = template.replace("{row}", &row.to_string());
            let ast_id = engine.intern_formula_ast(&parse(&formula).unwrap());
            formulas.push(FormulaIngestRecord::new(
                row,
                col,
                ast_id,
                Some(Arc::<str>::from(formula.as_str())),
            ));
        }
    }
    engine
        .ingest_formula_batches(vec![FormulaIngestBatch::new("Sheet1", formulas)])
        .unwrap();
    assert!(engine.baseline_stats().formula_plane_active_span_count > 0);
    engine.evaluate_all().unwrap();
    for row in [1, 60, 120] {
        assert_eq!(number(&engine, row, 3), 0.0);
        assert_eq!(number(&engine, row, 4), 2.0 * (0.5 - 0.4 - 0.1));
    }
}

#[test]
fn results_below_the_smallest_normal_number_underflow_to_zero() {
    for config in configs() {
        let engine = engine_with(
            config,
            &[
                (1, 2, "=1E-307/100"),
                (2, 2, "=POWER(10,-309)"),
                (3, 2, "=10^-309"),
                (4, 2, "=1E-200*1E-120"),
                (5, 2, "=-1E-307/100"),
                (6, 2, "=1E-300/1E8"),
                (7, 2, "=1E-300/1E7"),
                (8, 2, "=1E-307*1"),
            ],
        );
        for row in 1..=6 {
            let n = number(&engine, row, 2);
            assert_eq!(n, 0.0, "row {row}");
            assert!(n.is_sign_positive(), "row {row}");
        }
        assert_eq!(number(&engine, 7, 2), 1e-300 / 1e7);
        assert_eq!(number(&engine, 8, 2), 1e-307);
    }
}
