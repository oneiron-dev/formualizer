use super::super::utils::{ARG_RANGE_NUM_LENIENT_ONE, coerce_num};
use super::{AggregateArgument, resolve_aggregate_argument};
use crate::args::ArgSchema;
use crate::engine::VisibilityMaskMode;
use crate::function::Function;
use crate::function_contract::FunctionDependencyContract;
use crate::traits::{ArgumentHandle, FunctionContext};
use arrow_array::Array;
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

/* ─────────────────────────── SUM() ──────────────────────────── */

#[derive(Debug)]
pub struct SumFn;

/// Adds numeric values across scalars and ranges.
///
/// `SUM` evaluates all arguments, coercing text to numbers where possible,
/// and returns the total. Blank cells and logical values in ranges are ignored.
///
/// # Remarks
/// - If any argument evaluates to an error, `SUM` propagates the first error it encounters.
/// - Unparseable text literals (e.g., `"foo"`) will result in a `#VALUE!` error.
/// - Numbers are added in order, a range row by row. Like a formula's final
///   `+`/`-`, the addition of the last number of a cell or range argument
///   compensates a cancellation to exactly 0 (`SUM(A1:A3)` is 0 for 123.45,
///   56.78 and -180.23); values given directly, such as
///   `SUM(123.45,56.78,-180.23)`, are added without compensation.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Basic scalar addition"
/// formula: "=SUM(10, 20, 5)"
/// expected: 35
/// ```
///
/// ```yaml,sandbox
/// title: "Summing a range"
/// grid:
///   A1: 10
///   A2: 20
///   A3: "N/A"
/// formula: "=SUM(A1:A3)"
/// expected: 30
/// ```
///
/// ```yaml,docs
/// related:
///   - SUMIF
///   - SUMIFS
///   - SUMPRODUCT
///   - AVERAGE
/// faq:
///   - q: "Why does SUM return #VALUE! for some text arguments?"
///     a: "Direct scalar text that cannot be parsed as a number raises #VALUE! during coercion."
///   - q: "Do text and logical values inside ranges get added?"
///     a: "No. In ranged inputs, only numeric cells contribute to the total."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: SUM
/// Type: SumFn
/// Min args: 0
/// Max args: variadic
/// Variadic: true
/// Signature: SUM(arg1...: number@range)
/// Arg schema: arg1{kinds=number,required=true,shape=range,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE, REDUCTION, NUMERIC_ONLY, STREAM_OK, PARALLEL_ARGS
/// [formualizer-docgen:schema:end]
impl Function for SumFn {
    func_caps!(PURE, REDUCTION, NUMERIC_ONLY, STREAM_OK, PARALLEL_ARGS);

