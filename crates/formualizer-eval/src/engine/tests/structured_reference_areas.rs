//! Named-table structured references that combine row items (#Headers,
//! #Data, #Totals, #All, #This Row) with columns resolve to their A1 areas.

use crate::engine::{Engine, EvalConfig};
use crate::reference::{CellRef, Coord, RangeRef};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// Sales at A1:B5: header row (Item, Qty), data a/1 b/2 c/3, totals row.
fn engine() -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let text = |s: &str| LiteralValue::Text(s.into());
    for (row, item, qty) in [
        (1, text("Item"), text("Qty")),
        (2, text("a"), LiteralValue::Number(1.0)),
        (3, text("b"), LiteralValue::Number(2.0)),
        (4, text("c"), LiteralValue::Number(3.0)),
        (5, text("Total"), LiteralValue::Number(6.0)),
    ] {
        engine.set_cell_value("Sheet1", row, 1, item).unwrap();
        engine.set_cell_value("Sheet1", row, 2, qty).unwrap();
    }
    let sheet = engine.sheet_id("Sheet1").unwrap();
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 1, true, true)),
        CellRef::new(sheet, Coord::from_excel(5, 2, true, true)),
    );
    engine
        .define_table(
            "Sales",
            range,
            true,
            vec!["Item".into(), "Qty".into()],
            true,
        )
        .unwrap();
    engine
}

fn eval(engine: &mut Engine<TestWorkbook>, row: u32, col: u32, formula: &str) -> LiteralValue {
    engine
        .set_cell_formula("Sheet1", row, col, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_cell("Sheet1", row, col).unwrap();
    engine.get_cell_value("Sheet1", row, col).unwrap()
}

#[test]
fn row_items_combine_with_columns() {
    let mut engine = engine();
    let n = LiteralValue::Number;
    assert_eq!(
        eval(&mut engine, 3, 3, "=Sales[[#This Row],[Qty]]*10"),
        n(20.0)
    );
    assert_eq!(
        eval(
            &mut engine,
            4,
            3,
            "=COUNTA(Sales[[#This Row],[Item]:[Qty]])"
        ),
        n(2.0)
    );
    assert_eq!(
        eval(&mut engine, 1, 5, "=Sales[[#Headers],[Qty]]"),
        LiteralValue::Text("Qty".into())
    );
    assert_eq!(
        eval(&mut engine, 2, 5, "=SUM(Sales[[#Data],[Qty]])"),
        n(6.0)
    );
    assert_eq!(eval(&mut engine, 3, 5, "=Sales[[#Totals],[Qty]]"), n(6.0));
    assert_eq!(
        eval(&mut engine, 4, 5, "=ROWS(Sales[[#All],[Item]])"),
        n(5.0)
    );
    assert_eq!(
        eval(&mut engine, 5, 5, "=ROWS(Sales[[#Headers],[#Data],[Qty]])"),
        n(4.0)
    );
    assert_eq!(eval(&mut engine, 6, 5, "=SUM(Sales)"), n(6.0));
}

#[test]
fn this_row_outside_the_table_body_is_a_value_error() {
    let mut engine = engine();
    let error = engine
        .set_cell_formula("Sheet1", 9, 3, parse("=Sales[[#This Row],[Qty]]").unwrap())
        .unwrap_err();
    assert_eq!(error.kind, ExcelErrorKind::Value);
}
