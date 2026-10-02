use crate::args::{ArgSchema, CoercionPolicy, ShapeKind};
use crate::coercion::compare_to_15_digits;
use formualizer_common::{ExcelError, LiteralValue};
use std::sync::LazyLock;

/// Small epsilon used to detect near-zero denominators in trig/hyperbolic functions.
pub const EPSILON_NEAR_ZERO: f64 = 1e-12;

/// Final non-finite guard for numeric aggregate results (SUM/MIN/MAX/...):
/// Excel never surfaces inf/NaN — an overflowed aggregate is `#NUM\!`, the
/// same parity rule operators already apply via `coercion::sanitize_numeric`.
/// One branch per aggregate CALL (apply to the finished reduction, never per
/// element — arrow kernels stay untouched).
pub fn aggregate_result(n: f64) -> LiteralValue {
    if n.is_finite() {
        LiteralValue::Number(n)
    } else {
        LiteralValue::Error(ExcelError::new_num())
    }
}

/// Coerce a value passed to a function's number parameter to `f64`, as Excel
/// does ([`crate::coercion::to_number_argument`]).
/// - Number/Int map to f64
/// - Boolean maps to 1.0/0.0
/// - Empty maps to 0.0
/// - Text converts like VALUE(): numeric text, then date/time text
/// - Others -> `#VALUE!`
pub fn coerce_num(value: &LiteralValue) -> Result<f64, ExcelError> {
    crate::coercion::to_number_argument(value)
}

/// Get a single numeric argument, with count and error checks.
pub fn unary_numeric_arg<'a, 'b>(
    args: &'a [crate::traits::ArgumentHandle<'a, 'b>],
) -> Result<f64, ExcelError> {
    if args.len() != 1 {
        return Err(ExcelError::new_value()
            .with_message(format!("Expected 1 argument, got {}", args.len())));
    }
    let v = args[0].value()?.into_literal();
    match v {
        LiteralValue::Error(e) => Err(e),
        other => coerce_num(&other),
    }
}

/// Get two numeric arguments, with count and error checks.
pub fn binary_numeric_args<'a, 'b>(
    args: &'a [crate::traits::ArgumentHandle<'a, 'b>],
) -> Result<(f64, f64), ExcelError> {
    if args.len() != 2 {
        return Err(ExcelError::new_value()
            .with_message(format!("Expected 2 arguments, got {}", args.len())));
    }
    let a = args[0].value()?.into_literal();
    let b = args[1].value()?.into_literal();
    let a_num = match a {
        LiteralValue::Error(e) => return Err(e),
        other => coerce_num(&other)?,
    };
    let b_num = match b {
        LiteralValue::Error(e) => return Err(e),
        other => coerce_num(&other)?,
    };
    Ok((a_num, b_num))
}

fn calc_from_literal<'b>(
    v: LiteralValue,
    date_system: crate::engine::DateSystem,
) -> crate::traits::CalcValue<'b> {
    match v {
        LiteralValue::Array(rows) => crate::traits::CalcValue::Range(
            crate::engine::range_view::RangeView::from_owned_rows(rows, date_system),
        ),
        other => crate::traits::CalcValue::Scalar(other),
    }
}

