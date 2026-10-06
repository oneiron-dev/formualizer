//! Excel's securities, depreciation and compounding functions (the COUP* family,
//! ACCRINT, ACCRINTM, PRICE, YIELD, DURATION, MDURATION, the discount
//! securities, the odd-period bonds, TBILL*, DDB, VDB, AMORLINC, AMORDEGRC,
//! FVSCHEDULE) as Excel for Windows 16.0.20430 computes them: the rows of
//! ops/excel-finance-probe-20261006.md (each formula in F1 of a blank sheet),
//! numbers to 1e-12 relative, errors by kind. YIELD and ODDFYIELD stop their
//! iteration a little before the root, so their rows hold to the corpus
//! tolerance (1e-9 relative or 1e-10 absolute).
//! COUPDAYS on actual/actual for a maturity after the 28th or at a month end
//! is #N/IMPL! (the workbook falls back), not a row.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// The value of `formula` in F1 of a sheet whose other cells are `cells`.
fn eval_with(formula: &str, cells: &[(&str, LiteralValue)]) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (address, value) in cells {
        let row: u32 = address[1..].parse().unwrap();
        let col = u32::from(address.as_bytes()[0] - b'A' + 1);
        match value {
            LiteralValue::Text(t) if t.starts_with('=') => engine
                .set_cell_formula("Sheet1", row, col, parse(t).unwrap())
                .unwrap(),
            _ => engine
                .set_cell_value("Sheet1", row, col, value.clone())
                .unwrap(),
        }
    }
    engine
        .set_cell_formula("Sheet1", 1, 6, parse(formula).unwrap())
        .unwrap_or_else(|e| panic!("{formula}: {e:?}"));
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 6).unwrap()
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

/// Numbers within `relative` of the expected value or within `absolute` of
/// it, errors by kind.
fn same(actual: &LiteralValue, expected: &LiteralValue, relative: f64, absolute: f64) -> bool {
    match (actual, expected) {
        (LiteralValue::Number(a), LiteralValue::Number(b)) => {
            a == b || (a - b).abs() <= relative * b.abs() || (a - b).abs() <= absolute
        }
        (LiteralValue::Int(a), LiteralValue::Number(b)) => *a as f64 == *b,
        (LiteralValue::Error(a), LiteralValue::Error(b)) => a.kind == b.kind,
        _ => actual == expected,
    }
}

fn assert_within(cases: &[(&str, LiteralValue)], relative: f64, absolute: f64) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(formula, expected)| {
            let actual = eval_with(formula, &[]);
            (!same(&actual, expected, relative, absolute))
                .then(|| format!("{formula}: {actual:?}, not {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn assert_cases(cases: &[(&str, LiteralValue)]) {
    assert_within(cases, 1e-12, 0.0);
}

#[test]
fn couppcd() {
    assert_cases(&[
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,31),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,31),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,31),2,0)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,31),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,31),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,31),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,31),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,31),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,31),2,0)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,31),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,31),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,31),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,8,31),2,0)", n(45535.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,8,31),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,30),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,30),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,30),2,0)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,30),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,30),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,30),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,30),2,0)", n(45534.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,30),4,0)", n(45534.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,30),2,0)", n(45534.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,30),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,30),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,30),4,0)", n(45442.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,8,30),2,0)", n(45534.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,8,30),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,29),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,29),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,29),2,0)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,29),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,29),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,29),2,0)", n(45533.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,29),4,0)", n(45533.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,29),2,1)", n(45533.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,29),2,0)", n(45533.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,29),4,0)", n(45625.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,29),4,0)", n(45441.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,8,29),2,0)", n(45533.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,8,29),4,0)", n(45625.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,2,28),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,2,28),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,2,28),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,2,28),2,0)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,2,28),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,2,28),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,2,28),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,2,28),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,2,28),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,2,28),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,2,28),2,0)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,2,28),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,2,28),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,2,28),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,2,28),2,0)", n(45535.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,2,28),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2032,2,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2032,2,29),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2032,2,29),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2032,2,29),2,0)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2032,2,29),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2032,2,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2032,2,29),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2032,2,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2032,2,29),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2032,2,29),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2032,2,29),2,0)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2032,2,29),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2032,2,29),2,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2032,2,29),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2032,2,29),2,0)", n(45535.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2032,2,29),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,11,30),2,0)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,11,30),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,11,30),2,1)", n(45260.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,11,30),2,0)", n(44895.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,11,30),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,11,30),2,0)", n(45260.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,11,30),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,11,30),2,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,11,30),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,11,30),2,1)", n(45443.0)),
        (
            "=COUPPCD(DATE(2024,11,30),DATE(2030,11,30),2,0)",
            n(45626.0),
        ),
        (
            "=COUPPCD(DATE(2024,11,30),DATE(2030,11,30),4,0)",
            n(45626.0),
        ),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,11,30),2,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,11,30),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,11,30),2,0)", n(45626.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,11,30),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,5,31),2,0)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,5,31),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,5,31),2,1)", n(45260.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,5,31),2,0)", n(44895.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,5,31),4,0)", n(44985.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,5,31),2,0)", n(45260.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,5,31),4,0)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,5,31),2,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,5,31),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,5,31),2,1)", n(45443.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,5,31),2,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,5,31),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,5,31),2,0)", n(45443.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,5,31),4,0)", n(45443.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,5,31),2,0)", n(45626.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,5,31),4,0)", n(45626.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,6,15),2,0)", n(45275.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,6,15),4,0)", n(45275.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,6,15),2,1)", n(45275.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,6,15),2,0)", n(44910.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,6,15),4,0)", n(44910.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,6,15),2,0)", n(45275.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,6,15),4,0)", n(45366.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,6,15),2,0)", n(45458.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,6,15),4,0)", n(45458.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,6,15),2,1)", n(45458.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,6,15),2,0)", n(45458.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,6,15),4,0)", n(45550.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,6,15),2,0)", n(45275.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,6,15),4,0)", n(45366.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,6,15),2,0)", n(45641.0)),
        ("=COUPPCD(DATE(2025,1,15),DATE(2030,6,15),4,0)", n(45641.0)),
        ("=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,1)", n(40497.0)),
        (
            "=COUPPCD(DATE(2011,11,15),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,11,16),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),12,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2.9,1)",
            n(40497.0),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),4.5,1)",
            n(40497.0),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),1.5,1)",
            n(40497.0),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),-2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),\"2\",1)",
            n(40497.0),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),TRUE,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,-1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,4.9)",
            n(40497.0),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,0.9)",
            n(40497.0),
        ),
        ("=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,)", n(40497.0)),
        ("=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2)", n(40497.0)),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,\"1\")",
            n(40497.0),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,TRUE)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,A1)",
            n(40497.0),
        ),
        ("=COUPPCD(40568.99,40862.2,2,1)", n(40497.0)),
        ("=COUPPCD(40862.1,40862.9,2,1)", error(ExcelErrorKind::Num)),
        ("=COUPPCD(\"2011-01-25\",\"2011-11-15\",2,1)", n(40497.0)),
        (
            "=COUPPCD(\"abc\",DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPPCD(-1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPPCD(0,DATE(2011,11,15),2,1)", n(0.0)),
        (
            "=COUPPCD(DATE(2011,1,25),2958466,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPPCD(DATE(2011,1,25),2958465,2,1)", n(40543.0)),
        (
            "=COUPPCD(DATE(2011,1,25),1/0,2,1)",
            error(ExcelErrorKind::Div),
        ),
        (
            "=SUM(COUPPCD({40568,40600},DATE(2011,11,15),2,1))",
            n(80994.0),
        ),
        ("=COUPPCD(A1,DATE(2011,11,15),2,1)", n(0.0)),
        ("=COUPPCD(59,DATE(1901,2,28),2,0)", n(59.0)),
        ("=COUPPCD(61,DATE(1901,8,31),2,0)", n(59.0)),
        ("=COUPPCD(DATE(2023,2,24),DATE(2030,8,31),2,1)", n(44804.0)),
        ("=COUPPCD(DATE(2023,2,24),DATE(2030,8,31),4,1)", n(44895.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,31),2,1)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,31),4,1)", n(44985.0)),
        ("=COUPPCD(DATE(2023,3,1),DATE(2030,8,31),2,1)", n(44985.0)),
        ("=COUPPCD(DATE(2023,3,1),DATE(2030,8,31),4,1)", n(44985.0)),
        ("=COUPPCD(DATE(2024,1,15),DATE(2030,8,31),2,1)", n(45169.0)),
        ("=COUPPCD(DATE(2024,1,15),DATE(2030,8,31),4,1)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,27),DATE(2030,8,31),2,1)", n(45169.0)),
        ("=COUPPCD(DATE(2024,2,27),DATE(2030,8,31),4,1)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,28),DATE(2030,8,31),2,1)", n(45169.0)),
        ("=COUPPCD(DATE(2024,2,28),DATE(2030,8,31),4,1)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,1),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,1),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,30),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,30),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,15),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,15),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,27),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,27),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,28),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,28),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPPCD(DATE(2024,8,31),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,8,31),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,9,1),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,9,1),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,9,29),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,9,29),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,9,30),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,9,30),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,15),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,15),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,31),4,1)", n(45626.0)),
        ("=COUPPCD(DATE(2023,2,24),DATE(2030,8,30),2,1)", n(44803.0)),
        ("=COUPPCD(DATE(2023,2,24),DATE(2030,8,30),4,1)", n(44895.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,30),2,1)", n(44985.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,8,30),4,1)", n(44985.0)),
        ("=COUPPCD(DATE(2023,3,1),DATE(2030,8,30),2,1)", n(44985.0)),
        ("=COUPPCD(DATE(2023,3,1),DATE(2030,8,30),4,1)", n(44985.0)),
        ("=COUPPCD(DATE(2024,1,15),DATE(2030,8,30),2,1)", n(45168.0)),
        ("=COUPPCD(DATE(2024,1,15),DATE(2030,8,30),4,1)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,27),DATE(2030,8,30),2,1)", n(45168.0)),
        ("=COUPPCD(DATE(2024,2,27),DATE(2030,8,30),4,1)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,28),DATE(2030,8,30),2,1)", n(45168.0)),
        ("=COUPPCD(DATE(2024,2,28),DATE(2030,8,30),4,1)", n(45260.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,1),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,1),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,30),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,30),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,15),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,15),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPPCD(DATE(2024,8,27),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,27),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPPCD(DATE(2024,8,28),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPPCD(DATE(2024,8,28),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,8,31),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,8,31),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,9,1),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,9,1),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,9,29),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,9,29),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,9,30),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,9,30),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,11,15),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,11,15),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPPCD(DATE(2023,2,24),DATE(2030,3,31),2,1)", n(44834.0)),
        ("=COUPPCD(DATE(2023,2,24),DATE(2030,3,31),4,1)", n(44926.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,3,31),2,1)", n(44834.0)),
        ("=COUPPCD(DATE(2023,2,28),DATE(2030,3,31),4,1)", n(44926.0)),
        ("=COUPPCD(DATE(2023,3,1),DATE(2030,3,31),2,1)", n(44834.0)),
        ("=COUPPCD(DATE(2023,3,1),DATE(2030,3,31),4,1)", n(44926.0)),
        ("=COUPPCD(DATE(2024,1,15),DATE(2030,3,31),2,1)", n(45199.0)),
        ("=COUPPCD(DATE(2024,1,15),DATE(2030,3,31),4,1)", n(45291.0)),
        ("=COUPPCD(DATE(2024,2,27),DATE(2030,3,31),2,1)", n(45199.0)),
        ("=COUPPCD(DATE(2024,2,27),DATE(2030,3,31),4,1)", n(45291.0)),
        ("=COUPPCD(DATE(2024,2,28),DATE(2030,3,31),2,1)", n(45199.0)),
        ("=COUPPCD(DATE(2024,2,28),DATE(2030,3,31),4,1)", n(45291.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,3,31),2,1)", n(45199.0)),
        ("=COUPPCD(DATE(2024,2,29),DATE(2030,3,31),4,1)", n(45291.0)),
        ("=COUPPCD(DATE(2024,3,1),DATE(2030,3,31),2,1)", n(45199.0)),
        ("=COUPPCD(DATE(2024,3,1),DATE(2030,3,31),4,1)", n(45291.0)),
        ("=COUPPCD(DATE(2024,3,30),DATE(2030,3,31),2,1)", n(45199.0)),
        ("=COUPPCD(DATE(2024,3,30),DATE(2030,3,31),4,1)", n(45291.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,3,31),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,5,15),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,5,15),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,5,31),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,8,27),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,8,27),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPPCD(DATE(2024,8,28),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,8,28),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,8,30),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPPCD(DATE(2024,8,31),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,8,31),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPPCD(DATE(2024,9,1),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,9,1),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPPCD(DATE(2024,9,29),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPPCD(DATE(2024,9,29),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPPCD(DATE(2024,9,30),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPPCD(DATE(2024,9,30),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPPCD(DATE(2024,11,15),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPPCD(DATE(2024,11,15),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPPCD(DATE(2024,11,30),DATE(2030,3,31),4,1)", n(45565.0)),
    ]);
}

#[test]
fn coupncd() {
    assert_cases(&[
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,31),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,31),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,31),1,3)", n(45535.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,31),2,0)", n(45169.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,31),4,0)", n(45077.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,31),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,31),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,31),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,31),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,31),1,3)", n(45535.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,31),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,31),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,31),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,31),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,8,31),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,8,31),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,30),2,0)", n(45534.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,30),4,0)", n(45442.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,30),1,3)", n(45534.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,30),2,0)", n(45168.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,30),4,0)", n(45076.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,30),2,0)", n(45534.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,30),4,0)", n(45442.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,30),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,30),4,0)", n(45626.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,30),1,3)", n(45899.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,30),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,30),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,30),2,0)", n(45534.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,30),4,0)", n(45534.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,8,30),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,8,30),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,29),2,0)", n(45533.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,29),4,0)", n(45441.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,29),1,3)", n(45533.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,29),2,0)", n(45167.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,29),4,0)", n(45075.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,29),2,0)", n(45533.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,29),4,0)", n(45441.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,29),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,29),4,0)", n(45625.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,29),1,3)", n(45898.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,29),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,29),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,29),2,0)", n(45533.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,29),4,0)", n(45533.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,8,29),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,8,29),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,2,28),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,2,28),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,2,28),1,3)", n(45716.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,2,28),2,0)", n(45169.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,2,28),4,0)", n(45077.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,2,28),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,2,28),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,2,28),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,2,28),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,2,28),1,3)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,2,28),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,2,28),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,2,28),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,2,28),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,2,28),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,2,28),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2032,2,29),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2032,2,29),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2032,2,29),1,3)", n(45716.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2032,2,29),2,0)", n(45169.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2032,2,29),4,0)", n(45077.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2032,2,29),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2032,2,29),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2032,2,29),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2032,2,29),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2032,2,29),1,3)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2032,2,29),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2032,2,29),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2032,2,29),2,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2032,2,29),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2032,2,29),2,0)", n(45716.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2032,2,29),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,11,30),2,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,11,30),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,11,30),1,3)", n(45626.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,11,30),2,0)", n(45077.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,11,30),4,0)", n(45077.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,11,30),2,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,11,30),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,11,30),2,0)", n(45626.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,11,30),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,11,30),1,3)", n(45626.0)),
        (
            "=COUPNCD(DATE(2024,11,30),DATE(2030,11,30),2,0)",
            n(45808.0),
        ),
        (
            "=COUPNCD(DATE(2024,11,30),DATE(2030,11,30),4,0)",
            n(45716.0),
        ),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,11,30),2,0)", n(45626.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,11,30),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,11,30),2,0)", n(45808.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,11,30),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,5,31),2,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,5,31),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,5,31),1,3)", n(45443.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,5,31),2,0)", n(45077.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,5,31),4,0)", n(45077.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,5,31),2,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,5,31),4,0)", n(45443.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,5,31),2,0)", n(45626.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,5,31),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,5,31),1,3)", n(45808.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,5,31),2,0)", n(45808.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,5,31),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,5,31),2,0)", n(45626.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,5,31),4,0)", n(45535.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,5,31),2,0)", n(45808.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,5,31),4,0)", n(45716.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,6,15),2,0)", n(45458.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,6,15),4,0)", n(45366.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,6,15),1,3)", n(45458.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,6,15),2,0)", n(45092.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,6,15),4,0)", n(45000.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,6,15),2,0)", n(45458.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,6,15),4,0)", n(45458.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,6,15),2,0)", n(45641.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,6,15),4,0)", n(45550.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,6,15),1,3)", n(45823.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,6,15),2,0)", n(45641.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,6,15),4,0)", n(45641.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,6,15),2,0)", n(45458.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,6,15),4,0)", n(45458.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,6,15),2,0)", n(45823.0)),
        ("=COUPNCD(DATE(2025,1,15),DATE(2030,6,15),4,0)", n(45731.0)),
        ("=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,1)", n(40678.0)),
        (
            "=COUPNCD(DATE(2011,11,15),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,11,16),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),12,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2.9,1)",
            n(40678.0),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),4.5,1)",
            n(40589.0),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),1.5,1)",
            n(40862.0),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),-2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),\"2\",1)",
            n(40678.0),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),TRUE,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,-1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,4.9)",
            n(40678.0),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,0.9)",
            n(40678.0),
        ),
        ("=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,)", n(40678.0)),
        ("=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2)", n(40678.0)),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,\"1\")",
            n(40678.0),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,TRUE)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,A1)",
            n(40678.0),
        ),
        ("=COUPNCD(40568.99,40862.2,2,1)", n(40678.0)),
        ("=COUPNCD(40862.1,40862.9,2,1)", error(ExcelErrorKind::Num)),
        ("=COUPNCD(\"2011-01-25\",\"2011-11-15\",2,1)", n(40678.0)),
        (
            "=COUPNCD(\"abc\",DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPNCD(-1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNCD(0,DATE(2011,11,15),2,1)", n(136.0)),
        (
            "=COUPNCD(DATE(2011,1,25),2958466,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNCD(DATE(2011,1,25),2958465,2,1)", n(40724.0)),
        (
            "=COUPNCD(DATE(2011,1,25),1/0,2,1)",
            error(ExcelErrorKind::Div),
        ),
        (
            "=SUM(COUPNCD({40568,40600},DATE(2011,11,15),2,1))",
            n(81356.0),
        ),
        ("=COUPNCD(A1,DATE(2011,11,15),2,1)", n(136.0)),
        ("=COUPNCD(59,DATE(1901,2,28),2,0)", n(244.0)),
        ("=COUPNCD(61,DATE(1901,8,31),2,0)", n(244.0)),
        ("=COUPNCD(DATE(2023,2,24),DATE(2030,8,31),2,1)", n(44985.0)),
        ("=COUPNCD(DATE(2023,2,24),DATE(2030,8,31),4,1)", n(44985.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,31),2,1)", n(45169.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,31),4,1)", n(45077.0)),
        ("=COUPNCD(DATE(2023,3,1),DATE(2030,8,31),2,1)", n(45169.0)),
        ("=COUPNCD(DATE(2023,3,1),DATE(2030,8,31),4,1)", n(45077.0)),
        ("=COUPNCD(DATE(2024,1,15),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,1,15),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,27),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,27),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,28),DATE(2030,8,31),2,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,28),DATE(2030,8,31),4,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPNCD(DATE(2024,3,1),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,3,1),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPNCD(DATE(2024,3,30),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,3,30),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPNCD(DATE(2024,5,15),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,5,15),DATE(2030,8,31),4,1)", n(45443.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,27),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,27),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,28),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,28),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,31),4,1)", n(45535.0)),
        ("=COUPNCD(DATE(2024,8,31),DATE(2030,8,31),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,8,31),DATE(2030,8,31),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,9,1),DATE(2030,8,31),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,9,1),DATE(2030,8,31),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,9,29),DATE(2030,8,31),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,9,29),DATE(2030,8,31),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,9,30),DATE(2030,8,31),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,9,30),DATE(2030,8,31),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,11,15),DATE(2030,8,31),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,15),DATE(2030,8,31),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,31),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,31),4,1)", n(45716.0)),
        ("=COUPNCD(DATE(2023,2,24),DATE(2030,8,30),2,1)", n(44985.0)),
        ("=COUPNCD(DATE(2023,2,24),DATE(2030,8,30),4,1)", n(44985.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,30),2,1)", n(45168.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,8,30),4,1)", n(45076.0)),
        ("=COUPNCD(DATE(2023,3,1),DATE(2030,8,30),2,1)", n(45168.0)),
        ("=COUPNCD(DATE(2023,3,1),DATE(2030,8,30),4,1)", n(45076.0)),
        ("=COUPNCD(DATE(2024,1,15),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,1,15),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,27),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,27),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,28),DATE(2030,8,30),2,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,28),DATE(2030,8,30),4,1)", n(45351.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPNCD(DATE(2024,3,1),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,3,1),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPNCD(DATE(2024,3,30),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,3,30),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPNCD(DATE(2024,5,15),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,5,15),DATE(2030,8,30),4,1)", n(45442.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,8,27),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,8,27),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,8,28),DATE(2030,8,30),2,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,8,28),DATE(2030,8,30),4,1)", n(45534.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,8,31),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,8,31),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,9,1),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,9,1),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,9,29),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,9,29),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,9,30),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,9,30),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,11,15),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,15),DATE(2030,8,30),4,1)", n(45626.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,30),2,1)", n(45716.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,8,30),4,1)", n(45716.0)),
        ("=COUPNCD(DATE(2023,2,24),DATE(2030,3,31),2,1)", n(45016.0)),
        ("=COUPNCD(DATE(2023,2,24),DATE(2030,3,31),4,1)", n(45016.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,3,31),2,1)", n(45016.0)),
        ("=COUPNCD(DATE(2023,2,28),DATE(2030,3,31),4,1)", n(45016.0)),
        ("=COUPNCD(DATE(2023,3,1),DATE(2030,3,31),2,1)", n(45016.0)),
        ("=COUPNCD(DATE(2023,3,1),DATE(2030,3,31),4,1)", n(45016.0)),
        ("=COUPNCD(DATE(2024,1,15),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,1,15),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,2,27),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,2,27),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,2,28),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,2,28),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,2,29),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,3,1),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,3,1),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,3,30),DATE(2030,3,31),2,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,3,30),DATE(2030,3,31),4,1)", n(45382.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,3,31),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPNCD(DATE(2024,5,15),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,5,15),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,5,31),DATE(2030,3,31),4,1)", n(45473.0)),
        ("=COUPNCD(DATE(2024,8,27),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,27),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,28),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,28),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,30),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,31),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,8,31),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,9,1),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,9,1),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,9,29),DATE(2030,3,31),2,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,9,29),DATE(2030,3,31),4,1)", n(45565.0)),
        ("=COUPNCD(DATE(2024,9,30),DATE(2030,3,31),2,1)", n(45747.0)),
        ("=COUPNCD(DATE(2024,9,30),DATE(2030,3,31),4,1)", n(45657.0)),
        ("=COUPNCD(DATE(2024,11,15),DATE(2030,3,31),2,1)", n(45747.0)),
        ("=COUPNCD(DATE(2024,11,15),DATE(2030,3,31),4,1)", n(45657.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,3,31),2,1)", n(45747.0)),
        ("=COUPNCD(DATE(2024,11,30),DATE(2030,3,31),4,1)", n(45657.0)),
    ]);
}

