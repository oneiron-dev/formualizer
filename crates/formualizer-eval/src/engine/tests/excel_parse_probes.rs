//! Excel for Windows 16.0.20430's values for the parsing lane of oneiron's wave 2
//! (ops/excel-parse-probe-20261008.md, jobs probe-w2-parse-1 to -5): `#REF!`, the deleted
//! reference Excel writes in a formula, as an operand of the range and intersection operators;
//! the spill reference operator `A1#` (stored `_xlfn.ANCHORARRAY(A1)`) wherever a reference
//! goes, in names too; and ERROR.TYPE's codes. Each formula as typed in F1 (or the first cell of
//! the range it spills to) of a sheet whose Z1 holds the recorder's =1111+2222, after the setup
//! cells (numbers, and formulas entered as dynamic arrays through Range.Formula2) and the
//! defined names; the spill probes also keep a Ctrl+Shift+Enter `=ROW(1:3)` in L1:L3. Evaluated
//! as the xlsx writer evaluates a file: legacy array semantics, every formula the recorder
//! entered declared a dynamic array, the Ctrl+Shift+Enter one a fixed 3x1 array. Numbers to
//! 1e-12 relative, errors by kind, a spill cell by cell.

use super::upstream_picks_probes::{V, cell, define, matches};
use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// A setup cell: a number, text, or a formula entered as a dynamic array.
enum Set {
    N(f64),
    T(&'static str),
    F(&'static str),
}

/// Formula, setup cells, defined names, the formula's cell (or spill range), Excel's values.
type Case = (
    &'static str,
    &'static [(&'static str, Set)],
    &'static [(&'static str, &'static str)],
    &'static str,
    &'static [&'static [V]],
);

/// The Ctrl+Shift+Enter array of the spill probes: `=ROW(1:3)` over L1:L3.
const LEGACY_ARRAY: (&str, &str, u32, u32) = ("L1", "=ROW(1:3)", 3, 1);

