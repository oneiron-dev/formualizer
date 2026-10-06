//! PERCENTOF: the share of a subset's sum in a total's sum.

use super::{AggregateArgument, resolve_aggregate_argument};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

/// The sum PERCENTOF takes of one argument: the numbers of a range or array
/// (text, logicals and blanks left out; an error in it is `#NUM!`), or a
/// single value read as a number argument (its own error propagates).
fn sum<'b>(arg: &ArgumentHandle<'_, 'b>, ctx: &dyn FunctionContext<'b>) -> Result<f64, ExcelError> {
    match resolve_aggregate_argument(arg, ctx)? {
        AggregateArgument::Range(view) => {
            let mut total = 0.0;
            view.for_each_cell(&mut |cell| {
                match cell {
                    LiteralValue::Error(_) => return Err(ExcelError::new(ExcelErrorKind::Num)),
                    LiteralValue::Number(n) => total += n,
                    LiteralValue::Int(i) => total += *i as f64,
                    LiteralValue::Boolean(_) | LiteralValue::Text(_) | LiteralValue::Empty => {}
                    other => total += other.as_serial_number().unwrap_or(0.0),
                }
                Ok(())
            })?;
            Ok(total)
        }
        AggregateArgument::ReferenceError(e) => Err(e),
        AggregateArgument::Scalar(LiteralValue::Error(e)) => Err(e),
        AggregateArgument::Scalar(value) => crate::builtins::utils::coerce_num(&value),
    }
}

#[derive(Debug)]
pub struct PercentOfFn;
/// Returns the sum of a subset of data as a share of the sum of all the data.
///
/// `PERCENTOF(data_subset, data_all)` is `SUM(data_subset)/SUM(data_all)`.
///
/// # Remarks
/// - Numbers in ranges and arrays are added; text, logicals and blanks in them
///   are left out. An error inside a range or array returns `#NUM!`, as in
///   Excel for Windows; an argument that is itself an error returns it.
/// - A single value reads like a number argument (`"2"` is 2, `TRUE` is 1,
///   other text `#VALUE!`).
/// - A total of 0 returns `#DIV/0!`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Share of a total"
/// formula: '=PERCENTOF({1,2,3},{1,2,3,4})'
/// expected: 0.6
/// ```
///
/// ```yaml,docs
/// related:
///   - SUM
///   - GROUPBY
///   - PIVOTBY
/// faq:
///   - q: "What does an error inside the data give?"
///     a: "#NUM!, wherever it is in either range; an error passed as the argument itself propagates."
/// ```
impl Function for PercentOfFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "PERCENTOF"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
            std::sync::LazyLock::new(|| vec![ArgSchema::any(), ArgSchema::any()]);
        &SCHEMA
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let result = (|| -> Result<f64, ExcelError> {
            if args.len() != 2 {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let subset = sum(&args[0], ctx)?;
            let all = sum(&args[1], ctx)?;
            if all == 0.0 {
                return Err(ExcelError::new(ExcelErrorKind::Div));
            }
            let share = subset / all;
            if share.is_finite() {
                Ok(share)
            } else {
                Err(ExcelError::new(ExcelErrorKind::Num))
            }
        })();
        Ok(CalcValue::Scalar(match result {
            Ok(n) => LiteralValue::Number(n),
            Err(e) => LiteralValue::Error(e),
        }))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(PercentOfFn));
}
