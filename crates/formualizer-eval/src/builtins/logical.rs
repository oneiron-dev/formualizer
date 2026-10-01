// crates/formualizer-eval/src/builtins/logical.rs

use super::utils::ARG_ANY_ONE;
use crate::args::ArgSchema;
use crate::function::{Function, FunctionResolution, resolution_to_reference};
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;

/* ─────────────────────────── TRUE() ─────────────────────────────── */

#[derive(Debug)]
pub struct TrueFn;
/// Returns the logical constant TRUE.
///
/// Use `TRUE()` when you want an explicit boolean value in formulas.
///
/// # Remarks
/// - `TRUE` takes no arguments and always returns the boolean value `TRUE`.
/// - No coercion or evaluation side effects are involved.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Return TRUE directly"
/// formula: '=TRUE()'
/// expected: true
/// ```
///
/// ```yaml,sandbox
/// title: "Use TRUE in branching"
/// formula: '=IF(TRUE(), "yes", "no")'
/// expected: "yes"
/// ```
///
/// ```yaml,docs
/// related:
///   - FALSE
///   - IF
///   - AND
/// faq:
///   - q: "Can TRUE accept arguments?"
///     a: "No. TRUE takes zero arguments and always returns the boolean constant TRUE."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: TRUE
/// Type: TrueFn
/// Min args: 0
/// Max args: 0
/// Variadic: false
/// Signature: TRUE()
/// Arg schema: []
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TrueFn {
    func_caps!(PURE);

    fn name(&self) -> &'static str {
        "TRUE"
    }
    fn min_args(&self) -> usize {
        0
    }

    fn eval<'a, 'b, 'c>(
        &self,
        _args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Boolean(
            true,
        )))
    }
}

/* ─────────────────────────── FALSE() ────────────────────────────── */

#[derive(Debug)]
pub struct FalseFn;
/// Returns the logical constant FALSE.
///
/// Use `FALSE()` when you want an explicit boolean false value in formulas.
///
/// # Remarks
/// - `FALSE` takes no arguments and always returns the boolean value `FALSE`.
/// - No coercion or evaluation side effects are involved.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Return FALSE directly"
/// formula: '=FALSE()'
/// expected: false
/// ```
///
/// ```yaml,sandbox
/// title: "Use FALSE in branching"
/// formula: '=IF(FALSE(), "yes", "no")'
/// expected: "no"
/// ```
///
/// ```yaml,docs
/// related:
///   - TRUE
///   - IF
///   - OR
/// faq:
///   - q: "Can FALSE accept arguments?"
///     a: "No. FALSE takes zero arguments and always returns the boolean constant FALSE."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: FALSE
/// Type: FalseFn
/// Min args: 0
/// Max args: 0
/// Variadic: false
/// Signature: FALSE()
/// Arg schema: []
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for FalseFn {
    func_caps!(PURE);

    fn name(&self) -> &'static str {
        "FALSE"
    }
    fn min_args(&self) -> usize {
        0
    }

    fn eval<'a, 'b, 'c>(
        &self,
        _args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Boolean(
            false,
        )))
    }
}

/* ─────────────────────────── AND() ──────────────────────────────── */