pub fn unary_numeric_elementwise<'a, 'b, F>(
    args: &'a [crate::traits::ArgumentHandle<'a, 'b>],
    ctx: &dyn crate::traits::FunctionContext<'b>,
    mut f: F,
) -> Result<crate::traits::CalcValue<'b>, ExcelError>
where
    F: FnMut(f64) -> Result<LiteralValue, ExcelError>,
{
    if args.len() != 1 {
        return Err(ExcelError::new_value()
            .with_message(format!("Expected 1 argument, got {}", args.len())));
    }

    let shape = if let Ok(rv) = args[0].range_view() {
        rv.dims()
    } else if let Ok(cv) = args[0].value() {
        match cv.into_literal() {
            LiteralValue::Array(arr) => (arr.len(), arr.first().map(|r| r.len()).unwrap_or(0)),
            _ => (1, 1),
        }
    } else {
        (1, 1)
    };

    if shape != (1, 1) {
        let mut out: Vec<Vec<LiteralValue>> = Vec::with_capacity(shape.0);
        if let Ok(view) = args[0].range_view() {
            view.for_each_row(&mut |row| {
                let mut out_row: Vec<LiteralValue> = Vec::with_capacity(row.len());
                for cell in row.iter() {
                    let num_opt = match cell {
                        LiteralValue::Error(e) => return Err(e.clone()),
                        other => crate::coercion::to_number_argument(other).ok(),
                    };
                    match num_opt {
                        Some(n) => out_row.push(f(n)?),
                        None => out_row.push(LiteralValue::Error(
                            ExcelError::new_value()
                                .with_message("Element is not coercible to number"),
                        )),
                    }
                }
                out.push(out_row);
                Ok(())
            })?;
        } else {
            let v = args[0].value()?.into_literal();
            let LiteralValue::Array(arr) = v else {
                // Defensive: if shape says array but value isn't, treat as scalar.
                let x = unary_numeric_arg(args)?;
                return Ok(calc_from_literal(f(x)?, ctx.date_system()));
            };

            for row in arr {
                let mut out_row: Vec<LiteralValue> = Vec::with_capacity(row.len());
                for cell in row {
                    let num_opt = match &cell {
                        LiteralValue::Error(e) => return Err(e.clone()),
                        other => crate::coercion::to_number_argument(other).ok(),
                    };
                    match num_opt {
                        Some(n) => out_row.push(f(n)?),
                        None => out_row.push(LiteralValue::Error(
                            ExcelError::new_value()
                                .with_message("Element is not coercible to number"),
                        )),
                    }
                }
                out.push(out_row);
            }
        }

        return Ok(calc_from_literal(
            LiteralValue::Array(out),
            ctx.date_system(),
        ));
    }

    let x = unary_numeric_arg(args)?;
    Ok(calc_from_literal(f(x)?, ctx.date_system()))
}

pub fn binary_numeric_elementwise<'a, 'b, F>(
    args: &'a [crate::traits::ArgumentHandle<'a, 'b>],
    ctx: &dyn crate::traits::FunctionContext<'b>,
    mut f: F,
) -> Result<crate::traits::CalcValue<'b>, ExcelError>
where
    F: FnMut(f64, f64) -> Result<LiteralValue, ExcelError>,
{
    if args.len() != 2 {
        return Err(ExcelError::new_value()
            .with_message(format!("Expected 2 arguments, got {}", args.len())));
    }

    use crate::broadcast::{broadcast_shape, project_index};

    enum Grid<'b> {
        Range(crate::engine::range_view::RangeView<'b>),
        Array(Vec<Vec<LiteralValue>>),
        Scalar(LiteralValue),
    }

    impl<'b> Grid<'b> {
        fn shape(&self) -> (usize, usize) {
            match self {
                Grid::Range(rv) => rv.dims(),
                Grid::Array(arr) => (arr.len(), arr.first().map(|r| r.len()).unwrap_or(0)),
                Grid::Scalar(_) => (1, 1),
            }
        }

        fn get(&self, r: usize, c: usize) -> LiteralValue {
            match self {
                Grid::Range(rv) => rv.get_cell(r, c),
                Grid::Array(arr) => arr
                    .get(r)
                    .and_then(|row| row.get(c))
                    .cloned()
                    .unwrap_or(LiteralValue::Empty),
                Grid::Scalar(v) => v.clone(),
            }
        }
    }

    fn to_grid<'a, 'b>(ah: &crate::traits::ArgumentHandle<'a, 'b>) -> Result<Grid<'b>, ExcelError> {
        if let Ok(rv) = ah.range_view() {
            return Ok(Grid::Range(rv));
        }
        let v = ah.value()?.into_literal();
        Ok(match v {
            LiteralValue::Array(arr) => Grid::Array(arr),
            other => Grid::Scalar(other),
        })
    }

    let g0 = to_grid(&args[0])?;
    let g1 = to_grid(&args[1])?;
    let s0 = g0.shape();
    let s1 = g1.shape();
    let target = broadcast_shape(&[s0, s1])?;

    if target != (1, 1) {
        let mut out: Vec<Vec<LiteralValue>> = Vec::with_capacity(target.0);
        for r in 0..target.0 {
            let mut out_row = Vec::with_capacity(target.1);
            for c in 0..target.1 {
                let (r0, c0) = project_index((r, c), s0);
                let (r1, c1) = project_index((r, c), s1);
                let lv0 = g0.get(r0, c0);
                let lv1 = g1.get(r1, c1);

                let n0 = match &lv0 {
                    LiteralValue::Error(e) => return Err(e.clone()),
                    other => crate::coercion::to_number_argument(other).ok(),
                };
                let n1 = match &lv1 {
                    LiteralValue::Error(e) => return Err(e.clone()),
                    other => crate::coercion::to_number_argument(other).ok(),
                };

                let out_cell = match (n0, n1) {
                    (Some(a), Some(b)) => f(a, b)?,
                    _ => LiteralValue::Error(
                        ExcelError::new_value()
                            .with_message("Elements are not coercible to numbers"),
                    ),
                };
                out_row.push(out_cell);
            }
            out.push(out_row);
        }
        return Ok(calc_from_literal(
            LiteralValue::Array(out),
            ctx.date_system(),
        ));
    }

    let (a, b) = binary_numeric_args(args)?;
    Ok(calc_from_literal(f(a, b)?, ctx.date_system()))
}

