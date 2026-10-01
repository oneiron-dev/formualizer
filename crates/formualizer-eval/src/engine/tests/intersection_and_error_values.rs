//! The reference operators (`:` range and space intersection), and functions
//! that inspect error values rather than propagating them.

use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;

/// A1:D4 hold 1..16 row by row; each formula goes in F1, F2, ...
fn eval_all(formulas: &[&str]) -> Vec<LiteralValue> {
    let mut engine = Engine::new(TestWorkbook::default(), EvalConfig::default());
    for r in 1..=4u32 {
        for c in 1..=4u32 {
            engine
                .set_cell_value(
                    "Sheet1",
                    r,
                    c,
                    LiteralValue::Number(((r - 1) * 4 + c) as f64),
                )
                .unwrap();
        }
    }
    for (i, f) in formulas.iter().enumerate() {
        engine
            .set_cell_formula("Sheet1", i as u32 + 1, 6, parse(f).unwrap())
            .unwrap();
    }
    engine.evaluate_all().unwrap();
    (0..formulas.len())
        .map(|i| engine.get_cell_value("Sheet1", i as u32 + 1, 6).unwrap())
        .collect()
}

fn n(v: f64) -> LiteralValue {
    LiteralValue::Number(v)
}

fn kind(v: &LiteralValue) -> Option<ExcelErrorKind> {
    match v {
        LiteralValue::Error(e) => Some(e.kind),
        _ => None,
    }
}

#[test]
fn space_operator_intersects_references() {
    let got = eval_all(&[
        "=SUM(A1:C3 B2:D4)",
        "=A1:D1 B1:B4",
        "=ROWS(A1:C3 B2:D4)",
        "=SUM(A:A 2:2)",
    ]);
    assert_eq!(got[0], n(6.0 + 7.0 + 10.0 + 11.0));
    assert_eq!(got[1], n(2.0));
    assert_eq!(got[2], n(2.0));
    assert_eq!(got[3], n(5.0));
}

#[test]
fn disjoint_intersection_is_null() {
    let got = eval_all(&["=A1:A2 C1:C2", "=ERROR.TYPE(A1:A2 C1:C2)"]);
    assert_eq!(kind(&got[0]), Some(ExcelErrorKind::Null));
    assert_eq!(got[1], n(1.0));
}

#[test]
fn error_inspecting_functions_see_errors() {
    let got = eval_all(&[
        "=ERROR.TYPE(qwertyzz)",
        "=TYPE(1/0)",
        "=TYPE(qwertyzz)",
        "=ISERROR(qwertyzz)",
        "=IFERROR(qwertyzz,7)",
        "=TYPE(\"a\")",
    ]);
    assert_eq!(got[0], n(5.0));
    assert_eq!(got[1], n(16.0));
    assert_eq!(got[2], n(16.0));
    assert_eq!(got[3], LiteralValue::Boolean(true));
    assert_eq!(got[4], n(7.0));
    assert_eq!(got[5], n(2.0));
}

#[test]
fn intersection_formulas_parse() {
    for f in [
        "=SUM(A1:C3 B2:D4)",
        "=A1:D1 B1:B4",
        "=ROWS(A1:C3 B2:D4)",
        "=SUM(A:A 2:2)",
    ] {
        assert!(parse(f).is_ok(), "{f}: {:?}", parse(f).err());
    }
}

/// A `:` range built from computed endpoints (`INDEX(..):A4`) is an ordinary
/// range reference: as a value it reads exactly like the literal range of the
/// same area.
#[test]
fn computed_colon_range_reads_like_literal_range() {
    let pairs = [
        ("=SUM(INDEX(A1:A4,2):A4*1)", "=SUM(A2:A4*1)"),
        ("=SUM(--((A1:INDEX(A1:A4,3))>=5))", "=SUM(--((A1:A3)>=5))"),
        ("=SUM(IF(B1:INDEX(B:B,4)>=6,1,0))", "=SUM(IF(B1:B4>=6,1,0))"),
        ("=SUM(LEN(INDEX(A1:A4,2):A4))", "=SUM(LEN(A2:A4))"),
        (
            "=SUM(1/COUNTIFS(INDEX(A1:A4,2):A4,INDEX(A1:A4,2):A4))",
            "=SUM(1/COUNTIFS(A2:A4,A2:A4))",
        ),
        (
            "=SUMPRODUCT(--(INDEX(A1:A4,2):A4>4))",
            "=SUMPRODUCT(--(A2:A4>4))",
        ),
        ("=SUM(INDEX(A1:D4,2,2):INDEX(A1:D4,3,3)+0)", "=SUM(B2:C3+0)"),
        ("=INDEX(A1:A4,2):INDEX(A1:A4,2)+1", "=A2:A2+1"),
        ("=CONCAT(INDEX(A1:D1,3):D1&\"-\")", "=CONCAT(C1:D1&\"-\")"),
    ];
    let formulas: Vec<&str> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let got = eval_all(&formulas);
    for (i, (computed, literal)) in pairs.iter().enumerate() {
        assert_eq!(got[2 * i], got[2 * i + 1], "{computed} vs {literal}");
    }
    assert_eq!(got[0], n(5.0 + 9.0 + 13.0));
    assert_eq!(got[2], n(2.0));
    assert_eq!(got[4], n(3.0));
    assert_eq!(got[6], n(4.0));
    assert_eq!(got[8], n(3.0));
    assert_eq!(got[10], n(3.0));
    assert_eq!(got[12], n(6.0 + 7.0 + 10.0 + 11.0));
    assert_eq!(got[14], n(6.0));
    assert_eq!(got[16], LiteralValue::Text("3-4-".into()));
}

