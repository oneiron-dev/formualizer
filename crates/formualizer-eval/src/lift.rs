//! Excel array lifting for scalar parameters.
//!
//! When a parameter that Excel types as a single value (number, text, logical
//! or any scalar) receives a multi-cell range or array, Excel evaluates the
//! function once per element and returns an array of the results. Arguments
//! are paired element-wise; a one-row or one-column argument is broadcast
//! across the other dimension, and positions outside a smaller argument's
//! extent become `#N/A`.
//!
//! Array-typed parameters (aggregates, lookup tables, criteria ranges, ...)
//! consume the whole array and are never lifted, so lifting is declared per
//! function and parameter from Excel's documented parameter types rather than
//! inferred from argument schemas.

use crate::engine::range_view::RangeView;
use crate::traits::{ArgumentHandle, CalcValue};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ASTNode, ASTNodeType};

/// Which parameter positions (0-based) take a single value.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Lift {
    /// Every parameter.
    All,
    /// Only the listed parameters.
    Only(&'static [usize]),
    /// `start`, `start + step`, ... (criteria of the *IFS family).
    Every { start: usize, step: usize },
}

impl Lift {
    fn lifts(self, index: usize) -> bool {
        match self {
            Lift::All => true,
            Lift::Only(positions) => positions.contains(&index),
            Lift::Every { start, step } => index >= start && (index - start) % step == 0,
        }
    }
}

/// Upper bound on lifted elements; larger shapes keep the unlifted path.
const MAX_LIFTED_ELEMENTS: usize = 4_000_000;