/// Forward-looking: clamp numeric result to Excel-friendly finite values.
/// Converts NaN to `#NUM!` and +/-Inf to large finite sentinels if desired.
pub fn sanitize_numeric_result(n: f64) -> Result<f64, ExcelError> {
    crate::coercion::sanitize_numeric(n)
}

/// Forward-looking: try converting text that looks like a number (Excel often parses text numbers).
pub fn coerce_text_to_number_maybe(value: &LiteralValue) -> Option<f64> {
    match value {
        LiteralValue::Text(_) => crate::coercion::to_number_lenient(value).ok(),
        _ => None,
    }
}

/// Forward-looking: common rounding strategy for functions requiring specific rounding.
pub fn round_to_precision(n: f64, digits: i32) -> f64 {
    if digits <= 0 {
        return n.round();
    }
    let factor = 10f64.powi(digits);
    (n * factor).round() / factor
}

pub fn collapse_if_scalar(
    rows: Vec<Vec<LiteralValue>>,
    date_system: crate::engine::DateSystem,
) -> crate::traits::CalcValue<'static> {
    if rows.len() == 1 && rows[0].len() == 1 {
        crate::traits::CalcValue::Scalar(rows[0][0].clone())
    } else {
        crate::traits::CalcValue::Range(crate::engine::range_view::RangeView::from_owned_rows(
            rows,
            date_system,
        ))
    }
}

// ─────────────────────────────── Criteria helpers (shared by *IF* aggregators) ───────────────────────────────

/// Match a value against a parsed `CriteriaPredicate` (see `crate::args::CriteriaPredicate`).
/// Implements Excel-style semantics for equality (case-insensitive text, lenient numeric,
/// date and time text equal to their serials), inequality comparisons with numeric
/// coercion, wildcard text matching, and type tests.
pub fn criteria_match(pred: &crate::args::CriteriaPredicate, v: &LiteralValue) -> bool {
    use crate::args::CriteriaPredicate as P;
    match pred {
        P::Eq(t) => values_equal_invariant(t, v) || date_text_equals(t, v),
        P::Ne(t) => !(values_equal_invariant(t, v) || date_text_equals(t, v)),
        P::Gt(_) | P::Ge(_) | P::Lt(_) | P::Le(_) => {
            let (n, holds) = numeric_criterion(pred).expect("an ordered criterion is numeric");
            criteria_ordered_number(v).is_some_and(|x| holds(compare_to_15_digits(x, n)))
        }
        P::TextLike {
            pattern,
            case_insensitive,
        } => text_like_match(pattern, *case_insensitive, v),
        P::IsBlank => matches!(v, LiteralValue::Empty),
        P::IsNumber => value_to_number(v).is_ok(),
        P::IsText => matches!(v, LiteralValue::Text(_)),
        P::IsLogical => matches!(v, LiteralValue::Boolean(_)),
    }
}

fn value_to_number(v: &LiteralValue) -> Result<f64, ExcelError> {
    crate::coercion::to_number_lenient(v)
}

/// Whether a cell's number equals a criterion's: to 15 significant digits,
/// like every numeric criterion (see [`numeric_criterion`]).
fn numbers_equal(a: f64, b: f64) -> bool {
    compare_to_15_digits(a, b) == Some(std::cmp::Ordering::Equal)
}

/// A criterion's number, and whether the order of a cell's number to it
/// meets the criterion.
type NumericCriterion = (f64, fn(Option<std::cmp::Ordering>) -> bool);

