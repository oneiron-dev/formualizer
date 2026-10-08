//! Excel's "Set precision as displayed" (`<calcPr fullPrecision="0"/>`): a
//! formula's numeric result is stored as its cell's number format shows it,
//! and every formula that reads the cell reads that stored value. Constants
//! keep the value the file holds.
//!
//! What Excel for Windows 16.0.20430 stores, by the section of the cell's
//! format that shows the value (ops/excel-precision-probe-20261008.md):
//! - General, a text section (`@`) and a date or time section store the
//!   value to 15 significant digits: `=1/3` is 0.333333333333333, `=0.1+0.2`
//!   0.3, whatever the column width.
//! - A number section stores the decimal it shows, after its percent signs
//!   and its scaling commas: `0.00%` stores `=1/3` as 0.3333 and `#,##0,"K"`
//!   stores 1234.5678 as 1000. Digits are read off the value to 15
//!   significant digits and rounded half away from zero (2.675 is 2.68 under
//!   `0.00`); the result is the double nearest that decimal.
//! - The 15 significant digits are the nearest, and a value exactly halfway
//!   between two (a 16th digit 5 and nothing after it) goes toward zero:
//!   100000000000001.5 is 100000000000001 and 32771/32768
//!   (1.000091552734375) is 1.00009155273437, under General and every number
//!   section.
//! - A scientific section stores as many significant digits as it has
//!   decimal places plus one, and two more for each percent sign, however
//!   many integer digits it shows: `##0.0E+0` stores `=1/3` as 0.33 (shown
//!   `330.0E-3`) and `0.0E+0%` stores it as 0.3333 (shown `3.3E-1%`).
//! - A fraction section stores the value's whole number plus the fraction it
//!   shows (also when it shows an improper fraction), computed in doubles:
//!   `?/?` stores 1.66 as 1 + 2/3 (shown `5/3`). A fixed denominator up to
//!   32768 rounds to it (`# ?/8` stores 2.675 as 2 5/8; `# ?/05` stores
//!   fifths, `# ?/0` takes one digit); a larger one is stored unrounded or by
//!   another rule and is refused. Otherwise the fraction is the last continued
//!   fraction convergent whose denominator has at most as many digits as
//!   the format's (`# ?/?` stores 13/17 as 3/4, not the closer 7/9).
//! - Conditions are not consulted: a positive value takes the first section
//!   and a negative value the second (when there is one), whatever the
//!   conditions say, and keeps its sign; `[>100]0;0.00` stores `=2/3` as 1.
//!   Excel reads a lone conditional section as that section and General.
//! - The last of two or three sections formats text when it holds `@`, so
//!   `0;@` and `[>1]0;@` store every number as `0` does (-2/3 as -1).
//! - An empty section passes the value to the next one (`;0.00` stores
//!   positive values with two decimals); with none left, 15 digits.
//! - Zero stays zero, and so does any value that rounds to it or below the
//!   smallest normal double (General stores 2^-1022 as 0, `0E+0` stores
//!   2.3E-308 as 0). For a value that rounds past the largest double
//!   (2^1023*(2-2^-52) under General, 1.79E+308 under `0.0E+0`) Excel saves
//!   #NUM!, yet compares the cell as a number (=A1=0 is FALSE, =A1*1 #NUM!).
//! - Excel will not open a workbook with a format such as `0.0@`, `@;0`,
//!   `0;0;0;0`, `# ?/100000`, `0.0 ?/?`, `E+0`, `[Foo]0`, `[Red][Blue]0`,
//!   three conditions, a bare `g` or more than 123 characters; such formats
//!   are refused.

use crate::builtins::text::number_format;

/// How "Set precision as displayed" stores a number under one format code.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayedFormat {
    /// The positive, negative and zero sections (at least one).
    sections: Vec<Section>,
}

