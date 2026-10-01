use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ASTNode, ASTNodeType, parse};

fn set_formula(engine: &mut Engine<TestWorkbook>, row: u32, col: u32, formula: &str) {
    engine
        .set_cell_formula("Sheet1", row, col, parse(formula).expect("parse formula"))
        .expect("set formula");
}

fn set_value(engine: &mut Engine<TestWorkbook>, row: u32, col: u32, value: LiteralValue) {
    engine
        .set_cell_value("Sheet1", row, col, value)
        .expect("set value");
}

fn assert_text(engine: &Engine<TestWorkbook>, row: u32, col: u32, expected: &str) {
    assert_eq!(
        engine.get_cell_value("Sheet1", row, col),
        Some(LiteralValue::Text(expected.into()))
    );
}

#[test]
fn concat_and_textjoin_expand_ranges_and_computed_arrays_in_formulas() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, col, text) in [(1, 1, "a"), (1, 2, "b"), (2, 1, "c"), (2, 2, "d")] {
        set_value(&mut engine, row, col, LiteralValue::Text(text.into()));
    }

    set_formula(&mut engine, 1, 4, "=CONCAT(A1:B2)");
    set_formula(&mut engine, 2, 4, "=TEXTJOIN(\"|\",TRUE,A1:B2)");
    set_formula(&mut engine, 3, 4, "=CONCAT(SEQUENCE(2,3))");
    set_formula(&mut engine, 4, 4, "=TEXTJOIN(\"-\",TRUE,SEQUENCE(2,2))");
    set_formula(&mut engine, 5, 4, "=CONCAT(OFFSET(A1,0,0,2,2))");
    set_formula(
        &mut engine,
        6,
        4,
        "=TEXTJOIN(\"|\",TRUE,INDIRECT(\"A1:B2\"))",
    );
    set_formula(&mut engine, 7, 4, "=CONCAT(CHOOSE(1,A1:B2,C1:C2))");
    set_formula(&mut engine, 8, 4, "=CONCAT(OFFSET(A1,-1,0))");
    set_formula(
        &mut engine,
        9,
        4,
        "=TEXTJOIN(\",\",TRUE,INDIRECT(\"not a reference\"))",
    );

    engine.evaluate_all().expect("evaluate formulas");

    assert_text(&engine, 1, 4, "abcd");
    assert_text(&engine, 2, 4, "a|b|c|d");
    assert_text(&engine, 3, 4, "123456");
    assert_text(&engine, 4, 4, "1-2-3-4");
    assert_text(&engine, 5, 4, "abcd");
    assert_text(&engine, 6, 4, "a|b|c|d");
    assert_text(&engine, 7, 4, "abcd");
    for (row, kind) in [(8, ExcelErrorKind::Ref), (9, ExcelErrorKind::Ref)] {
        assert!(matches!(
            engine.get_cell_value("Sheet1", row, 4),
            Some(LiteralValue::Error(error)) if error.kind == kind && error.message.is_none()
        ));
    }
}

#[test]
fn concatenate_lifts_over_arena_literal_and_computed_arrays() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (col, rows) in [
        (
            1,
            vec![
                vec![LiteralValue::Text("top".into()), LiteralValue::Int(2)],
                vec![LiteralValue::Int(3), LiteralValue::Int(4)],
            ],
        ),
        (4, Vec::new()),
    ] {
        let formula = ASTNode::new(
            ASTNodeType::Function {
                name: "CONCATENATE".into(),
                args: vec![
                    ASTNode::new(ASTNodeType::Literal(LiteralValue::Array(rows)), None),
                    ASTNode::new(ASTNodeType::Literal(LiteralValue::Text("!".into())), None),
                ],
            },
            None,
        );
        engine
            .set_cell_formula("Sheet1", 1, col, formula)
            .expect("set arena literal formula");
    }
    set_formula(&mut engine, 1, 6, "=CONCATENATE(SEQUENCE(2,2),\"!\")");

    engine.evaluate_all().expect("evaluate formulas");

    // Each element is concatenated and the results spill.
    assert_text(&engine, 1, 1, "top!");
    assert_text(&engine, 1, 2, "2!");
    assert_text(&engine, 2, 1, "3!");
    assert_text(&engine, 2, 2, "4!");
    assert_text(&engine, 1, 4, "!");
    assert_text(&engine, 1, 6, "1!");
    assert_text(&engine, 2, 7, "4!");
}

#[test]
fn textjoin_formula_range_blanks_obey_ignore_empty() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    set_value(&mut engine, 1, 1, LiteralValue::Text("a".into()));
    set_value(&mut engine, 1, 3, LiteralValue::Text(String::new()));
    set_value(&mut engine, 1, 4, LiteralValue::Text("d".into()));
    set_formula(&mut engine, 1, 6, "=TEXTJOIN(\"-\",TRUE,A1:D1)");
    set_formula(&mut engine, 2, 6, "=TEXTJOIN(\"-\",FALSE,A1:D1)");

    engine.evaluate_all().expect("evaluate formulas");

    assert_text(&engine, 1, 6, "a-d");
    assert_text(&engine, 2, 6, "a---d");
}

