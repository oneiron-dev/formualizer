use crate::engine::{DateSystem, Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use chrono::NaiveDate;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

#[derive(Clone, Copy, Debug)]
enum Expected {
    Number(f64),
    Boolean(bool),
    Text(&'static str),
    Error(ExcelErrorKind),
}

fn eval_formula(system: DateSystem, formula: &str) -> LiteralValue {
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig::default().with_date_system(system),
    );
    engine
        .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine
        .get_cell_value("Sheet1", 1, 1)
        .unwrap_or(LiteralValue::Empty)
}

fn assert_expected(system: DateSystem, formula: &str, oracle: &str, expected: Expected) {
    let actual = eval_formula(system, formula);
    match expected {
        Expected::Number(value) => {
            assert_eq!(actual, LiteralValue::Number(value), "{formula} ({oracle})")
        }
        Expected::Boolean(value) => {
            assert_eq!(actual, LiteralValue::Boolean(value), "{formula} ({oracle})")
        }
        Expected::Text(value) => assert_eq!(
            actual,
            LiteralValue::Text(value.to_string()),
            "{formula} ({oracle})"
        ),
        Expected::Error(kind) => match actual {
            LiteralValue::Error(error) => assert_eq!(error.kind, kind, "{formula} ({oracle})"),
            other => panic!("{formula} ({oracle}): expected {kind:?}, got {other:?}"),
        },
    }
}

#[test]
fn date_time_text_arithmetic_oracle_table() {
    let cases = [
        (
            "=\"1/1/03\"-\"6/01/2002\"",
            Expected::Number(214.0),
            Expected::Number(214.0),
        ),
        (
            "=\"1/1/2003\"-\"6/1/2002\"",
            Expected::Number(214.0),
            Expected::Number(214.0),
        ),
        (
            "=\"1/1/03\"+0",
            Expected::Number(37_622.0),
            Expected::Number(36_160.0),
        ),
        (
            "=-\"1/1/03\"",
            Expected::Number(-37_622.0),
            Expected::Number(-36_160.0),
        ),
        (
            "=\"1/1/03\"*1",
            Expected::Number(37_622.0),
            Expected::Number(36_160.0),
        ),
        (
            "=\"1/1/03\"/1",
            Expected::Number(37_622.0),
            Expected::Number(36_160.0),
        ),
        (
            "=\"1/1/03\"^1",
            Expected::Number(37_622.0),
            Expected::Number(36_160.0),
        ),
        (
            "=\"1/1/03\"%",
            Expected::Number(376.22),
            Expected::Number(361.6),
        ),
        (
            "=\"12:00\"-\"6:00\"",
            Expected::Number(0.25),
            Expected::Number(0.25),
        ),
        (
            "=\"1/1/03 12:00\"+0",
            Expected::Number(37_622.5),
            Expected::Number(36_160.5),
        ),
        (
            "=\"1-Jan-03\"+0",
            Expected::Number(37_622.0),
            Expected::Number(36_160.0),
        ),
        (
            "=ISNUMBER(\"1/1/03\"+0)",
            Expected::Boolean(true),
            Expected::Boolean(true),
        ),
    ];

    for (formula, expected_1900, expected_1904) in cases {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "oracle: lo-verified",
            expected_1900,
        );
        assert_expected(
            DateSystem::Excel1904,
            formula,
            "oracle: lo-verified",
            expected_1904,
        );
    }
}

#[test]
fn two_digit_year_window_honors_workbook_date_system_for_every_accepted_format() {
    let cases = [
        ("=\"1/1/29\"+0", 47_119.0, 45_657.0),
        ("=\"1/1/30\"+0", 10_959.0, 9_497.0),
        ("=\"January 1, 29\"+0", 47_119.0, 45_657.0),
        ("=\"January 1, 30\"+0", 10_959.0, 9_497.0),
        ("=\"Jan 1, 29\"+0", 47_119.0, 45_657.0),
        ("=\"Jan 1, 30\"+0", 10_959.0, 9_497.0),
        ("=\"1-Jan-29\"+0", 47_119.0, 45_657.0),
        ("=\"1-Jan-30\"+0", 10_959.0, 9_497.0),
    ];

    for (formula, expected_1900, expected_1904) in cases {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "oracle: lo-verified",
            Expected::Number(expected_1900),
        );
        assert_expected(
            DateSystem::Excel1904,
            formula,
            "oracle: lo-verified",
            Expected::Number(expected_1904),
        );
    }
}

#[test]
fn invalid_date_time_text_remains_value_error() {
    let cases = [
        "=\"2/30/03\"+0",
        "=\"abc\"+0",
        "=\"\"+0",
        "=\"13/13/13\"+0",
        "=\"123-456\"+0",
        "=\"15/01/2003\"+0",
        "=\"1/1/03T12:00\"+0",
    ];

    for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
        for formula in cases {
            assert_expected(
                system,
                formula,
                "oracle: lo-verified",
                Expected::Error(ExcelErrorKind::Value),
            );
        }
    }
}

