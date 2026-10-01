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
//!
//! A reference-returning function lifted the same way (`OFFSET(A1,{0;1},0)`)
//! returns an array of references. It has no value of its own; a function
//! taking a reference or a single value there is evaluated once per
//! reference instead (`SUBTOTAL(9,OFFSET(A1,{0;1},0))` is `{A1;A2}`).

use crate::engine::range_view::RangeView;
use crate::traits::{ArgumentHandle, CalcValue};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};

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

/// An array of references: one reference, or the error in its place, per element.
pub(crate) type ReferenceArray = Vec<Vec<Result<ReferenceType, ExcelError>>>;

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

/// Excel's reference parameters of builtins that are evaluated once per
/// reference of an array of references. Their single-value parameters
/// (criteria) lift over it as well, as do those of every `lift_spec` builtin.
fn reference_lift_spec(name: &str) -> Option<Lift> {
    Some(match name {
        "SUBTOTAL" => Lift::Every { start: 1, step: 1 },
        "AGGREGATE" => Lift::Every { start: 2, step: 1 },
        "SUMIF" | "COUNTIF" | "AVERAGEIF" | "SUMIFS" | "COUNTIFS" | "AVERAGEIFS" | "MAXIFS"
        | "MINIFS" | "COUNTBLANK" => Lift::All,
        _ => return None,
    })
}

/// The single-value parameters of the reference-returning builtins, which
/// return an array of references when lifted.
pub(crate) fn reference_array_spec(name: &str) -> Option<Lift> {
    if name.eq_ignore_ascii_case("OFFSET") {
        Some(Lift::Only(&[1, 2, 3, 4]))
    } else if name.eq_ignore_ascii_case("INDIRECT") {
        Some(Lift::All)
    } else {
        None
    }
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
    broadcast_at(rows, row, col)
        .cloned()
        .unwrap_or_else(|| LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na)))
}

/// Element `(row, col)` of `rows` broadcast to a larger shape; `None` beyond
/// a longer dimension.
fn broadcast_at<T>(rows: &[Vec<T>], row: usize, col: usize) -> Option<&T> {
    let height = rows.len();
    let width = rows.first().map_or(0, Vec::len);
    let r = if height == 1 { 0 } else { row };
    let c = if width == 1 { 0 } else { col };
    rows.get(r).and_then(|cells| cells.get(c))
}

/// The broadcast shape of several arrays.
pub(crate) fn broadcast_dims<'r, T: 'r>(
    arrays: impl IntoIterator<Item = &'r Vec<Vec<T>>>,
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

/// Evaluate `call` once per element when a lifted parameter holds an array,
/// or a lifted or reference parameter holds an array of references.
/// Returns `None` when no such argument is a multi-cell array.
pub(crate) fn lift_call<'a, 'b, F>(
    name: &str,
    args: &[ArgumentHandle<'a, 'b>],
    call: F,
) -> Result<Option<CalcValue<'b>>, ExcelError>
where
    F: for<'x> Fn(&[ArgumentHandle<'x, 'b>]) -> Result<CalcValue<'b>, ExcelError>,
{
    let values = lift_spec(name);
    let Some(references) = reference_lift_spec(name).or(values) else {
        return Ok(None);
    };
    let mut arrays = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if !references.lifts(index) || arg.is_omitted() {
            continue;
        }
        if values.is_some_and(|spec| spec.lifts(index)) {
            let Ok(value) = arg.value() else {
                return Ok(None);
            };
            if let Some(rows) = array_rows(&value) {
                arrays.push((index, literal_nodes(rows)));
                continue;
            }
            // An array of references has no value; it reads as an error.
            if !matches!(value, CalcValue::Scalar(LiteralValue::Error(_))) {
                continue;
            }
        }
        if let Some(refs) = arg.reference_array()? {
            arrays.push((index, reference_nodes(refs)));
        }
    }
    let lifted = each_element(args, &arrays, |handles| match call(handles) {
        Ok(value) => Ok(element(value)),
        Err(error) if error.kind == ExcelErrorKind::Cancelled => Err(error),
        Err(error) => Ok(LiteralValue::Error(error)),
    })?;
    Ok(lifted.map(|rows| array_result(rows, args[0].date_system())))
}

