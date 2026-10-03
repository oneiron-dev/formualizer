//! The whole-number readings settled by Excel for Windows 16.0.20430 (probes
//! 4-6 of ops/excel-int-coercion-probe-20261003.md), with the shapes INDEX
//! returns. One reader, `floor(x + 2^-22)` with negatives floored, serves
//! INDEX, VLOOKUP/HLOOKUP, ADDRESS, DATE, SEQUENCE and LEFT/RIGHT; ROUND's
//! digits snap toward zero, `trunc(x + 2^-22)`; LARGE (and AGGREGATE 14) take
//! the ceiling of k after its bounds; OFFSET, CHOOSE, SMALL, AGGREGATE 15, MID,
//! REPT, ROUNDUP and ROUNDDOWN truncate without a window.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// The value of `formula` in T1 of an otherwise blank sheet.
fn eval(formula: &str) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_formula("Sheet1", 1, 20, parse(formula).unwrap())
        .unwrap_or_else(|e| panic!("{formula}: {e:?}"));
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 20).unwrap()
}

fn n(n: f64) -> LiteralValue {
    LiteralValue::Number(n)
}

fn text(s: &str) -> LiteralValue {
    LiteralValue::Text(s.into())
}

fn error(kind: ExcelErrorKind) -> LiteralValue {
    LiteralValue::Error(kind.into())
}

fn same(actual: &LiteralValue, expected: &LiteralValue) -> bool {
    match (actual, expected) {
        (LiteralValue::Number(a), LiteralValue::Number(b)) => (a - b).abs() <= 1e-12 * b.abs(),
        // DATE's result is a date: compare its serial number.
        (LiteralValue::Date(_) | LiteralValue::DateTime(_), LiteralValue::Number(b)) => {
            actual.as_serial_number() == Some(*b)
        }
        (LiteralValue::Int(a), LiteralValue::Number(b)) => *a as f64 == *b,
        (LiteralValue::Error(a), LiteralValue::Error(b)) => a.kind == b.kind,
        _ => actual == expected,
    }
}