/// Excel's scalar parameters for builtins whose other parameters take arrays
/// or references, and for builtins whose parameters are all single values.
pub(crate) fn lift_spec(name: &str) -> Option<Lift> {
    Some(match name {
        // Text
        "ASC" | "CHAR" | "CLEAN" | "CODE" | "CONCATENATE" | "DOLLAR" | "EXACT" | "FIND"
        | "FINDB" | "FIXED" | "LEFT" | "LEFTB" | "LEN" | "LENB" | "LOWER" | "MID" | "MIDB"
        | "NUMBERVALUE" | "PROPER" | "REPLACE" | "REPLACEB" | "REPT" | "RIGHT" | "RIGHTB"
        | "SEARCH" | "SEARCHB" | "SUBSTITUTE" | "T" | "TEXT" | "TRIM" | "UNICHAR" | "UNICODE"
        | "UPPER" | "VALUE" | "ROMAN" | "ARABIC" | "BASE" | "DECIMAL" | "DOLLARDE" | "DOLLARFR" => {
            Lift::All
        }
        // The delimiter may be an array of alternative delimiters.
        "TEXTBEFORE" | "TEXTAFTER" => Lift::Only(&[0, 2, 3, 4, 5]),
        // Date and time
        "DATE" | "DATEDIF" | "DATEVALUE" | "DAY" | "DAYS" | "DAYS360" | "EDATE" | "EOMONTH"
        | "HOUR" | "ISOWEEKNUM" | "MINUTE" | "MONTH" | "SECOND" | "TIME" | "TIMEVALUE"
        | "WEEKDAY" | "WEEKNUM" | "YEAR" | "YEARFRAC" => Lift::All,
        // Holidays are an array.
        "NETWORKDAYS" | "WORKDAY" => Lift::Only(&[0, 1]),
        "NETWORKDAYS.INTL" | "WORKDAY.INTL" => Lift::Only(&[0, 1, 2]),
        // Information and logical
        "ISBLANK" | "ISERR" | "ISERROR" | "ISEVEN" | "ISLOGICAL" | "ISNA" | "ISNONTEXT"
        | "ISNUMBER" | "ISODD" | "ISTEXT" | "N" | "ERROR.TYPE" | "NOT" => Lift::All,
        // Math and engineering
        "ABS" | "ACOS" | "ACOSH" | "ACOT" | "ACOTH" | "ASIN" | "ASINH" | "ATAN" | "ATAN2"
        | "ATANH" | "CEILING" | "CEILING.MATH" | "CEILING.PRECISE" | "ISO.CEILING" | "COMBIN"
        | "COMBINA" | "COS" | "COSH" | "COT" | "COTH" | "CSC" | "CSCH" | "SEC" | "SECH"
        | "DEGREES" | "RADIANS" | "EVEN" | "ODD" | "EXP" | "FACT" | "FACTDOUBLE" | "FLOOR"
        | "FLOOR.MATH" | "FLOOR.PRECISE" | "INT" | "LN" | "LOG" | "LOG10" | "MOD" | "MROUND"
        | "PERMUT" | "POWER" | "QUOTIENT" | "ROUND" | "ROUNDDOWN" | "ROUNDUP" | "SIGN" | "SIN"
        | "SINH" | "SQRT" | "SQRTPI" | "TAN" | "TANH" | "TRUNC" | "BITAND" | "BITOR" | "BITXOR"
        | "BITLSHIFT" | "BITRSHIFT" | "DELTA" | "GESTEP" | "DEC2BIN" | "DEC2HEX" | "DEC2OCT"
        | "BIN2DEC" | "BIN2HEX" | "BIN2OCT" | "HEX2BIN" | "HEX2DEC" | "HEX2OCT" | "OCT2BIN"
        | "OCT2DEC" | "OCT2HEX" | "CONVERT" | "ERF" | "ERFC" | "ERF.PRECISE" | "ERFC.PRECISE"
        | "GAMMA" | "GAMMALN" | "GAMMALN.PRECISE" => Lift::All,
        // Financial
        "PMT" | "IPMT" | "PPMT" | "PV" | "FV" | "NPER" | "RATE" | "EFFECT" | "NOMINAL" | "DB"
        | "DDB" | "SLN" | "SYD" | "ISPMT" | "RRI" | "PDURATION" => Lift::All,
        // Statistical distributions
        "NORM.DIST" | "NORM.INV" | "NORM.S.DIST" | "NORM.S.INV" | "STANDARDIZE" | "FISHER"
        | "FISHERINV" | "EXPON.DIST" | "POISSON.DIST" | "BINOM.DIST" | "PHI" | "GAUSS" => Lift::All,
        // Lookup: the looked-up value, index and mode are single values.
        "VLOOKUP" | "HLOOKUP" => Lift::Only(&[0, 2, 3]),
        "MATCH" => Lift::Only(&[0, 2]),
        "XMATCH" => Lift::Only(&[0, 2, 3]),
        "XLOOKUP" => Lift::Only(&[0, 4, 5]),
        "LOOKUP" => Lift::Only(&[0]),
        "ADDRESS" => Lift::All,
        // Aggregates over an array with a single-value parameter.
        "LARGE" | "SMALL" | "PERCENTILE.INC" | "PERCENTILE.EXC" | "QUARTILE.INC"
        | "QUARTILE.EXC" => Lift::Only(&[1]),
        "PERCENTRANK.INC" | "PERCENTRANK.EXC" => Lift::Only(&[1, 2]),
        "RANK.EQ" | "RANK.AVG" => Lift::Only(&[0, 2]),
        "COUNTIF" | "SUMIF" | "AVERAGEIF" => Lift::Only(&[1]),
        "COUNTIFS" => Lift::Every { start: 1, step: 2 },
        "SUMIFS" | "AVERAGEIFS" | "MAXIFS" | "MINIFS" => Lift::Every { start: 2, step: 2 },
        _ => return None,
    })
}

/// The rows of a multi-cell array value; `None` for scalars and single cells.
pub(crate) fn array_rows(value: &CalcValue<'_>) -> Option<Vec<Vec<LiteralValue>>> {
    match value {
        CalcValue::Range(view) => {
            let (rows, cols) = view.dims();
            if rows == 0 || cols == 0 || (rows == 1 && cols == 1) {
                return None;
            }
            Some(
                (0..rows)
                    .map(|r| (0..cols).map(|c| view.get_cell(r, c)).collect())
                    .collect(),
            )
        }
        CalcValue::Scalar(LiteralValue::Array(rows))
        | CalcValue::AnnotatedScalar(LiteralValue::Array(rows), _) => {
            let cols = rows.first().map_or(0, Vec::len);
            if rows.is_empty() || cols == 0 || (rows.len() == 1 && cols == 1) {
                None
            } else {
                Some(rows.clone())
            }
        }
        _ => None,
    }
}

