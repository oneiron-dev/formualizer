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

/// CELL reads the upper-left cell of the area a structured reference
/// selects: the first data cell for `Sales[]`, `Sales[#Data]`, the bare name
/// and columns, the header row only for `[#All]` and `[#Headers]`.
#[test]
fn cell_reads_the_first_cell_of_the_selected_area() {
    let mut engine = engine();
    for (row, formula, expected) in [
        (1, r#"=CELL("address",Sales[])"#, "$A$2"),
        (2, r#"=CELL("address",Sales[ ])"#, "$A$2"),
        (3, r#"=CELL("address",Sales[#Data])"#, "$A$2"),
        (4, r#"=CELL("address",Sales)"#, "$A$2"),
        (5, r#"=CELL("address",Sales[Qty])"#, "$B$2"),
        (6, r#"=CELL("address",Sales[[Item]:[Qty]])"#, "$A$2"),
        (7, r#"=CELL("address",Sales[#All])"#, "$A$1"),
        (8, r#"=CELL("address",Sales[#Headers])"#, "$A$1"),
        (9, r#"=CELL("address",Sales[#Totals])"#, "$A$5"),
    ] {
        assert_eq!(
            eval(&mut engine, row, 6, formula),
            LiteralValue::Text(expected.into()),
            "{formula}"
        );
    }
    for (row, formula, expected) in [
        (10, r#"=CELL("row",Sales[])"#, 2),
        (11, r#"=CELL("row",Sales[qty])"#, 2),
        (12, r#"=CELL("col",Sales[Qty])"#, 2),
        (13, r#"=CELL("row",Sales[#Totals])"#, 5),
    ] {
        match eval(&mut engine, row, 6, formula) {
            LiteralValue::Int(actual) => assert_eq!(actual, expected, "{formula}"),
            LiteralValue::Number(actual) => assert_eq!(actual, expected as f64, "{formula}"),
            other => panic!("{formula}: expected {expected}, got {other:?}"),
        }
    }
    // An unknown column is a #REF! reference, as when it is read.
    match eval(&mut engine, 14, 6, r#"=CELL("address",Sales[Nope])"#) {
        LiteralValue::Error(error) => assert_eq!(error.kind, ExcelErrorKind::Ref),
        other => panic!("CELL over an unknown column: expected #REF!, got {other:?}"),
    }
}

/// ISFORMULA and FORMULATEXT read the first data cell of `F[]`, `F[#Data]`
/// and a column, where the formulas are, not the header cell.
#[test]
fn isformula_and_formulatext_read_the_first_data_cell() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_value("Sheet1", 1, 12, LiteralValue::Text("Äh".into()))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 12, parse("=1+1").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 3, 12, parse("=2+2").unwrap())
        .unwrap();
    let sheet = engine.sheet_id("Sheet1").unwrap();
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 12, true, true)),
        CellRef::new(sheet, Coord::from_excel(3, 12, true, true)),
    );
    engine
        .define_table("F", range, true, vec!["Äh".into()], false)
        .unwrap();

    let yes = LiteralValue::Boolean(true);
    let no = LiteralValue::Boolean(false);
    // Over several cells ISFORMULA tests each one, so each formula gets its
    // own column to spill down; its first value is the area's first cell.
    assert_eq!(eval(&mut engine, 1, 1, "=ISFORMULA(F[])"), yes);
    assert_eq!(eval(&mut engine, 1, 2, "=ISFORMULA(F[#Data])"), yes);
    assert_eq!(eval(&mut engine, 1, 3, "=ISFORMULA(F)"), yes);
    // Column names match case-insensitively beyond ASCII, as when read.
    assert_eq!(eval(&mut engine, 1, 4, "=ISFORMULA(F[äH])"), yes);
    assert_eq!(eval(&mut engine, 1, 5, "=ISFORMULA(F[#All])"), no);
    assert_eq!(eval(&mut engine, 1, 6, "=ISFORMULA(F[#Headers])"), no);
    assert_eq!(
        eval(&mut engine, 7, 1, "=FORMULATEXT(F[])"),
        LiteralValue::Text("=1+1".into())
    );
    assert_eq!(
        eval(&mut engine, 8, 1, "=FORMULATEXT(F[Äh])"),
        LiteralValue::Text("=1+1".into())
    );
    match eval(&mut engine, 9, 1, "=FORMULATEXT(F[#All])") {
        LiteralValue::Error(error) => assert_eq!(error.kind, ExcelErrorKind::Na),
        other => panic!("FORMULATEXT of the header cell: expected #N/A, got {other:?}"),
    }
}

/// Without a header row the data body starts on the table's first row.
#[test]
fn cell_on_a_table_without_a_header_row_starts_at_its_first_row() {
    let mut engine = engine();
    let sheet = engine.sheet_id("Sheet1").unwrap();
    for row in 1..=3 {
        engine
            .set_cell_value("Sheet1", row, 8, LiteralValue::Number(row as f64))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 9, LiteralValue::Number(10.0 * row as f64))
            .unwrap();
    }
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 8, true, true)),
        CellRef::new(sheet, Coord::from_excel(3, 9, true, true)),
    );
    engine
        .define_table(
            "NoHdr",
            range,
            false,
            vec!["Column1".into(), "Column2".into()],
            false,
        )
        .unwrap();
    assert_eq!(
        eval(&mut engine, 1, 6, r#"=CELL("address",NoHdr[])"#),
        LiteralValue::Text("$H$1".into())
    );
    assert_eq!(
        eval(&mut engine, 2, 6, r#"=CELL("address",NoHdr[Column2])"#),
        LiteralValue::Text("$I$1".into())
    );
}

fn assert_error(value: LiteralValue, kind: ExcelErrorKind, what: &str) {
    match value {
        LiteralValue::Error(error) => assert_eq!(error.kind, kind, "{what}"),
        other => panic!("{what}: expected {kind:?}, got {other:?}"),
    }
}

/// `#This Row` reads the formula's row of the data body only: below the
/// table, and on its header or totals row, it is #VALUE! (Microsoft, "Using
/// structured references with Excel tables"). The formula is still entered.
#[test]
fn this_row_off_the_data_body_is_a_value_error() {
    let mut engine = engine();
    for (row, col, formula) in [
        (9, 3, "=Sales[[#This Row],[Qty]]"),
        // Row 5 is the totals row, row 1 the header row.
        (5, 3, "=INDEX(Sales[[#This Row],[Qty]],1)"),
        (5, 4, "=Sales[@Qty]"),
        (1, 3, "=Sales[@Qty]"),
        // Unqualified, inside the table's totals and header rows.
        (5, 1, "=[@Qty]"),
        (1, 1, "=[@Qty]"),
    ] {
        assert_error(
            eval(&mut engine, row, col, formula),
            ExcelErrorKind::Value,
            formula,
        );
    }
    // On a data row it is that row's cell.
    assert_eq!(
        eval(&mut engine, 4, 3, "=INDEX(Sales[[#This Row],[Qty]],1)"),
        LiteralValue::Number(3.0)
    );
    // The metadata functions see the same #VALUE! on the totals row.
    for (col, formula) in [
        (6, r#"=CELL("address",Sales[@Qty])"#),
        (7, "=ISFORMULA(Sales[@Qty])"),
        (8, "=FORMULATEXT(Sales[@Qty])"),
    ] {
        assert_error(
            eval(&mut engine, 5, col, formula),
            ExcelErrorKind::Value,
            formula,
        );
    }
}

/// OFFSET and INDEX take a structured reference's area from the table's
/// placement, not by reading it, so only the cell they select is read: a
/// formula in another cell of the column that reads their result is no cycle.
#[test]
fn offset_and_index_over_a_table_read_only_the_selected_cell() {
    use crate::engine::{CycleConfig, CycleDetection, CyclePolicy};
    for formula in ["=OFFSET(T[Qty],0,0,1,1)", "=INDEX(T[Qty],1)"] {
        let config = EvalConfig::default().with_cycle(CycleConfig {
            detection: CycleDetection::Runtime,
            policy: CyclePolicy::Error,
        });
        let mut engine = Engine::new(TestWorkbook::new(), config);
        // T at Q1:Q3: Qty, =7, =C9.
        engine
            .set_cell_value("Sheet1", 1, 17, LiteralValue::Text("Qty".into()))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 2, 17, parse("=7").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 3, 17, parse("=C9").unwrap())
            .unwrap();
        let sheet = engine.sheet_id("Sheet1").unwrap();
        let range = RangeRef::new(
            CellRef::new(sheet, Coord::from_excel(1, 17, true, true)),
            CellRef::new(sheet, Coord::from_excel(3, 17, true, true)),
        );
        engine
            .define_table("T", range, true, vec!["Qty".into()], false)
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 9, 3, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_all().unwrap();
        let n = LiteralValue::Number(7.0);
        assert_eq!(
            engine.get_cell_value("Sheet1", 9, 3),
            Some(n.clone()),
            "{formula}"
        );
        assert_eq!(engine.get_cell_value("Sheet1", 3, 17), Some(n), "{formula}");
    }
}

/// A position past the reference is #REF! however large: it is never
/// truncated to a smaller one, nor overflows, for any kind of reference.
#[test]
fn index_positions_beyond_the_reference_are_ref_errors() {
    let mut engine = engine();
    for (row, formula) in [
        (1, "=INDEX(Sales[Qty],4294967297)"),
        (2, "=INDEX(Sales[Qty],1,4294967297)"),
        (3, "=INDEX(Sales[Qty],4294967295)"),
        (4, "=INDEX(Sales,4294967297,1)"),
        (5, "=INDEX(B2:B4,4294967297)"),
        (6, "=INDEX(B2:B4,4294967295)"),
        (7, "=INDEX(A2:B2,1,4294967297)"),
        (8, "=INDEX(B:B,4294967297)"),
        (9, "=INDEX(B2,1,4294967297)"),
        (10, "=SUM(INDEX(A2:B4,0,4294967297))"),
        (11, "=SUM(INDEX(A2:B4,4294967297,0))"),
        (12, "=INDEX({1,2,3},1,4294967297)"),
        (13, "=INDEX({1;2;3},4294967297)"),
        (14, "=INDEX(B2:B4,1E+300)"),
    ] {
        assert_error(
            eval(&mut engine, row, 7, formula),
            ExcelErrorKind::Ref,
            formula,
        );
    }
}

/// A structured reference is a reference like its A1 area, so it is an end
/// of `:` and an operand of the intersection ` ` (Sales[Qty] is B2:B4).
#[test]
fn structured_references_compose_with_reference_operators() {
    let mut engine = engine();
    let n = LiteralValue::Number;
    assert_eq!(
        eval(&mut engine, 1, 7, "=SUM(INDEX(Sales[Qty],1):Sales[Qty])"),
        n(6.0)
    );
    assert_eq!(
        eval(&mut engine, 2, 7, "=SUM(INDEX(Sales[Qty],0) Sales[Qty])"),
        n(6.0)
    );
    assert_eq!(eval(&mut engine, 3, 7, "=ROWS(Sales[Qty]:B5)"), n(4.0));
    assert_eq!(
        eval(&mut engine, 4, 7, "=COUNTA(Sales[Item]:Sales[Qty])"),
        n(6.0)
    );
    assert_eq!(eval(&mut engine, 5, 7, "=Sales[Qty] 3:3"), n(2.0));
    assert_eq!(eval(&mut engine, 6, 7, "=ROWS(A6:Sales[Item])"), n(5.0));
    assert_error(
        eval(&mut engine, 7, 7, "=Sales[Qty] D:D"),
        ExcelErrorKind::Null,
        "disjoint intersection",
    );
    // From another sheet the table's area stays on its sheet.
    engine.add_sheet("Other").unwrap();
    engine
        .set_cell_formula("Other", 1, 1, parse("=SUM(Sales[Qty]:Sheet1!B5)").unwrap())
        .unwrap();
    engine.evaluate_cell("Other", 1, 1).unwrap();
    assert_eq!(engine.get_cell_value("Other", 1, 1), Some(n(12.0)));
}

/// A structured reference in a union is the area it selects, so INDEX's
/// area_num numbers it like an A1 range and ` ` intersects it like one
/// (Sales[Item] is A2:A4, Sales[Qty] B2:B4, both on Sheet1).
#[test]
fn index_area_num_selects_among_structured_reference_areas() {
    let mut engine = engine();
    let n = LiteralValue::Number;
    let text = |s: &str| LiteralValue::Text(s.into());
    assert_eq!(
        eval(&mut engine, 1, 7, "=INDEX((Sales[Item],Sales[Qty]),2,1,2)"),
        n(2.0)
    );
    assert_eq!(
        eval(&mut engine, 2, 7, "=INDEX((Sales[Item],Sales[Qty]),3,1)"),
        text("c")
    );
    assert_eq!(
        eval(&mut engine, 3, 7, "=INDEX((A2:A4,Sales[Qty]),3,1,2)"),
        n(3.0)
    );
    assert_eq!(
        eval(
            &mut engine,
            4,
            7,
            "=SUM(INDEX((Sales[Item],Sales[Qty]),0,1,2))"
        ),
        n(6.0)
    );
    // The intersection with row 3 has the areas A3 and B3.
    assert_eq!(
        eval(
            &mut engine,
            5,
            7,
            "=INDEX(((Sales[Item],Sales[Qty]) 3:3),1,1,2)"
        ),
        n(2.0)
    );
    assert_error(
        eval(&mut engine, 6, 7, "=INDEX((Sales[Item],Sales[Qty]),1,1,3)"),
        ExcelErrorKind::Ref,
        "area past the last",
    );

    // From another sheet the table's areas stay on its sheet: with a plain
    // reference to that sheet they are one union, with a cell of the
    // formula's own sheet the areas lie on two sheets (#VALUE!).
    engine.add_sheet("Other").unwrap();
    for (row, formula) in [
        (1, "=INDEX((Sheet1!A2:A4,Sales[Qty]),2,1,2)"),
        (2, "=INDEX((Sales[Item],Sales[Qty]),3,1,2)"),
        (3, "=INDEX((Sales[Qty],A1),1,1,1)"),
    ] {
        engine
            .set_cell_formula("Other", row, 1, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_cell("Other", row, 1).unwrap();
    }
    assert_eq!(engine.get_cell_value("Other", 1, 1), Some(n(2.0)));
    assert_eq!(engine.get_cell_value("Other", 2, 1), Some(n(3.0)));
    assert_error(
        engine.get_cell_value("Other", 3, 1).unwrap(),
        ExcelErrorKind::Value,
        "areas on two sheets",
    );

    // Editing a cell of the selected area recalculates.
    engine.set_cell_value("Sheet1", 3, 2, n(20.0)).unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 1, 7), Some(n(20.0)));
    assert_eq!(engine.get_cell_value("Sheet1", 5, 7), Some(n(20.0)));
    assert_eq!(engine.get_cell_value("Other", 1, 1), Some(n(20.0)));
}

/// T at A1:C5 (A, B, C; totals row) with a formula in A3. A `#This Row`
/// reference built at run time (INDIRECT) is read at the formula's row by
/// CELL, ISFORMULA and FORMULATEXT, like the same reference written directly.
#[test]
fn runtime_this_row_references_use_the_formula_row() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (col, header) in [(1, "A"), (2, "B"), (3, "C")] {
        engine
            .set_cell_value("Sheet1", 1, col, LiteralValue::Text(header.into()))
            .unwrap();
    }
    for row in 2..=4 {
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(10.0 * row as f64))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 3, 1, parse("=1+1").unwrap())
        .unwrap();
    let sheet = engine.sheet_id("Sheet1").unwrap();
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 1, true, true)),
        CellRef::new(sheet, Coord::from_excel(5, 3, true, true)),
    );
    engine
        .define_table(
            "T",
            range,
            true,
            vec!["A".into(), "B".into(), "C".into()],
            true,
        )
        .unwrap();
    let text = |s: &str| LiteralValue::Text(s.into());
    for (col, formula, expected) in [
        (
            5,
            r#"=CELL("row",INDIRECT("T[@A]"))"#,
            LiteralValue::Number(3.0),
        ),
        (6, r#"=CELL("address",INDIRECT("T[@B]"))"#, text("$B$3")),
        (
            7,
            r#"=CELL("contents",INDIRECT("T[@B]"))"#,
            LiteralValue::Number(30.0),
        ),
        (
            8,
            r#"=ISFORMULA(INDIRECT("T[@A]"))"#,
            LiteralValue::Boolean(true),
        ),
        (9, r#"=FORMULATEXT(INDIRECT("T[@A]"))"#, text("=1+1")),
        (
            10,
            r#"=ISFORMULA(INDIRECT("T[[#This Row],[B]]"))"#,
            LiteralValue::Boolean(false),
        ),
    ] {
        assert_eq!(eval(&mut engine, 3, col, formula), expected, "{formula}");
    }
    // INDIRECT's own value is that row's cell too.
    assert_eq!(
        eval(&mut engine, 3, 11, r#"=INDIRECT("T[@B]")"#),
        LiteralValue::Number(30.0)
    );
    assert_eq!(
        eval(&mut engine, 3, 12, r#"=SUM(INDIRECT("T[B]"))"#),
        LiteralValue::Number(90.0)
    );
    // On the totals row the same reference is #VALUE!.
    assert_error(
        eval(&mut engine, 5, 5, r#"=CELL("row",INDIRECT("T[@A]"))"#),
        ExcelErrorKind::Value,
        "CELL of #This Row on the totals row",
    );
    assert_error(
        eval(&mut engine, 5, 11, r#"=INDIRECT("T[@B]")"#),
        ExcelErrorKind::Value,
        "INDIRECT of #This Row on the totals row",
    );

    // Evaluated as written, without ingest's A1 rewrite, CELL, ISFORMULA and
    // FORMULATEXT read `#This Row` at the formula's row.
    let sheet = engine.sheet_id("Sheet1").unwrap();
    let at_row_3 = CellRef::new(sheet, Coord::from_excel(3, 20, true, true));
    let interp = crate::interpreter::Interpreter::new_with_cell(&engine, "Sheet1", at_row_3);
    for (formula, expected) in [
        (r#"=CELL("row",T[@A])"#, LiteralValue::Number(3.0)),
        (r#"=CELL("contents",T[@B])"#, LiteralValue::Number(30.0)),
        ("=ISFORMULA(T[@A])", LiteralValue::Boolean(true)),
        ("=FORMULATEXT(T[@A])", text("=1+1")),
    ] {
        let value = interp
            .evaluate_ast(&parse(formula).unwrap())
            .unwrap()
            .into_literal();
        let value = match value {
            LiteralValue::Int(i) => LiteralValue::Number(i as f64),
            other => other,
        };
        assert_eq!(value, expected, "{formula}");
    }
}

/// CELL describes the reference it is given: a table on another sheet is
/// addressed with the file's and that sheet's names, as Excel addresses any
/// cell on another sheet (ops/excel-hostinfo-probe-20261008.md), and
/// "filename" names that sheet.
#[test]
fn cell_names_the_sheet_of_a_table_on_another_sheet() {
    let config = EvalConfig {
        workbook_file_name: Some("Book.xlsx".into()),
        ..EvalConfig::default()
    };
    let mut engine = Engine::new(TestWorkbook::new(), config);
    engine.add_sheet("Data").unwrap();
    for (row, qty) in [
        (1, LiteralValue::Text("Qty".into())),
        (2, LiteralValue::Number(1.0)),
    ] {
        engine.set_cell_value("Data", row, 2, qty).unwrap();
    }
    let data = engine.sheet_id("Data").unwrap();
    let range = RangeRef::new(
        CellRef::new(data, Coord::from_excel(1, 2, true, true)),
        CellRef::new(data, Coord::from_excel(2, 2, true, true)),
    );
    engine
        .define_table("F", range, true, vec!["Qty".into()], false)
        .unwrap();
    let text = |s: &str| LiteralValue::Text(s.into());
    for (row, formula, expected) in [
        (1, r#"=CELL("address",F[Qty])"#, "[Book.xlsx]Data!$B$2"),
        (2, r#"=CELL("filename",F[Qty])"#, "[Book.xlsx]Data"),
        (
            3,
            r#"=CELL("address",F[[#Data],[Qty]])"#,
            "[Book.xlsx]Data!$B$2",
        ),
        (4, r#"=CELL("filename",A1)"#, "[Book.xlsx]Sheet1"),
    ] {
        assert_eq!(
            eval(&mut engine, row, 5, formula),
            text(expected),
            "{formula}"
        );
    }
    // On the table's own sheet the address is unqualified.
    engine
        .set_cell_formula("Data", 1, 5, parse(r#"=CELL("address",F[Qty])"#).unwrap())
        .unwrap();
    engine.evaluate_cell("Data", 1, 5).unwrap();
    assert_eq!(engine.get_cell_value("Data", 1, 5), Some(text("$B$2")));
}

/// With the header row turned off, a reference straight to the headers is
/// #REF! when read, as it is for CELL's metadata; column references still
/// read their data.
#[test]
fn hidden_headers_are_a_ref_error_when_read() {
    let mut engine = engine();
    let sheet = engine.sheet_id("Sheet1").unwrap();
    for row in 1..=3 {
        engine
            .set_cell_value("Sheet1", row, 8, LiteralValue::Number(row as f64))
            .unwrap();
    }
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 8, true, true)),
        CellRef::new(sheet, Coord::from_excel(3, 8, true, true)),
    );
    engine
        .define_table("NoHdr", range, false, vec!["Column1".into()], false)
        .unwrap();
    for (row, formula) in [
        (1, "=SUM(NoHdr[#Headers])"),
        (2, "=NoHdr[#Headers]"),
        (3, r#"=CELL("contents",NoHdr[#Headers])"#),
        (4, r#"=CELL("type",NoHdr[#Headers])"#),
        (5, r#"=CELL("address",NoHdr[#Headers])"#),
        (6, "=NoHdr[[#Headers],[Column1]]"),
    ] {
        assert_error(
            eval(&mut engine, row, 10, formula),
            ExcelErrorKind::Ref,
            formula,
        );
    }
    assert_eq!(
        eval(&mut engine, 7, 10, "=SUM(NoHdr[Column1])"),
        LiteralValue::Number(6.0)
    );
}

/// CELL("type") is "v" for a cell holding an error value: only an argument
/// that is no reference propagates its error.
#[test]
fn cell_type_of_an_error_cell_is_a_value() {
    let mut engine = engine();
    engine
        .set_cell_formula("Sheet1", 2, 2, parse("=1/0").unwrap())
        .unwrap();
    let text = |s: &str| LiteralValue::Text(s.into());
    assert_eq!(
        eval(&mut engine, 1, 7, r#"=CELL("type",Sales[Qty])"#),
        text("v")
    );
    assert_eq!(eval(&mut engine, 2, 7, r#"=CELL("type",B2)"#), text("v"));
    assert_error(
        eval(&mut engine, 3, 7, r#"=CELL("contents",B2)"#),
        ExcelErrorKind::Div,
        "CELL contents of an error cell",
    );
    assert_error(
        eval(&mut engine, 4, 7, r#"=CELL("type",1/0)"#),
        ExcelErrorKind::Div,
        "CELL type of an error argument",
    );
    assert_error(
        eval(&mut engine, 5, 7, r#"=CELL("type",INDIRECT("no such"))"#),
        ExcelErrorKind::Ref,
        "CELL type of a reference that does not resolve",
    );
    assert_eq!(
        eval(&mut engine, 6, 7, r#"=CELL("type",Sales[Item])"#),
        text("l")
    );
}

/// ISFORMULA over several cells tests each one, as Microsoft lists it among
/// the functions that return arrays ("Excel functions that return ranges or
/// arrays"): SUMPRODUCT(--ISFORMULA(range)) counts formulas, and a dynamic
/// array formula spills one result per cell. A legacy formula (no array
/// flag) takes the first.
#[test]
fn isformula_over_several_cells_tests_each_one() {
    // F at B1:B4: Qty, =1+1, =2+2, 5.
    let build = || {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        engine
            .set_cell_value("Sheet1", 1, 2, LiteralValue::Text("Qty".into()))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 2, 2, parse("=1+1").unwrap())
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 3, 2, parse("=2+2").unwrap())
            .unwrap();
        engine
            .set_cell_value("Sheet1", 4, 2, LiteralValue::Number(5.0))
            .unwrap();
        let sheet = engine.sheet_id("Sheet1").unwrap();
        let range = RangeRef::new(
            CellRef::new(sheet, Coord::from_excel(1, 2, true, true)),
            CellRef::new(sheet, Coord::from_excel(4, 2, true, true)),
        );
        engine
            .define_table("F", range, true, vec!["Qty".into()], false)
            .unwrap();
        engine
    };
    let n = LiteralValue::Number;
    let yes = LiteralValue::Boolean(true);
    let no = LiteralValue::Boolean(false);

    let mut engine = build();
    assert_eq!(
        eval(&mut engine, 1, 5, "=SUMPRODUCT(--ISFORMULA(F[Qty]))"),
        n(2.0)
    );
    assert_eq!(
        eval(&mut engine, 2, 5, "=SUMPRODUCT(--ISFORMULA(B1:B4))"),
        n(2.0)
    );
    assert_eq!(
        eval(&mut engine, 3, 5, "=SUMPRODUCT(--ISFORMULA(B3:B4))"),
        n(1.0)
    );
    assert_eq!(
        eval(&mut engine, 4, 5, "=SUMPRODUCT(--ISFORMULA(B4))"),
        n(0.0)
    );
    // Spilled: one result per cell of B1:B4.
    eval(&mut engine, 1, 7, "=ISFORMULA(B1:B4)");
    let spilled: Vec<_> = (1..=4)
        .map(|row| engine.get_cell_value("Sheet1", row, 7).unwrap())
        .collect();
    assert_eq!(
        spilled,
        vec![no.clone(), yes.clone(), yes.clone(), no.clone()]
    );

    // A legacy formula reads the first cell; SUMPRODUCT still counts all.
    let mut engine = build();
    for (row, formula) in [
        (1, "=ISFORMULA(B1:B4)"),
        (2, "=ISFORMULA(F[Qty])"),
        (3, "=SUMPRODUCT(--ISFORMULA(B1:B4))"),
    ] {
        engine
            .set_cell_formula("Sheet1", row, 5, parse(formula).unwrap())
            .unwrap();
    }
    engine.use_legacy_array_semantics();
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 1, 5), Some(no));
    assert_eq!(engine.get_cell_value("Sheet1", 2, 5), Some(yes));
    assert_eq!(engine.get_cell_value("Sheet1", 3, 5), Some(n(2.0)));
}

/// Inside a table an unqualified `[Col]` is that table's column: the
/// parser's `[Name]` (the `[TableName]` data-body shorthand) is read as the
/// containing table's column when no table has that name.
#[test]
fn unqualified_column_inside_a_table_is_its_column() {
    let mut engine = engine();
    let sheet = engine.sheet_id("Sheet1").unwrap();
    // Calc at J1:L4: Item, Qty, Calc.
    for (col, header) in [(10, "Item"), (11, "Qty"), (12, "Calc")] {
        engine
            .set_cell_value("Sheet1", 1, col, LiteralValue::Text(header.into()))
            .unwrap();
    }
    for row in 2..=4 {
        engine
            .set_cell_value("Sheet1", row, 11, LiteralValue::Number(row as f64))
            .unwrap();
    }
    let range = RangeRef::new(
        CellRef::new(sheet, Coord::from_excel(1, 10, true, true)),
        CellRef::new(sheet, Coord::from_excel(4, 12, true, true)),
    );
    engine
        .define_table(
            "Calc",
            range,
            true,
            vec!["Item".into(), "Qty".into(), "Calc".into()],
            false,
        )
        .unwrap();
    let n = LiteralValue::Number;
    assert_eq!(eval(&mut engine, 2, 12, "=INDEX([Qty],1)"), n(2.0));
    assert_eq!(eval(&mut engine, 3, 12, "=SUM(OFFSET([Qty],0,0))"), n(9.0));
    assert_eq!(eval(&mut engine, 4, 12, "=SUM([qty])"), n(9.0));
    // A table of that name still wins, as `[Sales]` is Sales' data body.
    assert_eq!(eval(&mut engine, 4, 10, "=SUM([Sales])"), n(6.0));
}

/// A table's bare name in a union is the table's data body (Sales is
/// A2:B4), also where the formula was not rewritten at entry; a LET name of
/// the same name shadows it, and a LET value is no reference, so the union
/// is #VALUE!.
#[test]
fn bare_table_name_in_a_union_is_its_data_body_unless_shadowed() {
    use crate::interpreter::Interpreter;
    let mut engine = engine();
    engine
        .set_cell_value("Sheet1", 1, 4, LiteralValue::Number(42.0))
        .unwrap();
    let interpreter = Interpreter::new(&engine, "Sheet1");
    let evaluate = |formula: &str| {
        interpreter
            .evaluate_ast(&parse(formula).unwrap())
            .unwrap()
            .into_literal()
    };
    assert_eq!(
        evaluate("=INDEX((Sales,D1),2,1,1)"),
        LiteralValue::Text("b".into())
    );
    assert_eq!(
        evaluate("=INDEX((Sales,D1),1,1,2)"),
        LiteralValue::Number(42.0)
    );
    assert_error(
        evaluate("=LET(Sales,5,INDEX((Sales,D1),1,1,2))"),
        ExcelErrorKind::Value,
        "a LET value shadowing the table",
    );
}
