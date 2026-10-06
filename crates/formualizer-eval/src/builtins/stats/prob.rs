//! PROB: the probability that values of a discrete distribution lie in a range.

use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

/// The elements of a range or array argument in row-major order, a single
/// value as one element.
fn elements(arg: &ArgumentHandle<'_, '_>) -> Result<Vec<LiteralValue>, ExcelError> {
    let mut out = Vec::new();
    match arg.value()? {
        CalcValue::Range(view) => view.for_each_cell(&mut |cell| {
            out.push(cell.clone());
            Ok(())
        })?,
        other => out.push(other.into_literal()),
    }
    Ok(out)
}

/// A number element; `None` for text, logicals and blanks, which drop their
/// pair.
fn number(value: &LiteralValue) -> Result<Option<f64>, ExcelError> {
    match value {
        LiteralValue::Error(e) => Err(e.clone()),
        LiteralValue::Number(n) => Ok(Some(*n)),
        LiteralValue::Int(i) => Ok(Some(*i as f64)),
        LiteralValue::Boolean(_) | LiteralValue::Text(_) | LiteralValue::Empty => Ok(None),
        other => Ok(other.as_serial_number()),
    }
}

fn limit(arg: &ArgumentHandle<'_, '_>) -> Result<f64, ExcelError> {
    match arg.value()?.into_literal() {
        LiteralValue::Error(e) => Err(e),
        other => crate::builtins::utils::coerce_num(&other),
    }
}

#[derive(Debug)]
pub struct ProbFn;
/// Returns the probability that values in a range lie between two limits.
///
/// `PROB(x_range, prob_range, lower_limit, [upper_limit])` adds the
/// probabilities of the x values from `lower_limit` to `upper_limit`; without
/// `upper_limit`, those equal to `lower_limit`.
///
/// # Remarks
/// - The two ranges are read in row-major order and must hold as many values
///   (`#N/A` otherwise); their shapes may differ.
/// - A pair whose x or probability is text, a logical or blank is left out;
///   an error in either propagates.
/// - The probabilities kept must add up to 1 when rounded to 15 significant
///   digits, else `#NUM!`. Excel checks nothing else: a negative probability or
///   one above 1 is accepted.
/// - The limits read like any number argument (numeric text, logicals, a blank
///   cell as 0).
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Probability of x = 2"
/// formula: '=PROB({0,1,2,3},{0.2,0.3,0.1,0.4},2)'
/// expected: 0.1
/// ```
///
/// ```yaml,sandbox
/// title: "Probability of 1 <= x <= 3"
/// formula: '=PROB({0,1,2,3},{0.2,0.3,0.1,0.4},1,3)'
/// expected: 0.8
/// ```
///
/// ```yaml,docs
/// related:
///   - BINOM.DIST
///   - NORM.DIST
/// faq:
///   - q: "How exactly must the probabilities add up to 1?"
///     a: "To 15 significant digits: {0.1,0.2,0.7} is accepted, {0.5,0.499999999999999} is #NUM!."
/// ```
impl Function for ProbFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "PROB"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
            std::sync::LazyLock::new(|| vec![ArgSchema::any()]);
        &SCHEMA
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let result = (|| -> Result<f64, ExcelError> {
            if !(3..=4).contains(&args.len()) {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let lower = limit(&args[2])?;
            let upper = match args.get(3) {
                Some(arg) if !arg.is_omitted() => limit(arg)?,
                _ => lower,
            };
            let xs = elements(&args[0])?;
            let ps = elements(&args[1])?;
            if xs.len() != ps.len() {
                return Err(ExcelError::new(ExcelErrorKind::Na));
            }
            let mut total = 0.0;
            let mut inside = 0.0;
            for (x, p) in xs.iter().zip(&ps) {
                let x = number(x)?;
                let p = number(p)?;
                if let (Some(x), Some(p)) = (x, p) {
                    total += p;
                    if lower <= x && x <= upper {
                        inside += p;
                    }
                }
            }
            if !crate::coercion::same_to_15_digits(total, 1.0) {
                return Err(ExcelError::new(ExcelErrorKind::Num));
            }
            Ok(inside)
        })();
        Ok(CalcValue::Scalar(match result {
            Ok(n) => LiteralValue::Number(n),
            Err(e) => LiteralValue::Error(e),
        }))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(ProbFn));
}