/// Counts the TRUE and FALSE values AND, OR and XOR read, Excel's way.
/// Every argument is evaluated and the first error wins. In references and
/// arrays only logicals and numbers count (text and blanks are skipped); a
/// direct argument may also be the text "TRUE" or "FALSE", and other direct
/// text is skipped. With nothing to count the result is `#VALUE!`.
pub(crate) fn count_logicals(
    args: &[ArgumentHandle<'_, '_>],
) -> Result<(usize, usize), ExcelError> {
    let (mut trues, mut falses) = (0usize, 0usize);
    let mut first_error: Option<ExcelError> = None;
    let mut count = |v: &LiteralValue, direct: bool, first_error: &mut Option<ExcelError>| {
        let logical = match v {
            LiteralValue::Boolean(b) => Some(*b),
            LiteralValue::Number(n) => Some(*n != 0.0),
            LiteralValue::Int(i) => Some(*i != 0),
            LiteralValue::Text(t) if direct && t.eq_ignore_ascii_case("TRUE") => Some(true),
            LiteralValue::Text(t) if direct && t.eq_ignore_ascii_case("FALSE") => Some(false),
            LiteralValue::Error(e) => {
                first_error.get_or_insert_with(|| e.clone());
                None
            }
            _ => None,
        };
        match logical {
            Some(true) => trues += 1,
            Some(false) => falses += 1,
            None => {}
        }
    };
    for arg in args {
        if arg.is_omitted() {
            // An empty argument slot reads as FALSE.
            count(&LiteralValue::Boolean(false), true, &mut first_error);
            continue;
        }
        let value = match arg.value() {
            Ok(value) => value,
            Err(e) => {
                first_error.get_or_insert(e);
                continue;
            }
        };
        match value {
            crate::traits::CalcValue::Range(view) => {
                view.for_each_cell(&mut |v| {
                    count(v, false, &mut first_error);
                    Ok(())
                })?;
            }
            other => match other.into_literal() {
                LiteralValue::Array(rows) => {
                    for v in rows.iter().flatten() {
                        count(v, false, &mut first_error);
                    }
                }
                v => count(&v, !arg.may_return_reference(), &mut first_error),
            },
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    if trues + falses == 0 {
        return Err(ExcelError::new_value().with_message("No logical values to evaluate"));
    }
    Ok((trues, falses))
}

fn logical_result<'b>(
    counts: Result<(usize, usize), ExcelError>,
    decide: impl Fn(usize, usize) -> bool,
) -> crate::traits::CalcValue<'b> {
    crate::traits::CalcValue::Scalar(match counts {
        Ok((trues, falses)) => LiteralValue::Boolean(decide(trues, falses)),
        Err(e) => LiteralValue::Error(e),
    })
}

#[derive(Debug)]
pub struct AndFn;
/// Returns TRUE only when all supplied values evaluate to TRUE.
///
/// `AND` evaluates every argument left to right.
///
/// # Remarks
/// - Booleans and numbers are accepted (`0` is FALSE, non-zero is TRUE).
/// - Text and blank cells inside references and arrays are skipped.
/// - Text given directly counts when it reads "TRUE"/"FALSE"; other text is skipped.
/// - The first error in argument order is returned, even after a FALSE.
/// - `#VALUE!` when no logical value is found.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "All truthy inputs"
/// formula: '=AND(TRUE, 1, 5)'
/// expected: true
/// ```
///
/// ```yaml,sandbox
/// title: "Text input causes VALUE error"
/// formula: '=AND(TRUE, "x")'
/// expected: "#VALUE!"
/// ```
///
/// ```yaml,docs
/// related:
///   - OR
///   - NOT
///   - XOR
/// faq:
///   - q: "What happens with blanks and text in AND?"
///     a: "Blank cells and text are skipped unless direct text reads TRUE/FALSE; #VALUE! when nothing logical remains."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: AND
/// Type: AndFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: AND(arg1...: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, REDUCTION, BOOL_ONLY, SHORT_CIRCUIT
/// [formualizer-docgen:schema:end]
impl Function for AndFn {
    func_caps!(PURE, REDUCTION, BOOL_ONLY, SHORT_CIRCUIT);

    fn name(&self) -> &'static str {
        "AND"
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
        Ok(logical_result(count_logicals(args), |_, falses| {
            falses == 0
        }))
    }
}

/* ─────────────────────────── OR() ───────────────────────────────── */

