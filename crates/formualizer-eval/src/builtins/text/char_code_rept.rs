//! CHAR, CODE, REPT text functions

use super::{
    super::utils::{ARG_ANY_ONE, ARG_ANY_TWO, coerce_num},
    scalar_text_value,
};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

fn scalar_like_value(arg: &ArgumentHandle<'_, '_>) -> Result<LiteralValue, ExcelError> {
    Ok(match arg.value()? {
        CalcValue::Scalar(v) | CalcValue::AnnotatedScalar(v, _) => v,
        CalcValue::Range(rv) => rv.get_cell(0, 0),
        CalcValue::Callable(_) => LiteralValue::Error(
            ExcelError::new(ExcelErrorKind::Calc).with_message("LAMBDA value must be invoked"),
        ),
    })
}

/// Characters 128-159 of Windows-1252, the ANSI code page Excel for Windows
/// uses for CHAR and CODE; 160-255 are the Latin-1 characters of the same
/// code. The five codes Windows-1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90,
/// 0x9D) read as the C1 control of the same code, as Windows converts them.
const WINDOWS_1252_80_9F: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

#[derive(Debug)]
pub struct CharFn;
/// Returns the character represented by a numeric code.
///
/// `CHAR` follows Excel for Windows, which reads codes `1..255` in the Windows-1252 code page.
///
/// # Remarks
/// - Input is truncated to an integer code.
/// - Valid code range is `1` through `255`; outside this range returns `#VALUE!`.
/// - Codes 128-255 map through Windows-1252 (`CHAR(128)` is the euro sign, `CHAR(160)` a
///   non-breaking space); its five undefined codes return the C1 control of the same code.
/// - Errors are propagated unchanged.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "ASCII character"
/// formula: '=CHAR(65)'
/// expected: "A"
/// ```
///
/// ```yaml,sandbox
/// title: "Out-of-range code"
/// formula: '=CHAR(300)'
/// expected: "#VALUE!"
/// ```
///
/// ```yaml,docs
/// related:
///   - CODE
///   - UNICHAR
///   - UNICODE
/// faq:
///   - q: "Which character set does CHAR use for codes 128-255?"
///     a: "Windows-1252, as Excel for Windows does: CHAR(128) is the euro sign and CHAR(160) a non-breaking space."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: CHAR
/// Type: CharFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: CHAR(arg1: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CharFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "CHAR"
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
        let v = scalar_like_value(&args[0])?;
        let n = match v {
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => coerce_num(&other)?,
        };

        let code = n.trunc() as i32;

        // Excel CHAR accepts 1-255
        if !(1..=255).contains(&code) {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        // Excel for Windows reads codes through Windows-1252.
        let unicode_char = match code {
            0x80..=0x9F => WINDOWS_1252_80_9F[code as usize - 0x80],
            _ => char::from(code as u8),
        };

        Ok(CalcValue::Scalar(LiteralValue::Text(
            unicode_char.to_string(),
        )))
    }
}

/// CODE(text) - Returns a numeric code for the first character in a text string
#[derive(Debug)]
pub struct CodeFn;
/// Returns the numeric code of the first character in text.
///
/// `CODE` follows Excel for Windows and reports Windows-1252 codes.
///
/// # Remarks
/// - Only the first character is inspected.
/// - Empty text returns `#VALUE!`.
/// - Text-like coercion is applied to non-text scalar inputs.
/// - Characters 128-255 of Windows-1252 map back to their codes; other characters report
///   63, the code of the "?" they convert to.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "ASCII code"
/// formula: '=CODE("A")'
/// expected: 65
/// ```
///
/// ```yaml,sandbox
/// title: "Extended mapping"
/// formula: '=CODE(CHAR(128))'
/// expected: 128
/// ```
///
/// ```yaml,docs
/// related:
///   - CHAR
///   - UNICODE
///   - UNICHAR
/// faq:
///   - q: "What if the input text is empty?"
///     a: "CODE returns #VALUE! because there is no first character to evaluate."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: CODE
/// Type: CodeFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: CODE(arg1: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CodeFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "CODE"
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
        let v = scalar_text_value(&args[0])?;
        let s = match v {
            LiteralValue::Text(t) => t,
            LiteralValue::Empty => {
                return Ok(CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new_value(),
                )));
            }
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => other.to_string(),
        };

        if s.is_empty() {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        let first_char = s.chars().next().unwrap();

        // The Windows-1252 code; characters outside it read as the "?" they
        // convert to.
        let code = match first_char as u32 {
            cp @ (0x00..=0x7F | 0xA0..=0xFF) => cp as i64,
            _ => WINDOWS_1252_80_9F
                .iter()
                .position(|&c| c == first_char)
                .map_or(63, |i| i as i64 + 0x80),
        };

        Ok(CalcValue::Scalar(LiteralValue::Int(code)))
    }
}

