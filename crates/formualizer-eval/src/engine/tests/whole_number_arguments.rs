//! How Excel for Windows reads a whole-number argument given a fraction.
//! INDEX's row_num, column_num and area_num, VLOOKUP's and HLOOKUP's index,
//! ADDRESS's row and column and DATE's parts take a value within 2^-22 below a
//! whole number as that number (`floor(n + 2^-22)`), so
//! `INDEX(range,10^6*MOD(row/10^6+col,1))` lands on `row` when MOD leaves
//! 1.9999999998354667. OFFSET, CHOOSE, SMALL, MID, REPT, TIME and INT truncate
//! without that window, and LARGE rounds its k.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// A1:A3 = 10, 20, 30 and B1:B3 = 40, 50, 60.
fn engine() -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for row in 1..=3u32 {
        let n = f64::from(row) * 10.0;
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Number(n))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(n + 30.0))
            .unwrap();
    }
    engine
}

fn eval(engine: &mut Engine<TestWorkbook>, formula: &str) -> LiteralValue {
    engine
        .set_cell_formula("Sheet1", 1, 20, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_cell("Sheet1", 1, 20).unwrap();
    engine.get_cell_value("Sheet1", 1, 20).unwrap()
}

fn assert_cases(cases: &[(&str, LiteralValue)]) {
    let mut engine = engine();
    for (formula, expected) in cases {
        assert_eq!(&eval(&mut engine, formula), expected, "{formula}");
    }
}

fn n(n: f64) -> LiteralValue {
    LiteralValue::Number(n)
}

fn text(s: &str) -> LiteralValue {
    LiteralValue::Text(s.into())
}

#[test]
fn index_positions_snap_within_two_to_the_minus_22() {
    assert_cases(&[
        ("=INDEX({10;20;30},2-1E-6)", n(10.0)),
        ("=INDEX({10;20;30},2-3E-7)", n(10.0)),
        ("=INDEX({10;20;30},2-2.39E-7)", n(10.0)),
        ("=INDEX({10;20;30},2-2.38E-7)", n(20.0)),
        ("=INDEX({10;20;30},2-1E-7)", n(20.0)),
        ("=INDEX({10;20;30},2-1E-16)", n(20.0)),
        ("=INDEX({10;20;30},1.9999999985)", n(20.0)),
        ("=INDEX({10;20;30},1.5)", n(10.0)),
        ("=INDEX({10;20;30},2.5)", n(20.0)),
        ("=INDEX({10;20;30},2.9999)", n(20.0)),
        ("=INDEX({10,20,30},1,2-1E-7)", n(20.0)),
        ("=INDEX({10,20,30},1,2.9999)", n(20.0)),
        // References read the same way, area_num included.
        ("=INDEX(A1:A3,2-1E-7)", n(20.0)),
        ("=INDEX(A1:A3,2-3E-7)", n(10.0)),
        ("=INDEX(A1:B3,3-1E-7,2-1E-7)", n(60.0)),
        ("=INDEX((A1:A3,B1:B3),1,1,2-1E-7)", n(40.0)),
        ("=INDEX((A1:A3,B1:B3),1,1,2.9999)", n(40.0)),
        // The window is absolute: 2^-22 at 1999 as at 2.
        ("=INDEX(SEQUENCE(3000),1999-2.2E-7)", n(1999.0)),
        ("=INDEX(SEQUENCE(3000),1999-2.4E-7)", n(1998.0)),
        ("=INDEX(SEQUENCE(200000),100000-2.2E-7)", n(100000.0)),
        ("=INDEX(SEQUENCE(200000),100000-2.6E-7)", n(99999.0)),
    ]);
}

#[test]
fn index_of_a_mod_encoded_row_lands_on_that_row() {
    // 10^6*MOD(row/10^6+col,1) is 1.9999999998354667 for row 2, column 2 and
    // 2998.999999999974 for row 2999: each is its row.
    let mut engine = engine();
    assert_eq!(
        eval(&mut engine, "=10^6*MOD(2/10^6+2,1)<2"),
        LiteralValue::Boolean(true)
    );
    for (row, col) in [(2, 2), (3, 2), (2, 7)] {
        assert_eq!(
            eval(
                &mut engine,
                &format!("=INDEX(A1:A3,10^6*MOD({row}/10^6+{col},1))")
            ),
            n(f64::from(row) * 10.0),
            "row {row} column {col}"
        );
    }
    assert_eq!(
        eval(
            &mut engine,
            "=INDEX(SEQUENCE(3000),10^6*MOD(2999/10^6+2,1))"
        ),
        n(2999.0)
    );
}

#[test]
fn index_inside_its_range_plans_its_constant_row_as_index_reads_it() {
    // A4 lies in A:A, a range the dependency plan keeps whole and reads
    // INDEX's constant row from. Row 4.9999999 is row 5, so A4 is not its own
    // precedent; row 3.9999999 is row 4, A4 itself, exactly like row 4.
    let mut engine = engine();
    engine
        .set_cell_formula("Sheet1", 4, 1, parse("=INDEX(A:A,4.9999999)+1").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 4, 1), Some(n(1.0)));

    let set = |formula: &str| {
        let mut fresh = self::engine();
        let set = fresh
            .set_cell_formula("Sheet1", 4, 1, parse(formula).unwrap())
            .map_err(|e| e.kind);
        fresh.evaluate_all().ok();
        (set, fresh.get_cell_value("Sheet1", 4, 1))
    };
    assert_eq!(set("=INDEX(A:A,3.9999999)+1"), set("=INDEX(A:A,4)+1"));
    assert_ne!(set("=INDEX(A:A,3.9999999)+1"), set("=INDEX(A:A,3)+1"));
}

