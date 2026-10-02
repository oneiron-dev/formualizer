//! Excel rules for numeric text in arrays and database records, D-function
//! text criteria, lookup index arrays, the final-operation zero
//! compensation with outer parentheses, and fractional-second time text.

use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

use crate::engine::{
    Engine, EvalConfig, FormulaIngestBatch, FormulaIngestRecord, FormulaPlaneMode,
};
use crate::test_workbook::TestWorkbook;

/// An engine holding `cells` (address, value) on Sheet1.
fn engine_with(cells: &[(&str, LiteralValue)]) -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (address, value) in cells {
        let (row, col, _, _) = formualizer_common::coord::parse_a1_1based(address).unwrap();
        engine
            .set_cell_value("Sheet1", row, col, value.clone())
            .unwrap();
    }
    engine
}

/// Evaluates each formula in its own cell of column Z and compares.
fn assert_values(engine: &mut Engine<TestWorkbook>, cases: &[(&str, LiteralValue)]) {
    for (i, (formula, _)) in cases.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", 1 + i as u32, 26, parse(formula).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    for (i, (formula, expected)) in cases.iter().enumerate() {
        let got = engine.get_cell_value("Sheet1", 1 + i as u32, 26);
        match (expected, &got) {
            (LiteralValue::Error(e), Some(LiteralValue::Error(g))) => {
                assert_eq!(e.kind, g.kind, "{formula}")
            }
            _ => assert_eq!(got.as_ref(), Some(expected), "{formula}"),
        }
    }
}

fn text(s: &str) -> LiteralValue {
    LiteralValue::Text(s.into())
}

fn number(n: f64) -> LiteralValue {
    LiteralValue::Number(n)
}

#[test]
fn sumproduct_treats_text_and_logical_entries_as_zero() {
    // Microsoft: "SUMPRODUCT treats non-numeric array entries as if they were
    // zeros". Numeric, currency or grouped text is not a number there; an
    // explicit conversion (--) still makes it one.
    let mut engine = engine_with(&[
        ("A1", text("5")),
        ("A2", LiteralValue::Boolean(true)),
        ("A3", number(4.0)),
    ]);
    assert_values(
        &mut engine,
        &[
            ("=SUMPRODUCT({\"$5\"},{2})", number(0.0)),
            ("=SUMPRODUCT({\"1,234\"},{2})", number(0.0)),
            ("=SUMPRODUCT({\"5\"},{2})", number(0.0)),
            ("=SUMPRODUCT({TRUE},{2})", number(0.0)),
            ("=SUMPRODUCT(A1:A3,{2;3;5})", number(20.0)),
            ("=SUMPRODUCT(A1:A3)", number(4.0)),
            ("=SUMPRODUCT(--{\"5\"},{2})", number(10.0)),
            ("=SUMPRODUCT(--(A1:A3=4),{2;3;5})", number(5.0)),
            ("=SUMPRODUCT((A1:A3=4)*1,{2;3;5})", number(5.0)),
            (
                "=SUMPRODUCT({\"x\",#N/A},{1,1})",
                LiteralValue::Error(formualizer_common::ExcelError::new(ExcelErrorKind::Na)),
            ),
        ],
    );
}

#[test]
fn database_records_hold_numbers_not_numeric_text() {
    // DCOUNT counts "the cells that contain numbers", DSUM adds "the numbers
    // in a field": a record's text, currency text included, is neither.
    let mut engine = engine_with(&[
        ("A1", text("Amount")),
        ("A2", text("$5")),
        ("A3", text("7")),
        ("A4", number(3.0)),
        ("C1", text("Amount")),
    ]);
    assert_values(
        &mut engine,
        &[
            ("=DCOUNT(A1:A4,\"Amount\",C1:C2)", number(1.0)),
            ("=DSUM(A1:A4,\"Amount\",C1:C2)", number(3.0)),
            ("=DAVERAGE(A1:A4,\"Amount\",C1:C2)", number(3.0)),
            ("=DMAX(A1:A4,\"Amount\",C1:C2)", number(3.0)),
            ("=DVARP(A1:A4,\"Amount\",C1:C2)", number(0.0)),
            ("=DCOUNTA(A1:A4,\"Amount\",C1:C2)", number(3.0)),
        ],
    );
}

