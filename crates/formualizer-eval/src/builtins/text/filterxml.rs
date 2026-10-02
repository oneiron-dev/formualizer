//! FILTERXML: XPath 1.0 over XML text, as Excel for Windows evaluates it
//! with MSXML.

use super::super::utils::{ARG_ANY_TWO, collapse_if_scalar};
use super::scalar_text_value;
use crate::args::ArgSchema;
use crate::engine::CancelToken;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;
use xml::{Document, is_xml_space};
use xpath::Failure;

mod xml;
mod xpath;

/// The longest XPath FILTERXML accepts.
const MAX_XPATH_CHARS: usize = 1024;

/// Parsing and evaluating an XPath recurses as deep as its parentheses and
/// brackets nest. Up to this nesting that takes little stack; a deeper XPath
/// (up to 512 levels fit in 1024 characters) runs on a thread of its own
/// with [`DEEP_XPATH_STACK_BYTES`], so it never overflows the caller's stack
/// (a 2 MiB rayon worker, say).
const INLINE_NESTING: usize = 32;

/// The stack of that thread: room for 512 levels several times over, in
/// debug builds too. Untouched stack pages cost no memory.
const DEEP_XPATH_STACK_BYTES: usize = 16 << 20;

fn to_text(arg: &ArgumentHandle<'_, '_>) -> Result<String, ExcelError> {
    Ok(match scalar_text_value(arg)? {
        LiteralValue::Text(s) => s,
        LiteralValue::Empty => String::new(),
        LiteralValue::Boolean(b) => if b { "TRUE" } else { "FALSE" }.into(),
        LiteralValue::Int(i) => crate::coercion::int_to_text(i),
        LiteralValue::Number(n) => crate::coercion::number_to_text(n),
        LiteralValue::Error(e) => return Err(e),
        other => other.to_string(),
    })
}

/// The text of each node `xpath` selects in `xml`, in document order (none
/// when it selects no node); `Err(Failure::Invalid)` when the XML or the
/// XPath is invalid or the XPath evaluates to something other than nodes
/// (`count(//a)`), `Err(Failure::Cancelled)` when `cancel` is signalled.
///
/// MSXML loads the XML without its white-space-only text nodes (white space is
/// not preserved unless `xml:space="preserve"`), and a node's text is trimmed
/// of leading and trailing white space; XPath tests see the untrimmed values.
fn select_node_texts(
    xml: &str,
    xpath: &str,
    cancel: Option<&CancelToken>,
) -> Result<Vec<String>, Failure> {
    if xpath.chars().count() > MAX_XPATH_CHARS {
        return Err(Failure::Invalid);
    }
    let tokens = xpath::tokenize(xpath)?;
    if tokens.nesting() <= INLINE_NESTING {
        return evaluate(xml, tokens, cancel);
    }
    // Where no thread can start (wasm), a deep XPath is #VALUE!.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DEEP_XPATH_STACK_BYTES)
            .spawn_scoped(scope, || evaluate(xml, tokens, cancel))
            .map_err(|_| Failure::Invalid)?
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// Parses the XPath, loads the XML and returns the text of the selected nodes.
fn evaluate(
    xml: &str,
    tokens: xpath::Tokens,
    cancel: Option<&CancelToken>,
) -> Result<Vec<String>, Failure> {
    let xpath = xpath::compile(tokens)?;
    let document = Document::parse(xml).ok_or(Failure::Invalid)?;
    Ok(xpath::select(&document, &xpath, cancel)?
        .into_iter()
        .map(|node| {
            document
                .string_value(node)
                .trim_matches(is_xml_space)
                .to_string()
        })
        .collect())
}

#[derive(Debug)]
pub struct FilterXmlFn;
/// Returns the values of the nodes an XPath 1.0 expression selects in XML text.
///
/// # Remarks
/// - Invalid XML, an invalid XPath (or one over 1024 characters, or with a
///   namespace prefix other than `xml`), an XPath that evaluates to a number,
///   string or boolean instead of nodes, and an XPath that selects no node all
///   return `#VALUE!`.
/// - Each selected node gives its text (an element's text content, an
///   attribute's value), trimmed of surrounding white space; an empty one is
///   `#VALUE!`.
/// - Text that reads as a number (as `VALUE` reads it) is returned as a number.
/// - Several nodes return a vertical array in document order.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Second item of a list"
/// formula: '=FILTERXML("<t><s>a</s><s>b</s></t>","//s[2]")'
/// expected: "b"
/// ```
///
/// ```yaml,sandbox
/// title: "Numeric text becomes a number"
/// formula: '=FILTERXML("<a>007</a>","//a")'
/// expected: 7
/// ```
///
/// ```yaml,docs
/// related:
///   - TEXTSPLIT
///   - SUBSTITUTE
/// faq:
///   - q: "Can the XPath return count() or a string?"
///     a: "No. FILTERXML returns nodes only; other XPath results are #VALUE!."
/// ```
impl Function for FilterXmlFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "FILTERXML"
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
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let xml = to_text(&args[0])?;
        let xpath = to_text(&args[1])?;
        let cancel = ctx.cancellation_token();
        let texts = match select_node_texts(&xml, &xpath, cancel.as_ref()) {
            Ok(texts) if !texts.is_empty() => texts,
            Err(Failure::Cancelled) => return Err(ExcelError::new(ExcelErrorKind::Cancelled)),
            _ => {
                return Ok(CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new_value(),
                )));
            }
        };
        let locale = ctx.locale();
        let year = args[0].current_year();
        let rows = texts
            .into_iter()
            .map(|text| {
                vec![if text.is_empty() {
                    LiteralValue::Error(ExcelError::new_value())
                } else {
                    let text = LiteralValue::Text(text);
                    // Excel reads no "inf" or "NaN" as a number.
                    match crate::coercion::to_arithmetic_number_with_locale(
                        &text,
                        &locale,
                        ctx.date_system(),
                        Some(year),
                    ) {
                        Ok(n) if n.is_finite() => LiteralValue::Number(n),
                        _ => text,
                    }
                }]
            })
            .collect();
        Ok(collapse_if_scalar(rows, ctx.date_system()))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(FilterXmlFn));
}

