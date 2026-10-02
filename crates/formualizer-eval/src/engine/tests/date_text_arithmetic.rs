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
fn only_spaces_around_numeric_and_time_text_are_ignored() {
    // A cell holding "14:41\n16:47\n": RIGHT(...,5) is "6:47\n", not a time.
    let lf = "\"14:41\"&CHAR(10)&\"16:47\"&CHAR(10)";
    let cases = [
        format!("=RIGHT({lf},5)-LEFT({lf},5)"),
        "=(\"5\"&CHAR(10))+0".to_string(),
        "=(CHAR(10)&\"5\")+0".to_string(),
        "=(\"5\"&CHAR(9))+0".to_string(),
        "=VALUE(\"5\"&CHAR(10))".to_string(),
        "=DATEVALUE(\"1/2/2023\"&CHAR(10))".to_string(),
        "=TIMEVALUE(\"6:47\"&CHAR(13))".to_string(),
    ];
    for formula in &cases {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "56855: Excel #VALUE!",
            Expected::Error(ExcelErrorKind::Value),
        );
    }
    for (formula, expected) in [
        ("=\"  5 \"+0", 5.0),
        ("=VALUE(\" 6:00  \")", 0.25),
        ("=\" 90 % \"+0", 0.9),
    ] {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "spaces are ignored",
            Expected::Number(expected),
        );
    }
}

#[test]
fn datevalue_month_name_text_ignores_only_surrounding_spaces() {
    // DATEVALUE's "day Month year" fallback rebuilds the text, so a leading
    // line feed or tab must be rejected before it lands inside the day.
    for formula in [
        "=DATEVALUE(CHAR(10)&\"2 January 2023\")",
        "=DATEVALUE(CHAR(9)&\"2 Jan 2023\")",
        "=DATEVALUE(\" \"&CHAR(10)&\"2 January 2023\")",
        "=DATEVALUE(\"2 January 2023\"&CHAR(10)&\" \")",
    ] {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "rule: only spaces are ignored",
            Expected::Error(ExcelErrorKind::Value),
        );
    }
    // DATEVALUE's result carries a date format, so compare serials.
    for formula in [
        "=DATEVALUE(\"2 January 2023\")",
        "=DATEVALUE(\"  2 January 2023 \")",
    ] {
        assert_eq!(
            eval_formula(DateSystem::Excel1900, formula)
                .as_serial_number_for(DateSystem::Excel1900),
            Some(44928.0),
            "{formula} (spaces are ignored)"
        );
    }
}

