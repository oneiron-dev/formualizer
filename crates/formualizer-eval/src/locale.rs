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
    /// Only finite numbers are numeric text: Excel has no NaN or infinity, so
    /// "NaN", "inf", "Infinity" and an out-of-range "1E400" (all of which Rust's
    /// float parser accepts) are not numbers, and `VALUE` of them is `#VALUE!`.
    pub fn parse_number_invariant(&self, s: &str) -> Option<f64> {
        let trimmed = s.trim_matches(' ');
        let n = if let Some(without_pct) = trimmed.strip_suffix('%') {
            without_pct.trim_end_matches(' ').parse::<f64>().ok()? / 100.0
        } else {
            trimmed.parse::<f64>().ok()?
        };
        n.is_finite().then_some(n)
    }

    /// Case folding for comparisons; invariant = ASCII lower.
    pub fn fold_case_invariant(&self, s: &str) -> String {
        s.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::Locale;

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
}
