use super::{super::utils::ARG_ANY_ONE, number_format, scalar_text_value};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

fn scalar_like_value(arg: &ArgumentHandle<'_, '_>) -> Result<LiteralValue, ExcelError> {
    Ok(match arg.value()? {
        crate::traits::CalcValue::Scalar(v) | crate::traits::CalcValue::AnnotatedScalar(v, _) => v,
        crate::traits::CalcValue::Range(rv) => rv.get_cell(0, 0),
        crate::traits::CalcValue::Callable(_) => LiteralValue::Error(
            ExcelError::new(ExcelErrorKind::Calc).with_message("LAMBDA value must be invoked"),
        ),
    })
}

/// The text VALUE, NUMBERVALUE and TEXT's format read. An empty slot written in
/// the call is empty text. An empty slot that IF selects stays the 0 Microsoft
/// documents for IF (`VALUE(IF(FALSE,1,))` is 0): the "" that `&` and the other
/// text functions read for it is not a number for VALUE to parse.
fn to_text<'a, 'b>(a: &ArgumentHandle<'a, 'b>) -> Result<String, ExcelError> {
    let v = if a.is_omitted() {
        scalar_text_value(a)?
    } else {
        scalar_like_value(a)?
    };
    Ok(match v {
        LiteralValue::Text(s) => s,
        LiteralValue::Empty => String::new(),
        LiteralValue::Boolean(b) => {
            if b {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        LiteralValue::Int(i) => crate::coercion::int_to_text(i),
        LiteralValue::Number(f) => crate::coercion::number_to_text(f),
        LiteralValue::Error(e) => return Err(e),
        other => other.to_string(),
    })
}

// VALUE(text) - parse number
#[derive(Debug)]
pub struct ValueFn;
/// Converts text that represents a number into a numeric value.
///
/// # Remarks
/// - Parsing uses locale-aware invariant number parsing from the function context.
/// - Text in a number format Excel recognizes in the en-US region converts:
///   `,` group separators, a leading `$`, parentheses for a negative number and
///   a trailing `%` (`"$1,000"` -> 1000, `"($5)"` -> -5, `"90%"` -> 0.9).
/// - Non-numeric text returns `#VALUE!`.
/// - Booleans and numbers are first coerced to text, then parsed.
/// - Errors are propagated unchanged.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Parse decimal text"
/// formula: '=VALUE("12.5")'
/// expected: 12.5
/// ```
///
/// ```yaml,sandbox
/// title: "Currency text"
/// formula: '=VALUE("$1,000")'
/// expected: 1000
/// ```
///
/// ```yaml,sandbox
/// title: "Invalid numeric text"
/// formula: '=VALUE("abc")'
/// expected: "#VALUE!"
/// ```
///
/// ```yaml,docs
/// related:
///   - TEXT
///   - N
///   - ISNUMBER
/// faq:
///   - q: "Does VALUE coerce arbitrary text like TRUE/FALSE?"
///     a: "VALUE parses numeric text only; non-numeric strings return #VALUE!."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: VALUE
/// Type: ValueFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: VALUE(arg1: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ValueFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "VALUE"
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
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let s = to_text(&args[0])?;
        // Numeric text, then date/time text (VALUE("1/2/2023") is a serial).
        let Ok(n) = crate::coercion::to_arithmetic_number_with_locale(
            &LiteralValue::Text(s),
            &ctx.locale(),
            ctx.date_system(),
            Some(args[0].current_year()),
        ) else {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        };
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(n)))
    }
}