#[test]
fn coupdaybs() {
    assert_cases(&[
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),2,2)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),2,3)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,31),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,31),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,31),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,31),2,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,31),2,4)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,31),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),2,4)", n(181.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),2,2)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),2,3)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,31),2,0)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,31),2,4)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,31),2,0)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,31),2,4)", n(91.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,8,31),2,0)", n(135.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,8,31),2,4)", n(135.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,30),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,30),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,30),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,30),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,30),2,2)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,30),2,3)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,30),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,30),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,30),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,30),2,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,30),2,4)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,30),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,30),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,30),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,30),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,30),2,2)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,30),2,3)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,30),2,0)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,30),2,4)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,30),2,0)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,30),2,4)", n(91.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,8,30),2,0)", n(135.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,8,30),2,4)", n(135.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,29),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,29),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,29),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,29),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,29),2,2)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,29),2,3)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,29),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,29),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,29),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,29),2,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,29),2,4)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,29),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,29),2,0)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,29),2,4)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,29),2,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,29),2,2)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,29),2,3)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,29),2,0)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,29),2,4)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,29),2,0)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,29),2,4)", n(91.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,8,29),2,0)", n(136.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,8,29),2,4)", n(136.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,2,28),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,2,28),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,2,28),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,2,28),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,2,28),2,2)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,2,28),2,3)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,2,28),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,2,28),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,2,28),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,2,28),2,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,2,28),2,4)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,2,28),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,2,28),2,4)", n(181.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,2,28),2,1)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,2,28),2,2)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,2,28),2,3)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,2,28),2,0)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,2,28),2,4)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,2,28),2,0)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,2,28),2,4)", n(91.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,2,28),2,0)", n(135.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,2,28),2,4)", n(135.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2032,2,29),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2032,2,29),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2032,2,29),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2032,2,29),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2032,2,29),2,2)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2032,2,29),2,3)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2032,2,29),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2032,2,29),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2032,2,29),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2032,2,29),2,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2032,2,29),2,4)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2032,2,29),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2032,2,29),2,4)", n(181.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2032,2,29),2,1)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2032,2,29),2,2)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2032,2,29),2,3)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2032,2,29),2,0)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2032,2,29),2,4)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2032,2,29),2,0)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2032,2,29),2,4)", n(91.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2032,2,29),2,0)", n(135.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2032,2,29),2,4)", n(135.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,11,30),2,0)", n(89.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,11,30),2,4)", n(89.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,11,30),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,11,30),2,1)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,11,30),2,2)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,11,30),2,3)", n(91.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,11,30),2,0)", n(88.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,11,30),2,4)", n(88.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,11,30),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,11,30),2,0)", n(120.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,11,30),2,4)", n(120.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,11,30),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,11,30),2,0)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,11,30),2,4)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,11,30),2,1)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,11,30),2,2)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,11,30),2,3)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,11,30),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,11,30),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,11,30),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,11,30),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,11,30),2,0)", n(45.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,11,30),2,4)", n(45.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,5,31),2,0)", n(89.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,5,31),2,4)", n(89.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,5,31),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,5,31),2,1)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,5,31),2,2)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,5,31),2,3)", n(91.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,5,31),2,0)", n(88.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,5,31),2,4)", n(88.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,5,31),4,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,5,31),2,0)", n(120.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,5,31),2,4)", n(120.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,5,31),4,0)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,5,31),2,0)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,5,31),2,4)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,5,31),2,1)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,5,31),2,2)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,5,31),2,3)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,5,31),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,5,31),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,5,31),2,0)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,5,31),2,4)", n(0.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,5,31),2,0)", n(45.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,5,31),2,4)", n(45.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,6,15),2,0)", n(74.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,6,15),2,4)", n(74.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,6,15),4,0)", n(74.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,6,15),2,1)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,6,15),2,2)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,6,15),2,3)", n(76.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,6,15),2,0)", n(73.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,6,15),2,4)", n(73.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,6,15),4,0)", n(73.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,6,15),2,0)", n(106.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,6,15),2,4)", n(105.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,6,15),4,0)", n(16.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,6,15),2,0)", n(75.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,6,15),2,4)", n(75.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,6,15),2,1)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,6,15),2,2)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,6,15),2,3)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,6,15),2,0)", n(165.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,6,15),2,4)", n(165.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,6,15),2,0)", n(166.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,6,15),2,4)", n(165.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,6,15),2,0)", n(30.0)),
        ("=COUPDAYBS(DATE(2025,1,15),DATE(2030,6,15),2,4)", n(30.0)),
        ("=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,1)", n(71.0)),
        (
            "=COUPDAYBS(DATE(2011,11,15),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,11,16),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),12,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2.9,1)",
            n(71.0),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),4.5,1)",
            n(71.0),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),1.5,1)",
            n(71.0),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),-2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),\"2\",1)",
            n(71.0),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),TRUE,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,-1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,4.9)",
            n(70.0),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,0.9)",
            n(70.0),
        ),
        ("=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,)", n(70.0)),
        ("=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2)", n(70.0)),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,\"1\")",
            n(71.0),
        ),
        (
            "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,TRUE)",
            error(ExcelErrorKind::Value),
        ),
        ("=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,A1)", n(70.0)),
        ("=COUPDAYBS(40568.99,40862.2,2,1)", n(71.0)),
        (
            "=COUPDAYBS(40862.1,40862.9,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYBS(\"2011-01-25\",\"2011-11-15\",2,1)", n(71.0)),
        (
            "=COUPDAYBS(\"abc\",DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYBS(-1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYBS(0,DATE(2011,11,15),2,1)", n(0.0)),
        (
            "=COUPDAYBS(DATE(2011,1,25),2958466,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYBS(DATE(2011,1,25),2958465,2,1)", n(25.0)),
        (
            "=COUPDAYBS(DATE(2011,1,25),1/0,2,1)",
            error(ExcelErrorKind::Div),
        ),
        (
            "=SUM(COUPDAYBS({40568,40600},DATE(2011,11,15),2,1))",
            n(174.0),
        ),
        ("=COUPDAYBS(A1,DATE(2011,11,15),2,1)", n(0.0)),
        ("=COUPDAYBS(59,DATE(1901,2,28),2,0)", n(0.0)),
        ("=COUPDAYBS(61,DATE(1901,8,31),2,0)", n(1.0)),
        ("=COUPDAYBS(DATE(2023,2,24),DATE(2030,8,31),2,1)", n(177.0)),
        ("=COUPDAYBS(DATE(2023,2,24),DATE(2030,8,31),4,1)", n(86.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,31),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,2,28),DATE(2030,8,31),4,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2023,3,1),DATE(2030,8,31),2,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2023,3,1),DATE(2030,8,31),4,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,1,15),DATE(2030,8,31),2,1)", n(137.0)),
        ("=COUPDAYBS(DATE(2024,1,15),DATE(2030,8,31),4,1)", n(46.0)),
        ("=COUPDAYBS(DATE(2024,2,27),DATE(2030,8,31),2,1)", n(180.0)),
        ("=COUPDAYBS(DATE(2024,2,27),DATE(2030,8,31),4,1)", n(89.0)),
        ("=COUPDAYBS(DATE(2024,2,28),DATE(2030,8,31),2,1)", n(181.0)),
        ("=COUPDAYBS(DATE(2024,2,28),DATE(2030,8,31),4,1)", n(90.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,2,29),DATE(2030,8,31),4,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,3,1),DATE(2030,8,31),2,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,3,1),DATE(2030,8,31),4,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,3,30),DATE(2030,8,31),2,1)", n(30.0)),
        ("=COUPDAYBS(DATE(2024,3,30),DATE(2030,8,31),4,1)", n(30.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,31),2,1)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,3,31),DATE(2030,8,31),4,1)", n(31.0)),
        ("=COUPDAYBS(DATE(2024,5,15),DATE(2030,8,31),2,1)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,5,15),DATE(2030,8,31),4,1)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,31),2,1)", n(92.0)),
        ("=COUPDAYBS(DATE(2024,5,31),DATE(2030,8,31),4,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,8,27),DATE(2030,8,31),2,1)", n(180.0)),
        ("=COUPDAYBS(DATE(2024,8,27),DATE(2030,8,31),4,1)", n(88.0)),
        ("=COUPDAYBS(DATE(2024,8,28),DATE(2030,8,31),2,1)", n(181.0)),
        ("=COUPDAYBS(DATE(2024,8,28),DATE(2030,8,31),4,1)", n(89.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(183.0)),
        ("=COUPDAYBS(DATE(2024,8,30),DATE(2030,8,31),4,1)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,8,31),DATE(2030,8,31),2,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,8,31),DATE(2030,8,31),4,1)", n(0.0)),
        ("=COUPDAYBS(DATE(2024,9,1),DATE(2030,8,31),2,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,9,1),DATE(2030,8,31),4,1)", n(1.0)),
        ("=COUPDAYBS(DATE(2024,9,29),DATE(2030,8,31),2,1)", n(29.0)),
        ("=COUPDAYBS(DATE(2024,9,29),DATE(2030,8,31),4,1)", n(29.0)),
        ("=COUPDAYBS(DATE(2024,9,30),DATE(2030,8,31),2,1)", n(30.0)),
        ("=COUPDAYBS(DATE(2024,9,30),DATE(2030,8,31),4,1)", n(30.0)),
        ("=COUPDAYBS(DATE(2024,11,15),DATE(2030,8,31),2,1)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,11,15),DATE(2030,8,31),4,1)", n(76.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,31),2,1)", n(91.0)),
        ("=COUPDAYBS(DATE(2024,11,30),DATE(2030,8,31),4,1)", n(0.0)),
    ]);
}

#[test]
fn coupdaysnc() {
    assert_cases(&[
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),2,4)", n(181.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),2,2)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),2,3)", n(184.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,31),2,4)", n(182.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,31),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,31),2,0)", n(149.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,31),2,4)", n(150.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,31),4,0)", n(59.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),2,0)", n(0.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),2,4)", n(0.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),2,2)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),2,3)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,31),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,31),2,4)", n(88.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,31),2,0)", n(89.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,31),2,4)", n(90.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,8,31),2,0)", n(45.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,8,31),2,4)", n(43.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,30),2,4)", n(181.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,30),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,30),2,1)", n(183.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,30),2,2)", n(183.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,30),2,3)", n(183.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,30),2,4)", n(182.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,30),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,30),2,0)", n(149.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,30),2,4)", n(150.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,30),4,0)", n(59.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,30),2,4)", n(178.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,30),2,1)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,30),2,2)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,30),2,3)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,30),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,30),2,4)", n(88.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,30),2,0)", n(89.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,30),2,4)", n(90.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,8,30),2,0)", n(45.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,8,30),2,4)", n(43.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,29),2,0)", n(179.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,29),2,4)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,29),4,0)", n(89.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,29),2,1)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,29),2,2)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,29),2,3)", n(182.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,29),2,0)", n(179.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,29),2,4)", n(181.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,29),4,0)", n(89.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,29),2,0)", n(148.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,29),2,4)", n(149.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,29),4,0)", n(58.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,29),2,4)", n(178.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,29),2,1)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,29),2,2)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,29),2,3)", n(182.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,29),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,29),2,4)", n(88.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,29),2,0)", n(88.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,29),2,4)", n(89.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,8,29),2,0)", n(45.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,8,29),2,4)", n(43.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,2,28),2,4)", n(181.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,2,28),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,2,28),2,1)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,2,28),2,2)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,2,28),2,3)", n(184.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,2,28),2,4)", n(182.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,2,28),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,2,28),2,0)", n(149.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,2,28),2,4)", n(150.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,2,28),4,0)", n(59.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,2,28),2,0)", n(0.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,2,28),2,4)", n(0.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,2,28),2,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,2,28),2,2)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,2,28),2,3)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,2,28),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,2,28),2,4)", n(88.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,2,28),2,0)", n(89.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,2,28),2,4)", n(90.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,2,28),2,0)", n(45.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,2,28),2,4)", n(43.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2032,2,29),2,4)", n(181.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2032,2,29),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2032,2,29),2,1)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2032,2,29),2,2)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2032,2,29),2,3)", n(184.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2032,2,29),2,4)", n(182.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2032,2,29),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2032,2,29),2,0)", n(149.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2032,2,29),2,4)", n(150.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2032,2,29),4,0)", n(59.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2032,2,29),2,0)", n(0.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2032,2,29),2,4)", n(0.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2032,2,29),2,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2032,2,29),2,2)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2032,2,29),2,3)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2032,2,29),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2032,2,29),2,4)", n(88.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2032,2,29),2,0)", n(89.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2032,2,29),2,4)", n(90.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2032,2,29),2,0)", n(45.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2032,2,29),2,4)", n(43.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,11,30),2,0)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,11,30),2,4)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,11,30),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,11,30),2,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,11,30),2,2)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,11,30),2,3)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,11,30),2,0)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,11,30),2,4)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,11,30),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,11,30),2,0)", n(60.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,11,30),2,4)", n(60.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,11,30),4,0)", n(59.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,11,30),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,11,30),2,4)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,11,30),2,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,11,30),2,2)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,11,30),2,3)", n(92.0)),
        (
            "=COUPDAYSNC(DATE(2024,11,30),DATE(2030,11,30),2,0)",
            n(180.0),
        ),
        (
            "=COUPDAYSNC(DATE(2024,11,30),DATE(2030,11,30),2,4)",
            n(180.0),
        ),
        (
            "=COUPDAYSNC(DATE(2024,5,31),DATE(2030,11,30),2,0)",
            n(180.0),
        ),
        (
            "=COUPDAYSNC(DATE(2024,5,31),DATE(2030,11,30),2,4)",
            n(180.0),
        ),
        (
            "=COUPDAYSNC(DATE(2025,1,15),DATE(2030,11,30),2,0)",
            n(135.0),
        ),
        (
            "=COUPDAYSNC(DATE(2025,1,15),DATE(2030,11,30),2,4)",
            n(135.0),
        ),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,5,31),2,0)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,5,31),2,4)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,5,31),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,5,31),2,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,5,31),2,2)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,5,31),2,3)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,5,31),2,0)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,5,31),2,4)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,5,31),4,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,5,31),2,0)", n(60.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,5,31),2,4)", n(60.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,5,31),4,0)", n(59.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,5,31),2,0)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,5,31),2,4)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,5,31),2,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,5,31),2,2)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,5,31),2,3)", n(92.0)),
        (
            "=COUPDAYSNC(DATE(2024,11,30),DATE(2030,5,31),2,0)",
            n(180.0),
        ),
        (
            "=COUPDAYSNC(DATE(2024,11,30),DATE(2030,5,31),2,4)",
            n(180.0),
        ),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,5,31),2,4)", n(180.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,5,31),2,0)", n(135.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,5,31),2,4)", n(135.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,6,15),2,0)", n(106.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,6,15),2,4)", n(106.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,6,15),4,0)", n(16.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,6,15),2,1)", n(107.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,6,15),2,2)", n(107.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,6,15),2,3)", n(107.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,6,15),2,0)", n(107.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,6,15),2,4)", n(107.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,6,15),4,0)", n(17.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,6,15),2,0)", n(74.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,6,15),2,4)", n(75.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,6,15),4,0)", n(74.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,6,15),2,0)", n(105.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,6,15),2,4)", n(105.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,6,15),2,1)", n(107.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,6,15),2,2)", n(107.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,6,15),2,3)", n(107.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,6,15),2,0)", n(15.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,6,15),2,4)", n(15.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,6,15),2,0)", n(14.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,6,15),2,4)", n(15.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,6,15),2,0)", n(150.0)),
        ("=COUPDAYSNC(DATE(2025,1,15),DATE(2030,6,15),2,4)", n(150.0)),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,1)",
            n(110.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,11,15),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,11,16),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),12,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2.9,1)",
            n(110.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),4.5,1)",
            n(21.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),1.5,1)",
            n(294.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),-2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),\"2\",1)",
            n(110.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),TRUE,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,-1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,4.9)",
            n(110.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,0.9)",
            n(110.0),
        ),
        ("=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,)", n(110.0)),
        ("=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2)", n(110.0)),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,\"1\")",
            n(110.0),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,TRUE)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,A1)",
            n(110.0),
        ),
        ("=COUPDAYSNC(40568.99,40862.2,2,1)", n(110.0)),
        (
            "=COUPDAYSNC(40862.1,40862.9,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYSNC(\"2011-01-25\",\"2011-11-15\",2,1)", n(110.0)),
        (
            "=COUPDAYSNC(\"abc\",DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYSNC(-1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYSNC(0,DATE(2011,11,15),2,1)", n(136.0)),
        (
            "=COUPDAYSNC(DATE(2011,1,25),2958466,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYSNC(DATE(2011,1,25),2958465,2,1)", n(156.0)),
        (
            "=COUPDAYSNC(DATE(2011,1,25),1/0,2,1)",
            error(ExcelErrorKind::Div),
        ),
        (
            "=SUM(COUPDAYSNC({40568,40600},DATE(2011,11,15),2,1))",
            n(188.0),
        ),
        ("=COUPDAYSNC(A1,DATE(2011,11,15),2,1)", n(136.0)),
        ("=COUPDAYSNC(59,DATE(1901,2,28),2,0)", n(180.0)),
        ("=COUPDAYSNC(61,DATE(1901,8,31),2,0)", n(179.0)),
        ("=COUPDAYSNC(DATE(2023,2,24),DATE(2030,8,31),2,1)", n(4.0)),
        ("=COUPDAYSNC(DATE(2023,2,24),DATE(2030,8,31),4,1)", n(4.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,31),2,1)", n(184.0)),
        ("=COUPDAYSNC(DATE(2023,2,28),DATE(2030,8,31),4,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2023,3,1),DATE(2030,8,31),2,1)", n(183.0)),
        ("=COUPDAYSNC(DATE(2023,3,1),DATE(2030,8,31),4,1)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,1,15),DATE(2030,8,31),2,1)", n(45.0)),
        ("=COUPDAYSNC(DATE(2024,1,15),DATE(2030,8,31),4,1)", n(45.0)),
        ("=COUPDAYSNC(DATE(2024,2,27),DATE(2030,8,31),2,1)", n(2.0)),
        ("=COUPDAYSNC(DATE(2024,2,27),DATE(2030,8,31),4,1)", n(2.0)),
        ("=COUPDAYSNC(DATE(2024,2,28),DATE(2030,8,31),2,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,2,28),DATE(2030,8,31),4,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),2,1)", n(184.0)),
        ("=COUPDAYSNC(DATE(2024,2,29),DATE(2030,8,31),4,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,3,1),DATE(2030,8,31),2,1)", n(183.0)),
        ("=COUPDAYSNC(DATE(2024,3,1),DATE(2030,8,31),4,1)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,3,30),DATE(2030,8,31),2,1)", n(154.0)),
        ("=COUPDAYSNC(DATE(2024,3,30),DATE(2030,8,31),4,1)", n(62.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,31),2,1)", n(153.0)),
        ("=COUPDAYSNC(DATE(2024,3,31),DATE(2030,8,31),4,1)", n(61.0)),
        ("=COUPDAYSNC(DATE(2024,5,15),DATE(2030,8,31),2,1)", n(108.0)),
        ("=COUPDAYSNC(DATE(2024,5,15),DATE(2030,8,31),4,1)", n(16.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,31),2,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,5,31),DATE(2030,8,31),4,1)", n(92.0)),
        ("=COUPDAYSNC(DATE(2024,8,27),DATE(2030,8,31),2,1)", n(4.0)),
        ("=COUPDAYSNC(DATE(2024,8,27),DATE(2030,8,31),4,1)", n(4.0)),
        ("=COUPDAYSNC(DATE(2024,8,28),DATE(2030,8,31),2,1)", n(3.0)),
        ("=COUPDAYSNC(DATE(2024,8,28),DATE(2030,8,31),4,1)", n(3.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),2,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,30),DATE(2030,8,31),4,1)", n(1.0)),
        ("=COUPDAYSNC(DATE(2024,8,31),DATE(2030,8,31),2,1)", n(181.0)),
        ("=COUPDAYSNC(DATE(2024,8,31),DATE(2030,8,31),4,1)", n(91.0)),
        ("=COUPDAYSNC(DATE(2024,9,1),DATE(2030,8,31),2,1)", n(180.0)),
        ("=COUPDAYSNC(DATE(2024,9,1),DATE(2030,8,31),4,1)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,9,29),DATE(2030,8,31),2,1)", n(152.0)),
        ("=COUPDAYSNC(DATE(2024,9,29),DATE(2030,8,31),4,1)", n(62.0)),
        ("=COUPDAYSNC(DATE(2024,9,30),DATE(2030,8,31),2,1)", n(151.0)),
        ("=COUPDAYSNC(DATE(2024,9,30),DATE(2030,8,31),4,1)", n(61.0)),
        (
            "=COUPDAYSNC(DATE(2024,11,15),DATE(2030,8,31),2,1)",
            n(105.0),
        ),
        ("=COUPDAYSNC(DATE(2024,11,15),DATE(2030,8,31),4,1)", n(15.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,31),2,1)", n(90.0)),
        ("=COUPDAYSNC(DATE(2024,11,30),DATE(2030,8,31),4,1)", n(90.0)),
    ]);
}

