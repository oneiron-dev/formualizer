use formualizer_common::{DateSystem, ExcelError, ExcelErrorKind, LiteralValue};

/// Centralized coercion and error policy utilities (Milestone 7).
/// These functions implement invariant, Excel-compatible coercions and
/// numeric sanitization. They should be used by the interpreter, builtins,
/// and evaluation pipelines (map/fold/window) instead of ad-hoc parsing.
/// Strict numeric coercion.
/// - Accepts Number/Int/Boolean/Empty/Date-like serial-bearing variants
/// - Rejects Text (returns #VALUE!)
pub fn to_number_strict(value: &LiteralValue) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Number(n) => Ok(*n),
        LiteralValue::Int(i) => Ok(*i as f64),
        LiteralValue::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
        LiteralValue::Empty => Ok(0.0),
        // Date/time/duration map to serials
        other if other.as_serial_number().is_some() => Ok(other.as_serial_number().unwrap()),
        LiteralValue::Error(e) => Err(e.clone()),
        _ => Err(ExcelError::new(ExcelErrorKind::Value)
            .with_message("Cannot convert to number (strict)")),
    }
}

/// Lenient numeric coercion.
/// - As strict, but also parses numeric text using ASCII/invariant rules
pub fn to_number_lenient(value: &LiteralValue) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Text(s) => crate::locale::Locale::invariant()
            .parse_number_invariant(s)
            .ok_or_else(|| {
                ExcelError::new(ExcelErrorKind::Value)
                    .with_message(format!("Cannot convert '{s}' to number"))
            }),
        _ => to_number_strict(value),
    }
}

thread_local! {
    /// Date system and clock year of the function call being evaluated on
    /// this thread, which [`to_number_argument`] reads date text in.
    static ARGUMENT_DATE_CONTEXT: std::cell::Cell<(DateSystem, Option<i32>)> =
        const { std::cell::Cell::new((DateSystem::Excel1900, None)) };
}

/// Restores the enclosing call's argument date context when dropped.
pub(crate) struct ArgumentDateContextGuard {
    previous: (DateSystem, Option<i32>),
}

impl Drop for ArgumentDateContextGuard {
    fn drop(&mut self) {
        ARGUMENT_DATE_CONTEXT.with(|context| context.set(self.previous));
    }
}

/// Make [`to_number_argument`] read date text in `system`, with year-less
/// dates in `current_year`, until the returned guard drops. The interpreter
/// enters it for every builtin call it makes
/// (`Interpreter::enter_function_call`), before the function runs, so no
/// function's own `dispatch` can bypass it.
pub(crate) fn enter_argument_date_context(
    system: DateSystem,
    current_year: Option<i32>,
) -> ArgumentDateContextGuard {
    let previous = ARGUMENT_DATE_CONTEXT.with(|context| context.replace((system, current_year)));
    ArgumentDateContextGuard { previous }
}

/// Excel's conversion of a value passed to a function's number parameter.
///
/// Text converts as VALUE() and the arithmetic operators convert it: numeric
/// text as [`to_number_lenient`], then date and time text in en-US order
/// (`"01/09/2020 15:02:40"` is 9 January 2020, 15:02:40), so `INT(A2)` with
/// that text in A2 is the date's serial rather than `#VALUE!`. Date text must
/// name a day of the date system's range (January 1, 1900 or 1904 through
/// December 31, 9999); other text is `#VALUE!`. The date system and the year
/// of year-less dates are those of the function call being evaluated
/// ([`enter_argument_date_context`]); outside a call they are the 1900 system
/// and no year-less dates.
///
/// Only a single value converts this way. Cells of a range and elements of an
/// array argument that Excel skips or zeroes when they hold text keep
/// [`to_number_lenient`].
pub fn to_number_argument(value: &LiteralValue) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Text(s) => to_number_lenient(value).or_else(|error| {
            let (system, current_year) = ARGUMENT_DATE_CONTEXT.with(std::cell::Cell::get);
            formualizer_common::parse_excel_datetime_text_to_serial_in_year_for(
                system,
                s,
                current_year,
            )
            .ok_or(error)
        }),
        _ => to_number_lenient(value),
    }
}