#[test]
fn database_text_criteria_without_operator_select_values_beginning_with_it() {
    // Microsoft (Advanced Filter criteria): "if you type the text Dav as a
    // criterion, Excel finds "Davolio," "David," and "Davis."";
    // ="=Davolio" is the exact match.
    let mut engine = engine_with(&[
        ("A1", text("Name")),
        ("B1", text("Amount")),
        ("A2", text("Davolio")),
        ("B2", number(10.0)),
        ("A3", text("David")),
        ("B3", number(20.0)),
        ("A4", text("Dav")),
        ("B4", number(40.0)),
        ("A5", text("a~bc")),
        ("B5", number(80.0)),
        ("A6", text("smithson")),
        ("B6", number(160.0)),
    ]);
    let criteria = [
        ("Dav", 70.0),
        ("dav", 70.0),
        ("Davi", 20.0),
        ("=Dav", 40.0),
        ("=David", 20.0),
        ("<>Dav", 270.0),
        ("D?v", 70.0),
        ("a~b", 80.0),
        ("sm?th", 160.0),
        ("x", 0.0),
    ];
    for (i, (criterion, _)) in criteria.iter().enumerate() {
        let col = 4 + i as u32;
        engine
            .set_cell_value("Sheet1", 1, col, text("Name"))
            .unwrap();
        engine
            .set_cell_value("Sheet1", 2, col, text(criterion))
            .unwrap();
    }
    let formulas: Vec<(String, LiteralValue)> = criteria
        .iter()
        .enumerate()
        .map(|(i, (_, expected))| {
            let col = formualizer_common::coord::col_letters_from_1based(4 + i as u32).unwrap();
            (
                format!("=DSUM(A1:B6,\"Amount\",{col}1:{col}2)"),
                number(*expected),
            )
        })
        .collect();
    let cases: Vec<(&str, LiteralValue)> = formulas
        .iter()
        .map(|(f, v)| (f.as_str(), v.clone()))
        .collect();
    assert_values(&mut engine, &cases);
}

#[test]
fn lookup_index_arguments_read_numeric_text_as_numbers() {
    // col_index_num and row_index_num are number parameters: numeric text
    // converts, element by element in an index array too; an error is the
    // result.
    let mut engine = engine_with(&[]);
    assert_values(
        &mut engine,
        &[
            ("=VLOOKUP(1,{1,42},\"2\",FALSE)", number(42.0)),
            ("=HLOOKUP(1,{1;42},\"2\",FALSE)", number(42.0)),
            ("=SUM(VLOOKUP(1,{1,42},{\"2\",2},FALSE))", number(84.0)),
            ("=SUM(HLOOKUP(1,{1;42},{\"2\";2},FALSE))", number(84.0)),
            (
                "=VLOOKUP(1,{1,42},\"x\",FALSE)",
                LiteralValue::Error(formualizer_common::ExcelError::new_value()),
            ),
            (
                "=VLOOKUP(1,{1,42},NA(),FALSE)",
                LiteralValue::Error(formualizer_common::ExcelError::new(ExcelErrorKind::Na)),
            ),
            ("=VLOOKUP(1,{1,42},MODE.MULT({2,2}),FALSE)", number(42.0)),
        ],
    );
}

#[test]
fn outer_parentheses_are_the_last_operation() {
    // Excel compensates a formula's final + or - to exactly 0, but with the
    // formula in parentheses the parentheses are the last operation and the
    // binary residue stays: =(1+2^-52-1) is 2^-52.
    let residue = 0.1 + 0.2 - 0.3;
    let mut engine = engine_with(&[
        ("A1", number(0.1)),
        ("A2", number(0.2)),
        ("A3", number(0.3)),
    ]);
    assert_values(
        &mut engine,
        &[
            ("=0.1+0.2-0.3", number(0.0)),
            ("=(0.1+0.2-0.3)", number(residue)),
            ("=((0.1+0.2-0.3))", number(residue)),
            ("=(0.1+0.2)-0.3", number(0.0)),
            ("=(1+2^-52-1)", number(2f64.powi(-52))),
            ("=1+2^-52-1", number(0.0)),
            ("=A1+A2-A3", number(0.0)),
            ("=(A1+A2-A3)", number(residue)),
            ("=1*(0.5-0.4-0.1)", number(0.5 - 0.4 - 0.1)),
        ],
    );
}