/// Converts locale-delimited text to a number.
///
/// Parses text using explicit decimal and group separators, independent of the
/// workbook's invariant locale.
///
/// # Remarks
/// - The decimal separator defaults to `.`.
/// - The group separator defaults to `,`.
/// - Only the first character of each separator is used.
/// - Spaces are ignored, even in the middle (`" 3 000 "` is 3000), and empty
///   text is 0.
/// - Group separators before the decimal separator are ignored; one after it,
///   or a second decimal separator, is `#VALUE!`.
/// - Percent suffixes are supported and scale the result by 100 per suffix.
///
/// ```yaml,sandbox
/// title: "Parse with explicit separators"
/// formula: '=NUMBERVALUE("1.234,56",",",".")'
/// expected: 1234.56
/// ```
///
/// ```yaml,sandbox
/// title: "Parse percent suffix"
/// formula: '=NUMBERVALUE("12.5%")'
/// expected: 0.125
/// ```
///
/// ```yaml,docs
/// related:
///   - VALUE
///   - TEXT
///   - DOLLAR
/// faq:
///   - q: "Does NUMBERVALUE use the global locale?"
///     a: "No. Decimal and group separators are passed explicitly as arguments."
/// ```
#[derive(Debug)]
pub struct NumberValueFn;

/// [formualizer-docgen:schema:start]
/// Name: NUMBERVALUE
/// Type: NumberValueFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: NUMBERVALUE(arg1...: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for NumberValueFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "NUMBERVALUE"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_ANY_ONE[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.is_empty() || args.len() > 3 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        let text = to_text(&args[0])?;
        let decimal_sep = if args.len() >= 2 {
            to_text(&args[1])?
        } else {
            ".".to_string()
        };
        let group_sep = if args.len() >= 3 {
            to_text(&args[2])?
        } else {
            ",".to_string()
        };

        // Microsoft's NUMBERVALUE remarks: only the first character of a
        // separator is used, spaces are ignored anywhere, "" is 0, group
        // separators before the decimal separator are ignored and one after it
        // is invalid, and each trailing percent sign divides by 100.
        let value_error = || {
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )))
        };
        let Some(decimal) = decimal_sep.chars().next() else {
            return value_error();
        };
        let group = group_sep.chars().next();
        if group == Some(decimal) {
            return value_error();
        }
        // A space that is itself a separator stays, so its position is checked.
        let mut body: String = text
            .trim()
            .chars()
            .filter(|&c| c != ' ' || c == decimal || Some(c) == group)
            .collect();
        if body.is_empty() {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0)));
        }
        // Each trailing percent sign divides by 100; spaces around them are
        // ignored like any other space, even when space is a separator
        // ("9% %" with group " " is 0.0009).
        let mut pct_count = 0;
        let mut number_len = body.len();
        while let Some(rest) = body[..number_len].strip_suffix('%') {
            number_len = rest.trim_end_matches(' ').len();
            pct_count += 1;
        }
        body.truncate(number_len);
        let (integer, fraction) = match body.split_once(decimal) {
            Some((integer, fraction)) => (integer, Some(fraction)),
            None => (body.as_str(), None),
        };
        if fraction.is_some_and(|f| f.contains(decimal) || group.is_some_and(|g| f.contains(g))) {
            return value_error();
        }
        let mut plain: String = integer.chars().filter(|&c| Some(c) != group).collect();
        // A "." that is not the decimal separator is not part of a number.
        if plain.contains('.') || fraction.is_some_and(|f| f.contains('.')) {
            return value_error();
        }
        if let Some(fraction) = fraction {
            plain.push('.');
            plain.push_str(fraction);
        }
        // A plain decimal number, and only a finite one: Rust's spellings
        // "inf" and "NaN" and text beyond the double range are not numbers.
        let Some(mut n) = crate::locale::parse_finite_number(&plain) else {
            return value_error();
        };
        for _ in 0..pct_count {
            n /= 100.0;
        }

        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(n)))
    }
}

