//! Caller-context functions as Excel for Windows 16.0.20430 computes them,
//! every row an Excel reading from probes 1-4 of
//! ops/excel-context-probe-20261006.md: INDIRECT with R1C1 text and the
//! spaces Excel accepts, OFFSET's sizes, CELL's workbook info types,
//! RANDBETWEEN's and RANDARRAY's arguments and draws. Each formula is in F1 of
//! a sheet holding the probe's setup cells and the recorder's Z1 canary.

use crate::engine::{Engine, EvalConfig, TemporalEgress};
use crate::test_workbook::TestWorkbook;
use ExcelErrorKind::{Calc, Div, Na, Num, Ref, Value};
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// The setup cells of a probe: none, probe 1's, or probes 2-4's.
#[derive(Clone, Copy)]
enum Setup {
    S0,
    S1,
    S2,
}
use Setup::{S0, S1, S2};

fn engine(setup: Setup) -> Engine<TestWorkbook> {
    // Dates leave as serial numbers, as the cache writer reads them.
    let config = EvalConfig {
        temporal_egress: TemporalEgress::Serial,
        ..EvalConfig::default()
    };
    let mut engine = Engine::new(TestWorkbook::new(), config);
    let formula = |engine: &mut Engine<TestWorkbook>, row, col, text: &str| {
        engine
            .set_cell_formula("Sheet1", row, col, parse(text).unwrap())
            .unwrap();
    };
    formula(&mut engine, 1, 26, "=1111+2222");
    let mut value = |row, col, value| engine.set_cell_value("Sheet1", row, col, value).unwrap();
    if !matches!(setup, S0) {
        value(1, 1, n(1.0));
        value(1, 2, n(2.0));
        value(2, 1, n(10.0));
        value(2, 2, n(20.0));
        value(3, 3, n(300.0));
        value(4, 4, n(4000.0));
        value(3, 1, text("t"));
    }
    if matches!(setup, S2) {
        value(1, 5, b(true));
        formula(&mut engine, 1, 3, "=\"\"");
        formula(&mut engine, 1, 4, "=1/0");
    }
    engine
}

/// The value of `formula` in F1 (row `row`, column F).
fn eval_at(setup: Setup, row: u32, formula: &str) -> LiteralValue {
    let mut engine = engine(setup);
    engine
        .set_cell_formula("Sheet1", row, 6, parse(formula).unwrap())
        .unwrap_or_else(|e| panic!("{formula}: {e:?}"));
    engine.evaluate_all().unwrap();
    engine
        .get_cell_value("Sheet1", row, 6)
        .unwrap_or(LiteralValue::Empty)
}

fn n(n: f64) -> LiteralValue {
    LiteralValue::Number(n)
}

fn b(flag: bool) -> LiteralValue {
    LiteralValue::Boolean(flag)
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
        (LiteralValue::Int(a), LiteralValue::Number(b)) => *a as f64 == *b,
        (LiteralValue::Error(a), LiteralValue::Error(b)) => a.kind == b.kind,
        // A formula whose result is a blank cell's value caches 0.
        (LiteralValue::Empty, LiteralValue::Number(b)) => *b == 0.0,
        _ => actual == expected,
    }
}