#[test]
fn formula_plane_keeps_parenthesized_and_bare_final_operations_apart() {
    // Rows alternate =(A+B-C) and =A+B-C (and the constant forms) over the
    // same 0.1/0.2/0.3 values: a span template must not carry one form's last
    // operation to the other; whole columns of one form still match.
    let residue = 0.1 + 0.2 - 0.3;
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_formula_plane_mode(mode),
        );
        let mut records = Vec::new();
        for row in 1..=12u32 {
            for (col, value) in [(1, 0.1), (2, 0.2), (3, 0.3)] {
                engine
                    .set_cell_value("Sheet1", row, col, number(value))
                    .unwrap();
            }
            let paren = row % 2 == 1;
            let relative = |paren: bool| {
                if paren {
                    format!("=(A{row}+B{row}-C{row})")
                } else {
                    format!("=A{row}+B{row}-C{row}")
                }
            };
            let constant = |paren: bool| {
                if paren {
                    "=(0.1+0.2-0.3)"
                } else {
                    "=0.1+0.2-0.3"
                }
                .to_string()
            };
            for (col, formula) in [
                (4, relative(paren)),
                (5, constant(paren)),
                (6, relative(true)),
                (7, relative(false)),
                (8, constant(true)),
                (9, constant(false)),
            ] {
                let ast_id = engine.intern_formula_ast(&parse(&formula).unwrap());
                records.push(FormulaIngestRecord::new(
                    row,
                    col,
                    ast_id,
                    Some(std::sync::Arc::<str>::from(formula)),
                ));
            }
        }
        engine
            .ingest_formula_batches(vec![FormulaIngestBatch::new("Sheet1", records)])
            .unwrap();
        engine.evaluate_all().unwrap();
        for row in 1..=12u32 {
            let alternating = if row % 2 == 1 { residue } else { 0.0 };
            for (col, expected) in [
                (4, alternating),
                (5, alternating),
                (6, residue),
                (7, 0.0),
                (8, residue),
                (9, 0.0),
            ] {
                assert_eq!(
                    engine.get_cell_value("Sheet1", row, col),
                    Some(number(expected)),
                    "{mode:?} row {row} col {col}"
                );
            }
        }
    }
}

#[test]
fn time_text_keeps_fractional_seconds_and_reads_minutes_and_seconds() {
    let mut engine = engine_with(&[("H1", number(7.0)), ("H2", number(5.0))]);
    engine
        .set_cell_formula("Sheet1", 1, 7, parse("=0.5/86400").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 2, 7, parse("=83.4/86400").unwrap())
        .unwrap();
    assert_values(
        &mut engine,
        &[
            (
                "=ROUND(TIMEVALUE(\"12:00:00.5\")*86400,6)",
                number(43_200.5),
            ),
            ("=ROUND(TIMEVALUE(\"0:00.5\")*86400,6)", number(0.5)),
            ("=ROUND(VALUE(\"0:00.5\")*86400,6)", number(0.5)),
            ("=ROUND(VALUE(\"1:23.4\")*86400,6)", number(83.4)),
            ("=COUNTIF(G1:G2,\"0:00.5\")", number(1.0)),
            ("=SUMIF(G1:G2,\"0:00.5\",H1:H2)", number(7.0)),
            ("=SUMIF(G1:G2,\"<0:01.0\",H1:H2)", number(7.0)),
            (
                "=VALUE(\"0:60.5\")",
                LiteralValue::Error(formualizer_common::ExcelError::new_value()),
            ),
        ],
    );
}
