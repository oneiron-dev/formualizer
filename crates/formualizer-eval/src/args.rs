use crate::traits::ArgumentHandle;
// Note: Validator no longer depends on EvaluationContext; keep it engine-agnostic.
use formualizer_common::{ArgKind, ExcelError, ExcelErrorKind, LiteralValue};
use smallvec::{SmallVec, smallvec};
use std::borrow::Cow;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ShapeKind {
    Scalar,
    Range,
    Array,
}

pub use formualizer_common::CoercionPolicy;

#[derive(Clone, Debug)]
pub struct ArgSchema {
    pub kinds: SmallVec<[ArgKind; 2]>,
    pub required: bool,
    pub by_ref: bool,
    pub shape: ShapeKind,
    pub coercion: CoercionPolicy,
    pub max: Option<usize>,
    pub repeating: Option<usize>,
    pub default: Option<LiteralValue>,
}

impl ArgSchema {
    pub fn any() -> Self {
        Self {
            kinds: smallvec![ArgKind::Any],
            required: true,
            by_ref: false,
            shape: ShapeKind::Scalar,
            coercion: CoercionPolicy::None,
            max: None,
            repeating: None,
            default: None,
        }
    }

    pub fn number_lenient_scalar() -> Self {
        Self {
            kinds: smallvec![ArgKind::Number],
            required: true,
            by_ref: false,
            shape: ShapeKind::Scalar,
            coercion: CoercionPolicy::NumberLenientText,
            max: None,
            repeating: None,
            default: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum CriteriaPredicate {
    Eq(LiteralValue),
    Ne(LiteralValue),
    Gt(f64),
    Ge(f64),
    Lt(f64),
    Le(f64),
    TextLike {
        pattern: String,
        case_insensitive: bool,
    },
    IsBlank,
    IsNumber,
    IsText,
    IsLogical,
}

#[derive(Debug)]
pub enum PreparedArg<'a> {
    Value(Cow<'a, LiteralValue>),
    Range(crate::engine::range_view::RangeView<'a>),
    Reference(formualizer_parse::parser::ReferenceType),
    Predicate(CriteriaPredicate),
}

pub struct PreparedArgs<'a> {
    pub items: Vec<PreparedArg<'a>>,
}

#[derive(Default)]
pub struct ValidationOptions {
    pub warn_only: bool,
    /// Minimum number of arguments the function requires.  When non-zero,
    /// `validate_and_prepare` rejects calls with fewer arguments before any
    /// per-argument validation runs, preventing out-of-bounds panics in
    /// `eval` implementations.
    pub min_args: usize,
}

// Legacy adapter removed in clean break.

/// A criterion operand that Excel reads as a number (`"5"`, `" 1e3 "`,
/// `"90%"`). Rust spellings such as `inf` or `NaN` stay text.
fn criteria_number(text: &str) -> Option<f64> {
    if text
        .chars()
        .any(|c| c.is_alphabetic() && !matches!(c, 'e' | 'E'))
    {
        return None;
    }
    crate::locale::Locale::invariant()
        .parse_number_invariant(text)
        .filter(|n| n.is_finite())
}

pub fn parse_criteria(v: &LiteralValue) -> Result<CriteriaPredicate, ExcelError> {
    match v {
        LiteralValue::Text(s) => {
            let s_trim = s.trim();

            let unquote = |t: &str| -> String {
                let t = t.trim();
                if let Some(inner) = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                    inner.replace("\"\"", "\"")
                } else {
                    t.to_string()
                }
            };

            // Operators: >=, <=, <>, >, <, =
            let ops = [">=", "<=", "<>", ">", "<", "="];
            for op in ops.iter() {
                if let Some(rhs) = s_trim.strip_prefix(op) {
                    let rhs_trim = rhs.trim();
                    // Try numeric parse for comparisons
                    if let Some(n) = criteria_number(rhs_trim) {
                        return Ok(match *op {
                            ">=" => CriteriaPredicate::Ge(n),
                            "<=" => CriteriaPredicate::Le(n),
                            ">" => CriteriaPredicate::Gt(n),
                            "<" => CriteriaPredicate::Lt(n),
                            "=" => CriteriaPredicate::Eq(LiteralValue::Number(n)),
                            "<>" => CriteriaPredicate::Ne(LiteralValue::Number(n)),
                            _ => unreachable!(),
                        });
                    }
                    // Fallback: non-numeric equals/neq text (support Excel-style quoted strings: ="aa")
                    let lit = LiteralValue::Text(unquote(rhs_trim));
                    return Ok(match *op {
                        "=" => CriteriaPredicate::Eq(lit),
                        "<>" => CriteriaPredicate::Ne(lit),
                        ">=" | "<=" | ">" | "<" => {
                            // Non-numeric compare: not fully supported; degrade to equality on full expression
                            CriteriaPredicate::Eq(LiteralValue::Text(s_trim.to_string()))
                        }
                        _ => unreachable!(),
                    });
                }
            }

            let plain = unquote(s_trim);

            // Wildcards * or ? => TextLike
            if plain.contains('*') || plain.contains('?') {
                return Ok(CriteriaPredicate::TextLike {
                    pattern: plain,
                    case_insensitive: true,
                });
            }
            // Booleans TRUE/FALSE
            let lower = plain.to_ascii_lowercase();
            if lower == "true" {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Boolean(true)));
            } else if lower == "false" {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Boolean(false)));
            }
            // A number written as text ("111111") is a numeric criterion, as
            // if written "=111111".
            if let Some(n) = criteria_number(&plain) {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Number(n)));
            }
            // Plain text equality
            Ok(CriteriaPredicate::Eq(LiteralValue::Text(plain)))
        }
        LiteralValue::Empty => Ok(CriteriaPredicate::IsBlank),
        LiteralValue::Number(n) => Ok(CriteriaPredicate::Eq(LiteralValue::Number(*n))),
        // Normalize integer criteria to Number for Excel-style numeric coercions
        // (e.g. blank == 0, numeric text == number, etc.)
        LiteralValue::Int(i) => Ok(CriteriaPredicate::Eq(LiteralValue::Number(*i as f64))),
        LiteralValue::Boolean(b) => Ok(CriteriaPredicate::Eq(LiteralValue::Boolean(*b))),
        // An error criterion counts the cells holding that error.
        LiteralValue::Error(e) => Ok(CriteriaPredicate::Eq(LiteralValue::Error(e.clone()))),
        LiteralValue::Array(arr) => {
            // Treat 1x1 array literals as scalars for criteria parsing
            if arr.len() == 1 && arr.first().map(|r| r.len()).unwrap_or(0) == 1 {
                parse_criteria(&arr[0][0])
            } else {
                Ok(CriteriaPredicate::Eq(LiteralValue::Array(arr.clone())))
            }
        }
        other => Ok(CriteriaPredicate::Eq(other.clone())),
    }
}

