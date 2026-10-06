//! BAHTTEXT: a number as Thai text with the baht and satang units.

use super::super::utils::{ARG_ANY_ONE, coerce_num};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;

const DIGITS: [&str; 10] = [
    "ศูนย์",
    "หนึ่ง",
    "สอง",
    "สาม",
    "สี่",
    "ห้า",
    "หก",
    "เจ็ด",
    "แปด",
    "เก้า",
];
/// Hundreds to hundred-thousands within a group of six digits.
const PLACES: [&str; 4] = ["ร้อย", "พัน", "หมื่น", "แสน"];
const MILLION: &str = "ล้าน";

/// The amount in satang, as decimal digits without leading zeros ("0" for
/// none). Excel rounds the number to 15 significant digits, an exact tie
/// toward zero (123456789012345.5 is 123456789012345 baht, 999999999999999.875
/// is 10^15), then to two decimals half away from zero (2.675 is 2.68, 0.125
/// is 0.13).
fn satang_digits(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    // The exact decimal expansion, far enough to see a tie at the 16th digit.
    let formatted = format!("{:.60e}", n.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap();
    let exponent: i64 = exponent.parse().unwrap();
    let wide: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    let mut decimal: Vec<u8> = wide[..15].to_vec();
    let mut exponent = exponent;
    let rest = &wide[15..];
    let tie = rest[0] == b'5' && rest[1..].iter().all(|&d| d == b'0');
    if rest[0] >= b'5' && !tie && increment(&mut decimal) {
        decimal.insert(0, b'1');
        decimal.pop();
        exponent += 1;
    }
    // decimal[i] has place value 10^(exponent - i); keep places >= 10^-2.
    let keep = exponent + 3;
    let mut digits: Vec<u8> = if keep <= 0 {
        Vec::new()
    } else {
        decimal[..(keep as usize).min(decimal.len())].to_vec()
    };
    let round_up = keep >= 0 && decimal.get(keep as usize).is_some_and(|&d| d >= b'5');
    while (digits.len() as i64) < keep {
        digits.push(b'0');
    }
    if round_up && increment(&mut digits) {
        digits.insert(0, b'1');
    }
    let text: String = String::from_utf8(digits).unwrap();
    let trimmed = text.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.into()
    }
}

/// Adds one to decimal digits in place; true when it carries out of them.
fn increment(digits: &mut [u8]) -> bool {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return false;
        }
    }
    true
}

/// Thai words for a group of up to six digits (leading zeros allowed).
/// A final 1 is "เอ็ด" after another digit of the group, or when higher
/// groups precede it (`higher`): 11 is สิบเอ็ด, 1,000,001 หนึ่งล้านเอ็ด.
fn group_words(group: &[u8], higher: bool, out: &mut String) {
    let others = group[..group.len() - 1].iter().any(|&d| d != b'0');
    for (i, &d) in group.iter().enumerate() {
        let d = (d - b'0') as usize;
        if d == 0 {
            continue;
        }
        match group.len() - 1 - i {
            0 if d == 1 && (others || higher) => out.push_str("เอ็ด"),
            0 => out.push_str(DIGITS[d]),
            1 => {
                match d {
                    1 => {}
                    2 => out.push_str("ยี่"),
                    _ => out.push_str(DIGITS[d]),
                }
                out.push_str("สิบ");
            }
            place => {
                out.push_str(DIGITS[d]);
                out.push_str(PLACES[place - 2]);
            }
        }
    }
}

/// Thai words for a positive whole number written as decimal digits: groups
/// of six digits joined by ล้าน (10^12 is หนึ่งล้านล้าน).
fn number_words(digits: &[u8], higher: bool, out: &mut String) {
    if digits.len() > 6 {
        let (high, low) = digits.split_at(digits.len() - 6);
        number_words(high, higher, out);
        out.push_str(MILLION);
        group_words(low, true, out);
    } else {
        group_words(digits, higher, out);
    }
}

pub(crate) fn baht_text(n: f64) -> String {
    let satang = satang_digits(n);
    let padded = format!("{satang:0>3}");
    let (baht, cents) = padded.split_at(padded.len() - 2);
    let baht = baht.trim_start_matches('0');
    let cents = cents.as_bytes();
    let mut out = String::new();
    if baht.is_empty() && cents == b"00" {
        out.push_str(DIGITS[0]);
        out.push_str("บาทถ้วน");
        return out;
    }
    if n < 0.0 {
        out.push_str("ลบ");
    }
    if !baht.is_empty() {
        number_words(baht.as_bytes(), false, &mut out);
        out.push_str("บาท");
    }
    if cents == b"00" {
        out.push_str("ถ้วน");
    } else {
        group_words(cents, false, &mut out);
        out.push_str("สตางค์");
    }
    out
}

#[derive(Debug)]
pub struct BahtTextFn;
/// Converts a number to Thai text and adds the suffix "Baht".
///
/// # Remarks
/// - The number is rounded to satang (two decimals) as ROUND rounds it; whole
///   baht end with ถ้วน, otherwise the satang follow with สตางค์, and an amount
///   below one baht has satang only. A negative amount starts with ลบ.
/// - Groups of six digits join with ล้าน, so 10^12 is หนึ่งล้านล้าน; a final 1
///   after other digits is เอ็ด (101 is หนึ่งร้อยเอ็ด).
/// - The argument reads like any number argument (numeric text, logicals, a
///   blank cell as 0); other text returns `#VALUE!`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Whole baht"
/// formula: '=BAHTTEXT(1234)'
/// expected: "หนึ่งพันสองร้อยสามสิบสี่บาทถ้วน"
/// ```
///
/// ```yaml,docs
/// related:
///   - TEXT
///   - DOLLAR
/// faq:
///   - q: "How are fractions of a baht rounded?"
///     a: "To two decimals, half away from zero on the number's 15-digit form: 2.675 is 2 baht 68 satang."
/// ```
impl Function for BahtTextFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "BAHTTEXT"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_ANY_ONE[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        if args.len() != 1 {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }
        let n = match args[0].value()?.into_literal() {
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => match coerce_num(&other) {
                Ok(n) => n,
                Err(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            },
        };
        Ok(CalcValue::Scalar(LiteralValue::Text(baht_text(n))))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(BahtTextFn));
}