/// What one section of a number format keeps of a value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Section {
    /// No codes: the value goes on to the next section.
    Empty,
    /// 15 significant digits.
    Significant,
    /// The digits down to `decimals` places of the value times
    /// 10^`shift` (`%` adds 2, a scaling comma takes 3).
    Fixed { decimals: i32, shift: i32 },
    /// `digits` significant digits.
    Scientific { digits: i32 },
    /// The whole number plus a fraction of the rest: over `denominator`
    /// when it is fixed, else the last convergent whose denominator has at
    /// most `places` digits. A format showing an improper fraction stores
    /// the same.
    Fraction {
        denominator: Option<u64>,
        places: u32,
    },
    /// A section whose stored value Excel's behaviour was not measured for.
    Unknown,
}

impl DisplayedFormat {
    /// The General format.
    pub fn general() -> Self {
        Self {
            sections: vec![Section::Significant],
        }
    }

    /// Read a format code. `Err` says why Excel's stored value cannot be
    /// reproduced for it.
    pub fn parse(code: &str) -> Result<Self, String> {
        let sections = number_format::displayed_sections(code)?;
        Ok(Self { sections })
    }

    /// The value a cell formatted with this format stores for `value`, or
    /// `None` where Excel's stored value is not known. An infinite value is
    /// a decimal past the largest double (see the module notes).
    pub fn round(&self, value: f64) -> Option<f64> {
        if value == 0.0 {
            return Some(0.0);
        }
        if !value.is_finite() {
            return Some(value);
        }
        let first = usize::from(value < 0.0 && self.sections.len() > 1);
        let section = self.sections[first..]
            .iter()
            .find(|section| **section != Section::Empty)
            .unwrap_or(&Section::Significant);
        let stored = section.round(value.abs())?;
        Some(if stored < f64::MIN_POSITIVE {
            0.0
        } else if value < 0.0 {
            -stored
        } else {
            stored
        })
    }
}

impl Section {
    /// The value stored for `magnitude` (positive and finite).
    fn round(&self, magnitude: f64) -> Option<f64> {
        Some(match *self {
            Section::Empty | Section::Significant => decimal(magnitude, 0, |_| 15),
            Section::Fixed { decimals, shift } => {
                decimal(magnitude, shift, |exponent| exponent + 1 + decimals)
            }
            Section::Scientific { digits } => decimal(magnitude, 0, |_| digits),
            Section::Fraction {
                denominator,
                places,
            } => {
                let integer = magnitude.trunc();
                let part = magnitude - integer;
                let (numerator, denominator) = match denominator {
                    Some(denominator) => ((part * denominator as f64).round(), denominator as f64),
                    None => {
                        let (numerator, denominator) =
                            number_format::convergent(part, 10u64.pow(places) - 1);
                        (numerator as f64, denominator as f64)
                    }
                };
                integer + numerator / denominator
            }
            Section::Unknown => return None,
        })
    }
}

/// `magnitude` to 15 significant digits ([`fifteen_digits`]), times
/// 10^`shift`, rounded half away from zero to the number of significant
/// digits `keep` gives for the decimal exponent of its first digit, then
/// divided by 10^`shift` again: the double nearest that decimal (infinite
/// past the largest). Every step is on decimal digits, as Excel shows them.
fn decimal(magnitude: f64, shift: i32, keep: impl Fn(i32) -> i32) -> f64 {
    let (mut digits, exponent) = fifteen_digits(magnitude);
    // `magnitude` is digits[0].digits[1..] x 10^exponent.
    let mut exponent = exponent + shift;
    let kept = keep(exponent);
    if kept < 0 || (kept == 0 && digits[0] < 5) {
        return 0.0;
    }
    if kept == 0 {
        digits = vec![1];
        exponent += 1;
    } else if (kept as usize) < digits.len() {
        let up = digits[kept as usize] >= 5;
        digits.truncate(kept as usize);
        if up {
            match digits.iter().rposition(|digit| *digit != 9) {
                Some(at) => {
                    digits[at] += 1;
                    digits.truncate(at + 1);
                }
                None => {
                    digits = vec![1];
                    exponent += 1;
                }
            }
        }
    }
    let integer: String = digits
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    let scale = exponent - shift - (digits.len() as i32 - 1);
    format!("{integer}e{scale}")
        .parse()
        .expect("decimal digits parse")
}

