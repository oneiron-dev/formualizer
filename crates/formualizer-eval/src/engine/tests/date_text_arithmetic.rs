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
fn datevalue_ignores_the_time_in_date_and_time_text() {
    // Microsoft's DATEVALUE: "Time information in the date_text argument is
    // ignored." DATEVALUE("1/2/2023 6:00") is 44928.
    for formula in [
        "=DATEVALUE(\"1/2/2023 6:00\")",
        "=DATEVALUE(\" 1/2/2023 6:00 \")",
        "=DATEVALUE(\"1/2/2023 6:00 PM\")",
        "=DATEVALUE(\"1/2/2023 23:59:59\")",
        "=DATEVALUE(\"2023-01-02 06:00\")",
        "=DATEVALUE(\"Jan 2, 2023 6:00\")",
    ] {
        assert_eq!(
            eval_formula(DateSystem::Excel1900, formula)
                .as_serial_number_for(DateSystem::Excel1900),
            Some(44928.0),
            "{formula}"
        );
    }
    // The text must still be date and time text, with only spaces around it.
    for formula in [
        "=DATEVALUE(\"1/2/2023 6:00\"&CHAR(10))",
        "=DATEVALUE(\"1/2/2023 6:00 XM\")",
    ] {
        assert_expected(
            DateSystem::Excel1900,
            formula,
            "rule: not date text",
            Expected::Error(ExcelErrorKind::Value),
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
fn group_separators_in_numeric_text_are_skipped() {
    for (formula, expected) in [
        ("=\"1,234\"+0", Expected::Number(1234.0)),
        ("=VALUE(\"1,234.5\")", Expected::Number(1234.5)),
        ("=SUM(\"1,234\",1)", Expected::Number(1235.0)),
        ("=TEXT(\"1,234\",\"0.0\")", Expected::Text("1234.0")),
        // 3_45896: a joined list of serials is one number, past the calendar.
        (
            "=TEXT(\"45627,45657\",\"yyyy-mm-dd\")",
            Expected::Error(ExcelErrorKind::Value),
        ),
        // A group shorter than three digits is not a number.
        ("=\"1,23\"+0", Expected::Error(ExcelErrorKind::Value)),
        ("=TEXT(\"1,23\",\"0.0\")", Expected::Text("1,23")),
    ] {
        assert_expected(DateSystem::Excel1900, formula, "en-US grouping", expected);
    }
}

#[test]
fn currency_and_parenthesized_numeric_text_is_a_number() {
    for (formula, expected) in [
        // Microsoft's VALUE example.
        ("=VALUE(\"$1,000\")", Expected::Number(1000.0)),
        ("=\"$1,234.50\"*1", Expected::Number(1234.5)),
        ("=--\"-$5\"", Expected::Number(-5.0)),
        ("=\"$-5\"+0", Expected::Number(-5.0)),
        ("=VALUE(\"($1,000)\")", Expected::Number(-1000.0)),
        ("=\"(250)\"+0", Expected::Number(-250.0)),
        // DOLLAR's text reads back as its number.
        ("=DOLLAR(-1234.5)*1", Expected::Number(-1234.5)),
        ("=SUM(\"$5\",1)", Expected::Number(6.0)),
        ("=TEXT(\"($5)\",\"0.00\")", Expected::Text("-5.00")),
        ("=VALUE(\"$5%\")", Expected::Error(ExcelErrorKind::Value)),
        ("=VALUE(\"(-5)\")", Expected::Error(ExcelErrorKind::Value)),
        ("=VALUE(\"--5\")", Expected::Error(ExcelErrorKind::Value)),
        ("=\"inf\"+0", Expected::Error(ExcelErrorKind::Value)),
    ] {
        assert_expected(DateSystem::Excel1900, formula, "en-US currency", expected);
    }
}

#[test]
fn criteria_keep_nonprinting_characters_and_quote_marks_like_the_cells() {
    // A1 holds CHAR(10)&"=5": the line feed is a character of the text, not
    // whitespace before an "=" operator, so the criterion A1 matches A1 and
    // not the number 5. Likewise "TRUE"&CHAR(10) is no logical, and quote
    // marks are characters of the text they stand in.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let sheet = "Sheet1";
    for (row, value) in [
        (1, LiteralValue::Text("\n=5".into())),
        (2, LiteralValue::Number(5.0)),
        (3, LiteralValue::Text("TRUE\n".into())),
        (4, LiteralValue::Boolean(true)),
        (5, LiteralValue::Text("\"x\"".into())),
        (6, LiteralValue::Text("x".into())),
    ] {
        engine.set_cell_value(sheet, row, 1, value).unwrap();
    }
    let cases = [
        ("=COUNTIF(A1,A1)", 1.0),
        ("=COUNTIF(A2,A1)", 0.0),
        ("=COUNTIF(A1:A6,CHAR(10)&\"=5\")", 1.0),
        ("=COUNTIFS(A1:A6,A1)", 1.0),
        ("=COUNTIF(A3,A3)", 1.0),
        ("=COUNTIF(A4,A3)", 0.0),
        ("=COUNTIF(A1:A6,CHAR(9)&\"TRUE\")", 0.0),
        ("=COUNTIF(A5,A5)", 1.0),
        ("=COUNTIF(A1:A6,\"\"\"x\"\"\")", 1.0),
        ("=COUNTIF(A1:A6,\"=\"\"x\"\"\")", 1.0),
        ("=COUNTIF(A1:A6,\"x\")", 1.0),
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
fn comparisons_keep_date_text_as_text_but_criteria_read_it() {
    // Comparison operators do not read text as a date or number: text sorts
    // above every number, as in Excel and LibreOffice.
    assert_expected(
        DateSystem::Excel1900,
        "=\"1/1/03\"<37623",
        "oracle: lo-verified",
        Expected::Boolean(false),
    );

    // COUNTIF criteria read date text as the date, as Excel and LO do.
    assert_expected(
        DateSystem::Excel1900,
        "=COUNTIF({37622},\"1/1/03\")",
        "oracle: lo-verified",
        Expected::Number(1.0),
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

/// Evaluate `formula` at J10 of an engine in `system` whose clock reads noon
/// UTC on `today`, with A1:A4 = b a b a and C1:C4 = 1 2 3 4; return the
/// `rows` x `cols` block from J10.
fn spill_at_clock(
    system: DateSystem,
    today: (i32, u32, u32),
    formula: &str,
    rows: u32,
    cols: u32,
) -> Vec<Vec<LiteralValue>> {
    use chrono::TimeZone;
    let timestamp_utc = chrono::Utc
        .with_ymd_and_hms(today.0, today.1, today.2, 12, 0, 0)
        .single()
        .unwrap();
    let config = EvalConfig {
        deterministic_mode: crate::engine::DeterministicMode::Enabled {
            timestamp_utc,
            timezone: crate::timezone::TimeZoneSpec::Utc,
        },
        ..Default::default()
    }
    .with_date_system(system);
    let mut engine = Engine::new(TestWorkbook::new(), config);
    for (row, (key, value)) in [("b", 1.0), ("a", 2.0), ("b", 3.0), ("a", 4.0)]
        .into_iter()
        .enumerate()
    {
        let row = row as u32 + 1;
        engine
            .set_cell_value("Sheet1", row, 1, LiteralValue::Text(key.into()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 3, LiteralValue::Number(value))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 10, 10, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    engine
                        .get_cell_value("Sheet1", 10 + r, 10 + c)
                        .unwrap_or(LiteralValue::Empty)
                })
                .collect()
        })
        .collect()
}

#[test]
fn functions_with_their_own_dispatch_read_date_text_in_the_workbook_context() {
    let n = LiteralValue::Number;
    let empty = LiteralValue::Empty;
    // MAKEARRAY and GROUPBY dispatch themselves. Their number arguments read
    // date text in the workbook's date system and the clock's year at the
    // top of a formula as well as nested: "1/2/1904" is serial 1 in the 1904
    // system, so MAKEARRAY spills one row.
    assert_eq!(
        spill_at_clock(
            DateSystem::Excel1904,
            (2026, 10, 2),
            "=MAKEARRAY(\"1/2/1904\",1,LAMBDA(i,j,i))",
            2,
            1
        ),
        vec![vec![n(1.0)], vec![empty.clone()]]
    );
    assert_eq!(
        spill_at_clock(
            DateSystem::Excel1904,
            (2026, 10, 2),
            "=ROWS(MAKEARRAY(\"1/2/1904\",1,LAMBDA(i,j,i)))",
            1,
            1
        ),
        vec![vec![n(1.0)]]
    );
    // Year-less date text reads in the clock's year: Jan 3 of 1900 is serial
    // 3 in the 1900 system, Jan 3 of 1904 serial 2 in the 1904 system.
    for (system, year, rows) in [
        (DateSystem::Excel1900, 1900, 3),
        (DateSystem::Excel1904, 1904, 2),
    ] {
        let mut expected: Vec<Vec<LiteralValue>> = (1..=rows).map(|r| vec![n(r as f64)]).collect();
        expected.push(vec![empty.clone()]);
        assert_eq!(
            spill_at_clock(
                system,
                (year, 6, 1),
                "=MAKEARRAY(\"Jan 3\",1,LAMBDA(i,j,i))",
                rows + 1,
                1
            ),
            expected,
            "{system:?}"
        );
    }
    // GROUPBY's total_depth "1/1/1904" is 0 in the 1904 system: no total row.
    let text = |s: &str| LiteralValue::Text(s.into());
    assert_eq!(
        spill_at_clock(
            DateSystem::Excel1904,
            (2026, 10, 2),
            "=GROUPBY(A1:A4,C1:C4,SUM,,\"1/1/1904\")",
            3,
            2
        ),
        vec![
            vec![text("a"), n(6.0)],
            vec![text("b"), n(4.0)],
            vec![empty.clone(), empty.clone()],
        ]
    );
}

fn assert_value_error(system: DateSystem, formula: &str) {
    match eval_with_text_a1(system, "1/9/2020", formula) {
        LiteralValue::Error(error) => {
            assert_eq!(error.kind, ExcelErrorKind::Value, "{formula} ({system:?})")
        }
        other => panic!("{formula} ({system:?}): expected #VALUE!, got {other:?}"),
    }
}

#[test]
fn date_text_outside_the_date_systems_range_is_no_number() {
    // Microsoft documents date text as January 1, 1900 (1904 in the 1904
    // system) through December 31, 9999; text outside the range is no date.
    for formula in [
        "=INT(\"1/1/1899\")",
        "=ABS(\"1/1/1899\")",
        "=SUM(\"12/31/1899 12:00\")",
        "=\"1/1/1899\"+0",
        "=VALUE(\"1/1/1899\")",
        "=DATEVALUE(\"1/1/1899\")",
        "=YEAR(\"1/1/1899\")",
    ] {
        for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
            assert_value_error(system, formula);
        }
    }
    for formula in [
        "=INT(\"12/31/1903\")",
        "=\"1/1/1900\"+0",
        "=DATEVALUE(\"1/1/1903\")",
    ] {
        assert_value_error(DateSystem::Excel1904, formula);
    }
    for (system, formula, expected) in [
        (DateSystem::Excel1900, "=INT(\"1/1/1900\")", 1.0),
        (DateSystem::Excel1900, "=INT(\"12/31/1903\")", 1461.0),
        (
            DateSystem::Excel1900,
            "=INT(\"12/31/9999 18:00\")",
            2_958_465.0,
        ),
        (DateSystem::Excel1904, "=INT(\"1/1/1904\")", 0.0),
        (DateSystem::Excel1904, "=DATEVALUE(\"1/2/1904\")", 1.0),
        (DateSystem::Excel1904, "=INT(\"12/31/9999\")", 2_957_003.0),
    ] {
        // DATEVALUE's result carries a date format; compare its serial.
        let actual = match eval_with_text_a1(system, "", formula) {
            LiteralValue::Number(n) => n,
            other => other
                .as_serial_number_for(system)
                .unwrap_or_else(|| panic!("{formula} ({system:?}): {other:?}")),
        };
        assert_eq!(actual, expected, "{formula} ({system:?})");
    }
}

#[test]
fn direct_arguments_of_list_functions_convert_date_text() {
    for (system, offset) in [
        (DateSystem::Excel1900, 0.0),
        (DateSystem::Excel1904, 1462.0),
    ] {
        // MULTINOMIAL: a value typed into the list is a number argument.
        for (formula, expected) in [
            ("=MULTINOMIAL(\"12:00\",1)", 1.0),
            ("=MULTINOMIAL(\"2\",1)", 3.0),
            ("=MULTINOMIAL({1,2},1)", 12.0),
        ] {
            assert_eq!(
                eval_with_text_a1(system, "1/9/2020", formula),
                LiteralValue::Number(expected),
                "{formula} ({system:?})"
            );
        }
        // "1/2/1900" is serial 2 in the 1900 system ("1/2/1904" in the 1904).
        let two = if offset == 0.0 {
            "=MULTINOMIAL(\"1/2/1900\",1)"
        } else {
            "=MULTINOMIAL(\"1/3/1904\",1)"
        };
        assert_eq!(
            eval_with_text_a1(system, "1/9/2020", two),
            LiteralValue::Number(3.0),
            "{two} ({system:?})"
        );
        assert_value_error(system, "=MULTINOMIAL(\"abc\",1)");

        // AVERAGE: text typed into the list that is no number is #VALUE!;
        // text in a reference is skipped.
        assert_value_error(system, "=AVERAGE(\"13/13/2020\",1)");
        assert_value_error(system, "=AVERAGE(\"abc\")");
        for (formula, expected) in [
            ("=AVERAGE(A1,1)", 1.0),
            ("=AVERAGE(A1:A2)", 2.0),
            ("=AVERAGE(\"12:00\",1.5)", 1.0),
            ("=AVERAGE(\"1/9/2020\",\"1/11/2020\")", 43840.0 - offset),
        ] {
            assert_eq!(
                eval_with_text_a1(system, "1/9/2020", formula),
                LiteralValue::Number(expected),
                "{formula} ({system:?})"
            );
        }
    }
}

#[test]
fn financial_number_arguments_convert_date_and_time_text() {
    for (system, offset) in [
        (DateSystem::Excel1900, 0.0),
        (DateSystem::Excel1904, 1462.0),
    ] {
        for (formula, expected) in [
            ("=SLN(\"12:00\",0,1)", 0.5),
            ("=SLN(\"5\",0,1)", 5.0),
            ("=SLN(\"1/9/2020\",0,1)", 43839.0 - offset),
            ("=SYD(\"12:00\",0,1,1)", 0.5),
            ("=PMT(0,1,\"12:00\")", -0.5),
            ("=FV(0,\"2\",-1)", 2.0),
        ] {
            assert_eq!(
                eval_with_text_a1(system, "1/9/2020", formula),
                LiteralValue::Number(expected),
                "{formula} ({system:?})"
            );
        }
        assert_value_error(system, "=SLN(\"abc\",0,1)");
        assert_value_error(system, "=PMT(0,1,\"13/13/2020\")");
    }
}

#[test]
fn financial_number_arguments_reject_nan_and_infinity_text() {
    // "NaN", "inf", "infinity" (any case or sign) and text past the double
    // range are not numeric text in Excel, so a financial number argument
    // holding them is #VALUE!, typed in the call or read from a cell. The
    // text coercion financial arguments share with VALUE() used to read them
    // as a NaN or an infinity: =DB(1000,100,"inf",1) overflowed DB's period
    // count and panicked.
    for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
        for text in ["NaN", "nan", "inf", "-Inf", "INFINITY", "1e400", "-1E400"] {
            for formula in [
                format!("=DB(1000,100,\"{text}\",1)"),
                format!("=DB(1000,100,10,\"{text}\")"),
                format!("=DDB(1000,100,\"{text}\",1)"),
                format!("=SLN(\"{text}\",0,1)"),
                format!("=SYD(1000,100,\"{text}\",1)"),
                format!("=PMT(\"{text}\",1,1)"),
                format!("=PV(0,\"{text}\",-1)"),
                format!("=FV(0,\"{text}\",-1)"),
                format!("=NPER(0,-1,\"{text}\")"),
                format!("=PRICE(DATE(2020,1,1),DATE(2030,1,1),0.05,\"{text}\",100,1)"),
                format!("=ACCRINTM(DATE(2020,1,1),DATE(2021,1,1),\"{text}\",1000)"),
                "=DB(1000,100,A1,1)".to_string(),
                "=PMT(0,1,A1)".to_string(),
                "=PRICE(DATE(2020,1,1),DATE(2030,1,1),A1,0.05,100,1)".to_string(),
            ] {
                match eval_with_text_a1(system, text, &formula) {
                    LiteralValue::Error(error) => assert_eq!(
                        error.kind,
                        ExcelErrorKind::Value,
                        "{formula} with A1 {text:?} ({system:?})"
                    ),
                    other => panic!(
                        "{formula} with A1 {text:?} ({system:?}): expected #VALUE!, got {other:?}"
                    ),
                }
            }
        }
    }
    // A finite life too long for a period count saturates rather than
    // overflowing: the rate rounds to 0, so the first year depreciates 0.
    assert_eq!(
        eval_with_text_a1(DateSystem::Excel1900, "", "=DB(1000,100,1E+300,1)"),
        LiteralValue::Number(0.0)
    );
    // Ordinary numeric text still converts.
    assert_eq!(
        eval_with_text_a1(DateSystem::Excel1900, "10", "=SLN(1000,0,A1)"),
        LiteralValue::Number(100.0)
    );
}

#[test]
fn nan_and_infinity_text_is_no_number_for_operators_and_number_arguments() {
    // The same rule for the arithmetic operators, VALUE() and the number
    // parameters of other functions: such text is #VALUE!, from a cell too.
    for text in ["NaN", "-nan", "inf", "+Infinity", "1e400"] {
        for formula in [
            format!("=\"{text}\"+0"),
            format!("=-\"{text}\""),
            format!("=VALUE(\"{text}\")"),
            format!("=INT(\"{text}\")"),
            format!("=ABS(\"{text}\")"),
            "=A1*1".to_string(),
            "=INT(A1)".to_string(),
        ] {
            match eval_with_text_a1(DateSystem::Excel1900, text, &formula) {
                LiteralValue::Error(error) => assert_eq!(
                    error.kind,
                    ExcelErrorKind::Value,
                    "{formula} with A1 {text:?}"
                ),
                other => panic!("{formula} with A1 {text:?}: expected #VALUE!, got {other:?}"),
            }
        }
        // A criterion holding such text is a text criterion: it counts the
        // cell holding that text, not a number.
        assert_eq!(
            eval_with_text_a1(
                DateSystem::Excel1900,
                text,
                &format!("=COUNTIF(A1:A2,\"{text}\")")
            ),
            LiteralValue::Number(1.0),
            "COUNTIF {text:?}"
        );
    }
}

#[test]
fn lcm_of_2_to_the_53_or_more_is_num() {
    // Microsoft: "If lcm(a,b) >= 2^53, LCM returns the #NUM! error value."
    for formula in [
        "=LCM(9999999989,9999999988)",
        "=LCM(\"1/1/9999\",9999999989)",
    ] {
        match eval_with_text_a1(DateSystem::Excel1900, "", formula) {
            LiteralValue::Error(error) => assert_eq!(error.kind, ExcelErrorKind::Num, "{formula}"),
            other => panic!("{formula}: expected #NUM!, got {other:?}"),
        }
    }
    // 6361 * 69431 * 20394401 = 2^53 - 1, the largest LCM Excel returns.
    for (formula, expected) in [
        ("=LCM(441650591,20394401)", 9_007_199_254_740_991.0),
        ("=LCM(6,\"1/10/1900\")", 30.0),
        ("=LCM(0,9999999989)", 0.0),
    ] {
        assert_eq!(
            eval_with_text_a1(DateSystem::Excel1900, "", formula),
            LiteralValue::Number(expected),
            "{formula}"
        );
    }
}

#[test]
fn number_arguments_read_currency_and_grouped_text_like_value() {
    // A function's number parameter converts text as VALUE() does, so the
    // en-US currency, group-separator and parenthesized forms are numbers
    // there before any date reading, in either date system; text in a
    // reference is still no number to SUM.
    for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
        for (formula, expected) in [
            ("=INT(A1)", 1234.0),
            ("=ROUND(A1,0)", 1235.0),
            ("=ABS(\"($5)\")", 5.0),
            ("=MAX(\"(5)\",-10)", -5.0),
            ("=SUM(\"1,000\",\"$5\")", 1005.0),
            ("=AVERAGE(\"$2\",\"4\")", 3.0),
            ("=MOD(\"1,234\",1000)", 234.0),
            ("=SLN(\"$1,000\",0,1)", 1000.0),
            ("=SUM(A1)", 0.0),
        ] {
            let actual = match eval_with_text_a1(system, "$1,234.50", formula) {
                LiteralValue::Number(n) => n,
                LiteralValue::Int(n) => n as f64,
                other => panic!("{formula} ({system:?}): {other:?}"),
            };
            assert_eq!(actual, expected, "{formula} ({system:?})");
        }
        for formula in ["=INT(\"$5%\")", "=ABS(\"1,23\")", "=SUM(\"(-5)\")"] {
            assert_value_error(system, formula);
        }
    }
    // Date functions read the number before trying date text: serial 1000
    // is September 26, 1902.
    match eval_with_text_a1(DateSystem::Excel1900, "", "=DAY(\"$1,000\")") {
        LiteralValue::Number(n) => assert_eq!(n, 26.0),
        LiteralValue::Int(n) => assert_eq!(n, 26),
        other => panic!("DAY(\"$1,000\"): {other:?}"),
    }
}