#[derive(Debug)]
pub struct OrFn;
/// Returns TRUE when any supplied value evaluates to TRUE.
///
/// `OR` evaluates every argument left to right.
///
/// # Remarks
/// - Booleans and numbers are accepted (`0` is FALSE, non-zero is TRUE).
/// - Text and blank cells inside references and arrays are skipped.
/// - Text given directly counts when it reads "TRUE"/"FALSE"; other text is skipped.
/// - The first error in argument order is returned, even after a TRUE.
/// - `#VALUE!` when no logical value is found.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "One truthy value makes OR true"
/// formula: '=OR(FALSE, 0, 2)'
/// expected: true
/// ```
///
/// ```yaml,sandbox
/// title: "No true values and text input"
/// formula: '=OR(FALSE, "x")'
/// expected: "#VALUE!"
/// ```
///
/// ```yaml,docs
/// related:
///   - AND
///   - NOT
///   - XOR
/// faq:
///   - q: "How does OR treat blanks and text?"
///     a: "Blank cells and text in references are skipped; direct text other than TRUE/FALSE returns #VALUE!."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: OR
/// Type: OrFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: OR(arg1...: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, REDUCTION, BOOL_ONLY, SHORT_CIRCUIT
/// [formualizer-docgen:schema:end]
impl Function for OrFn {
    func_caps!(PURE, REDUCTION, BOOL_ONLY, SHORT_CIRCUIT);

    fn name(&self) -> &'static str {
        "OR"
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
        Ok(logical_result(count_logicals(args), |trues, _| trues > 0))
    }
}

/* ─────────────────────────── IF() ───────────────────────────────── */

#[derive(Debug)]
pub struct IfFn;
/// Returns one value when a condition is TRUE and another when FALSE.
///
/// `IF(condition, value_if_true, [value_if_false])` supports two or three arguments.
///
/// # Remarks
/// - Condition coercion: booleans are used directly, numbers use `0` as FALSE and non-zero as TRUE.
/// - A blank condition is treated as FALSE.
/// - Text `"TRUE"`/`"FALSE"` (any case) are logical; other text conditions return `#VALUE!`.
/// - With only two arguments, the FALSE branch defaults to logical `FALSE`.
/// - A selected empty argument slot (`IF(FALSE,1,)`) returns 0, also element-wise
///   for an array condition. `&` and text functions such as CONCAT and LEN read
///   an empty slot that a single-value condition selects as empty text, so
///   `IF(FALSE,"not ",)&"ok"` is `"ok"`; VALUE and NUMBERVALUE still read 0.
/// - An array condition selects element-wise between the (broadcast) branches.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Numeric condition"
/// formula: '=IF(2, "yes", "no")'
/// expected: "yes"
/// ```
///
/// ```yaml,sandbox
/// title: "Two-argument IF defaults false branch"
/// formula: '=IF(0, 10)'
/// expected: false
/// ```
///
/// ```yaml,docs
/// related:
///   - IFS
///   - IFERROR
///   - IFNA
/// faq:
///   - q: "What is returned when IF has only two arguments and condition is FALSE?"
///     a: "The false branch defaults to logical FALSE when value_if_false is omitted."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: IF
/// Type: IfFn
/// Min args: 2
/// Max args: variadic
/// Variadic: true
/// Signature: IF(arg1...: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, RETURNS_REFERENCE, SHORT_CIRCUIT
/// [formualizer-docgen:schema:end]
impl Function for IfFn {
    fn propagate_format(
        &self,
        result: &crate::traits::CalcValue<'_>,
    ) -> Option<crate::format::FormatId> {
        result.format_id()
    }

    func_caps!(PURE, SHORT_CIRCUIT, RETURNS_REFERENCE, MAY_SPILL);

    fn name(&self) -> &'static str {
        "IF"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn variadic(&self) -> bool {
        true
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        // Single variadic any schema so we can enforce precise 2 or 3 arity inside eval()
        static ONE: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| vec![ArgSchema::any()]);
        &ONE[..]
    }

    fn eval_reference<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Option<Result<formualizer_parse::parser::ReferenceType, ExcelError>> {
        match try_resolve_if_reference_or_value(args) {
            Ok(Some(result)) => resolution_to_reference(Ok(result)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    fn resolve_reference_or_value<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
        value_fallback: &dyn Fn() -> Result<crate::traits::CalcValue<'b>, ExcelError>,
    ) -> Result<FunctionResolution<'b>, ExcelError> {
        match try_resolve_if_reference_or_value(args)? {
            Some(result) => Ok(result),
            None => value_fallback().map(FunctionResolution::Value),
        }
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.len() < 2 || args.len() > 3 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value()
                    .with_message(format!("IF expects 2 or 3 arguments, got {}", args.len())),
            )));
        }

        let condition = args[0].value()?;
        if let Some(conditions) = crate::lift::array_rows(&condition) {
            return Ok(if_over_array(args, conditions));
        }
        let b = match if_condition(condition.into_literal()) {
            Ok(b) => b,
            Err(error) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error))),
        };

        if b {
            args[1].value()
        } else if let Some(arg) = args.get(2) {
            arg.value()
        } else {
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Boolean(
                false,
            )))
        }
    }
}

