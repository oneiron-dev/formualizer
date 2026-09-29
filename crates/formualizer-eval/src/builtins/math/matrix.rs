//! Matrix functions: MMULT.

use super::super::utils::ARG_ANY_TWO;
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;

/// A numeric matrix from a range or array argument. Every element must be a
/// number: text, logical and blank elements are `#VALUE!`, errors propagate.
fn numeric_matrix(arg: &ArgumentHandle<'_, '_>) -> Result<Vec<Vec<f64>>, ExcelError> {
    let value = arg.value()?;
    let rows = crate::lift::array_rows(&value).unwrap_or_else(|| vec![vec![value.into_literal()]]);
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| match cell {
                    LiteralValue::Number(n) => Ok(n),
                    LiteralValue::Int(i) => Ok(i as f64),
                    LiteralValue::Error(e) => Err(e),
                    _ => Err(ExcelError::new_value()),
                })
                .collect()
        })
        .collect()
}

#[derive(Debug)]
pub struct MmultFn;
/// Returns the matrix product of two arrays.
///
/// `MMULT(array1, array2)` has as many rows as `array1` and as many columns as
/// `array2`.
///
/// # Remarks
/// - The column count of `array1` must equal the row count of `array2`,
///   otherwise the result is `#VALUE!`.
/// - Any empty, text or logical element returns `#VALUE!`; errors propagate.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Row times column"
/// formula: '=MMULT({1,2},{3;4})'
/// expected: 11
/// ```
///
/// ```yaml,docs
/// related:
///   - SUMPRODUCT
///   - TRANSPOSE
/// faq:
///   - q: "What shape does MMULT return?"
///     a: "rows(array1) by columns(array2); it spills when that is larger than one cell."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: MMULT
/// Type: MmultFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: MMULT(arg1: any@scalar, arg2: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for MmultFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "MMULT"
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
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let error = |e: ExcelError| Ok(CalcValue::Scalar(LiteralValue::Error(e)));
        if args.len() != 2 {
            return error(ExcelError::new_value());
        }
        let (a, b) = match (numeric_matrix(&args[0]), numeric_matrix(&args[1])) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return error(e),
        };
        let inner = a.first().map_or(0, Vec::len);
        let width = b.first().map_or(0, Vec::len);
        if inner == 0 || width == 0 || inner != b.len() {
            return error(ExcelError::new_value());
        }
        let product: Vec<Vec<LiteralValue>> = a
            .iter()
            .map(|row| {
                (0..width)
                    .map(|j| LiteralValue::Number((0..inner).map(|k| row[k] * b[k][j]).sum()))
                    .collect()
            })
            .collect();
        if product.len() == 1 && width == 1 {
            return Ok(CalcValue::Scalar(product[0][0].clone()));
        }
        Ok(crate::lift::array_result(product, args[0].date_system()))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(MmultFn));
}

#[cfg(test)]
mod tests {
    use crate::engine::{Engine, EvalConfig};
    use crate::test_workbook::TestWorkbook;
    use formualizer_common::{ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::parse;

    fn eval(formula: &str) -> LiteralValue {
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        engine
            .set_cell_formula("Sheet1", 1, 1, parse(formula).unwrap())
            .unwrap();
        engine.evaluate_cell("Sheet1", 1, 1).unwrap();
        engine.get_cell_value("Sheet1", 1, 1).unwrap()
    }

    #[test]
    fn mmult_products_and_errors() {
        assert_eq!(eval("=MMULT({1,2},{3;4})"), LiteralValue::Number(11.0));
        assert_eq!(
            eval("=SUM(MMULT({1,2;3,4},{1;1}))"),
            LiteralValue::Number(10.0)
        );
        assert_eq!(
            eval("=INDEX(MMULT({1,2;3,4},{5,6;7,8}),2,1)"),
            LiteralValue::Number(43.0)
        );
        for bad in ["=MMULT({1,2},{3,4})", "=MMULT({1,\"x\"},{3;4})"] {
            assert!(
                matches!(eval(bad), LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value),
                "{bad}"
            );
        }
        assert!(
            matches!(eval("=MMULT({1,2},{3;1/0})"), LiteralValue::Error(e) if e.kind == ExcelErrorKind::Div)
        );
    }
}
