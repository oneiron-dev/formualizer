//! REGEXTEST, REGEXEXTRACT and REGEXREPLACE, as Excel for Windows computes
//! them with PCRE2 (UTF and UCP mode, CR/LF newlines).
//!
//! A pattern PCRE2 rejects is `#VALUE!`. A construct this engine does not
//! match ([`syntax`]), or a search that needs more backtracking than the
//! engine's budget, is `#N/IMPL!`, so the workbook goes to a fallback engine
//! rather than holding a value Excel may not give.

mod replace;
mod syntax;
mod vm;

use super::{super::utils::coerce_num, scalar_text_value};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;
use syntax::PatternError;
use vm::{Exhausted, Matcher, Regex};

fn value_error() -> ExcelError {
    ExcelError::new(ExcelErrorKind::Value)
}

fn not_computed(why: &str) -> ExcelError {
    ExcelError::new(ExcelErrorKind::NImpl).with_message(format!("REGEX: {why}"))
}

fn exhausted(_: Exhausted) -> ExcelError {
    not_computed("the search needs more backtracking than the engine allows")
}

enum MatchError {
    Exhausted,
    SplitCharacter,
}

impl From<Exhausted> for MatchError {
    fn from(_: Exhausted) -> Self {
        MatchError::Exhausted
    }
}

fn match_error(error: MatchError) -> ExcelError {
    match error {
        MatchError::Exhausted => exhausted(Exhausted),
        MatchError::SplitCharacter => value_error(),
    }
}

/// A text argument: numbers in their General format, logicals as TRUE and
/// FALSE, a blank as empty text; an error propagates.
fn text_arg(arg: &ArgumentHandle<'_, '_>) -> Result<String, ExcelError> {
    match scalar_text_value(arg)? {
        LiteralValue::Error(e) => Err(e),
        other => Ok(crate::coercion::to_text_invariant(&other)),
    }
}

/// An optional whole-number argument: omitted or empty is `default`; a
/// number truncates toward zero; `TRUE` is 1, numeric text converts, other
/// text (empty text included) is `#VALUE!`; a blank cell is 0.
fn int_arg(args: &[ArgumentHandle<'_, '_>], index: usize, default: i64) -> Result<i64, ExcelError> {
    let Some(arg) = args.get(index).filter(|arg| !arg.is_omitted()) else {
        return Ok(default);
    };
    match arg.value()?.into_literal() {
        LiteralValue::Error(e) => Err(e),
        other => {
            let n = coerce_num(&other)?.trunc();
            if n.is_finite() && n.abs() < 1e15 {
                Ok(n as i64)
            } else {
                Err(value_error())
            }
        }
    }
}

/// case_sensitivity: 0 (the default) case-sensitive, 1 caseless.
fn caseless_arg(args: &[ArgumentHandle<'_, '_>], index: usize) -> Result<bool, ExcelError> {
    match int_arg(args, index, 0)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(value_error()),
    }
}

fn compile(pattern: &str, caseless: bool) -> Result<Regex, ExcelError> {
    let parsed = syntax::parse(pattern, caseless).and_then(Regex::new);
    parsed.map_err(|error| match error {
        PatternError::Invalid(_) => value_error(),
        PatternError::Unsupported(why) => not_computed(&why),
    })
}