#[test]
fn criteria_read_linefeed_text_as_text_like_the_cells() {
    // A3 holds the text "5"&CHAR(10), which is not numeric text. As a
    // criterion it is the same text, so a cell matches its own value, and it
    // does not match the number 5 in A1.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let sheet = "Sheet1";
    engine
        .set_cell_value(sheet, 1, 1, LiteralValue::Number(5.0))
        .unwrap();
    engine
        .set_cell_value(sheet, 3, 1, LiteralValue::Text("5\n".into()))
        .unwrap();
    engine
        .set_cell_value(sheet, 3, 2, LiteralValue::Number(7.0))
        .unwrap();
    let cases = [
        ("=COUNTIF(A3,A3)", 1.0),
        ("=COUNTIF(A3,\"5\"&CHAR(10))", 1.0),
        ("=COUNTIF(A3,\"=5\"&CHAR(10))", 1.0),
        ("=SUMIF(A3,A3,B3)", 7.0),
        ("=COUNTIFS(A3,A3)", 1.0),
        ("=SUMIFS(B3,A3,A3)", 7.0),
        ("=AVERAGEIF(A3,A3,B3)", 7.0),
        ("=COUNTIF(A1,A3)", 0.0),
        ("=COUNTIF(A1:A3,5)", 1.0),
        ("=COUNTIF(A1:A3,\" 5 \")", 1.0),
    ];
    for (row, (formula, _)) in (10u32..).zip(cases.iter()) {
        engine
            .set_cell_formula(sheet, row, 1, parse(formula).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    for (row, (formula, expected)) in (10u32..).zip(cases.iter()) {
        assert_eq!(
            engine.get_cell_value(sheet, row, 1),
            Some(LiteralValue::Number(*expected)),
            "{formula}"
        );
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

#[test]
fn numeric_month_and_number_that_is_no_day_reads_as_month_year() {
    // Excel reads a two-part `m/n` as month/day in the current year, else as
    // month/year on the 1st with the 2029 window (Microsoft's table: 12/99,
    // 11/95, 1/99; 13/99 stays text).
    for (formula, expected) in [
        ("=DATEVALUE(\"12/99\")", 36495.0),
        ("=DATEVALUE(\"11/95\")", 35004.0),
        ("=DATEVALUE(\"1/99\")", 36161.0),
        ("=\"12/99\"+0", 36495.0),
        ("=DATEVALUE(\"2/30\")", 10990.0),
        ("=DATEVALUE(\"12-99\")", 36495.0),
        ("=VALUE(\"4/31\")", 11414.0),
        ("=YEAR(\"1/00\")", 2000.0),
        ("=MONTH(\"11/95\")", 11.0),
        ("=DAY(\"1/30\")", 30.0),
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
        DateSystem::Excel1904,
        "=\"12/99\"+0",
        "excel",
        Expected::Number(35033.0),
    );
    for formula in ["=DATEVALUE(\"13/99\")", "=\"0/99\"+0"] {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "excel",
            Expected::Error(ExcelErrorKind::Value),
        );
    }
}

/// Evaluate `formula` in B1 of a sheet whose A1 holds `a1` as text.
fn eval_with_text_a1(system: DateSystem, a1: &str, formula: &str) -> LiteralValue {
    let mut engine = Engine::new(
        TestWorkbook::new(),
        EvalConfig::default().with_date_system(system),
    );
    engine
        .set_cell_value("Sheet1", 1, 1, LiteralValue::Text(a1.into()))
        .unwrap();
    engine
        .set_cell_value("Sheet1", 2, 1, LiteralValue::Number(2.0))
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 2, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine
        .get_cell_value("Sheet1", 1, 2)
        .unwrap_or(LiteralValue::Empty)
}

#[test]
fn number_arguments_read_date_text_like_value() {
    // Excel converts text passed to a function's number parameter as VALUE()
    // and the operators do, date and time text included (en-US M/d/yyyy):
    // INT(A2) with A2 holding "01/09/2020 15:02:40" is 43839, not #VALUE!.
    let stamp = "01/09/2020 15:02:40";
    let fraction = (15.0 * 3600.0 + 2.0 * 60.0 + 40.0) / 86400.0;
    for (system, offset) in [
        (DateSystem::Excel1900, 0.0),
        (DateSystem::Excel1904, 1462.0),
    ] {
        let day = 43839.0 - offset;
        for (formula, expected) in [
            ("=INT(A1)", day),
            ("=TRUNC(A1)", day),
            ("=ROUNDDOWN(A1,0)", day),
            ("=ABS(\"1/9/2020\")", day),
            ("=MAX(\"1/9/2020\",1)", day),
            ("=SUM(\"1/9/2020\",\"1\")", day + 1.0),
            ("=AVERAGE(\"1/9/2020\",\"1/11/2020\")", day + 1.0),
            ("=COUNT(\"1/9/2020\",\"12:00\")", 2.0),
            ("=SUM(INT({\"1/9/2020\",\"2\"}))", day + 2.0),
            ("=MOD(\"1/9/2020 12:00\",1)", 0.5),
            ("=ROUND(\"12:00\",2)", 0.5),
        ] {
            let actual = eval_with_text_a1(system, stamp, formula);
            let actual = match actual {
                LiteralValue::Number(n) => n,
                LiteralValue::Int(n) => n as f64,
                other => other
                    .as_serial_number_for(system)
                    .unwrap_or_else(|| panic!("{formula} ({system:?}): {other:?}")),
            };
            assert_eq!(actual, expected, "{formula} ({system:?})");
        }
        match eval_with_text_a1(system, stamp, "=A1-INT(A1)") {
            LiteralValue::Number(n) => assert!((n - fraction).abs() < 1e-9, "{n}"),
            other => panic!("A1-INT(A1) ({system:?}): {other:?}"),
        }
        // Year-less date text reads in the clock's year, as the operators do.
        assert_eq!(
            eval_with_text_a1(system, "Jan 3", "=INT(A1)"),
            eval_with_text_a1(system, "Jan 3", "=A1+0"),
            "INT of year-less date text ({system:?})"
        );
    }
}

#[test]
fn date_text_in_references_and_non_number_text_are_unchanged() {
    for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
        // Text in a reference is no number to SUM, COUNT or SUMPRODUCT, even
        // when it reads as a date.
        for (formula, expected) in [
            ("=SUM(A1)", 0.0),
            ("=SUM(A1:A2)", 2.0),
            ("=COUNT(A1:A2)", 1.0),
            ("=SUMPRODUCT(A1:A2)", 2.0),
            ("=MEDIAN({\"1/9/2020\",5})", 5.0),
            ("=N(A1)", 0.0),
        ] {
            assert_eq!(
                eval_with_text_a1(system, "1/9/2020", formula),
                LiteralValue::Number(expected),
                "{formula} ({system:?})"
            );
        }
        for formula in ["=INT(A1)", "=ABS(\"1/9/2020x\")", "=SUM(\"13/13/2020\")"] {
            match eval_with_text_a1(system, "abc", formula) {
                LiteralValue::Error(error) => {
                    assert_eq!(error.kind, ExcelErrorKind::Value, "{formula}")
                }
                other => panic!("{formula} ({system:?}): expected #VALUE!, got {other:?}"),
            }
        }
    }
}