fn assert_cases(cases: &[(&str, LiteralValue)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(formula, expected)| {
            let actual = eval(formula);
            (!same(&actual, expected)).then(|| format!("{formula}: {actual:?}, not {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `formula` entered in F1 of an otherwise blank sheet: the values of the
/// `rows` x `cols` block from F1, and whether the cells just right of it and
/// just below it are empty (the spill stops there).
fn spill(formula: &str, rows: u32, cols: u32) -> (Vec<Vec<LiteralValue>>, bool) {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_formula("Sheet1", 1, 6, parse(formula).unwrap())
        .unwrap_or_else(|e| panic!("{formula}: {e:?}"));
    engine.evaluate_all().unwrap();
    let at = |row: u32, col: u32| {
        engine
            .get_cell_value("Sheet1", row, col)
            .unwrap_or(LiteralValue::Empty)
    };
    let block = (1..=rows)
        .map(|row| (6..6 + cols).map(|col| at(row, col)).collect())
        .collect();
    let blank = |v: LiteralValue| matches!(v, LiteralValue::Empty);
    let ends = blank(at(rows + 1, 6)) && blank(at(1, 6 + cols));
    (block, ends)
}

fn assert_spill(formula: &str, expected: &[&[LiteralValue]]) {
    let (rows, cols) = (expected.len() as u32, expected[0].len() as u32);
    let (block, ends) = spill(formula, rows, cols);
    for (r, row) in expected.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            assert!(
                same(&block[r][c], value),
                "{formula} at row {} column {}: {:?}, not {value:?}",
                r + 1,
                c + 1,
                block[r][c]
            );
        }
    }
    assert!(ends, "{formula}: the result is larger than {rows}x{cols}");
}

#[test]
fn confirmed_snaps() {
    assert_cases(&[
        ("=ADDRESS(1,2-1E-7)", text("$B$1")),
        ("=ADDRESS(1,2-3E-7)", text("$A$1")),
        ("=ADDRESS(1,1,2-1E-7)", text("A$1")),
        ("=ADDRESS(1,1,2-3E-7)", text("$A$1")),
        ("=DATE(2026-1E-7,1,1)", n(46023.0)),
        ("=DATE(2026-3E-7,1,1)", n(45658.0)),
        ("=DATE(2026,1,2-1E-7)", n(46024.0)),
        ("=DATE(2026,1,2-3E-7)", n(46023.0)),
        ("=COLUMN(INDEX((A1:A3,B1:B3),1,1,2-1E-7))", n(2.0)),
        ("=COLUMN(INDEX((A1:A3,B1:B3),1,1,2-3E-7))", n(1.0)),
        ("=ROW(INDEX(A1:A3,2-1E-7))", n(2.0)),
    ]);
}

#[test]
fn large_takes_the_ceiling_of_k_within_its_bounds() {
    use ExcelErrorKind::Num;
    assert_cases(&[
        ("=LARGE({10,20,30},2.1)", n(10.0)),
        ("=LARGE({10,20,30},2.4)", n(10.0)),
        ("=LARGE({10,20,30},2.5)", n(10.0)),
        ("=LARGE({10,20,30},2.6)", n(10.0)),
        ("=LARGE({10,20,30},1.5)", n(20.0)),
        ("=LARGE({10,20,30},1.0000001)", n(20.0)),
        ("=LARGE({10,20,30},2-1E-7)", n(20.0)),
        ("=LARGE({10,20,30},0.6)", error(Num)),
        ("=LARGE({10,20,30},3.0000001)", error(Num)),
        ("=AGGREGATE(14,6,{10,20,30},2.1)", n(10.0)),
        ("=AGGREGATE(14,6,{10,20,30},2.4)", n(10.0)),
        ("=AGGREGATE(14,6,{10,20,30},2.6)", n(10.0)),
        ("=AGGREGATE(14,6,{10,20,30},3.0000001)", error(Num)),
        ("=AGGREGATE(14,6,{10,20,30},0.6)", error(Num)),
    ]);
}

#[test]
fn snapped_negatives_are_floored() {
    use ExcelErrorKind::Value;
    assert_cases(&[
        ("=INDEX({10;20;30},-0.5)", error(Value)),
        ("=INDEX({10;20;30},-0.9999999)", error(Value)),
        ("=INDEX({10;20;30},-1.5)", error(Value)),
        ("=DATE(2026,-0.5,1)", n(45962.0)),
        ("=DATE(2026,1,-0.5)", n(46021.0)),
        ("=DATE(2026,-1E-9,1)", n(45992.0)),
    ]);
    assert_spill(
        "=INDEX({10;20;30},-1E-9)",
        &[&[n(10.0)], &[n(20.0)], &[n(30.0)]],
    );
}

#[test]
fn sequence_left_right_and_round_snap() {
    use ExcelErrorKind::Value;
    assert_cases(&[
        ("=ROWS(SEQUENCE(2-1E-7))", n(2.0)),
        ("=ROWS(SEQUENCE(2-3E-7))", n(1.0)),
        ("=ROWS(SEQUENCE(2.4))", n(2.0)),
        ("=ROWS(SEQUENCE(2.6))", n(2.0)),
        ("=ROWS(SEQUENCE(1.5))", n(1.0)),
        ("=COLUMNS(SEQUENCE(1,2-1E-7))", n(2.0)),
        ("=ROWS(SEQUENCE(-0.5))", error(Value)),
        ("=LEFT(\"abcd\",2-1E-7)", text("ab")),
        ("=LEFT(\"abcd\",2-3E-7)", text("a")),
        ("=LEFT(\"abcd\",2.4)", text("ab")),
        ("=LEFT(\"abcd\",2.6)", text("ab")),
        ("=LEFT(\"abcd\",-0.5)", error(Value)),
        ("=RIGHT(\"abcd\",2-1E-7)", text("cd")),
        ("=RIGHT(\"abcd\",2.6)", text("cd")),
        ("=ROUND(1.23456,2-1E-7)", n(1.23)),
        ("=ROUND(1.23456,2-3E-7)", n(1.2)),
        ("=ROUND(1.23456,2.4)", n(1.23)),
        ("=ROUND(1.23456,2.6)", n(1.23)),
        ("=ROUND(1234.5,-0.5)", n(1235.0)),
        ("=ROUND(1234.5678,-2)", n(1200.0)),
        ("=ROUND(1234.5678,-2-1E-7)", n(1200.0)),
        ("=ROUND(1234.5678,-2+1E-7)", n(1230.0)),
        // Numeric text converts before the whole-number reading.
        ("=LEFT(\"abcd\",\"1.9999999\")", text("ab")),
        ("=RIGHT(\"abcd\",\"2.6\")", text("cd")),
        ("=LEFT(\"abcd\",\"bad\")", error(Value)),
        ("=ADDRESS(\"1.9999999\",1)", text("$A$2")),
        ("=ADDRESS(1,\"1.9999999\")", text("$B$1")),
        ("=ADDRESS(1,1,\"1.9999999\")", text("A$1")),
        ("=ADDRESS(\"x\",1)", error(Value)),
        ("=ADDRESS(1,1,\"x\")", error(Value)),
    ]);
}

#[test]
fn truncating_arguments_have_no_window() {
    assert_cases(&[
        ("=COLUMN(OFFSET(A1,0,2-1E-7))", n(2.0)),
        ("=ROW(OFFSET(A1,2-1E-7,0))", n(2.0)),
        ("=ROW(OFFSET(A5,-0.5,0))", n(5.0)),
        ("=CHOOSE(2-1E-7,\"a\",\"b\",\"c\")", text("a")),
        ("=CHOOSE(2.6,\"a\",\"b\",\"c\")", text("b")),
        ("=MID(\"abc\",2-1E-7,1)", text("a")),
        ("=MID(\"abcd\",2.6,1)", text("b")),
        ("=MID(\"abcd\",1,2-1E-7)", text("a")),
        ("=MID(\"abcd\",1,2.6)", text("ab")),
        ("=REPT(\"x\",2-1E-7)", text("x")),
        ("=REPT(\"x\",2.6)", text("xx")),
        ("=SMALL({10,20,30},2.5)", n(20.0)),
        ("=AGGREGATE(15,6,{10,20,30},2.6)", n(20.0)),
        ("=AGGREGATE(15,6,{10,20,30},2-1E-7)", n(10.0)),
        ("=ROUNDUP(1.23456,2-1E-7)", n(1.3)),
        ("=ROUNDUP(1.23456,2.6)", n(1.24)),
        ("=ROUNDUP(1234.5,-0.5)", n(1235.0)),
        ("=ROUNDDOWN(1.23456,2.6)", n(1.23)),
        ("=VLOOKUP(1,{1,10,20},2.6,FALSE)", n(10.0)),
    ]);
}

#[test]
fn index_of_its_own_column_selecting_another_row_is_no_cycle() {
    // F1 =INDEX(F:F,2-1E-7)+1 reads F2, not F1: 1, no circular reference.
    // Under the default configuration (runtime cycle detection, #CIRC! for a
    // live cycle) a formula is circular only when it reads its own cell.
    assert_spill("=INDEX(F:F,2-1E-7)+1", &[&[n(1.0)]]);

    // F1 holds `formula`, G1 the selector `g1`, F2 the formula `f2`.
    let run = |formula: &str, g1: Option<&str>, f2: Option<&str>| {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        for (row, col, f) in [(1, 7, g1), (2, 6, f2)] {
            if let Some(f) = f {
                engine
                    .set_cell_formula("Sheet1", row, col, parse(f).unwrap())
                    .unwrap();
            }
        }
        engine
            .set_cell_formula("Sheet1", 1, 6, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_all().unwrap();
        engine.get_cell_value("Sheet1", 1, 6).unwrap()
    };
    let circ = error(ExcelErrorKind::Circ);
    for (formula, g1, f2, expected) in [
        ("=INDEX(F:F,2-1E-7)+1", None, Some("=41"), n(42.0)),
        ("=INDEX(F:F,1)+1", None, None, circ.clone()),
        ("=INDEX(F:F,1-1E-7)+1", None, None, circ.clone()),
        ("=INDEX(F:F,G1)+1", Some("=2"), None, n(1.0)),
        ("=INDEX(F:F,G1)+1", Some("=1"), None, circ.clone()),
        ("=INDEX(F:F,G1)+SUM(F:F)", Some("=2"), None, circ),
    ] {
        let actual = run(formula, g1, f2);
        assert!(
            same(&actual, &expected),
            "{formula} with G1 {g1:?}, F2 {f2:?}: {actual:?}, not {expected:?}"
        );
    }
}

/// `formula` in Result!J2 of a workbook whose Sheet1 holds A1 = 11.
fn on_result_sheet(formula: &str) -> Engine<TestWorkbook> {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Number(11.0))
        .unwrap();
    engine.add_sheet("Result").unwrap();
    engine
        .set_cell_formula("Result", 2, 10, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine
}

#[test]
fn index_result_extent_matches_what_index_selects() {
    // A whole column or row INDEX selects does not fit from J2: #SPILL!, the
    // same for row 0 written as numeric text or under two signs, for a lone
    // index into a one-row source (it selects columns) and for a lone row
    // index into a two-dimensional source (the entire row).
    for formula in [
        "=INDEX(Sheet1!A:A,0,1)+0",
        "=INDEX(Sheet1!A:A,\"0\",1)+0",
        "=INDEX(Sheet1!A:A,--0,1)+0",
        "=INDEX(Sheet1!A:A,,1)+0",
        "=INDEX(Sheet1!1:1,0.9)+0",
        "=INDEX(Sheet1!$1:$1048576,1.9999999)+0",
        "=INDEX(Sheet1!$1:$1048576,2,)+0",
        // A scalar area_num is one area, however it is written.
        "=INDEX(Sheet1!A:A,0,1,ABS(1))+0",
        "=INDEX(Sheet1!A:A,0,1,--1)+0",
        "=IFERROR(INDEX(Sheet1!A:A,0,1,\"1\"),99)",
    ] {
        let engine = on_result_sheet(formula);
        let value = engine.get_cell_value("Result", 2, 10);
        assert!(
            matches!(&value, Some(LiteralValue::Error(e)) if e.kind == ExcelErrorKind::Spill),
            "{formula}: {value:?}"
        );
    }
    // A lone index into a one-row source, or an explicit row and column, is
    // one cell.
    for formula in ["=INDEX(Sheet1!1:1,1)+0", "=INDEX(Sheet1!A:A,1,1)+0"] {
        let engine = on_result_sheet(formula);
        assert_eq!(
            engine.get_cell_value("Result", 2, 10),
            Some(n(11.0)),
            "{formula}"
        );
    }
    // An array area_num is lifted: one value per element, not a column each,
    // however the array is written.
    for formula in [
        "=INDEX(Sheet1!A:A,0,1,{1,1})",
        "=INDEX(Sheet1!A:A,0,1,SEQUENCE(1,2,1,0))+0",
        "=INDEX(Sheet1!A:A,0,1,FILTER({1,1},{TRUE,TRUE}))+0",
    ] {
        let engine = on_result_sheet(formula);
        assert_eq!(
            engine.get_cell_value("Result", 2, 10),
            Some(n(11.0)),
            "{formula}"
        );
        assert_eq!(
            engine.get_cell_value("Result", 2, 11),
            Some(n(11.0)),
            "{formula}"
        );
        assert!(
            matches!(
                engine.get_cell_value("Result", 3, 10),
                None | Some(LiteralValue::Empty)
            ),
            "{formula}"
        );
    }
}

#[test]
fn index_shapes() {
    // Numeric text "0" is row 0, the whole column.
    assert_spill(
        "=INDEX({10;20;30},\"0\")",
        &[&[n(10.0)], &[n(20.0)], &[n(30.0)]],
    );
    // One index into a one-row array selects a column: a scalar.
    assert_spill("=INDEX({10,20,30},1)", &[&[n(10.0)]]);
    assert_spill("=INDEX({10,20,30},2)", &[&[n(20.0)]]);
    // An array row_num gives one value per element.
    assert_spill("=INDEX({10;20;30},{1;2})", &[&[n(10.0)], &[n(20.0)]]);
    // An array area_num lifts per element and gives values, not references.
    assert_spill("=INDEX((A1:A3,B1:B3),1,1,{1,2})", &[&[n(0.0), n(0.0)]]);
    assert_spill(
        "=INDEX((A1:A3,B1:B3),{1;2},1,{1,2})",
        &[&[n(0.0), n(0.0)], &[n(0.0), n(0.0)]],
    );
    let value = || error(ExcelErrorKind::Value);
    assert_spill(
        "=COLUMN(INDEX((A1:A3,B1:B3),1,1,{1,2}))",
        &[&[value(), value()]],
    );
}