/// Lenient numeric coercion that resolves temporal values in a date system.
///
/// Identical to [`to_number_lenient`] except that date-bearing literals are
/// converted to serials using the workbook's date system instead of the
/// implicit Excel-1900 default.
pub fn to_serial_lenient(value: &LiteralValue, system: DateSystem) -> Result<f64, ExcelError> {
    to_serial_lenient_in_year(value, system, None)
}

/// [`to_serial_lenient`] that also reads date/time text (the date functions'
/// argument coercion); year-less date text reads in `current_year`.
pub fn to_serial_lenient_in_year(
    value: &LiteralValue,
    system: DateSystem,
    current_year: Option<i32>,
) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => value.as_serial_number_for(system).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Value)
                .with_message("Cannot convert to date/time serial")
        }),
        LiteralValue::Text(s) => to_number_lenient(value).or_else(|error| {
            formualizer_common::parse_excel_datetime_text_to_serial_in_year_for(
                system,
                s,
                current_year,
            )
            .ok_or(error)
        }),
        _ => to_number_lenient(value),
    }
}

/// Strict numeric coercion that resolves temporal values in a date system.
///
/// Identical to [`to_number_strict`] except that date-bearing literals are
/// converted to serials using the workbook's date system instead of the
/// implicit Excel-1900 default. Unlike [`to_serial_lenient`] it does **not**
/// parse numeric text, so callers that reject text today keep rejecting it.
///
/// This is the coercion financial builtins want for arguments Excel treats as
/// plain numbers on the sheet: a date cell *is* a number there, so a
/// `Date`/`DateTime`/`Time`/`Duration` literal must become its serial rather
/// than being dropped or rejected.
pub(crate) fn to_serial_strict(
    value: &LiteralValue,
    system: DateSystem,
) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => value.as_serial_number_for(system).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Value)
                .with_message("Cannot convert to date/time serial")
        }),
        _ => to_number_strict(value),
    }
}

/// Context-aware lenient numeric coercion using locale.
pub fn to_number_lenient_with_locale(
    value: &LiteralValue,
    loc: &crate::locale::Locale,
) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Text(s) => loc.parse_number_invariant(s).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Value)
                .with_message(format!("Cannot convert '{s}' to number"))
        }),
        _ => to_number_strict(value),
    }
}

/// Numeric coercion for arithmetic operators.
///
/// This deliberately extends only the operator boundary: numeric text is
/// parsed first using the engine's invariant locale, then date/time text is
/// parsed by `formualizer-common` and encoded in the workbook's date system.
/// Aggregate arguments, comparisons, criteria, and functions such as `N`
/// continue to use their existing coercion policies.
/// Year-less date text (`Jan 3`) reads in `current_year`, Excel's clock year.
pub(crate) fn to_arithmetic_number_with_locale(
    value: &LiteralValue,
    loc: &crate::locale::Locale,
    system: DateSystem,
    current_year: Option<i32>,
) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Text(s) => loc
            .parse_number_invariant(s)
            .or_else(|| {
                formualizer_common::parse_excel_datetime_text_to_serial_in_year_for(
                    system,
                    s,
                    current_year,
                )
            })
            .ok_or_else(|| {
                ExcelError::new(ExcelErrorKind::Value)
                    .with_message(format!("Cannot convert '{s}' to arithmetic operand"))
            }),
        LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => value.as_serial_number_for(system).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Value)
                .with_message("Cannot convert date/time to arithmetic operand")
        }),
        _ => to_number_strict(value),
    }
}