/// A criterion that compares numbers (`">5"`, `"<=0.3"`, `"=7"`, `"<>0"`, `7`).
/// Excel orders the two numbers as the comparison operators do, equal when
/// they agree to 15 significant digits, for every operator: 8:30
/// (0.35416666666666669) meets `">="&A1` with A1 8:30, whose text makes the
/// criterion `">=0.354166666666667"`, and `"<"&A1` skips it.
fn numeric_criterion(pred: &crate::args::CriteriaPredicate) -> Option<NumericCriterion> {
    use crate::args::CriteriaPredicate as P;
    use std::cmp::Ordering::{Equal, Greater, Less};
    Some(match pred {
        P::Gt(n) => (*n, |o| o == Some(Greater)),
        P::Ge(n) => (*n, |o| matches!(o, Some(Greater | Equal))),
        P::Lt(n) => (*n, |o| o == Some(Less)),
        P::Le(n) => (*n, |o| matches!(o, Some(Less | Equal))),
        P::Eq(LiteralValue::Number(n)) => (*n, |o| o == Some(Equal)),
        P::Eq(LiteralValue::Int(i)) => (*i as f64, |o| o == Some(Equal)),
        P::Ne(LiteralValue::Number(n)) => (*n, |o| o != Some(Equal)),
        P::Ne(LiteralValue::Int(i)) => (*i as f64, |o| o != Some(Equal)),
        _ => return None,
    })
}

/// `criteria_match` of a numeric criterion (see [`numeric_criterion`]) for
/// each number of a numbers lane, null where the lane holds no number. The
/// cached and vectorized criteria masks use it, so they compare numbers like
/// the scalar matcher does. `None` for any other criterion.
pub(crate) fn numeric_criteria_mask(
    numbers: &arrow_array::Float64Array,
    pred: &crate::args::CriteriaPredicate,
) -> Option<arrow_array::BooleanArray> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let (n, holds) = numeric_criterion(pred)?;
    // The doubles equal to n to 15 digits form one run, so a finite cell is
    // ordered by two plain comparisons.
    let (low, high) = crate::coercion::same_to_15_digits_bounds(n);
    Some(arrow_array::BooleanArray::from_unary(numbers, |x| {
        holds(if !x.is_finite() || !n.is_finite() {
            compare_to_15_digits(x, n)
        } else if x < low {
            Some(Less)
        } else if x > high {
            Some(Greater)
        } else {
            Some(Equal)
        })
    }))
}

/// A numeric (in)equality criterion reads a cell's text as the date or time
/// Excel reads it as: `SUMIFS(K:K,A:A,DATE(2021,3,1))` sums the rows holding
/// the text "3-1-21", as numeric text already equals its number. The text is
/// read in the function call's date context
/// ([`crate::coercion::argument_date_text_serial`]), and its serial equals
/// the criterion's number as numbers do, to 15 significant digits
/// ([`numbers_equal`]). A logical is never a number for criteria.
fn date_text_equals(criterion: &LiteralValue, v: &LiteralValue) -> bool {
    let LiteralValue::Text(text) = v else {
        return false;
    };
    let wanted = match criterion {
        LiteralValue::Number(_)
        | LiteralValue::Int(_)
        | LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => {
            criterion.as_serial_number_for(crate::coercion::argument_date_context().0)
        }
        _ => None,
    };
    wanted.is_some_and(|wanted| {
        crate::coercion::argument_date_text_serial(text)
            .is_some_and(|serial| numbers_equal(wanted, serial))
    })
}

/// The number a cell offers to an ordered numeric criterion (`">5"`, `"<=0"`).
/// Criteria compare like types only: a blank cell or a logical is not a
/// number, so `COUNTIF(r,"<5")` skips blanks and FALSE, and `">0"` skips TRUE.
fn criteria_ordered_number(v: &LiteralValue) -> Option<f64> {
    match v {
        LiteralValue::Empty | LiteralValue::Boolean(_) => None,
        _ => value_to_number(v).ok(),
    }
}

