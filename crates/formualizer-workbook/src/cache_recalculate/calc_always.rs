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
//!   (SUMIF, AVERAGEIF), which reads cells outside its references;
//! - reads a calculated-always cell through a cell, range, name or table
//!   reference, on any sheet.
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
use formualizer_parse::{Token, TokenSubType, TokenType, Tokenizer};
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

/// The workbook's defined names and those whose formulas are calculated
/// always, lowercase.
struct Names {
    defined: HashSet<String>,
    calc_always: HashSet<String>,
}

/// How a formula's call of a function makes it calculated always.
enum Call {
    Volatile,
    Unknown,
    Known,
}

fn call(name: &str, names: &Names) -> Call {
    // A call of a LAMBDA value, `LAMBDA(x,x)(1)`, names no function.
    if name.is_empty() {
        return Call::Known;
    }
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("_XLUDF.") || upper.starts_with("_XLL.") {
        return Call::Unknown;
    }
    let (bare, prefixed) = ["_XLFN._XLWS.", "_XLFN.", "_XLWS."]
        .iter()
        .find_map(|p| upper.strip_prefix(p))
        .map_or((upper.as_str(), false), |bare| (bare, true));
    let lower = name.to_ascii_lowercase();
    if VOLATILE.contains(&bare) || names.calc_always.contains(&lower) {
        Call::Volatile
    } else if prefixed
        || bare.starts_with("_XLPM.")
        || BUILT_IN.binary_search(&bare).is_ok()
        || names.defined.contains(&lower)
    {
        Call::Known
    } else {
        Call::Unknown
    }
}

/// Rows and columns of an A1 cell or area operand (`$A$1:B2`, `A:B`, `1:2`).
fn dimensions(operand: &str) -> Option<(u32, u32)> {
    let local = operand.rsplit_once('!').map_or(operand, |(_, area)| area);
    let (start, end) = local.split_once(':').unwrap_or((local, local));
    let endpoint = |text: &str| {
        let text = text.replace('$', "");
        if let Ok((row, col, _, _)) = formualizer_common::coord::parse_a1_1based(&text) {
            Some((Some(row), Some(col)))
        } else if let Ok(row) = text.parse::<u32>() {
            Some((Some(row), None))
        } else if !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphabetic()) {
            let col = formualizer_common::coord::parse_a1_1based(&format!("{text}1"))
                .ok()?
                .1;
            Some((None, Some(col)))
        } else {
            None
        }
    };
    let (a, b) = (endpoint(start)?, endpoint(end)?);
    let span = |x: Option<u32>, y: Option<u32>, all: u32| match (x, y) {
        (Some(x), Some(y)) => Some(x.abs_diff(y) + 1),
        (None, None) => Some(all),
        _ => None,
    };
    Some((span(a.0, b.0, 1_048_576)?, span(a.1, b.1, 16_384)?))
}

/// SUMIF/AVERAGEIF whose third argument is not the size of the first: Excel
/// sums the cells of the criteria range's size from the sum range's corner.
fn resized_sum_range(tokens: &[Token], open: usize) -> bool {
    let mut args: Vec<Vec<&Token>> = vec![Vec::new()];
    let mut depth = 0usize;
    for token in &tokens[open + 1..] {
        match (token.token_type, token.subtype) {
            (TokenType::Func | TokenType::Paren | TokenType::Array, TokenSubType::Open) => {
                depth += 1
            }
            (TokenType::Func | TokenType::Paren | TokenType::Array, TokenSubType::Close) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            (TokenType::Sep, TokenSubType::Arg) if depth == 0 => {
                args.push(Vec::new());
                continue;
            }
            (TokenType::Whitespace, _) => continue,
            _ => {}
        }
        args.last_mut().expect("argument").push(token);
    }
    let area = |arg: &[&Token]| match arg {
        [token]
            if token.token_type == TokenType::Operand && token.subtype == TokenSubType::Range =>
        {
            dimensions(&token.value)
        }
        _ => None,
    };
    matches!(&args[..], [range, _, sum] if area(range).zip(area(sum)).is_some_and(|(a, b)| a != b))
}

/// Whether a formula's own text makes it calculated always.
fn calculated_always(text: &str, names: &Names) -> bool {
    let tokens = Tokenizer::new_best_effort(&format!("={text}")).items;
    tokens
        .iter()
        .enumerate()
        .any(|(i, token)| match (token.token_type, token.subtype) {
            (TokenType::Func, TokenSubType::Open) => {
                let name = token.value.trim_end_matches('(');
                !matches!(call(name, names), Call::Known)
                    || (["SUMIF", "AVERAGEIF"]
                        .iter()
                        .any(|f| name.eq_ignore_ascii_case(f))
                        && resized_sum_range(&tokens, i))
            }
            (TokenType::Operand, TokenSubType::Range) if !names.calc_always.is_empty() => {
                let name = token
                    .value
                    .rsplit_once('!')
                    .map_or(token.value.as_str(), |(_, n)| n);
                names.calc_always.contains(&name.to_ascii_lowercase())
            }
            _ => false,
        })
}

