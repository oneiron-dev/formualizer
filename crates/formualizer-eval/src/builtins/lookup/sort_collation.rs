//! Text order used by SORT and SORTBY.
//!
//! Excel sorts text with its default sort order, not by code point:
//! - Text is compared left to right, character by character, ignoring case
//!   ("A100" sorts after "A1" and before "A11").
//! - Apostrophes (') and hyphens (-) are ignored, except that two strings
//!   that are the same apart from them sort the one without first ("coop"
//!   before "co-op").
//! - Space, punctuation and symbols sort before digits, and digits before
//!   letters. Among the ASCII symbols the order is
//!   `` (space) ! " # $ % & ( ) * , . / : ; ? @ [ \ ] ^ _ ` { | } ~ + < = > ``.
//! - An accented letter sorts with its base letter, after the plain letter
//!   when the strings are otherwise equal ("e" < "é" < "f"); "ß", "æ", "œ"
//!   and "ĳ" sort as "ss", "ae", "oe" and "ij".
//!
//! Strings that differ only in case compare equal, so a stable sort keeps
//! them in source order.

use std::cmp::Ordering;

/// Compares two strings in Excel's sort order (see the module docs).
pub(crate) fn cmp_text_for_sort(a: &str, b: &str) -> Ordering {
    let primary = Elements::new(a)
        .map(|e| e.primary)
        .cmp(Elements::new(b).map(|e| e.primary));
    primary
        .then_with(|| {
            Elements::new(a)
                .map(|e| e.accent)
                .cmp(Elements::new(b).map(|e| e.accent))
        })
        .then_with(|| ignored_marks(a).cmp(ignored_marks(b)))
}

/// The symbols in the order Microsoft documents for Excel's sort. Symbols
/// weigh less than `DIGIT_BASE`, digits less than `LETTER_BASE`.
const SYMBOL_ORDER: &str = " !\"#$%&()*,./:;?@[\\]^_`{|}~+<=>";

const DIGIT_BASE: u32 = 0x0020_0000;
const LETTER_BASE: u32 = 0x0040_0000;

// Accent weights, in the order the accented forms of one letter sort.
const PLAIN: u8 = 0;
const ACUTE: u8 = 1;
const GRAVE: u8 = 2;
const BREVE: u8 = 3;
const CIRCUMFLEX: u8 = 4;
const CARON: u8 = 5;
const RING: u8 = 6;
const DIAERESIS: u8 = 7;
const DOUBLE_ACUTE: u8 = 8;
const TILDE: u8 = 9;
const DOT: u8 = 10;
const CEDILLA: u8 = 11;
const OGONEK: u8 = 12;
const MACRON: u8 = 13;
const STROKE: u8 = 14;
const VARIANT: u8 = 15;

