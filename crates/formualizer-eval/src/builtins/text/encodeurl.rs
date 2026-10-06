//! ENCODEURL: percent-encoding as Excel for Windows does it.

use super::{super::utils::ARG_ANY_ONE, scalar_text_value};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;

/// The longest result ENCODEURL returns: Excel for Windows gives 32766
/// characters and `#VALUE!` from 32767 on.
const MAX_RESULT: usize = 32766;

#[derive(Debug)]
pub struct EncodeUrlFn;
/// Returns text percent-encoded for use in a URL.
///
/// # Remarks
/// - Letters `A-Z` and `a-z`, digits, `-`, `_` and `.` are kept; every other
///   character is written as the `%XX` codes of its UTF-8 bytes, in upper
///   case. Excel for Windows encodes `~` too (`%7E`).
/// - A number or logical is encoded as its text (`1.5`, `TRUE`); an empty
///   cell gives empty text.
/// - A result of 32767 characters or more returns `#VALUE!` (Excel for
///   Windows returns at most 32766).
/// - Excel for Mac and Excel for the web do not have ENCODEURL.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Spaces and separators"
/// formula: '=ENCODEURL("a b&c=d")'
/// expected: "a%20b%26c%3Dd"
/// ```
///
/// ```yaml,docs
/// related:
///   - WEBSERVICE
///   - FILTERXML
/// faq:
///   - q: "Is the tilde kept?"
///     a: "No. Excel for Windows writes it as %7E; only letters, digits, - _ and . are kept."
/// ```
impl Function for EncodeUrlFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ENCODEURL"
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
        let text = match scalar_text_value(&args[0])? {
            LiteralValue::Error(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            other => crate::coercion::to_text_invariant(&other),
        };
        let mut out = String::with_capacity(text.len());
        for byte in text.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
        Ok(CalcValue::Scalar(if out.len() > MAX_RESULT {
            LiteralValue::Error(ExcelError::new_value())
        } else {
            LiteralValue::Text(out)
        }))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(EncodeUrlFn));
}