fn values_equal_invariant(a: &LiteralValue, b: &LiteralValue) -> bool {
    match (a, b) {
        (LiteralValue::Number(x), LiteralValue::Number(y)) => numbers_equal(*x, *y),
        (LiteralValue::Int(x), LiteralValue::Int(y)) => numbers_equal(*x as f64, *y as f64),
        (LiteralValue::Boolean(x), LiteralValue::Boolean(y)) => x == y,
        // A logical never equals a number (or a date) for criteria: TRUE=1 is
        // FALSE, so COUNTIF(r,1) skips TRUE and COUNTIF(r,"<>0") counts FALSE.
        (LiteralValue::Boolean(_), _) | (_, LiteralValue::Boolean(_)) => false,
        (LiteralValue::Text(x), LiteralValue::Text(y)) => x.to_lowercase() == y.to_lowercase(),
        // Treat blank and empty text as equal (Excel semantics)
        (LiteralValue::Text(x), LiteralValue::Empty) if x.is_empty() => true,
        (LiteralValue::Empty, LiteralValue::Text(y)) if y.is_empty() => true,
        (LiteralValue::Empty, LiteralValue::Empty) => true,
        (LiteralValue::Error(x), LiteralValue::Error(y)) => x.kind == y.kind,
        (LiteralValue::Error(_), _) | (_, LiteralValue::Error(_)) => false,
        // A blank cell is not the number 0 for criteria: COUNTIF(A:A,0)
        // skips blanks and COUNTIF(A:A,"<>0") counts them.
        (LiteralValue::Number(_) | LiteralValue::Int(_), LiteralValue::Empty)
        | (LiteralValue::Empty, LiteralValue::Number(_) | LiteralValue::Int(_)) => false,
        // Date/time/duration equality: compare by serial value.
        // This matches criteria semantics (COUNTIF(S), SUMIF(S), database criteria, etc.) where
        // date-like values participate in numeric comparisons.
        (x, y) if x.as_serial_number().is_some() && y.as_serial_number().is_some() => x
            .as_serial_number()
            .zip(y.as_serial_number())
            .is_some_and(|(sx, sy)| numbers_equal(sx, sy)),
        (LiteralValue::Number(x), _) => value_to_number(b).is_ok_and(|y| numbers_equal(*x, y)),
        (_, LiteralValue::Number(_)) => values_equal_invariant(b, a),
        _ => false,
    }
}

/// Wildcard criteria (`"a*"`, `"?*"`, `"*"`) match text only: numbers,
/// logicals and blank cells never match, as in Excel.
fn text_like_match(pattern: &str, case_insensitive: bool, v: &LiteralValue) -> bool {
    let s = match v {
        LiteralValue::Text(t) => t.clone(),
        _ => return false,
    };
    let (pat, text) = if case_insensitive {
        (pattern.to_lowercase(), s.to_lowercase())
    } else {
        (pattern.to_string(), s)
    };

    // Fast-path for anchored patterns without '?' or escape sequences
    if !pat.contains('?') && !pat.contains("~*") && !pat.contains("~?") {
        // Pattern like "text*" - starts with
        if pat.ends_with('*') && !pat[..pat.len() - 1].contains('*') {
            return text.starts_with(&pat[..pat.len() - 1]);
        }
        // Pattern like "*text" - ends with
        if pat.starts_with('*') && !pat[1..].contains('*') {
            return text.ends_with(&pat[1..]);
        }
        // Pattern like "*text*" - contains
        if pat.starts_with('*') && pat.ends_with('*') && !pat[1..pat.len() - 1].contains('*') {
            return text.contains(&pat[1..pat.len() - 1]);
        }
        // Pattern with no wildcards - exact match
        if !pat.contains('*') {
            return text == pat;
        }
    }

    // Fall back to general wildcard matching for complex patterns
    wildcard_match(&pat, &text)
}

fn wildcard_match(pat: &str, text: &str) -> bool {
    // Simple glob-like matcher for * and ? (non-greedy backtracking).
    fn helper(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        match p[0] {
            b'*' => {
                for i in 0..=t.len() {
                    if helper(&p[1..], &t[i..]) {
                        return true;
                    }
                }
                false
            }
            b'?' => {
                if t.is_empty() {
                    false
                } else {
                    helper(&p[1..], &t[1..])
                }
            }
            ch => {
                if t.first().copied() == Some(ch) {
                    helper(&p[1..], &t[1..])
                } else {
                    false
                }
            }
        }
    }
    helper(pat.as_bytes(), text.as_bytes())
}

// ─────────────────────────────── ArgSchema presets ───────────────────────────────

/// Single scalar argument of any type.
/// Used by many unary or variadic-any functions (e.g., `LEN`, `TYPE`, simple wrappers).
pub static ARG_ANY_ONE: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| vec![ArgSchema::any()]);

/// Two scalar arguments of any type.
/// Used by generic binary functions (e.g., comparisons, concatenation variants).
pub static ARG_ANY_TWO: LazyLock<Vec<ArgSchema>> =
    LazyLock::new(|| vec![ArgSchema::any(), ArgSchema::any()]);

