//! Excel for Windows 16.0.20430's values for the three upstream parity picks of round 7
//! (ops/excel-upstream-picks-probe-20261008.md, job probe-upstream-picks-1): `^` groups left
//! to right (upstream 2475598b), a ragged array literal is not a formula (a3a5d796), and
//! XLOOKUP compares the lengths its lookup and return arrays declare before it searches
//! (c2c724d7), with the review's cases (job probe-upstream-picks-3: a reference a function
//! returns, approximate matches on a single cell, the range operator over values). Each formula as typed in F1 (or the first cell of the range it spills to) of a
//! sheet whose Z1 holds the recorder's =1111+2222; a defined name as the Name Manager holds
//! it, defined the way the workbook loader defines it (a range address as a range name, open
//! bounds reaching the sheet's edge; anything else as a formula name). Numbers to 1e-12
//! relative, errors by kind, a spill cell by cell.

use crate::engine::named_range::{NameScope, NamedDefinition};
use crate::engine::{Engine, EvalConfig};
use crate::reference::{CellRef, Coord, RangeRef};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ReferenceType, parse};

/// An expected cell.
enum V {
    N(f64),
    T(&'static str),
    B(bool),
    E(ExcelErrorKind),
    Blank,
}

/// Formula, numbers set first, defined names, the formula's cell (or spill range), Excel's values.
type Case = (
    &'static str,
    &'static [(&'static str, f64)],
    &'static [(&'static str, &'static str)],
    &'static str,
    &'static [&'static [V]],
);

/// The XLOOKUP cases' sheet: A1:A5 = 1..5, B1:B5 = 10..50, C1:C5 = 100..500, A8:C8 = 1, 2, 3
/// and A9:D9 = 10, 20, 30, 40 (row 6, column E and the rest blank).
const GRID: &[(&str, f64)] = &[
    ("A1", 1.0),
    ("B1", 10.0),
    ("C1", 100.0),
    ("A2", 2.0),
    ("B2", 20.0),
    ("C2", 200.0),
    ("A3", 3.0),
    ("B3", 30.0),
    ("C3", 300.0),
    ("A4", 4.0),
    ("B4", 40.0),
    ("C4", 400.0),
    ("A5", 5.0),
    ("B5", 50.0),
    ("C5", 500.0),
    ("A8", 1.0),
    ("B8", 2.0),
    ("C8", 3.0),
    ("A9", 10.0),
    ("B9", 20.0),
    ("C9", 30.0),
    ("D9", 40.0),
];

fn cell(address: &str) -> (u32, u32) {
    let letters: String = address
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let row = address[letters.len()..].parse().unwrap();
    let col = letters
        .bytes()
        .fold(0, |acc, b| acc * 26 + u32::from(b - b'A' + 1));
    (row, col)
}

fn matches(actual: Option<&LiteralValue>, expected: &V) -> bool {
    match (actual, expected) {
        (None | Some(LiteralValue::Empty), V::Blank) => true,
        (Some(LiteralValue::Number(a)), V::N(b)) => a == b || (a - b).abs() <= 1e-12 * b.abs(),
        (Some(LiteralValue::Int(a)), V::N(b)) => *a as f64 == *b,
        (Some(LiteralValue::Text(a)), V::T(b)) => a == b,
        (Some(LiteralValue::Boolean(a)), V::B(b)) => a == b,
        (Some(LiteralValue::Error(a)), V::E(b)) => a.kind == *b,
        _ => false,
    }
}

/// A name over a cell or range address is a cell or range name, as `convert_defined_name` in
/// the Calamine backend loads it (`Sheet1!$A:$A` is rows 1 to 1,048,576); anything else is a
/// formula name.
fn define(
    engine: &mut Engine<TestWorkbook>,
    name: &str,
    refers: &str,
) -> Result<(), formualizer_common::ExcelError> {
    let definition = match ReferenceType::from_string(refers.trim_start_matches('=')) {
        Ok(ReferenceType::Cell {
            sheet: Some(sheet),
            row,
            col,
            ..
        }) => {
            let id = engine.sheet_id(&sheet).unwrap();
            NamedDefinition::Cell(CellRef::new(id, Coord::new(row - 1, col - 1, true, true)))
        }
        Ok(ReferenceType::Range {
            sheet: Some(sheet),
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        }) => {
            let id = engine.sheet_id(&sheet).unwrap();
            let rows = match (start_row, end_row) {
                (None, None) => (1, 1_048_576),
                (first, last) => (first.unwrap(), last.unwrap()),
            };
            let cols = match (start_col, end_col) {
                (None, None) => (1, 16_384),
                (first, last) => (first.unwrap(), last.unwrap()),
            };
            NamedDefinition::Range(RangeRef::new(
                CellRef::new(id, Coord::new(rows.0 - 1, cols.0 - 1, true, true)),
                CellRef::new(id, Coord::new(rows.1 - 1, cols.1 - 1, true, true)),
            ))
        }
        _ => NamedDefinition::Formula {
            ast: parse(refers).unwrap(),
            dependencies: Vec::new(),
            range_deps: Vec::new(),
        },
    };
    engine.define_name(name, definition, NameScope::Workbook)
}

