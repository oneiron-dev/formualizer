/// Locale contract for the engine.
///
/// Milestone 0 intentionally uses an invariant locale:
///
/// - Numeric parsing follows Excel for Windows in the en-US region (`.` decimal separator,
///   `,` group separators in the integer part, `$` currency, parentheses negatives and a
///   trailing percent suffix: `"$1,000" -> 1000`, `"(5)" -> -5`, `"90%" -> 0.9`).
/// - Strings are case-folded with ASCII-only rules (`to_ascii_lowercase`).
///
/// This means locale-dependent inputs like `"1.234,56"` are *not* interpreted as numbers.
/// Callers should surface `#VALUE!` for locale-dependent numeric coercions (e.g. `VALUE()`)
/// rather than silently producing a wrong number.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Locale;

impl Locale {
    pub const fn invariant() -> Self {
        Locale
    }

    /// Parse numeric text the way Excel for Windows reads it in the en-US
    /// region, wherever text becomes a number (VALUE, arithmetic, numeric
    /// arguments, criteria): see [`parse_en_us_number`]. Like Excel it
    /// ignores only the spaces around the text; a tab, line feed or no-break
    /// space there leaves it text.
    ///
    /// Only finite numbers are numbers (see [`parse_finite_number`]): `NaN`,
    /// `inf`, `infinity` and `1e400` give `None` (`#VALUE!` where a number is
    /// required).
    pub fn parse_number_invariant(&self, s: &str) -> Option<f64> {
        parse_en_us_number(s.trim_matches(' '))
    }

    /// Case folding for comparisons; invariant = ASCII lower.
    pub fn fold_case_invariant(&self, s: &str) -> String {
        s.to_ascii_lowercase()
    }
}

/// The decimal number `text` spells (`"5"`, `"-2.5E-1"`, `".5"`), when it is
/// finite. Every reading of numeric text goes through here.
///
/// Excel holds only finite numbers, so only those are numeric text: Rust's
/// float spellings `NaN`, `inf` and `infinity` (any case or sign) are text to
/// Excel, and text beyond the double range (`1e400`) is not a number either;
/// all of them give `None`.
pub fn parse_finite_number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A number written in one of the constant number formats Excel for Windows
/// recognizes in the en-US region (Microsoft's VALUE example: `"$1,000"` is
/// 1000). Leading and trailing spaces are already gone. The text is
///
/// - an optional sign and an optional `$`, in either order (`"-$5"`,
///   `"$-5"`), or a number in parentheses, which is negative (`"(1,000)"`,
///   `"($1,000)"`; no sign inside);
/// - digits whose integer part may carry `,` group separators
///   ([`parse_magnitude`]), an optional `.` fraction and `e`/`E` exponent;
/// - an optional trailing `%` (spaces may come before it) that divides by
///   100; a currency amount takes no `%`.
///
/// Anything else (`"--5"`, `"5-"`, `"$5%"`, `"1,23"`, `"inf"`) is not a
/// number, and neither is a value beyond the double range (`"1e400"`): the
/// digits are read by [`parse_finite_number`].
fn parse_en_us_number(text: &str) -> Option<f64> {
    let (negative, body) = match text.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => (false, text),
    };
    let (body, percent) = match body.strip_suffix('%') {
        Some(rest) => (rest.trim_end_matches(' '), true),
        None => (body, false),
    };
    let mut rest = body;
    let mut sign = None;
    let mut currency = false;
    loop {
        if sign.is_none()
            && !negative
            && let Some(r) = rest.strip_prefix(['+', '-'])
        {
            sign = rest.chars().next();
            rest = r;
        } else if !currency && let Some(r) = rest.strip_prefix('$') {
            currency = true;
            rest = r;
        } else {
            break;
        }
    }
    if currency && percent {
        return None;
    }
    let mut n = parse_magnitude(rest)?;
    if percent {
        n /= 100.0;
    }
    if negative || sign == Some('-') {
        n = -n;
    }
    // Excel has no negative zero: "-0" and "(0)" are 0.
    Some(crate::coercion::normalize_zero(n))
}