#[test]
fn index_row_just_below_one_is_a_row_not_the_whole_column() {
    // Row 1-1E-9 is row 1, so INDEX(...,1-1E-9,0) spills one row.
    let mut engine = engine();
    engine
        .set_cell_formula(
            "Sheet1",
            10,
            1,
            parse("=INDEX({1,2;3,4},1-1E-9,0)").unwrap(),
        )
        .unwrap();
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 10, 1), Some(n(1.0)));
    assert_eq!(engine.get_cell_value("Sheet1", 10, 2), Some(n(2.0)));
    assert!(matches!(
        engine.get_cell_value("Sheet1", 11, 1),
        None | Some(LiteralValue::Empty)
    ));
}

#[test]
fn lookup_index_address_and_date_snap() {
    assert_cases(&[
        ("=VLOOKUP(1,{1,10,100},2-1E-7,FALSE)", n(10.0)),
        ("=VLOOKUP(1,{1,10,100},2-3E-7,FALSE)", n(1.0)),
        ("=VLOOKUP(1,{1,10,100},2.9999,FALSE)", n(10.0)),
        ("=HLOOKUP(1,{1;10;100},2-1E-7,FALSE)", n(10.0)),
        ("=HLOOKUP(1,{1;10;100},2-3E-7,FALSE)", n(1.0)),
        ("=HLOOKUP(1,{1;10;100},2.9999,FALSE)", n(10.0)),
        ("=ADDRESS(2-1E-7,1)", text("$A$2")),
        ("=ADDRESS(2-3E-7,1)", text("$A$1")),
        ("=ADDRESS(2.9999,1)", text("$A$2")),
        ("=ADDRESS(1,2-1E-7)", text("$B$1")),
        ("=ADDRESS(1,1,2-1E-7)", text("A$1")),
        (
            "=DATE(2026,2-1E-7,1)=DATE(2026,2,1)",
            LiteralValue::Boolean(true),
        ),
        (
            "=DATE(2026,2-3E-7,1)=DATE(2026,1,1)",
            LiteralValue::Boolean(true),
        ),
        (
            "=DATE(2026,2.9999,1)=DATE(2026,2,1)",
            LiteralValue::Boolean(true),
        ),
        (
            "=DATE(2026,1,2-1E-7)=DATE(2026,1,2)",
            LiteralValue::Boolean(true),
        ),
        (
            "=DATE(2026-1E-7,1,1)=DATE(2026,1,1)",
            LiteralValue::Boolean(true),
        ),
    ]);
}

#[test]
fn truncating_arguments_keep_truncating() {
    assert_cases(&[
        ("=OFFSET(A1,2-1E-7,0)", n(20.0)),
        ("=CHOOSE(2-1E-7,\"a\",\"b\")", text("a")),
        ("=CHOOSE(2.9999,\"a\",\"b\",\"c\")", text("b")),
        ("=SMALL({10,20,30},2-1E-7)", n(10.0)),
        ("=SMALL({10,20,30},2.9999)", n(20.0)),
        ("=MID(\"abc\",2-1E-7,1)", text("a")),
        ("=REPT(\"x\",2-1E-7)", text("x")),
        ("=INT(2-1E-7)", n(1.0)),
        ("=TRUNC(2-1E-7)", n(1.0)),
        ("=ROUNDDOWN(2-1E-7,0)", n(1.0)),
        ("=TIME(2-1E-7,0,0)=TIME(1,0,0)", LiteralValue::Boolean(true)),
    ]);
}

#[test]
fn large_rounds_k_within_its_bounds() {
    let num =
        |e: &LiteralValue| matches!(e, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Num);
    assert_cases(&[
        ("=LARGE({10,20,30},2-1E-7)", n(20.0)),
        ("=LARGE({10,20,30},2.6)", n(10.0)),
        ("=LARGE({10,20,30},2.9999)", n(10.0)),
        ("=LARGE({10,20,30},2.4)", n(20.0)),
        ("=LARGE({10,20,30},1)", n(30.0)),
    ]);
    let mut engine = engine();
    assert!(num(&eval(&mut engine, "=LARGE({10,20,30},3.1)")));
    assert!(num(&eval(&mut engine, "=LARGE({10,20,30},0.6)")));
}
