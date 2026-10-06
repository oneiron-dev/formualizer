//! Which formulas Excel saves as calculated always (`ca="1"`).
//!
//! After a full calculation Excel saves `ca="1"` on a formula that
//! - calls a volatile function (NOW, TODAY, RAND, RANDBETWEEN, RANDARRAY,
//!   OFFSET, INDIRECT, CELL, INFO, FORMULATEXT), directly or through a
//!   defined name;
//! - calls a function Excel does not have: a user-defined (`_xludf.`, add-in
//!   `_xll.` or VBA) function or another application's function, whose
//!   value is #NAME?;
//! - sums or averages a range whose size differs from its criteria range
//!   (SUMIF, AVERAGEIF), which reads cells outside its references; the
//!   ranges are those the formula evaluates at its cell, so each cell of a
//!   shared formula is judged on its own;
//! - reads a calculated-always cell through a cell, range, name or table
//!   reference, on any sheet; a defined name reads the cells its formula
//!   refers to.
//!
//! A formula uses the defined name its sheet sees: the sheet's own name of
//! that spelling, else the workbook's (`localSheetId`). A workbook-level
//! name's formula is read on the sheet of the formula using it.
//!
//! Each member of a multi-cell array formula carries its own flag, the empty
//! `<f ca="1"/>`: every member when the array formula itself is calculated
//! always, otherwise the members whose own elements of the referenced ranges
//! are (a range as tall or as wide as the array lines up with its rows or
//! columns; any other reference reaches every member).
//!
//! Excel flags readers as it evaluates them, so a reference in a branch of
//! IF or CHOOSE that is not taken, or an unknown function there, flags
//! nothing. The dependency graph used here holds every reference, so such a
//! reader is flagged as well. Flags the file already carries are kept and
//! spread to their readers; an array formula the file flags without reading
//! a flagged cell is taken to be calculated always on its own.
use super::{IoError, package, sheet, unsupported};
use crate::workbook::WBResolver;
use formualizer_common::CellAddress;
use formualizer_eval::engine::Engine;
use formualizer_eval::engine::inspect::{
    DependentsOptions, NameResolution, PrecedentOptions, SemanticReference,
};
use formualizer_eval::interpreter::Interpreter;
use formualizer_eval::{CellRef, Coord};
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};
use formualizer_parse::{TokenSubType, TokenType, Tokenizer};
use std::collections::{HashMap, HashSet};

