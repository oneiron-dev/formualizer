//! Excel's compatibility names for its distributions (NORMSDIST, CHIDIST,
//! TDIST, ...) and the current names that share their numerics and rules,
//! as Excel for Windows 16.0.20430 reads them: every row of
//! ops/excel-legacy-functions-probe-20261006.md (probes 1-5, each formula in
//! F1 of a blank sheet), numbers to 1e-12 relative, errors by kind. The
//! 6 formulas Excel refuses at entry (NORMSDIST(1,TRUE), ...) are not
//! rows: no workbook holds them.

use crate::engine::{Engine, EvalConfig};
use crate::test_workbook::TestWorkbook;
use formualizer_common::{ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::parse;

/// The value of `formula` in F1 of a sheet whose other cells are `cells`
/// ((row, column, number)).
fn eval_with(formula: &str, cells: &[(u32, u32, f64)]) -> LiteralValue {
    let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
    for &(row, col, value) in cells {
        engine
            .set_cell_value("Sheet1", row, col, LiteralValue::Number(value))
            .unwrap();
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

fn same(actual: &LiteralValue, expected: &LiteralValue) -> bool {
    match (actual, expected) {
        (LiteralValue::Number(a), LiteralValue::Number(b)) => (a - b).abs() <= 1e-12 * b.abs(),
        (LiteralValue::Int(a), LiteralValue::Number(b)) => *a as f64 == *b,
        (LiteralValue::Error(a), LiteralValue::Error(b)) => a.kind == b.kind,
        _ => actual == expected,
    }
}

fn assert_cases(cases: &[(&str, LiteralValue)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(formula, expected)| {
            let actual = eval_with(formula, &[]);
            (!same(&actual, expected)).then(|| format!("{formula}: {actual:?}, not {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn normsdist() {
    assert_cases(&[
        ("=NORMSDIST(0)", n(0.5)),
        ("=NORMSDIST(1)", n(0.841344746068543)),
        ("=NORMSDIST(-1.5)", n(0.06680720126885806)),
        ("=NORMSDIST(1.959963985)", n(0.9750000000268816)),
        ("=NORMSDIST(-8.5)", n(9.479534822203247e-18)),
        ("=NORMSDIST(-37.5)", n(4.605353009581954e-308)),
        ("=NORMSDIST(10)", n(1.0)),
        ("=NORMSDIST(\"1\")", n(0.841344746068543)),
        ("=NORMSDIST(\"a\")", error(ExcelErrorKind::Value)),
        ("=NORMSDIST(TRUE)", n(0.841344746068543)),
        ("=NORMSDIST(A1)", n(0.5)),
        ("=NORMSDIST(#N/A)", error(ExcelErrorKind::Na)),
        ("=SUM(NORMSDIST({0,1}))", n(1.341344746068543)),
        ("=NORMSDIST(-38.5)", n(0.0)),
        ("=NORMSDIST(-40)", n(0.0)),
        ("=NORMSDIST(-20)", n(2.753624118606156e-89)),
        ("=NORMSDIST(5)", n(0.9999997133484281)),
    ]);
}

#[test]
fn normsinv() {
    assert_cases(&[
        ("=NORMSINV(0.5)", n(0.0)),
        ("=NORMSINV(0.975)", n(1.9599639845400536)),
        ("=NORMSINV(0.025)", n(-1.9599639845400538)),
        ("=NORMSINV(0.001)", n(-3.090232306167813)),
        ("=NORMSINV(1E-10)", n(-6.361340902404056)),
        ("=NORMSINV(0.999999)", n(4.753424308817089)),
        ("=NORMSINV(0.908789)", n(1.3333346730441071)),
        ("=NORMSINV(0)", error(ExcelErrorKind::Num)),
        ("=NORMSINV(1)", error(ExcelErrorKind::Num)),
        ("=NORMSINV(-0.1)", error(ExcelErrorKind::Num)),
        ("=NORMSINV(\"a\")", error(ExcelErrorKind::Value)),
    ]);
}

#[test]
fn normdist() {
    assert_cases(&[
        ("=NORMDIST(42,40,1.5,TRUE)", n(0.9087887802741321)),
        ("=NORMDIST(42,40,1.5,FALSE)", n(0.10934004978399575)),
        ("=NORMDIST(1,0,1,TRUE)", n(0.841344746068543)),
        ("=NORMDIST(-3,0,2,FALSE)", n(0.06475879783294587)),
        ("=NORMDIST(1,0,0,TRUE)", error(ExcelErrorKind::Num)),
        ("=NORMDIST(1,0,-1,TRUE)", error(ExcelErrorKind::Num)),
        ("=NORMDIST(1,0,1,2)", n(0.841344746068543)),
        ("=NORMDIST(1,0,1,\"TRUE\")", n(0.841344746068543)),
        ("=NORMDIST(1,0,1,A1)", n(0.24197072451914337)),
        ("=NORMDIST(1,0,1,)", n(0.24197072451914337)),
        ("=NORMDIST(1,0,1,\"1\")", error(ExcelErrorKind::Value)),
        ("=NORMDIST(1,0,1,\"yes\")", error(ExcelErrorKind::Value)),
        ("=NORMDIST(1,0,1,\"false\")", n(0.24197072451914337)),
    ]);
}

#[test]
fn norminv() {
    assert_cases(&[
        ("=NORMINV(0.908789,40,1.5)", n(42.00000200956616)),
        ("=NORMINV(0.5,10,2)", n(10.0)),
        ("=NORMINV(0.01,0,1)", n(-2.3263478740408408)),
        ("=NORMINV(0,0,1)", error(ExcelErrorKind::Num)),
        ("=NORMINV(1,0,1)", error(ExcelErrorKind::Num)),
        ("=NORMINV(0.5,0,0)", error(ExcelErrorKind::Num)),
        ("=NORMINV(0.5,0,-1)", error(ExcelErrorKind::Num)),
        ("=NORMINV(0.999999999,5,3)", n(22.993421058804916)),
    ]);
}

#[test]
fn lognormdist() {
    assert_cases(&[
        ("=LOGNORMDIST(4,3.5,1.2)", n(0.03908355570680048)),
        ("=LOGNORMDIST(1,0,1)", n(0.5)),
        ("=LOGNORMDIST(10,1,2)", n(0.7425711705549419)),
        ("=LOGNORMDIST(0,0,1)", error(ExcelErrorKind::Num)),
        ("=LOGNORMDIST(-1,0,1)", error(ExcelErrorKind::Num)),
        ("=LOGNORMDIST(1,0,0)", error(ExcelErrorKind::Num)),
        ("=LOGNORMDIST(1,0,-1)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn loginv() {
    assert_cases(&[
        ("=LOGINV(0.039084,3.5,1.2)", n(4.000025218680638)),
        ("=LOGINV(0.5,0,1)", n(1.0)),
        ("=LOGINV(0.9,1,2)", n(35.272482631261816)),
        ("=LOGINV(0,0,1)", error(ExcelErrorKind::Num)),
        ("=LOGINV(1,0,1)", error(ExcelErrorKind::Num)),
        ("=LOGINV(0.5,0,0)", error(ExcelErrorKind::Num)),
        ("=LOGINV(0.5,0,-1)", error(ExcelErrorKind::Num)),
        ("=LOGINV(1E-10,0,1)", n(0.0017270493538983833)),
    ]);
}

#[test]
fn tdist() {
    assert_cases(&[
        ("=TDIST(1.959999998,60,2)", n(0.05464492997592096)),
        ("=TDIST(1.959999998,60,1)", n(0.02732246498796048)),
        ("=TDIST(0,10,1)", n(0.5)),
        ("=TDIST(0,10,2)", n(1.0)),
        ("=TDIST(2.5,3,2)", n(0.08770664700806553)),
        ("=TDIST(-1,10,1)", error(ExcelErrorKind::Num)),
        ("=TDIST(-1,10,2)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,10,3)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,10,0)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,10,1.5)", n(0.17044656615102993)),
        ("=TDIST(1,10,2.9)", n(0.34089313230205986)),
        ("=TDIST(1,10,0.9)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,0,1)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,0.5,1)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,1.9,2)", n(0.5000000000000001)),
        ("=TDIST(1,10.7,2)", n(0.34089313230205986)),
        ("=TDIST(1,1E10,2)", n(0.3173105078871111)),
        ("=TDIST(40,2,2)", n(0.0006244146721847406)),
        ("=TDIST(\"2.5\",3,2)", n(0.08770664700806553)),
        ("=TDIST(TRUE,10,1)", n(0.17044656615102993)),
        ("=TDIST(1,10,\"1\")", n(0.17044656615102993)),
        ("=TDIST(1,10,)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,10,TRUE)", n(0.17044656615102993)),
        ("=TDIST(1E-10,5,2)", n(1.0)),
        ("=TDIST(1,1E11,2)", error(ExcelErrorKind::Num)),
        ("=TDIST(1,1E6,2)", n(0.317310749833578)),
        ("=TDIST(1E-5,5,2)", n(0.9999924078662037)),
    ]);
}

#[test]
fn chidist() {
    assert_cases(&[
        ("=CHIDIST(18.307,10)", n(0.05000058909139811)),
        ("=CHIDIST(0.5,1)", n(0.4795001221869535)),
        ("=CHIDIST(3,2)", n(0.22313016014842982)),
        ("=CHIDIST(0,10)", n(1.0)),
        ("=CHIDIST(-1,10)", error(ExcelErrorKind::Num)),
        ("=CHIDIST(1,0)", error(ExcelErrorKind::Num)),
        ("=CHIDIST(1,0.5)", error(ExcelErrorKind::Num)),
        ("=CHIDIST(3,2.9)", n(0.22313016014842982)),
        ("=CHIDIST(3,1E10)", n(1.0)),
        ("=CHIDIST(3,9999999999)", n(1.0)),
        ("=CHIDIST(1000,2)", n(7.124576406741286e-218)),
        ("=CHIDIST(3,\"a\")", error(ExcelErrorKind::Value)),
        ("=SUM(CHIDIST({1,2},2))", n(0.9744101008840758)),
        ("=CHIDIST(3,\"2\")", n(0.22313016014842982)),
        ("=CHIDIST(40,3)", n(1.0655090334255863e-08)),
        ("=CHIDIST(0.001,5)", n(0.9999999983185123)),
        ("=CHIDIST(3,1E11)", error(ExcelErrorKind::Num)),
        ("=CHIDIST(1000000,1000000)", n(0.49981193680339453)),
    ]);
}

#[test]
fn chiinv() {
    assert_cases(&[
        ("=CHIINV(0.050001,10)", n(18.306973456961057)),
        ("=CHIINV(0.5,2)", n(1.3862943611198906)),
        ("=CHIINV(0.95,1)", n(0.003932140000019529)),
        ("=CHIINV(1E-6,3)", n(30.6648497062136)),
        ("=CHIINV(1,10)", n(0.0)),
        ("=CHIINV(0,10)", error(ExcelErrorKind::Num)),
        ("=CHIINV(-0.1,10)", error(ExcelErrorKind::Num)),
        ("=CHIINV(1.1,10)", error(ExcelErrorKind::Num)),
        ("=CHIINV(0.5,0)", error(ExcelErrorKind::Num)),
        ("=CHIINV(0.5,2.9)", n(1.3862943611198906)),
        ("=CHIINV(0.5,1E10)", n(9999999999.333334)),
        ("=CHIINV(0.5,1.9)", n(0.4549364231195729)),
        ("=CHIINV(0.001,50)", n(86.66081519040313)),
        ("=CHIINV(0.5,1000000)", n(999999.3333334123)),
    ]);
}

#[test]
fn fdist() {
    assert_cases(&[
        ("=FDIST(15.2068649,6,4)", n(0.009999999952464606)),
        ("=FDIST(1,2,2)", n(0.5)),
        ("=FDIST(2.5,5,10)", n(0.1020022766442698)),
        ("=FDIST(0,6,4)", n(1.0)),
        ("=FDIST(-1,6,4)", error(ExcelErrorKind::Num)),
        ("=FDIST(1,0,4)", error(ExcelErrorKind::Num)),
        ("=FDIST(1,6,0)", error(ExcelErrorKind::Num)),
        ("=FDIST(2,6.9,4.9)", n(0.26171875)),
        ("=FDIST(1,1E10,4)", n(0.5939941502360268)),
        ("=FDIST(1,4,1E10)", n(0.40600584976397325)),
        ("=FDIST(1000,1,1)", n(0.020124978303644132)),
        ("=FDIST(0.001,10,20)", n(0.9999999999999378)),
        ("=FDIST(1,1E11,4)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn finv() {
    assert_cases(&[
        ("=FINV(0.01,6,4)", n(15.20686486115753)),
        ("=FINV(0.5,2,2)", n(1.0)),
        ("=FINV(0.05,5,10)", n(3.325834530413013)),
        ("=FINV(1,6,4)", n(0.0)),
        ("=FINV(0,6,4)", error(ExcelErrorKind::Num)),
        ("=FINV(1.1,6,4)", error(ExcelErrorKind::Num)),
        ("=FINV(0.05,6.9,4.9)", n(6.163132282688633)),
        ("=FINV(0.5,0,4)", error(ExcelErrorKind::Num)),
        ("=FINV(0.001,1,1)", n(405284.0679028489)),
        ("=FINV(0.999,10,20)", n(0.12814356913492045)),
        ("=FINV(0.5,1000000,1000000)", n(0.9999999999999999)),
        ("=FINV(0.5,4,1E11)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn binomdist() {
    assert_cases(&[
        ("=BINOMDIST(6,10,0.5,FALSE)", n(0.20507812500000006)),
        ("=BINOMDIST(6,10,0.5,TRUE)", n(0.828125)),
        ("=BINOMDIST(3,10,0.3,TRUE)", n(0.6496107184)),
        ("=BINOMDIST(6.9,10,0.5,FALSE)", n(0.20507812500000006)),
        ("=BINOMDIST(6,10.9,0.5,FALSE)", n(0.20507812500000006)),
        ("=BINOMDIST(11,10,0.5,FALSE)", error(ExcelErrorKind::Num)),
        ("=BINOMDIST(-1,10,0.5,FALSE)", error(ExcelErrorKind::Num)),
        ("=BINOMDIST(6,10,1.1,FALSE)", error(ExcelErrorKind::Num)),
        ("=BINOMDIST(6,10,-0.1,TRUE)", error(ExcelErrorKind::Num)),
        ("=BINOMDIST(0,10,0,FALSE)", n(1.0)),
        ("=BINOMDIST(10,10,1,TRUE)", n(1.0)),
        ("=BINOMDIST(0,0,0.5,FALSE)", n(1.0)),
        ("=BINOMDIST(6,10,0.5,\"FALSE\")", n(0.20507812500000006)),
        ("=BINOMDIST(500,1000,0.5,TRUE)", n(0.5126125090891803)),
        ("=BINOMDIST(1000,1000,0.999,FALSE)", n(0.36769542477096373)),
    ]);
}

#[test]
fn poisson() {
    assert_cases(&[
        ("=POISSON(2,5,FALSE)", n(0.08422433748856833)),
        ("=POISSON(2,5,TRUE)", n(0.12465201948308113)),
        ("=POISSON(2.9,5,FALSE)", n(0.08422433748856833)),
        ("=POISSON(-1,5,TRUE)", error(ExcelErrorKind::Num)),
        ("=POISSON(2,-1,TRUE)", error(ExcelErrorKind::Num)),
        ("=POISSON(0,0,FALSE)", n(1.0)),
        ("=POISSON(2,0,FALSE)", n(0.0)),
        ("=POISSON(2,0,TRUE)", n(1.0)),
        ("=POISSON(100,50,TRUE)", n(0.9999999998430253)),
        ("=POISSON(2,5,\"x\")", error(ExcelErrorKind::Value)),
        ("=POISSON(1000,1000,TRUE)", n(0.5084093671685059)),
        ("=POISSON(30,2,FALSE)", n(5.478363323846002e-25)),
    ]);
}

#[test]
fn expondist() {
    assert_cases(&[
        ("=EXPONDIST(0.2,10,TRUE)", n(0.8646647167633873)),
        ("=EXPONDIST(0.2,10,FALSE)", n(1.353352832366127)),
        ("=EXPONDIST(0,10,FALSE)", n(10.0)),
        ("=EXPONDIST(-0.1,10,TRUE)", error(ExcelErrorKind::Num)),
        ("=EXPONDIST(0.2,0,TRUE)", error(ExcelErrorKind::Num)),
        ("=EXPONDIST(0.2,-1,TRUE)", error(ExcelErrorKind::Num)),
        ("=EXPONDIST(0,10,TRUE)", n(0.0)),
    ]);
}

#[test]
fn gammadist() {
    assert_cases(&[
        ("=GAMMADIST(10.00001131,9,2,FALSE)", n(0.03263913041829401)),
        ("=GAMMADIST(10.00001131,9,2,TRUE)", n(0.06809400386978734)),
        ("=GAMMADIST(5,1,1,TRUE)", n(0.9932620530009145)),
        ("=GAMMADIST(0,9,2,TRUE)", n(0.0)),
        ("=GAMMADIST(0,1,2,FALSE)", error(ExcelErrorKind::Num)),
        ("=GAMMADIST(0,0.5,2,FALSE)", error(ExcelErrorKind::Num)),
        ("=GAMMADIST(0,2,2,FALSE)", n(0.0)),
        ("=GAMMADIST(-1,9,2,TRUE)", error(ExcelErrorKind::Num)),
        ("=GAMMADIST(1,0,2,TRUE)", error(ExcelErrorKind::Num)),
        ("=GAMMADIST(1,9,0,TRUE)", error(ExcelErrorKind::Num)),
        ("=GAMMADIST(0,1,2,TRUE)", n(0.0)),
        ("=GAMMADIST(0,0.5,2,TRUE)", n(0.0)),
        ("=GAMMADIST(1E-300,0.5,2,FALSE)", n(3.9894228040142567e+149)),
    ]);
}

#[test]
fn weibull() {
    assert_cases(&[
        ("=WEIBULL(105,20,100,TRUE)", n(0.929581390069277)),
        ("=WEIBULL(105,20,100,FALSE)", n(0.035588864024503564)),
        ("=WEIBULL(0,1,2,FALSE)", n(0.0)),
        ("=WEIBULL(0,0.5,2,FALSE)", n(0.0)),
        ("=WEIBULL(0,2,1,FALSE)", n(0.0)),
        ("=WEIBULL(-1,20,100,TRUE)", error(ExcelErrorKind::Num)),
        ("=WEIBULL(1,0,100,TRUE)", error(ExcelErrorKind::Num)),
        ("=WEIBULL(1,20,0,TRUE)", error(ExcelErrorKind::Num)),
        ("=WEIBULL(0,1,2,TRUE)", n(0.0)),
        ("=WEIBULL(1E-300,0.5,2,FALSE)", n(3.535533905932696e+149)),
    ]);
}

#[test]
fn betadist() {
    assert_cases(&[
        ("=BETADIST(2,8,10,1,3)", n(0.6854705810546873)),
        ("=BETADIST(0.5,2,3)", n(0.6875)),
        ("=BETADIST(0,2,3)", n(0.0)),
        ("=BETADIST(1,2,3)", n(1.0)),
        ("=BETADIST(-0.1,2,3)", error(ExcelErrorKind::Num)),
        ("=BETADIST(1.1,2,3)", error(ExcelErrorKind::Num)),
        ("=BETADIST(0.5,0,3)", error(ExcelErrorKind::Num)),
        ("=BETADIST(0.5,2,0)", error(ExcelErrorKind::Num)),
        ("=BETADIST(2,8,10,3,1)", error(ExcelErrorKind::Num)),
        ("=BETADIST(2,8,10,2,2)", error(ExcelErrorKind::Num)),
        ("=BETADIST(1,8,10,1,3)", n(0.0)),
        ("=BETADIST(3,8,10,1,3)", n(1.0)),
        ("=BETADIST(2,8,10,1)", error(ExcelErrorKind::Num)),
        ("=BETADIST(0.5,2,3,0,)", n(0.6875)),
        ("=BETADIST(0.5,2,3,,)", n(0.6875)),
        ("=BETADIST(0.5,2,3,0.5)", n(0.0)),
        ("=BETADIST(0.25,0.5,0.5)", n(0.33333333333333337)),
        ("=BETADIST(0.999,2,3)", n(0.9999999960029999)),
    ]);
}

#[test]
fn betainv() {
    assert_cases(&[
        ("=BETAINV(0.685470581,8,10,1,3)", n(1.9999999999631426)),
        ("=BETAINV(0.5,2,2)", n(0.5)),
        ("=BETAINV(0.2,2,5)", n(0.13988068826995784)),
        ("=BETAINV(0,2,3)", error(ExcelErrorKind::Num)),
        ("=BETAINV(1,2,3)", error(ExcelErrorKind::Num)),
        ("=BETAINV(1.1,2,3)", error(ExcelErrorKind::Num)),
        ("=BETAINV(0.5,0,3)", error(ExcelErrorKind::Num)),
        ("=BETAINV(0.5,2,3,1,1)", error(ExcelErrorKind::Num)),
        ("=BETAINV(0.5,2,3,2,1)", error(ExcelErrorKind::Num)),
        ("=BETAINV(0.5,2,3,1)", error(ExcelErrorKind::Num)),
        ("=BETAINV(0.5,2,3,0,)", n(0.3857275681323895)),
        ("=BETAINV(0.5,2,3,0.5)", n(0.6928637840661948)),
        ("=BETAINV(0.9,0.5,0.5)", n(0.9755282581475768)),
        ("=BETAINV(1E-6,2,3)", n(0.00040835946020425715)),
    ]);
}

#[test]
fn hypgeomdist() {
    assert_cases(&[
        ("=HYPGEOMDIST(1,4,8,20)", n(0.3632610939112486)),
        ("=HYPGEOMDIST(0,4,8,20)", n(0.10216718266253864)),
        ("=HYPGEOMDIST(4,4,8,20)", n(0.014447884416924652)),
        ("=HYPGEOMDIST(1.9,4.9,8.9,20.9)", n(0.3632610939112486)),
        ("=HYPGEOMDIST(5,4,8,20)", n(0.0)),
        ("=HYPGEOMDIST(3,4,2,20)", n(0.0)),
        ("=HYPGEOMDIST(0,4,18,20)", n(0.0)),
        ("=HYPGEOMDIST(-1,4,8,20)", error(ExcelErrorKind::Num)),
        ("=HYPGEOMDIST(1,21,8,20)", error(ExcelErrorKind::Num)),
        ("=HYPGEOMDIST(1,4,21,20)", error(ExcelErrorKind::Num)),
        ("=HYPGEOMDIST(1,0,8,20)", n(0.0)),
        ("=HYPGEOMDIST(0,4,0,20)", n(1.0)),
        ("=HYPGEOMDIST(1,4,8,0)", error(ExcelErrorKind::Num)),
        ("=HYPGEOMDIST(1,-1,8,20)", error(ExcelErrorKind::Num)),
        ("=HYPGEOMDIST(1,4,-1,20)", error(ExcelErrorKind::Num)),
        ("=HYPGEOMDIST(0,0,0,1)", n(1.0)),
        ("=HYPGEOMDIST(50,100,500,1000)", n(0.08389209209281297)),
    ]);
}

#[test]
fn negbinomdist() {
    assert_cases(&[
        ("=NEGBINOMDIST(10,5,0.25)", n(0.05504866037517785)),
        ("=NEGBINOMDIST(10.9,5.9,0.25)", n(0.05504866037517785)),
        ("=NEGBINOMDIST(0,1,0.5)", n(0.5)),
        ("=NEGBINOMDIST(1,1,0.5)", n(0.25)),
        ("=NEGBINOMDIST(0,2,0.5)", n(0.25)),
        ("=NEGBINOMDIST(-1,5,0.25)", error(ExcelErrorKind::Num)),
        ("=NEGBINOMDIST(10,0,0.25)", error(ExcelErrorKind::Num)),
        ("=NEGBINOMDIST(10,5,0)", error(ExcelErrorKind::Num)),
        ("=NEGBINOMDIST(10,5,1)", error(ExcelErrorKind::Num)),
        ("=NEGBINOMDIST(10,5,-0.1)", error(ExcelErrorKind::Num)),
        ("=NEGBINOMDIST(100,50,0.3)", n(0.015579745764339922)),
    ]);
}

#[test]
fn chitest() {
    assert_cases(&[
        (
            "=CHITEST({58,35;11,25;10,23},{45.35,47.65;17.56,18.44;16.09,16.91})",
            n(0.00030819201700830936),
        ),
        ("=CHITEST({10,20,30},{20,20,20})", n(0.006737946999085467)),
    ]);
}

#[test]
fn critbinom() {
    assert_cases(&[
        ("=CRITBINOM(6,0.5,0.75)", n(4.0)),
        ("=CRITBINOM(6.9,0.5,0.75)", n(4.0)),
        ("=CRITBINOM(6,0.5,0)", error(ExcelErrorKind::Num)),
        ("=CRITBINOM(6,0.5,1)", error(ExcelErrorKind::Num)),
        ("=CRITBINOM(0,0.5,0.5)", n(0.0)),
        ("=CRITBINOM(-1,0.5,0.5)", error(ExcelErrorKind::Num)),
        ("=CRITBINOM(6,0,0.5)", error(ExcelErrorKind::Num)),
        ("=CRITBINOM(6,1,0.5)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn ftest() {
    assert_cases(&[(
        "=FTEST({6,7,9,15,21},{20,28,31,38,40})",
        n(0.6483178467861743),
    )]);
}

#[test]
fn gammainv() {
    assert_cases(&[
        ("=GAMMAINV(0.068094,9,2)", n(10.00001119143718)),
        ("=GAMMAINV(0,9,2)", n(0.0)),
        ("=GAMMAINV(1,9,2)", error(ExcelErrorKind::Num)),
        ("=GAMMAINV(0.99,0.5,1)", n(3.317448300510606)),
        ("=GAMMAINV(0.5,9.5,2)", n(18.337652896756474)),
    ]);
}

#[test]
fn ttest() {
    assert_cases(&[(
        "=TTEST({3,4,5,8,9,1,2,4,5},{6,19,3,2,14,4,5,17,1},2,1)",
        n(0.19601578492528193),
    )]);
}

#[test]
fn ztest() {
    assert_cases(&[
        ("=ZTEST({3,6,7,8,6,5,4,2,1,9},4)", n(0.09057419685136381)),
        ("=ZTEST({3,6,7,8,6,5,4,2,1,9},6,2)", n(0.9226355382573109)),
    ]);
}

#[test]
fn tinv() {
    assert_cases(&[
        ("=TINV(0.05,10)", n(2.2281388519862744)),
        ("=TINV(0.05,10.9)", n(2.2281388519862744)),
        ("=TINV(1,10)", n(0.0)),
        ("=TINV(0,10)", error(ExcelErrorKind::Num)),
        ("=TINV(0.05,0.5)", error(ExcelErrorKind::Num)),
        ("=TINV(1.1,10)", n(-0.12889018929327162)),
        ("=TINV(0.001,1)", n(636.6192487687196)),
        ("=TINV(0.5,3)", n(0.7648923284043451)),
        ("=TINV(1.99,10)", n(-3.169272672616951)),
        ("=TINV(2,10)", error(ExcelErrorKind::Num)),
        ("=TINV(2.5,10)", error(ExcelErrorKind::Num)),
        ("=TINV(0.5,1E11)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn confidence() {
    assert_cases(&[
        ("=CONFIDENCE(0.05,2.5,50)", n(0.6929519121748386)),
        ("=CONFIDENCE(0.05,2.5,50.7)", n(0.6929519121748386)),
    ]);
}

#[test]
fn covar() {
    assert_cases(&[("=COVAR({3,2,4,5,6},{9,7,12,15,17})", n(5.2))]);
}

#[test]
fn forecast() {
    assert_cases(&[(
        "=FORECAST(30,{6,7,9,15,21},{20,28,31,38,40})",
        n(10.607253086419755),
    )]);
}

#[test]
fn mode() {
    assert_cases(&[("=MODE(5.6,4,4,3,2,4)", n(4.0))]);
}

#[test]
fn percentile() {
    assert_cases(&[("=PERCENTILE({1,3,2,4},0.3)", n(1.9))]);
}

#[test]
fn percentrank() {
    assert_cases(&[("=PERCENTRANK({13,12,11,8,4,3,2,1,1,1},2)", n(0.333))]);
}

#[test]
fn quartile() {
    assert_cases(&[("=QUARTILE({1,2,4,7,8,9,10,12},1)", n(3.5))]);
}

#[test]
fn stdev() {
    assert_cases(&[(
        "=STDEV(1345,1301,1368,1322,1310,1370,1318,1350,1303,1299)",
        n(27.46391571984349),
    )]);
}

#[test]
fn stdevp() {
    assert_cases(&[(
        "=STDEVP(1345,1301,1368,1322,1310,1370,1318,1350,1303,1299)",
        n(26.054558142482477),
    )]);
}

#[test]
fn var() {
    assert_cases(&[(
        "=VAR(1345,1301,1368,1322,1310,1370,1318,1350,1303,1299)",
        n(754.2666666666665),
    )]);
}

#[test]
fn varp() {
    assert_cases(&[(
        "=VARP(1345,1301,1368,1322,1310,1370,1318,1350,1303,1299)",
        n(678.8399999999999),
    )]);
}

#[test]
fn ceiling() {
    assert_cases(&[
        ("=CEILING(-2.5,-2)", n(-4.0)),
        ("=CEILING(-2.5,2)", n(-2.0)),
        ("=CEILING(2.5,-2)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn floor() {
    assert_cases(&[
        ("=FLOOR(-2.5,-2)", n(-2.0)),
        ("=FLOOR(2.5,-2)", error(ExcelErrorKind::Num)),
        ("=FLOOR(-2.5,2)", n(-4.0)),
    ]);
}

#[test]
fn concatenate() {
    assert_cases(&[("=CONCATENATE(\"a\",1,TRUE)", text("a1TRUE"))]);
}

#[test]
fn binom_inv() {
    assert_cases(&[
        ("=BINOM.INV(6,0.5,0)", error(ExcelErrorKind::Num)),
        ("=BINOM.INV(6,0.5,1)", error(ExcelErrorKind::Num)),
        ("=BINOM.INV(6.9,0.5,0.75)", n(4.0)),
        ("=BINOM.INV(6,0,0.5)", error(ExcelErrorKind::Num)),
        ("=BINOM.INV(6,1,0.5)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn t_inv_2t() {
    assert_cases(&[
        ("=T.INV.2T(0.05,10.9)", n(2.2281388519862744)),
        ("=T.INV.2T(1,10)", n(0.0)),
        ("=T.INV.2T(1.1,10)", n(-0.12889018929327162)),
        ("=T.INV.2T(2,10)", error(ExcelErrorKind::Num)),
        ("=T.INV.2T(0.05,10)", n(2.2281388519862744)),
    ]);
}

#[test]
fn gamma_inv() {
    assert_cases(&[
        ("=GAMMA.INV(1,9,2)", error(ExcelErrorKind::Num)),
        ("=GAMMA.INV(0.068094,9,2)", n(10.00001119143718)),
        ("=GAMMA.INV(0.5,1,1)", n(0.6931471805599453)),
    ]);
}

#[test]
fn t_dist_2t() {
    assert_cases(&[
        ("=T.DIST.2T(1,10.7)", n(0.34089313230205986)),
        ("=T.DIST.2T(-1,10)", error(ExcelErrorKind::Num)),
        ("=T.DIST.2T(1,1E11)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn t_dist_rt() {
    assert_cases(&[
        ("=T.DIST.RT(1,10.7)", n(0.17044656615102993)),
        ("=T.DIST.RT(-1,10)", n(0.8295534338489701)),
        ("=T.DIST.RT(1E-9,5)", n(0.5)),
        ("=T.DIST.RT(1,1E11)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn t_dist() {
    assert_cases(&[
        ("=T.DIST(1,10.7,TRUE)", n(0.8295534338489701)),
        ("=T.DIST(1,10.7,FALSE)", n(0.23036198922913867)),
        ("=T.DIST(1E-10,5,TRUE)", n(0.5)),
        ("=T.DIST(1,1E11,TRUE)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn chisq_dist_rt() {
    assert_cases(&[
        ("=CHISQ.DIST.RT(3,2.9)", n(0.22313016014842982)),
        ("=CHISQ.DIST.RT(1000,2)", n(7.124576406741286e-218)),
        ("=CHISQ.DIST.RT(3,1E11)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn chisq_dist() {
    assert_cases(&[
        ("=CHISQ.DIST(3,2.9,TRUE)", n(0.7768698398515702)),
        ("=CHISQ.DIST(3,2.9,FALSE)", n(0.11156508007421491)),
        ("=CHISQ.DIST(0,1,FALSE)", error(ExcelErrorKind::Num)),
        ("=CHISQ.DIST(0,2,FALSE)", n(0.5)),
        ("=CHISQ.DIST(0,3,FALSE)", n(0.0)),
        ("=CHISQ.DIST(0,4,FALSE)", n(0.0)),
        ("=CHISQ.DIST(3,1E11,TRUE)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn chisq_inv_rt() {
    assert_cases(&[
        ("=CHISQ.INV.RT(0.5,2.9)", n(1.3862943611198906)),
        ("=CHISQ.INV.RT(1,10)", n(0.0)),
    ]);
}

#[test]
fn chisq_inv() {
    assert_cases(&[
        ("=CHISQ.INV(0.5,2.9)", n(1.3862943611198906)),
        ("=CHISQ.INV(0,10)", n(0.0)),
        ("=CHISQ.INV(0.95,10)", n(18.30703805327514)),
        ("=CHISQ.INV(0.5,2)", n(1.3862943611198906)),
    ]);
}

#[test]
fn f_dist_rt() {
    assert_cases(&[
        ("=F.DIST.RT(2,6.9,4.9)", n(0.26171875)),
        ("=F.DIST.RT(1,1E11,4)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn f_dist() {
    assert_cases(&[
        ("=F.DIST(2,6.9,4.9,TRUE)", n(0.73828125)),
        ("=F.DIST(2,6.9,4.9,FALSE)", n(0.15820312499999997)),
        ("=F.DIST(0,2,4,FALSE)", n(1.0)),
        ("=F.DIST(0,1,4,FALSE)", error(ExcelErrorKind::Num)),
        ("=F.DIST(0,3,4,FALSE)", n(0.0)),
        ("=F.DIST(1,4,1E11,TRUE)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn f_inv_rt() {
    assert_cases(&[
        ("=F.INV.RT(0.05,6.9,4.9)", n(6.163132282688633)),
        ("=F.INV.RT(1,6,4)", n(0.0)),
    ]);
}

#[test]
fn t_inv() {
    assert_cases(&[
        ("=T.INV(0.975,10.9)", n(2.2281388519862744)),
        ("=T.INV(0.5,1)", n(0.0)),
        ("=T.INV(0.975,10)", n(2.2281388519862744)),
    ]);
}

#[test]
fn norm_s_dist() {
    assert_cases(&[("=NORM.S.DIST(-37.5,TRUE)", n(4.605353009581954e-308))]);
}

#[test]
fn gamma_dist() {
    assert_cases(&[
        ("=GAMMA.DIST(0,1,2,FALSE)", error(ExcelErrorKind::Num)),
        ("=GAMMA.DIST(0,0.5,2,FALSE)", error(ExcelErrorKind::Num)),
        ("=GAMMA.DIST(0,2,2,FALSE)", n(0.0)),
    ]);
}

#[test]
fn weibull_dist() {
    assert_cases(&[
        ("=WEIBULL.DIST(0,1,2,FALSE)", n(0.0)),
        ("=WEIBULL.DIST(0,0.5,2,FALSE)", n(0.0)),
        ("=WEIBULL.DIST(0,2,1,FALSE)", n(0.0)),
    ]);
}

#[test]
fn beta_inv() {
    assert_cases(&[
        ("=BETA.INV(1,2,3)", error(ExcelErrorKind::Num)),
        ("=BETA.INV(0,2,3)", error(ExcelErrorKind::Num)),
        ("=BETA.INV(0.5,2,3,0,)", n(0.3857275681323895)),
        ("=BETA.INV(0.5,2,2)", n(0.5)),
    ]);
}

#[test]
fn beta_dist() {
    assert_cases(&[
        ("=BETA.DIST(0.5,2,3,TRUE,0,)", n(0.6875)),
        ("=BETA.DIST(2,8,10,TRUE,1)", error(ExcelErrorKind::Num)),
        ("=BETA.DIST(0.5,2,3,FALSE,0,)", n(1.5)),
        ("=BETA.DIST(0,0.5,3,FALSE)", error(ExcelErrorKind::Num)),
        ("=BETA.DIST(0,1,3,FALSE)", error(ExcelErrorKind::Num)),
        ("=BETA.DIST(1,2,1,FALSE)", error(ExcelErrorKind::Num)),
        ("=BETA.DIST(1,2,3,FALSE)", n(0.0)),
        ("=BETA.DIST(0,2,3,FALSE)", n(0.0)),
    ]);
}

#[test]
fn hypgeom_dist() {
    assert_cases(&[
        ("=HYPGEOM.DIST(5,4,8,20,FALSE)", n(0.0)),
        ("=HYPGEOM.DIST(0,4,18,20,FALSE)", n(0.0)),
        ("=HYPGEOM.DIST(0,4,18,20,TRUE)", n(0.0)),
        ("=HYPGEOM.DIST(1,4,8,20,\"TRUE\")", n(0.46542827657378727)),
    ]);
}

#[test]
fn binom_dist() {
    assert_cases(&[
        ("=BINOM.DIST(0,10,0,FALSE)", n(1.0)),
        ("=BINOM.DIST(10,10,1,TRUE)", n(1.0)),
        ("=BINOM.DIST(6,10,0.5,\"FALSE\")", n(0.20507812500000006)),
    ]);
}

#[test]
fn poisson_dist() {
    assert_cases(&[
        ("=POISSON.DIST(0,0,FALSE)", n(1.0)),
        ("=POISSON.DIST(2,0,FALSE)", n(0.0)),
    ]);
}

#[test]
fn norm_dist() {
    assert_cases(&[("=NORM.DIST(1,0,1,\"TRUE\")", n(0.841344746068543))]);
}

#[test]
fn negbinom_dist() {
    assert_cases(&[
        ("=NEGBINOM.DIST(0,1,0.5,FALSE)", n(0.5)),
        ("=NEGBINOM.DIST(10,5,0.25,\"TRUE\")", n(0.31351405847817665)),
    ]);
}

#[test]
fn chisq_test() {
    assert_cases(&[
        (
            "=CHISQ.TEST({10,20,30},{20,20,20})",
            n(0.006737946999085467),
        ),
        (
            "=CHISQ.TEST({10;20;30},{20;20;20})",
            n(0.006737946999085467),
        ),
        (
            "=CHISQ.TEST({58,35;11,25;10,23},{45.35,47.65;17.56,18.44;16.09,16.91})",
            n(0.00030819201700830936),
        ),
        ("=CHISQ.TEST({1,2},{1,2,3})", error(ExcelErrorKind::Na)),
        ("=CHISQ.TEST({10},{20})", error(ExcelErrorKind::Na)),
        ("=CHISQ.TEST({18,22},{20,20})", n(0.5270892568655381)),
    ]);
}

#[test]
fn z_test() {
    assert_cases(&[
        ("=Z.TEST({3,6,7,8,6,5,4,2,1,9},4)", n(0.09057419685136381)),
        ("=Z.TEST({3,6,7,8,6,5,4,2,1,9},6,2)", n(0.9226355382573109)),
        ("=Z.TEST({1,2,3,4,5},2,1)", n(0.012673659338734126)),
        ("=Z.TEST({1,2,3,4,5},2)", n(0.07864960352514257)),
        ("=Z.TEST({5},4)", error(ExcelErrorKind::Div)),
        ("=Z.TEST({1,2,3,4,5},2,)", error(ExcelErrorKind::Num)),
    ]);
}

#[test]
fn f_inv() {
    assert_cases(&[
        ("=F.INV(0.95,6.9,4.9)", n(6.163132282688627)),
        ("=F.INV(0,6,4)", n(0.0)),
        ("=F.INV(0.95,5,10)", n(3.3258345304130126)),
    ]);
}

#[test]
fn confidence_norm() {
    assert_cases(&[
        ("=CONFIDENCE.NORM(0.05,2.5,50)", n(0.6929519121748386)),
        ("=CONFIDENCE.NORM(0.05,2.5,50.7)", n(0.6929519121748386)),
    ]);
}

#[test]
fn norm_s_inv() {
    assert_cases(&[("=NORM.S.INV(0.975)", n(1.9599639845400536))]);
}

#[test]
fn norm_inv() {
    assert_cases(&[
        ("=NORM.INV(0.908789,40,1.5)", n(42.00000200956616)),
        ("=NORM.INV(0.841344746068543,0,1)", n(0.9999999999999996)),
    ]);
}

#[test]
fn lognorm_inv() {
    assert_cases(&[
        ("=LOGNORM.INV(0.039084,3.5,1.2)", n(4.000025218680638)),
        ("=LOGNORM.INV(0.841344746068543,0,1)", n(2.718281828459044)),
    ]);
}

#[test]
fn lognorm_dist() {
    assert_cases(&[("=LOGNORM.DIST(4,3.5,1.2,\"TRUE\")", n(0.03908355570680048))]);
}

#[test]
fn expon_dist() {
    assert_cases(&[("=EXPON.DIST(0.2,10,\"TRUE\")", n(0.8646647167633873))]);
}

#[test]
fn erfc() {
    assert_cases(&[
        ("=ERFC(4)", n(1.5417257900280017e-08)),
        ("=ERFC(5)", n(1.537459794428034e-12)),
        ("=ERFC(6.5)", n(3.8421483271206487e-20)),
        ("=ERFC(10)", n(2.0884875837625446e-45)),
        ("=ERFC(26)", n(5.663192408856144e-296)),
        ("=ERFC(-5)", n(1.9999999999984626)),
    ]);
}

#[test]
fn erfc_precise() {
    assert_cases(&[("=ERFC.PRECISE(5)", n(1.537459794428034e-12))]);
}

#[test]
fn erf() {
    assert_cases(&[
        ("=ERF(4.5)", n(0.9999999998033839)),
        ("=ERF(1,5)", n(0.15729920704874767)),
    ]);
}

#[test]
fn confidence_t() {
    assert_cases(&[
        ("=CONFIDENCE.T(0.05,2,25)", n(0.8255594246512102)),
        ("=CONFIDENCE.T(0.1,5,10)", n(2.898406037752281)),
        ("=CONFIDENCE.T(0.05,2,25.7)", n(0.8255594246512102)),
    ]);
}

#[test]
fn f_test() {
    assert_cases(&[("=F.TEST({1,2,3,4},{1,1,1,5})", n(0.4909417371545593))]);
}

#[test]
fn t_test() {
    assert_cases(&[
        (
            "=T.TEST({1,2,3,4},{2,4,6,9,12},2,3)",
            n(0.08226778382678882),
        ),
        (
            "=T.TEST({1,2,3,4},{2,4,6,9,12},1,2)",
            n(0.045537276809267065),
        ),
    ]);
}

#[test]
fn binom_dist_range() {
    assert_cases(&[
        ("=BINOM.DIST.RANGE(10,0,0)", n(1.0)),
        ("=BINOM.DIST.RANGE(10,1,10)", n(1.0)),
        ("=BINOM.DIST.RANGE(60,0.75,48)", n(0.0839749674290475)),
    ]);
}

#[test]
fn rank_reads_its_reference() {
    let actual = eval_with(
        "=RANK(A3,A1:A5,1)",
        &[
            (1, 1, 7.0),
            (2, 1, 3.5),
            (3, 1, 3.5),
            (4, 1, 1.0),
            (5, 1, 2.0),
        ],
    );
    assert!(same(&actual, &n(3.0)), "{actual:?}");
}

/// Where Excel's own inverse loses precision at enormous degrees of freedom,
/// the fork keeps the true quantile (mpmath for the F row; the median of
/// chi-square(1E11) is 1E11 - 2/3 to 1E-12).
#[test]
fn inverses_keep_the_quantile_where_excel_loses_it() {
    assert_cases(&[
        // Excel 0.8391734532306122, 5.0E-8 off
        ("=FINV(0.5,4,1E10)", n(0.8391734950652554)),
        // Excel 100026582354.38625, 2.7E-4 off
        ("=CHIINV(0.5,1E11)", n(99999999999.33333)),
        // Excel 100026582354.38625, 2.7E-4 off
        ("=CHISQ.INV.RT(0.5,1E11)", n(99999999999.33333)),
    ]);
}
