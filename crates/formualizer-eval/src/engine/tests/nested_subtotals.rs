//! SUBTOTAL and AGGREGATE (options 0-3) skip cells whose formulas are
//! themselves SUBTOTAL or AGGREGATE, so subtotals are not counted twice.

use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

use crate::engine::{Engine, EvalConfig, FormulaPlaneMode};
use crate::test_workbook::TestWorkbook;

fn evaluate(mode: FormulaPlaneMode, formulas: &[&str]) -> Vec<Option<LiteralValue>> {
    let mut engine = Engine::new(
        TestWorkbook::default(),
        EvalConfig::default().with_formula_plane_mode(mode),
    );
    // A1:A4 = 10..40, A5 subtotals them, B1:B4 = 1..4 with B5 a subtotal
    // written inside a larger expression.
    for (row, value) in [(1, 10.0), (2, 20.0), (3, 30.0), (4, 40.0)] {
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(value))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(value / 10.0))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 5, 1, parse("=SUBTOTAL(9,A1:A4)").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 5, 2, parse("=1+AGGREGATE(9,0,B1:B4)").unwrap())
        .unwrap();
    for (i, f) in formulas.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", i as u32 + 1, 4, parse(f).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    (0..formulas.len())
        .map(|i| engine.get_cell_value("Sheet1", i as u32 + 1, 4))
        .collect()
}

#[test]
fn nested_subtotals_are_not_counted_twice() {
    let formulas = [
        "=SUBTOTAL(9,A1:A5)",
        "=SUBTOTAL(109,A1:A5)",
        "=SUBTOTAL(2,A1:B5)",
        "=AGGREGATE(9,0,A1:A5)",
        "=AGGREGATE(9,4,A1:A5)",
        "=SUBTOTAL(9,B1:B5)",
        "=SUM(A1:A5)",
    ];
    let expected = [100.0, 100.0, 8.0, 100.0, 200.0, 10.0, 200.0];
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let got = evaluate(mode, &formulas);
        for ((formula, want), value) in formulas.iter().zip(expected).zip(got) {
            assert_eq!(
                value,
                Some(LiteralValue::Number(want)),
                "{formula} in {mode:?} mode"
            );
        }
    }
}