/// Functions whose formulas Excel recalculates at every calculation.
const VOLATILE: &[&str] = &[
    "CELL",
    "FORMULATEXT",
    "INDIRECT",
    "INFO",
    "NOW",
    "OFFSET",
    "RAND",
    "RANDARRAY",
    "RANDBETWEEN",
    "TODAY",
];
/// The functions a file names without a prefix, those of the Excel 2007 file
/// format; functions added later are written `_xlfn.NAME`, so any other bare
/// name is not one of Excel's functions. Sorted for binary search.
#[rustfmt::skip]
const BUILT_IN: &[&str] = &[
    "ABS", "ACCRINT", "ACCRINTM", "ACOS", "ACOSH", "ADDRESS", "AMORDEGRC", "AMORLINC", "AND",
    "AREAS", "ASC", "ASIN", "ASINH", "ATAN", "ATAN2", "ATANH", "AVEDEV", "AVERAGE", "AVERAGEA",
    "AVERAGEIF", "AVERAGEIFS", "BAHTTEXT", "BESSELI", "BESSELJ", "BESSELK", "BESSELY", "BETADIST",
    "BETAINV", "BIN2DEC", "BIN2HEX", "BIN2OCT", "BINOMDIST", "CEILING", "CELL", "CHAR", "CHIDIST",
    "CHIINV", "CHITEST", "CHOOSE", "CLEAN", "CODE", "COLUMN", "COLUMNS", "COMBIN", "COMPLEX",
    "CONCATENATE", "CONFIDENCE", "CONVERT", "CORREL", "COS", "COSH", "COUNT", "COUNTA",
    "COUNTBLANK", "COUNTIF", "COUNTIFS", "COUPDAYBS", "COUPDAYS", "COUPDAYSNC", "COUPNCD",
    "COUPNUM", "COUPPCD", "COVAR", "CRITBINOM", "CUBEKPIMEMBER", "CUBEMEMBER",
    "CUBEMEMBERPROPERTY", "CUBERANKEDMEMBER", "CUBESET", "CUBESETCOUNT", "CUBEVALUE", "CUMIPMT",
    "CUMPRINC", "DATE", "DATEDIF", "DATEVALUE", "DAVERAGE", "DAY", "DAYS360", "DB", "DCOUNT",
    "DCOUNTA", "DDB", "DEC2BIN", "DEC2HEX", "DEC2OCT", "DEGREES", "DELTA", "DEVSQ", "DGET",
    "DISC", "DMAX", "DMIN", "DOLLAR", "DOLLARDE", "DOLLARFR", "DPRODUCT", "DSTDEV", "DSTDEVP",
    "DSUM", "DURATION", "DVAR", "DVARP", "ECMA.CEILING", "EDATE", "EFFECT", "EOMONTH", "ERF",
    "ERFC", "ERROR.TYPE", "EVEN", "EXACT", "EXP", "EXPONDIST", "FACT", "FACTDOUBLE", "FALSE",
    "FDIST", "FIND", "FINDB", "FINV", "FISHER", "FISHERINV", "FIXED", "FLOOR", "FORECAST",
    "FREQUENCY", "FTEST", "FV", "FVSCHEDULE", "GAMMADIST", "GAMMAINV", "GAMMALN", "GCD",
    "GEOMEAN", "GESTEP", "GETPIVOTDATA", "GROWTH", "HARMEAN", "HEX2BIN", "HEX2DEC", "HEX2OCT",
    "HLOOKUP", "HOUR", "HYPERLINK", "HYPGEOMDIST", "IF", "IFERROR", "IMABS", "IMAGINARY",
    "IMARGUMENT", "IMCONJUGATE", "IMCOS", "IMDIV", "IMEXP", "IMLN", "IMLOG10", "IMLOG2",
    "IMPOWER", "IMPRODUCT", "IMREAL", "IMSIN", "IMSQRT", "IMSUB", "IMSUM", "INDEX", "INDIRECT",
    "INFO", "INT", "INTERCEPT", "INTRATE", "IPMT", "IRR", "ISBLANK", "ISERR", "ISERROR", "ISEVEN",
    "ISLOGICAL", "ISNA", "ISNONTEXT", "ISNUMBER", "ISO.CEILING", "ISODD", "ISPMT", "ISREF",
    "ISTEXT", "JIS", "KURT", "LARGE", "LCM", "LEFT", "LEFTB", "LEN", "LENB", "LINEST", "LN",
    "LOG", "LOG10", "LOGEST", "LOGINV", "LOGNORMDIST", "LOOKUP", "LOWER", "MATCH", "MAX", "MAXA",
    "MDETERM", "MDURATION", "MEDIAN", "MID", "MIDB", "MIN", "MINA", "MINUTE", "MINVERSE", "MIRR",
    "MMULT", "MOD", "MODE", "MONTH", "MROUND", "MULTINOMIAL", "N", "NA", "NEGBINOMDIST",
    "NETWORKDAYS", "NETWORKDAYS.INTL", "NOMINAL", "NORMDIST", "NORMINV", "NORMSDIST", "NORMSINV",
    "NOT", "NOW", "NPER", "NPV", "OCT2BIN", "OCT2DEC", "OCT2HEX", "ODD", "ODDFPRICE", "ODDFYIELD",
    "ODDLPRICE", "ODDLYIELD", "OFFSET", "OR", "PEARSON", "PERCENTILE", "PERCENTRANK", "PERMUT",
    "PHONETIC", "PI", "PMT", "POISSON", "POWER", "PPMT", "PRICE", "PRICEDISC", "PRICEMAT", "PROB",
    "PRODUCT", "PROPER", "PV", "QUARTILE", "QUOTIENT", "RADIANS", "RAND", "RANDBETWEEN", "RANK",
    "RATE", "RECEIVED", "REPLACE", "REPLACEB", "REPT", "RIGHT", "RIGHTB", "ROMAN", "ROUND",
    "ROUNDDOWN", "ROUNDUP", "ROW", "ROWS", "RSQ", "RTD", "SEARCH", "SEARCHB", "SECOND",
    "SERIESSUM", "SIGN", "SIN", "SINH", "SKEW", "SLN", "SLOPE", "SMALL", "SQRT", "SQRTPI",
    "STANDARDIZE", "STDEV", "STDEVA", "STDEVP", "STDEVPA", "STEYX", "SUBSTITUTE", "SUBTOTAL",
    "SUM", "SUMIF", "SUMIFS", "SUMPRODUCT", "SUMSQ", "SUMX2MY2", "SUMX2PY2", "SUMXMY2", "SYD",
    "T", "TAN", "TANH", "TBILLEQ", "TBILLPRICE", "TBILLYIELD", "TDIST", "TEXT", "TIME",
    "TIMEVALUE", "TINV", "TODAY", "TRANSPOSE", "TREND", "TRIM", "TRIMMEAN", "TRUE", "TRUNC",
    "TTEST", "TYPE", "UPPER", "VALUE", "VAR", "VARA", "VARP", "VARPA", "VDB", "VLOOKUP",
    "WEEKDAY", "WEEKNUM", "WEIBULL", "WORKDAY", "WORKDAY.INTL", "XIRR", "XNPV", "YEAR",
    "YEARFRAC", "YIELD", "YIELDDISC", "YIELDMAT", "ZTEST",
];

