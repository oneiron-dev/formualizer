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
fn index_and_offset_read_structured_references() {
    let mut engine = engine();
    let n = LiteralValue::Number;
    let text = |s: &str| LiteralValue::Text(s.into());
    // Each structured reference is its A1 area: Sales[Qty] is B2:B4,
    // Sales and Sales[#Data] are A2:B4, Sales[#All] is A1:B5.
    assert_eq!(eval(&mut engine, 1, 7, "=INDEX(Sales[Qty],2)"), n(2.0));
    assert_eq!(eval(&mut engine, 2, 7, "=INDEX(Sales,3,1)"), text("c"));
    assert_eq!(eval(&mut engine, 3, 7, "=INDEX(Sales[#Data],1,2)"), n(1.0));
    assert_eq!(
        eval(&mut engine, 4, 7, "=INDEX(Sales[#All],1,2)"),
        text("Qty")
    );
    assert_eq!(
        eval(&mut engine, 5, 7, "=INDEX(Sales[[Item]:[Qty]],2,2)"),
        n(2.0)
    );
    assert_eq!(eval(&mut engine, 6, 7, "=SUM(INDEX(Sales,,2))"), n(6.0));
    assert_eq!(eval(&mut engine, 7, 7, "=ROWS(INDEX(Sales,,2))"), n(3.0));
    assert_eq!(eval(&mut engine, 8, 7, "=SUM(INDEX(Sales[Qty],0))"), n(6.0));
    assert_eq!(
        eval(&mut engine, 9, 7, "=SUMIFS(INDEX(Sales,,2),A2:A4,\"b\")"),
        n(2.0)
    );
    assert_eq!(
        eval(&mut engine, 10, 7, "=OFFSET(Sales[Qty],1,0,1,1)"),
        n(2.0)
    );
    // Out of the selected area stays #REF!, as for the A1 range.
    let LiteralValue::Error(error) = eval(&mut engine, 11, 7, "=INDEX(Sales[Qty],4)") else {
        panic!("INDEX past the data body must be an error");
    };
    assert_eq!(error.kind, ExcelErrorKind::Ref);

    // From another sheet the reference stays on the table's sheet.
    engine.add_sheet("Other").unwrap();
    engine
        .set_cell_formula("Other", 1, 1, parse("=INDEX(Sales[Item],2)").unwrap())
        .unwrap();
    engine.evaluate_cell("Other", 1, 1).unwrap();
    assert_eq!(engine.get_cell_value("Other", 1, 1).unwrap(), text("b"));
}

#[test]
fn index_and_offset_over_a_table_combine_with_plain_references() {
    let mut engine = engine();
    let n = LiteralValue::Number;
    // INDEX and OFFSET return the reference into the table's A1 area, so a
    // plain reference on the same sheet makes a range with it, as with
    // B2:INDEX(B2:B4,3): B2:INDEX(Sales[Qty],3) is B2:B4.
    assert_eq!(
        eval(&mut engine, 1, 7, "=SUM(B2:INDEX(Sales[Qty],3))"),
        n(6.0)
    );
    assert_eq!(
        eval(
            &mut engine,
            2,
            7,
            "=SUM($B$2:INDEX(Sales[Qty],MATCH(\"c\",Sales[Item],0)))"
        ),
        n(6.0)
    );
    assert_eq!(
        eval(&mut engine, 3, 7, "=ROWS(A2:INDEX(Sales[Item],2))"),
        n(2.0)
    );
    assert_eq!(
        eval(&mut engine, 4, 7, "=SUM(INDEX(Sales[Qty],1):B4)"),
        n(6.0)
    );
    assert_eq!(
        eval(&mut engine, 5, 7, "=SUM(B2:OFFSET(Sales[Qty],2,0,1,1))"),
        n(6.0)
    );
    // INDEX(Sales,0,0) is the whole data body, A2:B4; with B5 it is A2:B5.
    assert_eq!(
        eval(&mut engine, 6, 7, "=COUNT(INDEX(Sales,0,0):B5)"),
        n(4.0)
    );
    assert_eq!(
        eval(&mut engine, 7, 7, "=ROWS(INDEX(Sales[Qty],0,0))"),
        n(3.0)
    );
    // A sheet-qualified end on the formula's own sheet is the same sheet.
    assert_eq!(
        eval(&mut engine, 8, 7, "=SUM(Sheet1!B2:INDEX(Sales[Qty],3))"),
        n(6.0)
    );
    assert_eq!(
        eval(&mut engine, 9, 7, "=SUM(Sheet1!B2:INDEX(B2:B4,3))"),
        n(6.0)
    );

    // From another sheet the table's area keeps its sheet.
    engine.add_sheet("Other").unwrap();
    engine
        .set_cell_formula(
            "Other",
            1,
            1,
            parse("=SUM(Sheet1!B2:INDEX(Sales[Qty],3))").unwrap(),
        )
        .unwrap();
    engine.evaluate_cell("Other", 1, 1).unwrap();
    assert_eq!(engine.get_cell_value("Other", 1, 1).unwrap(), n(6.0));
}