fn asc_convert(text: &str) -> String {
    text.chars()
        .map(|c| {
            let cp = c as u32;
            if cp == 0x3000 {
                ' '
            } else if (0xFF01..=0xFF5E).contains(&cp) {
                char::from_u32(cp - 0xFF01 + 0x21).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// Converts full-width Latin and ASCII characters to half-width text.
///
/// Maps full-width ASCII punctuation, digits, letters, and ideographic space to
/// their half-width equivalents while leaving other characters unchanged.
///
/// ```yaml,sandbox
/// title: "Convert full-width letters and digits"
/// formula: '=ASC("ＡＢＣ１２３")'
/// expected: "ABC123"
/// ```
///
/// ```yaml,sandbox
/// title: "Convert ideographic space"
/// formula: '=ASC("Ａ　Ｂ")'
/// expected: "A B"
/// ```
///
/// ```yaml,docs
/// related:
///   - CHAR
///   - CODE
///   - UNICHAR
/// faq:
///   - q: "Are non-ASCII full-width characters transliterated?"
///     a: "No. ASC only maps the full-width ASCII block and ideographic space."
/// ```
#[derive(Debug)]
pub struct AscFn;
/// [formualizer-docgen:schema:start]
/// Name: ASC
/// Type: AscFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: ASC(arg1: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for AscFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ASC"
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
        let v = scalar_text_value(&args[0])?;
        let s = match v {
            LiteralValue::Text(t) => t,
            LiteralValue::Empty => String::new(),
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => other.to_string(),
        };
        Ok(CalcValue::Scalar(LiteralValue::Text(asc_convert(&s))))
    }
}

/// REPT(text, number_times) - Repeats text a given number of times
#[derive(Debug)]
pub struct ReptFn;
/// Repeats a text string a specified number of times.
///
/// # Remarks
/// - Repeat count is truncated to an integer.
/// - Negative counts return `#VALUE!`.
/// - Output longer than 32,767 characters returns `#VALUE!`.
/// - Non-text first argument is coerced to text.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Repeat text three times"
/// formula: '=REPT("ab", 3)'
/// expected: "ababab"
/// ```
///
/// ```yaml,sandbox
/// title: "Negative count"
/// formula: '=REPT("x", -1)'
/// expected: "#VALUE!"
/// ```
///
/// ```yaml,docs
/// related:
///   - CONCAT
///   - TEXTJOIN
///   - SUBSTITUTE
/// faq:
///   - q: "Can REPT return very long strings?"
///     a: "Only up to 32,767 characters; longer results return #VALUE! like Excel."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: REPT
/// Type: ReptFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: REPT(arg1: any@scalar, arg2: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ReptFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "REPT"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_ANY_TWO[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let text_val = scalar_text_value(&args[0])?;
        let count_val = scalar_like_value(&args[1])?;

        let text = match text_val {
            LiteralValue::Text(t) => t,
            LiteralValue::Empty => String::new(),
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => other.to_string(),
        };

        let count = match count_val {
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => coerce_num(&other)?,
        };

        let count = count.trunc() as i64;

        if count < 0 {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        // Excel limits result to 32767 characters
        let max_result_len = 32767;
        let result_len = text.len() * (count as usize);
        if result_len > max_result_len {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        let result = text.repeat(count as usize);
        Ok(CalcValue::Scalar(LiteralValue::Text(result)))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(CharFn));
    crate::function_registry::register_builtin(Arc::new(CodeFn));
    crate::function_registry::register_builtin(Arc::new(AscFn));
    crate::function_registry::register_builtin(Arc::new(ReptFn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_parse::parser::{ASTNode, ASTNodeType};

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }
    fn lit(v: LiteralValue) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(v), None)
    }

    #[test]
    fn char_basic() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(CharFn));
        let ctx = interp(&wb);
        let n = lit(LiteralValue::Number(65.0));
        let f = ctx.context.get_function("", "CHAR").unwrap();
        assert_eq!(
            f.dispatch(
                &[ArgumentHandle::new(&n, &ctx)],
                &ctx.function_context(None)
            )
            .unwrap()
            .into_literal(),
            LiteralValue::Text("A".to_string())
        );
    }

    #[test]
    fn char_and_code_use_windows_1252() {
        use formualizer_parse::parser::parse;
        let wb = TestWorkbook::new()
            .with_function(std::sync::Arc::new(CharFn))
            .with_function(std::sync::Arc::new(CodeFn));
        let ctx = interp(&wb);
        let eval = |f: &str| ctx.evaluate_ast(&parse(f).unwrap()).unwrap().into_literal();
        let text = |s: &str| LiteralValue::Text(s.into());
        assert_eq!(eval("=CHAR(65)"), text("A"));
        assert_eq!(eval("=CHAR(128)"), text("\u{20ac}"));
        assert_eq!(eval("=CHAR(150)"), text("\u{2013}"));
        assert_eq!(eval("=CHAR(160)"), text("\u{a0}"));
        assert_eq!(eval("=CHAR(169)"), text("\u{a9}"));
        assert_eq!(eval("=CHAR(233)"), text("\u{e9}"));
        assert_eq!(eval("=CHAR(255.9)"), text("\u{ff}"));
        // Undefined in Windows-1252: the C1 control of the same code.
        assert_eq!(eval("=CHAR(129)"), text("\u{81}"));
        assert_eq!(eval("=CHAR(157)"), text("\u{9d}"));
        assert_eq!(eval("=CODE(\"\u{20ac}\")"), LiteralValue::Int(128));
        assert_eq!(eval("=CODE(\"\u{2020}\")"), LiteralValue::Int(134));
        assert_eq!(eval("=CODE(\"\u{a0}x\")"), LiteralValue::Int(160));
        assert_eq!(eval("=CODE(\"\u{e9}\")"), LiteralValue::Int(233));
        // Outside Windows-1252, with no best-fit substitute.
        assert_eq!(eval("=CODE(\"\u{3042}\")"), LiteralValue::Int(63));
        assert_eq!(eval("=CODE(\"\u{2011}\")"), LiteralValue::Int(63));
        assert_eq!(eval("=CODE(\"\u{80}\")"), LiteralValue::Int(63));
        for code in 1..=255 {
            assert_eq!(
                eval(&format!("=CODE(CHAR({code}))")),
                LiteralValue::Int(code),
                "CHAR({code}) round trip"
            );
        }
        for f in [
            "=CHAR(0)",
            "=CHAR(0.5)",
            "=CHAR(256)",
            "=CHAR(-1)",
            "=CODE(\"\")",
        ] {
            assert!(
                matches!(eval(f), LiteralValue::Error(ref e) if e.kind == ExcelErrorKind::Value),
                "{f} is #VALUE!"
            );
        }
    }

    #[test]
    fn code_basic() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(CodeFn));
        let ctx = interp(&wb);
        let s = lit(LiteralValue::Text("A".to_string()));
        let f = ctx.context.get_function("", "CODE").unwrap();
        assert_eq!(
            f.dispatch(
                &[ArgumentHandle::new(&s, &ctx)],
                &ctx.function_context(None)
            )
            .unwrap()
            .into_literal(),
            LiteralValue::Int(65)
        );
    }

    #[test]
    fn asc_converts_full_width_ascii_and_space() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AscFn));
        let ctx = interp(&wb);
        let s = lit(LiteralValue::Text("ＡＢＣ１２３！　x".to_string()));
        let f = ctx.context.get_function("", "ASC").unwrap();
        assert_eq!(
            f.dispatch(
                &[ArgumentHandle::new(&s, &ctx)],
                &ctx.function_context(None)
            )
            .unwrap()
            .into_literal(),
            LiteralValue::Text("ABC123! x".to_string())
        );
    }

    #[test]
    fn rept_basic() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(ReptFn));
        let ctx = interp(&wb);
        let s = lit(LiteralValue::Text("ab".to_string()));
        let n = lit(LiteralValue::Number(3.0));
        let f = ctx.context.get_function("", "REPT").unwrap();
        assert_eq!(
            f.dispatch(
                &[ArgumentHandle::new(&s, &ctx), ArgumentHandle::new(&n, &ctx)],
                &ctx.function_context(None)
            )
            .unwrap()
            .into_literal(),
            LiteralValue::Text("ababab".to_string())
        );
    }
}