/// Whether `name`, a function name as a file writes it, names one of Excel's
/// functions: a bare name of the Excel 2007 file format's functions, or a
/// name with the `_xlfn.` (or `_xlfn._xlws.`) prefix of those added since. A
/// user-defined (`_xludf.`), add-in (`_xll.`) or other bare name is not:
/// without the workbook, add-in or application that defines it Excel
/// evaluates it to #NAME? (Excel for Windows 16.0.20430 on the
/// SpreadsheetBench corpus: IMAGE written without its prefix, ClrCnt, EOM,
/// arrayformula and __xludf.DUMMYFUNCTION are #NAME?, which IFERROR and the
/// criteria of SUMIFS see).
pub fn is_excel_function(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("_XLUDF.") || upper.starts_with("_XLL.") {
        return false;
    }
    ["_XLFN.", "_XLWS."].iter().any(|p| upper.starts_with(p))
        || BUILT_IN.binary_search(&upper.as_str()).is_ok()
}

/// Calculated-always flags per sheet, for each formula cell of
/// [`sheet::Scan::cells`] and each member of [`sheet::Scan::members`].
pub(super) struct CalcAlways {
    pub cells: Vec<Vec<bool>>,
    pub members: Vec<Vec<bool>>,
}

/// An area a formula reads: sheet index, first row, first column, last row
/// and last column (1-based).
type Area = (usize, u32, u32, u32, u32);

/// A formula cell or an array member, by index into its sheet's scan.
enum Slot {
    Cell(usize),
    Member(usize),
}

/// Where the formulas of each sheet find the workbook's defined names.
struct Scopes {
    /// Index into the defined names by the sheet a name is local to (`None`:
    /// the workbook) and its lowercase spelling.
    names: HashMap<(Option<usize>, String), usize>,
    /// Sheet indexes by lowercase sheet name.
    sheets: HashMap<String, usize>,
}

impl Scopes {
    fn new<'a>(
        sheets: impl IntoIterator<Item = &'a str>,
        defined: &[package::DefinedName],
    ) -> Self {
        Self {
            names: defined
                .iter()
                .enumerate()
                .map(|(i, n)| ((n.sheet, n.name.to_ascii_lowercase()), i))
                .collect(),
            sheets: sheets
                .into_iter()
                .enumerate()
                .map(|(i, name)| (name.to_lowercase(), i))
                .collect(),
        }
    }

    /// The defined name a formula on `sheet` means by `text` (`Name`,
    /// `Sheet1!Name`): the sheet's own name of that spelling, else the
    /// workbook's (ECMA-376 18.2.5, `localSheetId`).
    fn resolve(&self, text: &str, sheet: usize) -> Option<usize> {
        if self.names.is_empty() {
            return None;
        }
        let (sheet, name) = match text.rsplit_once('!') {
            Some((qualifier, name)) => {
                let qualifier = qualifier
                    .strip_prefix('\'')
                    .and_then(|q| q.strip_suffix('\''))
                    .map_or_else(|| qualifier.to_owned(), |q| q.replace("''", "'"));
                (self.sheets.get(&qualifier.to_lowercase()).copied(), name)
            }
            None => (Some(sheet), text),
        };
        let name = name.to_ascii_lowercase();
        sheet
            .and_then(|s| self.names.get(&(Some(s), name.clone())))
            .or_else(|| self.names.get(&(None, name)))
            .copied()
    }
}