#[test]
fn running_total_from_index_to_this_row() {
    let mut engine = engine();
    // Run at D1:F4: Item, Qty and a running total of Qty.
    let text = |s: &str| LiteralValue::Text(s.into());
    for (row, item, qty) in [
        (1, text("Item"), text("Qty")),
        (2, text("a"), LiteralValue::Number(1.0)),
        (3, text("b"), LiteralValue::Number(2.0)),
        (4, text("c"), LiteralValue::Number(3.0)),
    ] {
        engine.set_cell_value("Sheet1", row, 4, item).unwrap();
        engine.set_cell_value("Sheet1", row, 5, qty).unwrap();
    }
    engine
        .set_cell_value("Sheet1", 1, 6, text("Total"))
        .unwrap();
    let sheet = engine.sheet_id("Sheet1").unwrap();
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 4, true, true)),
        CellRef::new(sheet, Coord::from_excel(4, 6, true, true)),
    );
    engine
        .define_table(
            "Run",
            range,
            true,
            vec!["Item".into(), "Qty".into(), "Total".into()],
            false,
        )
        .unwrap();
    for row in 2..=4 {
        engine
            .set_cell_formula(
                "Sheet1",
                row,
                6,
                parse("=SUM(INDEX(Run[Qty],1):Run[[#This Row],[Qty]])").unwrap(),
            )
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    let totals: Vec<_> = (2..=4)
        .map(|row| engine.get_cell_value("Sheet1", row, 6).unwrap())
        .collect();
    let n = LiteralValue::Number;
    assert_eq!(totals, vec![n(1.0), n(3.0), n(6.0)]);
}

#[test]
fn empty_brackets_are_the_data_body() {
    let mut engine = engine();
    let n = LiteralValue::Number;
    // Sales[] is Sales[#Data] (A2:B4): no header, no totals row.
    assert_eq!(eval(&mut engine, 1, 4, "=ROWS(Sales[])"), n(3.0));
    assert_eq!(eval(&mut engine, 2, 4, "=COUNTA(Sales[])"), n(6.0));
    assert_eq!(eval(&mut engine, 3, 4, "=SUM(Sales[])"), n(6.0));
    assert_eq!(eval(&mut engine, 4, 4, "=MIN(ROW(Sales[]))"), n(2.0));
    // The whole table is still only Sales[#All].
    assert_eq!(eval(&mut engine, 5, 4, "=ROWS(Sales[#All])"), n(5.0));
    assert_eq!(eval(&mut engine, 6, 4, "=COUNTA(Sales[#All])"), n(10.0));
    assert_eq!(eval(&mut engine, 7, 4, "=ROWS(Sales[#Data])"), n(3.0));
    assert_eq!(eval(&mut engine, 8, 4, "=ROWS(Sales)"), n(3.0));
}

#[test]
fn this_row_outside_the_table_body_is_a_value_error() {
    let mut engine = engine();
    let error = engine
        .set_cell_formula("Sheet1", 9, 3, parse("=Sales[[#This Row],[Qty]]").unwrap())
        .unwrap_err();
    assert_eq!(error.kind, ExcelErrorKind::Value);
}
