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
    /// `"<>"` with a wildcard pattern: every cell that `TextLike` would not match.
    NotTextLike {
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
/// `"90%"`). Rust spellings such as `inf` or `NaN` stay text: numeric text is
/// only ever a finite number ([`crate::locale::parse_finite_number`]).
fn criteria_number(text: &str) -> Option<f64> {
    crate::locale::Locale::invariant().parse_number_invariant(text)
}

/// A criterion operand that Excel reads as a number, a date or a time
/// (`"5"`, `"3/1/2021"`, `"May 2, 1900"`, `"12:30"`). COUNTIF, SUMIF and the
/// other criteria functions are not type-specific for numbers and dates: date
/// and time text is its serial, read as the function call reads date text in
/// its number arguments, in the workbook's date system with a year-less date
/// in the clock's year ([`crate::coercion::argument_date_text_serial`]).
fn criteria_serial(text: &str) -> Option<f64> {
    criteria_number(text).or_else(|| crate::coercion::argument_date_text_serial(text))
}

/// An Excel error value written as criteria text (`#N/A`, `#DIV/0!`).
fn criteria_error(text: &str) -> Option<ExcelErrorKind> {
    ExcelErrorKind::try_parse(text).filter(|kind| {
        matches!(
            kind,
            ExcelErrorKind::Null
                | ExcelErrorKind::Ref
                | ExcelErrorKind::Name
                | ExcelErrorKind::Value
                | ExcelErrorKind::Div
                | ExcelErrorKind::Na
                | ExcelErrorKind::Num
                | ExcelErrorKind::Spill
                | ExcelErrorKind::Calc
        )
    })
}

/// Parse a criteria value (`">=5"`, `"<5/3/2011"`, `"a*"`, `7`) into a
/// predicate. Operands that read as numbers, dates or times are numeric.
pub fn parse_criteria(v: &LiteralValue) -> Result<CriteriaPredicate, ExcelError> {
    match v {
        LiteralValue::Text(s) => {
            let s_trim = s.trim();

            // Text criteria keep their spaces: "="&A9 with A9 = "   2E" matches
            // only "   2E". Numbers are read without the spaces around them.
            let unquote = |t: &str| -> String {
                let trimmed = t.trim();
                if let Some(inner) = trimmed.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                    inner.replace("\"\"", "\"")
                } else {
                    t.to_string()
                }
            };

            // Operators: >=, <=, <>, >, <, =
            let ops = [">=", "<=", "<>", ">", "<", "="];
            for op in ops.iter() {
                if let Some(rhs) = s.trim_start().strip_prefix(op) {
                    let rhs_trim = rhs.trim();
                    // Try numeric parse for comparisons. Like the cells it is
                    // compared with, a criterion number, date or time ignores
                    // only the spaces around it: "=5"&CHAR(10) is text.
                    if let Some(n) = criteria_serial(rhs.trim_matches(' ')) {
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
                    // An error spelled out ("<>#N/A") compares against error cells.
                    let lit = match criteria_error(rhs_trim) {
                        Some(kind) if matches!(*op, "=" | "<>") => {
                            LiteralValue::Error(ExcelError::new(kind))
                        }
                        _ => LiteralValue::Text(unquote(rhs)),
                    };
                    // Wildcards apply after "=" and "<>" as they do with no operator.
                    if let LiteralValue::Text(t) = &lit
                        && (t.contains('*') || t.contains('?'))
                    {
                        let (pattern, case_insensitive) = (t.clone(), true);
                        match *op {
                            "=" => {
                                return Ok(CriteriaPredicate::TextLike {
                                    pattern,
                                    case_insensitive,
                                });
                            }
                            "<>" => {
                                return Ok(CriteriaPredicate::NotTextLike {
                                    pattern,
                                    case_insensitive,
                                });
                            }
                            _ => {}
                        }
                    }
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

            let plain = unquote(s);

            // Wildcards * or ? => TextLike
            if plain.contains('*') || plain.contains('?') {
                return Ok(CriteriaPredicate::TextLike {
                    pattern: plain,
                    case_insensitive: true,
                });
            }
            // Booleans TRUE/FALSE
            let lower = plain.trim().to_ascii_lowercase();
            if lower == "true" {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Boolean(true)));
            } else if lower == "false" {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Boolean(false)));
            }
            // A number written as text ("111111") is a numeric criterion, as
            // if written "=111111"; so is a date or a time ("3/1/2021"). Only
            // the spaces around it are ignored, as for the cells: "5"&CHAR(10)
            // is a text criterion that matches the text "5"&CHAR(10) and not
            // the number 5.
            if let Some(n) = criteria_serial(plain.trim_matches(' ')) {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Number(n)));
            }
            if let Some(kind) = criteria_error(plain.trim()) {
                return Ok(CriteriaPredicate::Eq(LiteralValue::Error(ExcelError::new(
                    kind,
                ))));
            }
            // Plain text equality
            Ok(CriteriaPredicate::Eq(LiteralValue::Text(plain)))
        }
        // A reference to an empty cell is the criterion 0.
        LiteralValue::Empty => Ok(CriteriaPredicate::Eq(LiteralValue::Number(0.0))),
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
    validate_and_prepare_reading(args, schema, options, |_| true)
}

