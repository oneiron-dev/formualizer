//! Volatile functions like RAND, RANDBETWEEN.
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;
use rand::Rng;

#[derive(Debug)]
pub struct RandFn;

/// Returns a uniformly distributed pseudo-random number in the interval `[0, 1)`.
///
/// `RAND` is volatile and recalculates whenever dependent formulas recalculate.
///
/// # Remarks
/// - The result is always greater than or equal to `0` and strictly less than `1`.
/// - Because the function is volatile, repeated evaluations can return different values.
/// - The engine seeds randomness per evaluation context for reproducible execution flows.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "RAND stays within bounds"
/// formula: "=LET(n, RAND(), AND(n>=0, n<1))"
/// expected: true
/// ```
///
/// ```yaml,sandbox
/// title: "Derived integer bucket from RAND"
/// formula: "=LET(n, INT(RAND()*10), AND(n>=0, n<=9))"
/// expected: true
/// ```
///
/// ```yaml,docs
/// related:
///   - RANDBETWEEN
///   - LET
///   - INT
/// faq:
///   - q: "Can RAND return the same value on every recalculation?"
///     a: "Not by default. RAND is volatile, so recalculation can produce a different sample each time."
///   - q: "If RAND is used twice in one formula, do both uses share one sample?"
///     a: "Only if you bind it once (for example with LET). Two separate RAND calls are two separate draws."
///   - q: "Why does RAND look deterministic in some engine runs?"
///     a: "Randomness is seeded per evaluation context, which keeps a run reproducible while still treating RAND as volatile across recalculations."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: RAND
/// Type: RandFn
/// Min args: 0
/// Max args: 0
/// Variadic: false
/// Signature: RAND()
/// Arg schema: []
/// Caps: VOLATILE
/// [formualizer-docgen:schema:end]
impl Function for RandFn {
    func_caps!(VOLATILE);

    fn name(&self) -> &'static str {
        "RAND"
    }
    fn min_args(&self) -> usize {
        0
    }

    fn eval<'a, 'b, 'c>(
        &self,
        _args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let mut rng = ctx.rng_for_current(self.function_salt());
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(
            rng.gen_range(0.0..1.0),
        )))
    }
}

impl RandBetweenFn {
    /// One draw between `bottom` and `top`: from `bottom` rounded up to `top`
    /// rounded down, but never below the low end (RANDBETWEEN(1.2,1.8) is 2);
    /// `#NUM!` when `bottom > top`.
    fn draw(&self, bottom: f64, top: f64, ctx: &dyn FunctionContext<'_>) -> LiteralValue {
        if bottom > top {
            return LiteralValue::Error(
                ExcelError::new(formualizer_common::ExcelErrorKind::Num)
                    .with_message("RANDBETWEEN: bottom > top".to_string()),
            );
        }
        let low = bottom.ceil();
        let high = top.floor().max(low);
        let mut rng = ctx.rng_for_current(self.function_salt());
        LiteralValue::Number(
            (low + (rng.gen_range(0.0..1.0) * (high - low + 1.0)).floor()).min(high),
        )
    }
}

/// A RANDBETWEEN argument: one value, or the elements of an array value.
enum Bound {
    Value(LiteralValue),
    Array(Vec<Vec<LiteralValue>>),
}

impl Bound {
    fn iter(&self) -> impl Iterator<Item = &Vec<Vec<LiteralValue>>> {
        match self {
            Self::Array(cells) => Some(cells),
            Self::Value(_) => None,
        }
        .into_iter()
    }
}

/// A RANDBETWEEN argument as Excel reads it: an empty argument is `#N/A` and
/// a multi-cell reference `#VALUE!`; an array value is drawn element by element.
fn random_bound(arg: &ArgumentHandle<'_, '_>) -> Result<Bound, ExcelError> {
    use formualizer_common::ExcelErrorKind;
    if arg.is_omitted() {
        return Err(ExcelError::new(ExcelErrorKind::Na));
    }
    let value = arg.value()?;
    match crate::lift::array_rows(&value) {
        Some(_) if arg.has_reference_semantics() => Err(ExcelError::new(ExcelErrorKind::Value)),
        Some(cells) => Ok(Bound::Array(cells)),
        None => Ok(Bound::Value(value.into_literal())),
    }
}

