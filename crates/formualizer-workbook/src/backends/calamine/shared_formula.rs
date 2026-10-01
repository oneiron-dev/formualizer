//! Expansion of xlsx shared-formula members from their master's text.
//!
//! ECMA-376 Part 1, 18.3.1.40 (`f`, attributes `si` and `t="shared"`): a
//! member's formula is the master's formula placed at the member's location,
//! i.e. the two have the same R1C1 form. Only relative cell references move
//! with the location. Sheet names, names, string literals and the contents of
//! structured references are not references to cells and never move.
//!
//! Calamine's expander shifts every alphanumeric token that parses as an A1
//! reference. Excel quotes a sheet name that looks like a reference (`'Q1'`,
//! `'FY2024'`, `'Jan2024'`), and calamine does not treat `'` as quoting, so
//! such a name was shifted as if it were a cell and the member pointed at
//! another sheet. This module hands calamine only the text outside those
//! constructs and copies them through unchanged.

use calamine::XlsxError;

/// Expand the master formula `formula` at `master` to `member` (zero-based
/// `(row, col)`), writing the member's formula into `out`.
///
/// Relative references shift by `member - master` exactly as calamine shifts
/// them. These parts are copied verbatim:
/// - quoted sheet names (`'Q1'`, `'Jan:Dec'`, `'[1]Q1'`, with `''` escapes);
/// - unquoted sheet-name prefixes, i.e. the token before `!` (`Sheet1`,
///   the 3D form `Jan:Dec`, `#REF!`'s `REF`);
/// - bracketed parts: structured references (`Sales[[#This Row],[Q1]]`,
///   with `'` escapes) and external-book indexes (`[1]`);
/// - a name directly before `[` (a table name) and any token containing a
///   non-ASCII letter (a name or unquoted sheet name, never a cell).
///
/// String literals are left to calamine, which already keeps them intact.
pub(super) fn expand_shared_formula_into(
    formula: &str,
    master: (u32, u32),
    member: (u32, u32),
    out: &mut String,
) -> Result<(), XlsxError> {
    let bytes = formula.as_bytes();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = string_end(bytes, i),
            b'\'' => {
                let end = quoted_name_end(bytes, i);
                kept.push((i, end));
                i = end;
            }
            b'[' => {
                let end = bracket_end(bytes, i);
                kept.push((i, end));
                i = end;
            }
            b if is_token_byte(b) => {
                let start = i;
                while i < bytes.len() && is_token_byte(bytes[i]) {
                    i += 1;
                }
                if matches!(bytes.get(i), Some(b'!' | b'['))
                    || bytes[start..i].iter().any(|b| !b.is_ascii())
                {
                    kept.push((start, i));
                }
            }
            _ => i += 1,
        }
    }

    if kept.is_empty() {
        return calamine::expand_shared_formula_into(formula, master, member, out);
    }

    let mut result = std::mem::take(out);
    result.clear();
    let mut shifted = String::new();
    let mut span_start = 0;
    for (start, end) in kept {
        if span_start < start {
            calamine::expand_shared_formula_into(
                &formula[span_start..start],
                master,
                member,
                &mut shifted,
            )?;
            result.push_str(&shifted);
        }
        result.push_str(&formula[start..end]);
        span_start = end;
    }
    if span_start < formula.len() {
        calamine::expand_shared_formula_into(&formula[span_start..], master, member, &mut shifted)?;
        result.push_str(&shifted);
    }
    *out = result;
    Ok(())
}

/// Bytes calamine groups into one reference-or-name token, plus non-ASCII
/// bytes so that a name with a non-ASCII letter stays one token.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'\\' | b'.' | b'_' | b'$' | b':') || !b.is_ascii()
}