#[test]
fn coupdays() {
    assert_cases(&[
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,31),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,31),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,31),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,31),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,31),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,31),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,30),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,30),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,30),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,30),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,30),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,30),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,30),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,30),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,8,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,29),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,29),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,29),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,29),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,29),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,29),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,29),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,29),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,8,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),4,1)", n(90.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,2,28),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,2,28),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,2,28),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,2,28),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,2,28),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,2,28),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,2,28),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),4,1)", n(90.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2032,2,29),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2032,2,29),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2032,2,29),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2032,2,29),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2032,2,29),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2032,2,29),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2032,2,29),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2032,2,29),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,11,30),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,5,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,5,31),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,5,31),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,5,31),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,5,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,5,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,5,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,5,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,5,31),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,5,31),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,5,31),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,5,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,5,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,5,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,5,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,5,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),2,2)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),2,3)", n(182.5)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),2,4)", n(180.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2025,1,15),DATE(2030,6,15),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,1)", n(181.0)),
        (
            "=COUPDAYS(DATE(2011,11,15),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,11,16),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),12,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2.9,1)",
            n(181.0),
        ),
        ("=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),4.5,1)", n(92.0)),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),1.5,1)",
            n(365.0),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),-2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),\"2\",1)",
            n(181.0),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),TRUE,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,-1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,4.9)",
            n(180.0),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,0.9)",
            n(180.0),
        ),
        ("=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,)", n(180.0)),
        ("=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2)", n(180.0)),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,\"1\")",
            n(181.0),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,TRUE)",
            error(ExcelErrorKind::Value),
        ),
        ("=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,A1)", n(180.0)),
        ("=COUPDAYS(40568.99,40862.2,2,1)", n(181.0)),
        ("=COUPDAYS(40862.1,40862.9,2,1)", error(ExcelErrorKind::Num)),
        ("=COUPDAYS(\"2011-01-25\",\"2011-11-15\",2,1)", n(181.0)),
        (
            "=COUPDAYS(\"abc\",DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPDAYS(-1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPDAYS(DATE(2011,1,25),2958466,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPDAYS(DATE(2011,1,25),2958465,2,1)", n(181.0)),
        (
            "=COUPDAYS(DATE(2011,1,25),1/0,2,1)",
            error(ExcelErrorKind::Div),
        ),
        (
            "=SUM(COUPDAYS({40568,40600},DATE(2011,11,15),2,1))",
            n(362.0),
        ),
        ("=COUPDAYS(59,DATE(1901,2,28),2,0)", n(180.0)),
        ("=COUPDAYS(61,DATE(1901,8,31),2,0)", n(180.0)),
        ("=COUPDAYS(DATE(2023,2,24),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,2,25),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,2,26),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,2,27),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,1),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,2),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,3),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,4),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,5),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,6),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,3,7),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,24),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,25),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,26),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,2),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,3),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,4),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,5),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,6),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,4,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,25),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,26),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,2),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,3),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,4),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,5),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,15),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,6,15),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,12,15),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,6,15),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2023,2,24),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,25),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,26),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,27),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,1),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,2),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,3),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,4),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,5),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,6),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,7),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,1,15),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,15),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,24),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,25),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,26),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,2),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,3),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,4),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,5),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,6),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,15),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,4,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,25),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,26),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,2),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,3),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,4),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,5),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,10,15),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,15),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,15),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,24),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,25),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,26),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,27),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,1),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,2),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,3),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,4),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,5),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,6),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,7),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,1,15),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,15),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,24),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,25),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,26),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,2),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,3),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,4),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,5),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,6),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,15),DATE(2030,9,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,4,15),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,15),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,15),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,15),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,15),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,25),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,26),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,2),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,3),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,4),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,5),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,15),DATE(2030,9,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,10,15),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,15),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,15),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,9,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,1,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,2,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,3,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,4,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,5,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,6,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,7,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,8,15),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,8,31),1,1)", n(366.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,9,15),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,10,15),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,15),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,12,15),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,8,31),1,1)", n(365.0)),
        ("=COUPDAYS(DATE(2023,2,24),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,24),DATE(2030,3,31),4,1)", n(90.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,2,28),DATE(2030,3,31),4,1)", n(90.0)),
        ("=COUPDAYS(DATE(2023,3,1),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2023,3,1),DATE(2030,3,31),4,1)", n(90.0)),
        ("=COUPDAYS(DATE(2024,1,15),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,15),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,3,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,5,15),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,15),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,3,31),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,3,31),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,3,31),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,3,31),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,3,31),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,3,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,3,31),4,1)", n(92.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,11,15),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,15),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,3,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,3,31),4,1)", n(91.0)),
        ("=COUPDAYS(DATE(2024,1,1),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,27),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,28),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,29),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,30),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,1),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,27),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,28),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,29),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,1),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,27),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,28),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,29),DATE(2030,10,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,1),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,27),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,28),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,29),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,30),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,1),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,27),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,28),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,29),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,1),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,27),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,28),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,29),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,30),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,1),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,27),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,28),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,1),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,27),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,28),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,29),DATE(2030,10,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,30),DATE(2030,10,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,1),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,27),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,28),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,29),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,1),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,27),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,28),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,29),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,30),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,10,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,1,1),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,27),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,28),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,29),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,30),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,1),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,27),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,28),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,29),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,1),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,27),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,28),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,29),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,1),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,27),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,28),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,29),DATE(2030,11,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,1),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,27),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,28),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,29),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,1),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,27),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,28),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,29),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,1),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,27),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,28),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,1),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,27),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,28),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,29),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,30),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,1),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,27),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,28),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,29),DATE(2030,11,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,1),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,27),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,28),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,29),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,30),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,11,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,1,1),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,27),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,28),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,29),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,30),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,1),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,27),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,28),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,29),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,1),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,27),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,28),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,29),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,1),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,27),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,28),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,29),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,30),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,1),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,27),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,28),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,29),DATE(2030,12,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,1),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,27),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,28),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,29),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,30),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,1),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,27),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,28),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,1),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,27),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,28),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,29),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,30),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,1),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,27),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,28),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,29),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,12,1),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,12,27),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,12,28),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,12,29),DATE(2030,12,31),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,12,30),DATE(2030,12,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,12,31),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,1,1),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,27),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,28),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,29),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,30),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,1),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,27),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,28),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,29),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,1),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,27),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,28),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,29),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,1),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,27),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,28),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,29),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,30),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,1),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,27),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,28),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,29),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,1),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,27),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,28),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,29),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,30),DATE(2031,1,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,1),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,27),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,28),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,1),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,27),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,28),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,29),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,30),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,1),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,27),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,28),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,29),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,1),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,27),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,28),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,29),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,30),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2031,1,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,1),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,27),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,28),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,29),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,30),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,1),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,27),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,28),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,29),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,1),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,27),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,28),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,29),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,1),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,27),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,28),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,29),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,30),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,1),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,27),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,28),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,29),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,1),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,27),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,28),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,29),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,30),DATE(2030,7,31),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,1),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,27),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,28),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,1),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,27),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,28),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,29),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,30),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,1),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,27),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,28),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,29),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,1),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,27),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,28),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,29),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,30),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,7,31),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,1,1),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,27),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,28),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,29),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,30),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,1,31),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,1),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,27),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,28),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,2,29),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,1),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,27),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,28),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,29),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,30),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,3,31),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,1),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,27),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,28),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,29),DATE(2030,4,30),2,1)", n(182.0)),
        ("=COUPDAYS(DATE(2024,4,30),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,1),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,27),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,28),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,29),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,30),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,5,31),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,1),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,27),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,28),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,29),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,6,30),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,1),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,27),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,28),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,29),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,30),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,7,31),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,1),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,27),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,28),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,29),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,30),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,8,31),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,1),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,27),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,28),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,29),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,9,30),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,1),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,27),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,28),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,29),DATE(2030,4,30),2,1)", n(183.0)),
        ("=COUPDAYS(DATE(2024,10,30),DATE(2030,4,30),2,1)", n(184.0)),
        ("=COUPDAYS(DATE(2024,10,31),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,1),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,27),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,28),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,29),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,11,30),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,1),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,27),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,28),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,29),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,30),DATE(2030,4,30),2,1)", n(181.0)),
        ("=COUPDAYS(DATE(2024,12,31),DATE(2030,4,30),2,1)", n(181.0)),
    ]);
}

#[test]
fn coupnum() {
    assert_cases(&[
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,31),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,31),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,31),4)", n(26.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,31),1)", n(8.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,31),2)", n(15.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,31),4)", n(30.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,31),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,31),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,31),4)", n(26.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,30),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,30),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,30),4)", n(26.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,30),1)", n(8.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,30),2)", n(15.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,30),4)", n(30.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,30),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,30),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,30),4)", n(26.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,29),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,29),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,8,29),4)", n(26.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,29),1)", n(8.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,29),2)", n(15.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,8,29),4)", n(30.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,29),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,29),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,8,29),4)", n(26.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,2,28),1)", n(6.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,2,28),2)", n(12.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,2,28),4)", n(24.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,2,28),1)", n(7.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,2,28),2)", n(14.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,2,28),4)", n(28.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,2,28),1)", n(6.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,2,28),2)", n(12.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,2,28),4)", n(24.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2032,2,29),1)", n(8.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2032,2,29),2)", n(16.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2032,2,29),4)", n(32.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2032,2,29),1)", n(9.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2032,2,29),2)", n(18.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2032,2,29),4)", n(36.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2032,2,29),1)", n(8.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2032,2,29),2)", n(16.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2032,2,29),4)", n(32.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,11,30),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,11,30),2)", n(14.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,11,30),4)", n(27.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,11,30),1)", n(8.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,11,30),2)", n(16.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,11,30),4)", n(31.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,11,30),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,11,30),2)", n(14.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,11,30),4)", n(27.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,5,31),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,5,31),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,5,31),4)", n(25.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,5,31),1)", n(8.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,5,31),2)", n(15.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,5,31),4)", n(29.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,5,31),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,5,31),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,5,31),4)", n(25.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,6,15),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,6,15),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,2,29),DATE(2030,6,15),4)", n(26.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,6,15),1)", n(8.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,6,15),2)", n(15.0)),
        ("=COUPNUM(DATE(2023,2,28),DATE(2030,6,15),4)", n(30.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,6,15),1)", n(7.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,6,15),2)", n(13.0)),
        ("=COUPNUM(DATE(2024,3,31),DATE(2030,6,15),4)", n(25.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,1)", n(2.0)),
        (
            "=COUPNUM(DATE(2011,11,15),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(DATE(2011,11,16),DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),12,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2.9,1)", n(2.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),4.5,1)", n(4.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),1.5,1)", n(1.0)),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),-2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),\"2\",1)", n(2.0)),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),TRUE,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,-1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,4.9)", n(2.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,0.9)", n(2.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,)", n(2.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2)", n(2.0)),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,\"1\")", n(2.0)),
        (
            "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,TRUE)",
            error(ExcelErrorKind::Value),
        ),
        ("=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,A1)", n(2.0)),
        ("=COUPNUM(40568.99,40862.2,2,1)", n(2.0)),
        ("=COUPNUM(40862.1,40862.9,2,1)", error(ExcelErrorKind::Num)),
        ("=COUPNUM(\"2011-01-25\",\"2011-11-15\",2,1)", n(2.0)),
        (
            "=COUPNUM(\"abc\",DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Value),
        ),
        (
            "=COUPNUM(-1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(0,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=COUPNUM(DATE(2011,1,25),2958466,2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNUM(DATE(2011,1,25),2958465,2,1)", n(15978.0)),
        (
            "=COUPNUM(DATE(2011,1,25),1/0,2,1)",
            error(ExcelErrorKind::Div),
        ),
        ("=SUM(COUPNUM({40568,40600},DATE(2011,11,15),2,1))", n(4.0)),
        (
            "=COUPNUM(A1,DATE(2011,11,15),2,1)",
            error(ExcelErrorKind::Num),
        ),
        ("=COUPNUM(59,DATE(1901,2,28),2,0)", n(2.0)),
        ("=COUPNUM(61,DATE(1901,8,31),2,0)", n(3.0)),
        (
            "=SUM(COUPNUM(DATE(2011,1,25),DATE(2011,11,15),{1,2,4}))",
            n(7.0),
        ),
    ]);
}

#[test]
fn price() {
    assert_cases(&[
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,1,0)",
            n(95.0780346202577),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,1,1)",
            n(95.07848312827248),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,2,0)",
            n(95.04287439939205),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,2,1)",
            n(95.04403378062287),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,2,2)",
            n(95.04522200592511),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,2,3)",
            n(95.04374127214868),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,2,4)",
            n(95.04287439939205),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,4,0)",
            n(95.02493064759942),
        ),
        (
            "=PRICE(DATE(2024,2,15),DATE(2032,11,15),0.0575,0.065,100,4,1)",
            n(95.02493064759942),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,1,0)",
            n(96.12036505736039),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,1,1)",
            n(96.11966270872861),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,2,0)",
            n(96.1104319641373),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,2,1)",
            n(96.10962867693007),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,2,2)",
            n(96.1104319641373),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,2,3)",
            n(96.10992562175049),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,2,4)",
            n(96.1104319641373),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,4,0)",
            n(96.08927855177909),
        ),
        (
            "=PRICE(DATE(2024,3,31),DATE(2030,8,31),0.0575,0.065,100,4,1)",
            n(96.08838533879137),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,1,0)",
            n(96.07619723024457),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,1,1)",
            n(96.07820737211034),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,2,0)",
            n(96.07491172253492),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,2,1)",
            n(96.07491172253492),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,2,2)",
            n(96.07491172253492),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,2,3)",
            n(96.07491172253492),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,2,4)",
            n(96.07491172253492),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,4,0)",
            n(96.0496443580148),
        ),
        (
            "=PRICE(DATE(2023,2,28),DATE(2029,8,30),0.0575,0.065,100,4,1)",
            n(96.0496443580148),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,1,0)",
            n(99.54437221953233),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,1,1)",
            n(99.54440389505467),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,2,0)",
            n(99.63680387409201),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,2,1)",
            n(99.63680387409201),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,2,2)",
            n(99.63680387409201),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,2,3)",
            n(99.63680387409201),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,2,4)",
            n(99.63680387409201),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,4,0)",
            n(99.63394652396705),
        ),
        (
            "=PRICE(DATE(2024,2,29),DATE(2024,8,31),0.0575,0.065,100,4,1)",
            n(99.63394652396705),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,1,0)",
            n(99.7720275039552),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,1,1)",
            n(99.7703360347438),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,2,0)",
            n(99.81296397712059),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,2,1)",
            n(99.81251255591567),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,2,2)",
            n(99.81713130358563),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,2,3)",
            n(99.81421717693317),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,2,4)",
            n(99.81504442611029),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,4,0)",
            n(99.83343221370329),
        ),
        (
            "=PRICE(DATE(2024,6,10),DATE(2024,8,31),0.0575,0.065,100,4,1)",
            n(99.8330356186216),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,1,0)",
            n(98.5695259479925),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,1,1)",
            n(98.5705700055486),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,2,0)",
            n(98.55399740480455),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,2,1)",
            n(98.55659126083445),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,2,2)",
            n(98.55990646720898),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,2,3)",
            n(98.55577519776085),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,2,4)",
            n(98.55399740480455),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,4,0)",
            n(98.54707408354015),
        ),
        (
            "=PRICE(DATE(2024,1,31),DATE(2026,2,28),0.0575,0.065,100,4,1)",
            n(98.54951591635512),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,1,0)",
            n(91.5362625018573),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,1,1)",
            n(91.53523502700618),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,2,0)",
            n(91.47291715264566),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,2,1)",
            n(91.47214393786089),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,2,2)",
            n(91.47530612185726),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,2,3)",
            n(91.47330801831605),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,2,4)",
            n(91.47371049630917),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,4,0)",
            n(91.44074754997057),
        ),
        (
            "=PRICE(DATE(2024,8,30),DATE(2045,8,31),0.0575,0.065,100,4,1)",
            n(91.44010071528803),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,1,0)",
            n(97.6722989614179),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,1,1)",
            n(97.67071043342614),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,2,0)",
            n(97.68550002241207),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,2,1)",
            n(97.68550002241207),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,2,2)",
            n(97.68550002241207),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,2,3)",
            n(97.68550002241207),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,2,4)",
            n(97.68550002241207),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,4,0)",
            n(97.66900366627067),
        ),
        (
            "=PRICE(DATE(2024,5,15),DATE(2027,11,15),0.0575,0.065,100,4,1)",
            n(97.66900366627067),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,100,2,0)",
            n(94.63436162132213),
        ),
        (
            "=PRICE(DATE(2017,11,15),DATE(2017,11,15),0.0575,0.065,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,100,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,100,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,100,2.9,0.9)",
            n(94.63436162132213),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,100,2)",
            n(94.63436162132213),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0,0.065,100,2,0)",
            n(53.59741245689783),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),-0.01,0.065,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0,100,2,0)",
            n(156.0625),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,-0.01,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,-100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,5,100,2,0)",
            n(0.7139529998252594),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,50,2,0)",
            n(67.8356553928732),
        ),
        (
            "=SUM(PRICE(DATE(2008,2,15),DATE(2017,11,15),{0.05,0.06},0.065,100,2,0))",
            n(185.7002928805204),
        ),
        (
            "=PRICE(DATE(2008,2,15),DATE(2017,11,15),TRUE,0.065,100,2,0)",
            error(ExcelErrorKind::Value),
        ),
    ]);
}

