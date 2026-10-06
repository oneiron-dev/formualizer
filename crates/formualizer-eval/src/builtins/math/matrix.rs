//! Matrix functions: MMULT, MDETERM, MINVERSE, MUNIT.

use super::super::utils::{ARG_ANY_ONE, ARG_ANY_TWO};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
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

/// A square numeric matrix argument (`#VALUE!` when it is not square).
fn square_matrix(arg: &ArgumentHandle<'_, '_>) -> Result<Vec<Vec<f64>>, ExcelError> {
    let matrix = numeric_matrix(arg)?;
    let n = matrix.len();
    if n == 0 || matrix.iter().any(|row| row.len() != n) {
        return Err(ExcelError::new_value());
    }
    Ok(matrix)
}

/// The LU factorisation of a square matrix with partial pivoting, the way
/// Excel's MDETERM and MINVERSE compute it: in each column the pivot is the
/// first entry of largest magnitude on or below the diagonal, the multipliers
/// are stored below the diagonal and U on and above it. `None` when a pivot
/// is exactly zero; a pivot that is only near zero stays (Excel inverts
/// `{1,2,3;4,5,6;7,8,9}` to entries near 4.5E15).
struct Lu {
    lu: Vec<Vec<f64>>,
    /// `perm[i]` is the original row now at row `i`.
    perm: Vec<usize>,
    odd_swaps: bool,
}

fn lu_factor(mut a: Vec<Vec<f64>>) -> Option<Lu> {
    let n = a.len();
    let mut perm: Vec<usize> = (0..n).collect();
    let mut odd_swaps = false;
    for k in 0..n {
        let mut p = k;
        for i in k + 1..n {
            if a[i][k].abs() > a[p][k].abs() {
                p = i;
            }
        }
        if a[p][k] == 0.0 {
            return None;
        }
        if p != k {
            a.swap(p, k);
            perm.swap(p, k);
            odd_swaps = !odd_swaps;
        }
        for i in k + 1..n {
            let l = a[i][k] / a[k][k];
            a[i][k] = l;
            for j in k + 1..n {
                a[i][j] -= l * a[k][j];
            }
        }
    }
    Some(Lu {
        lu: a,
        perm,
        odd_swaps,
    })
}

fn num_result(n: f64) -> LiteralValue {
    if n.is_finite() {
        LiteralValue::Number(n)
    } else {
        LiteralValue::Error(ExcelError::new_num())
    }
}

#[derive(Debug)]
pub struct MdetermFn;
/// Returns the determinant of a square matrix.
///
/// # Remarks
/// - Computed as Excel for Windows computes it, from an LU factorisation with
///   partial pivoting: `MDETERM({3,6,1;1,1,0;3,10,2})` is 0.9999999999999998.
/// - A matrix that is not square, or holds an empty, text or logical element,
///   returns `#VALUE!`; errors propagate. A determinant beyond the number range
///   returns `#NUM!`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Two by two"
/// formula: '=MDETERM({3,6;1,1})'
/// expected: -3
/// ```
///
/// ```yaml,docs
/// related:
///   - MINVERSE
///   - MMULT
/// faq:
///   - q: "Why is the determinant of an integer matrix not a whole number?"
///     a: "Excel computes it in floating point through an LU factorisation, and so does this function."
/// ```
impl Function for MdetermFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "MDETERM"
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
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        if args.len() != 1 {
            return Ok(CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }
        let matrix = match square_matrix(&args[0]) {
            Ok(matrix) => matrix,
            Err(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
        };
        let det = match lu_factor(matrix) {
            None => 0.0,
            Some(lu) => {
                let mut det = 1.0;
                for (k, row) in lu.lu.iter().enumerate() {
                    det *= row[k];
                }
                if lu.odd_swaps { -det } else { det }
            }
        };
        Ok(CalcValue::Scalar(num_result(det)))
    }
}

