use chrono::NaiveDate;

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::parse;

#[test]
fn countifs_date_criteria_with_ampersand_concatenation() {
    // Regression for criteria strings like ">="&C1 where C1 is a date.
    // Libre/Excel treat dates as serials for criteria parsing.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let sheet = "Calculations";

    // Criteria bounds: 2024-11-01 .. 2024-11-30
    engine
        .set_cell_value(
            sheet,
            1,
            3,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 11, 1).unwrap()),
        )
        .unwrap();
    engine
        .set_cell_value(
            sheet,
            1,
            4,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 11, 30).unwrap()),
        )
        .unwrap();

    // Values: 11/15, 11/29, 12/13
    engine
        .set_cell_value(
            sheet,
            110,
            3,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 11, 15).unwrap()),
        )
        .unwrap();
    engine
        .set_cell_value(
            sheet,
            111,
            3,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 11, 29).unwrap()),
        )
        .unwrap();
    engine
        .set_cell_value(
            sheet,
            112,
            3,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 12, 13).unwrap()),
        )
        .unwrap();

    // COUNTIFS(C110:C112,">="&C1,C110:C112,"<="&D1)
    engine
        .set_cell_formula(
            sheet,
            109,
            8,
            parse("=COUNTIFS(C110:C112,\">=\"&C1,C110:C112,\"<=\"&D1)").unwrap(),
        )
        .unwrap();

    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value(sheet, 109, 8),
        Some(LiteralValue::Number(2.0))
    );
}

#[test]
fn countifs_date_equality_accepts_date_literal_criteria() {
    // Criteria passed as a date value (not a string) should work.
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    let sheet = "Calculations";

    let target = NaiveDate::from_ymd_opt(2024, 11, 29).unwrap();

    engine
        .set_cell_value(sheet, 1, 3, LiteralValue::Date(target))
        .unwrap();

    engine
        .set_cell_value(
            sheet,
            110,
            3,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 11, 15).unwrap()),
        )
        .unwrap();
    engine
        .set_cell_value(sheet, 111, 3, LiteralValue::Date(target))
        .unwrap();
    engine
        .set_cell_value(
            sheet,
            112,
            3,
            LiteralValue::Date(NaiveDate::from_ymd_opt(2024, 12, 13).unwrap()),
        )
        .unwrap();

    engine
        .set_cell_formula(sheet, 10, 8, parse("=COUNTIFS(C110:C112,C1)").unwrap())
        .unwrap();

    engine.evaluate_all().unwrap();
    assert_eq!(
        engine.get_cell_value(sheet, 10, 8),
        Some(LiteralValue::Number(1.0))
    );
}