/// Reference consumers keep receiving the `:` range as a reference, and an
/// endpoint that fails (INDEX out of bounds) makes the range an error value
/// that ISERROR and ERROR.TYPE can inspect.
#[test]
fn computed_colon_range_keeps_reference_semantics() {
    let got = eval_all(&[
        "=SUM(INDEX(A1:A4,2):A4)",
        "=ROWS(B1:INDEX(B:B,4))",
        "=INDEX(A1:INDEX(A1:A4,4),2)",
        "=COUNTIFS(INDEX(A1:A4,2):A4,\">4\")",
        "=SUM(ROW(INDEX(A1:A4,2):A4))",
        "=ISERROR(INDEX(A1:A4,5):A4)",
        "=ERROR.TYPE(A1:INDEX(A1:A4,5))",
        "=ISERROR(INDIRECT(\"Missing!A1\"):INDIRECT(\"Missing!A2\"))",
        "=IFERROR(INDIRECT(\"Missing!A1\"):INDIRECT(\"Missing!A2\"),-1)",
    ]);
    assert_eq!(got[0], n(27.0));
    assert_eq!(got[1], n(4.0));
    assert_eq!(got[2], n(5.0));
    assert_eq!(got[3], n(3.0));
    assert_eq!(got[4], n(2.0 + 3.0 + 4.0));
    assert_eq!(got[5], LiteralValue::Boolean(true));
    assert_eq!(got[6], n(4.0));
    // A range that forms but cannot be read (missing sheet) is an error value.
    assert_eq!(got[7], LiteralValue::Boolean(true));
    assert_eq!(got[8], n(-1.0));
}

/// An error row_num or column_num makes INDEX that error where INDEX is used
/// as a reference (a `:` endpoint, SUM's range) just as in value context, so
/// IFNA catches MATCH's #N/A; a non-numeric index stays #VALUE!.
#[test]
fn index_reference_propagates_an_error_index() {
    let got = eval_all(&[
        "=SUM(INDEX(A1:D4,NA(),1))",
        "=SUM(A1:INDEX(A1:D4,NA(),1))",
        "=SUM(INDEX(A1:D4,1,MATCH(99,A1:D1,0)):D4)",
        "=IFNA(SUM(INDEX(A1:D4,MATCH(99,A1:A4,0),2):INDEX(A1:D4,4,2)),\"none\")",
        "=SUM(INDEX(A1:D4,1/0,NA()))",
        "=ROWS(A1:INDEX(A1:A4,#REF!))",
        "=INDEX(A1:D4,NA(),1)",
        "=SUM(INDEX(A1:D4,\"x\",1))",
        "=SUM(A1:INDEX(A1:D4,2,2))",
        "=SUM(INDEX(A1:D4,5,1))",
    ]);
    assert_eq!(kind(&got[0]), Some(ExcelErrorKind::Na));
    assert_eq!(kind(&got[1]), Some(ExcelErrorKind::Na));
    assert_eq!(kind(&got[2]), Some(ExcelErrorKind::Na));
    assert_eq!(got[3], LiteralValue::Text("none".into()));
    assert_eq!(kind(&got[4]), Some(ExcelErrorKind::Div));
    assert_eq!(kind(&got[5]), Some(ExcelErrorKind::Ref));
    assert_eq!(kind(&got[6]), Some(ExcelErrorKind::Na));
    assert_eq!(kind(&got[7]), Some(ExcelErrorKind::Value));
    assert_eq!(got[8], n(1.0 + 2.0 + 5.0 + 6.0));
    assert_eq!(kind(&got[9]), Some(ExcelErrorKind::Ref));
}

/// A `:` range as a whole formula spills like the literal range, and under
/// stored-workbook semantics a plain cell intersects it while a CSE array
/// fills its extent.
#[test]
fn computed_colon_range_formula_result_matches_literal_range() {
    let mut engine = Engine::new(TestWorkbook::default(), EvalConfig::default());
    for r in 1..=4u32 {
        engine
            .set_cell_value("Sheet1", r, 1, n((r * 10) as f64))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 1, 3, parse("=INDEX(A1:A4,2):A4").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 4, parse("=A2:A4").unwrap())
        .unwrap();
    engine.evaluate_all().unwrap();
    for r in 1..=3u32 {
        let want = n(((r + 1) * 10) as f64);
        assert_eq!(engine.get_cell_value("Sheet1", r, 3), Some(want.clone()));
        assert_eq!(engine.get_cell_value("Sheet1", r, 4), Some(want));
    }

    let mut engine = Engine::new(TestWorkbook::default(), EvalConfig::default());
    for r in 1..=4u32 {
        engine
            .set_cell_value("Sheet1", r, 1, n((r * 10) as f64))
            .unwrap();
    }
    engine
        .set_cell_formula("Sheet1", 3, 3, parse("=INDEX(A1:A4,2):A4").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 3, 4, parse("=A2:A4").unwrap())
        .unwrap();
    engine
        .set_cell_formula("Sheet1", 1, 5, parse("=INDEX(A1:A4,2):A4*2").unwrap())
        .unwrap();
    engine.use_legacy_array_semantics();
    engine.declare_array_formula("Sheet1", 1, 5, 3, 1, false);
    engine.evaluate_all().unwrap();
    assert_eq!(engine.get_cell_value("Sheet1", 3, 3), Some(n(30.0)));
    assert_eq!(engine.get_cell_value("Sheet1", 3, 4), Some(n(30.0)));
    assert_eq!(engine.get_cell_value("Sheet1", 1, 5), Some(n(40.0)));
    assert_eq!(engine.get_cell_value("Sheet1", 2, 5), Some(n(60.0)));
    assert_eq!(engine.get_cell_value("Sheet1", 3, 5), Some(n(80.0)));
}