#[cfg(test)]
mod tests {
    use super::xml::Document;
    use super::xpath::{self, Failure};
    use crate::engine::{CancelToken, Engine, EvalConfig};
    use crate::test_workbook::TestWorkbook;
    use formualizer_common::{ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::parse;
    use std::time::{Duration, Instant};

    const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

    /// The split idiom: `items` separated by `delimiter`, as `<t><s>` nodes.
    fn split(items: &str, delimiter: &str) -> String {
        format!(r#""<t><s>"&SUBSTITUTE("{items}","{delimiter}","</s><s>")&"</s></t>""#)
    }

    fn engine(legacy: bool, cells: &[(u32, u32, &str)]) -> Engine<TestWorkbook> {
        let mut engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig {
                enable_parallel: false,
                ..Default::default()
            },
        );
        for &(row, col, content) in cells {
            if content.starts_with('=') {
                engine
                    .set_cell_formula("Sheet1", row, col, parse(content).unwrap())
                    .unwrap();
            } else {
                let value = content
                    .parse::<f64>()
                    .map(LiteralValue::Number)
                    .unwrap_or_else(|_| LiteralValue::Text(content.into()));
                engine.set_cell_value("Sheet1", row, col, value).unwrap();
            }
        }
        if legacy {
            engine.use_legacy_array_semantics();
        }
        engine.evaluate_all().unwrap();
        engine
    }

    /// The value of `formula` evaluated in E1.
    fn eval(formula: &str) -> LiteralValue {
        engine(false, &[(1, 5, formula)])
            .get_cell_value("Sheet1", 1, 5)
            .unwrap_or(LiteralValue::Empty)
    }

    /// The cells `formula` (in E1) fills down column E, up to the first empty one.
    fn spill(formula: &str) -> Vec<LiteralValue> {
        let engine = engine(false, &[(1, 5, formula)]);
        (1..)
            .map_while(|row| engine.get_cell_value("Sheet1", row, 5))
            .collect()
    }

    fn text(s: &str) -> LiteralValue {
        LiteralValue::Text(s.into())
    }

    fn number(n: f64) -> LiteralValue {
        LiteralValue::Number(n)
    }

    fn error_kind(value: LiteralValue) -> ExcelErrorKind {
        match value {
            LiteralValue::Error(error) => error.kind,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn splits_text_into_a_vertical_spill() {
        assert_eq!(
            spill(&format!("=FILTERXML({},\"//s\")", split("a,b,c", ","))),
            vec![text("a"), text("b"), text("c")]
        );
        // One node is a single value: nothing spills.
        let engine = engine(
            false,
            &[(1, 5, "=FILTERXML(\"<t><s>a</s><s>b</s></t>\",\"//s[2]\")")],
        );
        assert_eq!(engine.get_cell_value("Sheet1", 1, 5), Some(text("b")));
        assert_eq!(engine.get_cell_value("Sheet1", 2, 5), None);
        assert_eq!(
            eval("=ROWS(FILTERXML(\"<t><s>a</s><s>b</s><s>c</s></t>\",\"//s\"))"),
            number(3.0)
        );
        assert_eq!(
            eval("=COLUMNS(FILTERXML(\"<t><s>a</s><s>b</s><s>c</s></t>\",\"//s\"))"),
            number(1.0)
        );
    }

    #[test]
    fn common_xpath_idioms() {
        let list = split("ABC|123|DEF|456|XY-1A|123", "|");
        let nodes = |xpath: &str| spill(&format!("=FILTERXML({list},\"{xpath}\")"));
        assert_eq!(nodes("//s[2]"), vec![number(123.0)]);
        assert_eq!(nodes("//s[last()]"), vec![number(123.0)]);
        assert_eq!(nodes("//s[.>200]"), vec![number(456.0)]);
        assert_eq!(
            nodes("//s[.*0=0]"),
            vec![number(123.0), number(456.0), number(123.0)]
        );
        assert_eq!(
            nodes("//s[not(.=preceding::*)]"),
            vec![
                text("ABC"),
                number(123.0),
                text("DEF"),
                number(456.0),
                text("XY-1A")
            ]
        );
        assert_eq!(
            nodes("//s[position()>4]"),
            vec![text("XY-1A"), number(123.0)]
        );
        assert_eq!(
            nodes("//s[starts-with(., 'XY') or contains(., 'E')]"),
            vec![text("DEF"), text("XY-1A")]
        );
        // A union comes back in document order.
        assert_eq!(nodes("(//s[5]|//s[1])"), vec![text("ABC"), text("XY-1A")]);
    }

    #[test]
    fn attributes_and_other_nodes() {
        assert_eq!(
            spill("=FILTERXML(\"<r><a x='1'/><a x='two'/></r>\",\"//a/@x\")"),
            vec![number(1.0), text("two")]
        );
        assert_eq!(
            eval("=FILTERXML(\"<r><a>x<b>y</b>z</a></r>\",\"/r/a\")"),
            text("xyz")
        );
        assert_eq!(
            eval("=FILTERXML(\"<r><a>x<b>y</b>z</a></r>\",\"//b/text()\")"),
            text("y")
        );
        // Text split by references and CDATA sections is one text node.
        assert_eq!(
            eval("=FILTERXML(\"<r><![CDATA[1<2]]> &amp; &#65;</r>\",\"/r/text()\")"),
            text("1<2 & A")
        );
        assert_eq!(
            eval("=FILTERXML(\"<r>&amp; &amp;</r>\",\"//r[.='& &']\")"),
            text("& &")
        );
    }

    #[test]
    fn numeric_text_becomes_a_number() {
        assert_eq!(eval("=FILTERXML(\"<a>1</a>\",\"//a\")"), number(1.0));
        assert_eq!(eval("=FILTERXML(\"<a>007</a>\",\"//a\")"), number(7.0));
        assert_eq!(eval("=FILTERXML(\"<a>-2.5</a>\",\"//a\")"), number(-2.5));
        assert_eq!(eval("=FILTERXML(\"<a>50%</a>\",\"//a\")"), number(0.5));
        assert_eq!(
            eval("=FILTERXML(\"<a>1/2/2020</a>\",\"//a\")"),
            number(43832.0)
        );
        assert_eq!(
            eval("=FILTERXML(\"<a>12 apples</a>\",\"//a\")"),
            text("12 apples")
        );
        assert_eq!(eval("=FILTERXML(\"<a>TRUE</a>\",\"//a\")"), text("TRUE"));
        for word in ["Nan", "inf", "Infinity", "1e999"] {
            assert_eq!(
                eval(&format!("=FILTERXML(\"<a>{word}</a>\",\"//a\")")),
                text(word)
            );
        }
    }

    #[test]
    fn node_text_is_trimmed_and_empty_nodes_are_value_errors() {
        // The XPath sees " a " untrimmed; the result is trimmed.
        assert_eq!(
            eval("=FILTERXML(\"<t><s> a </s></t>\",\"//s[.=' a ']\")"),
            text("a")
        );
        let values = spill(&format!("=FILTERXML({},\"//s\")", split("a,,b", ",")));
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], text("a"));
        assert_eq!(error_kind(values[1].clone()), ExcelErrorKind::Value);
        assert_eq!(values[2], text("b"));
        // A blank cell split this way is one empty node.
        assert_eq!(
            error_kind(eval(&format!("=FILTERXML({},\"//s\")", split("", ",")))),
            ExcelErrorKind::Value
        );
        assert_eq!(
            error_kind(eval("=FILTERXML(\"<t><s> </s></t>\",\"//s\")")),
            ExcelErrorKind::Value
        );
    }

    #[test]
    fn white_space_only_text_nodes_are_dropped() {
        let xml = "\"<t>\"&CHAR(10)&\" <s>1</s>\"&CHAR(10)&\" <s>2</s>\"&CHAR(10)&\"</t>\"";
        assert_eq!(
            spill(&format!("=FILTERXML({xml},\"/t/node()\")")),
            vec![number(1.0), number(2.0)]
        );
        assert_eq!(
            eval(&format!("=FILTERXML({xml},\"/t/node()[2]\")")),
            number(2.0)
        );
        // xml:space="preserve" keeps them (and the node's text is still trimmed).
        assert_eq!(
            error_kind(eval(
                "=FILTERXML(\"<t xml:space='preserve'> <s>1</s></t>\",\"/t/node()[1]\")"
            )),
            ExcelErrorKind::Value
        );
    }

    #[test]
    fn invalid_xml_xpath_or_no_match_is_value_error() {
        for formula in [
            // Invalid XML.
            "=FILTERXML(\"<a>1\",\"//a\")",
            "=FILTERXML(\"<a>1</A>\",\"//a\")",
            "=FILTERXML(\"<a>1</a><b/>\",\"//a\")",
            "=FILTERXML(\"text\",\"//a\")",
            "=FILTERXML(\"\",\"//a\")",
            "=FILTERXML(\"<a>&nbsp;</a>\",\"//a\")",
            "=FILTERXML(\"<a>1 & 2</a>\",\"//a\")",
            "=FILTERXML(\"<p:a>1</p:a>\",\"//a\")",
            "=FILTERXML(42,\"//a\")",
            // Invalid XPath.
            "=FILTERXML(\"<a>1</a>\",\"//a[1\")",
            "=FILTERXML(\"<a>1</a>\",\"//[1]\")",
            "=FILTERXML(\"<a>1</a>\",\"\")",
            "=FILTERXML(\"<a>1</a>\",\"//a[foo()]\")",
            "=FILTERXML(\"<a>1</a>\",\"$v\")",
            // Undeclared namespace prefixes.
            "=FILTERXML(\"<a xmlns:p='urn:p'><p:b>1</p:b></a>\",\"//p:b\")",
            "=FILTERXML(\"<a xmlns:p='urn:p'><p:b>1</p:b></a>\",\"//p:*\")",
            // No node selected.
            "=FILTERXML(\"<a>1</a>\",\"//b\")",
            "=FILTERXML(\"<t><s>a</s></t>\",\"//s[2]\")",
            "=FILTERXML(\"<a xmlns='urn:x'><b>1</b></a>\",\"//b\")",
            // Results that are not nodes.
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"count(//s)\")",
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"sum(//s)\")",
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"string(//s)\")",
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"number(//s[2])\")",
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"boolean(//s)\")",
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"1=1\")",
            "=FILTERXML(\"<t><s>1</s><s>2</s></t>\",\"'s'\")",
        ] {
            assert_eq!(
                error_kind(eval(formula)),
                ExcelErrorKind::Value,
                "{formula}"
            );
        }
        // Prefixed names are fine inside string literals, and local-name()
        // reaches namespaced nodes.
        assert_eq!(
            eval(
                "=FILTERXML(\"<a xmlns:p='urn:p'><p:b>1</p:b></a>\",\"//*[local-name()='b' or .='p:b']\")"
            ),
            number(1.0)
        );
        assert_eq!(
            eval("=FILTERXML(\"<?xml version='1.0' encoding='UTF-8'?><a>1</a>\",\"child::a\")"),
            number(1.0)
        );
    }

    #[test]
    fn xpath_is_limited_to_1024_characters() {
        // "//a" and 1021 spaces is 1024 characters.
        assert_eq!(
            eval("=FILTERXML(\"<a>1</a>\",\"//a\"&REPT(\" \",1021))"),
            number(1.0)
        );
        assert_eq!(
            error_kind(eval("=FILTERXML(\"<a>1</a>\",\"//a\"&REPT(\" \",1022))")),
            ExcelErrorKind::Value
        );
    }

    #[test]
    fn error_arguments_propagate() {
        assert_eq!(
            error_kind(eval("=FILTERXML(1/0,\"//a\")")),
            ExcelErrorKind::Div
        );
        assert_eq!(
            error_kind(eval("=FILTERXML(\"<a>1</a>\",NA())")),
            ExcelErrorKind::Na
        );
        assert_eq!(error_kind(eval("=FILTERXML(NA(),1/0)")), ExcelErrorKind::Na);
    }

    #[test]
    fn criteria_of_sumifs_lift_over_the_nodes() {
        // C1:D4 = names and amounts; F1 lists the names to add up.
        let cells = [
            (1, 3, "x"),
            (1, 4, "1"),
            (2, 3, "y"),
            (2, 4, "10"),
            (3, 3, "z"),
            (3, 4, "100"),
            (4, 3, "x"),
            (4, 4, "1000"),
            (1, 6, "x, z"),
            (
                2,
                6,
                "=SUM(SUMIFS(D:D,C:C,_xlfn.FILTERXML(\"<k><m>\"&SUBSTITUTE(F1,\", \",\"</m><m>\")&\"</m></k>\",\"//m\")))",
            ),
        ];
        for legacy in [false, true] {
            assert_eq!(
                engine(legacy, &cells).get_cell_value("Sheet1", 2, 6),
                Some(number(1101.0)),
                "legacy={legacy}"
            );
        }
    }

    #[test]
    fn xlfn_prefix_resolves() {
        assert_eq!(eval("=_xlfn.FILTERXML(\"<a>1</a>\",\"//a\")"), number(1.0));
    }

    #[test]
    fn lifts_over_arrays_of_xml() {
        let engine = engine(
            false,
            &[(1, 5, "=FILTERXML({\"<a>1</a>\",\"<a>2</a>\"},\"//a\")")],
        );
        assert_eq!(engine.get_cell_value("Sheet1", 1, 5), Some(number(1.0)));
        assert_eq!(engine.get_cell_value("Sheet1", 1, 6), Some(number(2.0)));
    }

    #[test]
    fn legacy_formula_intersects_a_range_of_xml() {
        let cells = [
            (1, 1, "<a>1</a>"),
            (2, 1, "<a>2</a>"),
            (3, 1, "<a>3</a>"),
            (2, 3, "=FILTERXML(A1:A3,\"//a\")"),
            (5, 3, "=FILTERXML(\"<t><s>a</s><s>b</s></t>\",\"//s\")"),
        ];
        let engine = engine(true, &cells);
        assert_eq!(engine.get_cell_value("Sheet1", 2, 3), Some(number(2.0)));
        // An array result of a formula without the array flag is its first value.
        assert_eq!(engine.get_cell_value("Sheet1", 5, 3), Some(text("a")));
        assert_eq!(engine.get_cell_value("Sheet1", 6, 3), None);
    }

    #[test]
    fn registration_and_parameter_classes() {
        crate::builtins::load_builtins();
        let fun = crate::function_registry::get("", "FILTERXML").unwrap();
        assert!(!fun.volatile());
        assert!(fun.caps().contains(crate::function::FnCaps::MAY_SPILL));
        for index in 0..2 {
            assert_eq!(
                crate::lift::legacy_arg(fun.as_ref(), index),
                crate::lift::LegacyArg::Value
            );
        }
        // Owner ruling: the web functions stay unknown (#NAME?).
        for name in ["WEBSERVICE", "ENCODEURL"] {
            assert!(crate::function_registry::get("", name).is_none(), "{name}");
        }
    }

    /// FILTERXML of literal XML and XPath (neither may hold a double quote).
    fn filterxml(xml: &str, xpath: &str) -> LiteralValue {
        eval(&format!("=FILTERXML(\"{xml}\",\"{xpath}\")"))
    }

    /// The nodes FILTERXML of literal XML and XPath spills.
    fn filterxml_spill(xml: &str, xpath: &str) -> Vec<LiteralValue> {
        spill(&format!("=FILTERXML(\"{xml}\",\"{xpath}\")"))
    }

    fn assert_value_error(xml: &str, xpath: &str) {
        assert_eq!(
            error_kind(filterxml(xml, xpath)),
            ExcelErrorKind::Value,
            "{xml} {xpath}"
        );
    }

    #[test]
    fn numeric_predicate_selects_only_the_equal_position() {
        // XPath 1.0 §2.4: a number is true only when it equals the position,
        // so a fraction selects nothing and FILTERXML is #VALUE!.
        let four = "<t><s>a</s><s>b</s><s>c</s><s>d</s></t>";
        for xpath in [
            "//s[1.5]",
            "//s[1.9]",
            "//s[(last()+1) div 2]",
            "(//s)[2.5]",
            "//s[0]",
            "//s[-1]",
            "//s[0 div 0]",
            "//s[1 div 0]",
        ] {
            assert_value_error(four, xpath);
        }
        assert_eq!(filterxml(four, "//s[2.0]"), text("b"));
        assert_eq!(filterxml(four, "//s[last() div 2]"), text("b"));
        assert_eq!(filterxml(four, "(//s)[round(2.5)]"), text("c"));
        assert_eq!(filterxml(four, "//s[position()=1.0]"), text("a"));
        // With an odd count the middle position is whole.
        assert_eq!(
            filterxml("<t><s>a</s><s>b</s><s>c</s></t>", "//s[(last()+1) div 2]"),
            text("b")
        );
    }

    #[test]
    fn xpath_reads_only_xpath_numbers_from_text() {
        // XPath 1.0 §4.4: number() reads optional white space, an optional
        // minus sign and digits with an optional point; anything else is NaN.
        assert_value_error("<t><s>a</s><s>1e3</s></t>", "//s[.*0=0]");
        assert_value_error("<t><s>a</s><s>+5</s></t>", "//s[.*0=0]");
        assert_value_error("<t><s>2</s><s>3e2</s></t>", "//s[.>200]");
        assert_eq!(
            filterxml_spill("<t><s>Infinity</s><s>5</s></t>", "//s[.>1]"),
            vec![number(5.0)]
        );
        assert_eq!(
            error_kind(eval(
                "=FILTERXML(\"<t><s>\"&UNICHAR(160)&\"5</s></t>\",\"//s[.=5]\")"
            )),
            ExcelErrorKind::Value
        );
        let not_numbers = "1e3|1E3|1.|0x10|Infinity|-Infinity|NaN|inf|1,000|--1|.|-|1 1|1.2.3|8";
        assert_eq!(
            eval(&format!(
                "=FILTERXML({},\"//s[number(.)=number(.)][.!='1.']\")",
                split(not_numbers, "|")
            )),
            number(8.0)
        );
        // XML white space around a number is fine.
        assert_eq!(
            eval(&format!(
                "=ROWS(FILTERXML({},\"//s[.*0=0]\"))",
                split(" 7 |5.|.5|-.5|-2|x", "|")
            )),
            number(5.0)
        );
        assert_eq!(
            eval("=FILTERXML(\"<t><s>\"&CHAR(9)&\"5\"&CHAR(10)&\"</s></t>\",\"//s[.=5]\")"),
            number(5.0)
        );
        // The string of a number: no exponent, 0 for -0, Infinity and NaN.
        let strings =
            "<t><s>0</s><s>0.5</s><s>Infinity</s><s>NaN</s><s>100000000000000000000</s></t>";
        assert_eq!(filterxml(strings, "//s[.=string(-0)]"), number(0.0));
        assert_eq!(filterxml(strings, "//s[.=concat(1 div 2,'')]"), number(0.5));
        assert_eq!(
            filterxml(strings, "//s[.=string(1 div 0)]"),
            text("Infinity")
        );
        assert_eq!(filterxml(strings, "//s[.=string(0 div 0)]"), text("NaN"));
        assert_eq!(
            filterxml(strings, "//s[.=string(100000*100000*100000*100000)]"),
            number(1e20)
        );
    }

    #[test]
    fn xml_line_ends_and_attribute_white_space_are_normalized() {
        // XML 1.0 §2.11: CR LF and a lone CR are a line feed.
        assert_eq!(
            eval("=CODE(MID(FILTERXML(\"<a>p\"&CHAR(13)&\"q</a>\",\"//a\"),2,1))"),
            number(10.0)
        );
        assert_eq!(
            eval("=LEN(FILTERXML(\"<a>x\"&CHAR(13)&CHAR(10)&\"y</a>\",\"//a\"))"),
            number(3.0)
        );
        assert_eq!(
            eval("=CODE(MID(FILTERXML(\"<a><![CDATA[p\"&CHAR(13)&\"q]]></a>\",\"//a\"),2,1))"),
            number(10.0)
        );
        // §3.3.3: a tab, CR or LF written in an attribute value is a space.
        for code in [9, 10, 13] {
            assert_eq!(
                eval(&format!(
                    "=CODE(MID(FILTERXML(\"<a x='p\"&CHAR({code})&\"q'/>\",\"//@x\"),2,1))"
                )),
                number(32.0),
                "CHAR({code})"
            );
        }
        assert_eq!(
            eval("=LEN(FILTERXML(\"<a x='p\"&CHAR(13)&CHAR(10)&\"q'/>\",\"//@x\"))"),
            number(3.0)
        );
        assert_eq!(
            eval(
                "=CODE(MID(FILTERXML(\"<r><!-- ' --><b c=\"\"p\"&CHAR(9)&\"q\"\"/></r>\",\"//@c\"),2,1))"
            ),
            number(32.0)
        );
        // A character reference keeps its character, and content keeps its
        // white space.
        assert_eq!(
            eval("=CODE(MID(FILTERXML(\"<a x='p&#10;q'/>\",\"//@x\"),2,1))"),
            number(10.0)
        );
        assert_eq!(
            eval("=CODE(MID(FILTERXML(\"<a>p&#13;q</a>\",\"//a\"),2,1))"),
            number(13.0)
        );
        assert_eq!(
            eval("=CODE(MID(FILTERXML(\"<a>x='p\"&CHAR(9)&\"q'</a>\",\"//a\"),5,1))"),
            number(9.0)
        );
    }

    #[test]
    fn characters_xml_forbids_are_invalid_xml() {
        // XML 1.0 §2.2 and WFC Legal Character: a fatal error, so #VALUE!.
        for formula in [
            "=FILTERXML(\"<a>\"&CHAR(1)&\"z</a>\",\"//a\")",
            "=FILTERXML(\"<a>&#1;</a>\",\"//a\")",
            "=FILTERXML(\"<a>&#x1F;z</a>\",\"//a\")",
            "=FILTERXML(\"<a>&#0;z</a>\",\"//a\")",
            "=FILTERXML(\"<a x='&#2;'>1</a>\",\"//a\")",
            "=FILTERXML(\"<!--\"&CHAR(2)&\"--><a>1</a>\",\"//a\")",
            "=FILTERXML(\"<a>1</a><?pi \"&CHAR(31)&\"?>\",\"//a\")",
            "=FILTERXML(\"<a xmlns:p='&#3;'>1</a>\",\"//a\")",
            "=FILTERXML(\"<a>&#xD800;</a>\",\"//a\")",
            "=FILTERXML(\"<a>&#xFFFE;</a>\",\"//a\")",
        ] {
            assert_eq!(
                error_kind(eval(formula)),
                ExcelErrorKind::Value,
                "{formula}"
            );
        }
        assert_eq!(filterxml("<a>&#9;z&#10;</a>", "//a"), text("z"));
        assert_eq!(filterxml("<a>&#x10000;z</a>", "//a"), text("\u{10000}z"));
        // In a comment or CDATA section `&#1;` is text, not a reference.
        assert_eq!(
            filterxml("<a><!--&#1;--><![CDATA[&#1;]]></a>", "//a"),
            text("&#1;")
        );
    }

    #[test]
    fn lang_matches_the_nearest_xml_lang() {
        // XPath 1.0 §4.3: the xml:lang of the node or its nearest ancestor is
        // the language or a sublanguage of it, ignoring case.
        let en = "<a xml:lang='en'><s>x</s></a>";
        assert_eq!(filterxml(en, "//s[lang('en')]"), text("x"));
        assert_eq!(filterxml(en, "//s[lang('EN')]"), text("x"));
        assert_eq!(
            filterxml("<a xml:lang='EN-us'><s>x</s></a>", "//s[lang('en')]"),
            text("x")
        );
        assert_eq!(
            filterxml("<a xml:lang='en-US'><s>x</s></a>", "//s[lang('en-us')]"),
            text("x")
        );
        assert_eq!(
            filterxml("<a xml:lang='en' b='y'/>", "//@b[lang('en')]"),
            text("y")
        );
        assert_eq!(
            filterxml("<a xml:lang='en'>z</a>", "//text()[lang('en')]"),
            text("z")
        );
        assert_value_error(en, "//s[lang('en-US')]");
        assert_value_error(en, "//s[lang('e')]");
        assert_value_error("<a xml:lang='english'><s>x</s></a>", "//s[lang('en')]");
        assert_value_error(
            "<a xml:lang='en'><s xml:lang='de'>x</s></a>",
            "//s[lang('en')]",
        );
        assert_value_error("<a><s>x</s></a>", "//s[lang('en')]");
        assert_value_error(en, "//s[lang()]");
    }

    #[test]
    fn deeply_nested_xpaths_evaluate_on_a_small_stack() {
        // The finding's repro: 510 parentheses around //a, 1023 characters.
        assert_eq!(
            eval("=FILTERXML(\"<a>1</a>\",REPT(\"(\",510)&\"//a\"&REPT(\")\",510))"),
            number(1.0)
        );
        fn nest(open: &str, inner: &str, close: &str, levels: usize) -> String {
            format!("{}{inner}{}", open.repeat(levels), close.repeat(levels))
        }
        let one = || "<a>1</a>".to_string();
        let deep_xml = |depth| format!("{}1{}", "<a>".repeat(depth), "</a>".repeat(depth));
        // The deepest XPaths of each kind within 1024 characters.
        let cases = vec![
            (one(), nest("(", "//a", ")", 510)),
            (one(), format!("//a[{}]", nest("(", "1", ")", 509))),
            (one(), format!("//a[{}]", nest("not(", "false()", ")", 201))),
            (one(), format!("//a[{}=1]", nest("-(", "1", ")", 338))),
            (one(), format!("//a[{}=253]", nest("1+(", "0", ")", 253))),
            (one(), format!("//a[{}1=1]", "-".repeat(1000))),
            (one(), format!("//a[{}1>0]", "1+".repeat(500))),
            (one(), format!("//a{}", "|//a".repeat(255))),
            (deep_xml(341), format!("/{}", nest("a[", "1", "]", 340))),
            // The deepest XML a cell holds.
            (deep_xml(4680), "//a[not(a)]".to_string()),
        ];
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                for (xml, xpath) in cases {
                    assert!(xpath.chars().count() <= super::MAX_XPATH_CHARS, "{xpath}");
                    assert_eq!(
                        super::select_node_texts(&xml, &xpath, None),
                        Ok(vec!["1".to_string()]),
                        "{xpath}"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
        // Past the nesting bound (only reachable without the character limit)
        // the XPath is invalid rather than deeper; the bound itself fits the
        // stack deep XPaths run on.
        std::thread::Builder::new()
            .stack_size(super::DEEP_XPATH_STACK_BYTES)
            .spawn(move || {
                let compile = |levels| {
                    let xpath = nest("(", "//a", ")", levels);
                    super::xpath::tokenize(&xpath).and_then(super::xpath::compile)
                };
                assert!(compile(super::xpath::MAX_NESTING + 1).is_err());
                assert!(compile(5000).is_err());
                assert!(compile(super::xpath::MAX_NESTING).is_ok());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn axes_follow_xpath_1_0() {
        let xml = "<r><a i='1'><b>1</b><c>2</c></a><d>3<e>4</e></d><f>5</f></r>";
        let nodes = |xpath: &str| filterxml_spill(xml, xpath);
        // Reverse axes count from the nearest node.
        assert_eq!(nodes("//e/preceding::*[1]"), vec![number(2.0)]);
        assert_eq!(nodes("//e/preceding::*[3]"), vec![number(12.0)]);
        assert_eq!(nodes("//e/ancestor::*[1]"), vec![number(34.0)]);
        assert_eq!(nodes("//e/ancestor-or-self::*[2]"), vec![number(34.0)]);
        assert_eq!(nodes("//c/preceding-sibling::*[1]"), vec![number(1.0)]);
        assert_eq!(nodes("//b/following-sibling::*"), vec![number(2.0)]);
        // preceding leaves out ancestors, following leaves out descendants;
        // the result is in document order.
        assert_eq!(
            nodes("//d/preceding::*"),
            vec![number(12.0), number(1.0), number(2.0)]
        );
        assert_eq!(
            nodes("//a/following::*"),
            vec![number(34.0), number(4.0), number(5.0)]
        );
        assert_eq!(
            nodes("//b/descendant-or-self::node()"),
            vec![number(1.0), number(1.0)]
        );
        // An attribute's element and its children come before and after it.
        assert_eq!(nodes("//@i/parent::*"), vec![number(12.0)]);
        assert_eq!(nodes("//@i/following::*[1]"), vec![number(1.0)]);
        assert_eq!(nodes("//b/parent::a/@i"), vec![number(1.0)]);
        assert_value_error(xml, "//@i/following-sibling::node()");
        assert_eq!(nodes("/descendant::*[2]"), vec![number(12.0)]);
        assert_eq!(nodes("//*[2]"), vec![number(2.0), number(34.0)]);
        assert_eq!(nodes("/r/*[last()]"), vec![number(5.0)]);
        assert_eq!(nodes("//*[count(*)=2]"), vec![number(12.0)]);
        assert_eq!(nodes("//e/../../f"), vec![number(5.0)]);
        assert_eq!(nodes("//f/self::f"), vec![number(5.0)]);
        assert_value_error(xml, "//f/self::g");
        assert_eq!(
            filterxml("<a xmlns:p='urn:p'>1</a>", "/a/namespace::p"),
            text("urn:p")
        );
        // Node types.
        let mixed = "<t><!--c--><?pi data?>x</t>";
        assert_eq!(
            filterxml_spill(mixed, "/t/node()"),
            vec![text("c"), text("data"), text("x")]
        );
        assert_eq!(filterxml(mixed, "/t/comment()"), text("c"));
        assert_eq!(
            filterxml(mixed, "/t/processing-instruction('pi')"),
            text("data")
        );
        assert_value_error(mixed, "/t/processing-instruction('other')");
        assert_eq!(filterxml(mixed, "/t/text()"), text("x"));
        assert_eq!(filterxml(mixed, "/"), text("x"));
    }

    #[test]
    fn functions_follow_xpath_1_0() {
        let xml = "<t><s> a  b </s><s>12345</s><s>-2.5</s></t>";
        let one = |xpath: &str| filterxml(xml, xpath);
        assert_eq!(one("//s[normalize-space()='a b']"), text("a  b"));
        assert_eq!(one("//s[substring(.,1.5,2.6)='234']"), number(12345.0));
        assert_eq!(one("//s[substring(.,0,3)='12']"), number(12345.0));
        assert_eq!(
            one("//s[substring(.,-42,1 div 0)='12345']"),
            number(12345.0)
        );
        assert_eq!(one("//s[2][substring(.,1,0 div 0)='']"), number(12345.0));
        assert_eq!(one("//s[translate(.,'135','ab')='a2b4']"), number(12345.0));
        assert_eq!(one("//s[round(.)=-2]"), number(-2.5));
        assert_eq!(one("//s[floor(.)=-3 and ceiling(.)=-2]"), number(-2.5));
        assert_eq!(
            one("//s[round(0.49999999999999994)=0 and . mod 10=5]"),
            number(12345.0)
        );
        assert_eq!(
            one("//s[5 mod -2=1 and -5 mod 2=-1 and . div 5=2469]"),
            number(12345.0)
        );
        assert_eq!(
            one("//s[string-length()=5 and string()='12345']"),
            number(12345.0)
        );
        assert_eq!(
            one("//s[substring-before(.,'34')='12' and substring-after(.,'23')='45']"),
            number(12345.0)
        );
        assert_eq!(one("//s[concat(.,'x','y')='12345xy']"), number(12345.0));
        assert_eq!(one("//s[number()=12345]"), number(12345.0));
        assert_eq!(
            one("(//s)[sum(//s[position()>1])=12342.5][2]"),
            number(12345.0)
        );
        assert_eq!(one("//s[count(../s)=3][last()]"), number(-2.5));
        assert_eq!(
            one(
                "//s[starts-with(.,'12') and contains(.,'34') and not(false()) and boolean(.) and true()]"
            ),
            number(12345.0)
        );
        let names = "<p:a xmlns:p='urn:p'><p:b x='1'/></p:a>";
        assert_eq!(filterxml(names, "//*[name()='p:b']/@x"), number(1.0));
        assert_eq!(
            filterxml(
                names,
                "//*[local-name()='b' and namespace-uri()='urn:p']/@x"
            ),
            number(1.0)
        );
        // Without a DTD no node has an ID.
        assert_value_error(xml, "id('x')");
        // Unknown functions, wrong argument counts and node-set arguments that
        // are not node-sets are invalid.
        for xpath in [
            "//s[concat('a')]",
            "//s[substring(.)]",
            "//s[true(1)]",
            "//s[count(1)=1]",
            "//s[sum('1')=1]",
            "//s[name(1)='']",
        ] {
            assert_value_error(xml, xpath);
        }
    }

    #[test]
    fn operators_and_lexical_rules() {
        let list = "<t><s>1</s><s>2</s><s>3</s></t>";
        let one = |xpath: &str| filterxml(list, xpath);
        // `*` multiplies after an operand and is a name test elsewhere.
        assert_eq!(one("//s[. * 2 = 4]"), number(2.0));
        assert_eq!(one("//s[.*.=9]"), number(3.0));
        assert_eq!(one("/t/*[2]"), number(2.0));
        // Operator names are element names where an operand starts.
        let keywords = "<t><div>1</div><and>2</and><mod>3</mod></t>";
        assert_eq!(filterxml(keywords, "//div[. div 1 = 1]"), number(1.0));
        assert_eq!(filterxml(keywords, "/t/and"), number(2.0));
        assert_eq!(filterxml(keywords, "//mod[. mod 2 = 1]"), number(3.0));
        // Precedence, and left to right within one.
        assert_eq!(one("//s[1 + 2 * 3 = 7][1]"), number(1.0));
        assert_eq!(one("//s[(1 + 2) * 3 = 9][1]"), number(1.0));
        assert_eq!(one("//s[10 - 2 - 3 = 5][1]"), number(1.0));
        assert_eq!(one("//s[8 div 4 div 2 = 1][1]"), number(1.0));
        assert_eq!(one("//s[1 = 1 = 1][1]"), number(1.0));
        assert_value_error(list, "//s[3 > 2 > 1]");
        assert_eq!(
            one("//s[--1 = 1 and -(-(1)) = 1 and - - 2 = 2][1]"),
            number(1.0)
        );
        assert_eq!(one("//s[1 or 0 and 0][1]"), number(1.0));
        // Comparisons with node-sets hold for some node.
        assert_eq!(one("//s[. = //s[3]]"), number(3.0));
        assert_eq!(one("/t[s = 2]/s[1]"), number(1.0));
        assert_eq!(one("/t[s != 1]/s[1]"), number(1.0));
        assert_eq!(one("/t[s > 2 and 2 < s]/s[1]"), number(1.0));
        assert_value_error(list, "/t[s > 3]");
        assert_value_error(list, "/t[3 < s]");
        assert_eq!(one("/t[s = true() and x = false()]/s[1]"), number(1.0));
        assert_eq!(
            one("/t[s[1] != s[2] and not(s[1] != s[1])]/s[1]"),
            number(1.0)
        );
        // White space between tokens.
        assert_eq!(one("// s [ count ( ../s ) = 3 ] [ 2 ]"), number(2.0));
        assert_eq!(one("/t/child :: s[3]"), number(3.0));
        for xpath in [
            // . and .. take no predicates.
            "//s/.[1]",
            "//s/..[1]",
            "/ /s",
            "//s[. = $v]",
            "//s[1 foo 2]",
            "//s[.=']",
            "//s[1e3]",
            "//s[@]",
            "//s[]",
            "//s/",
            "bogus::s",
            "//s[. ! 1]",
        ] {
            assert_value_error(list, xpath);
        }
    }

    #[test]
    fn attributes_keep_their_source_order() {
        // MSXML keeps attributes in source order (XPath 1.0 leaves the order
        // to the implementation).
        let a = "<a z='1' b='2' m='3'/>";
        assert_eq!(
            filterxml_spill(a, "/a/@*"),
            vec![number(1.0), number(2.0), number(3.0)]
        );
        assert_eq!(filterxml(a, "/a/@*[1]"), number(1.0));
        assert_eq!(filterxml(a, "/a/@*[last()]"), number(3.0));
        assert_eq!(
            filterxml_spill("<r><a z='1' b='2'/><c y='3' x='4'/></r>", "//@*"),
            vec![number(1.0), number(2.0), number(3.0), number(4.0)]
        );
        // Namespace declarations are not attributes.
        assert_eq!(
            filterxml_spill(
                "<a z='1' xmlns:p='urn:p' p:b='2' xmlns='urn:d' c='3'/>",
                "/*/@*"
            ),
            vec![number(1.0), number(2.0), number(3.0)]
        );
    }

    #[test]
    fn default_namespace_declarations_cover_the_content_they_are_in() {
        // Namespaces in XML 1.0 §6.2: a default declaration on a prefixed
        // element covers its unprefixed content, which an unprefixed name
        // test (null namespace) then does not match.
        let prefixed = "<p:a xmlns:p='urn:p' xmlns='urn:d'><b>1</b></p:a>";
        assert_value_error(prefixed, "//b");
        assert_eq!(
            filterxml(prefixed, "//*[namespace-uri()='urn:d']"),
            number(1.0)
        );
        assert_eq!(filterxml(prefixed, "//*[local-name()='b']"), number(1.0));
        // RSS 1.0.
        let rss = "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#' xmlns='http://purl.org/rss/1.0/'><item><title>x</title></item></rdf:RDF>";
        assert_value_error(rss, "//item/title");
        assert_eq!(filterxml(rss, "//*[local-name()='title']"), text("x"));
        // xmlns='' takes the default namespace away for the whole subtree.
        let undeclared = "<a xmlns='urn:d'><b xmlns=''><c>1</c></b></a>";
        assert_eq!(filterxml(undeclared, "//c"), number(1.0));
        assert_eq!(filterxml(undeclared, "/*/b/c"), number(1.0));
        assert_value_error(
            undeclared,
            "//*[local-name()='c' and namespace-uri()='urn:d']",
        );
        assert_eq!(
            filterxml_spill(undeclared, "//c/namespace::*"),
            vec![text(XML_NAMESPACE)]
        );
        // An inner declaration holds for its subtree only.
        let nested = "<a xmlns='urn:1'><b xmlns='urn:2'><c>2</c></b><d>1</d></a>";
        assert_eq!(
            filterxml(nested, "//*[namespace-uri()='urn:1' and not(*)]"),
            number(1.0)
        );
        assert_eq!(
            filterxml(nested, "//*[namespace-uri()='urn:2' and not(*)]"),
            number(2.0)
        );
        // Unprefixed attributes are in no namespace, under a default too.
        assert_eq!(filterxml("<a xmlns='urn:d' x='1'/>", "//@x"), number(1.0));
    }

    #[test]
    fn the_namespace_axis_has_one_order() {
        // The predeclared xml prefix first, then the declarations in effect
        // in document order: a recalculation spills them the same way.
        let four = "<a xmlns:p='urn:p' xmlns:q='urn:q' xmlns:r='urn:r' xmlns:t='urn:t'/>";
        let expected = vec![
            text(XML_NAMESPACE),
            text("urn:p"),
            text("urn:q"),
            text("urn:r"),
            text("urn:t"),
        ];
        for _ in 0..40 {
            assert_eq!(filterxml_spill(four, "/a/namespace::*"), expected);
        }
        // An inner declaration of a prefix takes its place; the default
        // namespace is a namespace node too.
        let nested =
            "<a xmlns:q='urn:q1' xmlns='urn:d'><b xmlns:p='urn:p' xmlns:q='urn:q2'>z</b></a>";
        assert_eq!(
            filterxml_spill(nested, "//*[local-name()='b']/namespace::*"),
            vec![
                text(XML_NAMESPACE),
                text("urn:d"),
                text("urn:p"),
                text("urn:q2")
            ]
        );
        assert_eq!(
            filterxml(nested, "//*[local-name()='b']/namespace::q"),
            text("urn:q2")
        );
        assert_eq!(filterxml(nested, "//*[count(namespace::*)=4]"), text("z"));
        // Namespace nodes come after their element and before its
        // attributes and children.
        let doc = "<a xmlns:p='urn:p'><b x='y'><c>1</c></b><d>2</d></a>";
        assert_eq!(
            filterxml_spill(doc, "//b/namespace::* | //b | //b/@x | //c"),
            vec![
                number(1.0),
                text(XML_NAMESPACE),
                text("urn:p"),
                text("y"),
                number(1.0)
            ]
        );
        assert_eq!(
            filterxml_spill(doc, "//b/namespace::p/following::*"),
            vec![number(1.0), number(2.0)]
        );
        assert_eq!(
            filterxml_spill(doc, "//d/namespace::p/preceding::*"),
            vec![number(1.0), number(1.0)]
        );
        assert_eq!(
            filterxml_spill(doc, "//c/namespace::p/ancestor::*"),
            vec![number(12.0), number(1.0), number(1.0)]
        );
        assert_eq!(
            filterxml(doc, "//c/namespace::*[name()='p']/.."),
            number(1.0)
        );
    }

    #[test]
    fn the_xml_prefix_is_bound_in_the_xpath() {
        // Namespaces in XML 1.0 §3: xml is bound by definition to the XML
        // namespace; no other prefix is bound for the XPath.
        assert_eq!(
            filterxml("<a xml:lang='en'>1</a>", "//@xml:lang"),
            text("en")
        );
        let two = "<r><a xml:lang='en'>1</a><a>2</a></r>";
        assert_eq!(filterxml(two, "//a[@xml:lang='en']"), number(1.0));
        assert_eq!(filterxml(two, "//a[not(@xml:lang)]"), number(2.0));
        assert_eq!(
            filterxml(
                "<r><s xml:space='preserve'> x </s><s>y</s></r>",
                "//s[@xml:space]"
            ),
            text("x")
        );
        assert_eq!(
            filterxml_spill("<a xml:lang='en' b='1' xml:space='default'/>", "/a/@xml:*"),
            vec![text("en"), text("default")]
        );
        assert_eq!(
            filterxml("<xml:a xml:lang='en'>1</xml:a>", "/xml:a[@xml:lang]"),
            number(1.0)
        );
        for xpath in [
            "//@p:x",
            "//p:*",
            "//@xml:lang()",
            "//xml:lang::a",
            "//@xml : lang",
            "//@xml:",
            "//@xml:1",
            "//@xml:a:b",
        ] {
            assert_value_error("<a xmlns:p='urn:p' p:x='1' xml:lang='en'/>", xpath);
        }
    }

    #[test]
    fn xml_must_be_well_formed() {
        // XML 1.0 and Namespaces in XML 1.0: each of these is a fatal error,
        // so #VALUE!.
        for xml in [
            // Attributes.
            "<a x='1' x='2'>1</a>",
            "<a x='<'>1</a>",
            "<a x=1>1</a>",
            "<a x='1'y='2'>1</a>",
            "<a x>1</a>",
            "<a x='&e;'>1</a>",
            "<a x='&'>1</a>",
            // Content.
            "<a>]]></a>",
            "<a>&e;</a>",
            "<a>&#X41;</a>",
            "<a>&#x;</a>",
            "<a>&am p;</a>",
            "<a/>x",
            "x<a/>",
            "<a/><b/>",
            "<a>",
            "</a>",
            "<a></b>",
            "<a></ a>",
            "< a></a>",
            "<1a/>",
            "<![CDATA[x]]><a/>",
            "<a/>&amp;",
            // Comments and processing instructions.
            "<!--a--b--><a/>",
            "<!--a---><a/>",
            "<a><?xml x?></a>",
            "<?XML x?><a/>",
            "<a><?1 x?></a>",
            // The XML declaration: at the very start, version first.
            " <?xml version='1.0'?><a/>",
            "<a/><?xml version='1.0'?>",
            "<?xml version='1.0'?><?xml version='1.0'?><a/>",
            "<?xml version='2.0'?><a/>",
            "<?xml encoding='UTF-8'?><a/>",
            "<?xml version='1.0' standalone='maybe'?><a/>",
            "<?xml version='1.0' standalone='yes' encoding='UTF-8'?><a/>",
            // The document type declaration: after the start of the prolog,
            // SYSTEM identifiers only.
            "<!DOCTYPE a><a/>",
            "<a><!DOCTYPE a></a>",
            "<?xml version='1.0'?><!DOCTYPE a PUBLIC 'x' 'y'><a/>",
            "<?xml version='1.0'?><!doctype a><a/>",
            // Namespaces.
            "<p:a/>",
            "<a p:x='1'/>",
            "<a xmlns:p=''/>",
            "<a xmlns:p='urn:u' xmlns:q='urn:u' p:x='1' q:x='2'/>",
            "<a xmlns:xml='urn:x'/>",
            "<a xmlns:p='http://www.w3.org/XML/1998/namespace'/>",
            "<a xmlns='http://www.w3.org/XML/1998/namespace'/>",
            "<a xmlns:xmlns='urn:x'/>",
            "<xmlns:a/>",
            "<a:b:c/>",
            "<a xmlns:p='urn:p'/><p:b/>",
        ] {
            assert_value_error(xml, "//*");
        }
        // A byte order mark belongs to an encoded file, not to text.
        assert_eq!(
            error_kind(eval("=FILTERXML(UNICHAR(65279)&\"<a>1</a>\",\"//a\")")),
            ExcelErrorKind::Value
        );
        for xml in [
            "<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><a>1</a>",
            "<?xml version='1.0'?><!DOCTYPE a SYSTEM 'a.dtd' [<!ELEMENT a ANY>]><a>1</a>",
            "<!--c--> <!DOCTYPE a><?pi x?><a x='>'>1</a><!--d--><?pi?>",
            "<a  x = '1' >1</a >",
            "<a x='&lt;&#62;'>1<!----><?xml-stylesheet x?></a>",
            "<p:a xmlns:p='urn:p' p:x='1' x='2'>1</p:a>",
            "<a xmlns:p='urn:u' p:x='1' x='2'>1</a>",
            "<a xmlns:xml='http://www.w3.org/XML/1998/namespace'>1</a>",
        ] {
            assert_eq!(filterxml(xml, "//*[.!='']"), number(1.0), "{xml}");
        }
        // `]]>` may be written with a reference.
        assert_eq!(filterxml("<a>]]&gt;</a>", "/a"), text("]]>"));
    }

    /// The nodes `xpath` selects in `document`, and the work it took.
    fn select_counting_work(document: &Document, xpath: &str) -> (Vec<usize>, u64) {
        let xpath = xpath::compile(xpath::tokenize(xpath).unwrap()).unwrap();
        let (nodes, work) = xpath::select_counting_work(document, &xpath);
        (nodes.unwrap(), work)
    }

    /// The split idiom's XML for `items`.
    fn split_xml(items: impl Iterator<Item = String>) -> String {
        format!(
            "<t><s>{}</s></t>",
            items.collect::<Vec<_>>().join("</s><s>")
        )
    }

    #[test]
    fn namespace_nodes_are_made_only_where_the_axis_reaches() {
        // The finding's repro: 1,100 declarations on the root and 4,000
        // children, 32,500 characters, one namespace node selected.
        let declarations: String = (1..=1100).map(|i| format!(" xmlns:p{i}='u'")).collect();
        let xml = format!("<r{declarations}>{}</r>", "<a/>".repeat(4000));
        assert_eq!(xml.len(), 32_500);
        let start = Instant::now();
        let document = Document::parse(&xml).unwrap();
        let (nodes, _) = select_counting_work(&document, "/r/namespace::p1");
        assert_eq!(nodes.len(), 1);
        assert_eq!(document.string_value(nodes[0]), "u");
        // The root's 1,100 prefixes and xml.
        assert_eq!(document.namespace_nodes_made(), 1101);
        let (nodes, _) = select_counting_work(&document, "/r/a[position() <= 3]/namespace::*");
        assert_eq!(nodes.len(), 3 * 1101);
        assert_eq!(document.namespace_nodes_made(), 4 * 1101);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        // Through FILTERXML.
        assert_eq!(
            super::select_node_texts(&xml, "/r/namespace::p1100", None),
            Ok(vec!["u".to_string()])
        );
    }

    #[test]
    fn steps_take_work_in_proportion_to_the_document() {
        // The finding's shapes: the following or preceding nodes of every
        // node, and nested descendants and ancestors. A step selects each
        // node once and walks no part of an axis twice, so twice the
        // document is about twice the work, not four times.
        let list = |n: usize| split_xml((0..n).map(|i| (i % 10).to_string()));
        let deep = |n: usize| format!("{}1{}", "<a>".repeat(n), "</a>".repeat(n));
        // The XML of a size, an XPath, and how many nodes it selects.
        type Case<'a> = (&'a dyn Fn(usize) -> String, &'a str, fn(usize) -> usize);
        let cases: [Case; 4] = [
            (&list, "//node()/following::node()", |n| 2 * n - 2),
            (&list, "//node()/preceding::node()", |n| 2 * n - 2),
            (&deep, "//a//a", |n| n - 1),
            (&deep, "//a/ancestor::a", |n| n - 1),
        ];
        for (xml, xpath, selected) in cases {
            let work = |n: usize| {
                let (nodes, work) = select_counting_work(&Document::parse(&xml(n)).unwrap(), xpath);
                assert_eq!(nodes.len(), selected(n), "{xpath}");
                work
            };
            let (small, large) = (work(500), work(1000));
            assert!(large < 3 * small, "{xpath}: {small} then {large}");
            // The finding's sizes (32,007 and 32,761 characters).
            let n = if xpath.contains("node()") { 4000 } else { 4680 };
            let start = Instant::now();
            work(n);
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "{xpath}: {:?}",
                start.elapsed()
            );
        }
    }

    #[test]
    fn comparisons_use_the_string_values_in_the_document() {
        // The finding's repros of the dedupe idiom, each node against all
        // nodes before or after it: 3,200 distinct and 4,000 equal items
        // (about 32,000 characters), well within a second.
        let distinct = split_xml((0..3200).map(|i| format!("{i:03}")));
        let equal = split_xml((0..4000).map(|_| "1".to_string()));
        for xpath in [
            "//s[not(.=preceding::*)]",
            "//s[not(.=following::*)]",
            "//s[not(preceding::*=.)]",
            "//s[not(.=preceding-sibling::s)]",
        ] {
            for (xml, kept) in [(&distinct, 3200), (&equal, 1)] {
                let start = Instant::now();
                let document = Document::parse(xml).unwrap();
                assert_eq!(
                    select_counting_work(&document, xpath).0.len(),
                    kept,
                    "{xpath}"
                );
                assert!(
                    start.elapsed() < Duration::from_secs(1),
                    "{xpath}: {:?}",
                    start.elapsed()
                );
                // Each item's text is one text node: nothing is copied.
                assert_eq!(document.texts_joined(), 0);
            }
        }
        // 1,500 empty items after a 1,000-deep chain around 12,000
        // characters: every item meets each chain element, whose
        // string-value is the whole text, borrowed rather than rebuilt.
        let nested = format!(
            "<r>{}{}{}{}</r>",
            "<x>".repeat(1000),
            "y".repeat(12_000),
            "</x>".repeat(1000),
            "<s/>".repeat(1500)
        );
        assert_eq!(nested.len(), 25_007);
        let start = Instant::now();
        let document = Document::parse(&nested).unwrap();
        assert_eq!(
            select_counting_work(&document, "//s[not(.=preceding::*)]")
                .0
                .len(),
            1
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(document.texts_joined(), 0);
        // Text in several text nodes is joined once, however often compared.
        let split_text = split_xml((0..200).map(|i| format!("{}<b/>{}", i % 3, i % 2)));
        let document = Document::parse(&split_text).unwrap();
        assert_eq!(
            select_counting_work(&document, "//s[not(.=preceding::s)]")
                .0
                .len(),
            6
        );
        assert_eq!(document.texts_joined(), 200);
    }

    #[test]
    fn a_cancelled_evaluation_stops() {
        let xml = split_xml((0..3200).map(|i| format!("{i:03}")));
        let token = CancelToken::new();
        assert_eq!(
            super::select_node_texts(&xml, "//s[not(.=preceding::*)][last()]", Some(&token)),
            Ok(vec!["3199".to_string()])
        );
        token.cancel();
        assert_eq!(
            super::select_node_texts(&xml, "//s[not(.=preceding::*)]", Some(&token)),
            Err(Failure::Cancelled)
        );
        // Deep XPaths run on a thread of their own, with the token.
        let deep = format!("{}//s{}", "(".repeat(40), ")".repeat(40));
        assert_eq!(
            super::select_node_texts(&xml, &deep, Some(&token)),
            Err(Failure::Cancelled)
        );
    }
}