#[test]
fn yield_rows() {
    assert_cases(&[
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,2,0)",
            n(0.06500000688073143),
        ),
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,2,1)",
            n(0.06500182060552322),
        ),
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,2,2)",
            n(0.06500368033042932),
        ),
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,2,3)",
            n(0.06500136292831674),
        ),
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,2,4)",
            n(0.06500000688073143),
        ),
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,4,0)",
            n(0.06497211733409354),
        ),
        (
            "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,4,1)",
            n(0.06497211733409364),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,1,0)",
            n(0.06717277680363055),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,1,1)",
            n(0.0671709825251668),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,2,0)",
            n(0.06712544419402723),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,2,1)",
            n(0.06712335337151576),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,2,2)",
            n(0.06712544419402515),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,2,3)",
            n(0.06712412623668086),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,2,4)",
            n(0.06712544419402723),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,4,0)",
            n(0.06706831784441886),
        ),
        (
            "=YIELD(DATE(2024,3,31),DATE(2030,8,31),0.0575,95.04287,100,4,1)",
            n(0.067066068989318),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,1,0)",
            n(0.06706054135280443),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,1,1)",
            n(0.06706562357809447),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,2,0)",
            n(0.06703323336277238),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,2,1)",
            n(0.06703323336277163),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,2,2)",
            n(0.06703323336276965),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,2,3)",
            n(0.06703323336277163),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,2,4)",
            n(0.06703323336277058),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,4,0)",
            n(0.06696894379473586),
        ),
        (
            "=YIELD(DATE(2023,2,28),DATE(2029,8,30),0.0575,95.04287,100,4,1)",
            n(0.06696894379473578),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,1,0)",
            n(0.1594400664050874),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,1,1)",
            n(0.1594487620412592),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,2,0)",
            n(0.16390200682146755),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,2,1)",
            n(0.16481257352603126),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,2,2)",
            n(0.16481257352603126),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,2,3)",
            n(0.16481257352603126),
        ),
        (
            "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,2,4)",
            n(0.16390200682146755),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,1,0)",
            n(0.2784572733218053),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,1,1)",
            n(0.280145052240567),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,2,0)",
            n(0.2867412540423458),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,2,1)",
            n(0.28971038303204316),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,2,2)",
            n(0.28971038303204316),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,2,3)",
            n(0.28971038303204316),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,2,4)",
            n(0.28953392796253663),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,4,0)",
            n(0.2910708688465702),
        ),
        (
            "=YIELD(DATE(2024,6,10),DATE(2024,8,31),0.0575,95.04287,100,4,1)",
            n(0.2940849893898389),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,2,0)",
            n(0.08388428523368063),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,2,1)",
            n(0.08392988983768741),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,2,2)",
            n(0.08398832445577495),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,2,3)",
            n(0.08391553110722173),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,2,4)",
            n(0.08388428523368063),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,4,0)",
            n(0.08365780361120995),
        ),
        (
            "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,4,1)",
            n(0.08370168696237046),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,1,0)",
            n(0.0739171731378705),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,1,1)",
            n(0.07390555348672297),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,2,0)",
            n(0.07383030844927838),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,2,1)",
            n(0.07383030844927829),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,2,2)",
            n(0.07383030844927693),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,2,3)",
            n(0.07383030844927829),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,2,4)",
            n(0.07383030844927838),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,4,0)",
            n(0.07369767989399677),
        ),
        (
            "=YIELD(DATE(2024,5,15),DATE(2027,11,15),0.0575,95.04287,100,4,1)",
            n(0.07369767989399718),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2016,11,15),0.0575,95.04287,100,2,0)",
            n(0.06500000688073143),
        ),
        (
            "=YIELD(DATE(2017,11,15),DATE(2017,11,15),0.0575,95.04287,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,95.04287,100,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,95.04287,100,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,95.04287,100,2.9,0.9)",
            n(0.06440961173154552),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,95.04287,100,2)",
            n(0.06440961173154552),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0,95,100,2,0)",
            n(0.1155327342747113),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),-0.01,95,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),-0.01,95,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,0,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,0,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,-5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,-5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,95,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,95,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,50,100,2,0)",
            n(2.2983747027388475),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,150,100,2,0)",
            n(0.004915380868431943),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,150,100,2,0)",
            n(-0.6921946890843604),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,200,100,2,0)",
            n(-1.0674470155343512),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,10,100,2,0)",
            n(0.6015798428068028),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,10,100,2,0)",
            n(19.824472161297994),
        ),
        (
            "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,300,100,2,0)",
            n(-0.07665248761626749),
        ),
        (
            "=YIELD(DATE(2017,6,1),DATE(2017,11,15),0.0575,300,100,2,0)",
            n(-1.4430187293028613),
        ),
        (
            "=SUM(YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,{95,96},100,2,0))",
            n(0.1275099793690268),
        ),
        (
            "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,80,100,2,0)",
            n(0.07683701148300208),
        ),
        (
            "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,84,100,2,0)",
            n(0.07244423184985062),
        ),
        (
            "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,88,100,2,0)",
            n(0.06834613344930558),
        ),
        (
            "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,100,100,2,0)",
            n(0.0575),
        ),
    ]);
}

#[test]
fn duration() {
    assert_cases(&[
        (
            "=DURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,0)",
            n(6.245358539423187),
        ),
        (
            "=DURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,1)",
            n(6.242611286675935),
        ),
        (
            "=DURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,2)",
            n(6.239802983867633),
        ),
        (
            "=DURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,3)",
            n(6.243303744902639),
        ),
        (
            "=DURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,4)",
            n(6.245358539423187),
        ),
        (
            "=DURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,4,1)",
            n(6.287956909554371),
        ),
        (
            "=DURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,0)",
            n(5.068102888623162),
        ),
        (
            "=DURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,1)",
            n(5.069974869299491),
        ),
        (
            "=DURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,2)",
            n(5.068102888623162),
        ),
        (
            "=DURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,3)",
            n(5.069282492884957),
        ),
        (
            "=DURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,4)",
            n(5.068102888623162),
        ),
        (
            "=DURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,4,1)",
            n(5.005222891029268),
        ),
        (
            "=DURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,0)",
            n(5.154213999734274),
        ),
        (
            "=DURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,1)",
            n(5.154213999734274),
        ),
        (
            "=DURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,2)",
            n(5.154213999734274),
        ),
        (
            "=DURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,3)",
            n(5.154213999734274),
        ),
        (
            "=DURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,4)",
            n(5.154213999734274),
        ),
        (
            "=DURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,4,1)",
            n(5.089462021464051),
        ),
        (
            "=DURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,0)",
            n(0.5),
        ),
        (
            "=DURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,1)",
            n(0.5),
        ),
        (
            "=DURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,2)",
            n(0.5),
        ),
        (
            "=DURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,3)",
            n(0.5),
        ),
        (
            "=DURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,4)",
            n(0.5),
        ),
        (
            "=DURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,4,1)",
            n(0.49508626075255896),
        ),
        (
            "=DURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,0)",
            n(0.2222222222222222),
        ),
        (
            "=DURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,1)",
            n(0.22282608695652173),
        ),
        (
            "=DURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,2)",
            n(0.2166666666666667),
        ),
        (
            "=DURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,3)",
            n(0.2205479452054795),
        ),
        (
            "=DURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,4)",
            n(0.21944444444444441),
        ),
        (
            "=DURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,4,1)",
            n(0.22282608695652173),
        ),
        (
            "=DURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,0)",
            n(1.8958972204177569),
        ),
        (
            "=DURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,1)",
            n(1.8922342167547532),
        ),
        (
            "=DURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,2)",
            n(1.8875638870844236),
        ),
        (
            "=DURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,3)",
            n(1.8933858048926426),
        ),
        (
            "=DURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,4)",
            n(1.8958972204177569),
        ),
        (
            "=DURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,4,1)",
            n(1.908935273004717),
        ),
        (
            "=DURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,0)",
            n(9.576901152753921),
        ),
        (
            "=DURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,1)",
            n(9.57961854405826),
        ),
        (
            "=DURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,2)",
            n(9.568567819420576),
        ),
        (
            "=DURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,3)",
            n(9.57553128974021),
        ),
        (
            "=DURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,4)",
            n(9.574123374976134),
        ),
        (
            "=DURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,4,1)",
            n(9.614492900844306),
        ),
        (
            "=DURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,0)",
            n(3.114358760965974),
        ),
        (
            "=DURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,1)",
            n(3.114358760965974),
        ),
        (
            "=DURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,2)",
            n(3.114358760965974),
        ),
        (
            "=DURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,3)",
            n(3.114358760965974),
        ),
        (
            "=DURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,4)",
            n(3.114358760965974),
        ),
        (
            "=DURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,4,1)",
            n(3.0794021203184836),
        ),
        (
            "=DURATION(DATE(2018,7,1),DATE(2048,1,1),0.08,0.09,2,1)",
            n(10.919145281591932),
        ),
        (
            "=DURATION(DATE(2017,11,15),DATE(2017,11,15),0.08,0.09,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,2.9,0.9)",
            n(6.704496339267792),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,2)",
            n(6.704496339267792),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0,0.09,2,0)",
            n(9.75),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),-0.01,0.09,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0,2,0)",
            n(7.638888888888889),
        ),
        (
            "=DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,-0.01,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=SUM(DURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,{0.09,0.1},2,0))",
            n(13.294864748207283),
        ),
    ]);
}

#[test]
fn mduration() {
    assert_cases(&[
        (
            "=MDURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,0)",
            n(5.976419654950419),
        ),
        (
            "=MDURATION(DATE(2024,2,15),DATE(2032,11,15),0.08,0.09,2,1)",
            n(5.973790704953048),
        ),
        (
            "=MDURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,0)",
            n(4.849859223562835),
        ),
        (
            "=MDURATION(DATE(2024,3,31),DATE(2030,8,31),0.08,0.09,2,1)",
            n(4.851650592631093),
        ),
        (
            "=MDURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,0)",
            n(4.932262200702655),
        ),
        (
            "=MDURATION(DATE(2023,2,28),DATE(2029,8,30),0.08,0.09,2,1)",
            n(4.932262200702655),
        ),
        (
            "=MDURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,0)",
            n(0.47846889952153115),
        ),
        (
            "=MDURATION(DATE(2024,2,29),DATE(2024,8,31),0.08,0.09,2,1)",
            n(0.47846889952153115),
        ),
        (
            "=MDURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,0)",
            n(0.2126528442317916),
        ),
        (
            "=MDURATION(DATE(2024,6,10),DATE(2024,8,31),0.08,0.09,2,1)",
            n(0.2132307052215519),
        ),
        (
            "=MDURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,0)",
            n(1.8142557133184278),
        ),
        (
            "=MDURATION(DATE(2024,1,31),DATE(2026,2,28),0.08,0.09,2,1)",
            n(1.8107504466552664),
        ),
        (
            "=MDURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,0)",
            n(9.164498710769303),
        ),
        (
            "=MDURATION(DATE(2024,8,30),DATE(2045,8,31),0.08,0.09,2,1)",
            n(9.167099085223215),
        ),
        (
            "=MDURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,0)",
            n(2.9802476181492574),
        ),
        (
            "=MDURATION(DATE(2024,5,15),DATE(2027,11,15),0.08,0.09,2,1)",
            n(2.9802476181492574),
        ),
        (
            "=MDURATION(DATE(2008,1,1),DATE(2016,1,1),0.08,0.09,2,1)",
            n(5.735669813918839),
        ),
        (
            "=MDURATION(DATE(2017,11,15),DATE(2017,11,15),0.08,0.09,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,2.9,0.9)",
            n(6.415785970591189),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0.09,2)",
            n(6.415785970591189),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0,0.09,2,0)",
            n(9.330143540669857),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),-0.01,0.09,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,0,2,0)",
            n(7.638888888888889),
        ),
        (
            "=MDURATION(DATE(2008,2,15),DATE(2017,11,15),0.08,-0.01,2,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn accrint() {
    assert_cases(&[
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,1,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,1,1)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,1)",
            n(16.57608695652174),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,2)",
            n(16.944444444444446),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,3)",
            n(16.71232876712329),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,4)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,4,0)",
            n(16.944444444444446),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,4,1)",
            n(16.57608695652174),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,1,0)",
            n(112.5),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,1,1)",
            n(112.02185792349728),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,0)",
            n(112.5),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,0,FALSE)",
            n(112.22222222222223),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,1)",
            n(112.2282608695652),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,1,FALSE)",
            n(111.41304347826086),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,2)",
            n(113.61111111111111),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,2,FALSE)",
            n(113.88888888888889),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,3)",
            n(112.73972602739728),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,3,FALSE)",
            n(112.32876712328768),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,4)",
            n(112.50306936771027),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,2,4,FALSE)",
            n(112.22222222222223),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,4,0)",
            n(112.5),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,4,0,FALSE)",
            n(112.5),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,4,1)",
            n(112.2282608695652),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2025,4,15),0.1,1000,4,1,FALSE)",
            n(111.41304347826086),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,1,0)",
            n(308.61111111111114),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,1,1)",
            n(308.4931506849315),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,0)",
            n(308.61111111111114),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,0,FALSE)",
            n(308.61111111111114),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,1)",
            n(308.42391304347825),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,1,FALSE)",
            n(306.25),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,2)",
            n(308.61111111111114),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,2,FALSE)",
            n(313.05555555555554),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,3)",
            n(308.4931506849315),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,3,FALSE)",
            n(308.76712328767127),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,4)",
            n(308.8888888888889),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,2,4,FALSE)",
            n(308.8888888888889),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,4,0)",
            n(308.61111111111114),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,4,0,FALSE)",
            n(283.33333333333337),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,4,1)",
            n(308.42391304347825),
        ),
        (
            "=ACCRINT(DATE(2023,2,28),DATE(2023,8,31),DATE(2026,3,31),0.1,1000,4,1,FALSE)",
            n(281.25),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,1,0)",
            n(50.0),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,1,1)",
            n(49.72677595628415),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,2,0)",
            n(50.0),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,2,1)",
            n(50.0),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,2,2)",
            n(50.55555555555556),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,2,3)",
            n(49.86301369863014),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,2,4)",
            n(50.0),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,4,0)",
            n(50.0),
        ),
        (
            "=ACCRINT(DATE(2024,1,15),DATE(2024,7,15),DATE(2024,7,15),0.1,1000,4,1)",
            n(50.0),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,1,0)",
            n(19.444444444444446),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,1,1)",
            n(19.672131147540984),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,2,0)",
            n(19.444444444444446),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,2,1)",
            n(19.78021978021978),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,2,2)",
            n(20.0),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,2,3)",
            n(19.726027397260275),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,2,4)",
            n(19.444444444444446),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,4,0)",
            n(19.444444444444446),
        ),
        (
            "=ACCRINT(DATE(2023,11,30),DATE(2024,2,29),DATE(2024,2,10),0.1,1000,4,1)",
            n(19.78021978021978),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,1,0)",
            n(89.44444444444444),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,1,1)",
            n(89.04109589041096),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,0)",
            n(89.44444444444444),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,0,FALSE)",
            n(39.44444444444445),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,1)",
            n(89.13043478260869),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,1,FALSE)",
            n(39.13043478260869),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,2)",
            n(90.0),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,2,FALSE)",
            n(40.0),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,3)",
            n(89.45205479452055),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,3,FALSE)",
            n(39.45205479452055),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,4)",
            n(89.16666666666667),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,4,FALSE)",
            n(39.16666666666667),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,4,0)",
            n(89.44444444444444),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,4,0,FALSE)",
            n(14.444444444444443),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,4,1)",
            n(89.13043478260869),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,4,1,FALSE)",
            n(14.130434782608695),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,1,0)",
            n(8.333333333333332),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,1,1)",
            n(8.19672131147541),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,2,0)",
            n(8.333333333333332),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,2,1)",
            n(8.152173913043478),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,2,2)",
            n(8.333333333333332),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,2,3)",
            n(8.21917808219178),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,2,4)",
            n(8.333333333333332),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,4,0)",
            n(8.333333333333332),
        ),
        (
            "=ACCRINT(DATE(2024,9,1),DATE(2024,8,31),DATE(2024,10,1),0.1,1000,4,1)",
            n(8.152173913043478),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,1,0)",
            n(143.33333333333334),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,1,1)",
            n(143.013698630137),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,0)",
            n(143.33333333333334),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,0,FALSE)",
            n(93.33333333333333),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,1)",
            n(142.66304347826087),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,1,FALSE)",
            n(92.66304347826086),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,2)",
            n(143.61111111111111),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,2,FALSE)",
            n(94.72222222222221),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,3)",
            n(143.013698630137),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,3,FALSE)",
            n(93.42465753424656),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,4)",
            n(143.05555555555557),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,2,4,FALSE)",
            n(93.05555555555556),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,4,0)",
            n(143.33333333333334),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,4,0,FALSE)",
            n(68.33333333333333),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,4,1)",
            n(143.20652173913044),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2024,6,15),0.1,1000,4,1,FALSE)",
            n(67.66304347826086),
        ),
        (
            "=ACCRINT(DATE(2008,3,1),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2008,3,5),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0,FALSE)",
            n(15.555555555555555),
        ),
        (
            "=ACCRINT(DATE(2007,4,5),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0,TRUE)",
            n(107.50000000000001),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0,1000,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),-0.1,1000,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,-1000,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,,2,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,0,2)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,0,\"x\")",
            error(ExcelErrorKind::Value),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,0,)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,,FALSE)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,3,1),0.1,1000,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,2,1),0.1,1000,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,3,1),DATE(2024,5,1),0.1,1000,2,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,2,1),DATE(2024,5,1),0.1,1000,2,0)",
            n(16.666666666666664),
        ),
        (
            "=SUM(ACCRINT(DATE(2008,3,1),DATE(2008,8,31),DATE(2008,5,1),{0.1,0.2},1000,2,0))",
            n(49.99999999999999),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,A1,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,\"1000\",2,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),0.1,1000,2,0,\"FALSE\")",
            error(ExcelErrorKind::Value),
        ),
        (
            "=ACCRINT(DATE(2023,1,10),DATE(2024,1,31),DATE(2023,12,1),0.1,1000,2,0,A1)",
            n(39.44444444444445),
        ),
        (
            "=ACCRINT(DATE(2024,3,1),DATE(2024,8,31),DATE(2024,5,1),TRUE,1000,2,0)",
            error(ExcelErrorKind::Value),
        ),
    ]);
}

#[test]
fn accrintm() {
    assert_cases(&[
        (
            "=ACCRINTM(DATE(2024,3,1),DATE(2024,8,31),0.1,1000,0)",
            n(50.0),
        ),
        (
            "=ACCRINTM(DATE(2024,3,1),DATE(2024,8,31),0.1,1000,1)",
            n(50.0),
        ),
        (
            "=ACCRINTM(DATE(2024,3,1),DATE(2024,8,31),0.1,1000,2)",
            n(50.83333333333333),
        ),
        (
            "=ACCRINTM(DATE(2024,3,1),DATE(2024,8,31),0.1,1000,3)",
            n(50.136986301369866),
        ),
        (
            "=ACCRINTM(DATE(2024,3,1),DATE(2024,8,31),0.1,1000,4)",
            n(49.72222222222222),
        ),
        (
            "=ACCRINTM(DATE(2023,2,28),DATE(2023,8,31),0.1,1000,0)",
            n(50.27777777777778),
        ),
        (
            "=ACCRINTM(DATE(2023,2,28),DATE(2023,8,31),0.1,1000,1)",
            n(50.41095890410959),
        ),
        (
            "=ACCRINTM(DATE(2023,2,28),DATE(2023,8,31),0.1,1000,2)",
            n(51.11111111111111),
        ),
        (
            "=ACCRINTM(DATE(2023,2,28),DATE(2023,8,31),0.1,1000,3)",
            n(50.41095890410959),
        ),
        (
            "=ACCRINTM(DATE(2023,2,28),DATE(2023,8,31),0.1,1000,4)",
            n(50.55555555555556),
        ),
        (
            "=ACCRINTM(DATE(2023,7,31),DATE(2024,3,31),0.1,1000,0)",
            n(66.66666666666666),
        ),
        (
            "=ACCRINTM(DATE(2023,7,31),DATE(2024,3,31),0.1,1000,1)",
            n(66.66666666666666),
        ),
        (
            "=ACCRINTM(DATE(2023,7,31),DATE(2024,3,31),0.1,1000,2)",
            n(67.77777777777779),
        ),
        (
            "=ACCRINTM(DATE(2023,7,31),DATE(2024,3,31),0.1,1000,3)",
            n(66.84931506849315),
        ),
        (
            "=ACCRINTM(DATE(2023,7,31),DATE(2024,3,31),0.1,1000,4)",
            n(66.66666666666666),
        ),
        (
            "=ACCRINTM(DATE(2022,12,31),DATE(2026,2,28),0.1,1000,0)",
            n(316.1111111111111),
        ),
        (
            "=ACCRINTM(DATE(2022,12,31),DATE(2026,2,28),0.1,1000,1)",
            n(316.26506024096386),
        ),
        (
            "=ACCRINTM(DATE(2022,12,31),DATE(2026,2,28),0.1,1000,2)",
            n(320.83333333333337),
        ),
        (
            "=ACCRINTM(DATE(2022,12,31),DATE(2026,2,28),0.1,1000,3)",
            n(316.4383561643836),
        ),
        (
            "=ACCRINTM(DATE(2022,12,31),DATE(2026,2,28),0.1,1000,4)",
            n(316.1111111111111),
        ),
        (
            "=ACCRINTM(DATE(2023,12,31),DATE(2024,12,31),0.1,1000,0)",
            n(100.0),
        ),
        (
            "=ACCRINTM(DATE(2023,12,31),DATE(2024,12,31),0.1,1000,1)",
            n(100.0),
        ),
        (
            "=ACCRINTM(DATE(2023,12,31),DATE(2024,12,31),0.1,1000,2)",
            n(101.66666666666666),
        ),
        (
            "=ACCRINTM(DATE(2023,12,31),DATE(2024,12,31),0.1,1000,3)",
            n(100.27397260273973),
        ),
        (
            "=ACCRINTM(DATE(2023,12,31),DATE(2024,12,31),0.1,1000,4)",
            n(100.0),
        ),
        (
            "=ACCRINTM(DATE(2024,1,30),DATE(2024,3,31),0.1,1000,0)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINTM(DATE(2024,1,30),DATE(2024,3,31),0.1,1000,1)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINTM(DATE(2024,1,30),DATE(2024,3,31),0.1,1000,2)",
            n(16.944444444444446),
        ),
        (
            "=ACCRINTM(DATE(2024,1,30),DATE(2024,3,31),0.1,1000,3)",
            n(16.71232876712329),
        ),
        (
            "=ACCRINTM(DATE(2024,1,30),DATE(2024,3,31),0.1,1000,4)",
            n(16.666666666666664),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,1000,3)",
            n(20.54794520547945),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0,1000,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,,0)",
            n(20.555555555555554),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,1000,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,1000)",
            n(20.555555555555554),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),-0.1,1000,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,4,1),0.1,1000,0)",
            n(0.0),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,3,1),0.1,1000,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=SUM(ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,{1000,2000},3))",
            n(61.64383561643835),
        ),
        (
            "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,A1,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn disc() {
    assert_cases(&[
        (
            "=DISC(DATE(2024,2,16),DATE(2024,3,1),97.975,100,0)",
            n(0.48600000000000243),
        ),
        (
            "=DISC(DATE(2024,2,16),DATE(2024,3,1),97.975,100,1)",
            n(0.5293928571428598),
        ),
        (
            "=DISC(DATE(2024,2,16),DATE(2024,3,1),97.975,100,2)",
            n(0.5207142857142883),
        ),
        (
            "=DISC(DATE(2024,2,16),DATE(2024,3,1),97.975,100,3)",
            n(0.5279464285714313),
        ),
        (
            "=DISC(DATE(2024,2,16),DATE(2024,3,1),97.975,100,4)",
            n(0.48600000000000243),
        ),
        (
            "=DISC(DATE(2023,7,31),DATE(2024,3,31),97.975,100,0)",
            n(0.030375000000000152),
        ),
        (
            "=DISC(DATE(2023,7,31),DATE(2024,3,31),97.975,100,1)",
            n(0.030375000000000152),
        ),
        (
            "=DISC(DATE(2023,7,31),DATE(2024,3,31),97.975,100,2)",
            n(0.02987704918032802),
        ),
        (
            "=DISC(DATE(2023,7,31),DATE(2024,3,31),97.975,100,3)",
            n(0.030292008196721464),
        ),
        (
            "=DISC(DATE(2023,7,31),DATE(2024,3,31),97.975,100,4)",
            n(0.030375000000000152),
        ),
        (
            "=DISC(DATE(2023,2,28),DATE(2023,8,31),97.975,100,0)",
            n(0.04027624309392285),
        ),
        (
            "=DISC(DATE(2023,2,28),DATE(2023,8,31),97.975,100,1)",
            n(0.04016983695652194),
        ),
        (
            "=DISC(DATE(2023,2,28),DATE(2023,8,31),97.975,100,2)",
            n(0.0396195652173915),
        ),
        (
            "=DISC(DATE(2023,2,28),DATE(2023,8,31),97.975,100,3)",
            n(0.04016983695652194),
        ),
        (
            "=DISC(DATE(2023,2,28),DATE(2023,8,31),97.975,100,4)",
            n(0.04005494505494526),
        ),
        (
            "=DISC(DATE(2024,1,31),DATE(2026,2,28),97.975,100,0)",
            n(0.009745989304812883),
        ),
        (
            "=DISC(DATE(2024,1,31),DATE(2026,2,28),97.975,100,1)",
            n(0.009747035573122577),
        ),
        (
            "=DISC(DATE(2024,1,31),DATE(2026,2,28),97.975,100,2)",
            n(0.009604743083004),
        ),
        (
            "=DISC(DATE(2024,1,31),DATE(2026,2,28),97.975,100,3)",
            n(0.009738142292490167),
        ),
        (
            "=DISC(DATE(2024,1,31),DATE(2026,2,28),97.975,100,4)",
            n(0.009745989304812883),
        ),
        (
            "=DISC(DATE(2023,12,31),DATE(2024,12,31),97.975,100,0)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2023,12,31),DATE(2024,12,31),97.975,100,1)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2023,12,31),DATE(2024,12,31),97.975,100,2)",
            n(0.019918032786885344),
        ),
        (
            "=DISC(DATE(2023,12,31),DATE(2024,12,31),97.975,100,3)",
            n(0.02019467213114764),
        ),
        (
            "=DISC(DATE(2023,12,31),DATE(2024,12,31),97.975,100,4)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2024,3,1),DATE(2025,3,1),97.975,100,0)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2024,3,1),DATE(2025,3,1),97.975,100,1)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2024,3,1),DATE(2025,3,1),97.975,100,2)",
            n(0.019972602739726127),
        ),
        (
            "=DISC(DATE(2024,3,1),DATE(2025,3,1),97.975,100,3)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2024,3,1),DATE(2025,3,1),97.975,100,4)",
            n(0.0202500000000001),
        ),
        (
            "=DISC(DATE(2023,1,31),DATE(2023,3,31),97.975,100,0)",
            n(0.12150000000000061),
        ),
        (
            "=DISC(DATE(2023,1,31),DATE(2023,3,31),97.975,100,1)",
            n(0.1252754237288142),
        ),
        (
            "=DISC(DATE(2023,1,31),DATE(2023,3,31),97.975,100,2)",
            n(0.12355932203389891),
        ),
        (
            "=DISC(DATE(2023,1,31),DATE(2023,3,31),97.975,100,3)",
            n(0.1252754237288142),
        ),
        (
            "=DISC(DATE(2023,1,31),DATE(2023,3,31),97.975,100,4)",
            n(0.12150000000000061),
        ),
        (
            "=DISC(DATE(2018,1,7),DATE(2048,1,1),97.975,100,1)",
            n(0.0006754155608119487),
        ),
        (
            "=DISC(DATE(2008,3,1),DATE(2008,3,1),97.975,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DISC(DATE(2008,3,2),DATE(2008,3,1),97.975,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),97.975,100,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),97.975,100)",
            n(0.48600000000000243),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),97.975,100,3.9)",
            n(0.5279464285714313),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),0,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),-1,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),97.975,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=DISC(DATE(2008,2,16),DATE(2008,3,1),120,100,0)",
            n(-4.799999999999999),
        ),
        (
            "=SUM(DISC(DATE(2008,2,16),DATE(2008,3,1),{97,98},100,2))",
            n(1.285714285714287),
        ),
    ]);
}