    fn name(&self) -> &'static str {
        "SUM"
    }
    fn min_args(&self) -> usize {
        0
    }
    fn variadic(&self) -> bool {
        true
    }
    fn dependency_contract(&self, arity: usize) -> Option<FunctionDependencyContract> {
        FunctionDependencyContract::static_reduction(arity, self.min_args())
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_RANGE_NUM_LENIENT_ONE[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let mut total = 0.0;
        for arg in args {
            match resolve_aggregate_argument(arg, ctx)? {
                AggregateArgument::Range(view) => {
                    // Propagate errors from range first
                    for res in view.errors_slices() {
                        let (_, _, err_cols) = res?;
                        for col in err_cols {
                            if col.null_count() < col.len() {
                                for i in 0..col.len() {
                                    if !col.is_null(i) {
                                        return Ok(crate::traits::CalcValue::Scalar(
                                            LiteralValue::Error(ExcelError::new(
                                                crate::arrow_store::unmap_error_code(col.value(i)),
                                            )),
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    // Excel adds a range's numbers one by one, row by row, and
                    // compensates its addition of a reference's last number
                    // like a formula's final `+` (Microsoft, "Example when a
                    // value reaches zero"): for 2.558, -1.333 and -1.225 in
                    // A1:A3, SUM(A1:A3), SUM(A1,A2,A3) and SUM(A1,A2,A3,0) are
                    // 0. A cancellation before a reference's last number is
                    // kept (a range ending in a 0 cell keeps its 5.68E-14), and
                    // so is one by a value given directly: SUM(A1,--A2,--A3)
                    // and SUM(2.558-1.333,-1.225) keep -2.22E-16. An array is
                    // such a value.
                    let mut last = None;
                    for res in view.numbers_slices() {
                        let (_, row_len, num_cols) = res?;
                        let cols: Vec<&arrow_array::Float64Array> = num_cols
                            .iter()
                            .map(|col| col.as_ref())
                            .filter(|col| col.null_count() < col.len())
                            .collect();
                        for row in 0..row_len {
                            for col in &cols {
                                if col.is_valid(row) {
                                    let value = col.value(row);
                                    last = Some((total, value));
                                    total += value;
                                }
                            }
                        }
                    }
                    if let Some((before, value)) = last
                        && arg.resolved_as_reference()
                    {
                        total = crate::coercion::snap_cancellation(total, before, value);
                    }
                }
                AggregateArgument::ReferenceError(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
                AggregateArgument::Scalar(v) => match v {
                    LiteralValue::Error(e) => {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                    }
                    v => total += coerce_num(&v)?,
                },
            }
        }
        Ok(crate::traits::CalcValue::Scalar(
            super::super::utils::aggregate_result(total),
        ))
    }
}

/* ─────────────────────────── COUNT() ──────────────────────────── */

#[derive(Debug)]
pub struct CountFn;

/// Counts numeric values across scalars and ranges.
///
/// `COUNT` evaluates all arguments and counts how many are numeric values.
/// Numbers, dates, and text representations of numbers (when supplied directly) are counted.
///
/// # Remarks
/// - Text values inside ranges are ignored and not counted.
/// - Blank cells and logical values in ranges are ignored.
/// - Error values are not counted, whether supplied directly (`COUNT(1/0)` is 0),
///   returned by a reference that fails (`COUNT(INDIRECT("x"))` is 0) or found
///   in a range or array.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Counting mixed scalar inputs"
/// formula: "=COUNT(1, \"x\", 2, 3)"
/// expected: 3
/// ```
///
/// ```yaml,sandbox
/// title: "Counting in a range"
/// grid:
///   A1: 10
///   A2: "foo"
///   A3: 20
/// formula: "=COUNT(A1:A3)"
/// expected: 2
/// ```
///
/// ```yaml,docs
/// related:
///   - COUNTA
///   - COUNTBLANK
///   - COUNTIF
///   - COUNTIFS
/// faq:
///   - q: "Why doesn't COUNT include text in a range?"
///     a: "COUNT only counts numeric values; text cells in ranges are ignored."
///   - q: "Can direct text like \"12\" be counted?"
///     a: "Yes. Direct scalar arguments are coerced and counted when they parse as numbers."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: COUNT
/// Type: CountFn
/// Min args: 0
/// Max args: variadic
/// Variadic: true
/// Signature: COUNT(arg1...: number@range)
/// Arg schema: arg1{kinds=number,required=true,shape=range,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE, REDUCTION, NUMERIC_ONLY, STREAM_OK
/// [formualizer-docgen:schema:end]
impl Function for CountFn {
    func_caps!(PURE, REDUCTION, NUMERIC_ONLY, STREAM_OK);

    fn name(&self) -> &'static str {
        "COUNT"
    }
    fn min_args(&self) -> usize {
        0
    }
    fn variadic(&self) -> bool {
        true
    }
    fn dependency_contract(&self, arity: usize) -> Option<FunctionDependencyContract> {
        FunctionDependencyContract::static_reduction(arity, self.min_args())
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_RANGE_NUM_LENIENT_ONE[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let mut count: i64 = 0;
        for arg in args {
            // Excel: "Arguments that are error values or text that cannot be
            // translated into numbers are not counted", however the error
            // reaches COUNT: as a value (1/0), from a reference that fails
            // (INDIRECT("x"), OFFSET(A1,-1,0)) or from the evaluation itself.
            let argument = match resolve_aggregate_argument(arg, ctx) {
                Ok(argument) => argument,
                Err(error) if error.kind == ExcelErrorKind::Cancelled => return Err(error),
                Err(_) => continue,
            };
            match argument {
                AggregateArgument::Range(view) => {
                    for res in view.numbers_slices() {
                        let (_, _, num_cols) = res?;
                        for col in num_cols {
                            count += (col.len() - col.null_count()) as i64;
                        }
                    }
                }
                AggregateArgument::ReferenceError(_) => {}
                AggregateArgument::Scalar(v) => {
                    if !matches!(v, LiteralValue::Empty | LiteralValue::Error(_))
                        && coerce_num(&v).is_ok()
                    {
                        count += 1;
                    }
                }
            }
        }
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(
            count as f64,
        )))
    }
}

/* ─────────────────────────── AVERAGE() ──────────────────────────── */

#[derive(Debug)]
pub struct AverageFn;

/// Returns the arithmetic mean of numeric values across scalars and ranges.
///
/// `AVERAGE` sums numeric inputs and divides by the count of numeric values that participated.
///
/// # Remarks
/// - Errors in any scalar argument or referenced range propagate immediately.
/// - In ranges, only numeric/date-time serial values are included; text and blanks are ignored.
/// - Scalar arguments use lenient number coercion with locale support.
/// - If no numeric values are found, `AVERAGE` returns `#DIV/0!`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Average of scalar values"
/// formula: "=AVERAGE(10, 20, 5)"
/// expected: 11.666666666666666
/// ```
///
/// ```yaml,sandbox
/// title: "Average over a mixed range"
/// grid:
///   A1: 10
///   A2: "x"
///   A3: 20
/// formula: "=AVERAGE(A1:A3)"
/// expected: 15
/// ```
///
/// ```yaml,sandbox
/// title: "No numeric values returns divide-by-zero"
/// formula: "=AVERAGE(\"x\", \"\")"
/// expected: "#DIV/0!"
/// ```
///
/// ```yaml,docs
/// related:
///   - SUM
///   - COUNT
///   - AVERAGEIF
///   - AVERAGEIFS
/// faq:
///   - q: "When does AVERAGE return #DIV/0!?"
///     a: "It returns #DIV/0! when no numeric values are found after filtering/coercion."
///   - q: "Do text cells in ranges affect the denominator?"
///     a: "No. Only numeric values are counted toward the divisor."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: AVERAGE
/// Type: AverageFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: AVERAGE(arg1...: number@range)
/// Arg schema: arg1{kinds=number,required=true,shape=range,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE, REDUCTION, NUMERIC_ONLY, STREAM_OK
/// [formualizer-docgen:schema:end]
impl Function for AverageFn {
    func_caps!(PURE, REDUCTION, NUMERIC_ONLY, STREAM_OK);

    fn name(&self) -> &'static str {
        "AVERAGE"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn variadic(&self) -> bool {
        true
    }
    fn dependency_contract(&self, arity: usize) -> Option<FunctionDependencyContract> {
        FunctionDependencyContract::static_reduction(arity, self.min_args())
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_RANGE_NUM_LENIENT_ONE[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let mut sum = 0.0f64;
        let mut cnt: i64 = 0;
        for arg in args {
            match resolve_aggregate_argument(arg, ctx)? {
                AggregateArgument::Range(view) => {
                    // Propagate errors from range first
                    for res in view.errors_slices() {
                        let (_, _, err_cols) = res?;
                        for col in err_cols {
                            if col.null_count() < col.len() {
                                for i in 0..col.len() {
                                    if !col.is_null(i) {
                                        return Ok(crate::traits::CalcValue::Scalar(
                                            LiteralValue::Error(ExcelError::new(
                                                crate::arrow_store::unmap_error_code(col.value(i)),
                                            )),
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    for res in view.numbers_slices() {
                        let (_, _, num_cols) = res?;
                        for col in num_cols {
                            sum += arrow::compute::kernels::aggregate::sum(col.as_ref())
                                .unwrap_or(0.0);
                            cnt += (col.len() - col.null_count()) as i64;
                        }
                    }
                }
                AggregateArgument::ReferenceError(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
                AggregateArgument::Scalar(v) => {
                    if let LiteralValue::Error(e) = v {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                    }
                    // A value typed into the list counts; text that is no
                    // number is #VALUE! (Microsoft: "Arguments that are error
                    // values or text that cannot be translated into numbers
                    // cause errors"). Text in a range is skipped above.
                    sum += coerce_num(&v)?;
                    cnt += 1;
                }
            }
        }
        if cnt == 0 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_div(),
            )));
        }
        Ok(crate::traits::CalcValue::Scalar(
            super::super::utils::aggregate_result(sum / (cnt as f64)),
        ))
    }
}

/* ──────────────────────── SUMPRODUCT() ───────────────────────── */

#[derive(Debug)]
pub struct SumProductFn;

/// Multiplies aligned values across arrays and returns the sum of those products.
///
/// `SUMPRODUCT` supports scalar or range inputs, applies broadcast semantics, and accumulates
/// the product for each aligned cell position.
///
/// # Remarks
/// - Input shapes must be broadcast-compatible; otherwise `SUMPRODUCT` returns `#VALUE!`.
/// - Non-numeric values are treated as `0` during multiplication.
/// - Any explicit error value in the inputs propagates immediately.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Pairwise sum of products"
/// formula: "=SUMPRODUCT({1,2,3}, {4,5,6})"
/// expected: 32
/// ```
///
/// ```yaml,sandbox
/// title: "Range-based sumproduct"
/// grid:
///   A1: 2
///   A2: 3
///   A3: 4
///   B1: 10
///   B2: 20
///   B3: 30
/// formula: "=SUMPRODUCT(A1:A3, B1:B3)"
/// expected: 200
/// ```
///
/// ```yaml,sandbox
/// title: "Text entries contribute zero"
/// formula: "=SUMPRODUCT({1,\"x\",3}, {1,1,1})"
/// expected: 4
/// ```
///
/// ```yaml,docs
/// related:
///   - SUM
///   - PRODUCT
///   - MMULT
///   - SUMIFS
/// faq:
///   - q: "Why does SUMPRODUCT return #VALUE! with some array shapes?"
///     a: "The argument arrays must be broadcast-compatible; incompatible shapes raise #VALUE!."
///   - q: "How are text values handled in multiplication?"
///     a: "Non-numeric values are treated as 0, unless an explicit error is present."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: SUMPRODUCT
/// Type: SumProductFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: SUMPRODUCT(arg1...: number@range)
/// Arg schema: arg1{kinds=number,required=true,shape=range,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE, REDUCTION
/// [formualizer-docgen:schema:end]
impl Function for SumProductFn {
    // Pure reduction over arrays; uses broadcasting and lenient coercion
    func_caps!(PURE, REDUCTION);

    fn name(&self) -> &'static str {
        "SUMPRODUCT"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        // Accept ranges or scalars; numeric lenient coercion
        &ARG_RANGE_NUM_LENIENT_ONE[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        use crate::broadcast::{broadcast_shape, project_index};

        if args.is_empty() {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0)));
        }

        // Helper: materialize an argument to a 2D array of LiteralValue
        let to_array = |ah: &ArgumentHandle<'_, 'b>| -> Result<Vec<Vec<LiteralValue>>, ExcelError> {
            match resolve_aggregate_argument(ah, ctx)? {
                AggregateArgument::Range(rv) => {
                    let mut rows: Vec<Vec<LiteralValue>> = Vec::new();
                    rv.for_each_row(&mut |row| {
                        rows.push(row.to_vec());
                        Ok(())
                    })?;
                    Ok(rows)
                }
                AggregateArgument::ReferenceError(error) => Err(error),
                AggregateArgument::Scalar(v) => Ok(match v {
                    LiteralValue::Array(arr) => arr,
                    other => vec![vec![other]],
                }),
            }
        };

        // Collect arrays and shapes
        let mut arrays: Vec<Vec<Vec<LiteralValue>>> = Vec::with_capacity(args.len());
        let mut shapes: Vec<(usize, usize)> = Vec::with_capacity(args.len());
        for a in args.iter() {
            let arr = to_array(a)?;
            let shape = (arr.len(), arr.first().map(|r| r.len()).unwrap_or(0));
            arrays.push(arr);
            shapes.push(shape);
        }

        // Compute broadcast target shape across all args
        let target = match broadcast_shape(&shapes) {
            Ok(s) => s,
            Err(_) => {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new_value(),
                )));
            }
        };

        // Iterate target shape, multiply coerced values across args, sum total
        let mut total = 0.0f64;
        for r in 0..target.0 {
            for c in 0..target.1 {
                let mut prod = 1.0f64;
                for (arr, &shape) in arrays.iter().zip(shapes.iter()) {
                    let (rr, cc) = project_index((r, c), shape);
                    let lv = arr
                        .get(rr)
                        .and_then(|row| row.get(cc))
                        .cloned()
                        .unwrap_or(LiteralValue::Empty);
                    match lv {
                        LiteralValue::Error(e) => {
                            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                        }
                        _ => match crate::coercion::to_number_lenient(&lv) {
                            Ok(n) => {
                                prod *= n;
                            }
                            Err(_) => {
                                // Non-numeric -> treated as 0 in SUMPRODUCT
                                prod *= 0.0;
                            }
                        },
                    }
                }
                total += prod;
            }
        }
        Ok(crate::traits::CalcValue::Scalar(
            super::super::utils::aggregate_result(total),
        ))
    }
}

#[cfg(test)]
mod tests_sumproduct {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_parse::LiteralValue;
    use formualizer_parse::parser::{ASTNode, ASTNodeType};

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }

    fn arr(vals: Vec<Vec<LiteralValue>>) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(LiteralValue::Array(vals)), None)
    }

    fn num(n: f64) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(LiteralValue::Number(n)), None)
    }

    #[test]
    fn sumproduct_basic_pairwise() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        // {1,2,3} * {4,5,6} = 1*4 + 2*5 + 3*6 = 32
        let a = arr(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(2),
            LiteralValue::Int(3),
        ]]);
        let b = arr(vec![vec![
            LiteralValue::Int(4),
            LiteralValue::Int(5),
            LiteralValue::Int(6),
        ]]);
        let args = vec![ArgumentHandle::new(&a, &ctx), ArgumentHandle::new(&b, &ctx)];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(32.0)
        );
    }

    #[test]
    fn sumproduct_variadic_three_arrays() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        // {1,2} * {3,4} * {2,2} = (1*3*2) + (2*4*2) = 6 + 16 = 22
        let a = arr(vec![vec![LiteralValue::Int(1), LiteralValue::Int(2)]]);
        let b = arr(vec![vec![LiteralValue::Int(3), LiteralValue::Int(4)]]);
        let c = arr(vec![vec![LiteralValue::Int(2), LiteralValue::Int(2)]]);
        let args = vec![
            ArgumentHandle::new(&a, &ctx),
            ArgumentHandle::new(&b, &ctx),
            ArgumentHandle::new(&c, &ctx),
        ];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(22.0)
        );
    }

    #[test]
    fn sumproduct_broadcast_scalar_over_array() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        // {1,2,3} * 10 => (1*10 + 2*10 + 3*10) = 60
        let a = arr(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(2),
            LiteralValue::Int(3),
        ]]);
        let s = num(10.0);
        let args = vec![ArgumentHandle::new(&a, &ctx), ArgumentHandle::new(&s, &ctx)];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(60.0)
        );
    }

    #[test]
    fn sumproduct_2d_arrays_broadcast_rows_cols() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        // A is 2x2, B is 1x2 -> broadcast B across rows
        // A = [[1,2],[3,4]], B = [[10,20]]
        // sum = 1*10 + 2*20 + 3*10 + 4*20 = 10 + 40 + 30 + 80 = 160
        let a = arr(vec![
            vec![LiteralValue::Int(1), LiteralValue::Int(2)],
            vec![LiteralValue::Int(3), LiteralValue::Int(4)],
        ]);
        let b = arr(vec![vec![LiteralValue::Int(10), LiteralValue::Int(20)]]);
        let args = vec![ArgumentHandle::new(&a, &ctx), ArgumentHandle::new(&b, &ctx)];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(160.0)
        );
    }

    #[test]
    fn sumproduct_non_numeric_treated_as_zero() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        // {1,"x",3} * {1,1,1} => 1*1 + 0*1 + 3*1 = 4
        let a = arr(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Text("x".into()),
            LiteralValue::Int(3),
        ]]);
        let b = arr(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(1),
            LiteralValue::Int(1),
        ]]);
        let args = vec![ArgumentHandle::new(&a, &ctx), ArgumentHandle::new(&b, &ctx)];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(4.0)
        );
    }

    #[test]
    fn sumproduct_error_in_input_propagates() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        let a = arr(vec![vec![LiteralValue::Int(1), LiteralValue::Int(2)]]);
        let e = ASTNode::new(
            ASTNodeType::Literal(LiteralValue::Error(ExcelError::new_na())),
            None,
        );
        let args = vec![ArgumentHandle::new(&a, &ctx), ArgumentHandle::new(&e, &ctx)];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        match f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal()
        {
            LiteralValue::Error(err) => assert_eq!(err, "#N/A"),
            v => panic!("expected error, got {v:?}"),
        }
    }

    #[test]
    fn sumproduct_incompatible_shapes_value_error() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumProductFn));
        let ctx = interp(&wb);
        // 1x3 and 1x2 -> #VALUE!
        let a = arr(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(2),
            LiteralValue::Int(3),
        ]]);
        let b = arr(vec![vec![LiteralValue::Int(4), LiteralValue::Int(5)]]);
        let args = vec![ArgumentHandle::new(&a, &ctx), ArgumentHandle::new(&b, &ctx)];
        let f = ctx.context.get_function("", "SUMPRODUCT").unwrap();
        match f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal()
        {
            LiteralValue::Error(e) => assert_eq!(e, "#VALUE!"),
            v => panic!("expected value error, got {v:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use formualizer_parse::LiteralValue;

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }

    #[test]
    fn test_sum_caps() {
        let sum_fn = SumFn;
        let caps = sum_fn.caps();

        // Check that the expected capabilities are set
        assert!(caps.contains(crate::function::FnCaps::PURE));
        assert!(caps.contains(crate::function::FnCaps::REDUCTION));
        assert!(caps.contains(crate::function::FnCaps::NUMERIC_ONLY));
        assert!(caps.contains(crate::function::FnCaps::STREAM_OK));

        // Check that other caps are not set
        assert!(!caps.contains(crate::function::FnCaps::VOLATILE));
        assert!(!caps.contains(crate::function::FnCaps::ELEMENTWISE));
    }

    #[test]
    fn test_sum_basic() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SumFn));
        let ctx = interp(&wb);
        let fctx = ctx.function_context(None);

        // Test basic SUM functionality by creating ArgumentHandles manually
        let dummy_ast_1 = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Number(1.0)),
            None,
        );
        let dummy_ast_2 = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Number(2.0)),
            None,
        );
        let dummy_ast_3 = formualizer_parse::parser::ASTNode::new(
            formualizer_parse::parser::ASTNodeType::Literal(LiteralValue::Number(3.0)),
            None,
        );

        let args = vec![
            ArgumentHandle::new(&dummy_ast_1, &ctx),
            ArgumentHandle::new(&dummy_ast_2, &ctx),
            ArgumentHandle::new(&dummy_ast_3, &ctx),
        ];

        let sum_fn = ctx.context.get_function("", "SUM").unwrap();
        let result = sum_fn.dispatch(&args, &fctx).unwrap().into_literal();
        assert_eq!(result, LiteralValue::Number(6.0));
    }
}