/// The argument (1 or 2) whose value IF returns for a single-value condition,
/// as `IfFn::eval` selects it. `None` when IF returns no argument's value: a
/// wrong argument count, an array or error condition, or a FALSE condition
/// with no value_if_false.
pub(crate) fn if_selected_argument(args: &[ArgumentHandle<'_, '_>]) -> Option<usize> {
    if !(2..=3).contains(&args.len()) {
        return None;
    }
    let condition = args[0].value().ok()?;
    if crate::lift::array_rows(&condition).is_some() {
        return None;
    }
    if if_condition(condition.into_literal()).ok()? {
        Some(1)
    } else {
        (args.len() == 3).then_some(2)
    }
}

/// IF condition coercion: logical, number (non-zero is TRUE), blank is FALSE,
/// and the text "TRUE"/"FALSE" in any case. Other text is `#VALUE!`.
pub(crate) fn if_condition(condition: LiteralValue) -> Result<bool, ExcelError> {
    match condition {
        LiteralValue::Boolean(b) => Ok(b),
        LiteralValue::Number(n) => Ok(n != 0.0),
        LiteralValue::Int(i) => Ok(i != 0),
        LiteralValue::Empty => Ok(false),
        LiteralValue::Error(error) => Err(error),
        LiteralValue::Text(text) if text.eq_ignore_ascii_case("TRUE") => Ok(true),
        LiteralValue::Text(text) if text.eq_ignore_ascii_case("FALSE") => Ok(false),
        _ => Err(ExcelError::new_value().with_message("IF condition must be boolean or number")),
    }
}

/// IF with an array condition selects element-wise between both branches,
/// each broadcast to the combined shape (a missing FALSE branch is FALSE).
fn if_over_array<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    conditions: Vec<Vec<LiteralValue>>,
) -> crate::traits::CalcValue<'b> {
    let branch = |index: usize| -> Vec<Vec<LiteralValue>> {
        let value = match args.get(index) {
            None => return vec![vec![LiteralValue::Boolean(false)]],
            Some(arg) => match arg.value() {
                Ok(value) => value,
                Err(error) => return vec![vec![LiteralValue::Error(error)]],
            },
        };
        crate::lift::array_rows(&value).unwrap_or_else(|| vec![vec![value.into_literal()]])
    };
    let (when_true, when_false) = (branch(1), branch(2));
    let (height, width) = crate::lift::broadcast_dims([&conditions, &when_true, &when_false]);
    let rows = (0..height)
        .map(|r| {
            (0..width)
                .map(
                    |c| match if_condition(crate::lift::broadcast_get(&conditions, r, c)) {
                        Ok(true) => crate::lift::broadcast_get(&when_true, r, c),
                        Ok(false) => crate::lift::broadcast_get(&when_false, r, c),
                        Err(error) => LiteralValue::Error(error),
                    },
                )
                .collect()
        })
        .collect();
    crate::lift::array_result(rows, args[0].date_system())
}