#[test]
fn excel_date_shapes_and_year_less_dates_use_the_clock_year() {
    // Excel en-US reads m-d-y with dashes and y/m/d with a four-digit year.
    for (formula, serial_1900) in [("=\"03-01-01\"+0", 36951.0), ("=\"2003/1/1\"+0", 37622.0)] {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "excel en-US",
            Expected::Number(serial_1900),
        );
        assert_expected(
            DateSystem::Excel1904,
            formula,
            "excel en-US",
            Expected::Number(serial_1900 - 1462.0),
        );
    }
    // Date text without a year reads in the evaluation clock's year, as
    // Excel's DATEVALUE documents; a malformed time stays #VALUE!.
    use chrono::Datelike;
    let year = chrono::Utc::now().year();
    let jan3 = NaiveDate::from_ymd_opt(year, 1, 3).unwrap();
    let expected = formualizer_common::date_to_serial_for(DateSystem::Excel1900, &jan3);
    for formula in [
        "=\"Jan-03\"+0",
        "=\"1/03\"+0",
        "=\"Jan 3\"+0",
        "=\"3-Jan\"+0",
    ] {
        match eval_formula(DateSystem::Excel1900, formula) {
            LiteralValue::Number(n) => assert!(
                n == expected || (n - expected).abs() == 365.0 || (n - expected).abs() == 366.0,
                "{formula}: {n} vs {expected}"
            ),
            other => panic!("{formula}: {other:?}"),
        }
    }
    assert_expected(
        DateSystem::Excel1900,
        "=\"12:00.5\"+0",
        "oracle: lo-verified",
        Expected::Error(ExcelErrorKind::Value),
    );
}

#[test]
fn date_typed_arithmetic_uses_the_1904_workbook_system() {
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig::default().with_date_system(DateSystem::Excel1904),
    );
    engine
        .set_cell_value(
            "Sheet1",
            1,
            1,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2003, 1, 1).unwrap()),
        )
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 2, parse("=A1*1").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 3, parse("=A1%").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();

    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 2),
        Some(LiteralValue::Number(36_160.0))
    );
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 3),
        Some(LiteralValue::Number(361.6))
    );
}

#[test]
fn known_comparison_and_criteria_divergences_remain_pinned() {
    // Comparison operators do not read text as a date or number: text sorts
    // above every number, as in Excel and LibreOffice.
    assert_expected(
        DateSystem::Excel1900,
        "=\"1/1/03\"<37623",
        "oracle: lo-verified",
        Expected::Boolean(false),
    );

    // Formualizer does not date-coerce COUNTIF criteria here; LO returns 1.
    // The criteria-coercion divergence is tracked separately from #289.
    assert_expected(
        DateSystem::Excel1900,
        "=COUNTIF({37622},\"1/1/03\")",
        "oracle: lo-verified divergence",
        Expected::Number(0.0),
    );
}

#[test]
fn non_arithmetic_text_semantics_are_unchanged() {
    let cases = [
        ("=\"5\"+\"3\"", Expected::Number(8.0)),
        ("=\"5\"-\"3\"", Expected::Number(2.0)),
        (
            "=SUM(\"1/1/03\",\"1\")",
            Expected::Error(ExcelErrorKind::Value),
        ),
        ("=N(\"1/1/03\")", Expected::Number(0.0)),
        ("=T(\"1/1/03\")", Expected::Text("1/1/03")),
        ("=\"1/1/03\"&\"\"", Expected::Text("1/1/03")),
        ("=+\"1/1/03\"", Expected::Text("1/1/03")),
        ("=\"1/1/03\"=37622", Expected::Boolean(false)),
    ];

    for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
        for (formula, expected) in cases {
            assert_expected(system, formula, "oracle: lo-verified", expected);
        }
    }
}

#[test]
fn date_functions_and_value_read_date_text() {
    for (formula, expected) in [
        ("=MONTH(\"July\"&1)", 7.0),
        ("=MONTH(\"3/15/2021\")", 3.0),
        ("=DAY(\"15-Mar-2021\")", 15.0),
        ("=YEAR(\"1 January 2023\")", 2023.0),
        ("=DAYS(\"15-MAR-2021\",\"1-FEB-2021\")", 42.0),
        ("=EOMONTH(\"3/15/2021\",0)", 44286.0),
        ("=WEEKDAY(\"3/15/2021\")", 2.0),
        ("=VALUE(\"1/2/2023\")", 44928.0),
        ("=DATEVALUE(\"1\"&\"June\"&\"2021\")", 44348.0),
        ("=--\"Jan 5 2023\"", 44931.0),
    ] {
        match eval_formula(DateSystem::Excel1900, formula) {
            LiteralValue::Number(n) => assert_eq!(n, expected, "{formula}"),
            LiteralValue::Int(n) => assert_eq!(n as f64, expected, "{formula}"),
            other => assert_eq!(
                other.as_serial_number_for(DateSystem::Excel1900),
                Some(expected),
                "{formula}"
            ),
        }
    }
    assert_expected(
        DateSystem::Excel1900,
        "=MONTH(\"not a date\")",
        "excel",
        Expected::Error(ExcelErrorKind::Value),
    );
}

#[test]
fn month_name_with_a_number_that_is_no_day_reads_as_a_year() {
    // Excel reads `Mon n` as month/year on the 1st when n is no day of the
    // month, so MONTH(name&0) is the month number.
    for (formula, expected) in [
        ("=MONTH(\"March\"&0)", 3.0),
        ("=YEAR(\"January0\")", 2000.0),
        ("=DATEVALUE(\"Jan 0\")", 36526.0),
        ("=DATEVALUE(\"Apr 31\")", 11414.0),
        ("=YEAR(\"Feb 30\")", 1930.0),
        ("=\"Jan 45\"+0", 16438.0),
        ("=VALUE(\"Mar-00\")", 36586.0),
        ("=DAY(\"Jan 31\")", 31.0),
    ] {
        match eval_formula(DateSystem::Excel1900, formula) {
            LiteralValue::Number(n) => assert_eq!(n, expected, "{formula}"),
            LiteralValue::Int(n) => assert_eq!(n as f64, expected, "{formula}"),
            other => assert_eq!(
                other.as_serial_number_for(DateSystem::Excel1900),
                Some(expected),
                "{formula}"
            ),
        }
    }
}