/// Element `(row, col)` of `rows` broadcast to a larger shape: a single row or
/// column repeats; positions beyond a longer dimension are `#N/A`.
pub(crate) fn broadcast_get(rows: &[Vec<LiteralValue>], row: usize, col: usize) -> LiteralValue {
    let height = rows.len();
    let width = rows.first().map_or(0, Vec::len);
    let r = if height == 1 { 0 } else { row };
    let c = if width == 1 { 0 } else { col };
    rows.get(r)
        .and_then(|cells| cells.get(c))
        .cloned()
        .unwrap_or_else(|| LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na)))
}

/// The broadcast shape of several arrays.
pub(crate) fn broadcast_dims<'r>(
    arrays: impl IntoIterator<Item = &'r Vec<Vec<LiteralValue>>>,
) -> (usize, usize) {
    arrays.into_iter().fold((1, 1), |(h, w), rows| {
        (h.max(rows.len()), w.max(rows.first().map_or(0, Vec::len)))
    })
}

/// A lifted element result: nested arrays keep their top-left value.
pub(crate) fn element(value: CalcValue<'_>) -> LiteralValue {
    match value.into_literal() {
        LiteralValue::Array(rows) => rows
            .into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .unwrap_or(LiteralValue::Empty),
        other => other,
    }
}

pub(crate) fn array_result<'b>(
    rows: Vec<Vec<LiteralValue>>,
    date_system: crate::engine::DateSystem,
) -> CalcValue<'b> {
    CalcValue::Range(RangeView::from_owned_rows(rows, date_system))
}

/// Evaluate `call` once per element when a lifted parameter holds an array.
/// Returns `None` when no lifted argument is a multi-cell array.
pub(crate) fn lift_call<'a, 'b, F>(
    spec: Lift,
    args: &[ArgumentHandle<'a, 'b>],
    call: F,
) -> Result<Option<CalcValue<'b>>, ExcelError>
where
    F: for<'x> Fn(&[ArgumentHandle<'x, 'b>]) -> Result<CalcValue<'b>, ExcelError>,
{
    let mut arrays = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if !spec.lifts(index) || arg.is_omitted() {
            continue;
        }
        let Ok(value) = arg.value() else {
            return Ok(None);
        };
        if let Some(rows) = array_rows(&value) {
            arrays.push((index, rows));
        }
    }
    if arrays.is_empty() {
        return Ok(None);
    }
    let (height, width) = broadcast_dims(arrays.iter().map(|(_, rows)| rows));
    let per_element = arrays.len();
    if height.saturating_mul(width).saturating_mul(per_element) > MAX_LIFTED_ELEMENTS {
        return Ok(None);
    }
    let mut nodes = Vec::with_capacity(height * width * per_element);
    for r in 0..height {
        for c in 0..width {
            for (_, rows) in &arrays {
                nodes.push(ASTNode::new(
                    ASTNodeType::Literal(broadcast_get(rows, r, c)),
                    None,
                ));
            }
        }
    }
    let mut handles: Vec<ArgumentHandle<'_, 'b>> = args.to_vec();
    let mut out = Vec::with_capacity(height);
    for r in 0..height {
        let mut row = Vec::with_capacity(width);
        for c in 0..width {
            let base = (r * width + c) * per_element;
            for (j, (index, _)) in arrays.iter().enumerate() {
                handles[*index] = args[*index].literal(&nodes[base + j]);
            }
            row.push(match call(&handles) {
                Ok(value) => element(value),
                Err(error) if error.kind == ExcelErrorKind::Cancelled => return Err(error),
                Err(error) => LiteralValue::Error(error),
            });
        }
        out.push(row);
    }
    Ok(Some(array_result(out, args[0].date_system())))
}