/// Logical coercion.
/// - Accepts Boolean
/// - Numbers: nonzero → true, zero → false
/// - Text: "TRUE"/"FALSE" (ASCII case-insensitive)
pub fn to_logical(value: &LiteralValue) -> Result<bool, ExcelError> {
    match value {
        LiteralValue::Boolean(b) => Ok(*b),
        LiteralValue::Number(n) => Ok(*n != 0.0),
        LiteralValue::Int(i) => Ok(*i != 0),
        LiteralValue::Text(s) => match s.to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ExcelError::new(ExcelErrorKind::Value)
                .with_message("Cannot convert text to logical")),
        },
        LiteralValue::Empty => Ok(false),
        LiteralValue::Error(e) => Err(e.clone()),
        _ => Err(ExcelError::new(ExcelErrorKind::Value).with_message("Cannot convert to logical")),
    }
}

/// Excel's text for a number used where text is needed (`&`, CONCAT, LEFT,
/// TEXTJOIN, ...), whatever the cell's display format. It keeps 15
/// significant digits (from 1E+99 up and below 1E-98 those 15 are rounded
/// again, to 14) and writes the number out in full up to 20 integer
/// digits, or below 1 while that takes
/// at most 20 characters; beyond that it uses E notation
/// (`0.333333333333333`, `1234567890123460`, `1.23456789012346E-05`,
/// `1.23456789012346E+20`). Like all of Excel's number formatting it rounds
/// half away from zero (100000000000000.5 is `100000000000001`). Zero has
/// no sign, and subnormal values read as 0.
pub fn number_to_text(n: f64) -> String {
    if !n.is_finite() {
        return n.to_string();
    }
    if n.abs() < f64::MIN_POSITIVE {
        return "0".into();
    }
    let (mut digits, mut exponent) = significant_digits(n.abs(), 15);
    if exponent.abs() > 98 {
        // Excel reaches 14 digits from the 15 it has already rounded to, not
        // from the binary value: 1.23456789012355E+99 is
        // `1.2345678901236E+99`, and a carry moves the exponent
        // (9.99999999999995E+99 is `1E+100`).
        let fifteen: u64 = digits.parse().expect("15 decimal digits");
        let fourteen = (fifteen + 5) / 10;
        if fourteen < 100_000_000_000_000 {
            digits = fourteen.to_string();
        } else {
            digits = "1".into();
            exponent += 1;
        }
    }
    let digits = digits.trim_end_matches('0');
    let sign = if n < 0.0 { "-" } else { "" };
    let leading_zeros = (-exponent - 1).max(0) as usize;
    if exponent > 19 || (exponent < 0 && 2 + leading_zeros + digits.len() > 20) {
        let (head, tail) = digits.split_at(1);
        let point = if tail.is_empty() { "" } else { "." };
        let esign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{head}{point}{tail}E{esign}{:02}", exponent.abs());
    }
    if exponent < 0 {
        return format!("{sign}0.{}{digits}", "0".repeat(leading_zeros));
    }
    let int_len = exponent as usize + 1;
    if digits.len() > int_len {
        format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
    } else {
        format!("{sign}{digits}{}", "0".repeat(int_len - digits.len()))
    }
}

/// `number_to_text` for a number the engine holds as an integer. Excel has
/// only one kind of number, so the text is the same as for the equal double:
/// 2^53 is `9007199254740990`, not `9007199254740992`.
pub fn int_to_text(i: i64) -> String {
    if i.unsigned_abs() < 1_000_000_000_000_000 {
        // Up to 15 digits an integer is its own text.
        i.to_string()
    } else {
        number_to_text(i as f64)
    }
}

