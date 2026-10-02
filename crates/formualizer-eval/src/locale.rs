/// Locale contract for the engine.
///
/// Milestone 0 intentionally uses an invariant locale:
///
/// - Numeric parsing is ASCII/invariant only (`.` decimal separator; no thousands separators),
///   with support for trailing percent suffix (`"90%" -> 0.9`).
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

    /// Parse a number using invariant rules (ASCII, dot decimal separator).
    ///
    /// Also supports percent-suffixed numeric text (e.g. "90%" -> 0.9),
    /// matching spreadsheet numeric-coercion behavior in numeric contexts.
    /// Like Excel it ignores only the spaces around the text; a tab, line
    /// feed or no-break space there leaves it text.
    ///
    /// Only finite numbers are numbers (see [`parse_finite_number`]): `NaN`,
    /// `inf`, `infinity` and `1e400` give `None` (`#VALUE!` where a number is
    /// required).
    pub fn parse_number_invariant(&self, s: &str) -> Option<f64> {
        let trimmed = s.trim_matches(' ');
        if let Some(without_pct) = trimmed.strip_suffix('%') {
            parse_finite_number(without_pct.trim_end_matches(' ')).map(|n| n / 100.0)
        } else {
            parse_finite_number(trimmed)
        }
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