pub fn validate_and_prepare<'a, 'b>(
    args: &'a [ArgumentHandle<'a, 'b>],
    schema: &[ArgSchema],
    options: ValidationOptions,
) -> Result<PreparedArgs<'a>, ExcelError> {
    // Minimum arity — reject too-few arguments before per-arg validation so
    // that individual `eval` implementations cannot panic on indexing.
    if options.min_args > 0 && args.len() < options.min_args {
        if options.warn_only {
            return Ok(PreparedArgs { items: Vec::new() });
        }
        return Err(ExcelError::new(ExcelErrorKind::Value).with_message(format!(
            "Too few arguments: expected at least {}, got {}",
            options.min_args,
            args.len()
        )));
    }

    // Arity: simple rule – if schema.len() == 1, allow variadic repetition; else match up to schema.len()
    if schema.is_empty() {
        return Ok(PreparedArgs { items: Vec::new() });
    }

    let mut items: Vec<PreparedArg<'a>> = Vec::with_capacity(args.len());
    for (idx, arg) in args.iter().enumerate() {
        let spec = if schema.len() == 1 {
            &schema[0]
        } else if idx < schema.len() {
            &schema[idx]
        } else {
            // Attempt to find a repeating spec (e.g., variadic tail like CHOOSE, SUM, etc.)
            if let Some(rep_spec) = schema.iter().find(|s| s.repeating.is_some()) {
                rep_spec
            } else if options.warn_only {
                continue;
            } else {
                return Err(
                    ExcelError::new(ExcelErrorKind::Value).with_message("Too many arguments")
                );
            }
        };

        // By-ref argument: require a reference (AST literal or function-returned)
        if spec.by_ref {
            match arg.as_reference_or_eval() {
                Ok(r) => {
                    items.push(PreparedArg::Reference(r));
                    continue;
                }
                Err(e) => {
                    if options.warn_only {
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        // Criteria policy: parse into predicate
        if matches!(spec.coercion, CoercionPolicy::Criteria) {
            let v = arg.value()?.into_literal();
            match parse_criteria(&v) {
                Ok(pred) => {
                    items.push(PreparedArg::Predicate(pred));
                    continue;
                }
                Err(e) => {
                    if options.warn_only {
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        // Shape handling
        match spec.shape {
            ShapeKind::Scalar => {
                // Collapse to scalar if needed (top-left for arrays)
                match arg.value() {
                    Ok(cv) => {
                        let v: Cow<'_, LiteralValue> = match cv {
                            crate::traits::CalcValue::Scalar(LiteralValue::Array(arr)) => {
                                let tl = arr
                                    .first()
                                    .and_then(|row| row.first())
                                    .cloned()
                                    .unwrap_or(LiteralValue::Empty);
                                Cow::Owned(tl)
                            }
                            crate::traits::CalcValue::Range(rv) => Cow::Owned(rv.get_cell(0, 0)),
                            crate::traits::CalcValue::Scalar(s)
                            | crate::traits::CalcValue::AnnotatedScalar(s, _) => Cow::Owned(s),
                            crate::traits::CalcValue::Callable(_) => {
                                Cow::Owned(LiteralValue::Error(
                                    ExcelError::new(ExcelErrorKind::Calc)
                                        .with_message("LAMBDA value must be invoked"),
                                ))
                            }
                        };
                        // Apply coercion policy to Value shapes when applicable
                        let coerced = match spec.coercion {
                            CoercionPolicy::None => v,
                            CoercionPolicy::NumberStrict => {
                                match crate::coercion::to_number_strict(v.as_ref()) {
                                    Ok(n) => Cow::Owned(LiteralValue::Number(n)),
                                    Err(e) => {
                                        if options.warn_only {
                                            v
                                        } else {
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                            CoercionPolicy::NumberLenientText => {
                                // Excel's lenient text coercion also reads
                                // date/time text ("3/15/2021", "10:30 AM").
                                match crate::coercion::to_number_lenient(v.as_ref()).or_else(
                                    |error| match v.as_ref() {
                                        LiteralValue::Text(text) => {
                                            formualizer_common::parse_excel_datetime_text_to_serial_in_year_for(
                                                arg.date_system(),
                                                text,
                                                Some(arg.current_year()),
                                            )
                                            .ok_or(error)
                                        }
                                        _ => Err(error),
                                    },
                                ) {
                                    Ok(n) => Cow::Owned(LiteralValue::Number(n)),
                                    Err(e) => {
                                        if options.warn_only {
                                            v
                                        } else {
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                            CoercionPolicy::Logical => {
                                match crate::coercion::to_logical(v.as_ref()) {
                                    Ok(b) => Cow::Owned(LiteralValue::Boolean(b)),
                                    Err(e) => {
                                        if options.warn_only {
                                            v
                                        } else {
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                            CoercionPolicy::Criteria => v, // handled per-function currently
                            CoercionPolicy::DateTimeSerial => {
                                match crate::coercion::to_datetime_serial(v.as_ref()) {
                                    Ok(n) => Cow::Owned(LiteralValue::Number(n)),
                                    Err(e) => {
                                        if options.warn_only {
                                            v
                                        } else {
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                        };
                        items.push(PreparedArg::Value(coerced))
                    }
                    Err(e) => items.push(PreparedArg::Value(Cow::Owned(LiteralValue::Error(e)))),
                }
            }
            ShapeKind::Range | ShapeKind::Array => match arg.resolve_once() {
                Ok(crate::traits::ResolvedArgument::Range(range))
                | Ok(crate::traits::ResolvedArgument::Value(crate::traits::CalcValue::Range(
                    range,
                ))) => items.push(PreparedArg::Range(range)),
                Ok(crate::traits::ResolvedArgument::Value(value)) => {
                    // Excel-compatible: range-accepting functions also accept scalars.
                    items.push(PreparedArg::Value(Cow::Owned(value.into_literal())))
                }
                Ok(crate::traits::ResolvedArgument::ReferenceError(error)) => {
                    items.push(PreparedArg::Value(Cow::Owned(LiteralValue::Error(error))))
                }
                Err(error) => {
                    items.push(PreparedArg::Value(Cow::Owned(LiteralValue::Error(error))))
                }
            },
        }
    }

    Ok(PreparedArgs { items })
}

#[cfg(test)]
mod criteria_tests {
    use super::*;
    use crate::builtins::criteria_match;

    fn text(s: &str) -> LiteralValue {
        LiteralValue::Text(s.into())
    }

    #[test]
    fn numeric_text_criterion_matches_numbers() {
        // SUMIF(A:A,"111111",B:B) sums rows holding the number 111111.
        for criterion in ["111111", " 111111 ", "=111111"] {
            let pred = parse_criteria(&text(criterion)).unwrap();
            assert!(
                criteria_match(&pred, &LiteralValue::Number(111111.0)),
                "{criterion}"
            );
            assert!(criteria_match(&pred, &text("111111")), "{criterion}");
            assert!(
                !criteria_match(&pred, &LiteralValue::Number(11111.0)),
                "{criterion}"
            );
        }
        let pct = parse_criteria(&text(">=50%")).unwrap();
        assert!(criteria_match(&pct, &LiteralValue::Number(0.5)));
        assert!(!criteria_match(&pct, &LiteralValue::Number(0.4)));
    }

    #[test]
    fn number_like_words_stay_text() {
        for word in ["inf", "NaN", "infinity"] {
            let pred = parse_criteria(&text(word)).unwrap();
            assert!(criteria_match(&pred, &text(word)), "{word}");
            assert!(
                !criteria_match(&pred, &LiteralValue::Number(f64::INFINITY)),
                "{word}"
            );
        }
    }
}
