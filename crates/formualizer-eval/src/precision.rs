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
//! - A scientific section stores as many significant digits as it has
//!   decimal places plus one, however many integer digits it shows:
//!   `##0.0E+0` stores `=1/3` as 0.33 (shown `330.0E-3`).
//! - A fraction section stores the value's whole number plus the fraction it
//!   shows (also when it shows an improper fraction), computed in doubles:
//!   `?/?` stores 1.66 as 1 + 2/3 (shown `5/3`). A fixed denominator rounds to it (`# ?/8`
//!   stores 2.675 as 2 5/8); otherwise the fraction is the last continued
//!   fraction convergent whose denominator has at most as many digits as
//!   the format's (`# ?/?` stores 13/17 as 3/4, not the closer 7/9).
//! - Conditions are not consulted: a positive value takes the first section
//!   and a negative value the second (when there is one), whatever the
//!   conditions say, and keeps its sign; `[>100]0;0.00` stores `=2/3` as 1.
//!   Excel reads a lone conditional section as that section and General.
//! - An empty section passes the value to the next one (`;0.00` stores
//!   positive values with two decimals); with none left, 15 digits.
//! - Zero stays zero, and so does any value that rounds to it.

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
    /// `None` where Excel's stored value is not known.
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
        Some(if !stored.is_finite() {
            value
        } else if stored == 0.0 {
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
                    None => convergent(part, 10u64.pow(places) - 1),
                };
                integer + numerator / denominator
            }
            Section::Unknown => return None,
        })
    }
}

/// The last continued-fraction convergent of `value` (non-negative) whose
/// denominator is at most `max_denominator`, as (numerator, denominator).
fn convergent(value: f64, max_denominator: u64) -> (f64, f64) {
    let limit = max_denominator as f64;
    // h(-2)/k(-2) = 0/1 and h(-1)/k(-1) = 1/0.
    let (mut h0, mut h1, mut k0, mut k1) = (0.0f64, 1.0f64, 1.0f64, 0.0f64);
    let mut best = (0.0, 1.0);
    let mut rest = value;
    for _ in 0..64 {
        let a = rest.floor();
        (h0, h1) = (h1, a * h1 + h0);
        (k0, k1) = (k1, a * k1 + k0);
        if k1 > limit {
            break;
        }
        best = (h1, k1);
        let fraction = rest - a;
        if fraction < 1e-12 {
            break;
        }
        rest = 1.0 / fraction;
    }
    best
}

/// The double nearest `magnitude` to 15 significant digits, times 10^`shift`,
/// rounded half away from zero to the number of significant digits `keep`
/// gives for the decimal exponent of its first digit, then divided by
/// 10^`shift` again. Every step is on decimal digits, as Excel shows them.
fn decimal(magnitude: f64, shift: i32, keep: impl Fn(i32) -> i32) -> f64 {
    let text = format!("{magnitude:.14e}");
    let (mantissa, exponent) = text.split_once('e').expect("scientific notation");
    let mut digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|digit| digit - b'0')
        .collect();
    // `magnitude` is digits[0].digits[1..] x 10^exponent.
    let mut exponent = exponent.parse::<i32>().expect("exponent") + shift;
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
    fn convergents_stop_at_the_denominator_limit() {
        assert_eq!(convergent(13.0 / 17.0, 9), (3.0, 4.0));
        assert_eq!(convergent(0.0625, 9), (0.0, 1.0));
        // In doubles, as Excel computes them: 0.29 expands as 0; 3, 2, 4, 2
        // (2.999999999999787), the fraction of 1.29 as 0; 3, 2, 4, 3.
        assert_eq!(convergent(0.29, 99), (20.0, 69.0));
        assert_eq!(convergent(1.29 - 1.0, 99), (9.0, 31.0));
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