/// Every match from the start of the text, as Excel for Windows finds them:
/// after an empty match the next is tried at the same place and must not be
/// empty there; failing that, the search moves on one UTF-16 unit, two when
/// it stands between CR and LF (Excel: REGEXREPLACE("a"&CHAR(13)&CHAR(10)&"b",
/// "","-") puts no "-" before the "b"). Moving one unit into a character
/// outside the Basic Multilingual Plane splits it, which Excel answers with
/// `#VALUE!` (`MatchError::SplitCharacter`).
fn all_matches(re: &Regex, text: &[char]) -> Result<Vec<Vec<Option<usize>>>, MatchError> {
    let mut matcher = Matcher::new(re, text);
    let mut out = Vec::new();
    let mut from = 0;
    let mut after_empty = false;
    while from <= text.len() {
        match matcher.find(from, after_empty, after_empty)? {
            Some(slots) => {
                let (start, end) = (slots[0].unwrap_or(from), slots[1].unwrap_or(from));
                after_empty = start == end;
                from = end.max(from);
                out.push(slots);
            }
            None if after_empty => {
                if from >= text.len() {
                    break;
                }
                if u32::from(text[from]) > 0xFFFF {
                    return Err(MatchError::SplitCharacter);
                }
                from += if from > 0 && text[from - 1] == '\r' && text[from] == '\n' {
                    2
                } else {
                    1
                };
                after_empty = false;
            }
            None => break,
        }
    }
    Ok(out)
}

fn substring(text: &[char], slots: &[Option<usize>], group: usize) -> Option<String> {
    match (
        slots.get(2 * group).copied().flatten(),
        slots.get(2 * group + 1).copied().flatten(),
    ) {
        (Some(s), Some(e)) if s <= e => Some(text[s..e].iter().collect()),
        (Some(_), Some(_)) => Some(String::new()),
        _ => None,
    }
}

fn row_result<'b>(items: Vec<LiteralValue>, arg: &ArgumentHandle<'_, 'b>) -> CalcValue<'b> {
    if items.len() == 1 {
        return CalcValue::Scalar(items.into_iter().next().unwrap());
    }
    crate::lift::array_result(vec![items], arg.date_system())
}

fn schema() -> &'static [ArgSchema] {
    static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
        std::sync::LazyLock::new(|| vec![ArgSchema::any()]);
    &SCHEMA
}

fn scalar<'b>(result: Result<LiteralValue, ExcelError>) -> Result<CalcValue<'b>, ExcelError> {
    Ok(CalcValue::Scalar(match result {
        Ok(value) => value,
        Err(e) => LiteralValue::Error(e),
    }))
}

#[derive(Debug)]
pub struct RegexTestFn;
/// Returns TRUE when a PCRE2 regular expression matches part of a text.
///
/// `REGEXTEST(text, pattern, [case_sensitivity])`: case_sensitivity 0 (the
/// default) is case-sensitive, 1 caseless.
///
/// # Remarks
/// - Patterns follow PCRE2 in Unicode mode, as Excel for Windows: `\w`, `\d`,
///   `\s` and `\b` are Unicode-aware, `.` matches neither CR nor LF, `$` also
///   matches before a final newline.
/// - An invalid pattern, or a case_sensitivity other than 0 or 1, returns
///   `#VALUE!`; a search PCRE2 would abandon at its match limit is `#VALUE!` in
///   Excel and is not computed here (`#N/IMPL!`).
/// - Recursion, subroutine calls, `\X`, backtracking verbs other than
///   `(*FAIL)`, variable-length lookbehind and duplicate group names are not
///   computed (`#N/IMPL!`).
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Caseless test"
/// formula: '=REGEXTEST("ABC","b",1)'
/// expected: true
/// ```
///
/// ```yaml,docs
/// related:
///   - REGEXEXTRACT
///   - REGEXREPLACE
/// faq:
///   - q: "Does \\w match accented letters?"
///     a: "Yes. Excel compiles patterns in Unicode mode, so \\w matches any letter, number, nonspacing mark or connector punctuation."
/// ```
impl Function for RegexTestFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "REGEXTEST"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        schema()
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        scalar((|| {
            if !(2..=3).contains(&args.len()) {
                return Err(value_error());
            }
            let text: Vec<char> = text_arg(&args[0])?.chars().collect();
            let pattern = text_arg(&args[1])?;
            let caseless = caseless_arg(args, 2)?;
            let re = compile(&pattern, caseless)?;
            let found = Matcher::new(&re, &text)
                .find(0, false, false)
                .map_err(exhausted)?;
            Ok(LiteralValue::Boolean(found.is_some()))
        })())
    }
}

