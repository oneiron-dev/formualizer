//! Excel's arithmetic near zero (Microsoft, "Floating-point arithmetic may
//! give inaccurate results in Excel"): a formula's last addition or
//! subtraction that lands within binary conversion error of zero is exactly 0
//! ("Example when a value reaches zero"), as is SUM's last addition of a
//! number read from a cell, and results too small for a normal double
//! underflow to 0, as Excel has no denormalized numbers.

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

#[test]
fn sum_compensates_its_last_addition_from_a_cell() {
    for config in configs() {
        let engine = engine_with(
            config,
            &[
                // A corpus row: SUM(A1:D1) with B1 and D1 empty is 0 in Excel.
                (1, 1, "-40411178.260000005"),
                (1, 3, "40411178.26"),
                (2, 1, "2.558"),
                (3, 1, "-1.333"),
                (4, 1, "-1.225"),
                (2, 2, "26"),
                (3, 2, "-21.99"),
                (4, 2, "-4.01"),
                // Row by row, H1:I2 adds H2 last.
                (1, 8, "-40411178.260000005"),
                (1, 9, "0"),
                (2, 8, "40411178.26"),
                (1, 6, "=SUM(A1:D1)"),
                (2, 6, "=SUM(A2:A4)"),
                (3, 6, "=SUM(A2,A3,A4)"),
                (4, 6, "=SUM(A2,A3,A4,0)"),
                (5, 6, "=SUM(B2:B4)"),
                (6, 6, "=SUM(A2:A4)*2"),
                (7, 6, "=SUM(OFFSET(A2,0,0,3,1))"),
                (8, 6, "=SUM(A2:A4,B2:B4)"),
                (9, 6, "=SUM(H1:I2)"),
                // An intersection of references, and a name LET binds to a
                // reference, are references.
                (10, 6, "=SUM(A2:A4 A1:A4)"),
                (11, 6, "=LET(area,A2:A4,SUM(area))"),
            ],
        );
        for row in 1..=11 {
            let n = number(&engine, row, 6);
            assert_eq!(n, 0.0, "row {row}");
            assert!(n.is_sign_positive(), "row {row}");
        }
    }
}

#[test]
fn sum_keeps_residues_of_values_and_of_earlier_additions() {
    for config in configs() {
        let engine = engine_with(
            config,
            &[
                (1, 1, "2.558"),
                (2, 1, "-1.333"),
                (3, 1, "-1.225"),
                (1, 2, "-40411178.260000005"),
                (2, 2, "40411178.26"),
                (3, 2, "0"),
                (4, 2, "5"),
                // Row by row, D1:E2 adds the 0 in D2 last.
                (1, 4, "-40411178.260000005"),
                (1, 5, "40411178.26"),
                (2, 4, "0"),
                (1, 6, "=SUM(2.558,-1.333,-1.225)"),
                (2, 6, "=SUM(A1,--A2,--A3)"),
                (3, 6, "=SUM(2.558-1.333,-1.225)"),
                (4, 6, "=SUM({2.558,-1.333,-1.225})"),
                (5, 6, "=SUM(A1:A3*1)"),
                (6, 6, "=SUM(B1:B3)"),
                (7, 6, "=SUM(D1:E2)"),
                (8, 6, "=SUM(B1:B4)"),
                (9, 6, "=SUM(123.45,56.78,-180.23)"),
            ],
        );
        let residue = 2.558 - 1.333 - 1.225;
        assert_ne!(residue, 0.0);
        for row in 1..=5 {
            assert_eq!(number(&engine, row, 6), residue, "row {row}");
        }
        let large = -40411178.260000005 + 40411178.26;
        assert_ne!(large, 0.0);
        assert_eq!(number(&engine, 6, 6), large);
        assert_eq!(number(&engine, 7, 6), large);
        assert_eq!(number(&engine, 8, 6), large + 5.0);
        assert_eq!(number(&engine, 9, 6), 123.45 + 56.78 - 180.23);
    }
}