#[test]
fn textjoin_omitted_ignore_empty_skips_empty_values() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    set_value(&mut engine, 1, 1, LiteralValue::Text("a".into()));
    set_value(&mut engine, 1, 3, LiteralValue::Text(String::new()));
    set_value(&mut engine, 1, 4, LiteralValue::Text("d".into()));
    set_formula(&mut engine, 1, 6, "=TEXTJOIN(\"-\",,A1:D1)");
    set_formula(
        &mut engine,
        2,
        6,
        "=TEXTJOIN(\",\",,IF({1,0,1,0},{\"x\",\"y\",\"z\",\"w\"},\"\"))",
    );
    // An explicit FALSE or 0, or a blank cell as the flag, keeps them.
    set_formula(&mut engine, 3, 6, "=TEXTJOIN(\"-\",0,A1:D1)");
    set_formula(&mut engine, 4, 6, "=TEXTJOIN(\"-\",H1,A1:D1)");

    engine.evaluate_all().expect("evaluate formulas");

    assert_text(&engine, 1, 6, "a-d");
    assert_text(&engine, 2, 6, "x,z");
    assert_text(&engine, 3, 6, "a---d");
    assert_text(&engine, 4, 6, "a---d");
}

#[test]
fn textjoin_uses_range_and_array_delimiters_in_turn() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // Microsoft's Example 3: a row of delimiters joins a table, commas
    // between the fields of a row and a semicolon between rows.
    for (row, values) in [
        (1, ["Tulsa", "OK", "74133", "US"]),
        (2, ["Seattle", "WA", "98109", "US"]),
        (3, ["end", "", "", ""]),
        (4, [",", ",", ",", ";"]),
    ] {
        for (col, value) in (1..).zip(values) {
            if !value.is_empty() {
                set_value(&mut engine, row, col, LiteralValue::Text(value.into()));
            }
        }
    }
    set_formula(&mut engine, 1, 6, "=TEXTJOIN(A4:D4,TRUE,A1:D3)");
    set_formula(
        &mut engine,
        2,
        6,
        "=TEXTJOIN({\"-\",\"+\"},TRUE,\"a\",\"b\",\"c\")",
    );
    // They start over when they run out, and carry on across arguments.
    set_formula(
        &mut engine,
        3,
        6,
        "=TEXTJOIN({\"-\",\"+\"},TRUE,A1:B1,\"c\",\"d\",\"e\")",
    );
    // A delimiter is used up only when it is placed: skipped empty values
    // take none, kept ones do.
    set_formula(
        &mut engine,
        4,
        6,
        "=TEXTJOIN({\"1\",\"2\",\"3\"},TRUE,\"a\",\"\",\"b\",\"c\")",
    );
    set_formula(
        &mut engine,
        5,
        6,
        "=TEXTJOIN({\"1\",\"2\",\"3\"},FALSE,\"a\",\"\",\"b\",\"c\")",
    );
    // A computed array; numbers and logicals are their text.
    set_formula(
        &mut engine,
        6,
        6,
        "=TEXTJOIN(IF({1,0},\"-\",\"+\"),TRUE,\"a\",\"b\",\"c\")",
    );
    set_formula(
        &mut engine,
        7,
        6,
        "=TEXTJOIN({0,TRUE},TRUE,\"a\",\"b\",\"c\")",
    );
    // An empty delimiter slot joins with nothing.
    set_formula(&mut engine, 8, 6, "=TEXTJOIN(,TRUE,\"a\",\"b\")");
    set_formula(
        &mut engine,
        9,
        6,
        "=TEXTJOIN({\"-\",#N/A},TRUE,\"a\",\"b\",\"c\")",
    );

    engine.evaluate_all().expect("evaluate formulas");

    assert_text(&engine, 1, 6, "Tulsa,OK,74133,US;Seattle,WA,98109,US;end");
    assert_text(&engine, 2, 6, "a-b+c");
    assert_text(&engine, 3, 6, "Tulsa-OK+c-d+e");
    assert_text(&engine, 4, 6, "a1b2c");
    assert_text(&engine, 5, 6, "a12b3c");
    assert_text(&engine, 6, 6, "a-b+c");
    assert_text(&engine, 7, 6, "a0bTRUEc");
    assert_text(&engine, 8, 6, "ab");
    match engine.get_cell_value("Sheet1", 9, 6) {
        Some(LiteralValue::Error(error)) => assert_eq!(error.kind, ExcelErrorKind::Na),
        other => panic!("expected #N/A at F9, got {other:?}"),
    }
}

