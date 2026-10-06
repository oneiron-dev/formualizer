//! The forms of round 6's functions a probe does not write: the trim-reference
//! operators and optional LAMBDA parameters as Excel's entry form spells them
//! (a file stores `_xlfn._TRO_TRAILING(A1:D6)` and `_xlop.b`), and the calls
//! the engine leaves to a fallback (`#N/IMPL!`) because their Excel result
//! depends on what it does not reproduce.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// The value of `formula` in F1 of a sheet with B2 = 1, C2 = 2, B3 = 3 and
/// C3 = 4 (and the probes' Z1 canary).
fn eval(formula: &str) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, col, value) in [(2, 2, 1.0), (2, 3, 2.0), (3, 2, 3.0), (3, 3, 4.0)] {
        engine
            .set_cell_value("Sheet1", row, col, LiteralValue::Number(value))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 1, 26, parse("=1111+2222").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 6, parse(formula).unwrap())
        .unwrap_or_else(|e| panic!("{formula}: {e:?}"));
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 6).unwrap()
}

fn assert_values(cases: &[(&str, LiteralValue)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(formula, expected)| {
            let actual = eval(formula);
            let same = match (&actual, expected) {
                (LiteralValue::Int(a), LiteralValue::Number(b)) => *a as f64 == *b,
                (LiteralValue::Error(a), LiteralValue::Error(b)) => a.kind == b.kind,
                _ => actual == *expected,
            };
            (!same).then(|| format!("{formula}: {actual:?}, not {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn n(n: f64) -> LiteralValue {
    LiteralValue::Number(n)
}

fn not_computed() -> LiteralValue {
    LiteralValue::Error(ExcelErrorKind::NImpl.into())
}

/// Excel for Windows 16.0.20430 typed in F1: `SUM(A1:.D6)` is 10, `ROWS` of
/// `A1:.D6` 3, `A1.:D6` 5, `A1.:.D6` 2; `A2:.A1` is the range A1:A2.
#[test]
fn trim_operators_as_typed() {
    assert_values(&[
        ("=SUM(A1:.D6)", n(10.0)),
        ("=ROWS(A1:.D6)", n(3.0)),
        ("=ROWS(A1.:D6)", n(5.0)),
        ("=ROWS(A1.:.D6)", n(2.0)),
        ("=COLUMNS(A1.:.D6)", n(2.0)),
        ("=ROWS(Sheet1!A1:.D6)", n(3.0)),
        ("=ROWS(D6:.A1)", n(3.0)),
        ("=ROWS(B:.B)", n(3.0)),
        ("=SUM(A1:.D6 B1:C9)", n(10.0)),
        ("=ISREF(A1:.D6)", LiteralValue::Boolean(true)),
        (
            "=ROWS(A5:.D6)",
            LiteralValue::Error(ExcelErrorKind::Ref.into()),
        ),
    ]);
    let ast = parse("=SUM(A1.:.D6)").unwrap();
    assert!(format!("{ast:?}").contains("_xlfn._TRO_ALL"), "{ast:?}");
}

/// Excel's entry form of an optional parameter, `[b]`, reads like the
/// file's `_xlop.b`.
#[test]
fn optional_lambda_parameters_as_typed() {
    assert_values(&[
        (
            "=LAMBDA(a,[b],ISOMITTED(b))(1)",
            LiteralValue::Boolean(true),
        ),
        ("=LAMBDA(a,[b],IF(ISOMITTED(b),a,a+b))(1,5)", n(6.0)),
        (
            "=LAMBDA(a,[b],[c],IF(ISOMITTED(c),\"no c\",c))(1,,3)",
            n(3.0),
        ),
        (
            "=_xlfn.LAMBDA(_xlpm.a,_xlop.b,_xlfn.ISOMITTED(_xlpm.b))(1)",
            LiteralValue::Boolean(true),
        ),
        ("=LAMBDA(a,b,ISOMITTED(b))(1,)", LiteralValue::Boolean(true)),
        (
            "=LAMBDA(a,b,ISOMITTED(b))(1)",
            LiteralValue::Error(ExcelErrorKind::Value.into()),
        ),
        (
            "=LAMBDA(a,[b],a+b)(1,2,3)",
            LiteralValue::Error(ExcelErrorKind::Value.into()),
        ),
        (
            "=MAP({1,2},LAMBDA(x,[y],ISOMITTED(y)))",
            LiteralValue::Boolean(true),
        ),
    ]);
}

/// Calls whose Excel result this engine does not reproduce are not
/// computed: the workbook goes to a fallback engine.
#[test]
fn calls_left_to_a_fallback() {
    assert_values(&[
        // Grapheme clusters, recursion and backtracking verbs.
        ("=REGEXTEST(\"é\",\"^\\X$\")", not_computed()),
        ("=REGEXTEST(\"aa\",\"(a)(?1)\")", not_computed()),
        ("=REGEXTEST(\"a\",\"a(*COMMIT)\")", not_computed()),
        // Exponential backtracking: Excel stops at PCRE2's match limit (#VALUE!).
        (
            "=REGEXTEST(REPT(\"a\",46)&\"!\",\"^(a|a)*$\")",
            not_computed(),
        ),
        // A series that is not exactly a line plus a season: Excel's result
        // depends on its optimizer (138.69607025814878).
        (
            "=FORECAST.ETS(25,{112,118,132,129,121,135,148,148,136,119,104,118,115,126,141,135,125,149,170,170,158,133,114,140},{1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24})",
            not_computed(),
        ),
        (
            "=FORECAST.ETS.STAT({10,20,30,40,50,60},{1,2,3,4,5,6},1)",
            not_computed(),
        ),
        ("=MUNIT(2001)", not_computed()),
    ]);
}