/// A bound's number: a number, numeric or date text, or a blank cell (0); a
/// logical (typed, in a cell or computed) or other text is `#VALUE!`.
fn bound_number(value: &LiteralValue, arg: &ArgumentHandle<'_, '_>) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Error(error) => Err(error.clone()),
        LiteralValue::Empty => Ok(0.0),
        LiteralValue::Boolean(_) | LiteralValue::Array(_) => {
            Err(ExcelError::new(formualizer_common::ExcelErrorKind::Value))
        }
        value => crate::coercion::to_serial_lenient_in_year(
            value,
            arg.date_system(),
            Some(arg.current_year()),
        ),
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(RandFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(RandBetweenFn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{interpreter::Interpreter, test_workbook::TestWorkbook};
    use formualizer_parse::LiteralValue;

    fn interp(wb: &TestWorkbook) -> Interpreter<'_> {
        wb.interpreter()
    }

    #[test]
    fn test_rand_caps() {
        let rand_fn = RandFn;
        let caps = rand_fn.caps();

        // Check that VOLATILE is set
        assert!(caps.contains(crate::function::FnCaps::VOLATILE));

        // Check that PURE is not set (volatile functions are not pure)
        assert!(!caps.contains(crate::function::FnCaps::PURE));
    }

    #[test]
    fn test_rand() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(RandFn));
        let ctx = interp(&wb);

        let f = ctx.context.get_function("", "RAND").unwrap();
        let fctx = ctx.function_context(None);
        let args: Vec<ArgumentHandle<'_, '_>> = Vec::new();
        let result = f.dispatch(&args, &fctx).unwrap().into_literal();
        match result {
            LiteralValue::Number(n) => assert!((0.0..1.0).contains(&n)),
            _ => panic!("Expected a number"),
        }
    }

    #[test]
    fn test_randbetween_basic() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(RandBetweenFn));
        let ctx = interp(&wb);
        let f = ctx.context.get_function("", "RANDBETWEEN").unwrap();
        let fctx = ctx.function_context(None);
        // Build two scalar args 1 and 3
        let lo = formualizer_parse::ASTNode::new(
            formualizer_parse::ASTNodeType::Literal(LiteralValue::Int(1)),
            None,
        );
        let hi = formualizer_parse::ASTNode::new(
            formualizer_parse::ASTNodeType::Literal(LiteralValue::Int(3)),
            None,
        );
        let args = vec![
            ArgumentHandle::new(&lo, &ctx),
            ArgumentHandle::new(&hi, &ctx),
        ];
        let v = f.dispatch(&args, &fctx).unwrap().into_literal();
        match v {
            LiteralValue::Number(n) => assert!([1.0, 2.0, 3.0].contains(&n)),
            _ => panic!("Expected a whole number"),
        }
    }
}

#[derive(Debug)]
pub struct RandBetweenFn;

/// Returns a random integer between two inclusive bounds.
///
/// `RANDBETWEEN` evaluates both bounds, then samples an integer in `[low, high]`.
///
/// # Remarks
/// - Bounds are numbers: numeric and date text convert and a blank cell is 0; a logical
///   (typed, in a cell or computed) or other text is `#VALUE!`, and an empty argument
///   `#N/A`.
/// - If `bottom > top`, the function returns `#NUM!`.
/// - The low end is `bottom` rounded up and the high end `top` rounded down, but never
///   below the low end: `RANDBETWEEN(1.2,1.8)` is 2 (Excel for Windows 16.0.20430).
/// - Each call draws its own value, and an array of bounds gives an array of draws; a
///   multi-cell reference is `#VALUE!`.
/// - The function is volatile and may return a different integer each recalculation.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Value always falls inside the requested interval"
/// formula: "=LET(n, RANDBETWEEN(1, 3), AND(n>=1, n<=3, INT(n)=n))"
/// expected: true
/// ```
///
/// ```yaml,sandbox
/// title: "Equal bounds produce a fixed value"
/// formula: "=RANDBETWEEN(7, 7)"
/// expected: 7
/// ```
///
/// ```yaml,sandbox
/// title: "Upper bound below lower bound is invalid"
/// formula: "=RANDBETWEEN(5, 1)"
/// expected: "#NUM!"
/// ```
///
/// ```yaml,docs
/// related:
///   - RAND
///   - INT
///   - LET
/// faq:
///   - q: "Are both bounds included in RANDBETWEEN?"
///     a: "Yes. RANDBETWEEN samples an integer in the closed interval [low, high]."
///   - q: "What happens with decimal bounds like RANDBETWEEN(1.9, 4.2)?"
///     a: "The bottom rounds up and the top down, so this behaves like RANDBETWEEN(2, 4)."
///   - q: "Is RANDBETWEEN deterministic?"
///     a: "It is volatile, so results can change on recalculation, though a single evaluation context uses seeded randomness for reproducible execution."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: RANDBETWEEN
/// Type: RandBetweenFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: RANDBETWEEN(arg1: number@scalar, arg2: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: VOLATILE
/// [formualizer-docgen:schema:end]
impl Function for RandBetweenFn {
    func_caps!(VOLATILE);

    fn name(&self) -> &'static str {
        "RANDBETWEEN"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &crate::builtins::utils::ARG_NUM_LENIENT_TWO[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let bottom = random_bound(&args[0])?;
        let top = random_bound(&args[1])?;
        let draw = |bottom: &LiteralValue, top: &LiteralValue| {
            let bounds = bound_number(bottom, &args[0])
                .and_then(|bottom| Ok((bottom, bound_number(top, &args[1])?)));
            match bounds {
                Ok((bottom, top)) => self.draw(bottom, top, ctx),
                Err(error) => LiteralValue::Error(error),
            }
        };
        // An array of bounds draws once per element (RANDBETWEEN({1,5},{1,5})
        // is {1,5}); a multi-cell reference is #VALUE! (Excel for Windows
        // 16.0.20430).
        let (rows, cols) = crate::lift::broadcast_dims(bottom.iter().chain(top.iter()));
        let value = match (&bottom, &top) {
            (Bound::Value(bottom), Bound::Value(top)) => draw(bottom, top),
            _ => {
                let element = |bound: &Bound, row: usize, col: usize| match bound {
                    Bound::Value(value) => value.clone(),
                    Bound::Array(cells) => crate::lift::broadcast_get(cells, row, col),
                };
                let cells = (0..rows)
                    .map(|row| {
                        (0..cols)
                            .map(|col| draw(&element(&bottom, row, col), &element(&top, row, col)))
                            .collect()
                    })
                    .collect();
                return Ok(crate::lift::array_result(cells, ctx.date_system()));
            }
        };
        Ok(crate::traits::CalcValue::Scalar(value))
    }
}