fn assert_cases(cases: &[(Setup, &str, LiteralValue)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(setup, formula, expected)| {
            let actual = eval_at(*setup, 1, formula);
            (!same(&actual, expected)).then(|| format!("{formula}: {actual:?}, not {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `formula` in F1 spills exactly `expected`.
fn assert_spill(setup: Setup, formula: &str, expected: &[&[LiteralValue]]) {
    let mut engine = engine(setup);
    engine
        .set_cell_formula("Sheet1", 1, 6, parse(formula).unwrap())
        .unwrap_or_else(|e| panic!("{formula}: {e:?}"));
    engine.evaluate_all().unwrap();
    let at = |row: u32, col: u32| {
        engine
            .get_cell_value("Sheet1", row, col)
            .unwrap_or(LiteralValue::Empty)
    };
    let (rows, cols) = (expected.len() as u32, expected[0].len() as u32);
    for (r, row) in expected.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            let actual = at(r as u32 + 1, c as u32 + 6);
            assert!(
                same(&actual, value),
                "{formula} at row {} column {}: {actual:?}, not {value:?}",
                r + 1,
                c + 1,
            );
        }
    }
    let blank = |v: LiteralValue| matches!(v, LiteralValue::Empty);
    assert!(
        blank(at(rows + 1, 6)) && blank(at(1, 6 + cols)),
        "{formula}: the result is larger than {rows}x{cols}"
    );
}

/// A relative R1C1 row wraps from the formula's own row: from F5,
/// R[1048575] is row 4 (CTX2_IND_lower_wrap5).
#[test]
fn relative_r1c1_wraps_from_the_formula_row() {
    assert!(same(
        &eval_at(S2, 5, r#"=ROW(INDIRECT("R[1048575]C",FALSE))"#),
        &n(4.0)
    ));
}

#[test]
fn cell_as_excel_computes_it() {
    assert_cases(&[
        (S1, r##"=CELL("row",C3)"##, n(3.0)),           // CTX1_CELL_row
        (S1, r##"=CELL("col",C3)"##, n(3.0)),           // CTX1_CELL_col
        (S1, r##"=CELL("address",C3)"##, text("$C$3")), // CTX1_CELL_address
        (S1, r##"=CELL("address",Sheet1!C3)"##, text("$C$3")), // CTX1_CELL_address_sheet
        (S1, r##"=CELL("ROW",C3)"##, n(3.0)),           // CTX1_CELL_ROW_upper
        (S1, r##"=CELL("row",B2:C3)"##, n(2.0)),        // CTX1_CELL_row_range
        (S1, r##"=CELL("contents",B2)"##, n(20.0)),     // CTX1_CELL_contents_num
        (S1, r##"=CELL("contents",A3)"##, text("t")),   // CTX1_CELL_contents_text
        (S1, r##"=CELL("contents",Z99)"##, n(0.0)),     // CTX1_CELL_contents_blank
        (S1, r##"=ISBLANK(CELL("contents",Z99))"##, b(true)), // CTX1_CELL_isblank_contents
        (S1, r##"=CELL("type",Z99)"##, text("b")),      // CTX1_CELL_type_blank
        (S1, r##"=CELL("type",A3)"##, text("l")),       // CTX1_CELL_type_text
        (S1, r##"=CELL("type",A1)"##, text("v")),       // CTX1_CELL_type_num
        (S1, r##"=CELL(" row",A1)"##, error(Value)),    // CTX1_CELL_space_type
        (S1, r##"=CELL("rows",A1)"##, error(Value)),    // CTX1_CELL_bad_type
        (S1, r##"=CELL("row",A:A)"##, n(1.0)),          // CTX1_CELL_row_col_A
        (S1, r##"=CELL("col",3:3)"##, n(1.0)),          // CTX1_CELL_col_row_3
        (S1, r##"=CELL("row",OFFSET(A1,4,0))"##, n(5.0)), // CTX1_CELL_row_offset
        (S1, r##"=CELL("contents",A1:B2)"##, n(1.0)),   // CTX1_CELL_contents_range
        (S1, r##"=CELL(A3,A1)"##, error(Value)),        // CTX1_CELL_type_from_cell
        (S1, r##"=CELL("contents",Z1)"##, n(3333.0)),   // CTX1_CELL_contents_formula
        (S1, r##"=CELL("type",Z2)"##, text("b")),       // CTX1_CELL_type_err_cell
        (S1, r##"=CELL("address",4:4)"##, text("$A$4")), // CTX1_CELL_address_whole_row
        (S1, r##"=CELL("address",D:D)"##, text("$D$1")), // CTX1_CELL_address_whole_col
        (S2, r##"=CELL("contents",Z99)&"x""##, text("x")), // CTX2_CELL_contents_concat
        (S2, r##"=ISNUMBER(CELL("contents",Z99))"##, b(false)), // CTX2_CELL_isnumber_contents
        (S2, r##"=CELL("contents",Z99)+1"##, n(1.0)),   // CTX2_CELL_contents_plus
        (S2, r##"=CELL("type",E1)"##, text("v")),       // CTX2_CELL_type_bool
        (S2, r##"=CELL("contents",E1)"##, b(true)),     // CTX2_CELL_contents_bool
        (S2, r##"=CELL("type",C1)"##, text("l")),       // CTX2_CELL_type_empty_text
        (S2, r##"=CELL("contents",C1)"##, text("")),    // CTX2_CELL_contents_empty_text
        (S2, r##"=CELL("type",D1)"##, text("v")),       // CTX2_CELL_type_err
        (S2, r##"=CELL("contents",D1)"##, error(Div)),  // CTX2_CELL_contents_err
        (S2, r##"=CELL("col",XFD1)"##, n(16384.0)),     // CTX2_CELL_col_xfd
        (S2, r##"=CELL("Row",C3)"##, n(3.0)),           // CTX2_CELL_Row_mixed
        (S2, r##"=CELL("row ",C3)"##, error(Value)),    // CTX2_CELL_row_trail
        (
            S2,
            r##"=CELL("address",INDIRECT("Sheet1!B2"))"##,
            text("$B$2"),
        ), // CTX2_CELL_address_indirect
        (S2, r##"=CELL("contents",A1:A3)"##, n(1.0)),   // CTX2_CELL_contents_spill
        (S2, r##"=CELL("type",G9)"##, text("b")),       // CTX2_CELL_type_formula_blank
        (S2, r##"=CELL("ADDRESS",b2)"##, text("$B$2")), // CTX2_CELL_address_lower
    ]);
}

#[test]
fn indirect_as_excel_computes_it() {
    assert_cases(&[
        (S1, r##"=INDIRECT("R1C1",FALSE)"##, n(1.0)), // CTX1_IND_abs_R1C1
        (S1, r##"=INDIRECT("R2C2",FALSE)"##, n(20.0)), // CTX1_IND_abs_R2C2
        (S1, r##"=INDIRECT("r2c2",FALSE)"##, n(20.0)), // CTX1_IND_lower_r2c2
        (S1, r##"=INDIRECT("R[2]C[-3]",FALSE)"##, n(300.0)), // CTX1_IND_rel_R2_Cm3
        (S1, r##"=INDIRECT("R[1]C[-5]",FALSE)"##, n(10.0)), // CTX1_IND_rel_R1_Cm5
        (S1, r##"=INDIRECT("R[-1]C",FALSE)"##, n(0.0)), // CTX1_IND_rel_Rm1_C
        (S1, r##"=INDIRECT("RC[-6]",FALSE)"##, n(0.0)), // CTX1_IND_rel_R_Cm6
        (S1, r##"=ROW(INDIRECT("RC",FALSE))"##, n(1.0)), // CTX1_IND_row_RC_self
        (S1, r##"=COLUMN(INDIRECT("RC[-2]",FALSE))"##, n(4.0)), // CTX1_IND_col_RCm2
        (S1, r##"=SUM(INDIRECT("R1C1:R2C2",FALSE))"##, n(33.0)), // CTX1_IND_sum_R1C1_R2C2
        (S1, r##"=SUM(INDIRECT("R1C1:R[1]C[-4]",FALSE))"##, n(33.0)), // CTX1_IND_sum_mixed_rel_end
        (S1, r##"=SUM(INDIRECT("R2C2:R1C1",FALSE))"##, n(33.0)), // CTX1_IND_sum_reversed
        (S1, r##"=ROWS(INDIRECT("R2",FALSE))"##, n(1.0)), // CTX1_IND_rows_R2
        (S1, r##"=COLUMNS(INDIRECT("R2",FALSE))"##, n(16384.0)), // CTX1_IND_cols_R2
        (S1, r##"=SUM(INDIRECT("R2",FALSE))"##, n(30.0)), // CTX1_IND_sum_R2
        (S1, r##"=SUM(INDIRECT("C2",FALSE))"##, n(22.0)), // CTX1_IND_sum_C2
        (S1, r##"=ROWS(INDIRECT("C2",FALSE))"##, n(1048576.0)), // CTX1_IND_rows_C2
        (S1, r##"=SUM(INDIRECT("C1:C2",FALSE))"##, n(33.0)), // CTX1_IND_sum_C1_C2
        (S1, r##"=SUM(INDIRECT("R[1]",FALSE))"##, n(30.0)), // CTX1_IND_sum_Rrel1
        (S1, r##"=SUM(INDIRECT("C[-5]",FALSE))"##, n(11.0)), // CTX1_IND_sum_Crelm5
        (S1, r##"=ROW(INDIRECT("R",FALSE))"##, n(1.0)), // CTX1_IND_row_R
        (S1, r##"=COLUMNS(INDIRECT("R",FALSE))"##, n(16384.0)), // CTX1_IND_cols_R
        (S1, r##"=COLUMN(INDIRECT("C",FALSE))"##, n(6.0)), // CTX1_IND_col_C
        (S1, r##"=ROWS(INDIRECT("C",FALSE))"##, n(1048576.0)), // CTX1_IND_rows_C
        (S1, r##"=INDIRECT("Sheet1!R2C2",FALSE)"##, n(20.0)), // CTX1_IND_sheet_R2C2
        (S1, r##"=INDIRECT("'Sheet1'!R2C2",FALSE)"##, n(20.0)), // CTX1_IND_qsheet_R2C2
        (S1, r##"=INDIRECT("sheet1!r2c2",FALSE)"##, n(20.0)), // CTX1_IND_lsheet_r2c2
        (S1, r##"=INDIRECT("Nope!R2C2",FALSE)"##, error(Ref)), // CTX1_IND_badsheet
        (S1, r##"=INDIRECT("R0C1",FALSE)"##, error(Ref)), // CTX1_IND_R0C1
        (S1, r##"=INDIRECT("R1048577C1",FALSE)"##, error(Ref)), // CTX1_IND_R1048577C1
        (S1, r##"=ROW(INDIRECT("R1048576C1",FALSE))"##, n(1048576.0)), // CTX1_IND_row_R1048576C1
        (S1, r##"=INDIRECT("R1C16385",FALSE)"##, error(Ref)), // CTX1_IND_R1C16385
        (S1, r##"=COLUMN(INDIRECT("R1C16384",FALSE))"##, n(16384.0)), // CTX1_IND_col_R1C16384
        (S1, r##"=INDIRECT("A1",FALSE)"##, error(Ref)), // CTX1_IND_A1_false
        (S1, r##"=INDIRECT("R1C1",TRUE)"##, error(Ref)), // CTX1_IND_R1C1_true
        (S1, r##"=INDIRECT("R1C1:B2",FALSE)"##, error(Ref)), // CTX1_IND_mixed_styles
        (S1, r##"=INDIRECT(" R1C1",FALSE)"##, error(Ref)), // CTX1_IND_lead_space
        (S1, r##"=INDIRECT("R1C1 ",FALSE)"##, n(1.0)), // CTX1_IND_trail_space
        (S1, r##"=INDIRECT("R[+1]C[-5]",FALSE)"##, n(10.0)), // CTX1_IND_plus_sign
        (S1, r##"=INDIRECT("R01C01",FALSE)"##, n(1.0)), // CTX1_IND_leading_zero
        (S1, r##"=INDIRECT("R[1.5]C",FALSE)"##, error(Ref)), // CTX1_IND_frac_offset
        (S1, r##"=INDIRECT("R[]C[-5]",FALSE)"##, n(1.0)), // CTX1_IND_empty_brackets
        (S1, r##"=INDIRECT("R[ 1]C[-5]",FALSE)"##, error(Ref)), // CTX1_IND_space_in_bracket
        (S1, r##"=INDIRECT("C1R1",FALSE)"##, error(Ref)), // CTX1_IND_C1R1
        (S1, r##"=ROWS(INDIRECT("R1C1:R2",FALSE))"##, error(Ref)), // CTX1_IND_cell_to_row
        (S1, r##"=ROWS(INDIRECT("R1:C1",FALSE))"##, error(Ref)), // CTX1_IND_row_to_col
        (S1, r##"=INDIRECT("R1C1",0)"##, n(1.0)),     // CTX1_IND_flag_zero
        (S1, r##"=INDIRECT("R1C1","FALSE")"##, n(1.0)), // CTX1_IND_flag_text
        (S1, r##"=INDIRECT("R1C1",Z9)"##, n(1.0)),    // CTX1_IND_flag_blank
        (S1, r##"=INDIRECT("$A$1",FALSE)"##, error(Ref)), // CTX1_IND_dollar_A1
        (S1, r##"=INDIRECT("R-1C1",FALSE)"##, error(Ref)), // CTX1_IND_R_minus1
        (S1, r##"=ROW(INDIRECT("R[1048575]C",FALSE))"##, n(1048576.0)), // CTX1_IND_row_far_rel
        (S1, r##"=INDIRECT("R[1048576]C",FALSE)"##, error(Ref)), // CTX1_IND_past_rel
        (S1, r##"=ROW(INDIRECT("R[-1]C",FALSE))"##, n(1048576.0)), // CTX1_IND_row_wrap_up
        (S1, r##"=COLUMN(INDIRECT("RC[-6]",FALSE))"##, n(16384.0)), // CTX1_IND_col_wrap_left
        (S1, r##"=INDIRECT("R1C1:R2C2:R3C3",FALSE)"##, error(Ref)), // CTX1_IND_three_parts
        (S1, r##"=INDIRECT("R1C1:R1C1",FALSE)"##, n(1.0)), // CTX1_IND_one_cell_range
        (
            S1,
            r##"=SUM(INDIRECT("R[0]C[-5]:R[2]C[-4]",FALSE))"##,
            n(33.0),
        ), // CTX1_IND_rel_range
        (S1, r##"=COLUMN(INDIRECT("R3C",FALSE))"##, n(6.0)), // CTX1_IND_col_R3C
        (S1, r##"=ROW(INDIRECT("RC1",FALSE))"##, n(1.0)), // CTX1_IND_row_RC1
        (S1, r##"=COLUMNS(INDIRECT("C1:C[1]",FALSE))"##, n(7.0)), // CTX1_IND_cols_C1_Crel
        (S1, r##"=ROWS(INDIRECT("R2C2:R3",FALSE))"##, error(Ref)), // CTX1_IND_cell_row_mix
        (S1, r##"=INDIRECT("Sheet1:Sheet1!R1C1",FALSE)"##, error(Ref)), // CTX1_IND_threeD
        (
            S1,
            r##"=CELL("address",INDIRECT("R4C4",FALSE))"##,
            text("$D$4"),
        ), // CTX1_IND_addr_R1C1
        (S1, r##"=INDIRECT("r[1]c[-5]",FALSE)"##, n(10.0)), // CTX1_IND_rc_lower_rel
        (S1, r##"=SUM(INDIRECT("R[1]:R[2]",FALSE))"##, n(330.0)), // CTX1_IND_R_bracket_noC
        (S1, r##"=SUM(INDIRECT("C[-5]:C[-4]",FALSE))"##, n(33.0)), // CTX1_IND_C_bracket_range
        (S1, r##"=INDIRECT("",FALSE)"##, error(Ref)), // CTX1_IND_empty_text
        (S2, r##"=ROW(INDIRECT("R[-1048575]C",FALSE))"##, n(2.0)), // CTX2_IND_wrap_up_max
        (S2, r##"=INDIRECT("R[-1048576]C",FALSE)"##, error(Ref)), // CTX2_IND_wrap_up_past
        (S2, r##"=COLUMN(INDIRECT("RC[16383]",FALSE))"##, n(5.0)), // CTX2_IND_wrap_right
        (S2, r##"=INDIRECT("RC[16384]",FALSE)"##, error(Ref)), // CTX2_IND_wrap_right_past
        (S2, r##"=COLUMN(INDIRECT("RC[-16383]",FALSE))"##, n(7.0)), // CTX2_IND_wrap_left_max
        (S2, r##"=INDIRECT("RC[-16384]",FALSE)"##, error(Ref)), // CTX2_IND_wrap_left_past
        (S2, r##"=ROW(INDIRECT("R[-1]",FALSE))"##, n(1048576.0)), // CTX2_IND_wrap_row_whole
        (
            S2,
            r##"=ROWS(INDIRECT("R[-1]C[-5]:R[1]C[-5]",FALSE))"##,
            n(1048575.0),
        ), // CTX2_IND_wrap_range_rows
        (
            S2,
            r##"=SUM(INDIRECT("R[-1]C[-5]:R[1]C[-5]",FALSE))"##,
            n(10.0),
        ), // CTX2_IND_wrap_range_sum
        (S2, r##"=INDIRECT("R1C1  ",FALSE)"##, n(1.0)), // CTX2_IND_two_spaces
        (S2, r##"=SUM(INDIRECT("R1C1:R2C2 ",FALSE))"##, n(33.0)), // CTX2_IND_range_trail
        (S2, r##"=SUM(INDIRECT("R1C1 :R2C2",FALSE))"##, n(33.0)), // CTX2_IND_range_mid_space
        (S2, r##"=SUM(INDIRECT("R1C1: R2C2",FALSE))"##, n(33.0)), // CTX2_IND_range_mid_space2
        (S2, r##"=INDIRECT("Sheet1!R1C1 ",FALSE)"##, n(1.0)), // CTX2_IND_sheet_trail
        (S2, r##"=INDIRECT("Sheet1! R1C1",FALSE)"##, n(1.0)), // CTX2_IND_sheet_space
        (S2, r##"=INDIRECT("r1C1",FALSE)"##, n(1.0)), // CTX2_IND_mixcase
        (S2, r##"=INDIRECT("R[-0]C[-5]",FALSE)"##, n(1.0)), // CTX2_IND_minus_zero
        (S2, r##"=INDIRECT("R[01]C[-05]",FALSE)"##, n(10.0)), // CTX2_IND_bracket_zero_pad
        (S2, r##"=ROWS(INDIRECT("R1048576",FALSE))"##, n(1.0)), // CTX2_IND_row_max
        (S2, r##"=INDIRECT("R1048577",FALSE)"##, error(Ref)), // CTX2_IND_row_past
        (S2, r##"=COLUMNS(INDIRECT("C16384",FALSE))"##, n(1.0)), // CTX2_IND_col_max
        (S2, r##"=INDIRECT("C16385",FALSE)"##, error(Ref)), // CTX2_IND_col_past
        (S2, r##"=INDIRECT("R[1048576]",FALSE)"##, error(Ref)), // CTX2_IND_row_rel_past
        (S2, r##"=COLUMNS(INDIRECT("C:C[1]",FALSE))"##, n(2.0)), // CTX2_IND_col_C_C1
        (S2, r##"=ROWS(INDIRECT("R:R[2]",FALSE))"##, n(3.0)), // CTX2_IND_R_R1
        (
            S2,
            r##"=ROWS(INDIRECT("R1C1:R1048576C16384",FALSE))"##,
            n(1048576.0),
        ), // CTX2_IND_full_grid
        (S2, r##"=INDIRECT("'Sheet1'!R[1]C[-5]",FALSE)"##, n(10.0)), // CTX2_IND_sheet_rel
        (S2, r##"=ROWS(INDIRECT("Sheet1!R1",FALSE))"##, n(1.0)), // CTX2_IND_sheet_row
        (
            S2,
            r##"=SUM(INDIRECT("Sheet1!R1C1:Sheet1!R2C2",FALSE))"##,
            error(Ref),
        ), // CTX2_IND_sheet_both
        (S2, r##"=SUM(INDIRECT("Sheet1!A1:Sheet1!B2"))"##, error(Ref)), // CTX2_IND_a1_sheet_both
        (S2, r##"=INDIRECT("A1 ")"##, n(1.0)),        // CTX2_IND_a1_trail
        (S2, r##"=INDIRECT(" A1")"##, error(Ref)),    // CTX2_IND_a1_lead
        (S2, r##"=SUM(INDIRECT("A1:B2 "))"##, n(33.0)), // CTX2_IND_a1_range_trail
        (S2, r##"=INDIRECT("Sheet1!A1 ")"##, n(1.0)), // CTX2_IND_a1_sheet_trail
        (S2, r##"=INDIRECT("R0000001C01",FALSE)"##, n(1.0)), // CTX2_IND_long_digits
        (S2, r##"=ROWS(INDIRECT("R[]",FALSE))"##, n(1.0)), // CTX2_IND_bracket_only_R
        (S2, r##"=INDIRECT("R1C1"&CHAR(9),FALSE)"##, error(Ref)), // CTX2_IND_tab_trail
        (S2, r##"=INDIRECT("R1C1"&CHAR(160),FALSE)"##, error(Ref)), // CTX2_IND_nbsp_trail
        (S2, r##"=INDIRECT("RR1C1",FALSE)"##, error(Ref)), // CTX2_IND_double_R
        (S2, r##"=INDIRECT("R [1]C",FALSE)"##, error(Ref)), // CTX2_IND_R_then_bracket_space
        (S2, r##"=INDIRECT("R[+ 1]C",FALSE)"##, error(Ref)), // CTX2_IND_bracket_plus_space
        (S2, r##"=INDIRECT("R[--1]C",FALSE)"##, error(Ref)), // CTX2_IND_bracket_double_minus
        (S2, r##"=INDIRECT("R1C1",FALSE)+INDIRECT("A2")"##, n(11.0)), // CTX2_IND_name_r1c1
        (S2, r##"=INDIRECT("C1R1",FALSE)"##, error(Ref)), // CTX2_IND_C_then_R
        (S2, r##"=SUM(INDIRECT("R1C1 : R2C2",FALSE))"##, n(33.0)), // CTX3_IND_r_sp_colon_sp
        (S2, r##"=INDIRECT("Sheet1 !R1C1",FALSE)"##, error(Ref)), // CTX3_IND_r_sheet_sp_bang
        (S2, r##"=INDIRECT("'Sheet1' !R1C1",FALSE)"##, error(Ref)), // CTX3_IND_r_qsheet_sp_bang
        (S2, r##"=INDIRECT("R1 C1",FALSE)"##, error(Ref)), // CTX3_IND_r_sp_inside
        (S2, r##"=INDIRECT("R[1] C[-5]",FALSE)"##, error(Ref)), // CTX3_IND_r_sp_inside_rel
        (S2, r##"=INDIRECT(" Sheet1!R1C1",FALSE)"##, error(Ref)), // CTX3_IND_r_lead_sheet
        (S2, r##"=INDIRECT("Sheet1!  R1C1",FALSE)"##, n(1.0)), // CTX3_IND_r_bang_two_sp
        (S2, r##"=SUM(INDIRECT("R1C1:  R2C2",FALSE))"##, n(33.0)), // CTX3_IND_r_colon_two_sp
        (S2, r##"=SUM(INDIRECT("R1 : R2",FALSE))"##, error(Ref)), // CTX3_IND_r_rows_sp
        (S2, r##"=INDIRECT("R[1 ]C[-5]",FALSE)"##, error(Ref)), // CTX3_IND_r_sp_bracket_end
        (
            S2,
            r##"=SUM(INDIRECT("R1C1"&CHAR(9)&":R2C2",FALSE))"##,
            error(Ref),
        ), // CTX3_IND_r_tab_colon
        (S2, r##"=INDIRECT("R 1C1",FALSE)"##, error(Ref)), // CTX3_IND_r_sp_after_R
        (S2, r##"=INDIRECT("R1 C[-5]",FALSE)"##, error(Ref)), // CTX3_IND_r_sp_before_C
        (S2, r##"=INDIRECT("Sheet1!",FALSE)"##, error(Ref)), // CTX3_IND_r_empty_after_bang
        (S2, r##"=INDIRECT(":",FALSE)"##, error(Ref)), // CTX3_IND_r_colon_only
        (S2, r##"=INDIRECT("R1C1:",FALSE)"##, error(Ref)), // CTX3_IND_r_trailing_colon
        (S2, r##"=ROWS(INDIRECT("Sheet1!R1:R2",FALSE))"##, n(2.0)), // CTX3_IND_r_sheet_both_rows
        (S2, r##"=SUM(INDIRECT("A1 :B2"))"##, n(33.0)), // CTX3_IND_a1_sp_colon
        (S2, r##"=SUM(INDIRECT("A1: B2"))"##, n(33.0)), // CTX3_IND_a1_colon_sp
        (S2, r##"=INDIRECT("Sheet1! A1")"##, n(1.0)), // CTX3_IND_a1_sheet_sp
        (S2, r##"=INDIRECT("Sheet1 !A1")"##, error(Ref)), // CTX3_IND_a1_sheet_sp_bang
        (S2, r##"=INDIRECT("A 1")"##, error(Ref)),    // CTX3_IND_a1_inside_sp
        (S2, r##"=INDIRECT("A1   ")"##, n(1.0)),      // CTX3_IND_a1_two_trail
        (S2, r##"=INDIRECT("A1"&CHAR(9))"##, error(Ref)), // CTX3_IND_a1_tab_trail
        (S2, r##"=SUM(INDIRECT("A : B"))"##, error(Ref)), // CTX3_IND_a1_cols_sp
        (S2, r##"=INDIRECT("$A$1 ")"##, n(1.0)),      // CTX3_IND_a1_dollar_sp
        (S2, r##"=INDIRECT("a1 ")"##, n(1.0)),        // CTX3_IND_a1_lower_sp
        (S2, r##"=INDIRECT("Nope ")"##, error(Ref)),  // CTX3_IND_a1_name_sp
        (S2, r##"=SUM(INDIRECT("R2 ",FALSE))"##, n(30.0)), // CTX4_IND_r_row_trail
        (S2, r##"=SUM(INDIRECT("C2 ",FALSE))"##, n(22.0)), // CTX4_IND_r_col_trail
        (S2, r##"=SUM(INDIRECT("R2:R3 ",FALSE))"##, n(330.0)), // CTX4_IND_r_rows_trail
        (S2, r##"=SUM(INDIRECT("R2 :R3",FALSE))"##, n(330.0)), // CTX4_IND_r_row_sp_colon
        (S2, r##"=SUM(INDIRECT("R2: R3",FALSE))"##, error(Ref)), // CTX4_IND_r_row_colon_sp
        (S2, r##"=SUM(INDIRECT("Sheet1! R2:R3",FALSE))"##, n(330.0)), // CTX4_IND_r_bang_sp_rows
        (S2, r##"=SUM(INDIRECT("Sheet1! R2",FALSE))"##, n(30.0)), // CTX4_IND_r_bang_sp_row
        (S2, r##"=SUM(INDIRECT("R[1] ",FALSE))"##, n(30.0)), // CTX4_IND_r_rel_row_trail
        (S2, r##"=ROW(INDIRECT("R ",FALSE))"##, n(1.0)), // CTX4_IND_r_R_trail
        (S2, r##"=COLUMN(INDIRECT("RC[-2] ",FALSE))"##, n(4.0)), // CTX4_IND_r_RC_trail
        (S2, r##"=SUM(INDIRECT("R1C1 : R2",FALSE))"##, error(Ref)), // CTX4_IND_r_cell_colon_row
        (
            S2,
            r##"=SUM(INDIRECT("R[0]C[-5] : R[1]C[-4]",FALSE))"##,
            n(33.0),
        ), // CTX4_IND_r_rel_sp_colon
        (S2, r##"=SUM(INDIRECT("A:A "))"##, n(11.0)), // CTX4_IND_a1_col_trail
        (S2, r##"=SUM(INDIRECT("2:2 "))"##, n(30.0)), // CTX4_IND_a1_row_trail
        (S2, r##"=SUM(INDIRECT("A :B"))"##, error(Ref)), // CTX4_IND_a1_cols_sp_colon
        (S2, r##"=SUM(INDIRECT("1 :2"))"##, error(Ref)), // CTX4_IND_a1_rows_sp_colon
        (S2, r##"=SUM(INDIRECT("Sheet1! A:A"))"##, n(11.0)), // CTX4_IND_a1_bang_sp_col
        (S2, r##"=SUM(INDIRECT("$A$1 : $B$2"))"##, n(33.0)), // CTX4_IND_a1_abs_sp_colon
        (S2, r##"=SUM(INDIRECT("'Sheet1'! A1:B2"))"##, n(33.0)), // CTX4_IND_a1_qsheet_bang_sp
        (S2, r##"=SUM(INDIRECT("A1: Sheet1!B2"))"##, error(Ref)), // CTX4_IND_a1_sp_colon_sheet2
        (S2, r##"=SUM(INDIRECT("A1:B2  "))"##, n(33.0)), // CTX4_IND_a1_name_trail
    ]);
}

#[test]
fn indirect_spills_as_excel_computes_it() {
    let cases: &[(Setup, &str, &[&[LiteralValue]])] = &[
        (
            S1,
            r##"=INDIRECT("R1C1:R2C2",FALSE)"##,
            &[&[n(1.0), n(2.0)], &[n(10.0), n(20.0)]],
        ), // CTX1_IND_spill_R1C1
    ];
    for (setup, formula, expected) in cases {
        assert_spill(*setup, formula, expected);
    }
}

#[test]
fn now_as_excel_computes_it() {
    assert_cases(&[
        (
            S0,
            r##"=ROUND(NOW()*86400*100,6)-ROUND(NOW()*86400*100,0)"##,
            n(0.0),
        ), // CTX2_now_cs
        (
            S0,
            r##"=ROUND(NOW()*86400*1000,6)-ROUND(NOW()*86400*1000,0)"##,
            n(0.0),
        ), // CTX2_now_ms
    ]);
}

#[test]
fn offset_as_excel_computes_it() {
    assert_cases(&[
        (S1, r##"=OFFSET(A1,1,1)"##, n(20.0)), // CTX1_OFF_basic
        (S1, r##"=SUM(OFFSET(A1,0,0,2,2))"##, n(33.0)), // CTX1_OFF_sum_2x2
        (S1, r##"=SUM(OFFSET(B2,0,0,-2,-2))"##, n(33.0)), // CTX1_OFF_neg_hw
        (S1, r##"=OFFSET(A1,-1,0)"##, error(Ref)), // CTX1_OFF_neg_row
        (S1, r##"=OFFSET(A1,0,-1)"##, error(Ref)), // CTX1_OFF_neg_col
        (S1, r##"=SUM(OFFSET(A1,0,0,0,1))"##, error(Ref)), // CTX1_OFF_zero_h
        (S1, r##"=OFFSET(A1,1.9,0)"##, n(10.0)), // CTX1_OFF_frac_row
        (S1, r##"=OFFSET(A1,"1",0)"##, n(10.0)), // CTX1_OFF_text_row
        (S1, r##"=OFFSET(A1,"x",0)"##, error(Value)), // CTX1_OFF_bad_text_row
        (S1, r##"=ROWS(OFFSET(A1:B2,1,1))"##, n(2.0)), // CTX1_OFF_rows_keep
        (S1, r##"=ROWS(OFFSET(A1,0,0,1048577))"##, error(Ref)), // CTX1_OFF_too_tall
        (S1, r##"=OFFSET(A1,,)"##, n(1.0)),    // CTX1_OFF_omitted
        (S1, r##"=SUM(OFFSET(A1:A2,0,1,,))"##, n(22.0)), // CTX1_OFF_omitted_hw
        (S1, r##"=OFFSET(A1,TRUE,0)"##, n(10.0)), // CTX1_OFF_bool_row
        (S1, r##"=OFFSET(A1:B2,0,0,1,1)"##, n(1.0)), // CTX1_OFF_shrink
        (S1, r##"=OFFSET(A1,1048575,0)"##, n(0.0)), // CTX1_OFF_last_row
        (S1, r##"=OFFSET(A1,1048576,0)"##, error(Ref)), // CTX1_OFF_past_row
        (S1, r##"=OFFSET(A1,0,16383)"##, n(0.0)), // CTX1_OFF_last_col
        (S1, r##"=OFFSET(A1,0,16384)"##, error(Ref)), // CTX1_OFF_past_col
        (S1, r##"=ROWS(OFFSET(A1,0,0,2.9,1))"##, n(2.0)), // CTX1_OFF_frac_h
        (S1, r##"=ROW(OFFSET(C3,-1.5,0))"##, n(2.0)), // CTX1_OFF_neg_frac_row
        (S1, r##"=OFFSET(A1,1/0,0)"##, error(Div)), // CTX1_OFF_err_row
        (S1, r##"=ROWS(OFFSET(A1,0,0,Z9,1))"##, error(Ref)), // CTX1_OFF_blank_h
        (S2, r##"=ROWS(OFFSET(B2,0,0,-2))"##, n(2.0)), // CTX2_OFF_neg_h_rows
        (S2, r##"=ROW(OFFSET(B2,0,0,-2))"##, n(1.0)), // CTX2_OFF_neg_h_row
        (S2, r##"=OFFSET(A1,0,0,-2)"##, error(Ref)), // CTX2_OFF_neg_h_top
        (S2, r##"=OFFSET(C3,0,0,-1,-1)"##, n(300.0)), // CTX2_OFF_neg_one
        (S2, r##"=ROW(OFFSET(C3:D4,0,0,-2))"##, n(2.0)), // CTX2_OFF_neg_h_range_row
        (S2, r##"=COLUMNS(OFFSET(C3:D4,0,0,-2))"##, n(2.0)), // CTX2_OFF_neg_h_range_cols
        (S2, r##"=ROWS(OFFSET(C3,0,0,-1.5))"##, n(1.0)), // CTX2_OFF_neg_frac_h
        (S2, r##"=ROWS(OFFSET(C3,0,0,-0.5))"##, n(2.0)), // CTX2_OFF_neg_small_h
        (
            S2,
            r##"=CELL("address",OFFSET(C3,1,1,-2,-2))"##,
            text("$C$3"),
        ), // CTX2_OFF_neg_addr
        (S2, r##"=COLUMN(OFFSET(C3,0,0,1,-3))"##, n(1.0)), // CTX2_OFF_neg_w_col
        (S2, r##"=OFFSET(B1,0,0,1,-3)"##, error(Ref)), // CTX2_OFF_neg_w_left
        (S2, r##"=SUM(OFFSET(D4,-1,-1,-2,-2))"##, n(320.0)), // CTX2_OFF_neg_hw_sum
        (
            S2,
            r##"=ROWS(OFFSET(A1048576,0,0,-1048576))"##,
            n(1048576.0),
        ), // CTX2_OFF_neg_h_big
        (S2, r##"=ROW(OFFSET(C3,0,0,-0.5))"##, n(3.0)), // CTX3_OFF_m05_row
        (S2, r##"=ROWS(OFFSET(C3,0,0,0.5))"##, n(2.0)), // CTX3_OFF_p05
        (S2, r##"=ROWS(OFFSET(C3,0,0,-0.9))"##, n(2.0)), // CTX3_OFF_m09
        (S2, r##"=ROWS(OFFSET(C3,0,0,-1.9))"##, n(1.0)), // CTX3_OFF_m19
        (S2, r##"=COLUMNS(OFFSET(C3,0,0,1,-0.5))"##, n(2.0)), // CTX3_OFF_m05_w
        (S2, r##"=COLUMN(OFFSET(C3,0,0,1,-0.5))"##, n(3.0)), // CTX3_OFF_m05_w_col
        (S2, r##"=ROWS(OFFSET(C3,0,0,-1E-300))"##, n(2.0)), // CTX3_OFF_tiny
        (S2, r##"=OFFSET(C1048576,0,0,-0.5)"##, error(Ref)), // CTX3_OFF_m05_edge
        (S2, r##"=SUM(OFFSET(C3,0,0,-0.5))"##, n(300.0)), // CTX3_OFF_m05_sum
        (
            S2,
            r##"=ROWS(OFFSET(A1,0,0,-0.5,-0.5))*10+COLUMNS(OFFSET(A1,0,0,-0.5,-0.5))"##,
            n(22.0),
        ), // CTX3_OFF_m05_both
        (S2, r##"=ROWS(OFFSET(C3,0,0,"-0.5"))"##, n(2.0)), // CTX3_OFF_negzero_text
        (S2, r##"=ROWS(OFFSET(C3,0,0,-1))"##, n(1.0)), // CTX3_OFF_m1
        (S2, r##"=ROW(OFFSET(C3,-0.5,0))"##, n(3.0)), // CTX3_OFF_rows_m05
        (S2, r##"=ROW(OFFSET(C3,0,0,0.5))"##, n(2.0)), // CTX4_OFF_p05_row
        (S2, r##"=OFFSET(A1,0,0,0.5)"##, error(Ref)), // CTX4_OFF_p05_top
        (S2, r##"=COLUMN(OFFSET(C3,0,0,1,0.5))"##, n(2.0)), // CTX4_OFF_p05_col
        (S2, r##"=ROWS(OFFSET(C3,0,0,0.9))"##, n(2.0)), // CTX4_OFF_p09_rows
        (S2, r##"=ROWS(OFFSET(C3,0,0,1.5))"##, n(1.0)), // CTX4_OFF_p15_rows
        (S2, r##"=SUM(OFFSET(C3,0,0,0.5,0.5))"##, n(320.0)), // CTX4_OFF_p05_sum
        (S2, r##"=ROW(OFFSET(C3,0,0,1E-300))"##, n(2.0)), // CTX4_OFF_tiny_pos
        (S2, r##"=ROW(OFFSET(C3:D4,0,0,0.5))"##, n(2.0)), // CTX4_OFF_p05_range
    ]);
}

#[test]
fn offset_spills_as_excel_computes_it() {
    let cases: &[(Setup, &str, &[&[LiteralValue]])] = &[
        (
            S1,
            r##"=OFFSET(A1,0,0,2,2)"##,
            &[&[n(1.0), n(2.0)], &[n(10.0), n(20.0)]],
        ), // CTX1_OFF_spill
        (
            S2,
            r##"=OFFSET(B2,0,0,-2,-2)"##,
            &[&[n(1.0), n(2.0)], &[n(10.0), n(20.0)]],
        ), // CTX2_OFF_neg_h_spill
        (
            S2,
            r##"=OFFSET(B2,0,0,0.5,0.5)"##,
            &[&[n(1.0), n(2.0)], &[n(10.0), n(20.0)]],
        ), // CTX4_OFF_p05_spill
    ];
    for (setup, formula, expected) in cases {
        assert_spill(*setup, formula, expected);
    }
}

#[test]
fn rand_as_excel_computes_it() {
    assert_cases(&[
        (S2, r##"=RAND()=RAND()"##, b(false)),   // CTX3_rand_eq
        (S2, r##"=RAND()-RAND()=0"##, b(false)), // CTX3_rand_minus
        (
            S2,
            r##"=SUM(--ISODD(MAKEARRAY(400,1,LAMBDA(r,c,RANDBETWEEN(1,6)+RANDBETWEEN(1,6)))))>0"##,
            b(true),
        ), // CTX3_rb_dice_odd
        (
            S2,
            r##"=ROWS(UNIQUE(MAKEARRAY(100,1,LAMBDA(r,c,RAND()))))"##,
            n(100.0),
        ), // CTX3_makearray_rand_distinct
        (S2, r##"=RANDBETWEEN(1.8,1.2)"##, error(Num)), // CTX3_rb_rev_frac
        (S2, r##"=RANDBETWEEN("TRUE",1)"##, error(Value)), // CTX3_rb_text_bool
        (S2, r##"=RANDBETWEEN(2.5,2.5)"##, n(3.0)), // CTX3_rb_eq_frac
        (S2, r##"=RANDBETWEEN(-2.5,-2.5)"##, n(-2.0)), // CTX3_rb_neg_eq_frac
        (S2, r##"=RANDBETWEEN(2.0000001,2.9)"##, n(3.0)), // CTX3_rb_small_gap
        (S2, r##"=RANDARRAY("TRUE")"##, error(Value)), // CTX3_ra_text_bool_rows
        (S2, r##"=RANDARRAY(1,1,"3","3",TRUE)"##, n(3.0)), // CTX3_ra_text_minmax
        (S2, r##"=RANDARRAY(1,1,TRUE,1)"##, n(1.0)), // CTX3_ra_bool_min
        (S2, r##"=RANDARRAY(1,1,1,2,"x")"##, error(Value)), // CTX3_ra_whole_x
        (S2, r##"=RANDARRAY(1,1,1,2,1/0)"##, error(Div)), // CTX3_ra_whole_err
        (S2, r##"=RANDARRAY(1,1,1/0,2)"##, error(Div)), // CTX3_ra_min_err
        (S2, r##"=RANDARRAY(-1,1/0)"##, error(Div)), // CTX3_ra_neg_then_err
        (S2, r##"=RANDARRAY(0,1,1/0)"##, error(Div)), // CTX3_ra_zero_then_err
        (S2, r##"=RANDARRAY(0,1,3,2)"##, error(Calc)), // CTX3_ra_rev_zero
        (S2, r##"=COLUMNS(RANDARRAY(1,2.9))"##, n(2.0)), // CTX3_ra_frac_cols
        (S2, r##"=ISNUMBER(RANDARRAY(,,,,))"##, b(true)), // CTX3_ra_omit_all
        (S2, r##"=RANDARRAY(1,1,3,3,)"##, n(3.0)), // CTX3_ra_omit_whole
        (
            S2,
            r##"=LET(x,RANDARRAY(1,1,0.5),AND(x>=0.5,x<1))"##,
            b(true),
        ), // CTX3_ra_max_omit
        (S2, r##"=RANDARRAY(1,1,"2.5","2.5",TRUE)"##, error(Value)), // CTX3_ra_whole_minmax_eq_text
        (S2, r##"=RANDBETWEEN(" 2","2")"##, n(2.0)), // CTX3_rb_text_space
    ]);
}

#[test]
fn rand_spills_as_excel_computes_it() {
    let cases: &[(Setup, &str, &[&[LiteralValue]])] = &[
        (
            S2,
            r##"=RANDBETWEEN(A1:A2,100)"##,
            &[&[error(Value)], &[LiteralValue::Empty]],
        ), // CTX3_rb_range_arg
    ];
    for (setup, formula, expected) in cases {
        assert_spill(*setup, formula, expected);
    }
}

#[test]
fn randarray_as_excel_computes_it() {
    assert_cases(&[
        (S2, r##"=RANDARRAY(1,1,1.5,3,TRUE)"##, error(Value)), // CTX2_ra_frac_min_int
        (S2, r##"=RANDARRAY(1,1,3,3)"##, n(3.0)),              // CTX2_ra_33
        (S2, r##"=RANDARRAY(1,1,3,2,TRUE)"##, error(Value)),   // CTX2_ra_rev_int
        (S2, r##"=RANDARRAY(1,0)"##, error(Calc)),             // CTX2_ra_cols0
        (S2, r##"=RANDARRAY(0,-1)"##, error(Value)),           // CTX2_ra_0_neg
        (S2, r##"=RANDARRAY(-1,0)"##, error(Value)),           // CTX2_ra_neg_0
        (S2, r##"=LET(x,RANDARRAY(1,1,,5),AND(x>=0,x<5))"##, b(true)), // CTX2_ra_omit_min
        (S2, r##"=RANDARRAY(1,1,5)"##, error(Value)),          // CTX2_ra_min_only
        (S2, r##"=RANDARRAY(0.5)"##, error(Calc)),             // CTX2_ra_half_rows
        (S2, r##"=ROWS(RANDARRAY("2",1,4,4))"##, n(2.0)),      // CTX2_ra_text_rows
        (S2, r##"=ROWS(RANDARRAY(TRUE,1,4,4))"##, n(1.0)),     // CTX2_ra_bool_rows
        (S2, r##"=RANDARRAY(1,1,2,2.5,1)"##, error(Value)),    // CTX2_ra_whole_1
        (S2, r##"=RANDARRAY(1,1,2,2,"TRUE")"##, n(2.0)),       // CTX2_ra_whole_text
        (S2, r##"=MAX(RANDARRAY(2000,1,1,2,TRUE))"##, n(2.0)), // CTX2_ra_int_max
        (S2, r##"=MIN(RANDARRAY(2000,1,1,2,TRUE))"##, n(1.0)), // CTX2_ra_int_min
        (
            S2,
            r##"=LET(a,RANDARRAY(2000,1,5,6),AND(MIN(a)>=5,MAX(a)<6))"##,
            b(true),
        ), // CTX2_ra_dec_range
        (S2, r##"=RANDARRAY(1,1,-2.5,-2.5,TRUE)"##, error(Value)), // CTX2_ra_neg_frac_int
        (S2, r##"=RANDARRAY(1,1,2,2.0,TRUE)"##, n(2.0)),       // CTX2_ra_22_int
        (S2, r##"=RANDARRAY(1,1/0)"##, error(Div)),            // CTX2_ra_err
        (S2, r##"=RANDARRAY(-1,-1)"##, error(Value)),          // CTX2_ra_neg_neg
        (
            S2,
            r##"=LET(a,RANDARRAY(100,1,-1E15,1E15,TRUE),AND(MIN(a)>=-1E15,MAX(a)<=1E15,SUM(a-INT(a))=0))"##,
            b(true),
        ), // CTX2_ra_big_int
        (S2, r##"=RANDARRAY(1,1,1E16,1E16,TRUE)"##, n(1e+16)), // CTX2_ra_huge_int
        (S2, r##"=RANDARRAY(1,1,2.5,2.5)"##, n(2.5)),          // CTX2_ra_frac_whole_eq
        (S2, r##"=RANDARRAY(1,1,2.5,2.5,TRUE)"##, error(Value)), // CTX2_ra_frac_whole_eq_int
        (S2, r##"=ROWS(RANDARRAY(Z9,1,4,4))"##, error(Calc)),  // CTX2_ra_blank_rows
        (S2, r##"=RANDARRAY("x")"##, error(Value)),            // CTX2_ra_text_bad
        (S2, r##"=RANDARRAY(-1,"x")"##, error(Value)),         // CTX4_RA_neg_text_rows
        (S2, r##"=RANDARRAY("x",-1)"##, error(Value)),         // CTX4_RA_text_then_neg
        (S2, r##"=RANDARRAY(0,"x")"##, error(Value)),          // CTX4_RA_zero_text
        (S2, r##"=RANDARRAY(-1,1,3,2)"##, error(Value)),       // CTX4_RA_rev_neg
        (S2, r##"=RANDARRAY(1,1,3,2.5,TRUE)"##, error(Value)), // CTX4_RA_rev_whole_frac
        (S2, r##"=RANDARRAY(0,1,1,2,"x")"##, error(Value)),    // CTX4_RA_whole_x_zero
        (S2, r##"=RANDARRAY(-1,1,1,2,"x")"##, error(Value)),   // CTX4_RA_neg_whole_x
        (S2, r##"=RANDARRAY(1,1,"x",2)"##, error(Value)),      // CTX4_RA_min_text_x
        (S2, r##"=RANDARRAY(1/0,1,NA())"##, error(Div)),       // CTX4_RA_rows_err_min_err
        (S2, r##"=RANDARRAY(NA(),1/0)"##, error(Na)),          // CTX4_RA_cols_na_rows_div
        (S2, r##"=RANDARRAY(1,1,FALSE,0)"##, n(0.0)),          // CTX4_RA_min_bool
        (S2, r##"=ROWS(RANDARRAY(1048577))"##, error(Value)),  // CTX4_RA_big_rows
        (S2, r##"=RANDARRAY(0,0,3,2)"##, error(Calc)),         // CTX4_RA_zero_rev
        (
            S2,
            r##"=LET(x,RANDARRAY(1,1,1.5,1.75),AND(x>=1.5,x<1.75))"##,
            b(true),
        ), // CTX4_RA_frac_min_dec
        (
            S2,
            r##"=LET(a,RANDARRAY(500,1,-3,3,TRUE),AND(MIN(a)=-3,MAX(a)=3))"##,
            b(true),
        ), // CTX4_RA_whole_neg_range
    ]);
}

#[test]
fn randarray_spills_as_excel_computes_it() {
    let cases: &[(Setup, &str, &[&[LiteralValue]])] = &[
        (S2, r##"=RANDARRAY(,2,4,4)"##, &[&[n(4.0), n(4.0)]]), // CTX2_ra_omit_rows
        (S2, r##"=RANDARRAY(2,,4,4)"##, &[&[n(4.0)], &[n(4.0)]]), // CTX2_ra_omit_cols
    ];
    for (setup, formula, expected) in cases {
        assert_spill(*setup, formula, expected);
    }
}

#[test]
fn randbetween_as_excel_computes_it() {
    assert_cases(&[
        (
            S2,
            r##"=MIN(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(1.5,2.5))))"##,
            n(2.0),
        ), // CTX2_rb_15_25_min
        (
            S2,
            r##"=MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(1.5,2.5))))"##,
            n(2.0),
        ), // CTX2_rb_15_25_max
        (
            S2,
            r##"=MIN(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(1.2,1.8))))"##,
            n(2.0),
        ), // CTX2_rb_12_18_min
        (
            S2,
            r##"=MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(1.2,1.8))))"##,
            n(2.0),
        ), // CTX2_rb_12_18_max
        (
            S2,
            r##"=MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(2,2.9))))"##,
            n(2.0),
        ), // CTX2_rb_2_29_max
        (
            S2,
            r##"=MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(1,1.5))))"##,
            n(1.0),
        ), // CTX2_rb_1_15_max
        (
            S2,
            r##"=MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(1.9,2.1))))"##,
            n(2.0),
        ), // CTX2_rb_19_21
        (S2, r##"=RANDBETWEEN(0.5,0.5)"##, n(1.0)), // CTX2_rb_05_05
        (S2, r##"=RANDBETWEEN(0.1,0.2)"##, n(1.0)), // CTX2_rb_01_02
        (
            S2,
            r##"=MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(-1.5,-1.2))))"##,
            n(-1.0),
        ), // CTX2_rb_m15_m12_max
        (
            S2,
            r##"=MIN(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(-1.5,-1.2))))"##,
            n(-1.0),
        ), // CTX2_rb_m15_m12_min
        (S2, r##"=RANDBETWEEN(-0.5,-0.4)"##, n(0.0)), // CTX2_rb_m05_m04
        (
            S2,
            r##"=MIN(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(0,1))))+10*MAX(MAKEARRAY(2000,1,LAMBDA(r,c,RANDBETWEEN(0,1))))"##,
            n(10.0),
        ), // CTX2_rb_0_1_minmax
        (S2, r##"=RANDBETWEEN(E1,1)"##, error(Value)), // CTX2_rb_bool_cell
        (S2, r##"=RANDBETWEEN(A1>0,1)"##, error(Value)), // CTX2_rb_bool_expr
        (
            S2,
            r##"=MAX(MAKEARRAY(500,1,LAMBDA(r,c,RANDBETWEEN(,2))))"##,
            error(Na),
        ), // CTX2_rb_omit_lo
        (S2, r##"=RANDBETWEEN(2,)"##, error(Na)),   // CTX2_rb_omit_hi
        (S2, r##"=RANDBETWEEN(1,1/0)"##, error(Div)), // CTX2_rb_err_hi
        (S2, r##"=RANDBETWEEN("1.5","1.5")"##, n(2.0)), // CTX2_rb_text_frac
        (S2, r##"=RANDBETWEEN(A3,2)"##, error(Value)), // CTX2_rb_text_cell
        (S2, r##"=RANDBETWEEN("",2)"##, error(Value)), // CTX2_rb_empty_text
        (
            S2,
            r##"=LET(x,RANDBETWEEN(-1E15,1E15),AND(x>=-1E15,x<=1E15,x=INT(x)))"##,
            b(true),
        ), // CTX2_rb_big_range
        (S2, r##"=RANDBETWEEN(1E16,1E16)"##, n(1e+16)), // CTX2_rb_huge
        (S2, r##"=RANDBETWEEN(-0.9,0)"##, n(0.0)),  // CTX2_rb_neg_zero
        (S2, r##"=RANDBETWEEN(C1,2)"##, error(Value)), // CTX2_rb_blank_text_cell
        (S2, r##"=RANDBETWEEN(D1,2)"##, error(Div)), // CTX2_rb_err_cell
        (S2, r##"=RANDBETWEEN(0,TRUE)"##, error(Value)), // CTX2_rb_bool_hi
        (
            S2,
            r##"=RANDBETWEEN("2026-01-01","2026-01-01")"##,
            n(46023.0),
        ), // CTX2_rb_date_text
    ]);
}

#[test]
fn randbetween_spills_as_excel_computes_it() {
    let cases: &[(Setup, &str, &[&[LiteralValue]])] = &[
        (S2, r##"=RANDBETWEEN({1,5},{1,5})"##, &[&[n(1.0), n(5.0)]]), // CTX2_rb_array
    ];
    for (setup, formula, expected) in cases {
        assert_spill(*setup, formula, expected);
    }
}

#[test]
fn volatile_as_excel_computes_it() {
    assert_cases(&[
        (S0, r##"=INT(NOW())=TODAY()"##, b(true)), // CTX1_VOL_now_int
        (S0, r##"=NOW()>=TODAY()"##, b(true)),     // CTX1_VOL_now_ge
        (S0, r##"=TODAY()-INT(TODAY())"##, n(0.0)), // CTX1_VOL_today_whole
        (S0, r##"=AND(RAND()>=0,RAND()<1)"##, b(true)), // CTX1_VOL_rand_range
        (S0, r##"=RANDBETWEEN(2,2)"##, n(2.0)),    // CTX1_VOL_rb_same
        (S0, r##"=RANDBETWEEN(1.5,2.5)"##, n(2.0)), // CTX1_VOL_rb_frac
        (S0, r##"=RANDBETWEEN(1.2,1.8)"##, n(2.0)), // CTX1_VOL_rb_inside
        (S0, r##"=RANDBETWEEN(3,1)"##, error(Num)), // CTX1_VOL_rb_rev
        (S0, r##"=RANDBETWEEN(-2.5,-2.5)"##, n(-2.0)), // CTX1_VOL_rb_neg_half
        (S0, r##"=RANDBETWEEN("2","2")"##, n(2.0)), // CTX1_VOL_rb_text
        (S0, r##"=RANDBETWEEN("a",2)"##, error(Value)), // CTX1_VOL_rb_bad_text
        (S0, r##"=RANDBETWEEN(TRUE,1)"##, error(Value)), // CTX1_VOL_rb_bool
        (S0, r##"=RANDBETWEEN(Z9,0)"##, n(0.0)),   // CTX1_VOL_rb_blank
        (S0, r##"=RANDBETWEEN(1E15,1E15)"##, n(1000000000000000.0)), // CTX1_VOL_rb_big
        (S0, r##"=RANDBETWEEN(1/0,2)"##, error(Div)), // CTX1_VOL_rb_err
        (S0, r##"=ISNUMBER(RANDARRAY())"##, b(true)), // CTX1_VOL_ra_isnum
        (S0, r##"=RANDARRAY(1,1,5,5)"##, n(5.0)),  // CTX1_VOL_ra_const
        (S0, r##"=RANDARRAY(1,1,5,5,TRUE)"##, n(5.0)), // CTX1_VOL_ra_const_int
        (S0, r##"=RANDARRAY(1,1,1,1.5,TRUE)"##, error(Value)), // CTX1_VOL_ra_frac_int
        (S0, r##"=RANDARRAY(1,1,3,2)"##, error(Value)), // CTX1_VOL_ra_rev
        (S0, r##"=RANDARRAY(0)"##, error(Calc)),   // CTX1_VOL_ra_zero
        (S0, r##"=RANDARRAY(-1)"##, error(Value)), // CTX1_VOL_ra_neg
        (S0, r##"=RANDARRAY(1.9,1,5,5)"##, n(5.0)), // CTX1_VOL_ra_frac_rows
        (S0, r##"=SUM(RANDARRAY(3,3,2,2,TRUE))"##, n(18.0)), // CTX1_VOL_ra_sum
        (S0, r##"=ROWS(RANDARRAY(3))"##, n(3.0)),  // CTX1_VOL_ra_rows
        (S0, r##"=COLUMNS(RANDARRAY(3,4))"##, n(4.0)), // CTX1_VOL_ra_cols
        (S0, r##"=RANDARRAY(,,4,4)"##, n(4.0)),    // CTX1_VOL_ra_omitted
        (
            S0,
            r##"=LET(a,RANDARRAY(50,1,1,3,TRUE),AND(MIN(a)>=1,MAX(a)<=3,SUM(a-INT(a))=0))"##,
            b(true),
        ), // CTX1_VOL_ra_int_range
        (
            S0,
            r##"=LET(x,RANDBETWEEN(-3,3),AND(x>=-3,x<=3,x=INT(x)))"##,
            b(true),
        ), // CTX1_VOL_rb_range
        (S0, r##"=RAND()*0"##, n(0.0)),            // CTX1_VOL_rand_zero
        (
            S0,
            r##"=TODAY()=DATE(YEAR(TODAY()),MONTH(TODAY()),DAY(TODAY()))"##,
            b(true),
        ), // CTX1_VOL_today_date
        (S0, r##"=NOW()-TODAY()<1"##, b(true)),    // CTX1_VOL_now_lt1
    ]);
}

#[test]
fn volatile_spills_as_excel_computes_it() {
    let cases: &[(Setup, &str, &[&[LiteralValue]])] = &[
        (
            S0,
            r##"=RANDARRAY(2,2,7,7,TRUE)"##,
            &[&[n(7.0), n(7.0)], &[n(7.0), n(7.0)]],
        ), // CTX1_VOL_ra_spill
    ];
    for (setup, formula, expected) in cases {
        assert_spill(*setup, formula, expected);
    }
}