#[test]
fn intrate() {
    assert_cases(&[
        (
            "=INTRATE(DATE(2024,2,16),DATE(2024,3,1),1000000,1014420,0)",
            n(0.34608),
        ),
        (
            "=INTRATE(DATE(2024,2,16),DATE(2024,3,1),1000000,1014420,1)",
            n(0.37698),
        ),
        (
            "=INTRATE(DATE(2024,2,16),DATE(2024,3,1),1000000,1014420,2)",
            n(0.3708),
        ),
        (
            "=INTRATE(DATE(2024,2,16),DATE(2024,3,1),1000000,1014420,3)",
            n(0.37595000000000006),
        ),
        (
            "=INTRATE(DATE(2024,2,16),DATE(2024,3,1),1000000,1014420,4)",
            n(0.34608),
        ),
        (
            "=INTRATE(DATE(2023,7,31),DATE(2024,3,31),1000000,1014420,0)",
            n(0.02163),
        ),
        (
            "=INTRATE(DATE(2023,7,31),DATE(2024,3,31),1000000,1014420,1)",
            n(0.02163),
        ),
        (
            "=INTRATE(DATE(2023,7,31),DATE(2024,3,31),1000000,1014420,2)",
            n(0.021275409836065576),
        ),
        (
            "=INTRATE(DATE(2023,7,31),DATE(2024,3,31),1000000,1014420,3)",
            n(0.021570901639344265),
        ),
        (
            "=INTRATE(DATE(2023,7,31),DATE(2024,3,31),1000000,1014420,4)",
            n(0.02163),
        ),
        (
            "=INTRATE(DATE(2023,2,28),DATE(2023,8,31),1000000,1014420,0)",
            n(0.028680662983425417),
        ),
        (
            "=INTRATE(DATE(2023,2,28),DATE(2023,8,31),1000000,1014420,1)",
            n(0.02860489130434783),
        ),
        (
            "=INTRATE(DATE(2023,2,28),DATE(2023,8,31),1000000,1014420,2)",
            n(0.028213043478260872),
        ),
        (
            "=INTRATE(DATE(2023,2,28),DATE(2023,8,31),1000000,1014420,3)",
            n(0.02860489130434783),
        ),
        (
            "=INTRATE(DATE(2023,2,28),DATE(2023,8,31),1000000,1014420,4)",
            n(0.028523076923076925),
        ),
        (
            "=INTRATE(DATE(2024,1,31),DATE(2026,2,28),1000000,1014420,0)",
            n(0.006940106951871658),
        ),
        (
            "=INTRATE(DATE(2024,1,31),DATE(2026,2,28),1000000,1014420,1)",
            n(0.006940851998243302),
        ),
        (
            "=INTRATE(DATE(2024,1,31),DATE(2026,2,28),1000000,1014420,2)",
            n(0.006839525691699605),
        ),
        (
            "=INTRATE(DATE(2024,1,31),DATE(2026,2,28),1000000,1014420,3)",
            n(0.006934519104084321),
        ),
        (
            "=INTRATE(DATE(2024,1,31),DATE(2026,2,28),1000000,1014420,4)",
            n(0.006940106951871658),
        ),
        (
            "=INTRATE(DATE(2023,12,31),DATE(2024,12,31),1000000,1014420,0)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2023,12,31),DATE(2024,12,31),1000000,1014420,1)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2023,12,31),DATE(2024,12,31),1000000,1014420,2)",
            n(0.014183606557377049),
        ),
        (
            "=INTRATE(DATE(2023,12,31),DATE(2024,12,31),1000000,1014420,3)",
            n(0.014380601092896175),
        ),
        (
            "=INTRATE(DATE(2023,12,31),DATE(2024,12,31),1000000,1014420,4)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2024,3,1),DATE(2025,3,1),1000000,1014420,0)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2024,3,1),DATE(2025,3,1),1000000,1014420,1)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2024,3,1),DATE(2025,3,1),1000000,1014420,2)",
            n(0.014222465753424658),
        ),
        (
            "=INTRATE(DATE(2024,3,1),DATE(2025,3,1),1000000,1014420,3)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2024,3,1),DATE(2025,3,1),1000000,1014420,4)",
            n(0.01442),
        ),
        (
            "=INTRATE(DATE(2023,1,31),DATE(2023,3,31),1000000,1014420,0)",
            n(0.08652),
        ),
        (
            "=INTRATE(DATE(2023,1,31),DATE(2023,3,31),1000000,1014420,1)",
            n(0.0892084745762712),
        ),
        (
            "=INTRATE(DATE(2023,1,31),DATE(2023,3,31),1000000,1014420,2)",
            n(0.0879864406779661),
        ),
        (
            "=INTRATE(DATE(2023,1,31),DATE(2023,3,31),1000000,1014420,3)",
            n(0.0892084745762712),
        ),
        (
            "=INTRATE(DATE(2023,1,31),DATE(2023,3,31),1000000,1014420,4)",
            n(0.08652),
        ),
        (
            "=INTRATE(DATE(2008,2,15),DATE(2008,5,15),1000000,1014420,2)",
            n(0.05768),
        ),
        (
            "=INTRATE(DATE(2008,3,1),DATE(2008,3,1),1000000,1014420,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=INTRATE(DATE(2008,3,2),DATE(2008,3,1),1000000,1014420,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),1000000,1014420,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),1000000,1014420)",
            n(0.34608),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),1000000,1014420,3.9)",
            n(0.37595000000000006),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),0,1014420,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),1000000,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),1000000,900000,0)",
            n(-2.4000000000000004),
        ),
        (
            "=INTRATE(DATE(2008,2,16),DATE(2008,3,1),-1,5,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn received() {
    assert_cases(&[
        (
            "=RECEIVED(DATE(2024,2,16),DATE(2024,3,1),1000000,0.0575,0)",
            n(1002401.5871358464),
        ),
        (
            "=RECEIVED(DATE(2024,2,16),DATE(2024,3,1),1000000,0.0575,1)",
            n(1002204.301811361),
        ),
        (
            "=RECEIVED(DATE(2024,2,16),DATE(2024,3,1),1000000,0.0575,2)",
            n(1002241.1225100572),
        ),
        (
            "=RECEIVED(DATE(2024,2,16),DATE(2024,3,1),1000000,0.0575,3)",
            n(1002210.3543431404),
        ),
        (
            "=RECEIVED(DATE(2024,2,16),DATE(2024,3,1),1000000,0.0575,4)",
            n(1002401.5871358464),
        ),
        (
            "=RECEIVED(DATE(2023,7,31),DATE(2024,3,31),1000000,0.0575,0)",
            n(1039861.3518197574),
        ),
        (
            "=RECEIVED(DATE(2023,7,31),DATE(2024,3,31),1000000,0.0575,1)",
            n(1039861.3518197574),
        ),
        (
            "=RECEIVED(DATE(2023,7,31),DATE(2024,3,31),1000000,0.0575,2)",
            n(1040552.649073619),
        ),
        (
            "=RECEIVED(DATE(2023,7,31),DATE(2024,3,31),1000000,0.0575,3)",
            n(1039974.9266319058),
        ),
        (
            "=RECEIVED(DATE(2023,7,31),DATE(2024,3,31),1000000,0.0575,4)",
            n(1039861.3518197574),
        ),
        (
            "=RECEIVED(DATE(2023,2,28),DATE(2023,8,31),1000000,0.0575,0)",
            n(1029770.3755086279),
        ),
        (
            "=RECEIVED(DATE(2023,2,28),DATE(2023,8,31),1000000,0.0575,1)",
            n(1029851.5885108064),
        ),
        (
            "=RECEIVED(DATE(2023,2,28),DATE(2023,8,31),1000000,0.0575,2)",
            n(1030278.7476389445),
        ),
        (
            "=RECEIVED(DATE(2023,2,28),DATE(2023,8,31),1000000,0.0575,3)",
            n(1029851.5885108064),
        ),
        (
            "=RECEIVED(DATE(2023,2,28),DATE(2023,8,31),1000000,0.0575,4)",
            n(1029939.7771324761),
        ),
        (
            "=RECEIVED(DATE(2024,1,31),DATE(2026,2,28),1000000,0.0575,0)",
            n(1135682.5136439635),
        ),
        (
            "=RECEIVED(DATE(2024,1,31),DATE(2026,2,28),1000000,0.0575,1)",
            n(1135665.9732818),
        ),
        (
            "=RECEIVED(DATE(2024,1,31),DATE(2026,2,28),1000000,0.0575,2)",
            n(1137953.1068490553),
        ),
        (
            "=RECEIVED(DATE(2024,1,31),DATE(2026,2,28),1000000,0.0575,3)",
            n(1135806.6950359023),
        ),
        (
            "=RECEIVED(DATE(2024,1,31),DATE(2026,2,28),1000000,0.0575,4)",
            n(1135682.5136439635),
        ),
        (
            "=RECEIVED(DATE(2023,12,31),DATE(2024,12,31),1000000,0.0575,0)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2023,12,31),DATE(2024,12,31),1000000,0.0575,1)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2023,12,31),DATE(2024,12,31),1000000,0.0575,2)",
            n(1062087.8877727133),
        ),
        (
            "=RECEIVED(DATE(2023,12,31),DATE(2024,12,31),1000000,0.0575,3)",
            n(1061185.3294762396),
        ),
        (
            "=RECEIVED(DATE(2023,12,31),DATE(2024,12,31),1000000,0.0575,4)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2024,3,1),DATE(2025,3,1),1000000,0.0575,0)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2024,3,1),DATE(2025,3,1),1000000,0.0575,1)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2024,3,1),DATE(2025,3,1),1000000,0.0575,2)",
            n(1061907.7467645),
        ),
        (
            "=RECEIVED(DATE(2024,3,1),DATE(2025,3,1),1000000,0.0575,3)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2024,3,1),DATE(2025,3,1),1000000,0.0575,4)",
            n(1061007.9575596817),
        ),
        (
            "=RECEIVED(DATE(2023,1,31),DATE(2023,3,31),1000000,0.0575,0)",
            n(1009676.0622633572),
        ),
        (
            "=RECEIVED(DATE(2023,1,31),DATE(2023,3,31),1000000,0.0575,1)",
            n(1009381.7191291663),
        ),
        (
            "=RECEIVED(DATE(2023,1,31),DATE(2023,3,31),1000000,0.0575,2)",
            n(1009513.2603773056),
        ),
        (
            "=RECEIVED(DATE(2023,1,31),DATE(2023,3,31),1000000,0.0575,3)",
            n(1009381.7191291663),
        ),
        (
            "=RECEIVED(DATE(2023,1,31),DATE(2023,3,31),1000000,0.0575,4)",
            n(1009676.0622633572),
        ),
        (
            "=RECEIVED(DATE(2008,2,15),DATE(2008,5,15),1000000,0.0575,2)",
            n(1014584.6544071021),
        ),
        (
            "=RECEIVED(DATE(2008,3,1),DATE(2008,3,1),1000000,0.0575,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=RECEIVED(DATE(2008,3,2),DATE(2008,3,1),1000000,0.0575,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),1000000,0.0575,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),1000000,0.0575)",
            n(1002401.5871358464),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),1000000,0.0575,3.9)",
            n(1002210.3543431404),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),0,0.0575,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),1000000,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),1000000,-0.01,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=RECEIVED(DATE(2008,2,16),DATE(2008,3,1),1000000,5,0)",
            n(1263157.8947368423),
        ),
    ]);
}