/// The references of a reference-returning call whose single-value
/// parameters (`spec`) hold an array, one per element of their broadcast
/// shape. Returns `None` when none of them is a multi-cell array.
pub(crate) fn lift_reference<'a, 'b, F>(
    spec: Lift,
    args: &[ArgumentHandle<'a, 'b>],
    call: F,
) -> Result<Option<ReferenceArray>, ExcelError>
where
    F: for<'x> Fn(&[ArgumentHandle<'x, 'b>]) -> Option<Result<ReferenceType, ExcelError>>,
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
            arrays.push((index, literal_nodes(rows)));
        }
    }
    each_element(args, &arrays, |handles| match call(handles) {
        Some(Err(error)) if error.kind == ExcelErrorKind::Cancelled => Err(error),
        Some(result) => Ok(result),
        None => Ok(Err(ExcelError::new(ExcelErrorKind::Ref))),
    })
}

fn literal_nodes(rows: Vec<Vec<LiteralValue>>) -> Vec<Vec<ASTNode>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|value| ASTNode::new(ASTNodeType::Literal(value), None))
                .collect()
        })
        .collect()
}

/// Reference elements as absolute references, so a relocated evaluation
/// (shared formulas) does not shift a reference that is already resolved.
fn reference_nodes(rows: ReferenceArray) -> Vec<Vec<ASTNode>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|reference| {
                    let node_type = match reference {
                        Ok(reference) => {
                            let reference = absolute(reference);
                            ASTNodeType::Reference {
                                original: reference.to_string(),
                                reference,
                            }
                        }
                        Err(error) => ASTNodeType::Literal(LiteralValue::Error(error)),
                    };
                    ASTNode::new(node_type, None)
                })
                .collect()
        })
        .collect()
}

fn absolute(reference: ReferenceType) -> ReferenceType {
    match reference {
        ReferenceType::Cell {
            sheet, row, col, ..
        } => ReferenceType::Cell {
            sheet,
            row,
            col,
            row_abs: true,
            col_abs: true,
        },
        ReferenceType::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        } => ReferenceType::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            start_row_abs: true,
            start_col_abs: true,
            end_row_abs: true,
            end_col_abs: true,
        },
        other => other,
    }
}

/// Evaluate `call` once per element of the broadcast shape of `arrays`, each
/// lifted argument replaced by its element (`#N/A` beyond a shorter array).
/// Returns `None` when there is nothing to lift.
fn each_element<'a, 'b, T, F>(
    args: &[ArgumentHandle<'a, 'b>],
    arrays: &[(usize, Vec<Vec<ASTNode>>)],
    call: F,
) -> Result<Option<Vec<Vec<T>>>, ExcelError>
where
    F: for<'x> Fn(&[ArgumentHandle<'x, 'b>]) -> Result<T, ExcelError>,
{
    if arrays.is_empty() {
        return Ok(None);
    }
    let (height, width) = broadcast_dims(arrays.iter().map(|(_, nodes)| nodes));
    if height.saturating_mul(width).saturating_mul(arrays.len()) > MAX_LIFTED_ELEMENTS {
        return Ok(None);
    }
    let na = ASTNode::new(
        ASTNodeType::Literal(LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na))),
        None,
    );
    let mut handles: Vec<ArgumentHandle<'_, 'b>> = args.to_vec();
    let mut out = Vec::with_capacity(height);
    for r in 0..height {
        let mut row = Vec::with_capacity(width);
        for c in 0..width {
            for (index, nodes) in arrays {
                handles[*index] = args[*index].literal(broadcast_at(nodes, r, c).unwrap_or(&na));
            }
            row.push(call(&handles)?);
        }
        out.push(row);
    }
    Ok(Some(out))
}