fn try_resolve_if_reference_or_value<'b>(
    args: &[ArgumentHandle<'_, 'b>],
) -> Result<Option<FunctionResolution<'b>>, ExcelError> {
    if args.len() < 2 || args.len() > 3 {
        return Ok(Some(FunctionResolution::Value(
            crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value()
                    .with_message(format!("IF expects 2 or 3 arguments, got {}", args.len())),
            )),
        )));
    }
    let condition = args[0].value()?.into_literal();
    if matches!(condition, LiteralValue::Array(_)) {
        return Ok(None);
    }
    let selected = match if_condition(condition) {
        Ok(selected) => selected,
        Err(error) => {
            return Ok(Some(FunctionResolution::Value(
                crate::traits::CalcValue::Scalar(LiteralValue::Error(error)),
            )));
        }
    };
    if selected {
        args[1].resolve_reference_or_value().map(Some)
    } else if let Some(arg) = args.get(2) {
        arg.resolve_reference_or_value().map(Some)
    } else {
        Ok(Some(FunctionResolution::Value(
            crate::traits::CalcValue::Scalar(LiteralValue::Boolean(false)),
        )))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(TrueFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(FalseFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(AndFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(OrFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(IfFn));
}

/* ─────────────────────────── tests ─────────────────────────────── */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{CycleConfig, CycleDetection, CyclePolicy, Engine, EvalConfig};
    use crate::traits::ArgumentHandle;
    use crate::{interpreter::Interpreter, test_workbook::TestWorkbook};
    use formualizer_common::ExcelErrorKind;
    use formualizer_parse::{LiteralValue, parser::Parser, parser::parse};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct CountFn(Arc<AtomicUsize>);
    impl Function for CountFn {
        func_caps!(PURE);
        fn name(&self) -> &'static str {
            "COUNTING"
        }
        fn min_args(&self) -> usize {
            0
        }
        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Boolean(
                true,
            )))
        }
    }

    #[derive(Debug)]
    struct ErrorFn(Arc<AtomicUsize>);
    impl Function for ErrorFn {
        func_caps!(PURE);
        fn name(&self) -> &'static str {
            "ERRORFN"
        }
        fn min_args(&self) -> usize {
            0
        }
        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )))
        }
    }

    fn interp(wb: &TestWorkbook) -> Interpreter<'_> {
        wb.interpreter()
    }

    fn evaluate_formula(formula: &str, wb: &TestWorkbook) -> LiteralValue {
        let mut parser = Parser::new(formula).expect("parser");
        let ast = parser.parse().expect("parse");
        wb.interpreter()
            .evaluate_ast(&ast)
            .expect("evaluate")
            .into_literal()
    }

    fn assert_error_kind(value: LiteralValue, kind: ExcelErrorKind) {
        assert!(
            matches!(value, LiteralValue::Error(ref error) if error.kind == kind),
            "expected {kind:?}, got {value:?}"
        );
    }

    #[test]
    fn test_true_false() {
        let wb = TestWorkbook::new()
            .with_function(std::sync::Arc::new(TrueFn))
            .with_function(std::sync::Arc::new(FalseFn));

        let ctx = interp(&wb);
        let t = ctx.context.get_function("", "TRUE").unwrap();
        let fctx = ctx.function_context(None);
        assert_eq!(
            t.eval(&[], &fctx).unwrap().into_literal(),
            LiteralValue::Boolean(true)
        );

        let f = ctx.context.get_function("", "FALSE").unwrap();
        assert_eq!(
            f.eval(&[], &fctx).unwrap().into_literal(),
            LiteralValue::Boolean(false)
        );
    }

    #[test]
    fn test_and_or() {
        let wb = TestWorkbook::new()
            .with_function(std::sync::Arc::new(AndFn))
            .with_function(std::sync::Arc::new(OrFn));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);

        let and = ctx.context.get_function("", "AND").unwrap();
        let or = ctx.context.get_function("", "OR").unwrap();
        // Build ArgumentHandles manually: TRUE, 1, FALSE
        let dummy_ast = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Boolean(true)),
            None,
        );
        let dummy_ast_false = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Boolean(false)),
            None,
        );
        let dummy_ast_one = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Int(1)),
            None,
        );
        let hs = vec![
            ArgumentHandle::new(&dummy_ast, &ctx),
            ArgumentHandle::new(&dummy_ast_one, &ctx),
        ];
        assert_eq!(
            and.eval(&hs, &fctx).unwrap().into_literal(),
            LiteralValue::Boolean(true)
        );

        let hs2 = vec![
            ArgumentHandle::new(&dummy_ast_false, &ctx),
            ArgumentHandle::new(&dummy_ast_one, &ctx),
        ];
        assert_eq!(
            and.eval(&hs2, &fctx).unwrap().into_literal(),
            LiteralValue::Boolean(false)
        );
        assert_eq!(
            or.eval(&hs2, &fctx).unwrap().into_literal(),
            LiteralValue::Boolean(true)
        );
    }

    #[test]
    fn and_evaluates_every_argument_like_excel() {
        let counter = Arc::new(AtomicUsize::new(0));
        let wb = TestWorkbook::new()
            .with_function(Arc::new(AndFn))
            .with_function(Arc::new(CountFn(counter.clone())));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);
        let and = ctx.context.get_function("", "AND").unwrap();

        // Build args: FALSE, COUNTING()
        let a_false = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Boolean(false)),
            None,
        );
        let counting_call = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Function {
                name: "COUNTING".into(),
                args: vec![],
            },
            None,
        );
        let hs = vec![
            ArgumentHandle::new(&a_false, &ctx),
            ArgumentHandle::new(&counting_call, &ctx),
        ];
        let out = and.eval(&hs, &fctx).unwrap().into_literal();
        assert_eq!(out, LiteralValue::Boolean(false));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "Excel evaluates every argument of AND and OR"
        );
    }

    #[test]
    fn or_evaluates_every_argument_like_excel() {
        let counter = Arc::new(AtomicUsize::new(0));
        let wb = TestWorkbook::new()
            .with_function(Arc::new(OrFn))
            .with_function(Arc::new(CountFn(counter.clone())));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);
        let or = ctx.context.get_function("", "OR").unwrap();

        // Build args: TRUE, COUNTING()
        let a_true = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Boolean(true)),
            None,
        );
        let counting_call = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Function {
                name: "COUNTING".into(),
                args: vec![],
            },
            None,
        );
        let hs = vec![
            ArgumentHandle::new(&a_true, &ctx),
            ArgumentHandle::new(&counting_call, &ctx),
        ];
        let out = or.eval(&hs, &fctx).unwrap().into_literal();
        assert_eq!(out, LiteralValue::Boolean(true));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "Excel evaluates every argument of AND and OR"
        );
    }

    #[test]
    fn or_evaluates_arguments_after_a_true_array() {
        let counter = Arc::new(AtomicUsize::new(0));
        let wb = TestWorkbook::new()
            .with_function(Arc::new(OrFn))
            .with_function(Arc::new(CountFn(counter.clone())));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);
        let or = ctx.context.get_function("", "OR").unwrap();

        // First arg is an array literal with first element 1 (truey), then zeros.
        let arr = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Array(vec![
                vec![formualizer_parse::parser::ASTNode::new(
                    formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Int(1)),
                    None,
                )],
                vec![formualizer_parse::parser::ASTNode::new(
                    formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Int(0)),
                    None,
                )],
            ]),
            None,
        );
        let counting_call = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Function {
                name: "COUNTING".into(),
                args: vec![],
            },
            None,
        );
        let hs = vec![
            ArgumentHandle::new(&arr, &ctx),
            ArgumentHandle::new(&counting_call, &ctx),
        ];
        let out = or.eval(&hs, &fctx).unwrap().into_literal();
        assert_eq!(out, LiteralValue::Boolean(true));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "Excel evaluates every argument of AND and OR"
        );
    }

    #[test]
    fn and_returns_first_error_when_no_decisive_false() {
        let err_counter = Arc::new(AtomicUsize::new(0));
        let wb = TestWorkbook::new()
            .with_function(Arc::new(AndFn))
            .with_function(Arc::new(ErrorFn(err_counter.clone())));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);
        let and = ctx.context.get_function("", "AND").unwrap();

        // AND(1, ERRORFN(), 1) => #VALUE!
        let one = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Int(1)),
            None,
        );
        let errcall = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Function {
                name: "ERRORFN".into(),
                args: vec![],
            },
            None,
        );
        let hs = vec![
            ArgumentHandle::new(&one, &ctx),
            ArgumentHandle::new(&errcall, &ctx),
            ArgumentHandle::new(&one, &ctx),
        ];
        let out = and.eval(&hs, &fctx).unwrap().into_literal();
        match out {
            LiteralValue::Error(e) => assert_eq!(e.to_string(), "#VALUE!"),
            _ => panic!("Expected error"),
        }
        assert_eq!(
            err_counter.load(Ordering::SeqCst),
            1,
            "ERRORFN should be evaluated once"
        );
    }

    #[test]
    fn or_returns_an_error_even_after_true() {
        let err_counter = Arc::new(AtomicUsize::new(0));
        let wb = TestWorkbook::new()
            .with_function(Arc::new(OrFn))
            .with_function(Arc::new(ErrorFn(err_counter.clone())));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);
        let or = ctx.context.get_function("", "OR").unwrap();

        // OR(TRUE, ERRORFN()) => #VALUE!: an error in any argument wins
        let a_true = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Boolean(true)),
            None,
        );
        let errcall = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Function {
                name: "ERRORFN".into(),
                args: vec![],
            },
            None,
        );
        let hs = vec![
            ArgumentHandle::new(&a_true, &ctx),
            ArgumentHandle::new(&errcall, &ctx),
        ];
        // Excel: OR(TRUE, error) is the error.
        let out = or.eval(&hs, &fctx).unwrap().into_literal();
        match out {
            LiteralValue::Error(e) => assert_eq!(e.to_string(), "#VALUE!"),
            other => panic!("Expected error, got {other:?}"),
        }
        assert_eq!(
            err_counter.load(Ordering::SeqCst),
            1,
            "ERRORFN should be evaluated once"
        );
    }

    #[test]
    fn and_or_xor_follow_excel_argument_rules() {
        use formualizer_parse::parser::parse;
        let wb = TestWorkbook::new()
            .with_function(Arc::new(AndFn))
            .with_function(Arc::new(OrFn))
            .with_function(Arc::new(crate::builtins::logical_ext::XorFn))
            .with_cell_a1("Sheet1", "A1", LiteralValue::Text("text".into()))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Boolean(false))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Boolean(false))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Text("TRUE".into()));
        let ctx = interp(&wb);
        let eval = |f: &str| ctx.evaluate_ast(&parse(f).unwrap()).unwrap().into_literal();
        let kind = |f: &str| match eval(f) {
            LiteralValue::Error(e) => e.kind,
            other => panic!("{f}: expected an error, got {other:?}"),
        };
        // Text in references is skipped; blanks too.
        assert_eq!(eval("=OR(A1:A3)"), LiteralValue::Boolean(false));
        assert_eq!(eval("=AND(A1:A4)"), LiteralValue::Boolean(false));
        assert_eq!(eval("=AND(TRUE,C1:C3)"), LiteralValue::Boolean(true));
        // Direct text converts when it spells a logical, and is skipped otherwise.
        assert_eq!(eval("=AND(TRUE,\"abc\")"), LiteralValue::Boolean(true));
        assert_eq!(eval("=OR(\"true\")"), LiteralValue::Boolean(true));
        assert_eq!(kind("=AND(B1)"), ExcelErrorKind::Value);
        // Nothing logical to read is #VALUE!; errors win over a decisive value.
        assert_eq!(kind("=AND(\"abc\")"), ExcelErrorKind::Value);
        assert_eq!(kind("=OR(C1:C3)"), ExcelErrorKind::Value);
        assert_eq!(kind("=AND(FALSE,1/0)"), ExcelErrorKind::Div);
        assert_eq!(eval("=XOR(TRUE,A1:A3,1)"), LiteralValue::Boolean(false));
    }

    #[test]
    fn if_treats_empty_condition_as_false() {
        let wb = TestWorkbook::new().with_function(Arc::new(IfFn));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);
        let iff = ctx.context.get_function("", "IF").unwrap();

        let cond_empty = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Empty),
            None,
        );
        let when_true = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Int(10)),
            None,
        );
        let when_false = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Int(20)),
            None,
        );

        let args = vec![
            ArgumentHandle::new(&cond_empty, &ctx),
            ArgumentHandle::new(&when_true, &ctx),
            ArgumentHandle::new(&when_false, &ctx),
        ];

        assert_eq!(
            iff.eval(&args, &fctx).unwrap().into_literal(),
            LiteralValue::Int(20)
        );
    }

    #[test]
    fn if_propagates_condition_error_kind() {
        let wb = TestWorkbook::new()
            .with_function(Arc::new(IfFn))
            .with_function(Arc::new(crate::builtins::info::NaFn));

        assert_error_kind(evaluate_formula("=IF(NA()=0,0,1)", &wb), ExcelErrorKind::Na);
        assert_error_kind(evaluate_formula("=IF(1/0>1,1,2)", &wb), ExcelErrorKind::Div);
    }

    #[test]
    fn if_errored_condition_records_no_arm_edges() {
        let config = EvalConfig::default().with_cycle(CycleConfig {
            detection: CycleDetection::Runtime,
            policy: CyclePolicy::Error,
        });
        let mut engine = Engine::new(TestWorkbook::new(), config);
        engine
            .set_cell_formula(
                "Sheet1",
                1,
                1,
                parse("=IF(NA()=0,INDEX(Q1:Q100,50),0)").expect("parse A1"),
            )
            .expect("set A1");
        engine
            .set_cell_formula("Sheet1", 50, 17, parse("=A1").expect("parse Q50"))
            .expect("set Q50");

        engine.evaluate_all().expect("evaluate");

        assert_error_kind(
            engine.get_cell_value("Sheet1", 1, 1).expect("A1 value"),
            ExcelErrorKind::Na,
        );
        assert!(
            !matches!(
                engine.get_cell_value("Sheet1", 50, 17),
                Some(LiteralValue::Error(error)) if error.kind == ExcelErrorKind::Circ
            ),
            "Q50 must not be circular when the IF condition errors"
        );
        assert_eq!(engine.last_cycle_telemetry().live_cycles_witnessed, 0);
    }

    #[test]
    fn if_text_condition_is_value_error() {
        let wb = TestWorkbook::new().with_function(Arc::new(IfFn));
        assert_error_kind(
            evaluate_formula("=IF(\"abc\",1,2)", &wb),
            ExcelErrorKind::Value,
        );
    }
}