#[derive(Debug)]
pub struct RegexExtractFn;
/// Extracts text that matches a PCRE2 regular expression.
///
/// `REGEXEXTRACT(text, pattern, [return_mode], [case_sensitivity])`:
/// return_mode 0 (the default) returns the first match, 1 every match as a
/// row, 2 the capture groups of the first match as a row.
///
/// # Remarks
/// - No match returns `#N/A`; a capture group that took no part in the match
///   is `#N/A` in mode 2; mode 2 for a pattern without capture groups is
///   `#VALUE!`.
/// - Mode 1 finds matches as PCRE2's global substitution does, so empty
///   matches appear: `REGEXEXTRACT("baaa","a*",1)` is `{"","aaa",""}`.
/// - The pattern rules of REGEXTEST apply.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "First number"
/// formula: '=REGEXEXTRACT("abc123def456","\d+")'
/// expected: "123"
/// ```
///
/// ```yaml,docs
/// related:
///   - REGEXTEST
///   - REGEXREPLACE
///   - TEXTSPLIT
/// faq:
///   - q: "Which way does mode 1 spill?"
///     a: "Across a row, one match per column."
/// ```
impl Function for RegexExtractFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "REGEXEXTRACT"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        schema()
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let result = (|| {
            if !(2..=4).contains(&args.len()) {
                return Err(value_error());
            }
            let text: Vec<char> = text_arg(&args[0])?.chars().collect();
            let pattern = text_arg(&args[1])?;
            let mode = int_arg(args, 2, 0)?;
            let caseless = caseless_arg(args, 3)?;
            if !(0..=2).contains(&mode) {
                return Err(value_error());
            }
            let re = compile(&pattern, caseless)?;
            let na = || LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na));
            match mode {
                0 | 2 => {
                    if mode == 2 && re.groups == 0 {
                        return Err(value_error());
                    }
                    let Some(slots) = Matcher::new(&re, &text)
                        .find(0, false, false)
                        .map_err(exhausted)?
                    else {
                        return Ok(vec![na()]);
                    };
                    if mode == 0 {
                        return Ok(vec![LiteralValue::Text(
                            substring(&text, &slots, 0).unwrap_or_default(),
                        )]);
                    }
                    Ok((1..=re.groups)
                        .map(|group| match substring(&text, &slots, group) {
                            Some(s) => LiteralValue::Text(s),
                            None => na(),
                        })
                        .collect())
                }
                _ => {
                    let matches = all_matches(&re, &text).map_err(match_error)?;
                    if matches.is_empty() {
                        return Ok(vec![na()]);
                    }
                    Ok(matches
                        .iter()
                        .map(|slots| {
                            LiteralValue::Text(substring(&text, slots, 0).unwrap_or_default())
                        })
                        .collect())
                }
            }
        })();
        Ok(match result {
            Ok(items) => row_result(items, &args[0]),
            Err(e) => CalcValue::Scalar(LiteralValue::Error(e)),
        })
    }
}