#[test]
fn pricedisc() {
    assert_cases(&[
        (
            "=PRICEDISC(DATE(2024,2,16),DATE(2024,3,1),0.0525,100,0)",
            n(99.78125),
        ),
        (
            "=PRICEDISC(DATE(2024,2,16),DATE(2024,3,1),0.0525,100,1)",
            n(99.79918032786885),
        ),
        (
            "=PRICEDISC(DATE(2024,2,16),DATE(2024,3,1),0.0525,100,2)",
            n(99.79583333333333),
        ),
        (
            "=PRICEDISC(DATE(2024,2,16),DATE(2024,3,1),0.0525,100,3)",
            n(99.7986301369863),
        ),
        (
            "=PRICEDISC(DATE(2024,2,16),DATE(2024,3,1),0.0525,100,4)",
            n(99.78125),
        ),
        (
            "=PRICEDISC(DATE(2023,7,31),DATE(2024,3,31),0.0525,100,0)",
            n(96.5),
        ),
        (
            "=PRICEDISC(DATE(2023,7,31),DATE(2024,3,31),0.0525,100,1)",
            n(96.5),
        ),
        (
            "=PRICEDISC(DATE(2023,7,31),DATE(2024,3,31),0.0525,100,2)",
            n(96.44166666666666),
        ),
        (
            "=PRICEDISC(DATE(2023,7,31),DATE(2024,3,31),0.0525,100,3)",
            n(96.4904109589041),
        ),
        (
            "=PRICEDISC(DATE(2023,7,31),DATE(2024,3,31),0.0525,100,4)",
            n(96.5),
        ),
        (
            "=PRICEDISC(DATE(2023,2,28),DATE(2023,8,31),0.0525,100,0)",
            n(97.36041666666667),
        ),
        (
            "=PRICEDISC(DATE(2023,2,28),DATE(2023,8,31),0.0525,100,1)",
            n(97.35342465753425),
        ),
        (
            "=PRICEDISC(DATE(2023,2,28),DATE(2023,8,31),0.0525,100,2)",
            n(97.31666666666666),
        ),
        (
            "=PRICEDISC(DATE(2023,2,28),DATE(2023,8,31),0.0525,100,3)",
            n(97.35342465753425),
        ),
        (
            "=PRICEDISC(DATE(2023,2,28),DATE(2023,8,31),0.0525,100,4)",
            n(97.34583333333333),
        ),
        (
            "=PRICEDISC(DATE(2024,1,31),DATE(2026,2,28),0.0525,100,0)",
            n(89.09166666666667),
        ),
        (
            "=PRICEDISC(DATE(2024,1,31),DATE(2026,2,28),0.0525,100,1)",
            n(89.09283759124088),
        ),
        (
            "=PRICEDISC(DATE(2024,1,31),DATE(2026,2,28),0.0525,100,2)",
            n(88.93125),
        ),
        (
            "=PRICEDISC(DATE(2024,1,31),DATE(2026,2,28),0.0525,100,3)",
            n(89.08287671232877),
        ),
        (
            "=PRICEDISC(DATE(2024,1,31),DATE(2026,2,28),0.0525,100,4)",
            n(89.09166666666667),
        ),
        (
            "=PRICEDISC(DATE(2023,12,31),DATE(2024,12,31),0.0525,100,0)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2023,12,31),DATE(2024,12,31),0.0525,100,1)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2023,12,31),DATE(2024,12,31),0.0525,100,2)",
            n(94.6625),
        ),
        (
            "=PRICEDISC(DATE(2023,12,31),DATE(2024,12,31),0.0525,100,3)",
            n(94.73561643835616),
        ),
        (
            "=PRICEDISC(DATE(2023,12,31),DATE(2024,12,31),0.0525,100,4)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2024,3,1),DATE(2025,3,1),0.0525,100,0)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2024,3,1),DATE(2025,3,1),0.0525,100,1)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2024,3,1),DATE(2025,3,1),0.0525,100,2)",
            n(94.67708333333333),
        ),
        (
            "=PRICEDISC(DATE(2024,3,1),DATE(2025,3,1),0.0525,100,3)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2024,3,1),DATE(2025,3,1),0.0525,100,4)",
            n(94.75),
        ),
        (
            "=PRICEDISC(DATE(2023,1,31),DATE(2023,3,31),0.0525,100,0)",
            n(99.125),
        ),
        (
            "=PRICEDISC(DATE(2023,1,31),DATE(2023,3,31),0.0525,100,1)",
            n(99.1513698630137),
        ),
        (
            "=PRICEDISC(DATE(2023,1,31),DATE(2023,3,31),0.0525,100,2)",
            n(99.13958333333333),
        ),
        (
            "=PRICEDISC(DATE(2023,1,31),DATE(2023,3,31),0.0525,100,3)",
            n(99.1513698630137),
        ),
        (
            "=PRICEDISC(DATE(2023,1,31),DATE(2023,3,31),0.0525,100,4)",
            n(99.125),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0.0525,100,2)",
            n(99.79583333333333),
        ),
        (
            "=PRICEDISC(DATE(2008,3,1),DATE(2008,3,1),0.0525,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEDISC(DATE(2008,3,2),DATE(2008,3,1),0.0525,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0.0525,100,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0.0525,100)",
            n(99.78125),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0.0525,100,3.9)",
            n(99.7986301369863),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0.0525,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),50,100,0)",
            n(-108.33333333333334),
        ),
        (
            "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),-0.05,100,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn yielddisc() {
    assert_cases(&[
        (
            "=YIELDDISC(DATE(2024,2,16),DATE(2024,3,1),99.795,100,0)",
            n(0.04930106718773444),
        ),
        (
            "=YIELDDISC(DATE(2024,2,16),DATE(2024,3,1),99.795,100,1)",
            n(0.05370294818663931),
        ),
        (
            "=YIELDDISC(DATE(2024,2,16),DATE(2024,3,1),99.795,100,2)",
            n(0.05282257198685834),
        ),
        (
            "=YIELDDISC(DATE(2024,2,16),DATE(2024,3,1),99.795,100,3)",
            n(0.05355621882000915),
        ),
        (
            "=YIELDDISC(DATE(2024,2,16),DATE(2024,3,1),99.795,100,4)",
            n(0.04930106718773444),
        ),
        (
            "=YIELDDISC(DATE(2023,7,31),DATE(2024,3,31),99.795,100,0)",
            n(0.0030813166992334027),
        ),
        (
            "=YIELDDISC(DATE(2023,7,31),DATE(2024,3,31),99.795,100,1)",
            n(0.0030813166992334027),
        ),
        (
            "=YIELDDISC(DATE(2023,7,31),DATE(2024,3,31),99.795,100,2)",
            n(0.00303080331072138),
        ),
        (
            "=YIELDDISC(DATE(2023,7,31),DATE(2024,3,31),99.795,100,3)",
            n(0.0030728978011480656),
        ),
        (
            "=YIELDDISC(DATE(2023,7,31),DATE(2024,3,31),99.795,100,4)",
            n(0.0030813166992334027),
        ),
        (
            "=YIELDDISC(DATE(2023,2,28),DATE(2023,8,31),99.795,100,0)",
            n(0.00408572380008849),
        ),
        (
            "=YIELDDISC(DATE(2023,2,28),DATE(2023,8,31),99.795,100,1)",
            n(0.004074929692826783),
        ),
        (
            "=YIELDDISC(DATE(2023,2,28),DATE(2023,8,31),99.795,100,2)",
            n(0.004019108738130526),
        ),
        (
            "=YIELDDISC(DATE(2023,2,28),DATE(2023,8,31),99.795,100,3)",
            n(0.004074929692826783),
        ),
        (
            "=YIELDDISC(DATE(2023,2,28),DATE(2023,8,31),99.795,100,4)",
            n(0.004063274768219872),
        ),
        (
            "=YIELDDISC(DATE(2024,1,31),DATE(2026,2,28),99.795,100,0)",
            n(0.0009886577644599153),
        ),
        (
            "=YIELDDISC(DATE(2024,1,31),DATE(2026,2,28),99.795,100,1)",
            n(0.0009887639005591595),
        ),
        (
            "=YIELDDISC(DATE(2024,1,31),DATE(2026,2,28),99.795,100,2)",
            n(0.0009743293910619455),
        ),
        (
            "=YIELDDISC(DATE(2024,1,31),DATE(2026,2,28),99.795,100,3)",
            n(0.0009878617437155837),
        ),
        (
            "=YIELDDISC(DATE(2024,1,31),DATE(2026,2,28),99.795,100,4)",
            n(0.0009886577644599153),
        ),
        (
            "=YIELDDISC(DATE(2023,12,31),DATE(2024,12,31),99.795,100,0)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2023,12,31),DATE(2024,12,31),99.795,100,1)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2023,12,31),DATE(2024,12,31),99.795,100,2)",
            n(0.00202053554048092),
        ),
        (
            "=YIELDDISC(DATE(2023,12,31),DATE(2024,12,31),99.795,100,3)",
            n(0.0020485985340987106),
        ),
        (
            "=YIELDDISC(DATE(2023,12,31),DATE(2024,12,31),99.795,100,4)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2024,3,1),DATE(2025,3,1),99.795,100,0)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2024,3,1),DATE(2025,3,1),99.795,100,1)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2024,3,1),DATE(2025,3,1),99.795,100,2)",
            n(0.0020260712542904567),
        ),
        (
            "=YIELDDISC(DATE(2024,3,1),DATE(2025,3,1),99.795,100,3)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2024,3,1),DATE(2025,3,1),99.795,100,4)",
            n(0.0020542111328222686),
        ),
        (
            "=YIELDDISC(DATE(2023,1,31),DATE(2023,3,31),99.795,100,0)",
            n(0.01232526679693361),
        ),
        (
            "=YIELDDISC(DATE(2023,1,31),DATE(2023,3,31),99.795,100,1)",
            n(0.01270825531322251),
        ),
        (
            "=YIELDDISC(DATE(2023,1,31),DATE(2023,3,31),99.795,100,2)",
            n(0.012534169624000283),
        ),
        (
            "=YIELDDISC(DATE(2023,1,31),DATE(2023,3,31),99.795,100,3)",
            n(0.01270825531322251),
        ),
        (
            "=YIELDDISC(DATE(2023,1,31),DATE(2023,3,31),99.795,100,4)",
            n(0.01232526679693361),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,100,2)",
            n(0.05282257198685834),
        ),
        (
            "=YIELDDISC(DATE(2008,3,1),DATE(2008,3,1),99.795,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDDISC(DATE(2008,3,2),DATE(2008,3,1),99.795,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,100,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,100)",
            n(0.04930106718773444),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,100,3.9)",
            n(0.05355621882000915),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),0,100,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),120,100,0)",
            n(-4.0),
        ),
        (
            "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,-1,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn pricemat() {
    assert_cases(&[
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.061,0.061,0)",
            n(99.92184640785078),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.061,0.061,1)",
            n(99.92212150382375),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.061,0.061,2)",
            n(99.919554041118),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.061,0.061,3)",
            n(99.92170220668623),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.061,0.061,4)",
            n(99.92184640785078),
        ),
        (
            "=PRICEMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.061,0.061,0)",
            n(99.74333556543138),
        ),
        (
            "=PRICEMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.061,0.061,1)",
            n(99.74243396061682),
        ),
        (
            "=PRICEMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.061,0.061,2)",
            n(99.73553731982828),
        ),
        (
            "=PRICEMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.061,0.061,3)",
            n(99.74243396061682),
        ),
        (
            "=PRICEMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.061,0.061,4)",
            n(99.74235634427254),
        ),
        (
            "=PRICEMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.061,0.061,0)",
            n(99.19852991978478),
        ),
        (
            "=PRICEMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.061,0.061,1)",
            n(99.19997090951658),
        ),
        (
            "=PRICEMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.061,0.061,2)",
            n(99.1774645660261),
        ),
        (
            "=PRICEMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.061,0.061,3)",
            n(99.19859151374891),
        ),
        (
            "=PRICEMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.061,0.061,4)",
            n(99.19852991978478),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.061,0.061,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.061,0.061,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.061,0.061,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.061,0.061,3)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.061,0.061,4)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.061,0.061,0)",
            n(99.96930374525655),
        ),
        (
            "=PRICEMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.061,0.061,1)",
            n(99.96947119111124),
        ),
        (
            "=PRICEMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.061,0.061,2)",
            n(99.96862160626226),
        ),
        (
            "=PRICEMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.061,0.061,3)",
            n(99.96947119111124),
        ),
        (
            "=PRICEMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.061,0.061,4)",
            n(99.96930374525655),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.061,0.061,0)",
            n(99.98449887555694),
        ),
        (
            "=PRICEMAT(DATE(2008,4,13),DATE(2008,4,13),DATE(2007,11,11),0.061,0.061,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2008,3,1),0.061,0.061,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2008,5,1),0.061,0.061,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.061,0.061,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.061,0.061)",
            n(99.98449887555694),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0,0.061,0)",
            n(99.02678674581475),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),-0.01,0.061,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.061,0,0)",
            n(100.98277777777777),
        ),
        (
            "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.061,-0.01,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=SUM(PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),{0.061,0.07},0.061,0))",
            n(200.11029954074797),
        ),
    ]);
}

#[test]
fn yieldmat() {
    assert_cases(&[
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.0625,100.0123,0)",
            n(0.06098540795829226),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.0625,100.0123,1)",
            n(0.06099726214424531),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.0625,100.0123,2)",
            n(0.0609791249446491),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.0625,100.0123,3)",
            n(0.06099428689820343),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,11,3),DATE(2023,11,11),0.0625,100.0123,4)",
            n(0.06098540795829226),
        ),
        (
            "=YIELDMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.0625,100.0123,0)",
            n(0.06050899415896217),
        ),
        (
            "=YIELDMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.0625,100.0123,1)",
            n(0.06050418673630381),
        ),
        (
            "=YIELDMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.0625,100.0123,2)",
            n(0.060479622427918),
        ),
        (
            "=YIELDMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.0625,100.0123,3)",
            n(0.06050418673630381),
        ),
        (
            "=YIELDMAT(DATE(2023,8,31),DATE(2025,2,28),DATE(2023,2,28),0.0625,100.0123,4)",
            n(0.0604986640979517),
        ),
        (
            "=YIELDMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.0625,100.0123,0)",
            n(0.05820021827687186),
        ),
        (
            "=YIELDMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.0625,100.0123,1)",
            n(0.05820206186479401),
        ),
        (
            "=YIELDMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.0625,100.0123,2)",
            n(0.058144486822579694),
        ),
        (
            "=YIELDMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.0625,100.0123,3)",
            n(0.05819851006268225),
        ),
        (
            "=YIELDMAT(DATE(2024,2,29),DATE(2026,3,31),DATE(2022,12,31),0.0625,100.0123,4)",
            n(0.05820021827687186),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.0625,100.0123,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.0625,100.0123,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.0625,100.0123,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.0625,100.0123,3)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2024,3,15),DATE(2024,9,15),DATE(2024,3,15),0.0625,100.0123,4)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.0625,100.0123,0)",
            n(0.059883281800087076),
        ),
        (
            "=YIELDMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.0625,100.0123,1)",
            n(0.05985843666795729),
        ),
        (
            "=YIELDMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.0625,100.0123,2)",
            n(0.05984298829738398),
        ),
        (
            "=YIELDMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.0625,100.0123,3)",
            n(0.05985843666795729),
        ),
        (
            "=YIELDMAT(DATE(2024,1,31),DATE(2024,3,31),DATE(2023,7,31),0.0625,100.0123,4)",
            n(0.059883281800087076),
        ),
        (
            "=YIELDMAT(DATE(2008,3,15),DATE(2008,11,3),DATE(2007,11,11),0.0625,100.0123,0)",
            n(0.06098540795829226),
        ),
        (
            "=YIELDMAT(DATE(2008,4,13),DATE(2008,4,13),DATE(2007,11,11),0.0625,100.0123,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2008,3,1),0.0625,100.0123,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2008,5,1),0.0625,100.0123,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.0625,100.0123,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.0625,100.0123)",
            n(0.06073787262778142),
        ),
        (
            "=YIELDMAT(DATE(2008,3,15),DATE(2008,11,3),DATE(2007,11,11),0,99,0)",
            n(0.015948963317384386),
        ),
        (
            "=YIELDMAT(DATE(2008,3,15),DATE(2008,11,3),DATE(2007,11,11),-0.01,99,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2008,3,15),DATE(2008,11,3),DATE(2007,11,11),0.0625,0,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=YIELDMAT(DATE(2008,3,15),DATE(2008,11,3),DATE(2007,11,11),0.0625,-1,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn tbilleq() {
    assert_cases(&[
        (
            "=TBILLEQ(DATE(2024,1,1),DATE(2024,1,2),0.0914)",
            n(0.09269297816167771),
        ),
        (
            "=TBILLEQ(DATE(2024,1,1),DATE(2024,4,1),0.0914)",
            n(0.09486110487126743),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2023,7,2),0.0914)",
            n(0.09715894330584461),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2023,7,3),0.0914)",
            n(0.09717191339563704),
        ),
        (
            "=TBILLEQ(DATE(2023,3,31),DATE(2023,10,17),0.0914)",
            n(0.09721325617642639),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2023,12,31),0.0914)",
            n(0.09963083108503262),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2024,1,1),0.0914)",
            n(0.09965155236972745),
        ),
        (
            "=TBILLEQ(DATE(2024,1,1),DATE(2025,1,1),0.0914)",
            n(0.09994537588135577),
        ),
        (
            "=TBILLEQ(DATE(2023,3,1),DATE(2024,3,1),0.0914)",
            n(0.09994537588135577),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2024,1,3),0.0914)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLEQ(DATE(2023,3,1),DATE(2024,2,29),0.0914)",
            n(0.09965155236972745),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),0.0914)",
            n(0.09415149356594302),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),-0.01)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),2)",
            n(3.093220338983051),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),6)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),1.5)",
            n(2.050561797752809),
        ),
        (
            "=TBILLEQ(DATE(2008,3,31),DATE(2008,3,31),0.09)",
            error(ExcelErrorKind::Num),
        ),
        ("=TBILLEQ(39538.9,39600.1,0.09)", n(0.09268664296597258)),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2023,12,31),0.5)",
            n(0.8465931578688602),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2023,12,31),0.9)",
            n(4.67949954082376),
        ),
        (
            "=TBILLEQ(DATE(2023,1,1),DATE(2024,1,2),0.0914)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLEQ(DATE(2024,2,29),DATE(2025,2,28),0.0914)",
            n(0.09965155236972745),
        ),
        (
            "=TBILLEQ(DATE(2024,2,29),DATE(2025,3,1),0.0914)",
            n(0.09994537588135577),
        ),
        (
            "=TBILLEQ(DATE(2023,2,28),DATE(2024,2,29),0.0914)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLEQ(DATE(2023,2,28),DATE(2024,2,28),0.0914)",
            n(0.09965155236972745),
        ),
        (
            "=TBILLEQ(DATE(2024,1,1),DATE(2024,12,31),0.0914)",
            n(0.09965155236972745),
        ),
    ]);
}

#[test]
fn tbillprice() {
    assert_cases(&[
        (
            "=TBILLPRICE(DATE(2024,1,1),DATE(2024,1,2),0.09)",
            n(99.97500000000001),
        ),
        ("=TBILLPRICE(DATE(2024,1,1),DATE(2024,4,1),0.09)", n(97.725)),
        ("=TBILLPRICE(DATE(2023,1,1),DATE(2023,7,2),0.09)", n(95.45)),
        (
            "=TBILLPRICE(DATE(2023,1,1),DATE(2023,7,3),0.09)",
            n(95.42500000000001),
        ),
        (
            "=TBILLPRICE(DATE(2023,3,31),DATE(2023,10,17),0.09)",
            n(95.0),
        ),
        ("=TBILLPRICE(DATE(2023,1,1),DATE(2023,12,31),0.09)", n(90.9)),
        ("=TBILLPRICE(DATE(2023,1,1),DATE(2024,1,1),0.09)", n(90.875)),
        ("=TBILLPRICE(DATE(2024,1,1),DATE(2025,1,1),0.09)", n(90.85)),
        ("=TBILLPRICE(DATE(2023,3,1),DATE(2024,3,1),0.09)", n(90.85)),
        (
            "=TBILLPRICE(DATE(2023,1,1),DATE(2024,1,3),0.09)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLPRICE(DATE(2023,3,1),DATE(2024,2,29),0.09)",
            n(90.875),
        ),
        ("=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),0.09)", n(98.45)),
        (
            "=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),-0.01)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),2)",
            n(65.55555555555556),
        ),
        (
            "=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),6)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),4.5)",
            n(22.499999999999996),
        ),
        (
            "=TBILLPRICE(DATE(2008,3,31),DATE(2008,3,31),0.09)",
            error(ExcelErrorKind::Num),
        ),
        ("=TBILLPRICE(39538.9,39600.1,0.09)", n(98.45)),
        (
            "=TBILLPRICE(DATE(2023,1,1),DATE(2024,1,2),0.09)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLPRICE(DATE(2024,2,29),DATE(2025,2,28),0.09)",
            n(90.875),
        ),
        ("=TBILLPRICE(DATE(2024,2,29),DATE(2025,3,1),0.09)", n(90.85)),
        (
            "=TBILLPRICE(DATE(2023,2,28),DATE(2024,2,29),0.09)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLPRICE(DATE(2023,2,28),DATE(2024,2,28),0.09)",
            n(90.875),
        ),
        (
            "=TBILLPRICE(DATE(2024,1,1),DATE(2024,12,31),0.09)",
            n(90.875),
        ),
        (
            "=SUM(TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),{0.09,0.1}))",
            n(196.72777777777776),
        ),
    ]);
}

#[test]
fn tbillyield() {
    assert_cases(&[
        (
            "=TBILLYIELD(DATE(2024,1,1),DATE(2024,1,2),98.45)",
            n(5.6678517013712435),
        ),
        (
            "=TBILLYIELD(DATE(2024,1,1),DATE(2024,4,1),98.45)",
            n(0.06228408463045323),
        ),
        (
            "=TBILLYIELD(DATE(2023,1,1),DATE(2023,7,2),98.45)",
            n(0.031142042315226614),
        ),
        (
            "=TBILLYIELD(DATE(2023,1,1),DATE(2023,7,3),98.45)",
            n(0.030971867220607886),
        ),
        (
            "=TBILLYIELD(DATE(2023,3,31),DATE(2023,10,17),98.45)",
            n(0.02833925850685622),
        ),
        (
            "=TBILLYIELD(DATE(2023,1,1),DATE(2023,12,31),98.45)",
            n(0.015571021157613307),
        ),
        (
            "=TBILLYIELD(DATE(2023,1,1),DATE(2024,1,1),98.45)",
            n(0.01552836082567464),
        ),
        (
            "=TBILLYIELD(DATE(2024,1,1),DATE(2025,1,1),98.45)",
            n(0.015485933610303943),
        ),
        (
            "=TBILLYIELD(DATE(2023,3,1),DATE(2024,3,1),98.45)",
            n(0.015485933610303943),
        ),
        (
            "=TBILLYIELD(DATE(2023,1,1),DATE(2024,1,3),98.45)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLYIELD(DATE(2023,3,1),DATE(2024,2,29),98.45)",
            n(0.01552836082567464),
        ),
        (
            "=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),98.45)",
            n(0.09141696292534264),
        ),
        (
            "=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),-1)",
            error(ExcelErrorKind::Num),
        ),
        ("=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),100)", n(0.0)),
        (
            "=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),150)",
            n(-1.935483870967742),
        ),
        (
            "=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),0.5)",
            n(1155.483870967742),
        ),
        (
            "=TBILLYIELD(DATE(2008,3,31),DATE(2008,3,31),0.09)",
            error(ExcelErrorKind::Num),
        ),
        ("=TBILLYIELD(39538.9,39600.1,0.09)", n(6445.806451612903)),
        (
            "=TBILLYIELD(DATE(2023,1,1),DATE(2024,1,2),98.45)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLYIELD(DATE(2024,2,29),DATE(2025,2,28),98.45)",
            n(0.01552836082567464),
        ),
        (
            "=TBILLYIELD(DATE(2024,2,29),DATE(2025,3,1),98.45)",
            n(0.015485933610303943),
        ),
        (
            "=TBILLYIELD(DATE(2023,2,28),DATE(2024,2,29),98.45)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=TBILLYIELD(DATE(2023,2,28),DATE(2024,2,28),98.45)",
            n(0.01552836082567464),
        ),
        (
            "=TBILLYIELD(DATE(2024,1,1),DATE(2024,12,31),98.45)",
            n(0.01552836082567464),
        ),
    ]);
}