/// [`validate_and_prepare`] that validates only the arguments `reads`
/// accepts (by index). An argument the function reads only as a reference
/// (ROWS's, INDEX's) is not read to validate it: reading every cell of
/// `A$1:A5` for `=ROWS(A$1:A5)` in A5 would be a read of the formula's own
/// cell that the formula never makes. The argument count is still checked.
pub fn validate_and_prepare_reading<'a, 'b>(
    args: &'a [ArgumentHandle<'a, 'b>],
    schema: &[ArgSchema],
    options: ValidationOptions,
    reads: impl Fn(usize) -> bool,
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
        if !reads(idx) {
            continue;
        }

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
    fn criterion_numbers_ignore_only_surrounding_spaces() {
        // "5"&CHAR(10) is not numeric text, in a criterion as in a cell: it is
        // a text criterion that matches the cell holding that text and not
        // the number 5.
        for criterion in ["5\n", "=5\n", "\t5", "=\n5", "5\u{a0}"] {
            let pred = parse_criteria(&text(criterion)).unwrap();
            assert!(
                !criteria_match(&pred, &LiteralValue::Number(5.0)),
                "{criterion:?}"
            );
        }
        for (criterion, cell) in [("5\n", "5\n"), ("=5\n", "5\n"), ("\t5", "\t5")] {
            let pred = parse_criteria(&text(criterion)).unwrap();
            assert!(criteria_match(&pred, &text(cell)), "{criterion:?}");
        }
        // A numeric criterion does not read the cell text "5\n" as 5 either.
        for criterion in [text("5"), text("=5"), LiteralValue::Number(5.0)] {
            let pred = parse_criteria(&criterion).unwrap();
            assert!(!criteria_match(&pred, &text("5\n")), "{criterion:?}");
        }
        // ">4"&CHAR(10) has no number to compare a cell with.
        let gt = parse_criteria(&text(">4\n")).unwrap();
        assert!(!criteria_match(&gt, &LiteralValue::Number(5.0)));
        // The spaces around a criterion number are still ignored.
        let ge = parse_criteria(&text(">= 4 ")).unwrap();
        assert!(criteria_match(&ge, &LiteralValue::Number(5.0)));
    }

    #[test]
    fn date_and_time_text_criteria_are_numbers() {
        // COUNTIFS(B2:B7,"<5/3/2011") compares dates (Microsoft's COUNTIFS
        // example); "May 2, 1900" is the serial 123, so it matches 123, "123"
        // and "0123".
        let may_3_2011 = 40666.0;
        assert!(matches!(
            parse_criteria(&text("<5/3/2011")).unwrap(),
            CriteriaPredicate::Lt(n) if n == may_3_2011
        ));
        assert!(matches!(
            parse_criteria(&text(">= May 3, 2011")).unwrap(),
            CriteriaPredicate::Ge(n) if n == may_3_2011
        ));
        let pred = parse_criteria(&text("May 2, 1900")).unwrap();
        for cell in [LiteralValue::Number(123.0), text("123"), text("0123")] {
            assert!(criteria_match(&pred, &cell), "{cell:?}");
        }
        let noon = parse_criteria(&text("12:00")).unwrap();
        assert!(criteria_match(&noon, &LiteralValue::Number(0.5)));
        // Text that is no date stays a text criterion.
        let code = parse_criteria(&text("Q1-2021")).unwrap();
        assert!(matches!(code, CriteriaPredicate::Eq(LiteralValue::Text(_))));
        assert!(criteria_match(&code, &text("q1-2021")));
    }

    #[test]
    fn numeric_equality_matches_date_text_cells() {
        // SUMIFS(K:K,A:A,DATE(2021,3,1)) sums the rows whose A holds the text
        // "3-1-21" (M/d/yy, en-US) as well as the date itself.
        let march_1_2021 = LiteralValue::Number(44256.0);
        let pred = parse_criteria(&march_1_2021).unwrap();
        for cell in [
            text("3-1-21"),
            text("3/1/2021"),
            text(" Mar 1, 2021 "),
            text("2021-03-01"),
        ] {
            assert!(criteria_match(&pred, &cell), "{cell:?}");
        }
        for cell in [text("3-2-21"), text("1-3-21"), text("gage")] {
            assert!(!criteria_match(&pred, &cell), "{cell:?}");
        }
        let other = parse_criteria(&text("<>3/1/2021")).unwrap();
        assert!(!criteria_match(&other, &text("3-1-21")));
        assert!(criteria_match(&other, &text("3-2-21")));
        // Ordered criteria compare numbers, not date text in a cell.
        let after = parse_criteria(&text(">44255")).unwrap();
        assert!(!criteria_match(&after, &text("3-1-21")));
        // A logical is still never a number.
        let one = parse_criteria(&LiteralValue::Number(1.0)).unwrap();
        assert!(!criteria_match(&one, &LiteralValue::Boolean(true)));
        // As for numbers, only the spaces around date text are ignored: a
        // line feed leaves it text, in the cell and in the criterion.
        assert!(!criteria_match(&pred, &text("3-1-21\n")));
        let line_fed = parse_criteria(&text("3/1/2021\n")).unwrap();
        assert!(matches!(
            line_fed,
            CriteriaPredicate::Eq(LiteralValue::Text(_))
        ));
        assert!(!criteria_match(&line_fed, &march_1_2021));
        assert!(criteria_match(&line_fed, &text("3/1/2021\n")));
    }

    #[test]
    fn date_and_time_text_criteria_equal_their_serials_to_15_digits() {
        // Midnight is 0, not 1E-13; noon is 0.5, not 0.5000000000005. Like
        // numbers, a serial equals a criterion that agrees to 15 significant
        // digits: "8:30" (0.35416666666666669) equals 0.354166666666667.
        let tiny = parse_criteria(&LiteralValue::Number(1e-13)).unwrap();
        assert!(!criteria_match(&tiny, &text("0:00")));
        let not_tiny = parse_criteria(&text("<>1E-13")).unwrap();
        assert!(criteria_match(&not_tiny, &text("0:00")));
        let zero = parse_criteria(&LiteralValue::Number(0.0)).unwrap();
        assert!(criteria_match(&zero, &text("0:00")));
        let near_noon = parse_criteria(&LiteralValue::Number(0.5000000000005)).unwrap();
        assert!(!criteria_match(&near_noon, &text("12:00")));
        let noon = parse_criteria(&LiteralValue::Number(0.5)).unwrap();
        assert!(criteria_match(&noon, &text("12:00")));
        let half_past_eight = parse_criteria(&LiteralValue::Number(0.354166666666667)).unwrap();
        assert!(criteria_match(&half_past_eight, &text("8:30")));
        let not_half_past_eight = parse_criteria(&text("<>0.354166666666667")).unwrap();
        assert!(!criteria_match(&not_half_past_eight, &text("8:30")));
    }

    #[test]
    fn time_criteria_keep_fractional_seconds() {
        // "12:00:00.5" is half a second after noon, in the criterion and in a
        // cell, so it neither equals nor falls below noon itself.
        let half_past = (43_200.0 + 0.5) / 86_400.0;
        let pred = parse_criteria(&text("12:00:00.5")).unwrap();
        assert!(matches!(
            pred,
            CriteriaPredicate::Eq(LiteralValue::Number(n)) if n == half_past
        ));
        assert!(!criteria_match(&pred, &LiteralValue::Number(0.5)));
        assert!(criteria_match(&pred, &text("12:00:00.5")));
        let below = parse_criteria(&text("<12:00:00.5")).unwrap();
        assert!(criteria_match(&below, &LiteralValue::Number(0.5)));
        let noon = parse_criteria(&LiteralValue::Number(0.5)).unwrap();
        assert!(!criteria_match(&noon, &text("12:00:00.5")));
    }

    #[test]
    fn date_text_before_1900_stays_a_text_criterion() {
        // DATEVALUE reads date text from January 1, 1900; "12/31/1899" is
        // text, so it neither matches serial 0 nor is matched by 0.
        let pred = parse_criteria(&text("12/31/1899")).unwrap();
        assert!(matches!(pred, CriteriaPredicate::Eq(LiteralValue::Text(_))));
        assert!(!criteria_match(&pred, &LiteralValue::Number(0.0)));
        assert!(criteria_match(&pred, &text("12/31/1899")));
        let zero = parse_criteria(&LiteralValue::Number(0.0)).unwrap();
        assert!(!criteria_match(&zero, &text("12/31/1899")));
        let one = parse_criteria(&LiteralValue::Number(1.0)).unwrap();
        assert!(criteria_match(&one, &text("1/1/1900")));
    }

    #[test]
    fn criteria_dates_follow_the_date_system_and_clock_year() {
        // Criteria read date text in the function call's date context: here
        // a 1904 workbook whose clock is in 2021.
        {
            let _call = crate::coercion::enter_argument_date_context(
                crate::engine::DateSystem::Excel1904,
                Some(2021),
            );
            let pred = parse_criteria(&text("3/1/2021")).unwrap();
            assert!(matches!(
                pred,
                CriteriaPredicate::Eq(LiteralValue::Number(n)) if n == 44256.0 - 1462.0
            ));
            assert!(criteria_match(&pred, &text("Mar 1")));
            assert!(criteria_match(
                &pred,
                &LiteralValue::Number(44256.0 - 1462.0)
            ));
        }
        // Outside a call (no clock year), year-less date text is not a date.
        assert!(matches!(
            parse_criteria(&text("Mar 1")).unwrap(),
            CriteriaPredicate::Eq(LiteralValue::Text(_))
        ));
    }

    #[test]
    fn eq_and_ne_criteria_apply_wildcards() {
        let ne = parse_criteria(&text("<>*approval")).unwrap();
        let eq = parse_criteria(&text("=*approval")).unwrap();
        for (value, matches_pattern) in [
            (text("In Approval"), true),
            (text("IN APPROVAL"), true),
            (text("In Progress"), false),
            (LiteralValue::Number(5.0), false),
            (LiteralValue::Boolean(true), false),
            (LiteralValue::Empty, false),
        ] {
            assert_eq!(criteria_match(&eq, &value), matches_pattern, "{value:?}");
            assert_eq!(criteria_match(&ne, &value), !matches_pattern, "{value:?}");
        }
        let ne_one_char = parse_criteria(&text("<>?")).unwrap();
        assert!(!criteria_match(&ne_one_char, &text("x")));
        assert!(criteria_match(&ne_one_char, &text("xy")));
        // Without a wildcard "<>" still compares the whole value.
        let ne_literal = parse_criteria(&text("<>in approval")).unwrap();
        assert!(!criteria_match(&ne_literal, &text("In Approval")));
        assert!(criteria_match(&ne_literal, &text("In Progress")));
        assert!(criteria_match(&ne_literal, &LiteralValue::Empty));
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
