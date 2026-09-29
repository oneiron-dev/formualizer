//! A formula that reads a cell of another formula's spill sees the spilled
//! value, even when it was scheduled before the spill landed.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

#[test]
fn readers_of_spilled_cells_see_the_spill() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_formula("Sheet1", 1, 2, parse("=A2*10").unwrap())
        .unwrap();
    engine
        .set_cell_value("Sheet1", 5, 5, LiteralValue::Number(100.0))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 1, parse("=SEQUENCE(3)+E5").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 3, parse("=SUM(A1:A3)+A3").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 2),
        Some(LiteralValue::Number(1020.0))
    );
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 3),
        Some(LiteralValue::Number(409.0))
    );
    engine
        .set_cell_value("Sheet1", 5, 5, LiteralValue::Number(0.0))
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 2),
        Some(LiteralValue::Number(20.0))
    );
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 3),
        Some(LiteralValue::Number(9.0))
    );
}