#[cfg(test)]
mod computed_array_tests {
    use super::*;
    use crate::builtins::lookup::ChooseFn;
    use crate::builtins::math::criteria_aggregates::{
        AverageIfFn, AverageIfsFn, CountAFn, CountIfFn, CountIfsFn, SumIfFn, SumIfsFn,
    };
    use crate::builtins::math::numeric::{SeriesSumFn, SumXMY2Fn, SumsqFn};
    use crate::builtins::math::reduction::MaxFn;
    use crate::builtins::reference_fns::{IndexFn, IndirectFn, OffsetFn};
    use crate::test_workbook::TestWorkbook;
    use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn workbook() -> TestWorkbook {
        crate::builtins::lookup::register_builtins();
        TestWorkbook::new()
            .with_function(Arc::new(SumFn))
            .with_function(Arc::new(CountFn))
            .with_function(Arc::new(CountAFn))
            .with_function(Arc::new(AverageFn))
            .with_function(Arc::new(MaxFn))
            .with_function(Arc::new(SeriesSumFn))
            .with_function(Arc::new(SumsqFn))
            .with_function(Arc::new(SumXMY2Fn))
            .with_function(Arc::new(ChooseFn))
            .with_function(Arc::new(IndexFn))
            .with_function(Arc::new(OffsetFn))
            .with_function(Arc::new(IndirectFn))
            .with_function(Arc::new(CountIfFn))
            .with_function(Arc::new(SumIfFn))
            .with_function(Arc::new(AverageIfFn))
            .with_function(Arc::new(CountIfsFn))
            .with_function(Arc::new(SumIfsFn))
            .with_function(Arc::new(AverageIfsFn))
            .with_cell_a1("Sheet1", "A1", LiteralValue::Number(10.0))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Number(20.0))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Number(10.0))
    }

    fn evaluate(wb: &TestWorkbook, formula: &str) -> Result<LiteralValue, ExcelError> {
        wb.interpreter()
            .evaluate_ast(&formualizer_parse::parser::parse(formula).unwrap())
            .map(CalcValue::into_literal)
    }

    #[test]
    fn ast_aggregates_consume_computed_arrays() {
        let wb = workbook();
        let cases = [
            ("=SUM(SEQUENCE(3))", LiteralValue::Number(6.0)),
            ("=COUNT(UNIQUE(A1:A3))", LiteralValue::Number(2.0)),
            ("=COUNTA(SORT(A1:A3))", LiteralValue::Number(3.0)),
            (
                "=AVERAGE(TRANSPOSE(A1:A3))",
                LiteralValue::Number(40.0 / 3.0),
            ),
            ("=MAX((A1:A3=10)*A1:A3)", LiteralValue::Number(10.0)),
            ("=SUM(OFFSET(A1,0,0,3,1))", LiteralValue::Number(40.0)),
            ("=SUM(INDIRECT(\"A1:A3\"))", LiteralValue::Number(40.0)),
        ];

        for (formula, expected) in cases {
            assert_eq!(evaluate(&wb, formula).unwrap(), expected, "{formula}");
        }
    }

    #[test]
    fn capability_functions_fall_back_to_scalar_and_array_values() {
        let wb = workbook();
        let cases = [
            ("=SUM(CHOOSE(1,5,6))", LiteralValue::Number(5.0)),
            ("=SERIESSUM(2,0,1,CHOOSE(1,3,4))", LiteralValue::Number(3.0)),
            ("=SUMSQ(CHOOSE(1,3,4))", LiteralValue::Number(9.0)),
            (
                "=SUMXMY2(CHOOSE(1,3,4),CHOOSE(1,2,5))",
                LiteralValue::Number(1.0),
            ),
            (
                "=SUM(CHOOSE(1,SEQUENCE(3),SEQUENCE(2)))",
                LiteralValue::Number(6.0),
            ),
            ("=SUM(INDEX(SEQUENCE(3),0))", LiteralValue::Number(6.0)),
        ];

        for (formula, expected) in cases {
            assert_eq!(evaluate(&wb, formula).unwrap(), expected, "{formula}");
        }
    }

    #[test]
    fn if_family_preserves_reference_errors_in_criteria_and_targets() {
        let wb = workbook();
        let formulas = [
            "=COUNTIF(OFFSET(A1,-1,0),1)",
            "=SUMIF(OFFSET(A1,-1,0),1)",
            "=AVERAGEIF(OFFSET(A1,-1,0),1)",
            "=COUNTIFS(OFFSET(A1,-1,0),1)",
            "=SUMIFS(A1,OFFSET(A1,-1,0),1)",
            "=AVERAGEIFS(A1,OFFSET(A1,-1,0),1)",
            "=SUMIF(A1,10,OFFSET(A1,-1,0))",
            "=AVERAGEIF(A1,10,OFFSET(A1,-1,0))",
            "=SUMIFS(OFFSET(A1,-1,0),A1,10)",
            "=AVERAGEIFS(OFFSET(A1,-1,0),A1,10)",
        ];

        for formula in formulas {
            assert_exact_error_value(evaluate(&wb, formula), ExcelErrorKind::Ref, None);
        }
    }

    #[test]
    fn colon_operator_keeps_reference_semantics() {
        use formualizer_parse::parser::{ASTNode, ASTNodeType};

        let wb = workbook();
        let colon = ASTNode::new(
            ASTNodeType::BinaryOp {
                op: ":".to_string(),
                left: Box::new(formualizer_parse::parser::parse("=A1").unwrap()),
                right: Box::new(formualizer_parse::parser::parse("=A3").unwrap()),
            },
            None,
        );
        let sum = ASTNode::new(
            ASTNodeType::Function {
                name: "SUM".to_string(),
                args: vec![colon],
            },
            None,
        );
        assert_eq!(
            wb.interpreter().evaluate_ast(&sum).unwrap().into_literal(),
            LiteralValue::Number(40.0)
        );
    }

    #[derive(Debug)]
    struct CountedArrayFn(Arc<AtomicUsize>);

    impl Function for CountedArrayFn {
        fn caps(&self) -> crate::function::FnCaps {
            crate::function::FnCaps::VOLATILE | crate::function::FnCaps::MAY_SPILL
        }

        fn name(&self) -> &'static str {
            "COUNTED_ARRAY"
        }

        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<CalcValue<'b>, ExcelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(CalcValue::Scalar(LiteralValue::Array(vec![vec![
                LiteralValue::Number(2.0),
                LiteralValue::Number(3.0),
            ]])))
        }
    }

    #[derive(Debug)]
    struct PretokenizedRangeFn(crate::engine::CancelToken);

    impl Function for PretokenizedRangeFn {
        fn name(&self) -> &'static str {
            "PRETOKENIZED_RANGE"
        }

        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            ctx: &dyn FunctionContext<'b>,
        ) -> Result<CalcValue<'b>, ExcelError> {
            Ok(CalcValue::Range(
                crate::engine::range_view::RangeView::from_owned_rows(
                    vec![vec![LiteralValue::Number(1.0)]],
                    ctx.date_system(),
                )
                .with_cancel_token(Some(self.0.clone())),
            ))
        }
    }

    #[derive(Debug)]
    struct CountedScalarFn(Arc<AtomicUsize>);

    impl Function for CountedScalarFn {
        fn name(&self) -> &'static str {
            "COUNTED_SCALAR"
        }

        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<CalcValue<'b>, ExcelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(CalcValue::Scalar(LiteralValue::Number(4.0)))
        }
    }

    #[derive(Debug)]
    struct ComputedFailureFn(Arc<AtomicUsize>);

    impl Function for ComputedFailureFn {
        fn name(&self) -> &'static str {
            "COMPUTED_FAILURE"
        }

        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<CalcValue<'b>, ExcelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ExcelError::new(ExcelErrorKind::Num).with_message("computed failure sentinel"))
        }
    }

    #[derive(Debug)]
    struct ReferenceFailureFn {
        reference_calls: Arc<AtomicUsize>,
        value_calls: Arc<AtomicUsize>,
    }

    impl Function for ReferenceFailureFn {
        fn caps(&self) -> crate::function::FnCaps {
            crate::function::FnCaps::RETURNS_REFERENCE
        }

        fn name(&self) -> &'static str {
            "REFERENCE_FAILURE"
        }

        fn eval_reference<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Option<Result<formualizer_parse::parser::ReferenceType, ExcelError>> {
            self.reference_calls.fetch_add(1, Ordering::SeqCst);
            Some(Err(
                ExcelError::new(ExcelErrorKind::Ref).with_message("reference failure sentinel")
            ))
        }

        fn eval<'a, 'b, 'c>(
            &self,
            _args: &'c [ArgumentHandle<'a, 'b>],
            _ctx: &dyn FunctionContext<'b>,
        ) -> Result<CalcValue<'b>, ExcelError> {
            self.value_calls.fetch_add(1, Ordering::SeqCst);
            Ok(CalcValue::Scalar(LiteralValue::Number(999.0)))
        }
    }

    fn assert_exact_error_value(
        actual: Result<LiteralValue, ExcelError>,
        kind: ExcelErrorKind,
        message: Option<&str>,
    ) {
        let LiteralValue::Error(error) = actual.expect("reference failures are formula values")
        else {
            panic!("expected an error value");
        };
        assert_eq!(error.kind, kind);
        assert_eq!(error.message.as_deref(), message);
    }

    #[test]
    fn computed_array_is_evaluated_once_and_errors_remain_exact() {
        let array_calls = Arc::new(AtomicUsize::new(0));
        let scalar_calls = Arc::new(AtomicUsize::new(0));
        let failure_calls = Arc::new(AtomicUsize::new(0));
        let reference_calls = Arc::new(AtomicUsize::new(0));
        let reference_value_calls = Arc::new(AtomicUsize::new(0));
        let wb = workbook()
            .with_function(Arc::new(CountedArrayFn(Arc::clone(&array_calls))))
            .with_function(Arc::new(CountedScalarFn(Arc::clone(&scalar_calls))))
            .with_function(Arc::new(ComputedFailureFn(Arc::clone(&failure_calls))))
            .with_function(Arc::new(ReferenceFailureFn {
                reference_calls: Arc::clone(&reference_calls),
                value_calls: Arc::clone(&reference_value_calls),
            }));

        assert_eq!(
            evaluate(&wb, "=SUM(COUNTED_ARRAY())").unwrap(),
            LiteralValue::Number(5.0)
        );
        assert_eq!(array_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            evaluate(&wb, "=SUM(COUNTED_ARRAY()*1)").unwrap(),
            LiteralValue::Number(5.0)
        );
        assert_eq!(array_calls.load(Ordering::SeqCst), 2);

        assert_eq!(
            evaluate(&wb, "=SUM(COUNTED_SCALAR())").unwrap(),
            LiteralValue::Number(4.0)
        );
        assert_eq!(scalar_calls.load(Ordering::SeqCst), 1);

        assert_exact_error_value(
            evaluate(&wb, "=SUM(COMPUTED_FAILURE())"),
            ExcelErrorKind::Num,
            Some("computed failure sentinel"),
        );
        assert_eq!(failure_calls.load(Ordering::SeqCst), 1);

        assert_exact_error_value(
            evaluate(&wb, "=SUM(REFERENCE_FAILURE())"),
            ExcelErrorKind::Ref,
            Some("reference failure sentinel"),
        );
        assert_eq!(reference_calls.load(Ordering::SeqCst), 1);
        assert_eq!(reference_value_calls.load(Ordering::SeqCst), 0);

        assert_exact_error_value(
            evaluate(&wb, "=SUM(OFFSET(A1,-1,0))"),
            ExcelErrorKind::Ref,
            None,
        );
        assert_exact_error_value(
            evaluate(&wb, "=SUM(INDIRECT(\"not a reference\"))"),
            ExcelErrorKind::Ref,
            None,
        );
    }

    #[test]
    fn pretokenized_range_keeps_cancellation_without_context_token() {
        let token = crate::engine::CancelToken::new();
        token.cancel();
        let wb = workbook().with_function(Arc::new(PretokenizedRangeFn(token)));

        let error = evaluate(&wb, "=SUM(PRETOKENIZED_RANGE())").unwrap_err();
        assert_eq!(error.kind, ExcelErrorKind::Cancelled);
        assert_eq!(error.message, None);
    }

    #[test]
    fn computed_array_range_walk_propagates_cancellation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let token = crate::engine::CancelToken::new();
        token.cancel();
        let wb = workbook()
            .with_cancellation_token(token)
            .with_function(Arc::new(CountedArrayFn(Arc::clone(&calls))));

        let error = evaluate(&wb, "=SUM(COUNTED_ARRAY())").unwrap_err();
        assert_eq!(error.kind, ExcelErrorKind::Cancelled);
        assert_eq!(error.message, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod tests_count {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_parse::LiteralValue;
    use formualizer_parse::parser::ASTNode;
    use formualizer_parse::parser::ASTNodeType;

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }

    #[test]
    fn count_numbers_ignores_text() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(CountFn));
        let ctx = interp(&wb);
        // COUNT({1,2,"x",3}) => 3
        let arr = LiteralValue::Array(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(2),
            LiteralValue::Text("x".into()),
            LiteralValue::Int(3),
        ]]);
        let node = ASTNode::new(ASTNodeType::Literal(arr), None);
        let args = vec![ArgumentHandle::new(&node, &ctx)];
        let f = ctx.context.get_function("", "COUNT").unwrap();
        let fctx = ctx.function_context(None);
        assert_eq!(
            f.dispatch(&args, &fctx).unwrap().into_literal(),
            LiteralValue::Number(3.0)
        );
    }

    #[test]
    fn count_multiple_args_and_scalars() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(CountFn));
        let ctx = interp(&wb);
        let n1 = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(10)), None);
        let n2 = ASTNode::new(ASTNodeType::Literal(LiteralValue::Text("n".into())), None);
        let arr = LiteralValue::Array(vec![vec![LiteralValue::Int(1), LiteralValue::Int(2)]]);
        let a = ASTNode::new(ASTNodeType::Literal(arr), None);
        let args = vec![
            ArgumentHandle::new(&a, &ctx),
            ArgumentHandle::new(&n1, &ctx),
            ArgumentHandle::new(&n2, &ctx),
        ];
        let f = ctx.context.get_function("", "COUNT").unwrap();
        // Two from array + scalar 10 = 3
        let fctx = ctx.function_context(None);
        assert_eq!(
            f.dispatch(&args, &fctx).unwrap().into_literal(),
            LiteralValue::Number(3.0)
        );
    }

    /// Excel: "Arguments that are error values or text that cannot be
    /// translated into numbers are not counted", so COUNT(1/0) is 0, not
    /// #DIV/0!.
    #[test]
    fn count_direct_error_arguments_are_not_counted() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(CountFn));
        let ctx = interp(&wb);
        let error = |code: &str| {
            ASTNode::new(
                ASTNodeType::Literal(LiteralValue::Error(ExcelError::from_error_string(code))),
                None,
            )
        };
        let div0 = error("#DIV/0!");
        let na = error("#N/A");
        let one = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(1)), None);
        let f = ctx.context.get_function("", "COUNT").unwrap();
        let fctx = ctx.function_context(None);

        let args = vec![ArgumentHandle::new(&div0, &ctx)];
        assert_eq!(
            f.dispatch(&args, &fctx).unwrap().into_literal(),
            LiteralValue::Number(0.0)
        );
        let args = vec![
            ArgumentHandle::new(&na, &ctx),
            ArgumentHandle::new(&one, &ctx),
            ArgumentHandle::new(&div0, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args, &fctx).unwrap().into_literal(),
            LiteralValue::Number(1.0)
        );
    }

    #[test]
    fn count_skips_computed_error_values() {
        use crate::engine::{Engine, EvalConfig};
        use formualizer_parse::parser::parse;

        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        let cases = [
            ("=COUNT(1/0)", 0.0),
            ("=COUNT(#N/A)", 0.0),
            ("=COUNT(1,NA(),\"2\",\"x\",SQRT(-1))", 2.0),
            ("=COUNT({1,#N/A,2})", 2.0),
        ];
        for (row, (formula, _)) in cases.iter().enumerate() {
            engine
                .set_cell_formula("Sheet1", row as u32 + 1, 1, parse(formula).unwrap())
                .unwrap();
        }
        engine.evaluate_all().unwrap();
        for (row, (formula, expected)) in cases.iter().enumerate() {
            assert_eq!(
                engine.get_cell_value("Sheet1", row as u32 + 1, 1),
                Some(LiteralValue::Number(*expected)),
                "{formula}"
            );
        }
    }

    /// INDIRECT and OFFSET return the error value #REF! for a reference that
    /// does not exist: COUNT does not count it and COUNTA does, as for any
    /// other error value, while SUM, AVERAGE and MAX still return it.
    #[test]
    fn errors_from_failing_references_follow_the_error_value_rules() {
        use crate::engine::{Engine, EvalConfig};
        use formualizer_parse::parser::parse;

        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        for (row, value) in [(1, 1.0), (2, 2.0)] {
            engine
                .set_cell_value("Sheet1", row, 1, LiteralValue::Number(value))
                .unwrap();
        }
        // A number, or the error kind for `None`.
        let cases = [
            ("=COUNT(INDIRECT(\"not a ref\"))", Some(0.0)),
            ("=COUNT(1,INDIRECT(\"zz\"),2)", Some(2.0)),
            ("=COUNT(OFFSET(A1,-1,0))", Some(0.0)),
            ("=COUNT(A1:A2,INDIRECT(\"zz\"))", Some(2.0)),
            ("=COUNT(INDIRECT(\"A1:A2\"))", Some(2.0)),
            ("=COUNTA(INDIRECT(\"zz\"))", Some(1.0)),
            ("=COUNTA(1,OFFSET(A1,-1,0))", Some(2.0)),
            ("=COUNTA(A1:A2,INDIRECT(\"zz\"))", Some(3.0)),
            ("=SUM(INDIRECT(\"zz\"))", None),
            ("=SUM(1,OFFSET(A1,-1,0))", None),
            ("=AVERAGE(INDIRECT(\"zz\"))", None),
            ("=MAX(1,INDIRECT(\"zz\"))", None),
        ];
        for (row, (formula, _)) in cases.iter().enumerate() {
            engine
                .set_cell_formula("Sheet1", row as u32 + 1, 3, parse(formula).unwrap())
                .unwrap();
        }
        engine.evaluate_all().unwrap();
        for (row, (formula, expected)) in cases.iter().enumerate() {
            match (engine.get_cell_value("Sheet1", row as u32 + 1, 3), expected) {
                (Some(LiteralValue::Number(n)), Some(expected)) => {
                    assert_eq!(n, *expected, "{formula}")
                }
                (Some(LiteralValue::Error(error)), None) => {
                    assert_eq!(error.kind, ExcelErrorKind::Ref, "{formula}")
                }
                (value, _) => panic!("{formula}: {value:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests_average {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_parse::LiteralValue;
    use formualizer_parse::parser::ASTNode;
    use formualizer_parse::parser::ASTNodeType;

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }

    #[test]
    fn average_basic_numbers() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AverageFn));
        let ctx = interp(&wb);
        let arr = LiteralValue::Array(vec![vec![
            LiteralValue::Int(2),
            LiteralValue::Int(4),
            LiteralValue::Int(6),
        ]]);
        let node = ASTNode::new(ASTNodeType::Literal(arr), None);
        let args = vec![ArgumentHandle::new(&node, &ctx)];
        let f = ctx.context.get_function("", "AVERAGE").unwrap();
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(4.0)
        );
    }

    #[test]
    fn average_mixed_with_text() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AverageFn));
        let ctx = interp(&wb);
        let arr = LiteralValue::Array(vec![vec![
            LiteralValue::Int(2),
            LiteralValue::Text("x".into()),
            LiteralValue::Int(6),
        ]]);
        let node = ASTNode::new(ASTNodeType::Literal(arr), None);
        let args = vec![ArgumentHandle::new(&node, &ctx)];
        let f = ctx.context.get_function("", "AVERAGE").unwrap();
        // average of 2 and 6 = 4
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Number(4.0)
        );
    }

    #[test]
    fn average_no_numeric_div0() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AverageFn));
        let ctx = interp(&wb);
        let arr = LiteralValue::Array(vec![vec![
            LiteralValue::Text("a".into()),
            LiteralValue::Text("b".into()),
        ]]);
        let node = ASTNode::new(ASTNodeType::Literal(arr), None);
        let args = vec![ArgumentHandle::new(&node, &ctx)];
        let f = ctx.context.get_function("", "AVERAGE").unwrap();
        let fctx = ctx.function_context(None);
        match f.dispatch(&args, &fctx).unwrap().into_literal() {
            LiteralValue::Error(e) => assert_eq!(e, "#DIV/0!"),
            v => panic!("expected #DIV/0!, got {v:?}"),
        }
    }

    #[test]
    fn average_direct_error_argument_propagates() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AverageFn));
        let ctx = interp(&wb);
        let err = ASTNode::new(
            ASTNodeType::Literal(LiteralValue::Error(ExcelError::from_error_string(
                "#DIV/0!",
            ))),
            None,
        );
        let args = vec![ArgumentHandle::new(&err, &ctx)];
        let f = ctx.context.get_function("", "AVERAGE").unwrap();
        let fctx = ctx.function_context(None);
        match f.dispatch(&args, &fctx).unwrap().into_literal() {
            LiteralValue::Error(e) => assert_eq!(e, "#DIV/0!"),
            v => panic!("unexpected {v:?}"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum VisibilityPolicy {
    IncludeAll,
    ExcludeFilterHidden,
    ExcludeManualOrFilterHidden,
}

/// Whether error values in the data are skipped. This applies to the cells
/// of a referenced range and the items of an array, not to an argument that
/// is itself an error (see [`ArgumentForm`]).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ErrorPolicy {
    Propagate,
    Ignore,
}

/// How SUBTOTAL/AGGREGATE read their data arguments.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ArgumentForm {
    /// SUBTOTAL and AGGREGATE 1-13 take references. An argument that is an
    /// error value or a failed reference is not a range whose error cells
    /// an option can skip, so that error is the result under every option.
    Reference,
    /// AGGREGATE 14-19 take an array; an error argument is one more data
    /// value, which the options ignore or propagate like any other.
    Array,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum AggregateOp {
    Average,
    Count,
    CountA,
    Max,
    Min,
    Product,
    StdevSample,
    StdevPopulation,
    Sum,
    VarSample,
    VarPopulation,
}

fn aggregate_op_from_function_num(function_num: i32) -> Option<AggregateOp> {
    match function_num {
        1 => Some(AggregateOp::Average),
        2 => Some(AggregateOp::Count),
        3 => Some(AggregateOp::CountA),
        4 => Some(AggregateOp::Max),
        5 => Some(AggregateOp::Min),
        6 => Some(AggregateOp::Product),
        7 => Some(AggregateOp::StdevSample),
        8 => Some(AggregateOp::StdevPopulation),
        9 => Some(AggregateOp::Sum),
        10 => Some(AggregateOp::VarSample),
        11 => Some(AggregateOp::VarPopulation),
        _ => None,
    }
}

fn parse_strict_int_arg(arg: &ArgumentHandle<'_, '_>) -> Result<i32, ExcelError> {
    strict_int(&arg.value()?.into_literal())
}

/// A function number or option of SUBTOTAL and AGGREGATE: a whole number,
/// numeric text included ("14" is 14), or the error it holds.
pub(crate) fn strict_int(raw: &LiteralValue) -> Result<i32, ExcelError> {
    if let LiteralValue::Error(e) = raw {
        return Err(e.clone());
    }

    let n = coerce_num(raw)?;
    if !n.is_finite() {
        return Err(ExcelError::new_value());
    }

    let rounded = n.round();
    if (n - rounded).abs() > 1e-9 {
        return Err(ExcelError::new_value());
    }

    if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        return Err(ExcelError::new_value());
    }

    Ok(rounded as i32)
}