fn check(cases: &[Case]) {
    let mut failures = Vec::new();
    'case: for (formula, setup, names, range, expected) in cases {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        for (address, number) in setup.iter() {
            let (row, col) = cell(address);
            engine
                .set_cell_value("Sheet1", row, col, LiteralValue::Number(*number))
                .unwrap();
        }
        engine
            .set_cell_formula("Sheet1", 1, 26, parse("=1111+2222").unwrap())
            .unwrap();
        for (name, refers) in names.iter() {
            if let Err(e) = define(&mut engine, name, refers) {
                failures.push(format!("{formula}: defining {name}: {e:?}"));
                continue 'case;
            }
        }
        let (top, left) = cell(range.split(':').next().unwrap());
        if let Err(e) = engine.set_cell_formula("Sheet1", top, left, parse(formula).unwrap()) {
            failures.push(format!("{formula}: {e:?}"));
            continue;
        }
        if let Err(e) = engine.evaluate_all() {
            failures.push(format!("{formula}: evaluate_all: {e:?}"));
            continue;
        }
        for (r, row) in expected.iter().enumerate() {
            for (c, want) in row.iter().enumerate() {
                let got = engine.get_cell_value("Sheet1", top + r as u32, left + c as u32);
                if !matches(got.as_ref(), want) {
                    failures.push(format!("{formula} at +{r},+{c}: {got:?}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn exponent_groups_left_to_right() {
    check(&[
        // E01: left: (2^3)^2=64; right: 512
        ("=2^3^2", &[], &[], "F1", &[&[V::N(64.0)]]),
        // E02: explicit right grouping 512
        ("=2^(3^2)", &[], &[], "F1", &[&[V::N(512.0)]]),
        // E03: explicit left grouping 64
        ("=(2^3)^2", &[], &[], "F1", &[&[V::N(64.0)]]),
        // E04: unary minus binds tighter: 4
        ("=-2^2", &[], &[], "F1", &[&[V::N(4.0)]]),
        // E05: left: (2^-1)^2=0.25; right: 2^((-1)^2)=2
        ("=2^-1^2", &[], &[], "F1", &[&[V::N(0.25)]]),
        // E06: (-2)^(-2)=0.25
        ("=-2^-2", &[], &[], "F1", &[&[V::N(0.25)]]),
        // E07: left 4; right 4^0.25=1.414
        ("=4^0.5^2", &[], &[], "F1", &[&[V::N(4.0)]]),
        // E08: left ((-2)^2)^3=64; right (-2)^8=256
        ("=-2^2^3", &[], &[], "F1", &[&[V::N(64.0)]]),
        // E09: left 8^-2=0.015625; right 2^(1/9)
        ("=2^3^-2", &[], &[], "F1", &[&[V::N(0.015625)]]),
        // E10: binary minus: -4
        ("=0-2^2", &[], &[], "F1", &[&[V::N(-4.0)]]),
        // E11: 18
        ("=2*3^2", &[], &[], "F1", &[&[V::N(18.0)]]),
        // E12: % before ^: 2^0.03
        ("=2^3%", &[], &[], "F1", &[&[V::N(1.0210121257071934)]]),
        // E13: left ((2^3)^2)^0.5=8; right 2^(3^(2^0.5))
        ("=2^3^2^0.5", &[], &[], "F1", &[&[V::N(8.0)]]),
        // E14: left (10^-2)^-1=100; right 10^(-0.5)
        ("=10^-2^-1", &[], &[], "F1", &[&[V::N(100.0)]]),
        // E15: left 64; right -512
        ("=(-2)^3^2", &[], &[], "F1", &[&[V::N(64.0)]]),
        // E16: text operand: 64
        ("=\"2\"^3^2", &[], &[], "F1", &[&[V::N(64.0)]]),
        // E17: inside a call: 65
        ("=SUM(2^3^2,1)", &[], &[], "F1", &[&[V::N(65.0)]]),
        // E18: comparison after ^: TRUE
        ("=2^3^2=64", &[], &[], "F1", &[&[V::B(true)]]),
        // E19: concatenation after ^: "64"
        ("=2^3^2&\"\"", &[], &[], "F1", &[&[V::T("64")]]),
        // E20: (-2)^0.5 #NUM!
        ("=-2^0.5", &[], &[], "F1", &[&[V::E(ExcelErrorKind::Num)]]),
        // E21: cells 2,3,2: 64
        (
            "=A1^A2^A3",
            &[("A1", 2.0), ("A2", 3.0), ("A3", 2.0)],
            &[],
            "F1",
            &[&[V::N(64.0)]],
        ),
        // E22: array: left {4,16}; right {2,16}
        ("=2^{1,2}^2", &[], &[], "F1:G1", &[&[V::N(4.0), V::N(16.0)]]),
    ]);
}

#[test]
fn ragged_array_literals_are_not_formulas() {
    // Excel refuses each of these as a cell formula (Range.Formula2 raises), and the
    // array ones as a defined name too (Names.Add raises); the loader leaves a name it
    // cannot parse undefined.
    let parsed: Vec<&str> = [
        "={1,2;3}",
        "={1;2,3}",
        "=SUM({1,2;3})",
        "=ROWS({1,2;3})",
        "=IFERROR(SUM({1,2;3}),0)",
        "={1;;2}",
        "={1,2;3,}",
        "={1,,2}",
    ]
    .into_iter()
    .chain(["={1,2;3}", "=SUM({1,2;3})", "={1,2;3,4;5}", "={1;2,3}"])
    .filter(|formula| parse(formula).is_ok())
    .collect();
    assert!(parsed.is_empty(), "parsed: {parsed:?}");
    check(&[
        // R09: rectangular control spills
        (
            "={1,2;3,4}",
            &[],
            &[],
            "F1:G2",
            &[&[V::N(1.0), V::N(2.0)], &[V::N(3.0), V::N(4.0)]],
        ),
        // R10: name Rag = {1,2;3}
        (
            "=SUM(Rag)",
            &[],
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Name)]],
        ),
        // R11: name Rag2 = {1;2,3}
        ("=ROWS(Rag2)", &[], &[], "F1", &[&[V::N(1.0)]]),
        // R12: name Grid = {1,2;3,4} control
        (
            "=SUM(Grid)",
            &[],
            &[("Grid", "={1,2;3,4}")],
            "F1",
            &[&[V::N(10.0)]],
        ),
        // R13: name RagSum = SUM({1,2;3})
        (
            "=SUM(RagSum)",
            &[],
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Name)]],
        ),
        // R14: name RagTwo = {1;2,3}
        (
            "=ROWS(RagTwo)",
            &[],
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Name)]],
        ),
        // R15: name RagThree = {1,2;3,4;5}
        (
            "=SUM(RagThree)",
            &[],
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Name)]],
        ),
    ]);
}