/// Single numeric scalar argument, with lenient text-to-number coercion.
/// Ideal for elementwise numeric functions (e.g., `SIN`, `COS`, `ABS`).
pub static ARG_NUM_LENIENT_ONE: LazyLock<Vec<ArgSchema>> =
    LazyLock::new(|| vec![{ ArgSchema::number_lenient_scalar() }]);

/// Two numeric scalar arguments, with lenient text-to-number coercion.
/// Suited for binary numeric operations (e.g., `ATAN2`, `POWER`, `LOG(base)`).
pub static ARG_NUM_LENIENT_TWO: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
    vec![{ ArgSchema::number_lenient_scalar() }, {
        ArgSchema::number_lenient_scalar()
    }]
});

/// Single range argument, numeric semantics with lenient text-to-number coercion.
/// Best for reductions over ranges (e.g., `SUM`, `AVERAGE`, `COUNT`-like families).
pub static ARG_RANGE_NUM_LENIENT_ONE: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
    vec![{
        let mut s = ArgSchema::number_lenient_scalar();
        s.shape = ShapeKind::Range;
        s.coercion = CoercionPolicy::NumberLenientText;
        s
    }]
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::CriteriaPredicate as P;

    #[test]
    fn numeric_lane_masks_match_the_scalar_matcher() {
        // The cached and vectorized masks and the scalar matcher compare a
        // cell's number with the criterion's to 15 significant digits alike,
        // for every operator, near the criterion and away from it.
        let mut state = 0x853c_49e6_748f_ea9bu64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state >> 11
        };
        for _ in 0..200 {
            let n =
                (next() as f64 / (1u64 << 53) as f64 - 0.5) * 10f64.powi((next() % 40) as i32 - 20);
            let mut cells = vec![
                n,
                -n,
                0.0,
                n * 2.0,
                n * (1.0 + 4e-15),
                n * (1.0 - 4e-15),
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NAN,
            ];
            let mut x = n;
            for _ in 0..60 {
                x = x.next_down();
            }
            for _ in 0..120 {
                cells.push(x);
                x = x.next_up();
            }
            let lane = arrow_array::Float64Array::from(cells.clone());
            for pred in [
                P::Gt(n),
                P::Ge(n),
                P::Lt(n),
                P::Le(n),
                P::Eq(LiteralValue::Number(n)),
                P::Ne(LiteralValue::Number(n)),
            ] {
                let mask = numeric_criteria_mask(&lane, &pred).unwrap();
                for (i, cell) in cells.iter().enumerate() {
                    assert_eq!(
                        mask.value(i),
                        criteria_match(&pred, &LiteralValue::Number(*cell)),
                        "{pred:?} on {cell:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn numeric_criteria_compare_to_15_digits() {
        // 8:30 is 0.35416666666666669; the criterion ">=0.354166666666667"
        // (from ">="&A1 with A1 8:30) is 0.35416666666666702.
        let cell = LiteralValue::Number(8.5 / 24.0);
        let n = 0.354166666666667;
        assert!(criteria_match(&P::Ge(n), &cell));
        assert!(criteria_match(&P::Le(n), &cell));
        assert!(!criteria_match(&P::Lt(n), &cell));
        assert!(!criteria_match(&P::Gt(n), &cell));
        assert!(criteria_match(&P::Eq(LiteralValue::Number(n)), &cell));
        assert!(!criteria_match(&P::Ne(LiteralValue::Number(n)), &cell));
        // Numeric text and the integer lane compare the same way.
        assert!(criteria_match(
            &P::Eq(LiteralValue::Number(0.3)),
            &LiteralValue::Text("0.30000000000000004".into())
        ));
        assert!(criteria_match(
            &P::Eq(LiteralValue::Int(1_000_000_000_000_006)),
            &LiteralValue::Int(1_000_000_000_000_005)
        ));
        // The 15th digit still counts, near zero too.
        assert!(!criteria_match(
            &P::Eq(LiteralValue::Number(1.0)),
            &LiteralValue::Number(1.00000000000001)
        ));
        assert!(!criteria_match(
            &P::Eq(LiteralValue::Number(0.0)),
            &LiteralValue::Number(1e-13)
        ));
        assert!(criteria_match(&P::Gt(0.0), &LiteralValue::Number(1e-13)));
    }
}