/// What a formula's text makes of it, read on a sheet.
#[derive(Default)]
struct Text {
    /// It calls a volatile function or one Excel does not have, or uses a
    /// calculated-always name.
    calc_always: bool,
    /// It uses a defined name.
    names: bool,
    /// It calls SUMIF or AVERAGEIF, whose sum range may be resized.
    sums: bool,
}

/// The workbook's defined names, as the formulas of each sheet see them.
struct Names<'a> {
    engine: &'a Engine<WBResolver>,
    sheets: &'a [package::Sheet],
    defined: &'a [package::DefinedName],
    scopes: Scopes,
    /// Whether a name is calculated always, by name and the sheet its
    /// formula is read on.
    calc_always: HashMap<(usize, usize), bool>,
}

impl Names<'_> {
    /// Whether name `i`, used by a formula on `sheet`, is calculated always.
    /// Its formula is read where it is evaluated: a sheet's own name on that
    /// sheet, a workbook-level name on the sheet using it.
    fn calculated_always(&mut self, i: usize, sheet: usize) -> bool {
        let defined = self.defined;
        let place = defined[i].sheet.unwrap_or(sheet);
        if let Some(&flag) = self.calc_always.get(&(i, place)) {
            return flag;
        }
        // A name reaching itself is not calculated always through itself.
        self.calc_always.insert((i, place), false);
        let formula = &defined[i].formula;
        let text = self.read(formula, place);
        let flag = text.calc_always
            || (text.sums
                && formualizer_parse::parse(format!("={formula}"))
                    .is_ok_and(|ast| resizes(self.engine, &ast, &self.sheets[place].name, 1, 1)));
        self.calc_always.insert((i, place), flag);
        flag
    }

    /// What the text of a formula on `sheet` calls and names.
    fn read(&mut self, text: &str, sheet: usize) -> Text {
        let mut out = Text::default();
        for token in Tokenizer::new_best_effort(&format!("={text}")).items {
            match (token.token_type, token.subtype) {
                (TokenType::Func, TokenSubType::Open) => {
                    let name = token.value.trim_end_matches('(');
                    out.sums |= ["SUMIF", "AVERAGEIF"]
                        .iter()
                        .any(|f| name.eq_ignore_ascii_case(f));
                    out.calc_always = out.calc_always || self.calls_calculated_always(name, sheet);
                }
                (TokenType::Operand, TokenSubType::Range) => {
                    if let Some(i) = self.scopes.resolve(&token.value, sheet) {
                        out.names = true;
                        out.calc_always = out.calc_always || self.calculated_always(i, sheet);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Whether calling `name` from a formula on `sheet` makes it calculated
    /// always: a volatile function, a calculated-always name, or a function
    /// Excel does not have (a user-defined `_xludf.` or add-in `_xll.`
    /// function, or a bare name that is neither one of the file format's
    /// functions nor a defined name the sheet sees).
    fn calls_calculated_always(&mut self, name: &str, sheet: usize) -> bool {
        // A call of a LAMBDA value, `LAMBDA(x,x)(1)`, names no function.
        if name.is_empty() {
            return false;
        }
        let upper = name.to_ascii_uppercase();
        if upper.starts_with("_XLUDF.") || upper.starts_with("_XLL.") {
            return true;
        }
        let (bare, prefixed) = ["_XLFN._XLWS.", "_XLFN.", "_XLWS."]
            .iter()
            .find_map(|p| upper.strip_prefix(p))
            .map_or((upper.as_str(), false), |bare| (bare, true));
        if VOLATILE.contains(&bare) {
            return true;
        }
        if prefixed || bare.starts_with("_XLPM.") || BUILT_IN.binary_search(&bare).is_ok() {
            return false;
        }
        self.scopes
            .resolve(name, sheet)
            .is_none_or(|i| self.calculated_always(i, sheet))
    }
}

/// Whether a SUMIF or AVERAGEIF in `ast`, evaluated at `row` and `col` of
/// `sheet`, sums a range sized unlike its criteria range. Excel then sums the
/// criteria range's shape from the sum range's top-left cell, cells outside
/// the references it names, and calculates the formula always. The ranges
/// are sized as they evaluate there: through names, tables and functions
/// returning references, and for each cell of a shared formula on its own.
fn resizes(engine: &Engine<WBResolver>, ast: &ASTNode, sheet: &str, row: u32, col: u32) -> bool {
    fn walk(node: &ASTNode, resized: &dyn Fn(&[ASTNode]) -> bool) -> bool {
        match &node.node_type {
            ASTNodeType::Function { name, args } => {
                (["SUMIF", "AVERAGEIF"]
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f))
                    && resized(args))
                    || args.iter().any(|arg| walk(arg, resized))
            }
            ASTNodeType::Call { callee, args } => {
                walk(callee, resized) || args.iter().any(|arg| walk(arg, resized))
            }
            ASTNodeType::UnaryOp { expr, .. } => walk(expr, resized),
            ASTNodeType::BinaryOp { left, right, .. } => {
                walk(left, resized) || walk(right, resized)
            }
            ASTNodeType::Array(rows) => rows.iter().flatten().any(|item| walk(item, resized)),
            _ => false,
        }
    }
    let Some(sheet_id) = engine.sheet_id(sheet) else {
        return false;
    };
    let interpreter = Interpreter::new_with_cell(
        engine,
        sheet,
        CellRef::new(sheet_id, Coord::from_excel(row, col, true, true)),
    );
    let span = |first: Option<u32>, last: Option<u32>, all: u32| match (first, last) {
        (Some(first), Some(last)) => first.abs_diff(last) + 1,
        (Some(first), None) => all.saturating_sub(first) + 1,
        (None, Some(last)) => last,
        (None, None) => all,
    };
    let size = |node: &ASTNode| {
        let reference = interpreter.evaluate_ast_as_reference(node).ok()?;
        match &reference {
            ReferenceType::Cell { .. } => Some((1, 1)),
            ReferenceType::Range {
                start_row,
                start_col,
                end_row,
                end_col,
                ..
            } => Some((
                span(*start_row, *end_row, 1_048_576),
                span(*start_col, *end_col, 16_384),
            )),
            ReferenceType::Table(_) => {
                let (rows, cols) = interpreter
                    .resolve_range_view(&reference, sheet)
                    .ok()?
                    .dims();
                Some((u32::try_from(rows).ok()?, u32::try_from(cols).ok()?))
            }
            _ => None,
        }
    };
    walk(
        ast,
        &|args| matches!(args, [range, _, sum] if size(range).zip(size(sum)).is_some_and(|(a, b)| a != b)),
    )
}

/// [`resizes`] for the formula the engine holds at a cell.
fn resizes_at(engine: &Engine<WBResolver>, sheet: &str, row: u32, col: u32) -> bool {
    matches!(engine.get_cell(sheet, row, col), Some((Some(ast), _)) if resizes(engine, &ast, sheet, row, col))
}

fn inspect_error(error: impl std::fmt::Display) -> IoError {
    unsupported(
        format!("dependency inspection: {error}"),
        "calculate-always flags",
    )
}

fn precedent_options() -> PrecedentOptions {
    PrecedentOptions::default()
        .with_max_links(u32::MAX)
        .with_max_work(u64::MAX)
}

/// The areas a formula reads through cell, range, name and table references,
/// resolved by the engine; with `names_only`, only those through names.
fn precedent_areas(
    engine: &Engine<WBResolver>,
    address: &CellAddress,
    sheet_index: &HashMap<&str, usize>,
    names_only: bool,
) -> Result<Vec<Area>, IoError> {
    let report = engine
        .precedents(address, &precedent_options())
        .map_err(inspect_error)?;
    let mut areas = Vec::new();
    let mut seen = HashSet::new();
    for precedent in &report.precedents {
        if !names_only || matches!(precedent.reference, SemanticReference::Name { .. }) {
            reference_areas(
                engine,
                &precedent.reference,
                address,
                sheet_index,
                &mut seen,
                &mut areas,
            )?;
        }
    }
    Ok(areas)
}

/// Add the areas a reference read by a formula at `at` stands for. The
/// cells a name's formula refers to are read by every formula using the
/// name, through the names it uses in turn (`seen`: the formula names
/// followed, by the sheet they are read from).
fn reference_areas(
    engine: &Engine<WBResolver>,
    reference: &SemanticReference,
    at: &CellAddress,
    sheet_index: &HashMap<&str, usize>,
    seen: &mut HashSet<(String, String)>,
    areas: &mut Vec<Area>,
) -> Result<(), IoError> {
    let cell = |c: &CellAddress| {
        Some((
            *sheet_index.get(c.sheet.as_str())?,
            c.row,
            c.column,
            c.row,
            c.column,
        ))
    };
    let range = |r: &formualizer_common::RangeAddress| {
        Some((
            *sheet_index.get(r.sheet.as_str())?,
            r.start_row,
            r.start_col,
            r.end_row,
            r.end_col,
        ))
    };
    let declared = |d: &formualizer_common::RangeArea| {
        Some((
            *sheet_index.get(d.sheet.as_str())?,
            d.start_row.unwrap_or(1),
            d.start_column.unwrap_or(1),
            d.end_row.unwrap_or(1_048_576),
            d.end_column.unwrap_or(16_384),
        ))
    };
    let area = match reference {
        SemanticReference::Cell(c)
        | SemanticReference::Name {
            resolution: NameResolution::Cell(c),
            ..
        } => cell(c),
        SemanticReference::Range {
            declared: d,
            resolved,
            ..
        }
        | SemanticReference::Name {
            resolution:
                NameResolution::Range {
                    declared: d,
                    resolved,
                },
            ..
        } => resolved.as_ref().map_or_else(|| declared(d), range),
        SemanticReference::Table { resolved, .. } => range(resolved),
        SemanticReference::Name {
            name,
            resolution: NameResolution::Formula { .. },
        } => {
            if seen.insert((at.sheet.clone(), name.to_ascii_lowercase())) {
                let report = engine
                    .name_precedents(at, name, &precedent_options())
                    .map_err(inspect_error)?;
                for precedent in &report.precedents {
                    reference_areas(
                        engine,
                        &precedent.reference,
                        &report.cell,
                        sheet_index,
                        seen,
                        areas,
                    )?;
                }
            }
            None
        }
        _ => None,
    };
    areas.extend(area);
    Ok(())
}

fn contains(&(a, top, left, bottom, right): &Area, (s, row, col): (usize, u32, u32)) -> bool {
    a == s && (top..=bottom).contains(&row) && (left..=right).contains(&col)
}

/// The formulas reading an area through names: the formula cells, flagged
/// once, and the array formulas, whose members line up with each flagged
/// cell of the area they read.
#[derive(Default)]
struct NamedReaders {
    cells: Vec<(usize, usize)>,
    arrays: Vec<(usize, usize)>,
}

/// The calculated-always formulas and array members of the evaluated
/// workbook: those the file flags and those the rule above flags.
pub(super) fn calc_always(
    engine: &Engine<WBResolver>,
    sheets: &[package::Sheet],
    plans: &[(Vec<u8>, sheet::Scan)],
    defined_names: &[package::DefinedName],
) -> Result<CalcAlways, IoError> {
    let mut names = Names {
        engine,
        sheets,
        defined: defined_names,
        scopes: Scopes::new(sheets.iter().map(|s| s.name.as_str()), defined_names),
        calc_always: HashMap::new(),
    };
    let mut flags = CalcAlways {
        cells: plans
            .iter()
            .map(|(_, s)| vec![false; s.cells.len()])
            .collect(),
        members: plans
            .iter()
            .map(|(_, s)| vec![false; s.members.len()])
            .collect(),
    };
    let mut queue = Vec::new();
    // Multi-cell array formulas the file flags, unless they read flagged
    // cells: those are calculated always on their own (below).
    let mut flagged_arrays = Vec::new();
    // The formulas using a defined name, per sheet.
    let mut uses_names = Vec::new();
    for (s, (_, scan)) in plans.iter().enumerate() {
        // A shared formula's descendants have their master's text, and what
        // it calls and names is the same wherever it is evaluated.
        let texts: Vec<Option<Text>> = scan
            .cells
            .iter()
            .map(|cell| {
                (!cell.formula_text.trim().is_empty()).then(|| names.read(&cell.formula_text, s))
            })
            .collect();
        let mut uses = Vec::with_capacity(scan.cells.len());
        for (i, cell) in scan.cells.iter().enumerate() {
            let text = texts[cell.shared_master.unwrap_or(i)].as_ref();
            uses.push(text.is_some_and(|t| t.names));
            // The ranges SUMIF sums are not: each cell is judged on its own.
            let own = text.is_some_and(|t| {
                t.calc_always || (t.sums && resizes_at(engine, &sheets[s].name, cell.row, cell.col))
            });
            if own || (cell.calc_always && cell.array_extent.is_none()) {
                flags.cells[s][i] = true;
                queue.push((s, cell.row, cell.col));
            } else if cell.calc_always {
                flagged_arrays.push((s, i));
            }
        }
        uses_names.push(uses);
        // A calculated-always array formula flags each of its members; the
        // file's own member flags are kept.
        for (j, member) in scan.members.iter().enumerate() {
            if member.marked || flags.cells[s][member.anchor] {
                flags.members[s][j] = true;
                queue.push((s, member.cell.row, member.cell.col));
            }
        }
    }
    if queue.is_empty() && flagged_arrays.is_empty() {
        return Ok(flags);
    }
    let sheet_index: HashMap<&str, usize> = sheets
        .iter()
        .enumerate()
        .map(|(i, sheet)| (sheet.name.as_str(), i))
        .collect();
    let address = |s: usize, row: u32, col: u32| {
        CellAddress::new(&sheets[s].name, row, col)
            .map_err(|e| IoError::from_backend("xlsx-coordinate", e))
    };
    let mut slots = HashMap::new();
    // The engine's dependency report stops at a defined name, so the readers
    // of each area read through names are collected from the formulas that
    // use one.
    let mut named: HashMap<Area, NamedReaders> = HashMap::new();
    for (s, (_, scan)) in plans.iter().enumerate() {
        for (i, cell) in scan.cells.iter().enumerate() {
            slots.insert((s, cell.row, cell.col), Slot::Cell(i));
            if uses_names[s][i] {
                let at = address(s, cell.row, cell.col)?;
                for area in precedent_areas(engine, &at, &sheet_index, true)? {
                    let readers = named.entry(area).or_default();
                    if cell.array_extent.is_some() {
                        readers.arrays.push((s, i));
                    } else {
                        readers.cells.push((s, i));
                    }
                }
            }
        }
        for (j, member) in scan.members.iter().enumerate() {
            slots.insert((s, member.cell.row, member.cell.col), Slot::Member(j));
        }
    }
    let mut named: Vec<_> = named.into_iter().collect();
    let mut reads: HashMap<(usize, usize), Vec<Area>> = HashMap::new();
    let mut whole = HashSet::new();
    let options = DependentsOptions::default()
        .with_max_results(u32::MAX)
        .with_max_work(u64::MAX);
    loop {
        let Some((s, row, col)) = queue.pop() else {
            // A flagged array formula that reads no flagged cell flags all
            // of its members, as a volatile one does.
            for (s, i) in flagged_arrays.drain(..) {
                if !flags.cells[s][i] {
                    flags.cells[s][i] = true;
                    let cell = &plans[s].1.cells[i];
                    queue.push((s, cell.row, cell.col));
                    for (j, member) in plans[s].1.members.iter().enumerate() {
                        if member.anchor == i && !flags.members[s][j] {
                            flags.members[s][j] = true;
                            queue.push((s, member.cell.row, member.cell.col));
                        }
                    }
                }
            }
            if queue.is_empty() {
                break;
            }
            continue;
        };
        let at = address(s, row, col)?;
        let report = engine.dependents(&at, &options).map_err(inspect_error)?;
        let mut readers = Vec::new();
        for dependent in report.dependents {
            // An array's anchor query also finds the readers of its members.
            if !dependent.via.is_empty() && !dependent.via.contains(&at) {
                continue;
            }
            if let Some(&t) = sheet_index.get(dependent.cell.sheet.as_str())
                && let Some(Slot::Cell(i)) =
                    slots.get(&(t, dependent.cell.row, dependent.cell.column))
            {
                readers.push((t, *i));
            }
        }
        for (area, found) in &mut named {
            if contains(area, (s, row, col)) {
                // A formula is flagged at once, and an array's members are
                // lined up with every flagged cell it reads, as for the
                // engine's readers above.
                readers.append(&mut found.cells);
                readers.extend_from_slice(&found.arrays);
            }
        }
        for (t, i) in readers {
            let cell = &plans[t].1.cells[i];
            if let Some((r1, c1, r2, c2)) = cell.array_extent {
                if !reads.contains_key(&(t, i)) {
                    let at = address(t, cell.row, cell.col)?;
                    reads.insert((t, i), precedent_areas(engine, &at, &sheet_index, false)?);
                }
                let areas = &reads[&(t, i)];
                let (height, width) = (r2 - r1 + 1, c2 - c1 + 1);
                // The members whose elements of a reference hold the cell.
                let mut reached: Vec<_> = areas
                    .iter()
                    .filter(|area| contains(area, (s, row, col)))
                    .map(|&(_, top, left, bottom, right)| {
                        let rows = if bottom - top + 1 == height {
                            let r = r1 + row - top;
                            (r, r)
                        } else {
                            (r1, r2)
                        };
                        let cols = if right - left + 1 == width {
                            let c = c1 + col - left;
                            (c, c)
                        } else {
                            (c1, c2)
                        };
                        (rows, cols)
                    })
                    .collect();
                if reached.is_empty() {
                    reached.push(((r1, r2), (c1, c2)));
                }
                // Every member is reached at most once.
                if reached.contains(&((r1, r2), (c1, c2))) {
                    reached = if whole.insert((t, i)) {
                        vec![((r1, r2), (c1, c2))]
                    } else {
                        Vec::new()
                    };
                }
                for ((top, bottom), (left, right)) in reached {
                    for r in top..=bottom {
                        for c in left..=right {
                            if let Some(Slot::Member(j)) = slots.get(&(t, r, c))
                                && !flags.members[t][*j]
                            {
                                flags.members[t][*j] = true;
                                queue.push((t, r, c));
                            }
                        }
                    }
                }
            }
            if !flags.cells[t][i] {
                flags.cells[t][i] = true;
                queue.push((t, cell.row, cell.col));
            }
        }
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_functions_are_sorted_for_binary_search() {
        assert!(BUILT_IN.windows(2).all(|w| w[0] < w[1]));
        for name in [
            "SUM",
            "sum",
            "AREAS",
            "_xlfn.XLOOKUP",
            "_xlfn._xlws.FILTER",
            "_xlfn.IMAGE",
        ] {
            assert!(is_excel_function(name), "{name}");
        }
        for name in [
            "IMAGE",
            "XLOOKUP",
            "ClrCnt",
            "EOM",
            "arrayformula",
            "__xludf.DUMMYFUNCTION",
            "_xludf.F",
            "_xll.F",
        ] {
            assert!(!is_excel_function(name), "{name}");
        }
    }

    #[test]
    fn a_sheet_sees_its_own_names_before_the_workbook_ones() {
        let name = |name: &str, sheet| package::DefinedName {
            name: name.to_owned(),
            sheet,
            formula: String::new(),
        };
        let defined = [
            name("X", None),
            name("x", Some(1)),
            name("Y", Some(0)),
            name("Z", Some(1)),
        ];
        let scopes = Scopes::new(["Sheet1", "My 'Sheet'"], &defined);
        assert_eq!(scopes.resolve("X", 0), Some(0));
        assert_eq!(scopes.resolve("X", 1), Some(1));
        assert_eq!(scopes.resolve("y", 0), Some(2));
        // Sheet1's Y is not the second sheet's, which has no workbook-level
        // Y to fall back on; nor is that sheet's Z Sheet1's.
        assert_eq!(scopes.resolve("Y", 1), None);
        assert_eq!(scopes.resolve("Z", 0), None);
        // A qualified name is the one that sheet sees.
        assert_eq!(scopes.resolve("'My ''Sheet'''!X", 0), Some(1));
        assert_eq!(scopes.resolve("sheet1!X", 1), Some(0));
        assert_eq!(scopes.resolve("Sheet1!Y", 1), Some(2));
        assert_eq!(scopes.resolve("'My ''Sheet'''!Y", 0), None);
        assert_eq!(scopes.resolve("A1", 0), None);
        assert_eq!(Scopes::new(["Sheet1"], &[]).resolve("X", 0), None);
    }
}