#[test]
fn textjoin_delimiter_range_reads_row_major_and_counts_blank_cells() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for (row, col, text) in [(1, 1, "1"), (1, 2, "2"), (2, 1, "3"), (2, 2, "4")] {
        set_value(&mut engine, row, col, LiteralValue::Text(text.into()));
    }
    set_formula(
        &mut engine,
        1,
        4,
        "=TEXTJOIN(A1:B2,TRUE,\"a\",\"b\",\"c\",\"d\",\"e\",\"f\")",
    );
    // A1:A3 ends below the last used row: its blank A3 is an empty
    // delimiter, so the cycle is "1", "3", "".
    set_formula(
        &mut engine,
        2,
        4,
        "=TEXTJOIN(A1:A3,TRUE,\"a\",\"b\",\"c\",\"d\",\"e\")",
    );

    engine.evaluate_all().expect("evaluate formulas");

    assert_text(&engine, 1, 4, "a1b2c3d4e1f");
    assert_text(&engine, 2, 4, "a1b3cd1e");
}

#[test]
fn expanded_formula_range_propagates_later_error_and_concatenate_stays_scalar() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    set_value(&mut engine, 1, 1, LiteralValue::Text("first".into()));
    set_formula(&mut engine, 1, 2, "=1/0");
    set_value(&mut engine, 1, 3, LiteralValue::Text("last".into()));
    set_formula(&mut engine, 1, 5, "=CONCAT(A1:C1)");
    set_formula(&mut engine, 2, 5, "=TEXTJOIN(\",\",TRUE,A1:C1)");
    set_formula(&mut engine, 3, 5, "=CONCATENATE(A1:C1,\"!\")");

    engine.evaluate_all().expect("evaluate formulas");

    for row in [1, 2] {
        match engine.get_cell_value("Sheet1", row, 5) {
            Some(LiteralValue::Error(error)) => assert_eq!(error.kind, ExcelErrorKind::Div),
            other => panic!("expected #DIV/0! at E{row}, got {other:?}"),
        }
    }
    assert_text(&engine, 3, 5, "first!");
}

#[test]
fn numbers_become_text_with_15_significant_digits() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    // A date-time serial loaded from a date-formatted cell.
    set_value(&mut engine, 1, 1, LiteralValue::Number(44468.756944444445));
    let cases = [
        ("=A1&\"\"", "44468.7569444444"),
        ("=RIGHT(A1,8)", "69444444"),
        ("=LEFT(1/3,5)", "0.333"),
        ("=MID(2/3,13,5)", "66667"),
        ("=CONCAT(0.1+0.2,\"|\",1/3)", "0.3|0.333333333333333"),
        ("=CONCATENATE(-1/3)", "-0.333333333333333"),
        ("=TEXTJOIN(\",\",TRUE,1/7,2.5)", "0.142857142857143,2.5"),
        ("=SUBSTITUTE(1/3,\"3\",\"x\")", "0.xxxxxxxxxxxxxxx"),
        ("=2^53&\"\"", "9007199254740990"),
        ("=1.2345678901234567E+19&\"\"", "12345678901234600000"),
        ("=1.2345678901234568E+20&\"\"", "1.23456789012346E+20"),
        ("=1.23456789E-5&\"\"", "0.0000123456789"),
        ("=1.2345678901234568E-5&\"\"", "1.23456789012346E-05"),
        ("=2E-50&\"\"", "2E-50"),
        // Unchanged: short numbers keep their plain digits.
        ("=123.45&\"\"", "123.45"),
        ("=-5&\"\"", "-5"),
        ("=0.5&\"\"", "0.5"),
        ("=TEXT(1/3,\"General\")", "0.333333333"),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        set_formula(&mut engine, row as u32 + 1, 3, formula);
    }
    set_formula(&mut engine, 1, 4, "=LEN(2/3)");

    engine.evaluate_all().expect("evaluate formulas");

    for (row, (formula, expected)) in cases.iter().enumerate() {
        assert_eq!(
            engine.get_cell_value("Sheet1", row as u32 + 1, 3),
            Some(LiteralValue::Text((*expected).into())),
            "{formula}"
        );
    }
    assert_eq!(
        engine.get_cell_value("Sheet1", 1, 4),
        Some(LiteralValue::Number(17.0))
    );
}

#[test]
fn numbers_become_text_rounding_ties_away_from_zero() {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    set_value(&mut engine, 1, 1, LiteralValue::Number(100000000000000.5));
    let cases = [
        ("=1234567890123.125&\"\"", "1234567890123.13"),
        ("=A1&\"\"", "100000000000001"),
        ("=-A1&\"\"", "-100000000000001"),
        ("=(10^15+5)&\"\"", "1000000000000010"),
        ("=CONCAT(70489670895608.25)", "70489670895608.3"),
        (
            "=TEXTJOIN(\",\",TRUE,999999999999999.5,13/2^20)",
            "1000000000000000,1.23977661132813E-05",
        ),
        ("=RIGHT(A1,3)", "001"),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        set_formula(&mut engine, row as u32 + 1, 3, formula);
    }

    engine.evaluate_all().expect("evaluate formulas");

    for (row, (formula, expected)) in cases.iter().enumerate() {
        assert_eq!(
            engine.get_cell_value("Sheet1", row as u32 + 1, 3),
            Some(LiteralValue::Text((*expected).into())),
            "{formula}"
        );
    }
}