fn check(cases: &[Case], legacy_array: Option<(&str, &str, u32, u32)>) {
    let mut failures = Vec::new();
    'case: for (formula, setup, names, range, expected) in cases {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        engine.use_legacy_array_semantics();
        let array_formula = |engine: &mut Engine<TestWorkbook>,
                             address: &str,
                             text: &str,
                             shape: Option<(u32, u32)>| {
            let (row, col) = cell(address);
            let (rows, cols) = shape.unwrap_or((1, 1));
            engine.declare_array_formula("Sheet1", row, col, rows, cols, shape.is_none());
            engine.set_cell_formula("Sheet1", row, col, parse(text).unwrap())
        };
        for (address, value) in setup.iter() {
            let (row, col) = cell(address);
            let result = match value {
                Set::N(n) => engine.set_cell_value("Sheet1", row, col, LiteralValue::Number(*n)),
                Set::T(t) => {
                    engine.set_cell_value("Sheet1", row, col, LiteralValue::Text((*t).into()))
                }
                Set::F(f) => array_formula(&mut engine, address, f, None),
            };
            if let Err(e) = result {
                failures.push(format!("{formula}: setting {address}: {e:?}"));
                continue 'case;
            }
        }
        if let Some((address, text, rows, cols)) = legacy_array
            && let Err(e) = array_formula(&mut engine, address, text, Some((rows, cols)))
        {
            failures.push(format!("{formula}: setting {address}: {e:?}"));
            continue;
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
        let anchor = range.split(':').next().unwrap();
        if let Err(e) = array_formula(&mut engine, anchor, formula, None) {
            failures.push(format!("{formula}: {e:?}"));
            continue;
        }
        if let Err(e) = engine.evaluate_all() {
            failures.push(format!("{formula}: evaluate_all: {e:?}"));
            continue;
        }
        let (top, left) = cell(anchor);
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

/// The `#REF!` probes' sheet: B2 = 7, B3 = 8, C2 = 9.
const SETUP_P: &[(&str, Set)] = &[
    ("B2", Set::N(7.0)),
    ("B3", Set::N(8.0)),
    ("C2", Set::N(9.0)),
];
const SETUP_PN: &[(&str, Set)] = SETUP_P;

/// The spill probes' sheet: column and 2x3 spills, cells that hold no spilling formula
/// (A10 a constant, A11 =1+1, A12 a 1x1 SEQUENCE, A14 ={7}), a blocked spill (A20 under
/// A21 = 9), a spill over another spill (J1), a spilled reference (A30), an empty FILTER's
/// #CALC! (A40), a spill of errors (A50) and an anchor that is itself a spill reference (A60).
const SETUP_S: &[(&str, Set)] = &[
    ("A1", Set::F("=SEQUENCE(3)")),
    ("A4", Set::N(10.0)),
    ("A5", Set::N(100.0)),
    ("C1", Set::F("=SEQUENCE(2,3)")),
    ("A10", Set::N(5.0)),
    ("A11", Set::F("=1+1")),
    ("A12", Set::F("=SEQUENCE(1)")),
    ("A14", Set::F("={7}")),
    ("A20", Set::F("=SEQUENCE(3)")),
    ("A21", Set::N(9.0)),
    ("J1", Set::F("=A1#*2")),
    ("B30", Set::N(4.0)),
    ("B31", Set::N(5.0)),
    ("B32", Set::N(6.0)),
    ("A30", Set::F("=B30:B32")),
    ("A40", Set::F("=FILTER(A1#,A1#>5)")),
    ("A50", Set::F("=SEQUENCE(3)/0")),
    ("A60", Set::F("=A1#")),
];
const SETUP_SN: &[(&str, Set)] = SETUP_S;

const SETUP_Q: &[(&str, Set)] = &[];
const SETUP_QE: &[(&str, Set)] = &[];

/// The second spill batch's sheet: the column and 2x3 spills, a constant (A10), a blocked
/// spill (A20), an empty FILTER (A40), anchors holding #DIV/0! (A70), SEQUENCE(0)'s #CALC!
/// (A71), @SEQUENCE(3) (A72), an array constant (A73), text (A77) and a spill holding "" (A80).
const SETUP_T: &[(&str, Set)] = &[
    ("A1", Set::F("=SEQUENCE(3)")),
    ("A4", Set::N(10.0)),
    ("A5", Set::N(100.0)),
    ("C1", Set::F("=SEQUENCE(2,3)")),
    ("A10", Set::N(5.0)),
    ("A20", Set::F("=SEQUENCE(3)")),
    ("A21", Set::N(9.0)),
    ("A40", Set::F("=FILTER(A1#,A1#>5)")),
    ("A70", Set::F("=1/0")),
    ("A71", Set::F("=SEQUENCE(0)")),
    ("A72", Set::F("=@SEQUENCE(3)")),
    ("A73", Set::F("={1;2;3}")),
    ("A77", Set::F("=\"x\"")),
    ("A80", Set::F("=IF(SEQUENCE(3)=2,\"\",SEQUENCE(3))")),
];
const SETUP_E: &[(&str, Set)] = SETUP_T;

/// `#REF!` beside `:` or ` ` is the reference error it stands for: the range, the intersection and
/// every function over them are #REF! (P01-P32; a union in an aggregate, P11, is not here).
#[test]
fn reference_error_is_a_range_operand() {
    check(
        &[
            // P01: error literal #REF! left of the range operator
            (
                "=#REF!:B2",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P02: #REF! right of the range operator
            (
                "=B2:#REF!",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P03: the 14207 name body, unqualified, under SUM
            (
                "=SUM(#REF!:INDEX(#REF!,COUNTA(#REF!)))",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P04: the 14207 name body as a cell formula
            (
                "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P05: ROWS of it
            (
                "=ROWS(#REF!:B2)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P06: ISREF of it
            ("=ISREF(#REF!:B2)", SETUP_P, &[], "F1", &[&[V::B(false)]]),
            // P07: ISERROR of it
            ("=ISERROR(#REF!:B2)", SETUP_P, &[], "F1", &[&[V::B(true)]]),
            // P08: ERROR.TYPE of it
            ("=ERROR.TYPE(#REF!:B2)", SETUP_P, &[], "F1", &[&[V::N(4.0)]]),
            // P09: IFERROR of it
            (
                "=IFERROR(#REF!:B2,\"x\")",
                SETUP_P,
                &[],
                "F1",
                &[&[V::T("x")]],
            ),
            // P10: #REF! in an intersection
            (
                "=#REF! B2",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P18: #REF! on both sides
            (
                "=#REF!:#REF!",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P19: sheet-qualified #REF! alone
            (
                "=Sheet1!#REF!",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P20: ISREF of #REF!
            ("=ISREF(#REF!)", SETUP_P, &[], "F1", &[&[V::B(false)]]),
            // P21: ISREF of a sheet-qualified #REF!
            (
                "=ISREF(Sheet1!#REF!)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::B(false)]],
            ),
            // P22: COUNTA of #REF!
            ("=COUNTA(#REF!)", SETUP_P, &[], "F1", &[&[V::N(1.0)]]),
            // P23: SUM over B2:#REF!
            (
                "=SUM(B2:#REF!)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P24: #REF! to an INDEX reference
            (
                "=#REF!:INDEX(B:B,3)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P25: SUM over a qualified #REF! to B2
            (
                "=SUM(Sheet1!#REF!:B2)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P26: IF returning it
            (
                "=IF(TRUE,#REF!:B2)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P27: lower-case #ref!
            (
                "=#ref!:B2",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P28: chained range operators
            (
                "=#REF!:B2:C3",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P29: parenthesized #REF! left of the range operator
            (
                "=(#REF!):B2",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P30: ROWS of an intersection with #REF!
            (
                "=ROWS(#REF! B2)",
                SETUP_P,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // P31: COUNTA over it
            ("=COUNTA(#REF!:B2)", SETUP_P, &[], "F1", &[&[V::N(1.0)]]),
            // P32: ISERR of the 14207 body
            (
                "=ISERR(#REF!:INDEX(#REF!,COUNTA(#REF!)))",
                SETUP_P,
                &[],
                "F1",
                &[&[V::B(true)]],
            ),
        ],
        None,
    );
}

/// Defined names Excel keeps after deleting what they referred to (SpreadsheetBench 14207).
#[test]
fn names_holding_a_deleted_reference() {
    check(
        &[
            // PN01: ISERROR of the 14207 name
            (
                "=ISERROR(nm)",
                SETUP_PN,
                &[(
                    "nm",
                    "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                )],
                "F1",
                &[&[V::B(true)]],
            ),
            // PN02: ERROR.TYPE of the 14207 name
            (
                "=ERROR.TYPE(nm)",
                SETUP_PN,
                &[(
                    "nm",
                    "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                )],
                "F1",
                &[&[V::N(4.0)]],
            ),
            // PN03: ROWS of the 14207 name
            (
                "=IFERROR(ROWS(nm),\"e\")",
                SETUP_PN,
                &[(
                    "nm",
                    "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                )],
                "F1",
                &[&[V::T("e")]],
            ),
            // PN04: the 14207 name itself
            (
                "=nm",
                SETUP_PN,
                &[(
                    "nm",
                    "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                )],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // PN05: ISREF of the 14207 name
            (
                "=ISREF(nm)",
                SETUP_PN,
                &[(
                    "nm",
                    "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                )],
                "F1",
                &[&[V::B(false)]],
            ),
            // PN06: COUNTA of the 14207 name
            (
                "=COUNTA(nm)",
                SETUP_PN,
                &[(
                    "nm",
                    "=Sheet1!#REF!:INDEX(Sheet1!#REF!,COUNTA(Sheet1!#REF!))",
                )],
                "F1",
                &[&[V::N(1.0)]],
            ),
            // PN07: ISREF of a name that is Sheet1!#REF!
            (
                "=ISREF(nm)",
                SETUP_PN,
                &[("nm", "=Sheet1!#REF!")],
                "F1",
                &[&[V::B(false)]],
            ),
            // PN08: ERROR.TYPE of it
            (
                "=ERROR.TYPE(nm)",
                SETUP_PN,
                &[("nm", "=Sheet1!#REF!")],
                "F1",
                &[&[V::N(4.0)]],
            ),
            // PN09: the name itself
            (
                "=nm",
                SETUP_PN,
                &[("nm", "=Sheet1!#REF!")],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
        ],
        None,
    );
}

/// `A1#` wherever a reference goes, and what it is for a cell that holds no spilling formula.
#[test]
fn spill_references() {
    check(
        &[
            // S01: SUM over a column spill
            ("=SUM(A1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S02: ROWS
            ("=ROWS(A1#)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S03: COLUMNS of a 2x3 spill
            ("=COLUMNS(C1#)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S04: ROWS of a 2x3 spill
            ("=ROWS(C1#)", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S05: INDEX into it
            ("=INDEX(A1#,2)", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S06: INDEX into a 2x3 spill
            ("=INDEX(C1#,2,3)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S07: the spill itself spills again
            (
                "=A1#",
                SETUP_S,
                &[],
                "F1:F4",
                &[&[V::N(1.0)], &[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // S08: range operator extends the spill
            ("=SUM(A1#:A5)", SETUP_S, &[], "F1", &[&[V::N(116.0)]]),
            // S09: ROWS of A1#:B9
            ("=ROWS(A1#:B9)", SETUP_S, &[], "F1", &[&[V::N(9.0)]]),
            // S10: COLUMNS of A1#:B9
            ("=COLUMNS(A1#:B9)", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S11: range operator inside the spill
            ("=SUM(C1#:C1)", SETUP_S, &[], "F1", &[&[V::N(21.0)]]),
            // S12: anchor is a constant
            (
                "=SUM(A10#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // S13: anchor is a single-value formula
            ("=SUM(A11#)", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S14: anchor is a 1x1 dynamic array
            ("=SUM(A12#)", SETUP_S, &[], "F1", &[&[V::N(1.0)]]),
            // S15: anchor is a 1x1 array literal
            ("=SUM(A14#)", SETUP_S, &[], "F1", &[&[V::N(7.0)]]),
            // S16: anchor's spill is blocked (#SPILL!)
            (
                "=SUM(A20#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Spill)]],
            ),
            // S17: anchor cell is empty
            (
                "=SUM(B1#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // S18: a spilled cell, not the anchor
            (
                "=SUM(A2#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // S19: OFFSET of the spill
            (
                "=SUM(OFFSET(A1#,1,0))",
                SETUP_S,
                &[],
                "F1",
                &[&[V::N(15.0)]],
            ),
            // S20: OFFSET of the spill, spilled
            (
                "=OFFSET(A1#,1,0)",
                SETUP_S,
                &[],
                "F1:F4",
                &[&[V::N(2.0)], &[V::N(3.0)], &[V::N(10.0)], &[V::Blank]],
            ),
            // S21: INDEX whole column of the spill
            (
                "=INDEX(A1#,0,1)",
                SETUP_S,
                &[],
                "F1:F4",
                &[&[V::N(1.0)], &[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // S22: COUNT
            ("=COUNT(A1#)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S23: MAX of a 2x3 spill
            ("=MAX(C1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S24: arithmetic over the spill
            (
                "=A1#*10",
                SETUP_S,
                &[],
                "F1:F4",
                &[&[V::N(10.0)], &[V::N(20.0)], &[V::N(30.0)], &[V::Blank]],
            ),
            // S25: SUMPRODUCT
            ("=SUMPRODUCT(A1#,A1#)", SETUP_S, &[], "F1", &[&[V::N(14.0)]]),
            // S26: XLOOKUP
            (
                "=XLOOKUP(2,A1#,A1#*10)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::N(20.0)]],
            ),
            // S27: MATCH
            ("=MATCH(3,A1#,0)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S28: SUMIF needs a range
            ("=SUMIF(A1#,\">1\")", SETUP_S, &[], "F1", &[&[V::N(5.0)]]),
            // S29: COUNTIF needs a range
            ("=COUNTIF(A1#,\">=2\")", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S30: a spill anchored by a formula over another spill
            ("=ROWS(A1#)+ROWS(J1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S31: SUM of the chained spill
            ("=SUM(J1#)", SETUP_S, &[], "F1", &[&[V::N(12.0)]]),
            // S32: sheet-qualified
            ("=SUM(Sheet1!A1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S33: absolute anchor
            ("=SUM($A$1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S34: TRANSPOSE
            (
                "=TRANSPOSE(A1#)",
                SETUP_S,
                &[],
                "F1:I1",
                &[&[V::N(1.0), V::N(2.0), V::N(3.0), V::Blank]],
            ),
            // S35: intersection with the spill
            ("=SUM(A1# A2:C2)", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S37: ISREF of a spill
            ("=ISREF(A1#)", SETUP_S, &[], "F1", &[&[V::B(true)]]),
            // S38: ISREF of a non-spill anchor
            ("=ISREF(A10#)", SETUP_S, &[], "F1", &[&[V::B(false)]]),
            // S39: ERROR.TYPE of a non-spill anchor
            ("=ERROR.TYPE(A10#)", SETUP_S, &[], "F1", &[&[V::N(4.0)]]),
            // S40: IFERROR over a non-spill anchor
            (
                "=IFERROR(SUM(A10#),\"none\")",
                SETUP_S,
                &[],
                "F1",
                &[&[V::T("none")]],
            ),
            // S41: anchor spills a reference
            ("=SUM(A30#)", SETUP_S, &[], "F1", &[&[V::N(15.0)]]),
            // S42: ROWS of a spilled reference
            ("=ROWS(A30#)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S43: anchor is #CALC! (empty FILTER)
            (
                "=SUM(A40#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // S44: anchor spills errors
            (
                "=SUM(A50#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Div)]],
            ),
            // S45: anchor of a legacy Ctrl+Shift+Enter array
            ("=SUM(L1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S46: implicit intersection of the spill
            ("=@A1#", SETUP_S, &[], "F1", &[&[V::N(1.0)]]),
            // S49: range operator result spilled
            (
                "=A1#:B9",
                SETUP_S,
                &[],
                "F1:G10",
                &[
                    &[V::N(1.0), V::N(0.0)],
                    &[V::N(2.0), V::N(0.0)],
                    &[V::N(3.0), V::N(0.0)],
                    &[V::N(10.0), V::N(0.0)],
                    &[V::N(100.0), V::N(0.0)],
                    &[V::N(0.0), V::N(0.0)],
                    &[V::N(0.0), V::N(0.0)],
                    &[V::N(0.0), V::N(0.0)],
                    &[V::N(0.0), V::N(0.0)],
                    &[V::Blank, V::Blank],
                ],
            ),
            // S50: range operator with the anchor
            ("=ROWS(A1#:A1)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S51: ROW of the spill
            (
                "=ROW(A1#)",
                SETUP_S,
                &[],
                "F1:F4",
                &[&[V::N(1.0)], &[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // S52: COLUMN of a 2x3 spill
            (
                "=COLUMN(C1#)",
                SETUP_S,
                &[],
                "F1:I1",
                &[&[V::N(3.0), V::N(4.0), V::N(5.0), V::Blank]],
            ),
            // S53: CELL address of the spill
            (
                "=CELL(\"address\",A1#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::T("$A$1")]],
            ),
            // S54: ADDRESS from ROWS
            (
                "=ADDRESS(ROWS(A1#),1)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::T("$A$3")]],
            ),
            // S55: last item idiom
            ("=INDEX(A1#,ROWS(A1#))", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S56: INDEX:INDEX inside the spill
            (
                "=SUM(INDEX(A1#,1):INDEX(A1#,2))",
                SETUP_S,
                &[],
                "F1",
                &[&[V::N(3.0)]],
            ),
            // S57: VLOOKUP
            (
                "=VLOOKUP(2,A1#,1,FALSE)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::N(2.0)]],
            ),
            // S58: AVERAGE
            ("=AVERAGE(A1#)", SETUP_S, &[], "F1", &[&[V::N(2.0)]]),
            // S59: COUNTA of a 2x3 spill
            ("=COUNTA(C1#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S60: LET over the spill
            ("=LET(x,A1#,SUM(x))", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S61: broadcast of a 3x1 and a 2x3 spill
            (
                "=A1#+C1#",
                SETUP_S,
                &[],
                "F1:H4",
                &[
                    &[V::N(2.0), V::N(3.0), V::N(4.0)],
                    &[V::N(6.0), V::N(7.0), V::N(8.0)],
                    &[
                        V::E(ExcelErrorKind::Na),
                        V::E(ExcelErrorKind::Na),
                        V::E(ExcelErrorKind::Na),
                    ],
                    &[V::Blank, V::Blank, V::Blank],
                ],
            ),
            // S62: anchor whose formula is a spill reference
            ("=SUM(A60#)", SETUP_S, &[], "F1", &[&[V::N(6.0)]]),
            // S63: ISERROR of a non-spill anchor
            ("=ISERROR(A10#)", SETUP_S, &[], "F1", &[&[V::B(true)]]),
            // S64: spill reference to a constant
            ("=A10#", SETUP_S, &[], "F1", &[&[V::E(ExcelErrorKind::Ref)]]),
            // S65: OFFSET sized by ROWS of the spill
            (
                "=SUM(OFFSET(A1,0,0,ROWS(A1#)))",
                SETUP_S,
                &[],
                "F1",
                &[&[V::N(6.0)]],
            ),
            // S66: ROWS of a blocked anchor
            ("=ROWS(A20#)", SETUP_S, &[], "F1", &[&[V::N(1.0)]]),
            // S67: ERROR.TYPE of a blocked anchor
            ("=ERROR.TYPE(A20#)", SETUP_S, &[], "F1", &[&[V::N(9.0)]]),
            // S68: ERROR.TYPE of a #CALC! anchor
            ("=ERROR.TYPE(A40#)", SETUP_S, &[], "F1", &[&[V::N(14.0)]]),
            // S69: ERROR.TYPE of an empty anchor
            ("=ERROR.TYPE(B1#)", SETUP_S, &[], "F1", &[&[V::N(4.0)]]),
            // S70: ERROR.TYPE of a spilled cell
            ("=ERROR.TYPE(A2#)", SETUP_S, &[], "F1", &[&[V::N(4.0)]]),
            // S71: ERROR.TYPE of a 1x1 dynamic array anchor
            (
                "=ERROR.TYPE(A12#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Na)]],
            ),
            // S72: ROWS of a 1x1 dynamic array anchor
            ("=ROWS(A12#)", SETUP_S, &[], "F1", &[&[V::N(1.0)]]),
            // S73: ROWS of a 1x1 array-literal anchor
            ("=ROWS(A14#)", SETUP_S, &[], "F1", &[&[V::N(1.0)]]),
            // S74: ROWS of a legacy array anchor
            ("=ROWS(L1#)", SETUP_S, &[], "F1", &[&[V::N(3.0)]]),
            // S75: ERROR.TYPE of a single-value formula anchor
            (
                "=ERROR.TYPE(A11#)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Na)]],
            ),
            // S76: spill to #REF!
            (
                "=SUM(A1#:#REF!)",
                SETUP_S,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
        ],
        Some(LEGACY_ARRAY),
    );
}

/// A defined name holding a spill reference, and the operator on a name.
#[test]
fn spill_references_in_names() {
    check(
        &[
            // SN01: name over a spill reference
            (
                "=SUM(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1",
                &[&[V::N(6.0)]],
            ),
            // SN02: ROWS of the name
            (
                "=ROWS(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1",
                &[&[V::N(3.0)]],
            ),
            // SN03: INDEX of the name
            (
                "=INDEX(sp,2)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1",
                &[&[V::N(2.0)]],
            ),
            // SN04: name over a 2x3 spill
            (
                "=SUM(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$C$1#")],
                "F1",
                &[&[V::N(21.0)]],
            ),
            // SN06: name over a constant's spill reference
            (
                "=SUM(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$10#")],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // SN07: the name spilled
            (
                "=sp",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1:F4",
                &[&[V::N(1.0)], &[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // SN09: spill operator on a cell name
            (
                "=SUM(anc#)",
                SETUP_SN,
                &[("anc", "=Sheet1!$A$1")],
                "F1",
                &[&[V::N(6.0)]],
            ),
            // SN10: ISREF of the name
            (
                "=ISREF(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1",
                &[&[V::B(true)]],
            ),
            // SN11: ERROR.TYPE of the name (not an error)
            (
                "=ERROR.TYPE(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1",
                &[&[V::E(ExcelErrorKind::Na)]],
            ),
            // SN12: name over a blocked spill
            (
                "=ERROR.TYPE(sp)",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$20#")],
                "F1",
                &[&[V::N(9.0)]],
            ),
            // SN13: OFFSET of the name
            (
                "=SUM(OFFSET(sp,1,0))",
                SETUP_SN,
                &[("sp", "=Sheet1!$A$1#")],
                "F1",
                &[&[V::N(15.0)]],
            ),
            // SN14: spill operator on a two-cell name
            (
                "=SUM(anc#)",
                SETUP_SN,
                &[("anc", "=Sheet1!$A$1:$A$2")],
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
        ],
        Some(LEGACY_ARRAY),
    );
}

/// What `#` takes: a LET name, a reference a function returns, a name holding no reference,
/// INDIRECT text; and more anchors.
#[test]
fn spill_reference_operands() {
    check(
        &[
            // T01: spill operator on a LET name bound to the anchor
            ("=LET(x,A1,SUM(x#))", SETUP_T, &[], "F1", &[&[V::N(6.0)]]),
            // T02: spill operator on an INDEX reference
            ("=SUM(INDEX(A:A,1)#)", SETUP_T, &[], "F1", &[&[V::N(6.0)]]),
            // T03: spill operator on an OFFSET reference
            ("=SUM(OFFSET(A1,0,0)#)", SETUP_T, &[], "F1", &[&[V::N(6.0)]]),
            // T04: spill operator on an undefined name
            (
                "=SUM(nosuch#)",
                SETUP_T,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Name)]],
            ),
            // T05: spill operator on a name holding a constant
            (
                "=SUM(k#)",
                SETUP_T,
                &[("k", "=5")],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            // T08: parenthesized spill reference
            ("=SUM((A1#))", SETUP_T, &[], "F1", &[&[V::N(6.0)]]),
            // T09: anchor holds #DIV/0!
            ("=ROWS(A70#)", SETUP_T, &[], "F1", &[&[V::N(1.0)]]),
            // T10: SUM over an anchor holding #DIV/0!
            (
                "=SUM(A70#)",
                SETUP_T,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Div)]],
            ),
            // T11: anchor is SEQUENCE(0)
            ("=ROWS(A71#)", SETUP_T, &[], "F1", &[&[V::N(1.0)]]),
            // T12: ERROR.TYPE of SEQUENCE(0)'s anchor
            ("=ERROR.TYPE(A71#)", SETUP_T, &[], "F1", &[&[V::N(14.0)]]),
            // T13: anchor is @SEQUENCE(3)
            ("=SUM(A72#)", SETUP_T, &[], "F1", &[&[V::N(1.0)]]),
            // T14: anchor is an array constant
            ("=ROWS(A73#)", SETUP_T, &[], "F1", &[&[V::N(3.0)]]),
            // T15: anchor is a text formula
            ("=A77#", SETUP_T, &[], "F1", &[&[V::T("x")]]),
            // T16: anchor spills an empty string
            ("=COUNTA(A80#)", SETUP_T, &[], "F1", &[&[V::N(3.0)]]),
            // T17: spill reference twice
            ("=SUM(A1#,A1#)", SETUP_T, &[], "F1", &[&[V::N(12.0)]]),
            // T18: MAX minus MIN
            ("=MAX(A1#)-MIN(A1#)", SETUP_T, &[], "F1", &[&[V::N(2.0)]]),
            // T19: INDEX 0,0
            (
                "=INDEX(A1#,0,0)",
                SETUP_T,
                &[],
                "F1:F4",
                &[&[V::N(1.0)], &[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // T20: array arithmetic
            (
                "=SUM(A1#*{1;10;100})",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(321.0)]],
            ),
            // T21: FILTER
            (
                "=FILTER(A1#,A1#>1)",
                SETUP_T,
                &[],
                "F1:F3",
                &[&[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // T22: SORT descending
            (
                "=SORT(A1#,,-1)",
                SETUP_T,
                &[],
                "F1:F4",
                &[&[V::N(3.0)], &[V::N(2.0)], &[V::N(1.0)], &[V::Blank]],
            ),
            // T23: XLOOKUP reference into the spill in a range
            (
                "=SUM(XLOOKUP(2,A1#,A1#):A3)",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(5.0)]],
            ),
            // T24: ISREF of INDEX into the spill
            ("=ISREF(INDEX(A1#,1))", SETUP_T, &[], "F1", &[&[V::B(true)]]),
            // T25: CELL row of the spill
            ("=CELL(\"row\",A1#)", SETUP_T, &[], "F1", &[&[V::N(1.0)]]),
            // T26: AREAS
            ("=AREAS(A1#)", SETUP_T, &[], "F1", &[&[V::N(1.0)]]),
            // T27: SUMPRODUCT of a condition
            (
                "=SUMPRODUCT((A1#>1)*A1#)",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(5.0)]],
            ),
            // T28: TEXTJOIN
            (
                "=TEXTJOIN(\",\",,A1#)",
                SETUP_T,
                &[],
                "F1",
                &[&[V::T("1,2,3")]],
            ),
            // T31: two spills
            ("=SUM(A1#)+COUNT(C1#)", SETUP_T, &[], "F1", &[&[V::N(12.0)]]),
            // T32: SUMIFS over the spill
            (
                "=SUMIFS(A1#,A1#,\">1\")",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(5.0)]],
            ),
            // T33: AVERAGEIF over a 2x3 spill
            (
                "=AVERAGEIF(C1#,\">2\")",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(4.5)]],
            ),
            // T34: SUBTOTAL
            ("=SUBTOTAL(9,A1#)", SETUP_T, &[], "F1", &[&[V::N(6.0)]]),
            // T35: AGGREGATE
            ("=AGGREGATE(9,6,A1#)", SETUP_T, &[], "F1", &[&[V::N(6.0)]]),
            // T36: RANK needs a reference
            ("=RANK(2,A1#)", SETUP_T, &[], "F1", &[&[V::N(2.0)]]),
            // T37: INDIRECT of spill text
            (
                "=INDIRECT(\"A1#\")",
                SETUP_T,
                &[],
                "F1:F4",
                &[&[V::N(1.0)], &[V::N(2.0)], &[V::N(3.0)], &[V::Blank]],
            ),
            // T38: SUM of INDIRECT of spill text
            (
                "=SUM(INDIRECT(\"A1#\"))",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(6.0)]],
            ),
            // T39: SUM of INDIRECT of qualified spill text
            (
                "=SUM(INDIRECT(\"Sheet1!A1#\"))",
                SETUP_T,
                &[],
                "F1",
                &[&[V::N(6.0)]],
            ),
        ],
        None,
    );
}

/// ERROR.TYPE's codes, #SPILL! 9 and #CALC! 14 among them; an uncalled LAMBDA is no error.
#[test]
fn error_type_codes() {
    check(
        &[
            // E01: a blocked spill
            ("=ERROR.TYPE(A20)", SETUP_E, &[], "F1", &[&[V::N(9.0)]]),
            // E02: an empty FILTER's #CALC!
            ("=ERROR.TYPE(A40)", SETUP_E, &[], "F1", &[&[V::N(14.0)]]),
            // E03: #NULL!
            ("=ERROR.TYPE(#NULL!)", SETUP_E, &[], "F1", &[&[V::N(1.0)]]),
            // E04: #DIV/0!
            ("=ERROR.TYPE(#DIV/0!)", SETUP_E, &[], "F1", &[&[V::N(2.0)]]),
            // E05: #VALUE!
            ("=ERROR.TYPE(#VALUE!)", SETUP_E, &[], "F1", &[&[V::N(3.0)]]),
            // E06: #REF!
            ("=ERROR.TYPE(#REF!)", SETUP_E, &[], "F1", &[&[V::N(4.0)]]),
            // E07: #NAME?
            ("=ERROR.TYPE(#NAME?)", SETUP_E, &[], "F1", &[&[V::N(5.0)]]),
            // E08: #NUM!
            ("=ERROR.TYPE(#NUM!)", SETUP_E, &[], "F1", &[&[V::N(6.0)]]),
            // E09: #N/A
            ("=ERROR.TYPE(#N/A)", SETUP_E, &[], "F1", &[&[V::N(7.0)]]),
            // E10: an uncalled LAMBDA
            (
                "=ERROR.TYPE(LAMBDA(x,x))",
                SETUP_E,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Na)]],
            ),
            // E11: SEQUENCE(0)
            ("=ERROR.TYPE(A71)", SETUP_E, &[], "F1", &[&[V::N(14.0)]]),
            // E12: not an error
            (
                "=ERROR.TYPE(5)",
                SETUP_E,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Na)]],
            ),
        ],
        None,
    );
}

/// SEQUENCE with no rows or no columns (a size that truncates to 0) is #CALC!, a negative one #VALUE!.
#[test]
fn sequence_without_rows_or_columns() {
    check(
        &[
            // Q01: SEQUENCE(0) in a cell
            (
                "=SEQUENCE(0)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // Q02: SEQUENCE(0,5) in a cell
            (
                "=SEQUENCE(0,5)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // Q03: SEQUENCE(5,0) in a cell
            (
                "=SEQUENCE(5,0)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // Q04: SEQUENCE(-3,5) in a cell
            (
                "=SEQUENCE(-3,5)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            // Q05: SEQUENCE(5,-1) in a cell
            (
                "=SEQUENCE(5,-1)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            // Q06: SEQUENCE(0.5) in a cell
            (
                "=SEQUENCE(0.5)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // Q07: SEQUENCE(-0.5) in a cell
            (
                "=SEQUENCE(-0.5)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            // Q08: SEQUENCE(0,0) in a cell
            (
                "=SEQUENCE(0,0)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // Q09: SEQUENCE(1E-9) in a cell
            (
                "=SEQUENCE(1E-9)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Calc)]],
            ),
            // Q10: SEQUENCE(-1) in a cell
            (
                "=SEQUENCE(-1)",
                SETUP_Q,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
        ],
        None,
    );
}

/// ERROR.TYPE tells SEQUENCE's #CALC! (14) from its #VALUE! (3).
#[test]
fn error_type_of_sequence_sizes() {
    check(
        &[
            // QE01: ERROR.TYPE of SEQUENCE(0)
            (
                "=ERROR.TYPE(SEQUENCE(0))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(14.0)]],
            ),
            // QE02: ERROR.TYPE of SEQUENCE(0,5)
            (
                "=ERROR.TYPE(SEQUENCE(0,5))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(14.0)]],
            ),
            // QE03: ERROR.TYPE of SEQUENCE(5,0)
            (
                "=ERROR.TYPE(SEQUENCE(5,0))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(14.0)]],
            ),
            // QE04: ERROR.TYPE of SEQUENCE(-3,5)
            (
                "=ERROR.TYPE(SEQUENCE(-3,5))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(3.0)]],
            ),
            // QE05: ERROR.TYPE of SEQUENCE(5,-1)
            (
                "=ERROR.TYPE(SEQUENCE(5,-1))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(3.0)]],
            ),
            // QE06: ERROR.TYPE of SEQUENCE(0.5)
            (
                "=ERROR.TYPE(SEQUENCE(0.5))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(14.0)]],
            ),
            // QE07: ERROR.TYPE of SEQUENCE(-0.5)
            (
                "=ERROR.TYPE(SEQUENCE(-0.5))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(3.0)]],
            ),
            // QE08: ERROR.TYPE of SEQUENCE(0,0)
            (
                "=ERROR.TYPE(SEQUENCE(0,0))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(14.0)]],
            ),
            // QE09: ERROR.TYPE of SEQUENCE(1E-9)
            (
                "=ERROR.TYPE(SEQUENCE(1E-9))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(14.0)]],
            ),
            // QE10: ERROR.TYPE of SEQUENCE(-1)
            (
                "=ERROR.TYPE(SEQUENCE(-1))",
                SETUP_QE,
                &[],
                "F1",
                &[&[V::N(3.0)]],
            ),
        ],
        None,
    );
}

/// The text-limit probes' sheet: H1 = REPT("a",32767), H3 = REPT("a",256), H4 = 16,383 characters
/// outside the Basic Multilingual Plane (32,766 UTF-16 units), H7 = REPT("a",32766), and the
/// column spill A1.
const SETUP_CL: &[(&str, Set)] = &[
    ("A1", Set::F("=SEQUENCE(3)")),
    ("H1", Set::F("=REPT(\"a\",32767)")),
    ("H3", Set::F("=REPT(\"a\",256)")),
    ("H4", Set::F("=REPT(UNICHAR(128512),16383)")),
    ("H7", Set::F("=REPT(\"a\",32766)")),
];

/// `&` and CONCATENATE keep the first 32,767 characters (UTF-16 units) of a longer join, without
/// an error; CONCAT and TEXTJOIN are #CALC! past it, SUBSTITUTE and REPLACE #VALUE! (CL, TX, CU
/// and RP).
#[test]
fn joins_past_32767_characters() {
    const KEPT: &[&[V]] = &[&[V::N(32767.0)]];
    const CALC: &[&[V]] = &[&[V::E(ExcelErrorKind::Calc)]];
    const CALC_CODE: &[&[V]] = &[&[V::N(14.0)]];
    let big: &[(&str, &str)] = &[("big", "=Sheet1!$H$1&\"a\"")];
    check(
        &[
            // CL02, CL04, CL06, CL09-CL11, CL15
            ("=LEN(H1&\"a\")", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(H1&H1)", SETUP_CL, &[], "F1", KEPT),
            (
                "=LEN(REPT(\"a\",16384)&REPT(\"b\",16384))",
                SETUP_CL,
                &[],
                "F1",
                KEPT,
            ),
            ("=IFERROR(LEN(H1&\"a\"),-1)", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(H1&1)", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(\"a\"&H1)", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(H1&TRUE)", SETUP_CL, &[], "F1", KEPT),
            // CL14: 32,768 units keep 32,767, the astral characters whole and the "a"
            ("=LEN(H4&\"ab\")", SETUP_CL, &[], "F1", &[&[V::N(16384.0)]]),
            // CL16, CL17
            ("=LEN(CONCATENATE(H1,\"a\"))", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(CONCAT(H1,\"a\"))", SETUP_CL, &[], "F1", CALC),
            // CL20: element by element
            (
                "=SUM(LEN(H1&{\"\",\"a\"}))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::N(65534.0)]],
            ),
            // TX04-TX07: the first 32,767 are kept
            ("=(H1&\"a\")=H1", SETUP_CL, &[], "F1", &[&[V::B(true)]]),
            ("=EXACT(H1&\"a\",H1)", SETUP_CL, &[], "F1", &[&[V::B(true)]]),
            ("=RIGHT(H1&\"b\",1)", SETUP_CL, &[], "F1", &[&[V::T("a")]]),
            ("=RIGHT(H7&\"bc\",1)", SETUP_CL, &[], "F1", &[&[V::T("b")]]),
            // TX09, TX19-TX21, TX25, TX27, TX29, TX38
            ("=LEN(IFERROR(H1&\"a\",\"e\"))", SETUP_CL, &[], "F1", KEPT),
            (
                "=FIND(\"b\",H1&\"b\")",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            ("=LEN(UPPER(H1&\"a\"))", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(H1&\"a\"&\"b\")", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(big)", SETUP_CL, big, "F1", KEPT),
            ("=LET(x,H1&\"a\",LEN(x))", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(H1&H1&H1)", SETUP_CL, &[], "F1", KEPT),
            ("=LEN(REPT(\"a\",32767)&\"a\")", SETUP_CL, &[], "F1", KEPT),
            // TX18: SUBSTITUTE past it is #VALUE!
            (
                "=LEN(SUBSTITUTE(H1,\"a\",\"bb\"))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            // CU01-CU04: CONCAT and TEXTJOIN count UTF-16 units (H4 is 32,766 of them)
            (
                "=ERROR.TYPE(CONCAT(H4,\"ab\"))",
                SETUP_CL,
                &[],
                "F1",
                CALC_CODE,
            ),
            (
                "=LEN(CONCAT(H4,\"a\"))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::N(16384.0)]],
            ),
            (
                "=ERROR.TYPE(TEXTJOIN(\"\",TRUE,H4,\"ab\"))",
                SETUP_CL,
                &[],
                "F1",
                CALC_CODE,
            ),
            (
                "=LEN(TEXTJOIN(\"\",TRUE,H4,\"a\"))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::N(16384.0)]],
            ),
            // RP01-RP03: REPLACE past it is #VALUE!
            (
                "=LEN(REPLACE(H1,1,0,H1))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::E(ExcelErrorKind::Value)]],
            ),
            (
                "=ERROR.TYPE(REPLACE(H1,1,0,\"a\"))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::N(3.0)]],
            ),
            ("=LEN(REPLACE(H1,1,1,\"b\"))", SETUP_CL, &[], "F1", KEPT),
            // TX15-TX17
            (
                "=ERROR.TYPE(CONCAT(H1,\"a\"))",
                SETUP_CL,
                &[],
                "F1",
                CALC_CODE,
            ),
            (
                "=LEN(TEXTJOIN(\"\",TRUE,H1,\"a\"))",
                SETUP_CL,
                &[],
                "F1",
                CALC,
            ),
            (
                "=ERROR.TYPE(TEXTJOIN(\"\",TRUE,H1,\"a\"))",
                SETUP_CL,
                &[],
                "F1",
                CALC_CODE,
            ),
        ],
        None,
    );
}

/// INDIRECT text ending in `#`: R1C1 text spills too, the `#` follows one cell or a name, and
/// text naming no anchor is #REF! like any INDIRECT text (IS and IR; A1 = SEQUENCE(3), F1 the
/// formula's cell).
#[test]
fn indirect_spill_text() {
    let names: &[(&str, &str)] = &[("anc", "=Sheet1!$A$1"), ("k", "=5")];
    let r: &[&[V]] = &[&[V::N(4.0)]];
    check(
        &[
            // IS01-IS04, IS10, IS11
            (
                "=ERROR.TYPE(INDIRECT(\"missing#\"))",
                SETUP_CL,
                names,
                "F1",
                r,
            ),
            ("=ERROR.TYPE(INDIRECT(\"k#\"))", SETUP_CL, names, "F1", r),
            (
                "=ERROR.TYPE(INDIRECT(\"#REF!#\"))",
                SETUP_CL,
                names,
                "F1",
                r,
            ),
            ("=ERROR.TYPE(INDIRECT(\"A0#\"))", SETUP_CL, names, "F1", r),
            ("=ERROR.TYPE(INDIRECT(\"A1##\"))", SETUP_CL, names, "F1", r),
            ("=ERROR.TYPE(INDIRECT(\"#\"))", SETUP_CL, names, "F1", r),
            // IS14: a range before the `#`
            (
                "=SUM(INDIRECT(\"A1:A1#\"))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::E(ExcelErrorKind::Ref)]],
            ),
            // IR01-IR03, IR05, IR07
            (
                "=ROWS(INDIRECT(\"R1C1#\",FALSE))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::N(3.0)]],
            ),
            (
                "=SUM(INDIRECT(\"R1C1#\",FALSE))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::N(6.0)]],
            ),
            (
                "=SUM(INDIRECT(\"Sheet1!R1C1#\",FALSE))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::N(6.0)]],
            ),
            (
                "=SUM(INDIRECT(\"RC[-5]#\",FALSE))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::N(6.0)]],
            ),
            (
                "=SUM(INDIRECT(\"anc#\",FALSE))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::N(6.0)]],
            ),
            // IR04: the spill is no error
            (
                "=ERROR.TYPE(INDIRECT(\"R1C1#\",FALSE))",
                SETUP_CL,
                names,
                "F1",
                &[&[V::E(ExcelErrorKind::Na)]],
            ),
        ],
        None,
    );
}

/// A text longer than 255 characters (UTF-16 units) is #VALUE! to ERROR.TYPE and an array to TYPE,
/// a join past 32,767 included (ET, TY, EC, TC, CL03, TX10, TX31).
#[test]
fn long_text_in_error_type_and_type() {
    const VALUE_CODE: &[&[V]] = &[&[V::N(3.0)]];
    const NOT_AN_ERROR: &[&[V]] = &[&[V::E(ExcelErrorKind::Na)]];
    const ARRAY: &[&[V]] = &[&[V::N(64.0)]];
    const TEXT: &[&[V]] = &[&[V::N(2.0)]];
    check(
        &[
            (
                "=ERROR.TYPE(REPT(\"a\",255))",
                SETUP_CL,
                &[],
                "F1",
                NOT_AN_ERROR,
            ),
            (
                "=ERROR.TYPE(REPT(\"a\",256))",
                SETUP_CL,
                &[],
                "F1",
                VALUE_CODE,
            ),
            ("=TYPE(REPT(\"a\",255))", SETUP_CL, &[], "F1", TEXT),
            ("=TYPE(REPT(\"a\",256))", SETUP_CL, &[], "F1", ARRAY),
            (
                "=ERROR.TYPE(REPT(UNICHAR(128512),128))",
                SETUP_CL,
                &[],
                "F1",
                VALUE_CODE,
            ),
            (
                "=ERROR.TYPE(REPT(UNICHAR(128512),127)&\"a\")",
                SETUP_CL,
                &[],
                "F1",
                NOT_AN_ERROR,
            ),
            (
                "=TYPE(REPT(UNICHAR(128512),128))",
                SETUP_CL,
                &[],
                "F1",
                ARRAY,
            ),
            (
                "=ISERROR(ERROR.TYPE(H3))",
                SETUP_CL,
                &[],
                "F1",
                &[&[V::B(false)]],
            ),
            ("=ERROR.TYPE(H1&\"a\")", SETUP_CL, &[], "F1", VALUE_CODE),
            ("=ERROR.TYPE(H1&\"\")", SETUP_CL, &[], "F1", VALUE_CODE),
            ("=ERROR.TYPE(H4&\"a\")", SETUP_CL, &[], "F1", VALUE_CODE),
            ("=TYPE(H1&\"a\")", SETUP_CL, &[], "F1", ARRAY),
        ],
        None,
    );
}

/// The criteria probes' sheet: H1 and J1 = REPT("a",32767), J2 = REPT("a",32766), H8 and J3 =
/// REPT("a",256), H9 = REPT("a",255), K3 = 5, and the database ranges L1:L2 ("crit" over 256
/// characters) and M1:M2 ("crit" over 255).
const SETUP_CR: &[(&str, Set)] = &[
    ("H1", Set::F("=REPT(\"a\",32767)")),
    ("J1", Set::F("=REPT(\"a\",32767)")),
    ("J2", Set::F("=REPT(\"a\",32766)")),
    ("J3", Set::F("=REPT(\"a\",256)")),
    ("H8", Set::F("=REPT(\"a\",256)")),
    ("H9", Set::F("=REPT(\"a\",255)")),
    ("K3", Set::N(5.0)),
    ("L1", Set::T("crit")),
    ("L2", Set::F("=REPT(\"a\",256)")),
    ("M1", Set::T("crit")),
    ("M2", Set::F("=REPT(\"a\",255)")),
];

/// A criterion text longer than 255 UTF-16 units, its operator included, is #VALUE! to COUNTIF,
/// SUMIF, AVERAGEIF and the IFS functions; the database functions take it (CI and CR).
#[test]
fn criteria_past_255_characters() {
    const VALUE: &[&[V]] = &[&[V::E(ExcelErrorKind::Value)]];
    check(
        &[
            ("=COUNTIF(J1:J2,H1&\"a\")", SETUP_CR, &[], "F1", VALUE),
            ("=COUNTIF(J1:J2,H1)", SETUP_CR, &[], "F1", VALUE),
            ("=COUNTIF(J3,H8)", SETUP_CR, &[], "F1", VALUE),
            (
                "=COUNTIF(J3,REPT(\"a\",255))",
                SETUP_CR,
                &[],
                "F1",
                &[&[V::N(0.0)]],
            ),
            ("=SUMIF(J3,H8,J3)", SETUP_CR, &[], "F1", VALUE),
            ("=AVERAGEIF(J3,H8,K3)", SETUP_CR, &[], "F1", VALUE),
            ("=COUNTIFS(J3,H8)", SETUP_CR, &[], "F1", VALUE),
            ("=SUMIFS(K3,J3,H8)", SETUP_CR, &[], "F1", VALUE),
            ("=AVERAGEIFS(K3,J3,H8)", SETUP_CR, &[], "F1", VALUE),
            ("=MAXIFS(K3,J3,H8)", SETUP_CR, &[], "F1", VALUE),
            ("=MINIFS(K3,J3,H8)", SETUP_CR, &[], "F1", VALUE),
            ("=COUNTIFS(J3,H9)", SETUP_CR, &[], "F1", &[&[V::N(0.0)]]),
            ("=COUNTIF(J3,\"=\"&H9)", SETUP_CR, &[], "F1", VALUE),
            ("=COUNTIF(J3,\"<>\"&H9)", SETUP_CR, &[], "F1", VALUE),
            (
                "=SUM(COUNTIF(J3,{\"a\",\"b\"}&H9))",
                SETUP_CR,
                &[],
                "F1",
                VALUE,
            ),
            ("=COUNTIF(J3,\"*\"&H9)", SETUP_CR, &[], "F1", VALUE),
            (
                "=DCOUNTA(L1:L2,1,L1:L2)",
                SETUP_CR,
                &[],
                "F1",
                &[&[V::N(1.0)]],
            ),
            (
                "=DCOUNTA(M1:M2,1,M1:M2)",
                SETUP_CR,
                &[],
                "F1",
                &[&[V::N(1.0)]],
            ),
            ("=COUNTIFS(J3,H8,J3,\"a*\")", SETUP_CR, &[], "F1", VALUE),
            (
                "=COUNTIF(J3,REPT(\"b\",128)&REPT(UNICHAR(128512),64))",
                SETUP_CR,
                &[],
                "F1",
                VALUE,
            ),
        ],
        None,
    );
}