/// The positive normal `a` rounded half up to `sig` significant digits: the
/// digits, and the decimal exponent of the first one.
fn significant_digits(a: f64, sig: usize) -> (String, i32) {
    let split = |sci: String| {
        let (mantissa, exponent) = sci.split_once('e').expect("scientific");
        let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
        (digits, exponent.parse::<i32>().expect("exponent"))
    };
    // Rust's formatting rounds the exact binary value correctly, but an exact
    // tie goes to even. A tie is a value that takes exactly one more digit,
    // a final 5; the next double up lies just past it and rounds up.
    let (longer, exponent) = split(format!("{a:.sig$e}"));
    let tie = longer.ends_with('5')
        && longer
            .parse::<u64>()
            .is_ok_and(|d| equals_decimal(a, d, exponent - sig as i32));
    let a = if tie { a.next_up() } else { a };
    split(format!("{a:.prec$e}", prec = sig - 1))
}

/// Whether the positive normal `a` is exactly `digits × 10^exponent`, for
/// odd `digits`.
fn equals_decimal(a: f64, digits: u64, exponent: i32) -> bool {
    let bits = a.to_bits();
    let significand = (bits & ((1 << 52) - 1)) | (1 << 52);
    let shift = significand.trailing_zeros();
    // a = odd × 2^power, and digits × 10^exponent is digits × 5^exponent (an
    // odd number, or a fraction with odd parts) × 2^exponent, so the powers of
    // two must match and then the odd parts: digits × 5^exponent = odd.
    let (odd, power) = (
        u128::from(significand >> shift),
        ((bits >> 52) & 0x7ff) as i32 - 1075 + shift as i32,
    );
    if power != exponent {
        return false;
    }
    let five = 5u128.checked_pow(exponent.unsigned_abs());
    let digits = u128::from(digits);
    if exponent >= 0 {
        five.and_then(|f| f.checked_mul(digits)) == Some(odd)
    } else {
        five.and_then(|f| f.checked_mul(odd)) == Some(digits)
    }
}

