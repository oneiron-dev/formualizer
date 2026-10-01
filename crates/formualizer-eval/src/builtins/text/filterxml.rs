//! FILTERXML: XPath 1.0 over XML text, as Excel for Windows evaluates it
//! with MSXML.

use super::super::utils::{ARG_ANY_TWO, collapse_if_scalar};
use super::scalar_text_value;
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;
use sxd_document::dom::{ChildOfElement, ChildOfRoot, Text};
use sxd_xpath::{Context, Factory, Value};

/// The longest XPath FILTERXML accepts.
const MAX_XPATH_CHARS: usize = 1024;

/// The namespace of the predeclared `xml:` prefix (`xml:space`).
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// XML white space: space, tab, carriage return and line feed.
fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn to_text(arg: &ArgumentHandle<'_, '_>) -> Result<String, ExcelError> {
    Ok(match scalar_text_value(arg)? {
        LiteralValue::Text(s) => s,
        LiteralValue::Empty => String::new(),
        LiteralValue::Boolean(b) => if b { "TRUE" } else { "FALSE" }.into(),
        LiteralValue::Int(i) => i.to_string(),
        LiteralValue::Number(n) => {
            let s = n.to_string();
            s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
        }
        LiteralValue::Error(e) => return Err(e),
        other => other.to_string(),
    })
}

/// Whether `xpath` uses a namespace prefix (`p:name`, `p:*`, `p:f()`) outside
/// its string literals. FILTERXML declares no prefixes for the XPath, so
/// MSXML rejects any; a single `:` only ever separates a prefix (`::` is an
/// axis).
fn has_prefixed_name(xpath: &str) -> bool {
    let mut quote = None;
    let mut chars = xpath.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                ':' if chars.peek() == Some(&':') => {
                    chars.next();
                }
                ':' => return true,
                _ => {}
            },
        }
    }
    false
}

/// Make a run of adjacent text nodes one node (the parser splits text at
/// entity references and CDATA sections), or drop it when it is only white
/// space and white space is not preserved.
fn merge_text_run(run: &mut Vec<Text<'_>>, preserve: bool) {
    let text: String = run.iter().map(|node| node.text()).collect();
    if !preserve && text.chars().all(is_xml_space) {
        run.iter().for_each(|node| node.remove_from_parent());
    } else if let [first, rest @ ..] = run.as_slice()
        && !rest.is_empty()
    {
        first.set_text(&text);
        rest.iter().for_each(|node| node.remove_from_parent());
    }
    run.clear();
}

/// The text of each node `xpath` selects in `xml`, in document order; `None`
/// when the XML or the XPath is invalid, the XPath evaluates to something
/// other than nodes (`count(//a)`) or selects no node.
///
/// MSXML loads the XML without its white-space-only text nodes (white space is
/// not preserved unless `xml:space="preserve"`), and a node's text is trimmed
/// of leading and trailing white space; XPath tests see the untrimmed values.
fn select_node_texts(xml: &str, xpath: &str) -> Option<Vec<String>> {
    if xpath.chars().count() > MAX_XPATH_CHARS || has_prefixed_name(xpath) {
        return None;
    }
    let package = sxd_document::parser::parse(xml).ok()?;
    let document = package.as_document();
    let mut elements: Vec<_> = document
        .root()
        .children()
        .into_iter()
        .filter_map(|child| match child {
            ChildOfRoot::Element(element) => Some((element, false)),
            _ => None,
        })
        .collect();
    let mut run = Vec::new();
    while let Some((element, inherited)) = elements.pop() {
        let preserve = match element.attribute_value((XML_NAMESPACE, "space")) {
            Some("preserve") => true,
            Some("default") => false,
            _ => inherited,
        };
        for child in element.children() {
            match child {
                ChildOfElement::Text(text) => run.push(text),
                other => {
                    merge_text_run(&mut run, preserve);
                    if let ChildOfElement::Element(child) = other {
                        elements.push((child, preserve));
                    }
                }
            }
        }
        merge_text_run(&mut run, preserve);
    }
    let xpath = Factory::new().build(xpath).ok()??;
    let Value::Nodeset(nodes) = xpath.evaluate(&Context::new(), document.root()).ok()? else {
        return None;
    };
    let texts: Vec<String> = nodes
        .document_order()
        .iter()
        .map(|node| node.string_value().trim_matches(is_xml_space).to_string())
        .collect();
    (!texts.is_empty()).then_some(texts)
}

#[derive(Debug)]
pub struct FilterXmlFn;
/// Returns the values of the nodes an XPath 1.0 expression selects in XML text.
///
/// # Remarks
/// - Invalid XML, an invalid XPath (or one over 1024 characters, or with a
///   namespace prefix), an XPath that evaluates to a number, string or boolean
///   instead of nodes, and an XPath that selects no node all return `#VALUE!`.
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
        let Some(texts) = select_node_texts(&xml, &xpath) else {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
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
    use crate::engine::{Engine, EvalConfig};
    use crate::test_workbook::TestWorkbook;
    use formualizer_common::{ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::parse;

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
}