#[cfg(test)]
mod excel_logical_tests {
    use crate::engine::{Engine, EvalConfig};
    use crate::test_workbook::TestWorkbook;
    use formualizer_common::{ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::parse;

    fn eval(formula: &str) -> LiteralValue {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        engine
            .set_cell_formula("Sheet1", 1, 1, parse("=1/0").unwrap())
            .unwrap();
        engine
            .set_cell_value("Sheet1", 2, 1, LiteralValue::Text("x".into()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", 3, 1, LiteralValue::Boolean(true))
            .unwrap();
        engine
            .set_cell_formula("Sheet1", 1, 5, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_all().unwrap();
        engine.get_cell_value("Sheet1", 1, 5).unwrap()
    }

    #[test]
    fn errors_win_over_a_deciding_value() {
        for formula in [
            "=OR(TRUE,A1)",
            "=OR(A1,TRUE)",
            "=AND(FALSE,A1)",
            "=OR(TRUE,A1:A3)",
        ] {
            match eval(formula) {
                LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Div, "{formula}"),
                other => panic!("{formula}: expected #DIV/0!, got {other:?}"),
            }
        }
    }

    #[test]
    fn text_and_blanks_in_references_are_skipped() {
        assert_eq!(eval("=AND(A2:A4)"), LiteralValue::Boolean(true));
        assert_eq!(eval("=OR(A2,FALSE)"), LiteralValue::Boolean(false));
        assert_eq!(eval("=AND(\"true\",1)"), LiteralValue::Boolean(true));
        match eval("=AND(A2:A2)") {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Value),
            other => panic!("expected #VALUE!, got {other:?}"),
        }
        // Direct text that is not TRUE/FALSE is skipped too.
        assert_eq!(eval("=OR(\"x\",TRUE)"), LiteralValue::Boolean(true));
    }
}