/// Invariant textification for comparisons/concatenation.
pub fn to_text_invariant(value: &LiteralValue) -> String {
    match value {
        LiteralValue::Text(s) => s.clone(),
        LiteralValue::Number(n) => number_to_text(*n),
        LiteralValue::Int(i) => int_to_text(*i),
        LiteralValue::Boolean(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        LiteralValue::Error(e) => e.to_string(),
        LiteralValue::Empty => "".into(),
        // Dates/times/durations are stored as serial numbers in spreadsheet engines.
        // Use invariant numeric serialization so downstream consumers (e.g., criteria strings
        // like ">="&A1) parse consistently.
        LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => number_to_text(value.as_serial_number().unwrap_or(0.0)),
        other => format!("{other:?}"),
    }
}

/// Numeric sanitization: NaN/Inf → #NUM!
/// Excel's `^` and POWER. `0^0` is `#NUM!` and `0` to a negative power is
/// `#DIV/0!`. A negative base with a fractional exponent is `#NUM!` unless
/// the exponent is the reciprocal of an odd integer, which takes the real
/// odd root (`(-8)^(1/3)` is -2). Overflow is `#NUM!`.
pub fn excel_power(base: f64, exponent: f64) -> Result<f64, ExcelError> {
    if base == 0.0 {
        if exponent == 0.0 {
            return Err(ExcelError::new_num());
        }
        if exponent < 0.0 {
            return Err(ExcelError::new_div());
        }
    }
    if base < 0.0 && exponent.fract() != 0.0 {
        let root = 1.0 / exponent;
        let whole = root.round();
        if (root - whole).abs() <= 1e-10 * whole.abs().max(1.0) && whole.rem_euclid(2.0) == 1.0 {
            return sanitize_numeric(-(-base).powf(exponent));
        }
        return Err(ExcelError::new_num());
    }
    sanitize_numeric(base.powf(exponent))
}

pub fn sanitize_numeric(n: f64) -> Result<f64, ExcelError> {
    if n.is_nan() || n.is_infinite() {
        return Err(ExcelError::new_num());
    }
    Ok(n)
}

/// Coerce to Excel serial (date/time/duration) or error.
pub fn to_datetime_serial(value: &LiteralValue) -> Result<f64, ExcelError> {
    match value.as_serial_number() {
        Some(n) => Ok(n),
        None => Err(ExcelError::new(ExcelErrorKind::Value)
            .with_message("Cannot convert to date/time serial")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_lenient_parses_text_and_booleans() {
        assert_eq!(
            to_number_lenient(&LiteralValue::Text(" 42 ".into())).unwrap(),
            42.0
        );
        assert_eq!(
            to_number_lenient(&LiteralValue::Boolean(true)).unwrap(),
            1.0
        );
        assert_eq!(to_number_lenient(&LiteralValue::Empty).unwrap(), 0.0);
    }

    #[test]
    fn number_lenient_parses_percent_text() {
        assert_eq!(
            to_number_lenient(&LiteralValue::Text("90%".into())).unwrap(),
            0.9
        );
        assert_eq!(
            to_number_lenient(&LiteralValue::Text(" 90.5% ".into())).unwrap(),
            0.905
        );
        assert!(to_number_lenient(&LiteralValue::Text("abc%".into())).is_err());
    }

    #[test]
    fn number_argument_reads_date_text_in_the_call_context() {
        let text = |s: &str| LiteralValue::Text(s.into());
        // Outside a function call: the 1900 system, no year-less dates.
        assert_eq!(to_number_argument(&text(" 42 ")).unwrap(), 42.0);
        assert_eq!(to_number_argument(&text("1/1/03")).unwrap(), 37_622.0);
        assert_eq!(to_number_argument(&text("12:00")).unwrap(), 0.5);
        assert!(to_number_argument(&text("Jan 3")).is_err());
        assert!(to_number_argument(&text("abc")).is_err());
        assert!(to_number_lenient(&text("1/1/03")).is_err());
        // Date text names a day of the date system's range.
        assert_eq!(to_number_argument(&text("1/1/1900")).unwrap(), 1.0);
        assert!(to_number_argument(&text("12/31/1899")).is_err());
        {
            let _system = enter_argument_date_context(DateSystem::Excel1904, None);
            assert_eq!(to_number_argument(&text("1/1/1904")).unwrap(), 0.0);
            assert!(to_number_argument(&text("12/31/1903")).is_err());
        }
        {
            let _outer = enter_argument_date_context(DateSystem::Excel1904, Some(2024));
            assert_eq!(to_number_argument(&text("1/1/03")).unwrap(), 36_160.0);
            {
                let _inner = enter_argument_date_context(DateSystem::Excel1900, Some(2003));
                assert_eq!(to_number_argument(&text("Jan 1")).unwrap(), 37_622.0);
            }
            // Leaving the inner call restores the outer call's context.
            assert_eq!(to_number_argument(&text("Jan 1 2003")).unwrap(), 36_160.0);
            assert_eq!(to_number_argument(&text("Jan 3")).unwrap(), 43_832.0);
        }
        assert!(to_number_argument(&text("Jan 3")).is_err());
        assert_eq!(to_number_argument(&text("1/1/03")).unwrap(), 37_622.0);
    }

    #[test]
    fn number_strict_rejects_text() {
        assert!(to_number_strict(&LiteralValue::Text("1".into())).is_err());
    }

    #[test]
    fn logical_from_number_and_text() {
        assert!(to_logical(&LiteralValue::Int(5)).unwrap());
        assert!(!to_logical(&LiteralValue::Number(0.0)).unwrap());
        assert!(to_logical(&LiteralValue::Text("TRUE".into())).unwrap());
        assert!(to_logical(&LiteralValue::Text("true".into())).unwrap());
        assert!(to_logical(&LiteralValue::Text(" True ".into())).is_err());
    }

    #[test]
    fn number_to_text_keeps_15_significant_digits() {
        // Excel's own renderings, as tabulated by Apache POI's
        // NumberToTextConverter.
        for (n, text) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (1.0001, "1.0001"),
            (756.0, "756"),
            (123.45678901234568, "123.456789012346"),
            (1234567.8901234567, "1234567.89012346"),
            (1.2345678901234568E-5, "1.23456789012346E-05"),
            (1.2345678901234567E-4, "0.000123456789012346"),
            (1.23456789E-5, "0.0000123456789"),
            (5.6789012345E-8, "0.000000056789012345"),
            (5.67890123456E-8, "5.67890123456E-08"),
            (9.999999999999123E-98, "9.99999999999912E-98"),
            (1.0000000000001235E-99, "1.0000000000001E-99"),
            (2.0E-50, "2E-50"),
            (1.2345678901234568E13, "12345678901234.6"),
            (1.2345678901234567E14, "123456789012346"),
            (1.2345678901234568E15, "1234567890123460"),
            (1.2345678901234567E19, "12345678901234600000"),
            (1.2345678901234568E20, "1.23456789012346E+20"),
            (-1.2345678901234567E19, "-12345678901234600000"),
            (-1.2345678901234568E20, "-1.23456789012346E+20"),
            (1.2345678901234576E100, "1.2345678901235E+100"),
            (1.7976931348623157E308, "1.7976931348623E+308"),
            (2.2250738585072014E-308, "2.2250738585072E-308"),
            (2.225073858507201E-308, "0"),
            (123499.9999999999, "123500"),
            (9.999999999999999E20, "1E+21"),
            (999999.9999999999, "1000000"),
            (9.999999999999999E-19, "0.000000000000000001"),
            (9.999999999999999E-20, "1E-19"),
            (-9.999999999999999E-9, "-0.00000001"),
            (100.6666666666667, "100.666666666667"),
            (0.1 + 0.2, "0.3"),
        ] {
            assert_eq!(number_to_text(n), text, "{n:e}");
        }
    }

    #[test]
    fn number_to_text_rounds_ties_away_from_zero() {
        for (n, text) in [
            // Exact ties at the 16th significant digit round away from zero.
            (1234567890123.125, "1234567890123.13"),
            (100000000000000.5, "100000000000001"),
            (-100000000000000.5, "-100000000000001"),
            (1e15 + 5.0, "1000000000000010"),
            (70489670895608.25, "70489670895608.3"),
            (999999999999999.5, "1000000000000000"),
            (13.0 / 1048576.0, "1.23977661132813E-05"),
            (-13.0 / 1048576.0, "-1.23977661132813E-05"),
            // Near ties round to the nearer side of the exact value.
            (0.1234567890123455, "0.123456789012345"),
            (0.3000000000000005, "0.3"),
            (1.000000000000005, "1.00000000000001"),
            (123456789012.3455, "123456789012.346"),
        ] {
            assert_eq!(number_to_text(n), text, "{n:e}");
        }
    }

    #[test]
    fn number_to_text_rounds_the_15_digits_again_at_extreme_exponents() {
        // Excel's renderings around 1E±98, 1E±99 and 1E±100, as tabulated by
        // Apache POI's NumberToTextConversionExamples (raw double bits). From
        // 1E+99 up and below 1E-98 the 15-digit result is rounded half up to
        // 14 digits, and a carry moves the exponent.
        for (bits, text) in [
            (0x544C_E634_5CF3_209C_u64, "1.23456789012345E+98"),
            (0x544C_E634_5CF3_209D, "1.23456789012346E+98"),
            (0x544C_E634_5CF3_20DF, "1.23456789012347E+98"),
            (0x544C_E634_5CF3_2121, "1.23456789012348E+98"),
            (0x5482_0FE0_BA17_F5E9, "1.2345678901236E+99"),
            (0x5482_0FE0_BA17_F5EA, "1.2345678901236E+99"),
            (0x5482_0FE0_BA17_F784, "1.2345678901237E+99"),
            (0x5482_0FE0_BA17_F785, "1.2345678901237E+99"),
            (0x5482_0FE0_BA17_F920, "1.2345678901238E+99"),
            (0x5482_0FE0_BA17_F921, "1.2345678901238E+99"),
            (0x547D_42AE_A287_9F19, "9.99999999999997E+98"),
            (0x547D_42AE_A287_9F1A, "9.99999999999998E+98"),
            (0x547D_42AE_A287_9F2A, "9.99999999999999E+98"),
            (0x547D_42AE_A287_9F2B, "1E+99"),
            (0x547D_42AE_A287_A0A0, "1E+99"),
            (0x547D_42AE_A287_A0A1, "1.0000000000001E+99"),
            (0x547D_42AE_A287_A3D8, "1.0000000000001E+99"),
            (0x547D_42AE_A287_A3D9, "1.0000000000002E+99"),
            (0x547D_42AE_A287_A710, "1.0000000000002E+99"),
            (0x547D_42AE_A287_A711, "1.0000000000003E+99"),
            (0x54B2_49AD_2594_C2F9, "9.9999999999997E+99"),
            (0x54B2_49AD_2594_C2FA, "9.9999999999998E+99"),
            (0x54B2_49AD_2594_C32D, "9.9999999999998E+99"),
            (0x54B2_49AD_2594_C32E, "9.9999999999999E+99"),
            (0x54B2_49AD_2594_C360, "9.9999999999999E+99"),
            (0x54B2_49AD_2594_C361, "1E+100"),
            (0x54B2_49AD_2594_C464, "1E+100"),
            (0x54B2_49AD_2594_C465, "1.0000000000001E+100"),
            (0x54B2_49AD_2594_C667, "1.0000000000001E+100"),
            (0x54B2_49AD_2594_C668, "1.0000000000002E+100"),
            (0x54B2_49AD_2594_C86A, "1.0000000000002E+100"),
            (0x54B2_49AD_2594_C86B, "1.0000000000003E+100"),
            (0x2B95_DF5C_A28E_F4A8, "1.00000000000003E-98"),
            (0x2B95_DF5C_A28E_F4A7, "1.00000000000002E-98"),
            (0x2B95_DF5C_A28E_F42C, "1E-98"),
            (0x2B95_DF5C_A28E_F3EC, "1E-98"),
            (0x2B95_DF5C_A28E_F3EB, "9.9999999999999E-99"),
            (0x2B95_DF5C_A28E_F3AE, "9.9999999999999E-99"),
            (0x2B95_DF5C_A28E_F3AD, "9.9999999999998E-99"),
            (0x2B95_DF5C_A28E_F371, "9.9999999999998E-99"),
            (0x2B95_DF5C_A28E_F370, "9.9999999999997E-99"),
            (0x2B61_7F7D_4ED8_C7F5, "1.0000000000003E-99"),
            (0x2B61_7F7D_4ED8_C7F4, "1.0000000000002E-99"),
            (0x2B61_7F7D_4ED8_C609, "1.0000000000002E-99"),
            (0x2B61_7F7D_4ED8_C608, "1.0000000000001E-99"),
            (0x2B61_7F7D_4ED8_C41C, "1.0000000000001E-99"),
            (0x2B61_7F7D_4ED8_C41B, "1E-99"),
            (0x2B61_7F7D_4ED8_C323, "1E-99"),
            (0x2B61_7F7D_4ED8_C322, "9.9999999999999E-100"),
            (0x2B61_7F7D_4ED8_C2F2, "9.9999999999999E-100"),
            (0x2B61_7F7D_4ED8_C2F1, "9.9999999999998E-100"),
            (0x2B61_7F7D_4ED8_C2C1, "9.9999999999998E-100"),
            (0x2B61_7F7D_4ED8_C2C0, "9.9999999999997E-100"),
            (0x0036_3199_16D6_7853, "1.2345678901235E-307"),
        ] {
            let n = f64::from_bits(bits);
            assert_eq!(number_to_text(n), text, "{n:e}");
            assert_eq!(number_to_text(-n), format!("-{text}"), "{:e}", -n);
        }
    }

    #[test]
    fn int_to_text_matches_the_equal_double() {
        for (i, text) in [
            (0_i64, "0"),
            (-7, "-7"),
            (999_999_999_999_999, "999999999999999"),
            (-999_999_999_999_999, "-999999999999999"),
            (1_000_000_000_000_000, "1000000000000000"),
            (1_000_000_000_000_005, "1000000000000010"),
            (1 << 53, "9007199254740990"),
            (-(1 << 53), "-9007199254740990"),
            (12_345_678_901_234_567, "12345678901234600"),
            (i64::MAX, "9223372036854780000"),
            (i64::MIN, "-9223372036854780000"),
        ] {
            assert_eq!(int_to_text(i), text, "{i}");
            assert_eq!(int_to_text(i), number_to_text(i as f64), "{i}");
        }
        for value in [
            LiteralValue::Int(1 << 53),
            LiteralValue::Number(2f64.powi(53)),
        ] {
            assert_eq!(to_text_invariant(&value), "9007199254740990", "{value:?}");
        }
    }

    /// `a` rounded half up to `sig` significant digits from its full
    /// decimal expansion.
    fn half_up_from_exact_digits(a: f64, sig: usize) -> (String, i32) {
        // A double's exact decimal expansion has under 800 significant digits.
        let exact = format!("{a:.1100e}");
        let (mantissa, exponent) = exact.split_once('e').unwrap();
        let all: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
        let mut kept = all[..sig].to_vec();
        let mut exponent: i32 = exponent.parse().unwrap();
        if all[sig] >= b'5' {
            match kept.iter().rposition(|&d| d != b'9') {
                Some(i) => {
                    kept[i] += 1;
                    kept[i + 1..].fill(b'0');
                }
                None => {
                    kept.fill(b'0');
                    kept[0] = b'1';
                    exponent += 1;
                }
            }
        }
        (String::from_utf8(kept).unwrap(), exponent)
    }

    #[test]
    fn significant_digits_round_the_exact_value_half_up() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state
        };
        let mut values = Vec::new();
        for _ in 0..20_000 {
            let a = f64::from_bits(next() >> 1);
            if a.is_normal() {
                values.push(a);
            }
        }
        // Exact ties at the 16th digit: odd × 2^k equal to a 16-digit decimal
        // ending in 5, so odd × 5^-k (k < 0) or odd / 5^k (k >= 0) is that
        // decimal, which needs -22 <= k <= 1.
        let mut ties = 0;
        for k in -22i32..=1 {
            let five = 5u64.pow(k.unsigned_abs());
            for _ in 0..200 {
                let odd = if k >= 0 {
                    (1_000_000_000_000_005 + 10 * (next() % 900_000_000_000_000)) * five
                } else {
                    let low = 1_000_000_000_000_000u64.div_ceil(five);
                    let high = 10_000_000_000_000_000 / five;
                    (low + next() % (high - low)) | 1
                };
                if odd < 1 << 53 {
                    values.push(odd as f64 * 2f64.powi(k));
                    ties += 1;
                }
            }
        }
        assert!(ties > 500, "{ties} ties");
        for a in values {
            for sig in [15, 14] {
                assert_eq!(
                    significant_digits(a, sig),
                    half_up_from_exact_digits(a, sig),
                    "{a:e} to {sig} digits"
                );
            }
        }
    }

    #[test]
    fn sanitize_numeric_nan_inf() {
        assert!(sanitize_numeric(f64::NAN).is_err());
        assert!(sanitize_numeric(f64::INFINITY).is_err());
        assert_eq!(sanitize_numeric(1.5).unwrap(), 1.5);
    }
}