#[test]
fn sum_keeps_a_residual_a_later_cell_leaves_uncancelled() {
    // Microsoft Q&A 4775315 (Excel for Windows): with 1.333, 1.225, -1.333,
    // -1.225 and 0 in A1:A5, SUM(A1,A2,A3,A4) and SUM(A1,A2,A3,A4,0) are 0,
    // SUM(A1,A2,A3,A4,A5) and SUM(A1:A5) about -2.22E-16, SUM(A1,A2,A3,A5,A4)
    // 0 and SUM(1.333,1.225,-1.333,-1.225) about -2.22E-16. In the question,
    // 1.75, 0.72, -2.47 and 0 sum to -4.44E-16, to 0 with the 0 cell emptied
    // and to 0 again with 1 and -1 below the 0.
    for config in configs() {
        let engine = engine_with(
            config,
            &[
                (1, 1, "1.333"),
                (2, 1, "1.225"),
                (3, 1, "-1.333"),
                (4, 1, "-1.225"),
                (5, 1, "0"),
                (1, 2, "1.75"),
                (2, 2, "0.72"),
                (3, 2, "-2.47"),
                (4, 2, "0"),
                (5, 2, "1"),
                (6, 2, "-1"),
                (1, 3, "1.75"),
                (2, 3, "0.72"),
                (3, 3, "-2.47"),
                (1, 6, "=SUM(A1,A2,A3,A4)"),
                (2, 6, "=SUM(A1,A2,A3,A4,0)"),
                (3, 6, "=SUM(A1,A2,A3,A5,A4)"),
                (4, 6, "=SUM(A1:A4)"),
                (5, 6, "=SUM(C1:C4)"),
                (6, 6, "=SUM(B1:B6)"),
                (7, 6, "=SUM(A1,A2,A3,A4,A5)"),
                (8, 6, "=SUM(A1:A5)"),
                (9, 6, "=SUM(1.333,1.225,-1.333,-1.225)"),
                (10, 6, "=SUM(B1:B4)"),
            ],
        );
        for row in 1..=6 {
            let n = number(&engine, row, 6);
            assert_eq!(n, 0.0, "row {row}");
            assert!(n.is_sign_positive(), "row {row}");
        }
        let residue = 1.333 + 1.225 - 1.333 - 1.225;
        assert_eq!(residue, -2.220446049250313e-16);
        for row in 7..=9 {
            assert_eq!(number(&engine, row, 6), residue, "row {row}");
        }
        let residue = 1.75 + 0.72 - 2.47;
        assert_eq!(residue, -4.440892098500626e-16);
        assert_eq!(number(&engine, 10, 6), residue);
    }
}

#[test]
fn sum_adds_in_order_and_compensates_only_the_last_addition() {
    // A budget column reported on MrExcel: Excel's SUM shows 5.68434E-14 with the
    // trailing 0 and 0 without it. Adding in order, the cancellation comes
    // at -392.17; with the 0 it is no longer the last addition.
    let budget = [
        "-140.92", "280", "-143.16", "280", "280", "-163.75", "-392.17", "0",
    ];
    for config in configs() {
        let mut cells: Vec<(u32, u32, &str)> = budget
            .iter()
            .enumerate()
            .map(|(i, v)| (i as u32 + 1, 1, *v))
            .collect();
        cells.push((1, 3, "=SUM(A1:A8)"));
        cells.push((2, 3, "=SUM(A1:A7)"));
        let engine = engine_with(config, &cells);
        let residue = budget[..7]
            .iter()
            .fold(0.0, |total, v| total + v.parse::<f64>().unwrap());
        assert_eq!(residue, 5.684341886080802e-14);
        assert_eq!(number(&engine, 1, 3), residue);
        assert_eq!(number(&engine, 2, 3), 0.0);
    }
}

#[test]
fn formula_plane_spans_compensate_sum() {
    let cfg =
        EvalConfig::default().with_formula_plane_mode(FormulaPlaneMode::AuthoritativeExperimental);
    let mut engine = Engine::new(TestWorkbook::default(), cfg);
    for row in 1..=120 {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(0.5 - 0.4))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(-0.1))
            .unwrap();
    }
    let mut formulas = Vec::new();
    for (col, template) in [
        (3, "=SUM(A{row}:B{row})"),
        (4, "=2*SUM(A{row}:B{row})"),
        (5, "=SUM(A{row},-B{row}*-1)"),
    ] {
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
    let residue = (0.5 - 0.4) + -0.1;
    assert_ne!(residue, 0.0);
    for row in [1, 60, 120] {
        assert_eq!(number(&engine, row, 3), 0.0);
        assert_eq!(number(&engine, row, 4), 0.0);
        assert_eq!(number(&engine, row, 5), residue);
    }
}