#[test]
fn xlookup_declared_lengths() {
    check(&[
        // X01: 3 vs 4 rows
        (
            "=XLOOKUP(2,A1:A3,B1:B4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X02: with if_not_found, value present
        (
            "=XLOOKUP(2,A1:A3,B1:B4,\"nf\")",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X03: with if_not_found, value absent
        (
            "=XLOOKUP(9,A1:A3,B1:B4,\"nf\")",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X04: search last to first
        (
            "=XLOOKUP(2,A1:A3,B1:B4,,0,-1)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X05: binary search
        (
            "=XLOOKUP(2,A1:A3,B1:B4,,0,2)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X06: wildcard mode
        (
            "=XLOOKUP(2,A1:A3,B1:B4,,2)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X07: 3 vs 2 rows
        (
            "=XLOOKUP(2,A1:A3,B1:B2)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X08: blank trailing lookup cell A6
        (
            "=XLOOKUP(2,A1:A6,B1:B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X09: blank trailing return cell B6
        (
            "=XLOOKUP(2,A1:A5,B1:B6)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X10: both with blank tail, equal: 20
        ("=XLOOKUP(2,A1:A6,B1:B6)", GRID, &[], "F1", &[&[V::N(20.0)]]),
        // X11: whole column vs bounded
        (
            "=XLOOKUP(2,A:A,B1:B10)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X12: whole column vs used length
        (
            "=XLOOKUP(2,A:A,B1:B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X13: bounded vs whole column
        (
            "=XLOOKUP(2,A1:A5,B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X14: whole vs whole: 20
        ("=XLOOKUP(2,A:A,B:B)", GRID, &[], "F1", &[&[V::N(20.0)]]),
        // X15: full-height bounded vs whole: 20
        (
            "=XLOOKUP(2,A1:A1048576,B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // X16: one short of whole
        (
            "=XLOOKUP(2,A2:A1048576,B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X17: whole vs bounded, absent, if_not_found
        (
            "=XLOOKUP(9,A:A,B1:B10,\"nf\")",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X18: row 3 vs 4 columns
        (
            "=XLOOKUP(2,A8:C8,A9:D9)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X19: whole row vs bounded
        (
            "=XLOOKUP(2,8:8,A9:D9)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X20: whole row vs whole row: 20
        ("=XLOOKUP(2,8:8,9:9)", GRID, &[], "F1", &[&[V::N(20.0)]]),
        // X21: row 3 vs 3: 20
        ("=XLOOKUP(2,A8:C8,A9:C9)", GRID, &[], "F1", &[&[V::N(20.0)]]),
        // X22: 2-D return, 3 vs 4 rows
        (
            "=XLOOKUP(2,A1:A3,B1:C4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X23: return array constant 2 rows
        (
            "=XLOOKUP(2,A1:A3,{10;20})",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X24: lookup array constant 3 vs 4
        (
            "=XLOOKUP(2,{1;2;3},B1:B4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X25: array constant equal: 20
        (
            "=XLOOKUP(2,A1:A3,{10;20;30})",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // X26: computed return array 4 rows
        (
            "=XLOOKUP(2,A1:A3,B1:B4*1)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X27: empty lookup range 3 vs 4
        (
            "=XLOOKUP(2,E1:E3,B1:B4,\"nf\")",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X28: empty whole column vs whole: nf
        (
            "=XLOOKUP(2,E:E,B:B,\"nf\")",
            GRID,
            &[],
            "F1",
            &[&[V::T("nf")]],
        ),
        // X29: reference-returning lookup array 3 vs 4
        (
            "=XLOOKUP(2,OFFSET(A1,0,0,3,1),B1:B4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X30: IF-returned lookup array 3 vs 4
        (
            "=XLOOKUP(2,IF(TRUE,A1:A3),B1:B4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X31: offset return equal: 30
        ("=XLOOKUP(2,A1:A3,B2:B4)", GRID, &[], "F1", &[&[V::N(30.0)]]),
        // X32: single-cell lookup vs 2 rows
        ("=XLOOKUP(1,A1,B1:B2)", GRID, &[], "F1", &[&[V::N(10.0)]]),
        // X33: horizontal lookup vs vertical return
        (
            "=XLOOKUP(2,A8:C8,B1:B3)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X34: vertical lookup vs horizontal return
        (
            "=XLOOKUP(2,A1:A3,A9:C9)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X35: 2-D lookup array
        (
            "=XLOOKUP(2,A1:B3,B1:B3)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X36: 2-D return row, equal: 20
        (
            "=INDEX(XLOOKUP(2,A1:A3,B1:C3),1,1)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // X37: IFERROR catches the shape error
        (
            "=IFERROR(XLOOKUP(2,A1:A3,B1:B4),\"caught\")",
            GRID,
            &[],
            "F1",
            &[&[V::T("caught")]],
        ),
        // X38: LET local lookup 3 vs 4
        (
            "=LET(r,A1:A3,XLOOKUP(2,r,B1:B4))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X39: LET, absent, if_not_found
        (
            "=LET(r,A1:A3,XLOOKUP(9,r,B1:B4,\"nf\"))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X40: LET equal: 20
        (
            "=LET(r,A1:A3,XLOOKUP(2,r,B1:B3))",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // X41: LET whole column vs bounded
        (
            "=LET(r,A:A,XLOOKUP(2,r,B1:B10))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X42: LET whole vs whole: 20
        (
            "=LET(r,A:A,s,B:B,XLOOKUP(2,r,s))",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // X43: LET local return 3 vs 4
        (
            "=LET(s,B1:B4,XLOOKUP(2,A1:A3,s))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X44: LAMBDA parameters 3 vs 4
        (
            "=LAMBDA(r,s,XLOOKUP(2,r,s))(A1:A3,B1:B4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X45: reference result, equal: 140
        (
            "=SUM(XLOOKUP(2,A1:A5,B1:B5):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(140.0)]],
        ),
        // X46: reference result, 3 vs 4
        (
            "=SUM(XLOOKUP(2,A1:A3,B1:B4):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X47: reference result, 3 vs 4, if_not_found
        (
            "=SUM(XLOOKUP(2,A1:A3,B1:B4,\"nf\"):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X48: reference result as a formula, 3 vs 4
        (
            "=XLOOKUP(2,A1:A3,B1:B4):B5",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X49: reference from whole columns: 4
        (
            "=ROWS(XLOOKUP(2,A:A,B:B):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(4.0)]],
        ),
        // X50: reference, whole vs bounded
        (
            "=ROWS(XLOOKUP(2,A:A,B1:B10):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X51: ISREF of a match: TRUE
        (
            "=ISREF(XLOOKUP(2,A1:A5,B1:B5))",
            GRID,
            &[],
            "F1",
            &[&[V::B(true)]],
        ),
        // X52: ISREF of the shape error
        (
            "=ISREF(XLOOKUP(2,A1:A3,B1:B4))",
            GRID,
            &[],
            "F1",
            &[&[V::B(false)]],
        ),
        // X53: LET reference, 3 vs 4
        (
            "=SUM(LET(r,A1:A3,XLOOKUP(2,r,B1:B4):B5))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // X54: LET reference, equal: 140
        (
            "=SUM(LET(r,A1:A5,XLOOKUP(2,r,B1:B5):B5))",
            GRID,
            &[],
            "F1",
            &[&[V::N(140.0)]],
        ),
        // X55: reference result spills 20..50
        (
            "=XLOOKUP(2,A1:A5,B1:B5):B5",
            GRID,
            &[],
            "F1:F4",
            &[&[V::N(20.0)], &[V::N(30.0)], &[V::N(40.0)], &[V::N(50.0)]],
        ),
        // X56: 2-D return spills the row
        (
            "=XLOOKUP(2,A1:A3,B1:C3)",
            GRID,
            &[],
            "F1:G1",
            &[&[V::N(20.0), V::N(200.0)]],
        ),
        // X57: single-cell lookup, row return spills
        (
            "=XLOOKUP(2,A2,B2:C2)",
            GRID,
            &[],
            "F1:G1",
            &[&[V::N(20.0), V::N(200.0)]],
        ),
        // N01: names 3 vs 4
        (
            "=XLOOKUP(2,L3,R4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Na)]],
        ),
        // N02: names 3 vs 4, absent, if_not_found
        (
            "=XLOOKUP(9,L3,R4,\"nf\")",
            GRID,
            &[],
            "F1",
            &[&[V::T("nf")]],
        ),
        // N03: names equal: 20
        (
            "=XLOOKUP(2,L3,R3)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Na)]],
        ),
        // N04: names whole column vs bounded
        (
            "=XLOOKUP(2,LA,R10)",
            GRID,
            &[("LA", "=Sheet1!$A:$A")],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // N05: names whole vs whole: 20
        (
            "=XLOOKUP(2,LA,RB)",
            GRID,
            &[("LA", "=Sheet1!$A:$A"), ("RB", "=Sheet1!$B:$B")],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // N06: names, blank trailing lookup cell
        (
            "=XLOOKUP(2,L6,R5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Na)]],
        ),
        // N07: OFFSET name 3 vs 4
        (
            "=XLOOKUP(2,LO,R4)",
            GRID,
            &[("LO", "=OFFSET(Sheet1!$A$1,0,0,3,1)")],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // N08: names, reference result, 3 vs 4
        (
            "=SUM(XLOOKUP(2,L3,R4):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Na)]],
        ),
        // N09: names, reference result: 140
        (
            "=SUM(XLOOKUP(2,LA,RB):B5)",
            GRID,
            &[("LA", "=Sheet1!$A:$A"), ("RB", "=Sheet1!$B:$B")],
            "F1",
            &[&[V::N(140.0)]],
        ),
        // M01: names 3 vs 4
        (
            "=XLOOKUP(2,LookA3,RetB4)",
            GRID,
            &[
                ("LookA3", "=Sheet1!$A$1:$A$3"),
                ("RetB4", "=Sheet1!$B$1:$B$4"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M02: names 3 vs 4, absent, if_not_found
        (
            "=XLOOKUP(9,LookA3,RetB4,\"nf\")",
            GRID,
            &[
                ("LookA3", "=Sheet1!$A$1:$A$3"),
                ("RetB4", "=Sheet1!$B$1:$B$4"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M03: names equal: 20
        (
            "=XLOOKUP(2,LookA3,RetB3)",
            GRID,
            &[
                ("LookA3", "=Sheet1!$A$1:$A$3"),
                ("RetB3", "=Sheet1!$B$1:$B$3"),
            ],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // M04: names whole column vs bounded
        (
            "=XLOOKUP(2,LookCol,RetB10)",
            GRID,
            &[
                ("LookCol", "=Sheet1!$A:$A"),
                ("RetB10", "=Sheet1!$B$1:$B$10"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M05: names whole vs whole: 20
        (
            "=XLOOKUP(2,LookCol,RetCol)",
            GRID,
            &[("LookCol", "=Sheet1!$A:$A"), ("RetCol", "=Sheet1!$B:$B")],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // M06: names, blank trailing lookup cell
        (
            "=XLOOKUP(2,LookA6,RetB5)",
            GRID,
            &[
                ("LookA6", "=Sheet1!$A$1:$A$6"),
                ("RetB5", "=Sheet1!$B$1:$B$5"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M07: names, both blank tail, equal: 20
        (
            "=XLOOKUP(2,LookA6,RetB6)",
            GRID,
            &[
                ("LookA6", "=Sheet1!$A$1:$A$6"),
                ("RetB6", "=Sheet1!$B$1:$B$6"),
            ],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // M08: OFFSET name 3 vs 4
        (
            "=XLOOKUP(2,LookOff,RetB4)",
            GRID,
            &[
                ("LookOff", "=OFFSET(Sheet1!$A$1,0,0,3,1)"),
                ("RetB4", "=Sheet1!$B$1:$B$4"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M09: INDEX:INDEX name 3 vs 4
        (
            "=XLOOKUP(2,LookIdx,RetB4)",
            GRID,
            &[
                (
                    "LookIdx",
                    "=INDEX(Sheet1!$A$1:$A$5,1):INDEX(Sheet1!$A$1:$A$5,3)",
                ),
                ("RetB4", "=Sheet1!$B$1:$B$4"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M10: INDEX:INDEX name equal: 20
        (
            "=XLOOKUP(2,LookIdx,RetB3)",
            GRID,
            &[
                (
                    "LookIdx",
                    "=INDEX(Sheet1!$A$1:$A$5,1):INDEX(Sheet1!$A$1:$A$5,3)",
                ),
                ("RetB3", "=Sheet1!$B$1:$B$3"),
            ],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // M11: name vs written range 3 vs 4
        (
            "=XLOOKUP(2,LookA3,B1:B4)",
            GRID,
            &[("LookA3", "=Sheet1!$A$1:$A$3")],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M12: whole-column name vs written bounded
        (
            "=XLOOKUP(2,LookCol,B1:B10)",
            GRID,
            &[("LookCol", "=Sheet1!$A:$A")],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M13: names, reference result, 3 vs 4
        (
            "=SUM(XLOOKUP(2,LookA3,RetB4):B5)",
            GRID,
            &[
                ("LookA3", "=Sheet1!$A$1:$A$3"),
                ("RetB4", "=Sheet1!$B$1:$B$4"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // M14: names, reference result, equal: 140
        (
            "=SUM(XLOOKUP(2,LookA3,RetB3):B5)",
            GRID,
            &[
                ("LookA3", "=Sheet1!$A$1:$A$3"),
                ("RetB3", "=Sheet1!$B$1:$B$3"),
            ],
            "F1",
            &[&[V::N(140.0)]],
        ),
        // M15: single-cell name vs 2-row name
        (
            "=XLOOKUP(1,OneCell,RetB2)",
            GRID,
            &[("OneCell", "=Sheet1!$A$1"), ("RetB2", "=Sheet1!$B$1:$B$2")],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
    ]);
}

/// A single-cell lookup array pairs with a one-row return array (the match is a row of it) or
/// a one-column one (the match is its column, which spills), in every match and search mode;
/// a return array of two or more rows and columns is `#VALUE!`, before `if_not_found`.
#[test]
fn xlookup_single_cell_lookup_array() {
    check(&[
        // S01: 1x1 vs 2x1
        (
            "=XLOOKUP(1,A1,B1:B2)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S02: 1x1 vs 3x1
        (
            "=XLOOKUP(1,A1,B1:B3)",
            GRID,
            &[],
            "F1:F3",
            &[&[V::N(10.0)], &[V::N(20.0)], &[V::N(30.0)]],
        ),
        // S03: 1x1 vs 2x2
        (
            "=XLOOKUP(1,A1,B1:C2)",
            GRID,
            &[],
            "F1:G2",
            &[
                &[V::E(ExcelErrorKind::Value), V::Blank],
                &[V::Blank, V::Blank],
            ],
        ),
        // S04: 1x1 vs 1x1
        ("=XLOOKUP(1,A1,B1)", GRID, &[], "F1", &[&[V::N(10.0)]]),
        // S05: 1x1 vs 2x1, absent
        (
            "=XLOOKUP(9,A1,B1:B2,\"nf\")",
            GRID,
            &[],
            "F1:F2",
            &[&[V::T("nf")], &[V::Blank]],
        ),
        // S06: 1x1 vs 2x2, absent
        (
            "=XLOOKUP(9,A1,B1:C2,\"nf\")",
            GRID,
            &[],
            "F1:G2",
            &[
                &[V::E(ExcelErrorKind::Value), V::Blank],
                &[V::Blank, V::Blank],
            ],
        ),
        // S07: 1x1 vs array 2x1
        (
            "=XLOOKUP(1,A1,{10;20})",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S08: array 1x1 vs 2x1
        (
            "=XLOOKUP(1,{1},B1:B2)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S09: 1x1 vs whole column
        (
            "=ROWS(XLOOKUP(1,A1,B:B))",
            GRID,
            &[],
            "F1",
            &[&[V::N(1048576.0)]],
        ),
        // S10: 1x1 vs 3x1 summed: 60
        (
            "=SUM(XLOOKUP(1,A1,B1:B3))",
            GRID,
            &[],
            "F1",
            &[&[V::N(60.0)]],
        ),
        // S11: 1x1 vs 1x2
        (
            "=XLOOKUP(1,A1,B1:C1)",
            GRID,
            &[],
            "F1:G1",
            &[&[V::N(10.0), V::N(100.0)]],
        ),
        // S12: 1x1, reference result through :
        (
            "=SUM(XLOOKUP(1,A1,B1:B3):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(150.0)]],
        ),
        // S13: 1x1, reference result rows
        (
            "=ROWS(XLOOKUP(1,A1,B1:B3):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(5.0)]],
        ),
        // S14: 1x1 range form vs 2x1
        (
            "=XLOOKUP(1,A1:A1,B1:B2)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S15: 1x1 at A2 vs B1:B3
        (
            "=XLOOKUP(2,A2,B1:B3)",
            GRID,
            &[],
            "F1:F3",
            &[&[V::N(10.0)], &[V::N(20.0)], &[V::N(30.0)]],
        ),
        // S16: 1x1 vs 3x2
        (
            "=XLOOKUP(1,A1,B1:C3)",
            GRID,
            &[],
            "F1:G3",
            &[
                &[V::E(ExcelErrorKind::Value), V::Blank],
                &[V::Blank, V::Blank],
                &[V::Blank, V::Blank],
            ],
        ),
        // S17: 1x1 vs 2x2 at row 2
        (
            "=XLOOKUP(2,A2,B2:C3)",
            GRID,
            &[],
            "F1:G2",
            &[
                &[V::E(ExcelErrorKind::Value), V::Blank],
                &[V::Blank, V::Blank],
            ],
        ),
        // S18: LET 1x1 vs 2x1
        (
            "=LET(k,A1,XLOOKUP(1,k,B1:B2))",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S19: 1x1 vs 2x1, last to first
        (
            "=XLOOKUP(1,A1,B1:B2,,0,-1)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S20: 1x1 vs 2x1, binary
        (
            "=XLOOKUP(1,A1,B1:B2,,0,2)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // S21: 1x1, ISREF
        (
            "=ISREF(XLOOKUP(1,A1,B1:B2))",
            GRID,
            &[],
            "F1",
            &[&[V::B(true)]],
        ),
        // S22: 1x1 vs two whole rows
        (
            "=XLOOKUP(1,A1,8:9)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // P01: single cell, exact or next smaller, summed
        (
            "=SUM(XLOOKUP(1,A1,B1:B2,,-1))",
            GRID,
            &[],
            "F1",
            &[&[V::N(30.0)]],
        ),
        // P02: single cell, exact or next larger, summed
        (
            "=SUM(XLOOKUP(1,A1,B1:B2,,1))",
            GRID,
            &[],
            "F1",
            &[&[V::N(30.0)]],
        ),
        // P03: single cell, next smaller, spill
        (
            "=XLOOKUP(1,A1,B1:B2,,-1)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // P04: single cell, smaller match
        (
            "=XLOOKUP(1.5,A1,B1:B2,,-1)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // P05: single cell, larger match
        (
            "=XLOOKUP(0.5,A1,B1:B2,,1)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // P06: single cell, no smaller
        (
            "=XLOOKUP(0.5,A1,B1:B2,,-1)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::E(ExcelErrorKind::Na)], &[V::Blank]],
        ),
        // P07: single cell vs row, next smaller
        (
            "=XLOOKUP(1,A1,B1:C1,,-1)",
            GRID,
            &[],
            "F1:G1",
            &[&[V::N(10.0), V::N(100.0)]],
        ),
        // P08: single cell, next smaller, last to first
        (
            "=XLOOKUP(1,A1,B1:B2,,-1,-1)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // P09: single cell, next smaller, binary
        (
            "=XLOOKUP(1,A1,B1:B2,,-1,2)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
        // P10: single cell vs 2x2, next smaller
        (
            "=XLOOKUP(1,A1,B1:C2,,-1)",
            GRID,
            &[],
            "F1:G2",
            &[
                &[V::E(ExcelErrorKind::Value), V::Blank],
                &[V::Blank, V::Blank],
            ],
        ),
        // P11: single cell, next smaller, reference result
        (
            "=SUM(XLOOKUP(1,A1,B1:B2,,-1):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(150.0)]],
        ),
        // P12: single cell, next larger, binary descending
        (
            "=XLOOKUP(1,A1,B1:B2,,1,-2)",
            GRID,
            &[],
            "F1:F2",
            &[&[V::N(10.0)], &[V::N(20.0)]],
        ),
    ]);
}

/// A reference a function returns (INDIRECT, OFFSET, INDEX, IF, CHOOSE, through LET or a name)
/// declares its own extent, as a written one does: `INDIRECT("A:A")` is every row.
#[test]
fn xlookup_arrays_returned_by_functions() {
    check(&[
        // F01: INDIRECT whole column vs whole column
        (
            "=XLOOKUP(2,INDIRECT(\"A:A\"),B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F02: INDIRECT whole column vs bounded
        (
            "=XLOOKUP(2,INDIRECT(\"A:A\"),B1:B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F03: INDIRECT 3 vs 4
        (
            "=XLOOKUP(2,INDIRECT(\"A1:A3\"),B1:B4)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F04: IF whole column vs whole column
        (
            "=XLOOKUP(2,IF(TRUE,A:A),B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F05: IF whole column vs bounded
        (
            "=XLOOKUP(2,IF(TRUE,A:A),B1:B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F06: CHOOSE whole column vs whole column
        (
            "=XLOOKUP(2,CHOOSE(1,A:A),B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F07: OFFSET full height vs whole column
        (
            "=XLOOKUP(2,OFFSET(A1,0,0,1048576,1),B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F08: INDEX column vs whole column
        (
            "=XLOOKUP(2,INDEX(A:B,0,1),B:B)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F09: INDEX column vs bounded
        (
            "=XLOOKUP(2,INDEX(A:B,0,1),B1:B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F10: whole column vs INDIRECT whole column
        (
            "=XLOOKUP(2,A:A,INDIRECT(\"B:B\"))",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F11: whole column vs INDIRECT bounded
        (
            "=XLOOKUP(2,A:A,INDIRECT(\"B1:B5\"))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F12: bounded vs INDIRECT whole column
        (
            "=XLOOKUP(2,A1:A5,INDIRECT(\"B:B\"))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F13: LET of INDIRECT whole column
        (
            "=LET(r,INDIRECT(\"A:A\"),XLOOKUP(2,r,B:B))",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F14: whole row vs INDIRECT whole row
        (
            "=XLOOKUP(2,8:8,INDIRECT(\"9:9\"))",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F15: INDIRECT whole row vs bounded
        (
            "=XLOOKUP(2,INDIRECT(\"8:8\"),A9:D9)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F16: IF return bounded equal
        (
            "=XLOOKUP(2,A1:A5,IF(TRUE,B1:B5))",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F17: whole column vs IF bounded
        (
            "=XLOOKUP(2,A:A,IF(TRUE,B1:B5))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // F18: INDIRECT whole column, last to first
        (
            "=XLOOKUP(2,INDIRECT(\"A:A\"),B:B,,0,-1)",
            GRID,
            &[],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F19: INDIRECT whole column, reference result
        (
            "=SUM(XLOOKUP(2,INDIRECT(\"A:A\"),B:B):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(140.0)]],
        ),
        // F20: INDIRECT name vs whole-column name
        (
            "=XLOOKUP(2,LookInd,RetCol)",
            GRID,
            &[
                ("LookInd", "=INDIRECT(\"Sheet1!A:A\")"),
                ("RetCol", "=Sheet1!$B:$B"),
            ],
            "F1",
            &[&[V::N(20.0)]],
        ),
        // F21: INDIRECT name vs bounded name
        (
            "=XLOOKUP(2,LookInd,RetB5)",
            GRID,
            &[
                ("LookInd", "=INDIRECT(\"Sheet1!A:A\")"),
                ("RetB5", "=Sheet1!$B$1:$B$5"),
            ],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
    ]);
}

/// The range operator over a function that gives a value: the value's error, or `#VALUE!`;
/// XLOOKUP's if_not_found written as a reference is a reference.
#[test]
fn range_operator_over_values() {
    check(&[
        // G01: range operator over XLOOKUP with array return, 3 vs 4
        (
            "=SUM(XLOOKUP(2,A1:A3,{10;20;30;40}):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G02: range operator over XLOOKUP's value 20
        (
            "=SUM(XLOOKUP(2,A1:A3,{10;20;30}):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G03: range operator over XLOOKUP's #N/A, array return
        (
            "=SUM(XLOOKUP(9,A1:A3,{10;20;30}):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Na)]],
        ),
        // G04: range operator over XLOOKUP with computed return, 3 vs 4
        (
            "=SUM(XLOOKUP(2,A1:A3,B1:B4*1):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G05: range operator over if_not_found text, array return
        (
            "=SUM(XLOOKUP(9,A1:A3,{10;20;30},\"nf\"):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G06: if_not_found a reference
        (
            "=SUM(XLOOKUP(9,A1:A3,B1:B3,B5):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::N(50.0)]],
        ),
        // G07: if_not_found text, reference return
        (
            "=SUM(XLOOKUP(9,A1:A3,B1:B3,\"nf\"):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G08: ROWS over a value range
        (
            "=ROWS(XLOOKUP(2,A1:A3,{10;20;30}):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G09: error on the right of the range operator
        (
            "=SUM(B1:XLOOKUP(2,A1:A3,{10;20;30;40}))",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G10: range operator over IF's #N/A
        (
            "=SUM(IF(TRUE,NA()):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Na)]],
        ),
        // G11: range operator over IF's number
        (
            "=SUM(IF(TRUE,5):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Value)]],
        ),
        // G12: range operator over CHOOSE's #DIV/0!
        (
            "=SUM(CHOOSE(1,1/0):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Div)]],
        ),
        // G14: range operator over INDEX out of range
        (
            "=SUM(INDEX(B1:B5,9):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::E(ExcelErrorKind::Ref)]],
        ),
        // G15: ISREF of a range over a value
        (
            "=ISREF(XLOOKUP(2,A1:A3,{10;20;30}):B5)",
            GRID,
            &[],
            "F1",
            &[&[V::B(false)]],
        ),
    ]);
}