/// End (exclusive) of the string literal opening at `start`; `""` is an
/// escaped quote. An unterminated literal runs to the end of the formula.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if bytes.get(i + 1) == Some(&b'"') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// End (exclusive) of the quoted sheet name opening at `start`; `''` is an
/// escaped apostrophe.
fn quoted_name_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// End (exclusive) of the bracketed part opening at `start`, with nested
/// brackets; inside it `'` escapes the next character (`[`, `]`, `#`, `'`).
fn bracket_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                i += 2;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::expand_shared_formula_into;

    fn expand(formula: &str, master: (u32, u32), member: (u32, u32)) -> String {
        let mut out = String::from("stale");
        expand_shared_formula_into(formula, master, member, &mut out).unwrap();
        out
    }

    #[test]
    fn quoted_sheet_names_that_look_like_references_never_shift() {
        // Excel quotes these names because they read as A1 references.
        assert_eq!(expand("'Q1'!A1*2", (0, 1), (1, 1)), "'Q1'!A2*2");
        assert_eq!(
            expand("SUM('FY2024'!B2:B9)+'Jan2024'!$A1", (0, 0), (0, 1)),
            "SUM('FY2024'!C2:C9)+'Jan2024'!$A1"
        );
        // Negative offsets (members left of or above the master) as well.
        assert_eq!(expand("'FY2024'!C3", (2, 2), (1, 1)), "'FY2024'!B2");
        // R1C1-looking names, 3D spans, external books and '' escapes.
        assert_eq!(
            expand("'R1C1'!A1+'C3'!A1", (0, 0), (1, 1)),
            "'R1C1'!B2+'C3'!B2"
        );
        assert_eq!(expand("SUM('Q1:Q4'!A1)", (0, 0), (0, 2)), "SUM('Q1:Q4'!C1)");
        assert_eq!(expand("'[1]Q1'!A1", (0, 0), (3, 0)), "'[1]Q1'!A4");
        assert_eq!(expand("'Q1''s'!A1&A1", (0, 0), (1, 0)), "'Q1''s'!A2&A2");
    }

    #[test]
    fn unquoted_sheet_prefixes_never_shift() {
        assert_eq!(expand("Sheet1!A1+B1", (0, 0), (1, 1)), "Sheet1!B2+C2");
        // A 3D span between unquoted sheets reads as a column range.
        assert_eq!(expand("SUM(Jan:Dec!B2)", (0, 0), (0, 1)), "SUM(Jan:Dec!C2)");
        // A writer that did not quote a cell-like sheet name.
        assert_eq!(expand("Q1!A1", (0, 0), (1, 0)), "Q1!A2");
        assert_eq!(expand("[1]Q1!A1", (0, 0), (0, 1)), "[1]Q1!B1");
        // Non-ASCII letters belong to the name, so A1é is not cell A1.
        assert_eq!(
            expand("A1é!A1+Année2020", (0, 0), (1, 0)),
            "A1é!A2+Année2020"
        );
        // Error literals stay as written.
        assert_eq!(
            expand("IFERROR(A1,#REF!)", (0, 0), (1, 0)),
            "IFERROR(A2,#REF!)"
        );
    }

    #[test]
    fn strings_and_structured_references_never_shift() {
        assert_eq!(
            expand("\"it's Q1\"&'Q1'!A1&\"A1\"\"B1\"&A1", (0, 0), (1, 0)),
            "\"it's Q1\"&'Q1'!A2&\"A1\"\"B1\"&A2"
        );
        assert_eq!(
            expand("Sales[[#This Row],[Q1]]*B1", (0, 0), (0, 1)),
            "Sales[[#This Row],[Q1]]*C1"
        );
        assert_eq!(
            expand("SUM(T[Q1'[x']])+A1", (0, 0), (2, 0)),
            "SUM(T[Q1'[x']])+A3"
        );
    }

    #[test]
    fn formulas_without_names_expand_as_calamine_does() {
        for (formula, master, member) in [
            ("A1+$B$2+C$3+$D4", (0, 0), (2, 3)),
            ("SUM(A:A,1:1,A1:B2)", (4, 3), (5, 5)),
            ("IF(A1>0,\"x\",B1)&A$1", (1, 1), (3, 2)),
            ("LOG10(A1)+_xlfn.XLOOKUP(A1,B:B,C:C)", (0, 0), (1, 1)),
        ] {
            let mut ours = String::new();
            let mut theirs = String::new();
            expand_shared_formula_into(formula, master, member, &mut ours).unwrap();
            calamine::expand_shared_formula_into(formula, master, member, &mut theirs).unwrap();
            assert_eq!(ours, theirs, "{formula}");
        }
    }
}