#[derive(Debug)]
pub struct MinverseFn;
/// Returns the inverse of a square matrix.
///
/// # Remarks
/// - Computed as Excel for Windows computes it: an LU factorisation with
///   partial pivoting, then forward and back substitution for each column of
///   the identity matrix.
/// - A singular matrix (a pivot of exactly zero) returns `#NUM!`; a nearly
///   singular one returns its very large inverse, as in Excel.
/// - A matrix that is not square, or holds an empty, text or logical element,
///   returns `#VALUE!`; errors propagate.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Inverse spills"
/// formula: '=INDEX(MINVERSE({4,-1;2,0}),2,2)'
/// expected: 2
/// ```
///
/// ```yaml,docs
/// related:
///   - MDETERM
///   - MMULT
///   - MUNIT
/// faq:
///   - q: "When does MINVERSE return #NUM!?"
///     a: "When elimination meets a pivot of exactly zero, or an entry of the inverse overflows."
/// ```
impl Function for MinverseFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "MINVERSE"
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
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let error = |e: ExcelError| Ok(CalcValue::Scalar(LiteralValue::Error(e)));
        if args.len() != 1 {
            return error(ExcelError::new_value());
        }
        let matrix = match square_matrix(&args[0]) {
            Ok(matrix) => matrix,
            Err(e) => return error(e),
        };
        let n = matrix.len();
        let Some(Lu { lu, perm, .. }) = lu_factor(matrix) else {
            return error(ExcelError::new_num());
        };
        let mut inverse = vec![vec![0.0; n]; n];
        let mut y = vec![0.0; n];
        let mut x = vec![0.0; n];
        for col in 0..n {
            for i in 0..n {
                let mut s = if perm[i] == col { 1.0 } else { 0.0 };
                for j in 0..i {
                    s -= lu[i][j] * y[j];
                }
                y[i] = s;
            }
            for i in (0..n).rev() {
                let mut s = y[i];
                for j in i + 1..n {
                    s -= lu[i][j] * x[j];
                }
                x[i] = s / lu[i][i];
            }
            for i in 0..n {
                if !x[i].is_finite() {
                    return error(ExcelError::new_num());
                }
                inverse[i][col] = x[i];
            }
        }
        if n == 1 {
            return Ok(CalcValue::Scalar(LiteralValue::Number(inverse[0][0])));
        }
        let rows = inverse
            .into_iter()
            .map(|row| row.into_iter().map(LiteralValue::Number).collect())
            .collect();
        Ok(crate::lift::array_result(rows, args[0].date_system()))
    }
}

/// The largest MUNIT dimension computed here (a 4,000,000-cell result).
const MAX_MUNIT: f64 = 2000.0;

#[derive(Debug)]
pub struct MunitFn;
/// Returns the identity matrix of a dimension.
///
/// # Remarks
/// - The dimension converts like any number argument (`"2"` is 2, `TRUE` is
///   1) and is truncated; below 1 it returns `#VALUE!`.
/// - A dimension above 2000 is not computed here (`#N/IMPL!`).
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Trace of the 4 by 4 identity"
/// formula: '=SUM(MUNIT(4))'
/// expected: 4
/// ```
///
/// ```yaml,docs
/// related:
///   - MINVERSE
///   - MMULT
/// faq:
///   - q: "Is a fractional dimension rounded?"
///     a: "No, it is truncated: MUNIT(2.7) is the 2 by 2 identity."
/// ```
impl Function for MunitFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "MUNIT"
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
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let error = |e: ExcelError| Ok(CalcValue::Scalar(LiteralValue::Error(e)));
        if args.len() != 1 {
            return error(ExcelError::new_value());
        }
        let n = match args[0].value()?.into_literal() {
            LiteralValue::Error(e) => return error(e),
            other => match crate::builtins::utils::coerce_num(&other) {
                Ok(n) => n.trunc(),
                Err(e) => return error(e),
            },
        };
        if n.is_nan() || n < 1.0 {
            return error(ExcelError::new_value());
        }
        if n > MAX_MUNIT {
            return error(
                ExcelError::new(ExcelErrorKind::NImpl)
                    .with_message("MUNIT above 2000 is not computed"),
            );
        }
        let n = n as usize;
        if n == 1 {
            return Ok(CalcValue::Scalar(LiteralValue::Number(1.0)));
        }
        let rows = (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| LiteralValue::Number(if i == j { 1.0 } else { 0.0 }))
                    .collect()
            })
            .collect();
        Ok(crate::lift::array_result(rows, args[0].date_system()))
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(MmultFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(MdetermFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(MinverseFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(MunitFn));
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