#[derive(Debug)]
pub struct RegexReplaceFn;
/// Replaces text that matches a PCRE2 regular expression.
///
/// `REGEXREPLACE(text, pattern, replacement, [occurrence], [case_sensitivity])`:
/// occurrence 0 (the default) replaces every match, n the nth, -n the nth
/// from the end; an occurrence past the matches leaves the text as it is.
///
/// # Remarks
/// - The replacement takes `$n`, `${n}`, `$name`, `${name}`, `$&` and `$0`
///   (the match), `$$`, `${n:-default}`, `${n:+set:unset}`, backslash escapes
///   (`\n`, `\t`, `\x41`, `\1`) and the case forcing `\U`, `\L`, `\E`, `\u`,
///   `\l`. A group that did not take part inserts nothing; an unknown group,
///   a lone `$` or `\` is `#VALUE!`.
/// - Matches are found as by REGEXEXTRACT's mode 1.
/// - The pattern rules of REGEXTEST apply.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Swap two words"
/// formula: '=REGEXREPLACE("John Smith","(\w+) (\w+)","$2, $1")'
/// expected: "Smith, John"
/// ```
///
/// ```yaml,docs
/// related:
///   - REGEXTEST
///   - REGEXEXTRACT
///   - SUBSTITUTE
/// faq:
///   - q: "How do I replace only the last match?"
///     a: "Give occurrence -1."
/// ```
impl Function for RegexReplaceFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "REGEXREPLACE"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        schema()
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        scalar((|| {
            if !(3..=5).contains(&args.len()) {
                return Err(value_error());
            }
            let text: Vec<char> = text_arg(&args[0])?.chars().collect();
            let pattern = text_arg(&args[1])?;
            let replacement = text_arg(&args[2])?;
            let occurrence = int_arg(args, 3, 0)?;
            let caseless = caseless_arg(args, 4)?;
            let re = compile(&pattern, caseless)?;
            let matches = all_matches(&re, &text).map_err(match_error)?;
            let chosen: Vec<usize> = match occurrence {
                0 => (0..matches.len()).collect(),
                n if n > 0 && (n as usize) <= matches.len() => vec![n as usize - 1],
                n if n < 0 && (n.unsigned_abs() as usize) <= matches.len() => {
                    vec![matches.len() - n.unsigned_abs() as usize]
                }
                _ => Vec::new(),
            };
            if chosen.is_empty() {
                // Excel reads the replacement only to replace a match: with
                // none, even "$" leaves the text as it is.
                return Ok(LiteralValue::Text(text.iter().collect()));
            }
            let pieces =
                replace::parse(&replacement, re.groups, &re.names).map_err(
                    |error| match error {
                        PatternError::Invalid(_) => value_error(),
                        PatternError::Unsupported(why) => not_computed(&why),
                    },
                )?;
            let mut out = String::new();
            let mut copied = 0;
            for index in chosen {
                let slots = &matches[index];
                let (start, end) = (slots[0].unwrap_or(0), slots[1].unwrap_or(0));
                if start > copied {
                    out.extend(&text[copied..start]);
                }
                out.push_str(&replace::expand(&pieces, &text, slots));
                copied = copied.max(end);
            }
            out.extend(&text[copied.min(text.len())..]);
            Ok(LiteralValue::Text(out))
        })())
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(RegexTestFn));
    crate::function_registry::register_builtin(Arc::new(RegexExtractFn));
    crate::function_registry::register_builtin(Arc::new(RegexReplaceFn));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backtracking steps one search takes; `None` past the budget.
    fn steps(pattern: &str, text: &str) -> Option<u64> {
        let re = compile(pattern, false).unwrap();
        let chars: Vec<char> = text.chars().collect();
        let mut matcher = Matcher::new(&re, &chars);
        matcher.find(0, false, false).ok().map(|_| matcher.steps)
    }

    /// Prints the steps of the searches Excel abandons at PCRE2's match
    /// limit (#VALUE!) and of the nearest ones it completes, to set the
    /// budget (`cargo test -p formualizer-eval regex_budget -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn regex_budget() {
        for n in 14..=20 {
            let text = format!("{}!", "a".repeat(n));
            println!("^(a|a)*$ {n}: {:?}", steps("^(a|a)*$", &text));
        }
        for n in [16, 18, 20, 22] {
            let text = format!("{}b", "a".repeat(n));
            println!("(a+)+$ {n}: {:?}", steps("(a+)+$", &text));
        }
        for n in [1000, 32000] {
            let text = format!("{}b", "a".repeat(n));
            println!("a*b {n}: {:?}", steps("a*b", &text));
        }
        println!(
            "(a|aa)+c 25: {:?}",
            steps("(a|aa)+c", &format!("{}b", "a".repeat(24)))
        );
    }
}