// TEXT(value, format_text) - Excel number-format codes
#[derive(Debug)]
pub struct TextFn;
/// Formats a value as text using a format pattern.
///
/// Formats with Excel's number-format language: sections, conditions, digit
/// placeholders, grouping/scaling, percent, scientific, fractions, text `@`,
/// and date/time tokens (see `number_format`).
///
/// # Remarks
/// - Requires exactly two arguments: value and format text.
/// - Numeric and date/time text is converted to its number before formatting.
///   Other text (and TRUE/FALSE) only passes through the format's text section;
///   without one it is returned unchanged (e.g. `=TEXT("abc","00")` -> `"abc"`).
/// - Error inputs are propagated unchanged.
/// - Numbers show at most 15 significant digits and round half away from zero.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Fixed decimal formatting"
/// formula: '=TEXT(12.3, "0.00")'
/// expected: "12.30"
/// ```
///
/// ```yaml,sandbox
/// title: "Percent formatting"
/// formula: '=TEXT(0.256, "0%")'
/// expected: "26%"
/// ```
///
/// ```yaml,docs
/// related:
///   - VALUE
///   - FIXED
///   - DOLLAR
/// faq:
///   - q: "How complete is format_text support?"
///     a: "Excel format codes are supported, including sections, conditions, fractions, scientific and date/time tokens; locale-specific codes render in en-US."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: TEXT
/// Type: TextFn
/// Min args: 2
/// Max args: 1
/// Variadic: false
/// Signature: TEXT(arg1: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TextFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "TEXT"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_ANY_ONE[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.len() != 2 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }
        let val = scalar_like_value(&args[0])?;
        if let LiteralValue::Error(e) = val {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
        }
        let fmt = to_text(&args[1])?;
        if fmt.is_empty() {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Text(
                String::new(),
            )));
        }
        let num = match val {
            LiteralValue::Number(f) => f,
            LiteralValue::Int(i) => i as f64,
            // Numeric and date/time text is formatted as its number; other
            // text only passes through the format's text section.
            LiteralValue::Text(t) => match crate::coercion::to_arithmetic_number_with_locale(
                &LiteralValue::Text(t.clone()),
                &ctx.locale(),
                ctx.date_system(),
                Some(args[0].current_year()),
            ) {
                Ok(n) => n,
                Err(_) => {
                    return Ok(crate::traits::CalcValue::Scalar(
                        match number_format::format_text(&t, &fmt) {
                            Ok(text) => LiteralValue::Text(text),
                            Err(error) => LiteralValue::Error(error),
                        },
                    ));
                }
            },
            // Logical values are not numbers to TEXT.
            LiteralValue::Boolean(b) => {
                let text = if b { "TRUE" } else { "FALSE" };
                return Ok(crate::traits::CalcValue::Scalar(
                    match number_format::format_text(text, &fmt) {
                        Ok(text) => LiteralValue::Text(text),
                        Err(error) => LiteralValue::Error(error),
                    },
                ));
            }
            LiteralValue::Empty => 0.0,
            LiteralValue::Error(e) => {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
            }
            other => match other.as_serial_number_for(ctx.date_system()) {
                Some(n) => n,
                None => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                        ExcelError::new_value(),
                    )));
                }
            },
        };
        Ok(crate::traits::CalcValue::Scalar(
            match number_format::format_number(num, &fmt, ctx.date_system()) {
                Ok(text) => LiteralValue::Text(text),
                Err(error) => LiteralValue::Error(error),
            },
        ))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(ValueFn));
    crate::function_registry::register_builtin(Arc::new(NumberValueFn));
    crate::function_registry::register_builtin(Arc::new(TextFn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_common::{ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::{ASTNode, ASTNodeType};
    fn lit(v: LiteralValue) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(v), None)
    }
    #[test]
    fn value_basic() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(ValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "VALUE").unwrap();
        let s = lit(LiteralValue::Text("12.5".into()));
        let out = f
            .dispatch(
                &[ArgumentHandle::new(&s, &ctx)],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert_eq!(out, LiteralValue::Number(12.5));
    }

    #[test]
    fn value_percent_text() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(ValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "VALUE").unwrap();
        let s = lit(LiteralValue::Text("90%".into()));
        let out = f
            .dispatch(
                &[ArgumentHandle::new(&s, &ctx)],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert_eq!(out, LiteralValue::Number(0.9));
    }

    #[test]
    fn value_and_numbervalue_reject_non_finite_spellings() {
        // "NaN", "inf" and "infinity" are Rust float spellings, not Excel
        // numeric text: VALUE and NUMBERVALUE give #VALUE! rather than a NaN
        // or an infinity that no Excel cell can hold.
        let wb = TestWorkbook::new()
            .with_function(std::sync::Arc::new(ValueFn))
            .with_function(std::sync::Arc::new(NumberValueFn));
        let ctx = wb.interpreter();
        for name in ["VALUE", "NUMBERVALUE"] {
            let f = ctx.context.get_function("", name).unwrap();
            for text in [
                "NaN", "nan", "-NaN", "inf", "-Inf", "Infinity", "NaN%", "1e400",
            ] {
                let s = lit(LiteralValue::Text(text.into()));
                let out = f
                    .dispatch(
                        &[ArgumentHandle::new(&s, &ctx)],
                        &ctx.function_context(None),
                    )
                    .unwrap()
                    .into_literal();
                match out {
                    LiteralValue::Error(e) => {
                        assert_eq!(e.kind, ExcelErrorKind::Value, "{name}({text:?})")
                    }
                    other => panic!("{name}({text:?}): expected #VALUE!, got {other:?}"),
                }
            }
            let s = lit(LiteralValue::Text("1e3".into()));
            let out = f
                .dispatch(
                    &[ArgumentHandle::new(&s, &ctx)],
                    &ctx.function_context(None),
                )
                .unwrap()
                .into_literal();
            assert_eq!(out, LiteralValue::Number(1000.0), "{name}(\"1e3\")");
        }
    }

    #[test]
    fn value_currency_text() {
        // Microsoft's VALUE example: =VALUE("$1,000") is 1000.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(ValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "VALUE").unwrap();
        for (text, expected) in [("$1,000", 1000.0), ("($1,000)", -1000.0), ("-$2.5", -2.5)] {
            let s = lit(LiteralValue::Text(text.into()));
            let out = f
                .dispatch(
                    &[ArgumentHandle::new(&s, &ctx)],
                    &ctx.function_context(None),
                )
                .unwrap()
                .into_literal();
            assert_eq!(out, LiteralValue::Number(expected), "VALUE({text:?})");
        }
    }

    #[test]
    fn numbervalue_supports_explicit_separators_and_percent() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(NumberValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "NUMBERVALUE").unwrap();
        let text = lit(LiteralValue::Text(" 1.234,50%% ".into()));
        let dec = lit(LiteralValue::Text(",".into()));
        let grp = lit(LiteralValue::Text(".".into()));
        let out = f
            .dispatch(
                &[
                    ArgumentHandle::new(&text, &ctx),
                    ArgumentHandle::new(&dec, &ctx),
                    ArgumentHandle::new(&grp, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert_eq!(out, LiteralValue::Number(0.12345));
    }

    #[test]
    fn numbervalue_rejects_bad_separators_and_multiple_decimals() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(NumberValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "NUMBERVALUE").unwrap();
        let text = lit(LiteralValue::Text("1.2.3".into()));
        let out = f
            .dispatch(
                &[ArgumentHandle::new(&text, &ctx)],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert!(matches!(out, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value));

        let sep = lit(LiteralValue::Text(".".into()));
        let out = f
            .dispatch(
                &[
                    ArgumentHandle::new(&lit(LiteralValue::Text("1.2".into())), &ctx),
                    ArgumentHandle::new(&sep, &ctx),
                    ArgumentHandle::new(&sep, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert!(matches!(out, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value));
    }

    #[test]
    fn numbervalue_follows_the_documented_separator_rules() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(NumberValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "NUMBERVALUE").unwrap();
        let run = |args: &[&str]| {
            let nodes: Vec<ASTNode> = args
                .iter()
                .map(|a| lit(LiteralValue::Text((*a).into())))
                .collect();
            let handles: Vec<ArgumentHandle> =
                nodes.iter().map(|n| ArgumentHandle::new(n, &ctx)).collect();
            f.dispatch(&handles, &ctx.function_context(None))
                .unwrap()
                .into_literal()
        };
        // Microsoft's remarks and examples.
        for (args, expected) in [
            (&[" 3 000 "][..], 3000.0),
            (&[""][..], 0.0),
            (&["9%%"][..], 0.0009),
            (&["2.500,27", ",", "."][..], 2500.27),
            (&["1,2.5"][..], 12.5),
            (&["1 234,5", ", ", "x"][..], 1234.5),
            (&["-1.5e3"][..], -1500.0),
            (&["1\u{a0}234.5", ".", "\u{a0}"][..], 1234.5),
        ] {
            assert_eq!(run(args), LiteralValue::Number(expected), "{args:?}");
        }
        for args in [
            &["1.5,3"][..],
            &["1.5", ",", " "][..],
            &["inf"][..],
            &["NaN"][..],
            &["1e400"][..],
            &["%"][..],
            &["1,5", ",x", ",y"][..],
            // A space-like group separator after the decimal separator.
            &["1.2\u{a0}3", ".", "\u{a0}"][..],
            &["1\t2"][..],
        ] {
            assert!(
                matches!(run(args), LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value),
                "{args:?}"
            );
        }
    }

    #[test]
    fn numbervalue_ignores_spaces_between_trailing_percent_signs() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(NumberValueFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "NUMBERVALUE").unwrap();
        let run = |args: &[&str]| {
            let nodes: Vec<ASTNode> = args
                .iter()
                .map(|a| lit(LiteralValue::Text((*a).into())))
                .collect();
            let handles: Vec<ArgumentHandle> =
                nodes.iter().map(|n| ArgumentHandle::new(n, &ctx)).collect();
            f.dispatch(&handles, &ctx.function_context(None))
                .unwrap()
                .into_literal()
        };
        // Spaces are ignored and every trailing % divides by 100, also when
        // the group separator is a space.
        for (args, expected) in [
            (&["9% %"][..], 0.0009),
            (&["9% %", ".", " "][..], 0.0009),
            (&["9 % %", ".", " "][..], 0.0009),
            (&["1 234 % ", ".", " "][..], 12.34),
            (&["9 %", ",", " "][..], 0.09),
        ] {
            assert_eq!(run(args), LiteralValue::Number(expected), "{args:?}");
        }
        for args in [&["% %", ".", " "][..], &["9% 5", ".", " "][..]] {
            assert!(
                matches!(run(args), LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value),
                "{args:?}"
            );
        }
    }

    #[test]
    fn text_basic_number() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(TextFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TEXT").unwrap();
        let n = lit(LiteralValue::Number(12.34));
        let fmt = lit(LiteralValue::Text("0.00".into()));
        let out = f
            .dispatch(
                &[
                    ArgumentHandle::new(&n, &ctx),
                    ArgumentHandle::new(&fmt, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert_eq!(out, LiteralValue::Text("12.34".into()));
    }

    #[test]
    fn text_clearly_non_numeric_text_passes_through() {
        // Excel returns the text argument unchanged when it is *clearly* not a
        // number (no digits): =TEXT("abc","00") -> "abc" (not #VALUE!). A numeric
        // format does not coerce arbitrary letters.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(TextFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TEXT").unwrap();
        for input in ["abc", "N/A", "hello world"] {
            let v = lit(LiteralValue::Text(input.into()));
            let fmt = lit(LiteralValue::Text("00".into()));
            let out = f
                .dispatch(
                    &[
                        ArgumentHandle::new(&v, &ctx),
                        ArgumentHandle::new(&fmt, &ctx),
                    ],
                    &ctx.function_context(None),
                )
                .unwrap()
                .into_literal();
            assert_eq!(
                out,
                LiteralValue::Text(input.into()),
                "TEXT({input:?},\"00\")"
            );
        }
    }

    #[test]
    fn text_coerces_number_and_date_text_and_passes_other_text() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(TextFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TEXT").unwrap();
        for (input, format, expected) in [
            ("12.5", "0.00", "12.50"),
            ("2024-03-05", "dddd", "Tuesday"),
            // `aaa`/`aaaa` are weekday codes, matched without case or accents.
            ("2024-03-05", "aaaa", "Tuesday"),
            ("45356", "\u{c5}\u{c5}\u{c5} d", "Tue 5"),
            ("abc", "aaaa", "abc"),
            // `General` beside a weekday code shows the value; a weekday alone
            // is the serial's own day, even a moment before midnight.
            ("45356", "aaaa General", "Tuesday 45356"),
            ("45356.999999", "aaaa", "Tuesday"),
            ("abc", "00", "abc"),
            ("1.234,56", "00", "1.234,56"),
            ("abc", "\"<\"@\">\"", "<abc>"),
        ] {
            let v = lit(LiteralValue::Text(input.into()));
            let fmt = lit(LiteralValue::Text(format.into()));
            let out = f
                .dispatch(
                    &[
                        ArgumentHandle::new(&v, &ctx),
                        ArgumentHandle::new(&fmt, &ctx),
                    ],
                    &ctx.function_context(None),
                )
                .unwrap()
                .into_literal();
            assert_eq!(
                out,
                LiteralValue::Text(expected.into()),
                "TEXT({input:?},{format:?})"
            );
        }
    }

    #[test]
    fn text_elapsed_span_too_long_to_count_is_value_error() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(TextFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TEXT").unwrap();
        let v = lit(LiteralValue::Number(1e20));
        let fmt = lit(LiteralValue::Text("[h]".into()));
        let out = f
            .dispatch(
                &[
                    ArgumentHandle::new(&v, &ctx),
                    ArgumentHandle::new(&fmt, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();
        assert!(
            matches!(&out, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value),
            "TEXT(1E20,\"[h]\") = {out:?}"
        );
    }

    #[test]
    fn text_rejects_format_codes_excel_cannot_read() {
        // Hungarian date codes in an en-US Excel: `n` is not a format code, so
        // TEXT is #VALUE! for numbers, text and logicals alike; `ó` and `p`
        // are plain literals.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(TextFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TEXT").unwrap();
        let eval = |value: LiteralValue, format: &str| {
            let v = lit(value);
            let fmt = lit(LiteralValue::Text(format.into()));
            f.dispatch(
                &[
                    ArgumentHandle::new(&v, &ctx),
                    ArgumentHandle::new(&fmt, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal()
        };
        for (value, format) in [
            (LiteralValue::Number(41645.0), "eeee.hh.nn "),
            (LiteralValue::Text("abc".into()), "nn"),
            (LiteralValue::Boolean(true), "n@"),
            (LiteralValue::Number(1.0), "0;0;0;@;0"),
        ] {
            let out = eval(value.clone(), format);
            assert!(
                matches!(&out, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value),
                "TEXT({value:?},{format:?}) = {out:?}"
            );
        }
        assert_eq!(
            eval(LiteralValue::Number(0.400544), "\u{f3}\u{f3}:pp:mm"),
            LiteralValue::Text("\u{f3}\u{f3}:pp:01".into())
        );
        assert_eq!(
            eval(LiteralValue::Text("abc".into()), "\"n\"@"),
            LiteralValue::Text("nabc".into())
        );
        // Code letters ignore case and accents, `b` is the Buddhist year next
        // to digits, and `!` shows the next letter as written.
        for (value, format, text) in [
            (45356.0, "EEEE", "2024"),
            (45356.0, "\u{e9}\u{e9}\u{e9}\u{e9}", "2024"),
            (45356.0, "dd/mm/bbbb", "05/03/2567"),
            (5.0, "0!n", "5n"),
        ] {
            assert_eq!(
                eval(LiteralValue::Number(value), format),
                LiteralValue::Text(text.into()),
                "TEXT({value},{format:?})"
            );
        }
        let out = eval(LiteralValue::Number(5.0), "0.0b");
        assert!(
            matches!(&out, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value),
            "{out:?}"
        );
    }
}