/// Unsigned digits with an optional `.` fraction and `e`/`E` exponent. The
/// integer part may carry en-US `,` group separators, as Excel reads
/// `"1,234"`: each comma follows a digit and is followed by at least three
/// digits (`"45627,45657"` is 4562745657; `"1,23"`, `"1,"` and `",5"` are not
/// numbers), and none comes after the decimal point or exponent.
fn parse_magnitude(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut plain = String::with_capacity(text.len());
    let mut i = 0;
    let mut digits = 0;
    // Digits since the last group separator, once one has been seen.
    let mut group: Option<usize> = None;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'0'..=b'9' => {
                plain.push(b as char);
                digits += 1;
                if let Some(width) = group.as_mut() {
                    *width += 1;
                }
            }
            b',' if digits > 0 && group.is_none_or(|width| width >= 3) => group = Some(0),
            _ => break,
        }
        i += 1;
    }
    if group.is_some_and(|width| width < 3) {
        return None;
    }
    if bytes.get(i) == Some(&b'.') {
        plain.push('.');
        i += 1;
        while let Some(&b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
            plain.push(b as char);
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if let Some(b'e' | b'E') = bytes.get(i) {
        plain.push('e');
        i += 1;
        if let Some(&b) = bytes.get(i).filter(|b| matches!(b, b'+' | b'-')) {
            plain.push(b as char);
            i += 1;
        }
        let start = i;
        while let Some(&b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
            plain.push(b as char);
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    if i != bytes.len() {
        return None;
    }
    parse_finite_number(&plain)
}

#[cfg(test)]
mod tests {
    use super::{Locale, parse_finite_number};

    #[test]
    fn parse_number_invariant_supports_percent_suffix() {
        let loc = Locale::invariant();
        assert_eq!(loc.parse_number_invariant("90%"), Some(0.9));
        assert_eq!(loc.parse_number_invariant(" 90.5% "), Some(0.905));
        assert_eq!(loc.parse_number_invariant("90 %"), Some(0.9));
    }

    #[test]
    fn parse_number_invariant_ignores_only_surrounding_spaces() {
        let loc = Locale::invariant();
        assert_eq!(loc.parse_number_invariant("  42 "), Some(42.0));
        for text in ["5\n", "\n5", "5\t", "\r5", "\u{a0}5", "5%\n"] {
            assert_eq!(loc.parse_number_invariant(text), None, "{text:?}");
        }
    }

    #[test]
    fn parse_number_invariant_skips_group_separators() {
        let loc = Locale::invariant();
        for (text, n) in [
            ("1,234", 1234.0),
            ("-1,234,567", -1234567.0),
            ("1,234.5", 1234.5),
            ("1,0000", 10000.0),
            ("45627,45657", 4562745657.0),
            ("1,234e2", 123400.0),
            (" 1,234% ", 12.34),
        ] {
            assert_eq!(loc.parse_number_invariant(text), Some(n), "{text}");
        }
        for text in [
            "1,23",
            "12,34,567",
            "1,",
            ",5",
            "1,,234",
            "1.234,5",
            "1e3,000",
            "1,2a4",
        ] {
            assert_eq!(loc.parse_number_invariant(text), None, "{text}");
        }
    }

    #[test]
    fn parse_number_invariant_reads_currency_and_parentheses() {
        let loc = Locale::invariant();
        for (text, n) in [
            ("$1,000", 1000.0),
            (" $1,234.50 ", 1234.5),
            ("-$5", -5.0),
            ("$-5", -5.0),
            ("+$5", 5.0),
            ("(1,000)", -1000.0),
            ("($1,000)", -1000.0),
            ("(2.5%)", -0.025),
            ("-.5", -0.5),
            ("5.", 5.0),
            ("1.5E-3", 0.0015),
        ] {
            assert_eq!(loc.parse_number_invariant(text), Some(n), "{text}");
        }
        // Excel has no negative zero.
        for text in ["-0", "(0)", "-$0.00"] {
            let n = loc.parse_number_invariant(text).unwrap();
            assert!(n == 0.0 && n.is_sign_positive(), "{text}");
        }
        for text in [
            "$",
            "-",
            "()",
            "$$5",
            "--5",
            "+-5",
            "5-",
            "(-5)",
            "-(5)",
            "$(5)",
            "(5",
            "5)",
            "$5%",
            "$ 5",
            "- 5",
            "( 5 )",
            "5$",
            "(0-2)",
            "1 234",
            "inf",
            "-infinity",
            "NaN",
            "1e400",
            "1e",
            "1e+",
            ".",
            "0x10",
        ] {
            assert_eq!(loc.parse_number_invariant(text), None, "{text}");
        }
    }

    #[test]
    fn parse_number_invariant_rejects_invalid_percent_text() {
        let loc = Locale::invariant();
        assert_eq!(loc.parse_number_invariant("abc%"), None);
        assert_eq!(loc.parse_number_invariant("%"), None);
        assert_eq!(loc.parse_number_invariant("90% trailing"), None);
    }

    #[test]
    fn parse_number_invariant_rejects_non_finite_text() {
        let loc = Locale::invariant();
        for text in [
            "NaN",
            "nan",
            "-NaN",
            "inf",
            "-inf",
            "+Infinity",
            "infinity",
            "NaN%",
            "1E400",
            "-1e400",
        ] {
            assert_eq!(loc.parse_number_invariant(text), None, "{text}");
        }
        // Finite numeric text, exponents included, still parses.
        assert_eq!(loc.parse_number_invariant("1E+05"), Some(100000.0));
        assert_eq!(loc.parse_number_invariant(" -2.5e-1 "), Some(-0.25));
    }

    #[test]
    fn parse_number_invariant_rejects_non_finite_spellings() {
        let loc = Locale::invariant();
        for text in [
            "NaN", "nan", "-NaN", "inf", "Inf", "-inf", "+INF", "infinity", "Infinity", " inf ",
            "NaN%", "inf%", "1e400", "-1e400", "1e400%",
        ] {
            assert_eq!(loc.parse_number_invariant(text), None, "{text:?}");
        }
        // Ordinary numeric text is unchanged.
        assert_eq!(loc.parse_number_invariant("1e3"), Some(1000.0));
        assert_eq!(loc.parse_number_invariant("-2.5E-1"), Some(-0.25));
        assert_eq!(loc.parse_number_invariant(" +7 "), Some(7.0));
    }

    #[test]
    fn parse_finite_number_reads_only_finite_numbers() {
        for text in [
            "NaN",
            "nan",
            "-NaN",
            "+nan",
            "inf",
            "-inf",
            "+Inf",
            "INFINITY",
            "-Infinity",
            "1e400",
            "-1e309",
            "",
        ] {
            assert_eq!(parse_finite_number(text), None, "{text:?}");
        }
        assert_eq!(parse_finite_number("5"), Some(5.0));
        assert_eq!(parse_finite_number(".5"), Some(0.5));
        assert_eq!(parse_finite_number("-2.5E-1"), Some(-0.25));
        assert_eq!(
            parse_finite_number("1.7976931348623157e308"),
            Some(f64::MAX)
        );
    }
}