fn criteria_value(engine: &mut Engine<TestWorkbook>, formula: &str) -> LiteralValue {
    engine
        .set_cell_formula("Sheet1", 1, 20, parse(formula).unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    engine.get_cell_value("Sheet1", 1, 20).unwrap()
}

fn texts_and_dates_setup(engine: &mut Engine<TestWorkbook>) {
    // A: date text and dates; B: amounts; C: names.
    let rows = [
        (LiteralValue::Text("3-1-21".into()), 1.0, "gage yount"),
        (LiteralValue::Text("3-1-21".into()), 2.0, "devon foulk"),
        (LiteralValue::Text("3-2-21".into()), 4.0, "gage yount"),
        (LiteralValue::Number(44256.0), 8.0, "gage"),
        (LiteralValue::Text("Mar 1, 2021".into()), 16.0, "andrew"),
        (LiteralValue::Text("closed".into()), 32.0, "gage"),
    ];
    for (i, (date, amount, name)) in rows.into_iter().enumerate() {
        let row = i as u32 + 1;
        engine.set_cell_value("Sheet1", row, 1, date).unwrap();
        engine
            .set_cell_value("Sheet1", row, 2, LiteralValue::Number(amount))
            .unwrap();
        engine
            .set_cell_value("Sheet1", row, 3, LiteralValue::Text(name.into()))
            .unwrap();
    }
}

#[test]
fn date_criterion_matches_cells_holding_date_text() {
    // Criteria functions are not type-specific for dates: a cell holding the
    // text "3-1-21" (M/d/yy) equals the criterion DATE(2021,3,1).
    for config in [EvalConfig::default(), super::common::arrow_eval_config()] {
        let mut engine = Engine::new(TestWorkbook::new(), config);
        texts_and_dates_setup(&mut engine);
        let cases = [
            ("=SUMIFS(B1:B6,A1:A6,DATE(2021,3,1))", 27.0),
            ("=SUMIFS(B1:B6,A1:A6,DATE(2021,3,1),C1:C6,\"*gage*\")", 9.0),
            ("=SUMIF(A1:A6,44256,B1:B6)", 27.0),
            ("=COUNTIF(A1:A6,\"3/1/2021\")", 4.0),
            ("=COUNTIFS(A1:A6,\"=March 1, 2021\")", 4.0),
            ("=AVERAGEIF(A1:A6,DATE(2021,3,2),B1:B6)", 4.0),
            ("=MAXIFS(B1:B6,A1:A6,DATE(2021,3,1))", 16.0),
            // Ordered criteria compare numbers only, not date text.
            ("=COUNTIF(A1:A6,\">=3/1/2021\")", 1.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                criteria_value(&mut engine, formula),
                LiteralValue::Number(expected),
                "{formula}"
            );
        }
    }
}

#[test]
fn date_text_criteria_compare_as_dates() {
    // Microsoft's COUNTIFS example: =COUNTIFS(A2:A7,"<5",B2:B7,"<5/3/2011") is 2.
    for config in [EvalConfig::default(), super::common::arrow_eval_config()] {
        let mut engine = Engine::new(TestWorkbook::new(), config);
        for (i, day) in (1..=6).enumerate() {
            let row = i as u32 + 2;
            engine
                .set_cell_value("Sheet1", row, 1, LiteralValue::Number(day as f64))
                .unwrap();
            engine
                .set_cell_value(
                    "Sheet1",
                    row,
                    2,
                    LiteralValue::Date(NaiveDate::from_ymd_opt(2011, 5, day).unwrap()),
                )
                .unwrap();
        }
        let cases = [
            ("=COUNTIFS(A2:A7,\"<5\",B2:B7,\"<5/3/2011\")", 2.0),
            ("=COUNTIF(B2:B7,\"5/3/2011\")", 1.0),
            ("=COUNTIF(B2:B7,\">=May 4, 2011\")", 3.0),
            ("=SUMIFS(A2:A7,B2:B7,\"<>5/3/2011\")", 18.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                criteria_value(&mut engine, formula),
                LiteralValue::Number(expected),
                "{formula}"
            );
        }
    }
}

#[test]
fn criteria_read_date_text_in_the_workbook_date_system() {
    // In a 1904 workbook March 1, 2021 is 44256 - 1462: a criterion written
    // as date text, and date text in the cells, read in that system, in the
    // IF functions and the D-functions alike.
    let march_1_2021_1904 = 44256.0 - 1462.0;
    for config in [EvalConfig::default(), super::common::arrow_eval_config()] {
        let config = config.with_date_system(crate::engine::DateSystem::Excel1904);
        let mut engine = Engine::new(TestWorkbook::new(), config);
        let rows = [
            (
                LiteralValue::Text("Date".into()),
                LiteralValue::Text("Amount".into()),
            ),
            (
                LiteralValue::Number(march_1_2021_1904),
                LiteralValue::Number(1.0),
            ),
            (
                LiteralValue::Text("3-1-21".into()),
                LiteralValue::Number(2.0),
            ),
            (LiteralValue::Number(44256.0), LiteralValue::Number(4.0)),
            (LiteralValue::Text("Date".into()), LiteralValue::Empty),
            (LiteralValue::Text("3/1/2021".into()), LiteralValue::Empty),
        ];
        for (i, (date, amount)) in rows.into_iter().enumerate() {
            let row = i as u32 + 1;
            engine.set_cell_value("Sheet1", row, 1, date).unwrap();
            engine.set_cell_value("Sheet1", row, 2, amount).unwrap();
        }
        let cases = [
            ("=COUNTIF(A2:A4,\"3/1/2021\")", 2.0),
            ("=SUMIF(A2:A4,\"3/1/2021\",B2:B4)", 3.0),
            ("=SUMIFS(B2:B4,A2:A4,DATE(2021,3,1))", 3.0),
            ("=MAXIFS(B2:B4,A2:A4,\"<>3/1/2021\")", 4.0),
            ("=DSUM(A1:B4,\"Amount\",A5:A6)", 3.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                criteria_value(&mut engine, formula),
                LiteralValue::Number(expected),
                "{formula}"
            );
        }
    }
}

#[test]
fn date_and_time_text_criteria_compare_serials_to_15_digits() {
    // Midnight ("0:00") is serial 0, noon 0.5, and "12:00:00.5" is half a
    // second after noon; "12/31/1899" is before Excel's date text range, so
    // it stays text. A serial equals a number that agrees to 15 significant
    // digits, as numbers do: "8:30" (0.35416666666666669) is
    // 0.354166666666667, the number "8:30"'s serial reads as in text.
    for config in [EvalConfig::default(), super::common::arrow_eval_config()] {
        let mut engine = Engine::new(TestWorkbook::new(), config);
        let cells = [
            LiteralValue::Text("0:00".into()),
            LiteralValue::Text("12:00".into()),
            LiteralValue::Number(0.5),
            LiteralValue::Text("12:00:00.5".into()),
            LiteralValue::Number(0.0),
            LiteralValue::Text("12/31/1899".into()),
            LiteralValue::Text("8:30".into()),
        ];
        for (i, cell) in cells.into_iter().enumerate() {
            engine
                .set_cell_value("Sheet1", i as u32 + 1, 1, cell)
                .unwrap();
        }
        let cases = [
            ("=COUNTIF(A1,1E-13)", 0.0),
            ("=COUNTIF(A1,\"<>1E-13\")", 1.0),
            ("=COUNTIF(A1,0)", 1.0),
            ("=COUNTIF(A2,0.5000000000005)", 0.0),
            ("=COUNTIF(A2,0.5)", 1.0),
            ("=COUNTIF(A3,\"12:00:00.5\")", 0.0),
            ("=COUNTIF(A3,\"<12:00:00.5\")", 1.0),
            ("=COUNTIF(A3,\"12:00:00\")", 1.0),
            ("=COUNTIF(A4,0.5)", 0.0),
            ("=COUNTIF(A4,\"12:00:00.5\")", 1.0),
            ("=COUNTIF(A5,\"12/31/1899\")", 0.0),
            ("=COUNTIF(A6,0)", 0.0),
            ("=COUNTIF(A6,\"12/31/1899\")", 1.0),
            ("=COUNTIF(A1:A6,0)", 2.0),
            ("=COUNTIF(A7,0.354166666666667)", 1.0),
            ("=COUNTIF(A7,\"<>0.354166666666667\")", 0.0),
            ("=COUNTIF(A7,0.35416666666667)", 0.0),
        ];
        for (formula, expected) in cases {
            assert_eq!(
                criteria_value(&mut engine, formula),
                LiteralValue::Number(expected),
                "{formula}"
            );
        }
    }
}