/// Lowercase accented letters and ligatures, sorted by code point:
/// (letter, base letters, accent of the first base letter).
const FOLDS: &[(char, &str, u8)] = &[
    ('ß', "ss", VARIANT),
    ('à', "a", GRAVE),
    ('á', "a", ACUTE),
    ('â', "a", CIRCUMFLEX),
    ('ã', "a", TILDE),
    ('ä', "a", DIAERESIS),
    ('å', "a", RING),
    ('æ', "ae", VARIANT),
    ('ç', "c", CEDILLA),
    ('è', "e", GRAVE),
    ('é', "e", ACUTE),
    ('ê', "e", CIRCUMFLEX),
    ('ë', "e", DIAERESIS),
    ('ì', "i", GRAVE),
    ('í', "i", ACUTE),
    ('î', "i", CIRCUMFLEX),
    ('ï', "i", DIAERESIS),
    ('ð', "d", STROKE),
    ('ñ', "n", TILDE),
    ('ò', "o", GRAVE),
    ('ó', "o", ACUTE),
    ('ô', "o", CIRCUMFLEX),
    ('õ', "o", TILDE),
    ('ö', "o", DIAERESIS),
    ('ø', "o", STROKE),
    ('ù', "u", GRAVE),
    ('ú', "u", ACUTE),
    ('û', "u", CIRCUMFLEX),
    ('ü', "u", DIAERESIS),
    ('ý', "y", ACUTE),
    ('ÿ', "y", DIAERESIS),
    ('ā', "a", MACRON),
    ('ă', "a", BREVE),
    ('ą', "a", OGONEK),
    ('ć', "c", ACUTE),
    ('ĉ', "c", CIRCUMFLEX),
    ('ċ', "c", DOT),
    ('č', "c", CARON),
    ('ď', "d", CARON),
    ('đ', "d", STROKE),
    ('ē', "e", MACRON),
    ('ĕ', "e", BREVE),
    ('ė', "e", DOT),
    ('ę', "e", OGONEK),
    ('ě', "e", CARON),
    ('ĝ', "g", CIRCUMFLEX),
    ('ğ', "g", BREVE),
    ('ġ', "g", DOT),
    ('ģ', "g", CEDILLA),
    ('ĥ', "h", CIRCUMFLEX),
    ('ħ', "h", STROKE),
    ('ĩ', "i", TILDE),
    ('ī', "i", MACRON),
    ('ĭ', "i", BREVE),
    ('į', "i", OGONEK),
    ('ı', "i", VARIANT),
    ('ĳ', "ij", VARIANT),
    ('ĵ', "j", CIRCUMFLEX),
    ('ķ', "k", CEDILLA),
    ('ĺ', "l", ACUTE),
    ('ļ', "l", CEDILLA),
    ('ľ', "l", CARON),
    ('ŀ', "l", DOT),
    ('ł', "l", STROKE),
    ('ń', "n", ACUTE),
    ('ņ', "n", CEDILLA),
    ('ň', "n", CARON),
    ('ō', "o", MACRON),
    ('ŏ', "o", BREVE),
    ('ő', "o", DOUBLE_ACUTE),
    ('œ', "oe", VARIANT),
    ('ŕ', "r", ACUTE),
    ('ŗ', "r", CEDILLA),
    ('ř', "r", CARON),
    ('ś', "s", ACUTE),
    ('ŝ', "s", CIRCUMFLEX),
    ('ş', "s", CEDILLA),
    ('š', "s", CARON),
    ('ţ', "t", CEDILLA),
    ('ť', "t", CARON),
    ('ŧ', "t", STROKE),
    ('ũ', "u", TILDE),
    ('ū', "u", MACRON),
    ('ŭ', "u", BREVE),
    ('ů', "u", RING),
    ('ű', "u", DOUBLE_ACUTE),
    ('ų', "u", OGONEK),
    ('ŵ', "w", CIRCUMFLEX),
    ('ŷ', "y", CIRCUMFLEX),
    ('ź', "z", ACUTE),
    ('ż', "z", DOT),
    ('ž', "z", CARON),
    ('ſ', "s", VARIANT),
    ('ΐ', "ι", DIAERESIS),
    ('ά', "α", ACUTE),
    ('έ', "ε", ACUTE),
    ('ή', "η", ACUTE),
    ('ί', "ι", ACUTE),
    ('ΰ', "υ", DIAERESIS),
    ('ϊ', "ι", DIAERESIS),
    ('ϋ', "υ", DIAERESIS),
    ('ό', "ο", ACUTE),
    ('ύ', "υ", ACUTE),
    ('ώ', "ω", ACUTE),
    ('ё', "е", DIAERESIS),
];

/// Apostrophes and hyphens: no weight, except as the last tie-break.
fn is_ignored(c: char) -> bool {
    matches!(c, '\'' | '-')
}

/// Combining marks carry no weight of their own.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Element {
    primary: u32,
    accent: u8,
}

/// The collation elements of a string: one per character, two for a
/// ligature, none for an ignored character.
struct Elements<'a> {
    chars: std::str::Chars<'a>,
    pending: Option<Element>,
}

impl<'a> Elements<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            chars: text.chars(),
            pending: None,
        }
    }
}