#[test]
fn oddfprice() {
    assert_cases(&[
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,0)",
            n(113.59920582823824),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            n(113.59771747407883),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,2)",
            n(113.5987996083253),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,3)",
            n(113.5961125952049),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,4)",
            n(113.59920582823824),
        ),
        (
            "=ODDFPRICE(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0785,0.0625,100,2,0)",
            n(105.4914784371264),
        ),
        (
            "=ODDFPRICE(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0785,0.0625,100,2,1)",
            n(105.4698794878743),
        ),
        (
            "=ODDFPRICE(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0785,0.0625,100,2,2)",
            n(105.43017326105627),
        ),
        (
            "=ODDFPRICE(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0785,0.0625,100,2,3)",
            n(105.47879273000068),
        ),
        (
            "=ODDFPRICE(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0785,0.0625,100,2,4)",
            n(105.4914784371264),
        ),
        (
            "=ODDFPRICE(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0785,0.0625,100,2,0)",
            n(106.45263480403213),
        ),
        (
            "=ODDFPRICE(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0785,0.0625,100,2,1)",
            n(106.45253468796287),
        ),
        (
            "=ODDFPRICE(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0785,0.0625,100,2,2)",
            n(106.32468277220532),
        ),
        (
            "=ODDFPRICE(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0785,0.0625,100,2,3)",
            n(106.43047713129518),
        ),
        (
            "=ODDFPRICE(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0785,0.0625,100,2,4)",
            n(106.45263480403213),
        ),
        (
            "=ODDFPRICE(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0785,0.0625,100,4,0)",
            n(103.63680959557986),
        ),
        (
            "=ODDFPRICE(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0785,0.0625,100,4,1)",
            n(103.61798190384522),
        ),
        (
            "=ODDFPRICE(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0785,0.0625,100,4,2)",
            n(103.61500404002432),
        ),
        (
            "=ODDFPRICE(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0785,0.0625,100,4,3)",
            n(103.63821525654438),
        ),
        (
            "=ODDFPRICE(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0785,0.0625,100,4,4)",
            n(103.63680959557986),
        ),
        (
            "=ODDFPRICE(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0785,0.0625,100,1,0)",
            n(108.23858198575189),
        ),
        (
            "=ODDFPRICE(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0785,0.0625,100,1,1)",
            n(108.23814271186883),
        ),
        (
            "=ODDFPRICE(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0785,0.0625,100,1,2)",
            n(108.00708897878675),
        ),
        (
            "=ODDFPRICE(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0785,0.0625,100,1,3)",
            n(108.21843047239881),
        ),
        (
            "=ODDFPRICE(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0785,0.0625,100,1,4)",
            n(108.23858198575189),
        ),
        (
            "=ODDFPRICE(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0785,0.0625,100,2,0)",
            n(103.74070877101448),
        ),
        (
            "=ODDFPRICE(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0785,0.0625,100,2,1)",
            n(103.76114763634918),
        ),
        (
            "=ODDFPRICE(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0785,0.0625,100,2,2)",
            n(103.76229865273831),
        ),
        (
            "=ODDFPRICE(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0785,0.0625,100,2,3)",
            n(103.76086356841934),
        ),
        (
            "=ODDFPRICE(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0785,0.0625,100,2,4)",
            n(103.76251432657004),
        ),
        (
            "=ODDFPRICE(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0785,0.0625,100,2,0)",
            n(103.72457364586558),
        ),
        (
            "=ODDFPRICE(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0785,0.0625,100,2,1)",
            n(103.71089834042685),
        ),
        (
            "=ODDFPRICE(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0785,0.0625,100,2,2)",
            n(103.708265396265),
        ),
        (
            "=ODDFPRICE(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0785,0.0625,100,2,3)",
            n(103.7075433805016),
        ),
        (
            "=ODDFPRICE(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0785,0.0625,100,2,4)",
            n(103.72457364586558),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0,0.0625,100,2,1)",
            n(46.896796581656126),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),-0.01,0.0625,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0,100,2,1)",
            n(196.58535911602206),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,-0.01,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,0,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,3,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2)",
            n(113.59920582823824),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2.9,1.9)",
            n(113.59771747407883),
        ),
        (
            "=ODDFPRICE(DATE(2008,10,15),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2009,3,1),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2009,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,10,1),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,4,10),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)",
            n(113.59771747407883),
        ),
    ]);
}

#[test]
fn oddfyield() {
    assert_cases(&[
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0,84.5,100,2,0)",
            n(0.013733327886369813),
        ),
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),-0.01,84.5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,0,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,10,15),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2009,3,1),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2009,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,10,1),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDFYIELD(DATE(2008,11,11),DATE(2021,4,10),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn oddlprice() {
    assert_cases(&[
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,0)",
            n(99.87828601472134),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,1)",
            n(99.87916768152911),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,2)",
            n(99.87916768152911),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,3)",
            n(99.87916768152911),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,4)",
            n(99.87828601472134),
        ),
        (
            "=ODDLPRICE(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,0.0405,100,2,0)",
            n(99.94701793072034),
        ),
        (
            "=ODDLPRICE(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,0.0405,100,2,1)",
            n(99.94693525046888),
        ),
        (
            "=ODDLPRICE(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,0.0405,100,2,2)",
            n(99.94693525046888),
        ),
        (
            "=ODDLPRICE(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,0.0405,100,2,3)",
            n(99.94693525046888),
        ),
        (
            "=ODDLPRICE(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,0.0405,100,2,4)",
            n(99.94701793072034),
        ),
        (
            "=ODDLPRICE(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,0.0405,100,2,0)",
            n(99.82888858109919),
        ),
        (
            "=ODDLPRICE(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,0.0405,100,2,1)",
            n(99.81778205275167),
        ),
        (
            "=ODDLPRICE(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,0.0405,100,2,2)",
            n(99.81778205275167),
        ),
        (
            "=ODDLPRICE(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,0.0405,100,2,3)",
            n(99.81778205275167),
        ),
        (
            "=ODDLPRICE(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,0.0405,100,2,4)",
            n(99.81752668000746),
        ),
        (
            "=ODDLPRICE(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,0.0405,100,4,0)",
            n(99.84610900981438),
        ),
        (
            "=ODDLPRICE(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,0.0405,100,4,1)",
            n(99.84553078613476),
        ),
        (
            "=ODDLPRICE(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,0.0405,100,4,2)",
            n(99.84553078613476),
        ),
        (
            "=ODDLPRICE(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,0.0405,100,4,3)",
            n(99.84553078613476),
        ),
        (
            "=ODDLPRICE(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,0.0405,100,4,4)",
            n(99.84664036510951),
        ),
        (
            "=ODDLPRICE(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,0.0405,100,1,0)",
            n(99.553614496065),
        ),
        (
            "=ODDLPRICE(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,0.0405,100,1,1)",
            n(99.55380806621977),
        ),
        (
            "=ODDLPRICE(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,0.0405,100,1,2)",
            n(99.55380806621977),
        ),
        (
            "=ODDLPRICE(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,0.0405,100,1,3)",
            n(99.55380806621977),
        ),
        (
            "=ODDLPRICE(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,0.0405,100,1,4)",
            n(99.553614496065),
        ),
        (
            "=ODDLPRICE(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,0.0405,100,2,0)",
            n(99.86518264924318),
        ),
        (
            "=ODDLPRICE(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,0.0405,100,2,1)",
            n(99.87596624605266),
        ),
        (
            "=ODDLPRICE(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,0.0405,100,2,2)",
            n(99.87596624605266),
        ),
        (
            "=ODDLPRICE(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,0.0405,100,2,3)",
            n(99.87596624605266),
        ),
        (
            "=ODDLPRICE(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,0.0405,100,2,4)",
            n(99.87736517152071),
        ),
        (
            "=ODDLPRICE(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,0.0405,100,2,0)",
            n(99.63258278731894),
        ),
        (
            "=ODDLPRICE(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,0.0405,100,2,1)",
            n(99.6214512670231),
        ),
        (
            "=ODDLPRICE(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,0.0405,100,2,2)",
            n(99.6214512670231),
        ),
        (
            "=ODDLPRICE(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,0.0405,100,2,3)",
            n(99.6214512670231),
        ),
        (
            "=ODDLPRICE(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,0.0405,100,2,4)",
            n(99.62217045601449),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,0)",
            n(99.87828601472134),
        ),
        (
            "=ODDLPRICE(DATE(2007,10,15),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2007,10,1),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,6,15),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2008,7,1),0.0375,0.0405,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0,0.0405,100,2,0)",
            n(98.58044164037855),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),-0.01,0.0405,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0,100,2,0)",
            n(101.33333333333333),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,-0.01,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,3,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2)",
            n(99.87828601472134),
        ),
        (
            "=SUM(ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,{0.0405,0.05},100,2,0))",
            n(199.42122633495424),
        ),
    ]);
}

#[test]
fn oddlyield() {
    assert_cases(&[
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,0)",
            n(0.04059278350515451),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,1)",
            n(0.040618683689051124),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,2)",
            n(0.040618683689051124),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,3)",
            n(0.040618683689051124),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,4)",
            n(0.04059278350515451),
        ),
        (
            "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,0)",
            n(0.04519223562916916),
        ),
        (
            "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,1)",
            n(0.04517988549187201),
        ),
        (
            "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,2)",
            n(0.04517988549187201),
        ),
        (
            "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,3)",
            n(0.04517988549187201),
        ),
        (
            "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,4)",
            n(0.04519223562916916),
        ),
        (
            "=ODDLYIELD(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,99.875,100,2,0)",
            n(0.03969142978124684),
        ),
        (
            "=ODDLYIELD(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,99.875,100,2,1)",
            n(0.039502395557476415),
        ),
        (
            "=ODDLYIELD(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,99.875,100,2,2)",
            n(0.039502395557476415),
        ),
        (
            "=ODDLYIELD(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,99.875,100,2,3)",
            n(0.039502395557476415),
        ),
        (
            "=ODDLYIELD(DATE(2023,10,15),DATE(2024,5,15),DATE(2023,8,31),0.0375,99.875,100,2,4)",
            n(0.03949843524444237),
        ),
        (
            "=ODDLYIELD(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,99.875,100,4,0)",
            n(0.039928668357354806),
        ),
        (
            "=ODDLYIELD(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,99.875,100,4,1)",
            n(0.03991949885696179),
        ),
        (
            "=ODDLYIELD(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,99.875,100,4,2)",
            n(0.03991949885696179),
        ),
        (
            "=ODDLYIELD(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,99.875,100,4,3)",
            n(0.03991949885696179),
        ),
        (
            "=ODDLYIELD(DATE(2024,3,10),DATE(2024,9,15),DATE(2024,2,29),0.0375,99.875,100,4,4)",
            n(0.03993659249415076),
        ),
        (
            "=ODDLYIELD(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,99.875,100,1,0)",
            n(0.037834478724959826),
        ),
        (
            "=ODDLYIELD(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,99.875,100,1,1)",
            n(0.03783235082760308),
        ),
        (
            "=ODDLYIELD(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,99.875,100,1,2)",
            n(0.03783235082760308),
        ),
        (
            "=ODDLYIELD(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,99.875,100,1,3)",
            n(0.03783235082760308),
        ),
        (
            "=ODDLYIELD(DATE(2024,1,2),DATE(2025,3,31),DATE(2023,6,30),0.0375,99.875,100,1,4)",
            n(0.037834478724959826),
        ),
        (
            "=ODDLYIELD(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,99.875,100,2,0)",
            n(0.040262753948222735),
        ),
        (
            "=ODDLYIELD(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,99.875,100,2,1)",
            n(0.04052343010269838),
        ),
        (
            "=ODDLYIELD(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,99.875,100,2,2)",
            n(0.04052343010269838),
        ),
        (
            "=ODDLYIELD(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,99.875,100,2,3)",
            n(0.04052343010269838),
        ),
        (
            "=ODDLYIELD(DATE(2023,3,1),DATE(2023,7,31),DATE(2023,2,28),0.0375,99.875,100,2,4)",
            n(0.04055815750084484),
        ),
        (
            "=ODDLYIELD(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,99.875,100,2,0)",
            n(0.03834639280313011),
        ),
        (
            "=ODDLYIELD(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,99.875,100,2,1)",
            n(0.03825327989217842),
        ),
        (
            "=ODDLYIELD(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,99.875,100,2,2)",
            n(0.03825327989217842),
        ),
        (
            "=ODDLYIELD(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,99.875,100,2,3)",
            n(0.03825327989217842),
        ),
        (
            "=ODDLYIELD(DATE(2022,11,30),DATE(2024,1,31),DATE(2022,8,31),0.0375,99.875,100,2,4)",
            n(0.03825406826193261),
        ),
        (
            "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,0)",
            n(0.04519223562916916),
        ),
        (
            "=ODDLYIELD(DATE(2007,10,15),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2007,10,1),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2008,6,15),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2008,7,1),0.0375,99.875,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0,99.875,100,2,0)",
            n(0.003520025031289112),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),-0.01,99.875,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0,100,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,0,2,0)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,150,100,2,0)",
            n(-0.9054575523704521),
        ),
        (
            "=ODDLYIELD(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,99.875,100,3,0)",
            error(ExcelErrorKind::Num),
        ),
    ]);
}

#[test]
fn vdb() {
    assert_cases(&[
        ("=VDB(2400,300,10,0,1,2,FALSE)", n(480.0)),
        ("=VDB(2400,300,10,0,1,2,TRUE)", n(480.0)),
        ("=VDB(2400,300,10,0,1,1.5,FALSE)", n(360.0)),
        ("=VDB(2400,300,10,0,1,1.5,TRUE)", n(360.0)),
        ("=VDB(2400,300,10,0,10,2,FALSE)", n(2100.0)),
        ("=VDB(2400,300,10,0,10,2,TRUE)", n(2100.000000000001)),
        ("=VDB(2400,300,10,0,10,1.5,FALSE)", n(2100.0)),
        ("=VDB(2400,300,10,0,10,1.5,TRUE)", n(1927.5014295822652)),
        ("=VDB(2400,300,10,1,2,2,FALSE)", n(384.0)),
        ("=VDB(2400,300,10,1,2,2,TRUE)", n(384.0)),
        ("=VDB(2400,300,10,1,2,1.5,FALSE)", n(306.0)),
        ("=VDB(2400,300,10,1,2,1.5,TRUE)", n(306.0)),
        ("=VDB(2400,300,10,5,6,2,FALSE)", n(157.28640000000001)),
        ("=VDB(2400,300,10,5,6,2,TRUE)", n(157.2864000000001)),
        ("=VDB(2400,300,10,5,6,1.5,FALSE)", n(159.7339125)),
        ("=VDB(2400,300,10,5,6,1.5,TRUE)", n(159.73391249999995)),
        ("=VDB(2400,300,10,6,7,2,FALSE)", n(125.82912000000002)),
        ("=VDB(2400,300,10,6,7,2,TRUE)", n(125.82912000000009)),
        ("=VDB(2400,300,10,6,7,1.5,FALSE)", n(151.289709375)),
        ("=VDB(2400,300,10,6,7,1.5,TRUE)", n(135.77382562499994)),
        ("=VDB(2400,300,10,7,8,2,FALSE)", n(100.66329600000002)),
        ("=VDB(2400,300,10,7,8,2,TRUE)", n(100.66329600000007)),
        ("=VDB(2400,300,10,7,8,1.5,FALSE)", n(151.289709375)),
        ("=VDB(2400,300,10,7,8,1.5,TRUE)", n(115.40775178124994)),
        ("=VDB(2400,300,10,9,10,2,FALSE)", n(22.122547200000042)),
        ("=VDB(2400,300,10,9,10,2,TRUE)", n(22.12254720000027)),
        ("=VDB(2400,300,10,9,10,1.5,FALSE)", n(151.289709375)),
        ("=VDB(2400,300,10,9,10,1.5,TRUE)", n(83.38210066195309)),
        ("=VDB(2400,300,10,0,0.5,2,FALSE)", n(240.0)),
        ("=VDB(2400,300,10,0,0.5,2,TRUE)", n(240.0)),
        ("=VDB(2400,300,10,0,0.5,1.5,FALSE)", n(180.0)),
        ("=VDB(2400,300,10,0,0.5,1.5,TRUE)", n(180.0)),
        ("=VDB(2400,300,10,0.5,1,2,FALSE)", n(216.0)),
        ("=VDB(2400,300,10,0.5,1,2,TRUE)", n(240.0)),
        ("=VDB(2400,300,10,0.5,1,1.5,FALSE)", n(166.5)),
        ("=VDB(2400,300,10,0.5,1,1.5,TRUE)", n(180.0)),
        ("=VDB(2400,300,10,0.5,1.5,2,FALSE)", n(432.0)),
        ("=VDB(2400,300,10,0.5,1.5,2,TRUE)", n(432.0)),
        ("=VDB(2400,300,10,0.5,1.5,1.5,FALSE)", n(333.0)),
        ("=VDB(2400,300,10,0.5,1.5,1.5,TRUE)", n(333.0)),
        ("=VDB(2400,300,10,2.3,3.7,2,FALSE)", n(381.1737600000001)),
        ("=VDB(2400,300,10,2.3,3.7,2,TRUE)", n(387.0720000000001)),
        ("=VDB(2400,300,10,2.3,3.7,1.5,FALSE)", n(332.8499700000001)),
        ("=VDB(2400,300,10,2.3,3.7,1.5,TRUE)", n(336.82950000000005)),
        ("=VDB(2400,300,10,4.5,9.25,2,FALSE)", n(569.1390336000001)),
        ("=VDB(2400,300,10,4.5,9.25,2,TRUE)", n(568.1440896000005)),
        ("=VDB(2400,300,10,4.5,9.25,1.5,TRUE)", n(623.8187290858006)),
        ("=VDB(2400,300,10,9.5,10,2,TRUE)", n(11.061273600000135)),
        ("=VDB(2400,300,10,9.5,10,1.5,TRUE)", n(41.691050330976545)),
        ("=VDB(2400,300,10,0,9.99,2,FALSE)", n(2099.7787745279998)),
        ("=VDB(2400,300,10,0,9.99,2,TRUE)", n(2099.778774528001)),
        ("=VDB(2400,300,10,0,9.99,1.5,FALSE)", n(2098.48710290625)),
        ("=VDB(2400,300,10,0,9.99,1.5,TRUE)", n(1926.6676085756458)),
        ("=VDB(2400,300,10,3,3,2,FALSE)", n(0.0)),
        ("=VDB(2400,300,10,3,3,2,TRUE)", n(0.0)),
        ("=VDB(2400,300,10,3,3,1.5,FALSE)", n(0.0)),
        ("=VDB(2400,300,10,3,3,1.5,TRUE)", n(0.0)),
        ("=VDB(2400,300,10,2.5,2.5,2,FALSE)", n(0.0)),
        ("=VDB(2400,300,10,2.5,2.5,2,TRUE)", n(0.0)),
        ("=VDB(2400,300,10,2.5,2.5,1.5,FALSE)", n(0.0)),
        ("=VDB(2400,300,10,2.5,2.5,1.5,TRUE)", n(0.0)),
        ("=VDB(2400,300,10*365,0,1)", n(1.3150684931506849)),
        ("=VDB(2400,300,10*12,0,1)", n(40.0)),
        ("=VDB(2400,300,10,0,1)", n(480.0)),
        ("=VDB(2400,300,10*12,6,18)", n(396.30605326475086)),
        ("=VDB(2400,300,10*12,6,18,1.5)", n(311.80893665823413)),
        ("=VDB(2400,300,10,0,0.875,1.5)", n(315.0)),
        ("=VDB(1000,100,2,0,2,2)", n(900.0)),
        ("=VDB(1000,100,2,0,2,3)", n(900.0)),
        ("=VDB(1000,100,2,1,2,3)", n(0.0)),
        ("=VDB(1000,100,2,0.5,1.5,2)", n(450.0)),
        ("=VDB(2400,0,10,0,10)", n(2400.0)),
        ("=VDB(2400,2400,10,0,5)", n(0.0)),
        ("=VDB(2400,2500,10,0,5)", n(-100.0)),
        ("=VDB(2400,300,10,5,4)", error(ExcelErrorKind::Num)),
        ("=VDB(2400,300,10,5,11)", error(ExcelErrorKind::Num)),
        ("=VDB(2400,300,10,-1,4)", error(ExcelErrorKind::Num)),
        ("=VDB(0,0,10,0,5)", n(0.0)),
        ("=VDB(-2400,300,10,0,5)", error(ExcelErrorKind::Num)),
        ("=VDB(2400,300,5.5,0,5.5)", n(2100.0)),
        ("=VDB(2400,300,5.5,5,5.5)", n(0.0)),
        ("=VDB(2400,300,0,0,0)", error(ExcelErrorKind::Div)),
        ("=VDB(2400,300,-5,0,1)", error(ExcelErrorKind::Num)),
        ("=VDB(2400,300,10,0,1,0)", n(210.0)),
        ("=VDB(2400,300,10,0,1,-1)", error(ExcelErrorKind::Num)),
        ("=VDB(2400,300,10,0,5,2,1)", n(1613.5680000000002)),
        ("=VDB(2400,300,10,0,5,2,0)", n(1613.568)),
        (
            "=VDB(2400,300,10,0,5,2,\"x\")",
            error(ExcelErrorKind::Value),
        ),
        ("=VDB(2400,300,10,0,5,2,)", n(1613.568)),
        ("=VDB(2400,300,10,0,5,,TRUE)", n(0.0)),
        ("=VDB(2400,300,10,0.3,7.6,2,TRUE)", n(1813.0814976000006)),
        ("=VDB(2400,300,10,0.3,7.6,2,FALSE)", n(1811.269558272)),
        ("=VDB(2400,300,10,0,10,0.5)", n(2100.0)),
        ("=VDB(2400,300,10,0,10,0.5,TRUE)", n(963.0313458278906)),
        ("=VDB(2400,2000,10,0,10)", n(400.0)),
        ("=VDB(2400,2000,10,1,2)", n(0.0)),
        ("=VDB(10000,1000,5,0,5,1.2)", n(9000.0)),
        ("=VDB(1000,100,1,0,1)", n(900.0)),
        ("=VDB(1000,100,0.5,0,0.5)", n(900.0)),
        ("=VDB(2400,300,10,9,10,2)", n(22.122547200000042)),
        ("=VDB(2400,300,10,8.5,10,2)", n(62.3878656)),
        ("=VDB(2400,300,10,8,9.5,2)", n(91.59191040000005)),
        ("=VDB(2400,300,10,9,9.5,2)", n(11.061273600000021)),
        ("=VDB(2400,300,10,9,9.75,2)", n(16.59191040000003)),
        ("=VDB(2400,300,10,7.5,8.5,2)", n(90.5969664)),
        ("=VDB(2400,300,10,9,10,1.5)", n(151.289709375)),
        ("=VDB(2400,300,10,8,9.5,1.5)", n(226.9345640625)),
        ("=VDB(2400,300,10,9,9.5,1.5)", n(75.6448546875)),
        ("=VDB(2400,300,10,9,9.75,1.5)", n(113.46728203125)),
        ("=VDB(2400,2500,10,0,1,2,FALSE)", n(-100.0)),
        ("=VDB(2400,2500,10,0,1,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,0,2,2,FALSE)", n(-100.0)),
        ("=VDB(2400,2500,10,0,2,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,1,2,2,FALSE)", n(0.0)),
        ("=VDB(2400,2500,10,1,2,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,0,10,2,FALSE)", n(-100.0)),
        ("=VDB(2400,2500,10,0,10,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,2,3,2,FALSE)", n(0.0)),
        ("=VDB(2400,2500,10,2,3,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,0,0.5,2,FALSE)", n(-50.0)),
        ("=VDB(2400,2500,10,0,0.5,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,0.5,1,2,FALSE)", n(-50.0)),
        ("=VDB(2400,2500,10,0.5,1,2,TRUE)", n(0.0)),
        ("=VDB(2400,2500,10,4,5,2,FALSE)", n(0.0)),
        ("=VDB(2400,2500,10,4,5,2,TRUE)", n(0.0)),
        ("=VDB(1000,3000,5,0,5)", n(-2000.0)),
        ("=VDB(1000,3000,5,1,3,1.5)", n(0.0)),
        ("=SUM(VDB(2400,300,10,{0,1},{1,2}))", n(864.0)),
        ("=VDB(2400,300,10,TRUE,2)", n(384.0)),
    ]);
}