fn row_is_visible(mask: Option<&arrow_array::BooleanArray>, relative_row: usize) -> bool {
    let Some(mask) = mask else {
        return true;
    };

    if relative_row >= mask.len() || mask.is_null(relative_row) {
        return true;
    }

    mask.value(relative_row)
}

fn numeric_from_range_value(value: &LiteralValue) -> Option<f64> {
    match value {
        LiteralValue::Number(n) => Some(*n),
        LiteralValue::Int(i) => Some(*i as f64),
        LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => coerce_num(value).ok(),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct AggregateCollector {
    numeric_values: Vec<f64>,
    counta: usize,
}

impl AggregateCollector {
    #[allow(clippy::too_many_arguments)]
    fn collect_args<'a, 'b>(
        args: &[ArgumentHandle<'a, 'b>],
        start_idx: usize,
        ctx: &dyn FunctionContext<'b>,
        op: AggregateOp,
        form: ArgumentForm,
        visibility_policy: VisibilityPolicy,
        error_policy: ErrorPolicy,
        skip_nested: bool,
    ) -> Result<Self, ExcelError> {
        let mut out = Self::default();

        for arg in args.iter().skip(start_idx) {
            match resolve_aggregate_argument(arg, ctx)? {
                AggregateArgument::Range(view) => {
                    out.collect_range_arg(
                        &view,
                        ctx,
                        op,
                        visibility_policy,
                        error_policy,
                        skip_nested,
                    )?;
                }
                AggregateArgument::ReferenceError(error) => {
                    if form == ArgumentForm::Reference {
                        return Err(error);
                    }
                    out.consume_scalar_value(LiteralValue::Error(error), op, error_policy)?;
                }
                AggregateArgument::Scalar(LiteralValue::Error(error))
                    if form == ArgumentForm::Reference =>
                {
                    return Err(error);
                }
                AggregateArgument::Scalar(value) => {
                    out.consume_scalar_value(value, op, error_policy)?;
                }
            }
        }

        Ok(out)
    }

    fn collect_range_arg<'b>(
        &mut self,
        view: &crate::engine::range_view::RangeView<'_>,
        ctx: &dyn FunctionContext<'b>,
        op: AggregateOp,
        visibility_policy: VisibilityPolicy,
        error_policy: ErrorPolicy,
        skip_nested: bool,
    ) -> Result<(), ExcelError> {
        // Cells that hold SUBTOTAL or AGGREGATE formulas are left out so a
        // subtotal over subtotals does not count them twice.
        let nested = if skip_nested {
            ctx.nested_aggregate_cells(view)
                .filter(|cells| !cells.is_empty())
        } else {
            None
        };
        let visibility_mask = match visibility_policy {
            VisibilityPolicy::IncludeAll => None,
            VisibilityPolicy::ExcludeFilterHidden => {
                ctx.get_row_visibility_mask(view, VisibilityMaskMode::ExcludeFilterHidden)
            }
            VisibilityPolicy::ExcludeManualOrFilterHidden => {
                ctx.get_row_visibility_mask(view, VisibilityMaskMode::ExcludeManualOrFilterHidden)
            }
        };

        let (_, cols) = view.dims();
        if cols == 0 {
            return Ok(());
        }

        for chunk in view.iter_row_chunks() {
            let chunk = chunk?;
            for row_offset in 0..chunk.row_len {
                let rel_row = chunk.row_start + row_offset;
                if !row_is_visible(visibility_mask.as_deref(), rel_row) {
                    continue;
                }

                for col in 0..cols {
                    if nested
                        .as_ref()
                        .is_some_and(|cells| cells.contains(&(rel_row, col)))
                    {
                        continue;
                    }
                    self.consume_range_value(view.get_cell(rel_row, col), op, error_policy)?;
                }
            }
        }

        Ok(())
    }

    fn consume_range_value(
        &mut self,
        value: LiteralValue,
        op: AggregateOp,
        error_policy: ErrorPolicy,
    ) -> Result<(), ExcelError> {
        match value {
            LiteralValue::Error(e) => {
                if op == AggregateOp::CountA {
                    if error_policy == ErrorPolicy::Ignore {
                        return Ok(());
                    }
                    self.counta += 1;
                    return Ok(());
                }
                match error_policy {
                    ErrorPolicy::Propagate => Err(e),
                    ErrorPolicy::Ignore => Ok(()),
                }
            }
            LiteralValue::Empty => Ok(()),
            other => {
                self.counta += 1;
                if let Some(n) = numeric_from_range_value(&other) {
                    self.numeric_values.push(n);
                }
                Ok(())
            }
        }
    }

    fn consume_scalar_value(
        &mut self,
        value: LiteralValue,
        op: AggregateOp,
        error_policy: ErrorPolicy,
    ) -> Result<(), ExcelError> {
        match value {
            LiteralValue::Error(e) => {
                if op == AggregateOp::CountA {
                    if error_policy == ErrorPolicy::Ignore {
                        return Ok(());
                    }
                    self.counta += 1;
                    return Ok(());
                }
                match error_policy {
                    ErrorPolicy::Propagate => Err(e),
                    ErrorPolicy::Ignore => Ok(()),
                }
            }
            LiteralValue::Array(rows) => {
                for row in rows {
                    for cell in row {
                        self.consume_range_value(cell, op, error_policy)?;
                    }
                }
                Ok(())
            }
            other => {
                match op {
                    AggregateOp::CountA => {
                        if !matches!(other, LiteralValue::Empty) {
                            self.counta += 1;
                        }
                    }
                    AggregateOp::Count => {
                        if !matches!(other, LiteralValue::Empty) && coerce_num(&other).is_ok() {
                            self.numeric_values.push(0.0);
                        }
                    }
                    _ => {
                        if let Ok(n) = coerce_num(&other) {
                            self.numeric_values.push(n);
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn variance(values: &[f64], sample: bool) -> Result<f64, ExcelError> {
        let n = values.len();
        if sample {
            if n < 2 {
                return Err(ExcelError::new_div());
            }
        } else if n == 0 {
            return Err(ExcelError::new_div());
        }

        let mean = values.iter().copied().sum::<f64>() / (n as f64);
        let mut ss = 0.0;
        for value in values {
            let d = *value - mean;
            ss += d * d;
        }

        if sample {
            Ok(ss / ((n - 1) as f64))
        } else {
            Ok(ss / (n as f64))
        }
    }

    fn finalize(self, op: AggregateOp) -> LiteralValue {
        use super::super::utils::aggregate_result;
        match op {
            AggregateOp::Average => {
                if self.numeric_values.is_empty() {
                    LiteralValue::Error(ExcelError::new_div())
                } else {
                    let sum = self.numeric_values.iter().copied().sum::<f64>();
                    aggregate_result(sum / (self.numeric_values.len() as f64))
                }
            }
            // Counts cannot overflow to non-finite; keep them branch-free.
            AggregateOp::Count => LiteralValue::Number(self.numeric_values.len() as f64),
            AggregateOp::CountA => LiteralValue::Number(self.counta as f64),
            AggregateOp::Max => aggregate_result(
                self.numeric_values
                    .iter()
                    .copied()
                    .reduce(f64::max)
                    .unwrap_or(0.0),
            ),
            AggregateOp::Min => aggregate_result(
                self.numeric_values
                    .iter()
                    .copied()
                    .reduce(f64::min)
                    .unwrap_or(0.0),
            ),
            AggregateOp::Product => {
                if self.numeric_values.is_empty() {
                    LiteralValue::Number(0.0)
                } else {
                    aggregate_result(self.numeric_values.iter().copied().product::<f64>())
                }
            }
            AggregateOp::StdevSample => match Self::variance(&self.numeric_values, true) {
                Ok(v) => aggregate_result(v.sqrt()),
                Err(e) => LiteralValue::Error(e),
            },
            AggregateOp::StdevPopulation => match Self::variance(&self.numeric_values, false) {
                Ok(v) => aggregate_result(v.sqrt()),
                Err(e) => LiteralValue::Error(e),
            },
            AggregateOp::Sum => aggregate_result(self.numeric_values.iter().copied().sum()),
            AggregateOp::VarSample => match Self::variance(&self.numeric_values, true) {
                Ok(v) => aggregate_result(v),
                Err(e) => LiteralValue::Error(e),
            },
            AggregateOp::VarPopulation => match Self::variance(&self.numeric_values, false) {
                Ok(v) => aggregate_result(v),
                Err(e) => LiteralValue::Error(e),
            },
        }
    }
}

#[derive(Debug)]
pub struct SubtotalFn;

/// [formualizer-docgen:schema:start]
/// Name: SUBTOTAL
/// Type: SubtotalFn
/// Min args: 2
/// Max args: variadic
/// Variadic: true
/// Signature: SUBTOTAL(arg1...: number@range)
/// Arg schema: arg1{kinds=number,required=true,shape=range,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: VOLATILE, REDUCTION, NUMERIC_ONLY, STREAM_OK
/// [formualizer-docgen:schema:end]
impl Function for SubtotalFn {
    func_caps!(VOLATILE, REDUCTION, NUMERIC_ONLY, STREAM_OK);

    fn name(&self) -> &'static str {
        "SUBTOTAL"
    }

    fn min_args(&self) -> usize {
        2
    }

    fn variadic(&self) -> bool {
        true
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_RANGE_NUM_LENIENT_ONE[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.len() < 2 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        let function_num = match parse_strict_int_arg(&args[0]) {
            Ok(v) => v,
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        // Rows a filter hides are always left out; 101-111 also leave out
        // rows hidden by hand.
        let (mapped_code, visibility) = if (1..=11).contains(&function_num) {
            (function_num, VisibilityPolicy::ExcludeFilterHidden)
        } else if (101..=111).contains(&function_num) {
            (
                function_num - 100,
                VisibilityPolicy::ExcludeManualOrFilterHidden,
            )
        } else {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        };

        let Some(op) = aggregate_op_from_function_num(mapped_code) else {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        };

        let collected = match AggregateCollector::collect_args(
            args,
            1,
            ctx,
            op,
            ArgumentForm::Reference,
            visibility,
            ErrorPolicy::Propagate,
            true,
        ) {
            Ok(c) => c,
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        Ok(crate::traits::CalcValue::Scalar(collected.finalize(op)))
    }
}

#[derive(Debug)]
pub struct AggregateFn;

/// [formualizer-docgen:schema:start]
/// Name: AGGREGATE
/// Type: AggregateFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: AGGREGATE(arg1...: number@range)
/// Arg schema: arg1{kinds=number,required=true,shape=range,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: VOLATILE, REDUCTION, NUMERIC_ONLY, STREAM_OK
/// [formualizer-docgen:schema:end]
impl Function for AggregateFn {
    func_caps!(VOLATILE, REDUCTION, NUMERIC_ONLY, STREAM_OK);

    fn name(&self) -> &'static str {
        "AGGREGATE"
    }

    fn min_args(&self) -> usize {
        3
    }

    fn variadic(&self) -> bool {
        true
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        &ARG_RANGE_NUM_LENIENT_ONE[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.len() < 3 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        let function_num = match parse_strict_int_arg(&args[0]) {
            Ok(v) => v,
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        if !(1..=19).contains(&function_num) {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_value(),
            )));
        }

        let options = match parse_strict_int_arg(&args[1]) {
            Ok(v) => v,
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        // Options 0-3 also skip nested SUBTOTAL/AGGREGATE results; 4-7 are
        // the same hidden-row/error choices without that exclusion.
        let skip_nested = (0..=3).contains(&options);
        let (visibility, error_policy) = match options {
            0 | 4 => (VisibilityPolicy::IncludeAll, ErrorPolicy::Propagate),
            1 | 5 => (
                VisibilityPolicy::ExcludeManualOrFilterHidden,
                ErrorPolicy::Propagate,
            ),
            2 | 6 => (VisibilityPolicy::IncludeAll, ErrorPolicy::Ignore),
            3 | 7 => (
                VisibilityPolicy::ExcludeManualOrFilterHidden,
                ErrorPolicy::Ignore,
            ),
            _ => {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new_value(),
                )));
            }
        };

        if let Some(op) = aggregate_op_from_function_num(function_num) {
            let collected = match AggregateCollector::collect_args(
                args,
                2,
                ctx,
                op,
                ArgumentForm::Reference,
                visibility,
                error_policy,
                skip_nested,
            ) {
                Ok(c) => c,
                Err(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
            };
            return Ok(crate::traits::CalcValue::Scalar(collected.finalize(op)));
        }

        // 12 MEDIAN and 13 MODE.SNGL take references; 14-19 are the array
        // form AGGREGATE(function_num, options, array, k). A multi-cell k is
        // lifted before this call, one call per element, like
        // LARGE(array,{1,2}) (see `lift::value_lift_spec`).
        let (data_args, form, k) = if function_num <= 13 {
            (args, ArgumentForm::Reference, None)
        } else {
            if args.len() != 4 {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new_value(),
                )));
            }
            match aggregate_k(args[3].value()?.into_literal()) {
                Ok(k) => (&args[..3], ArgumentForm::Array, Some(k)),
                Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
            }
        };
        let collected = match AggregateCollector::collect_args(
            data_args,
            2,
            ctx,
            AggregateOp::Sum,
            form,
            visibility,
            error_policy,
            skip_nested,
        ) {
            Ok(c) => c,
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };
        let result = aggregate_order_statistic(function_num, collected.numeric_values, k);
        Ok(crate::traits::CalcValue::Scalar(match result {
            Ok(n) => LiteralValue::Number(n),
            Err(e) => LiteralValue::Error(e),
        }))
    }
}

/// The k (or quart) of AGGREGATE's array form: an error is the result, and a
/// one-cell array is its value. A larger array is lifted before this call
/// (one call per element); one that reaches it unlifted is `#VALUE!`, never
/// its first element.
fn aggregate_k(k: LiteralValue) -> Result<f64, ExcelError> {
    match k {
        LiteralValue::Error(e) => Err(e),
        LiteralValue::Array(rows) => {
            let mut cells = rows.into_iter().flatten();
            match (cells.next(), cells.next()) {
                (Some(cell), None) => aggregate_k(cell),
                _ => Err(ExcelError::new_value()),
            }
        }
        other => coerce_num(&other),
    }
}

/// AGGREGATE functions 12-19 over the collected numbers.
fn aggregate_order_statistic(
    function_num: i32,
    mut nums: Vec<f64>,
    k: Option<f64>,
) -> Result<f64, ExcelError> {
    use crate::builtins::stats::{nth_smallest, percentile_exc, percentile_inc};
    let k = k.unwrap_or(0.0);
    match function_num {
        12 => {
            if nums.is_empty() {
                return Err(ExcelError::new_num());
            }
            nums.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = nums.len();
            Ok(if n % 2 == 1 {
                nums[n / 2]
            } else {
                (nums[n / 2 - 1] + nums[n / 2]) / 2.0
            })
        }
        13 => {
            // Most frequent value; ties go to the value seen first.
            let mut best: Option<(f64, usize)> = None;
            for (i, &v) in nums.iter().enumerate() {
                if nums[..i].contains(&v) {
                    continue;
                }
                let count = nums[i..].iter().filter(|&&x| x == v).count();
                if count > 1 && best.is_none_or(|(_, c)| count > c) {
                    best = Some((v, count));
                }
            }
            best.map(|(v, _)| v).ok_or_else(ExcelError::new_na)
        }
        14 | 15 => {
            let k = k.trunc();
            // A NaN k (no Excel number) fails every bound, like k < 1.
            if k.is_nan() || k < 1.0 || k as usize > nums.len() {
                return Err(ExcelError::new_num());
            }
            let k = k as usize;
            let index = if function_num == 14 {
                nums.len() - k
            } else {
                k - 1
            };
            Ok(nth_smallest(&mut nums, index))
        }
        16 => percentile_inc(&mut nums, k),
        18 => percentile_exc(&mut nums, k),
        17 | 19 => {
            let quart = k.trunc();
            let valid = if function_num == 17 {
                0.0..=4.0
            } else {
                1.0..=3.0
            };
            if !valid.contains(&quart) {
                return Err(ExcelError::new_num());
            }
            if function_num == 17 {
                percentile_inc(&mut nums, quart / 4.0)
            } else {
                percentile_exc(&mut nums, quart / 4.0)
            }
        }
        _ => Err(ExcelError::new_value()),
    }
}

#[cfg(test)]
mod tests_subtotal_aggregate {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_common::{ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::{ASTNode, ASTNodeType};

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }

    fn lit(value: LiteralValue) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(value), None)
    }

    fn dispatch(
        ctx: &crate::interpreter::Interpreter<'_>,
        fn_name: &str,
        nodes: &[ASTNode],
    ) -> LiteralValue {
        let args: Vec<_> = nodes.iter().map(|n| ArgumentHandle::new(n, ctx)).collect();
        let f = ctx.context.get_function("", fn_name).expect("function");
        f.dispatch(&args, &ctx.function_context(None))
            .expect("dispatch")
            .into_literal()
    }

    fn assert_num_close(value: LiteralValue, expected: f64) {
        match value {
            LiteralValue::Number(n) => assert!((n - expected).abs() < 1e-9, "{n} != {expected}"),
            LiteralValue::Int(i) => assert!(((i as f64) - expected).abs() < 1e-9),
            other => panic!("expected numeric {expected}, got {other:?}"),
        }
    }

    fn assert_error_kind(value: LiteralValue, expected: ExcelErrorKind) {
        match value {
            LiteralValue::Error(e) => assert_eq!(e.kind, expected),
            other => panic!("expected error {:?}, got {other:?}", expected),
        }
    }

    #[test]
    fn subtotal_function_num_mapping_basics() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SubtotalFn));
        let ctx = interp(&wb);
        let values = LiteralValue::Array(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(2),
            LiteralValue::Int(3),
        ]]);

        let cases: &[(i64, f64)] = &[
            (1, 2.0),
            (2, 3.0),
            (3, 3.0),
            (4, 3.0),
            (5, 1.0),
            (6, 6.0),
            (7, 1.0),
            (8, (2.0f64 / 3.0).sqrt()),
            (9, 6.0),
            (10, 1.0),
            (11, 2.0 / 3.0),
        ];

        for (code, expected) in cases {
            let args = vec![lit(LiteralValue::Int(*code)), lit(values.clone())];
            let out = dispatch(&ctx, "SUBTOTAL", &args);
            assert_num_close(out, *expected);
        }
    }

    #[test]
    fn subtotal_counta_counts_errors_as_non_empty() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SubtotalFn));
        let ctx = interp(&wb);

        let args = vec![
            lit(LiteralValue::Int(3)),
            lit(LiteralValue::Array(vec![vec![
                LiteralValue::Int(1),
                LiteralValue::Error(ExcelError::new_div()),
                LiteralValue::Text("x".into()),
                LiteralValue::Text("".into()),
            ]])),
        ];
        let out = dispatch(&ctx, "SUBTOTAL", &args);
        assert_num_close(out, 4.0);
    }

    #[test]
    fn subtotal_invalid_function_num_returns_value_error() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SubtotalFn));
        let ctx = interp(&wb);

        let args = vec![
            lit(LiteralValue::Number(9.5)),
            lit(LiteralValue::Array(vec![vec![LiteralValue::Int(1)]])),
        ];
        let out = dispatch(&ctx, "SUBTOTAL", &args);
        assert_error_kind(out, ExcelErrorKind::Value);
    }

    #[test]
    fn subtotal_requires_ref_argument() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(SubtotalFn));
        let ctx = interp(&wb);

        let out = dispatch(&ctx, "SUBTOTAL", &[lit(LiteralValue::Int(9))]);
        assert_error_kind(out, ExcelErrorKind::Value);
    }

    #[test]
    fn aggregate_requires_options_and_ref_argument() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);

        let out = dispatch(
            &ctx,
            "AGGREGATE",
            &[lit(LiteralValue::Int(9)), lit(LiteralValue::Int(0))],
        );
        assert_error_kind(out, ExcelErrorKind::Value);
    }

    #[test]
    fn aggregate_options_zero_to_three_control_error_behavior() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);
        let values = LiteralValue::Array(vec![vec![
            LiteralValue::Int(10),
            LiteralValue::Error(ExcelError::new_div()),
            LiteralValue::Int(30),
        ]]);

        let opt0 = dispatch(
            &ctx,
            "AGGREGATE",
            &[
                lit(LiteralValue::Int(9)),
                lit(LiteralValue::Int(0)),
                lit(values.clone()),
            ],
        );
        assert_error_kind(opt0, ExcelErrorKind::Div);

        let opt1 = dispatch(
            &ctx,
            "AGGREGATE",
            &[
                lit(LiteralValue::Int(9)),
                lit(LiteralValue::Int(1)),
                lit(values.clone()),
            ],
        );
        assert_error_kind(opt1, ExcelErrorKind::Div);

        let opt2 = dispatch(
            &ctx,
            "AGGREGATE",
            &[
                lit(LiteralValue::Int(9)),
                lit(LiteralValue::Int(2)),
                lit(values.clone()),
            ],
        );
        assert_num_close(opt2, 40.0);

        let opt3 = dispatch(
            &ctx,
            "AGGREGATE",
            &[
                lit(LiteralValue::Int(9)),
                lit(LiteralValue::Int(3)),
                lit(values),
            ],
        );
        assert_num_close(opt3, 40.0);
    }

    #[test]
    fn aggregate_counta_option_ignore_errors_skips_error_values() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);

        let out = dispatch(
            &ctx,
            "AGGREGATE",
            &[
                lit(LiteralValue::Int(3)),
                lit(LiteralValue::Int(2)),
                lit(LiteralValue::Array(vec![vec![
                    LiteralValue::Int(1),
                    LiteralValue::Error(ExcelError::new_div()),
                    LiteralValue::Text("x".into()),
                ]])),
            ],
        );
        assert_num_close(out, 2.0);
    }

    fn aggregate(
        ctx: &crate::interpreter::Interpreter<'_>,
        args: Vec<LiteralValue>,
    ) -> LiteralValue {
        let nodes: Vec<ASTNode> = args.into_iter().map(lit).collect();
        dispatch(ctx, "AGGREGATE", &nodes)
    }

    fn data() -> LiteralValue {
        // {5, #N/A, 3, 8, 3}
        LiteralValue::Array(vec![vec![
            LiteralValue::Int(5),
            LiteralValue::Error(ExcelError::new_na()),
            LiteralValue::Int(3),
            LiteralValue::Int(8),
            LiteralValue::Int(3),
        ]])
    }

    #[test]
    fn aggregate_options_four_to_seven_choose_error_handling() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);
        let sum = |option| {
            aggregate(
                &ctx,
                vec![LiteralValue::Int(9), LiteralValue::Int(option), data()],
            )
        };
        assert_error_kind(sum(4), ExcelErrorKind::Na);
        assert_error_kind(sum(5), ExcelErrorKind::Na);
        assert_num_close(sum(6), 19.0);
        assert_num_close(sum(7), 19.0);
    }

    #[test]
    fn aggregate_order_statistics_array_form() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);
        let call = |f: i64, k: Option<f64>| {
            let mut args = vec![LiteralValue::Int(f), LiteralValue::Int(6), data()];
            args.extend(k.map(LiteralValue::Number));
            aggregate(&ctx, args)
        };
        assert_num_close(call(12, None), 4.0); // MEDIAN {3,3,5,8}
        assert_num_close(call(13, None), 3.0); // MODE.SNGL
        assert_num_close(call(14, Some(1.0)), 8.0); // LARGE
        assert_num_close(call(15, Some(2.0)), 3.0); // SMALL
        assert_num_close(call(16, Some(0.5)), 4.0); // PERCENTILE.INC
        assert_num_close(call(17, Some(3.0)), 5.75); // QUARTILE.INC
        assert_num_close(call(18, Some(0.5)), 4.0); // PERCENTILE.EXC
        assert_num_close(call(19, Some(1.0)), 3.0); // QUARTILE.EXC
        assert_error_kind(call(15, Some(5.0)), ExcelErrorKind::Num);
        assert_error_kind(call(14, None), ExcelErrorKind::Value);
        // Without ignoring errors the #N/A propagates.
        let strict = aggregate(
            &ctx,
            vec![
                LiteralValue::Int(15),
                LiteralValue::Int(4),
                data(),
                LiteralValue::Int(1),
            ],
        );
        assert_error_kind(strict, ExcelErrorKind::Na);
    }

    #[test]
    fn aggregate_array_form_lifts_array_k() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);
        // Each result cell as text: numbers as is, errors by kind.
        let call = |f: i64, option: i64, k: Vec<Vec<LiteralValue>>| -> Vec<Vec<String>> {
            let k = LiteralValue::Array(k);
            let args = vec![LiteralValue::Int(f), LiteralValue::Int(option), data(), k];
            let cells = match aggregate(&ctx, args) {
                LiteralValue::Array(rows) => rows,
                other => vec![vec![other]],
            };
            cells
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|cell| match cell {
                            LiteralValue::Number(n) => n.to_string(),
                            LiteralValue::Error(e) => e.kind.to_string(),
                            other => format!("{other:?}"),
                        })
                        .collect()
                })
                .collect()
        };
        let int = LiteralValue::Int;
        // LARGE(data,{1,2}) and SMALL(data,{1;2}): one result per k, in k's shape.
        assert_eq!(call(14, 6, vec![vec![int(1), int(2)]]), [["8", "5"]]);
        assert_eq!(
            call(15, 6, vec![vec![int(1)], vec![int(2)]]),
            [["3"], ["3"]]
        );
        assert_eq!(call(17, 6, vec![vec![int(0), int(4)]]), [["3", "8"]]);
        // Each element is checked on its own: a bad k is that element's error.
        assert_eq!(
            call(
                14,
                6,
                vec![vec![
                    int(1),
                    int(9),
                    LiteralValue::Error(ExcelError::new_div()),
                    LiteralValue::Text("x".into()),
                ]]
            ),
            [["8", "#NUM!", "#DIV/0!", "#VALUE!"]]
        );
        // A data error that is not ignored is each element's result.
        assert_eq!(call(14, 4, vec![vec![int(1), int(2)]]), [["#N/A", "#N/A"]]);
        // A 1x1 array k is a scalar k.
        assert_eq!(call(14, 6, vec![vec![int(2)]]), [["5"]]);
    }

    #[test]
    fn aggregate_array_form_reads_text_function_number_and_rejects_nan_k() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(AggregateFn));
        let ctx = interp(&wb);
        let text = |s: &str| LiteralValue::Text(s.into());
        let num = LiteralValue::Number;
        let row = |cells: Vec<LiteralValue>| LiteralValue::Array(vec![cells]);
        let numbers = || row(vec![num(1.0), num(2.0), num(3.0)]);
        let kinds = |value: LiteralValue| -> Vec<String> {
            let cells = match value {
                LiteralValue::Array(rows) => rows.into_iter().flatten().collect(),
                other => vec![other],
            };
            cells
                .into_iter()
                .map(|cell| match cell {
                    LiteralValue::Number(n) => n.to_string(),
                    LiteralValue::Error(e) => e.kind.to_string(),
                    other => format!("{other:?}"),
                })
                .collect()
        };
        // A function number given as numeric text still selects the array
        // form, so a multi-cell k is evaluated element by element in its shape.
        for (f, expected) in [("14", ["3", "2"]), ("15", ["1", "2"])] {
            let args = vec![text(f), num(6.0), numbers(), row(vec![num(1.0), num(2.0)])];
            assert_eq!(kinds(aggregate(&ctx, args)), expected, "{f}");
        }
        let args = vec![
            text("14"),
            num(6.0),
            numbers(),
            row(vec![num(1.0), LiteralValue::Error(ExcelError::new_na())]),
        ];
        assert_eq!(kinds(aggregate(&ctx, args)), ["3", "#N/A"]);
        // "NaN" is no numeric text: that element is #VALUE! (option 6 ignores
        // errors in the data, not an invalid k), the others still compute.
        for (f, expected) in [("14", ["#VALUE!", "3"]), ("15", ["#VALUE!", "1"])] {
            let args = vec![
                text(f),
                num(6.0),
                numbers(),
                row(vec![text("NaN"), num(1.0)]),
            ];
            assert_eq!(kinds(aggregate(&ctx, args)), expected, "{f}");
        }
        // A numeric NaN k (no Excel number) is out of range rather than an index.
        for f in 14..=19 {
            assert_eq!(
                aggregate_order_statistic(f, vec![1.0, 2.0, 3.0], Some(f64::NAN))
                    .map_err(|e| e.kind),
                Err(ExcelErrorKind::Num),
                "{f}"
            );
        }
        // A multi-cell k that reaches the evaluation unlifted is #VALUE!, not
        // its first element; a one-cell array is its value.
        assert_eq!(
            aggregate_k(LiteralValue::Array(vec![vec![num(1.0), num(2.0)]])).map_err(|e| e.kind),
            Err(ExcelErrorKind::Value)
        );
        assert_eq!(
            aggregate_k(LiteralValue::Array(vec![vec![num(1.0)], vec![num(2.0)]]))
                .map_err(|e| e.kind),
            Err(ExcelErrorKind::Value)
        );
        assert_eq!(
            aggregate_k(LiteralValue::Array(vec![vec![num(2.0)]])),
            Ok(2.0)
        );
    }
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(SumProductFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(SumFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(CountFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(AverageFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(SubtotalFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(AggregateFn));
}