/// The 15 significant digits of `magnitude` (positive and finite) and the
/// decimal exponent of the first: the nearest, and toward zero when
/// `magnitude` is exactly halfway between two.
fn fifteen_digits(magnitude: f64) -> (Vec<u8>, i32) {
    // Sixteen digits are exact when `magnitude` is a tie: the 16th is a 5
    // with nothing after it.
    let (sixteen, exponent) = scientific_digits(&format!("{magnitude:.15e}"));
    if sixteen[15] == 5 && is_exactly(&sixteen, exponent - 15, magnitude) {
        return (sixteen[..15].to_vec(), exponent);
    }
    scientific_digits(&format!("{magnitude:.14e}"))
}

/// The digits and exponent of Rust's `{:e}` notation.
fn scientific_digits(text: &str) -> (Vec<u8>, i32) {
    let (mantissa, exponent) = text.split_once('e').expect("scientific notation");
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|digit| digit - b'0')
        .collect();
    (digits, exponent.parse().expect("exponent"))
}

/// Whether `magnitude` is exactly the integer `digits` (odd: it ends in 5)
/// times 10^`scale`. Such a product is a double only when its odd part,
/// `digits` x 5^`scale`, or `digits` / 5^-`scale`, is an integer below 2^53.
fn is_exactly(digits: &[u8], scale: i32, magnitude: f64) -> bool {
    const LIMIT: u128 = 1 << 53;
    let integer = digits
        .iter()
        .fold(0u128, |integer, digit| integer * 10 + u128::from(*digit));
    if scale >= 0 {
        let odd = integer * 5u128.pow(scale.min(2) as u32);
        return scale <= 1 && odd < LIMIT && magnitude == (odd as f64) * 2f64.powi(scale);
    }
    let fives = 5u128.checked_pow(scale.unsigned_abs());
    match fives {
        Some(fives) if integer % fives == 0 && integer / fives < LIMIT => {
            magnitude == ((integer / fives) as f64) / 2f64.powi(-scale)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(code: &str, value: f64) -> f64 {
        DisplayedFormat::parse(code)
            .unwrap_or_else(|reason| panic!("{code:?}: {reason}"))
            .round(value)
            .unwrap_or_else(|| panic!("{code:?} {value}: unknown"))
    }

    #[test]
    fn decimal_rounding_keeps_the_shown_digits() {
        assert_eq!(decimal(2.675, 0, |e| e + 1 + 2), 2.68);
        assert_eq!(decimal(0.125, 0, |e| e + 1 + 2), 0.13);
        assert_eq!(decimal(99.995, 0, |e| e + 1 + 2), 100.0);
        assert_eq!(decimal(0.4, 0, |e| e + 1), 0.0);
        assert_eq!(decimal(0.6, 0, |e| e + 1), 1.0);
        assert_eq!(decimal(0.06, 0, |e| e + 1), 0.0);
        assert_eq!(decimal(9.96, 0, |e| e + 1 + 1), 10.0);
        assert_eq!(decimal(1.0 / 3.0, 0, |_| 15), 0.333333333333333);
        assert_eq!(decimal(1.0 / 3.0, 2, |e| e + 1), 0.33);
        assert_eq!(decimal(1234.5678, -3, |e| e + 1), 1000.0);
    }

    #[test]
    fn signs_pick_the_section_and_conditions_are_ignored() {
        assert_eq!(stored("[>100]0;0.00", 2.0 / 3.0), 1.0);
        assert_eq!(stored("[>100]0;0.00", -2.5), -2.5);
        assert_eq!(stored("0.00;(0.0)", -1234.5678), -1234.6);
        assert_eq!(stored("0.00;(0.0)", -0.0001), 0.0);
        assert!(stored("0.00;(0.0)", -0.0001).is_sign_positive());
        assert_eq!(stored(";0.00", 1.0 / 3.0), 0.33);
        assert_eq!(stored("0.00;;0", -2.5), -3.0);
        assert_eq!(stored("[>=0]0.0", -1234.5678), -1234.5678);
    }
}