impl Iterator for Elements<'_> {
    type Item = Element;

    fn next(&mut self) -> Option<Element> {
        if let Some(element) = self.pending.take() {
            return Some(element);
        }
        loop {
            let c = self.chars.next()?;
            if is_ignored(c) || is_combining_mark(c) {
                continue;
            }
            if c.is_ascii_digit() {
                return Some(Element {
                    primary: DIGIT_BASE + (c as u32 - '0' as u32),
                    accent: PLAIN,
                });
            }
            if c.is_alphabetic() {
                let lower = if c.is_ascii() {
                    c.to_ascii_lowercase()
                } else {
                    c.to_lowercase().next().unwrap_or(c)
                };
                let letter = |base: char, accent: u8| Element {
                    primary: LETTER_BASE + base as u32,
                    accent,
                };
                if lower.is_ascii() {
                    return Some(letter(lower, PLAIN));
                }
                return Some(match FOLDS.binary_search_by(|(k, _, _)| k.cmp(&lower)) {
                    Ok(i) => {
                        let (_, base, accent) = FOLDS[i];
                        let mut bases = base.chars();
                        let first = bases.next().unwrap_or(lower);
                        if let Some(second) = bases.next() {
                            self.pending = Some(letter(second, PLAIN));
                        }
                        letter(first, accent)
                    }
                    Err(_) => letter(lower, PLAIN),
                });
            }
            if c.is_numeric() {
                // Non-ASCII digits and numeric signs: after 0-9.
                return Some(Element {
                    primary: DIGIT_BASE + 10 + c as u32,
                    accent: PLAIN,
                });
            }
            let symbol = if c.is_whitespace() { ' ' } else { c };
            let rank = match SYMBOL_ORDER.find(symbol) {
                Some(index) => index as u32,
                None => SYMBOL_ORDER.len() as u32 + symbol as u32,
            };
            return Some(Element {
                primary: rank,
                accent: PLAIN,
            });
        }
    }
}

/// Where the ignored apostrophes and hyphens sit, for the last tie-break: a
/// string without them sorts first.
fn ignored_marks(text: &str) -> impl Iterator<Item = (usize, char)> + '_ {
    text.chars().enumerate().filter(|(_, c)| is_ignored(*c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(values: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        out.sort_by(|a, b| cmp_text_for_sort(a, b));
        out
    }

    #[test]
    fn fold_table_is_sorted_for_binary_search() {
        assert!(FOLDS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn apostrophes_and_hyphens_are_ignored() {
        assert_eq!(sorted(&["a-c", "ab"]), ["ab", "a-c"]);
        assert_eq!(sorted(&["a'c", "ab"]), ["ab", "a'c"]);
        assert_eq!(sorted(&["co-op", "coop"]), ["coop", "co-op"]);
        assert_eq!(sorted(&["can't", "cant"]), ["cant", "can't"]);
        assert_eq!(cmp_text_for_sort("co-op", "coop"), Ordering::Greater);
    }

    #[test]
    fn symbols_then_digits_then_letters() {
        assert_eq!(sorted(&["a", "~"]), ["~", "a"]);
        assert_eq!(sorted(&["b", "{x"]), ["{x", "b"]);
        assert_eq!(sorted(&["a", "1", "_"]), ["_", "1", "a"]);
        // The documented symbol order, ending with + < = >.
        let symbols: Vec<String> = SYMBOL_ORDER.chars().rev().map(String::from).collect();
        let symbols: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let expected: Vec<String> = SYMBOL_ORDER.chars().map(String::from).collect();
        assert_eq!(sorted(&symbols), expected);
        // Character by character, not as numbers.
        assert_eq!(sorted(&["A11", "A100", "A1"]), ["A1", "A100", "A11"]);
    }

    #[test]
    fn accented_letters_sort_with_their_base_letter() {
        assert_eq!(sorted(&["f", "é"]), ["é", "f"]);
        assert_eq!(sorted(&["f", "É", "e"]), ["e", "É", "f"]);
        assert_eq!(
            sorted(&["resume", "zebra", "résumé"]),
            ["resume", "résumé", "zebra"]
        );
        assert_eq!(sorted(&["Ötzi", "Oz", "Ober"]), ["Ober", "Ötzi", "Oz"]);
        assert_eq!(
            sorted(&["strasse", "straße", "strata"]),
            ["strasse", "straße", "strata"]
        );
        assert_eq!(sorted(&["aeon", "æon", "afar"]), ["aeon", "æon", "afar"]);
        // A decomposed accent weighs nothing.
        assert_eq!(cmp_text_for_sort("e\u{0301}", "e"), Ordering::Equal);
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(cmp_text_for_sort("ABC", "abc"), Ordering::Equal);
        assert_eq!(cmp_text_for_sort("Straße", "STRASSE"), Ordering::Greater);
        assert_eq!(sorted(&["b", "A", "a", "B"]), ["A", "a", "b", "B"]);
    }
}