fn inspect_error(error: impl std::fmt::Display) -> IoError {
    unsupported(
        format!("dependency inspection: {error}"),
        "calculate-always flags",
    )
}

/// The areas a formula reads through cell, range, name and table references,
/// resolved by the engine; with `names_only`, only those through names.
fn precedent_areas(
    engine: &Engine<WBResolver>,
    address: &CellAddress,
    sheet_index: &HashMap<&str, usize>,
    names_only: bool,
) -> Result<Vec<Area>, IoError> {
    let options = PrecedentOptions::default()
        .with_max_links(u32::MAX)
        .with_max_work(u64::MAX);
    let report = engine
        .precedents(address, &options)
        .map_err(inspect_error)?;
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
    Ok(report
        .precedents
        .iter()
        .filter_map(|p| match &p.reference {
            SemanticReference::Name { resolution, .. } => match resolution {
                NameResolution::Cell(c) => cell(c),
                NameResolution::Range {
                    declared: d,
                    resolved,
                } => resolved.as_ref().map_or_else(|| declared(d), range),
                _ => None,
            },
            _ if names_only => None,
            SemanticReference::Cell(c) => cell(c),
            SemanticReference::Range {
                declared: d,
                resolved,
                ..
            } => resolved.as_ref().map_or_else(|| declared(d), range),
            SemanticReference::Table { resolved, .. } => range(resolved),
            _ => None,
        })
        .collect())
}

fn contains(&(a, top, left, bottom, right): &Area, (s, row, col): (usize, u32, u32)) -> bool {
    a == s && (top..=bottom).contains(&row) && (left..=right).contains(&col)
}

/// Whether a formula's text reads a defined name.
fn reads_name(text: &str, names: &Names) -> bool {
    Tokenizer::new_best_effort(&format!("={text}"))
        .items
        .iter()
        .any(|token| {
            token.token_type == TokenType::Operand
                && token.subtype == TokenSubType::Range
                && names.defined.contains(
                    &token
                        .value
                        .rsplit_once('!')
                        .map_or(token.value.as_str(), |(_, n)| n)
                        .to_ascii_lowercase(),
                )
        })
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
        defined: defined_names
            .iter()
            .map(|n| n.name.to_ascii_lowercase())
            .collect(),
        calc_always: HashSet::new(),
    };
    // A name is calculated always when its formula is, possibly through
    // other names.
    loop {
        let found: Vec<String> = defined_names
            .iter()
            .map(|n| n.name.to_ascii_lowercase())
            .zip(defined_names)
            .filter(|(key, n)| {
                !names.calc_always.contains(key) && calculated_always(&n.formula, &names)
            })
            .map(|(key, _)| key)
            .collect();
        if found.is_empty() {
            break;
        }
        names.calc_always.extend(found);
    }
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
    for (s, (_, scan)) in plans.iter().enumerate() {
        let own: Vec<bool> = scan
            .cells
            .iter()
            .map(|cell| {
                !cell.formula_text.trim().is_empty()
                    && calculated_always(&cell.formula_text, &names)
            })
            .collect();
        for (i, cell) in scan.cells.iter().enumerate() {
            if own[cell.shared_master.unwrap_or(i)]
                || (cell.calc_always && cell.array_extent.is_none())
            {
                flags.cells[s][i] = true;
                queue.push((s, cell.row, cell.col));
            } else if cell.calc_always {
                flagged_arrays.push((s, i));
            }
        }
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
    // of each named area are collected from the formulas that name one.
    let mut named: HashMap<Area, Vec<(usize, usize)>> = HashMap::new();
    for (s, (_, scan)) in plans.iter().enumerate() {
        let mentions: Vec<bool> = scan
            .cells
            .iter()
            .map(|cell| !names.defined.is_empty() && reads_name(&cell.formula_text, &names))
            .collect();
        for (i, cell) in scan.cells.iter().enumerate() {
            slots.insert((s, cell.row, cell.col), Slot::Cell(i));
            if mentions[cell.shared_master.unwrap_or(i)] {
                let at = address(s, cell.row, cell.col)?;
                for area in precedent_areas(engine, &at, &sheet_index, true)? {
                    named.entry(area).or_default().push((s, i));
                }
            }
        }
        for (j, member) in scan.members.iter().enumerate() {
            slots.insert((s, member.cell.row, member.cell.col), Slot::Member(j));
        }
    }
    let mut named: Vec<_> = named
        .into_iter()
        .map(|(area, readers)| (area, Some(readers)))
        .collect();
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
            if found.is_some() && contains(area, (s, row, col)) {
                readers.extend(found.take().expect("unread name"));
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
    }

    #[test]
    fn operand_dimensions() {
        assert_eq!(dimensions("$A$1:B3"), Some((3, 2)));
        assert_eq!(dimensions("'My Sheet'!C5"), Some((1, 1)));
        assert_eq!(dimensions("A:C"), Some((1_048_576, 3)));
        assert_eq!(dimensions("$2:$4"), Some((3, 16_384)));
        assert_eq!(dimensions("Prices"), None);
    }
}