#[test]
fn ddb() {
    assert_cases(&[
        ("=DDB(2400,300,10,1.5)", n(429.32505167995964)),
        ("=DDB(2400,300,10,0.5)", n(480.0)),
        ("=DDB(2400,300,10,9.5)", n(60.1439563122924)),
        ("=DDB(2400,300,10,10.5)", error(ExcelErrorKind::Num)),
        ("=DDB(2400,300,10,2.5,1.5)", n(282.1180603931623)),
        ("=DDB(1000,100,2,1,3)", n(900.0)),
        ("=DDB(1000,100,2,2,3)", n(0.0)),
        ("=DDB(1000,100,2,1.5,3)", n(0.0)),
        ("=DDB(2400,300,10,10)", n(22.12254720000027)),
        ("=DDB(2400,300,10,1)", n(480.0)),
        ("=DDB(2400,2300,10,2)", n(0.0)),
        ("=DDB(2400,300,5.5,5.5)", n(13.967965339602642)),
        ("=DDB(2400,300,10,0)", error(ExcelErrorKind::Num)),
        ("=DDB(2400,300,10,3,1)", n(194.40000000000003)),
        ("=DDB(2400,300,7.5,2.25,2)", n(434.316927794804)),
        ("=DDB(10000,1000,5,0.25)", n(4000.0)),
        ("=DDB(10000,1000,5,0.75)", n(4000.0)),
        ("=DDB(10000,1000,5,0.999)", n(4000.0)),
        ("=DDB(10000,1000,5,1.9)", n(2525.7834699574214)),
        ("=DDB(10000,1000,5,2.5)", n(1859.03200617956)),
        ("=DDB(10000,1000,5,5.9)", error(ExcelErrorKind::Num)),
        ("=DDB(10000,1000,5,4.5)", n(669.2515222246416)),
        ("=DDB(10000,1000,5,0.5)", n(4000.0)),
        ("=DDB(10000,1000,5,0.5,1.5)", n(3000.0)),
        ("=DDB(1000,900,5,0.5)", n(100.0)),
        ("=DDB(2400,300,10,TRUE)", n(480.0)),
    ]);
}

#[test]
fn amordegrc() {
    assert_cases(&[
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.9,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.9,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.6,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.6,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.5,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.5,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.45,1)",
            n(593.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.45,1)",
            n(904.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.4,1)",
            n(527.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.4,1)",
            n(937.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.34,1)",
            n(448.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.34,1)",
            n(976.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,(1/3),1)",
            n(439.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,(1/3),1)",
            n(981.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.3,1)",
            n(395.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.3,1)",
            n(902.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.26,1)",
            n(343.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.26,1)",
            n(802.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.25,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.25,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.24,1)",
            n(422.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.24,1)",
            n(949.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.22,1)",
            n(387.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.22,1)",
            n(886.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.2,1)",
            n(351.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.2,1)",
            n(820.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.19,1)",
            n(334.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.19,1)",
            n(785.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.18,1)",
            n(316.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.18,1)",
            n(750.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,(1/6),1)",
            n(293.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,(1/6),1)",
            n(702.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.166,1)",
            n(365.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.166,1)",
            n(845.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.16,1)",
            n(351.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.16,1)",
            n(820.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.1,1)",
            n(220.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.1,1)",
            n(545.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,1,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,1,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,1.5,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,1.5,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,2,0.15,1)",
            n(485.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.15,1)",
            n(303.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,4,0.15,1)",
            n(190.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,5,0.15,1)",
            n(158.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,6,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,7,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,8,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,9,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,10,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,0,0.25,0)",
            n(2979.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,1,0.25,0)",
            n(2633.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,2,0.25,0)",
            n(2194.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,3,0.25,0)",
            n(2194.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,4,0.25,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,5,0.25,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,6,0.25,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,0)",
            n(753.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,0)",
            n(618.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,0)",
            n(386.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,0)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,1)",
            n(755.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,1)",
            n(617.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,1)",
            n(386.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,3)",
            n(757.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,3)",
            n(616.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,3)",
            n(385.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,3)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,4)",
            n(753.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,4)",
            n(618.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,4)",
            n(386.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,4)",
            n(328.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,12,31),DATE(2008,12,31),300,1,0.15,1)",
            n(563.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,12,31),DATE(2008,8,19),300,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),3000,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),2400,1,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),0,3,0.15,1)",
            n(303.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1.7,0.15,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,-1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,-0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(0,DATE(2008,8,19),DATE(2008,12,31),0,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(-2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,1,10),DATE(2009,6,30),300,0,0.15,1)",
            n(1320.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,1,10),DATE(2009,6,30),300,1,0.15,1)",
            n(405.0),
        ),
        (
            "=AMORDEGRC(1234567.89,DATE(2008,8,19),DATE(2008,12,31),1000.5,2,0.123,0)",
            n(233252.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),-300,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,0,0.15,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),900,0,0.25,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,1,0.15,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),900,1,0.25,1)",
            n(776.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,2,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),900,2,0.25,1)",
            n(647.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,3,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),900,3,0.25,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,4,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),900,4,0.25,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,5,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),900,5,0.25,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.4999,1)",
            n(659.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.4999,1)",
            n(871.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.4999,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.5001,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.5001,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.5001,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.49,1)",
            n(646.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.49,1)",
            n(877.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.49,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.2499,1)",
            n(439.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.2499,1)",
            n(980.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.2499,1)",
            n(245.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.2501,1)",
            n(330.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.2501,1)",
            n(777.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.2501,1)",
            n(647.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.1666,1)",
            n(366.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.1666,1)",
            n(847.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.1666,1)",
            n(288.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.1667,1)",
            n(293.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.1667,1)",
            n(702.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.1667,1)",
            n(312.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.3334,1)",
            n(439.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.3334,1)",
            n(981.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.3334,1)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.3333,1)",
            n(439.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.3333,1)",
            n(980.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.3333,1)",
            n(490.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,12,31),DATE(2008,12,31),300,0,0.15,1)",
            n(900.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,12,31),DATE(2008,12,31),300,1,0.15,1)",
            n(563.0),
        ),
        (
            "=AMORDEGRC(2400,DATE(2008,12,31),DATE(2008,12,31),300,2,0.15,1)",
            n(352.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,0,0.3,0)",
            n(3575.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,0,0.1,0)",
            n(1986.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,1,0.3,0)",
            n(2891.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,1,0.1,0)",
            n(2004.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,2,0.3,0)",
            n(1767.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,2,0.1,0)",
            n(1503.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,3,0.3,0)",
            n(1767.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,3,0.1,0)",
            n(1127.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,4,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,4,0.1,0)",
            n(845.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,5,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,5,0.1,0)",
            n(634.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,6,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,6,0.1,0)",
            n(475.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,7,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,7,0.1,0)",
            n(357.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,8,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORDEGRC(10000,DATE(2023,3,15),DATE(2023,12,31),500,8,0.1,0)",
            n(535.0),
        ),
    ]);
}

#[test]
fn amorlinc() {
    assert_cases(&[
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,1)",
            n(131.8032786885246),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,2,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,3,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,4,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,5,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,6,0.15,1)",
            n(168.19672131147536),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,7,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,8,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,9,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,10,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,0,0.3,0)",
            n(2383.333333333333),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,1,0.3,0)",
            n(3000.0),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,2,0.3,0)",
            n(3000.0),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,3,0.3,0)",
            n(1116.666666666667),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,4,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,5,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORLINC(10000,DATE(2023,3,15),DATE(2023,12,31),500,6,0.3,0)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,0)",
            n(301.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,0)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,0)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,0)",
            n(131.99999999999997),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,1)",
            n(301.9672131147541),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,1)",
            n(131.8032786885246),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,3)",
            n(302.7945205479452),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,3)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,3)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,3)",
            n(132.16438356164383),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,0,0.15,4)",
            n(301.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,1,0.15,4)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2024,2,29),DATE(2024,12,31),300,2,0.15,4)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,0,0.15,4)",
            n(131.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,12,31),DATE(2008,12,31),300,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,12,31),DATE(2008,8,19),300,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),3000,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),2400,1,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),0,3,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1.7,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,-1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,-0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(0,DATE(2008,8,19),DATE(2008,12,31),0,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(-2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,5)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,2)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,1,10),DATE(2009,6,30),300,0,0.15,1)",
            n(528.1967213114754),
        ),
        (
            "=AMORLINC(2400,DATE(2008,1,10),DATE(2009,6,30),300,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(1234567.89,DATE(2008,8,19),DATE(2008,12,31),1000.5,2,0.123,0)",
            n(151851.85046999998),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),-300,1,0.15,1)",
            error(ExcelErrorKind::Num),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,0,0.15,1)",
            n(131.8032786885246),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,2,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,3,0.15,1)",
            n(48.19672131147536),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,4,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),1500,5,0.15,1)",
            n(0.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,12,31),DATE(2008,12,31),300,0,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,12,31),DATE(2008,12,31),300,1,0.15,1)",
            n(360.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,12,31),DATE(2008,12,31),300,2,0.15,1)",
            n(360.0),
        ),
        (
            "=SUM(AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,{1,2},0.15,1))",
            n(720.0),
        ),
        (
            "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,TRUE,0.15,1)",
            error(ExcelErrorKind::Value),
        ),
    ]);
}

#[test]
fn fvschedule() {
    assert_cases(&[
        ("=FVSCHEDULE(1,{0.09,0.11,0.1})", n(1.3308900000000004)),
        ("=FVSCHEDULE(1000,{-0.5,0.2,-1})", n(0.0)),
        ("=FVSCHEDULE(1000,0.05)", n(1050.0)),
        ("=FVSCHEDULE(1000,{0.1;0.2})", n(1320.0)),
        (
            "=FVSCHEDULE(1000,{0.1,\"a\"})",
            error(ExcelErrorKind::Value),
        ),
        ("=FVSCHEDULE(1000,{0.1,TRUE})", error(ExcelErrorKind::Value)),
        ("=FVSCHEDULE(1000,\"0.1\")", n(1100.0)),
        ("=FVSCHEDULE(1000,TRUE)", error(ExcelErrorKind::Value)),
        ("=FVSCHEDULE(1000,{0.1,#N/A})", error(ExcelErrorKind::Na)),
        ("=FVSCHEDULE(\"1000\",{0.1})", n(1100.0)),
        ("=FVSCHEDULE(\"x\",{0.1})", error(ExcelErrorKind::Value)),
        ("=FVSCHEDULE(1000,A1:A3)", n(1000.0)),
        ("=FVSCHEDULE(A1,{0.1})", n(0.0)),
        ("=FVSCHEDULE(-1000,{0.1,0.2})", n(-1320.0)),
        ("=FVSCHEDULE(1E300,{1E10,1E10})", n(0.0)),
        ("=SUM(FVSCHEDULE({1000;2000},{0.1,0.2}))", n(3960.0)),
        ("=FVSCHEDULE(TRUE,{0.1})", error(ExcelErrorKind::Value)),
    ]);
}

/// Excel stops its iteration short of the root: 1e-9 relative or 1e-10
/// absolute, the corpus comparison's tolerance.
#[test]
fn yield_rows_iterated() {
    assert_within(
        &[
            (
                "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,1,0)",
                n(0.06505568014794991),
            ),
            (
                "=YIELD(DATE(2024,2,15),DATE(2032,11,15),0.0575,95.04287,100,1,1)",
                n(0.06505639669310567),
            ),
            (
                "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,4,0)",
                n(0.1627329691583733),
            ),
            (
                "=YIELD(DATE(2024,2,29),DATE(2024,8,31),0.0575,95.04287,100,4,1)",
                n(0.1627329691583667),
            ),
            (
                "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,1,0)",
                n(0.08437260169609731),
            ),
            (
                "=YIELD(DATE(2024,1,31),DATE(2026,2,28),0.0575,95.04287,100,1,1)",
                n(0.0843903387490036),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,1,0)",
                n(0.061776976919896275),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,1,1)",
                n(0.061776264565193764),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,2,0)",
                n(0.06174436196588508),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,2,1)",
                n(0.061743870140256875),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,2,2)",
                n(0.061745885450841025),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,2,3)",
                n(0.0617446108248952),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,2,4)",
                n(0.06174486724301965),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,4,0)",
                n(0.06172799139021122),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,95.04287,100,4,1)",
                n(0.061727608140042196),
            ),
            (
                "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0,95,100,2,0)",
                n(0.005267775907907244),
            ),
            (
                "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,50,100,2,0)",
                n(0.16063907715482847),
            ),
            (
                "=YIELD(DATE(2008,2,15),DATE(2017,11,15),0.0575,200,100,2,0)",
                n(-0.03001618818878727),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,92,100,2,0)",
                n(0.0645080393507521),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,96,100,2,0)",
                n(0.06090086489453154),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,104,100,2,0)",
                n(0.05428445438499384),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,108,100,2,0)",
                n(0.05123619405807132),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,112,100,2,0)",
                n(0.04833962135034755),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,116,100,2,0)",
                n(0.04558116289963718),
            ),
            (
                "=YIELD(DATE(2024,8,30),DATE(2045,8,31),0.0575,120,100,2,0)",
                n(0.042948940060094684),
            ),
        ],
        1e-9,
        1e-10,
    );
}

/// Excel stops its iteration short of the root: 1e-9 relative or 1e-10
/// absolute, the corpus comparison's tolerance.
#[test]
fn oddfyield_iterated() {
    assert_within(
        &[
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
                n(0.07724554159729888),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,1)",
                n(0.07724706259741908),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,2)",
                n(0.07724502192067954),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,3)",
                n(0.0772500780535923),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,4)",
                n(0.07724554159729888),
            ),
            (
                "=ODDFYIELD(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0575,84.5,100,2,0)",
                n(0.08016194662313164),
            ),
            (
                "=ODDFYIELD(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0575,84.5,100,2,1)",
                n(0.08013053181826389),
            ),
            (
                "=ODDFYIELD(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0575,84.5,100,2,2)",
                n(0.0800524964572335),
            ),
            (
                "=ODDFYIELD(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0575,84.5,100,2,3)",
                n(0.08014853727544936),
            ),
            (
                "=ODDFYIELD(DATE(2023,9,1),DATE(2030,8,31),DATE(2023,5,10),DATE(2024,2,29),0.0575,84.5,100,2,4)",
                n(0.08016194662312044),
            ),
            (
                "=ODDFYIELD(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0575,84.5,100,2,0)",
                n(0.09829232537216014),
            ),
            (
                "=ODDFYIELD(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0575,84.5,100,2,1)",
                n(0.09830055950217056),
            ),
            (
                "=ODDFYIELD(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0575,84.5,100,2,2)",
                n(0.09799063481909753),
            ),
            (
                "=ODDFYIELD(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0575,84.5,100,2,3)",
                n(0.09826242308438596),
            ),
            (
                "=ODDFYIELD(DATE(2023,11,20),DATE(2028,9,15),DATE(2023,1,10),DATE(2024,3,15),0.0575,84.5,100,2,4)",
                n(0.09829232537216014),
            ),
            (
                "=ODDFYIELD(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0575,84.5,100,4,0)",
                n(0.10094715551303508),
            ),
            (
                "=ODDFYIELD(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0575,84.5,100,4,1)",
                n(0.10090116748180164),
            ),
            (
                "=ODDFYIELD(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0575,84.5,100,4,2)",
                n(0.10089331495815154),
            ),
            (
                "=ODDFYIELD(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0575,84.5,100,4,3)",
                n(0.10098933136255027),
            ),
            (
                "=ODDFYIELD(DATE(2024,2,10),DATE(2027,10,31),DATE(2024,1,5),DATE(2024,4,30),0.0575,84.5,100,4,4)",
                n(0.10094715551299864),
            ),
            (
                "=ODDFYIELD(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0575,84.5,100,1,0)",
                n(0.0867628485749121),
            ),
            (
                "=ODDFYIELD(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0575,84.5,100,1,1)",
                n(0.08676190948833185),
            ),
            (
                "=ODDFYIELD(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0575,84.5,100,1,2)",
                n(0.08631239550038285),
            ),
            (
                "=ODDFYIELD(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0575,84.5,100,1,3)",
                n(0.08671675963286003),
            ),
            (
                "=ODDFYIELD(DATE(2023,6,30),DATE(2030,6,30),DATE(2022,3,1),DATE(2024,6,30),0.0575,84.5,100,1,4)",
                n(0.0867628485749121),
            ),
            (
                "=ODDFYIELD(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0575,84.5,100,2,0)",
                n(0.13007061593928543),
            ),
            (
                "=ODDFYIELD(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0575,84.5,100,2,1)",
                n(0.1301759070067441),
            ),
            (
                "=ODDFYIELD(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0575,84.5,100,2,2)",
                n(0.13015214394266544),
            ),
            (
                "=ODDFYIELD(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0575,84.5,100,2,3)",
                n(0.1301817670466403),
            ),
            (
                "=ODDFYIELD(DATE(2024,1,31),DATE(2026,8,31),DATE(2023,12,15),DATE(2024,2,29),0.0575,84.5,100,2,4)",
                n(0.1301538259841203),
            ),
            (
                "=ODDFYIELD(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0575,84.5,100,2,0)",
                n(0.1298643140211796),
            ),
            (
                "=ODDFYIELD(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0575,84.5,100,2,1)",
                n(0.1297612432760098),
            ),
            (
                "=ODDFYIELD(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0575,84.5,100,2,2)",
                n(0.12972188395650938),
            ),
            (
                "=ODDFYIELD(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0575,84.5,100,2,3)",
                n(0.12975688410570904),
            ),
            (
                "=ODDFYIELD(DATE(2023,3,1),DATE(2025,9,30),DATE(2023,1,31),DATE(2023,9,30),0.0575,84.5,100,2,4)",
                n(0.1298643140211796),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,150,100,2,0)",
                n(0.013314247608937969),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,20,100,2,0)",
                n(0.3213721898793946),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2)",
                n(0.07724554159729888),
            ),
            (
                "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)",
                n(0.07724554159729888),
            ),
        ],
        1e-9,
        1e-10,
    );
}

#[test]
fn schedules_from_cells() {
    let cases: &[(&str, &[(&str, LiteralValue)], LiteralValue)] = &[
        (
            "=FVSCHEDULE(1000,A1:A4)",
            &[("A1", n(0.1)), ("A3", n(0.2))],
            n(1320.0),
        ),
        (
            "=FVSCHEDULE(1000,A1:A3)",
            &[("A1", n(0.1)), ("A2", text("abc")), ("A3", n(0.2))],
            error(ExcelErrorKind::Value),
        ),
        (
            "=FVSCHEDULE(1000,A1:A3)",
            &[("A1", n(0.1)), ("A2", text("0.5")), ("A3", n(0.2))],
            error(ExcelErrorKind::Value),
        ),
        (
            "=FVSCHEDULE(1000,A1:A3)",
            &[
                ("A1", n(0.1)),
                ("A2", LiteralValue::Boolean(true)),
                ("A3", n(0.2)),
            ],
            error(ExcelErrorKind::Value),
        ),
        (
            "=FVSCHEDULE(1000,A1:A2)",
            &[("A1", n(0.1)), ("A2", text("=NA()"))],
            error(ExcelErrorKind::Na),
        ),
    ];
    for (formula, cells, expected) in cases {
        let actual = eval_with(formula, cells);
        assert!(
            same(&actual, expected, 1e-12, 0.0),
            "{formula}: {actual:?}, not {expected:?}"
        );
    }
}

/// COUPDAYS on actual/actual for a maturity after the 28th with a February
/// coupon follows a count the probes did not pin down (182 for the 184-day
/// period from 2024-02-29 to 2024-08-31 when settlement is 2024-03-31, 184
/// when it is 2024-08-30), and so does a previous coupon date before day 0:
/// the call is #N/IMPL!, so the workbook falls back. Every other case is a row
/// above.
#[test]
fn coupdays_actual_month_end_maturity_is_not_computed() {
    for formula in [
        "=COUPDAYS(DATE(2024,3,31),DATE(2030,8,31),2,1)",
        "=COUPDAYS(DATE(2024,8,30),DATE(2030,8,30),2,1)",
        "=COUPDAYS(DATE(2024,1,15),DATE(2030,8,29),2,1)",
        "=COUPDAYS(DATE(2024,11,30),DATE(2030,5,31),4,1)",
        "=COUPDAYS(0,DATE(2011,11,15),2,1)",
    ] {
        assert_eq!(
            eval_with(formula, &[]),
            error(ExcelErrorKind::NImpl),
            "{formula}"
        );
    }
}
