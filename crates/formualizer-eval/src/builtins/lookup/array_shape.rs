//! Dynamic array shape helpers: TOCOL, TOROW, EXPAND, WRAPROWS and WRAPCOLS.

use super::super::utils::collapse_if_scalar;
use crate::args::{ArgSchema, CoercionPolicy, ShapeKind};
use crate::function::Function;
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{ArgKind, ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

/// Converts an array or range into a single column.
///
/// Flattens input values into one column, with options to ignore blanks/errors
/// and to scan by row or by column.
///
/// # Remarks
/// - `ignore` values are `0` keep all, `1` ignore blanks, `2` ignore errors, `3` ignore both.
/// - `scan_by_column` defaults to FALSE, so values are read row by row.
///
/// ```yaml,sandbox
/// title: "Flatten row by row"
/// formula: "=TOCOL({1,2;3,4})"
/// expected: [[1],[2],[3],[4]]
/// ```
///
/// ```yaml,sandbox
/// title: "Scan by column"
/// formula: "=TOCOL({1,2;3,4},0,TRUE)"
/// expected: [[1],[3],[2],[4]]
/// ```
///
/// ```yaml,docs
/// related:
///   - TOROW
///   - HSTACK
///   - VSTACK
/// faq:
///   - q: "Can TOCOL filter blanks and errors?"
///     a: "Yes. Use the ignore argument to drop blanks, errors, or both."
/// ```
#[derive(Debug)]
pub struct ToColFn;

/// Converts an array or range into a single row.
///
/// Flattens input values into one row, with options to ignore blanks/errors and
/// to scan by row or by column.
///
/// # Remarks
/// - `ignore` values are `0` keep all, `1` ignore blanks, `2` ignore errors, `3` ignore both.
/// - `scan_by_column` defaults to FALSE, so values are read row by row.
///
/// ```yaml,sandbox
/// title: "Flatten to row"
/// formula: "=TOROW({1,2;3,4})"
/// expected: [[1,2,3,4]]
/// ```
///
/// ```yaml,sandbox
/// title: "Scan by column"
/// formula: "=TOROW({1,2;3,4},0,TRUE)"
/// expected: [[1,3,2,4]]
/// ```
///
/// ```yaml,docs
/// related:
///   - TOCOL
///   - HSTACK
///   - VSTACK
/// faq:
///   - q: "Does TOROW preserve row order by default?"
///     a: "Yes. The default scan order is row-major."
/// ```
#[derive(Debug)]
pub struct ToRowFn;

fn schema() -> &'static [ArgSchema] {
    use once_cell::sync::Lazy;
    static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
        vec![
            ArgSchema {
                kinds: smallvec::smallvec![ArgKind::Range, ArgKind::Any],
                required: true,
                by_ref: false,
                shape: ShapeKind::Range,
                coercion: CoercionPolicy::None,
                max: None,
                repeating: None,
                default: None,
            },
            ArgSchema {
                kinds: smallvec::smallvec![ArgKind::Number],
                required: false,
                by_ref: false,
                shape: ShapeKind::Scalar,
                coercion: CoercionPolicy::NumberLenientText,
                max: None,
                repeating: None,
                default: Some(LiteralValue::Int(0)),
            },
            ArgSchema {
                kinds: smallvec::smallvec![ArgKind::Logical, ArgKind::Number],
                required: false,
                by_ref: false,
                shape: ShapeKind::Scalar,
                coercion: CoercionPolicy::None,
                max: None,
                repeating: None,
                default: Some(LiteralValue::Boolean(false)),
            },
        ]
    });
    &SCHEMA
}

fn materialize_arg<'b>(arg: &ArgumentHandle<'_, 'b>) -> Result<Vec<Vec<LiteralValue>>, ExcelError> {
    if let Ok(view) = arg.range_view() {
        let mut rows = Vec::new();
        view.for_each_row(&mut |row| {
            rows.push(row.to_vec());
            Ok(())
        })?;
        return Ok(rows);
    }

    Ok(match arg.value()?.into_literal() {
        LiteralValue::Array(rows) => rows,
        v => vec![vec![v]],
    })
}

fn ignore_mode<'b>(args: &[ArgumentHandle<'_, 'b>]) -> Result<i64, ExcelError> {
    if args.len() < 2 {
        return Ok(0);
    }
    let raw = args[1].value()?.into_literal();
    let n = match raw {
        LiteralValue::Int(i) => i,
        LiteralValue::Number(n) => n as i64,
        LiteralValue::Error(e) => return Err(e),
        other => crate::coercion::to_number_argument(&other)? as i64,
    };
    if !(0..=3).contains(&n) {
        return Err(
            ExcelError::new(ExcelErrorKind::Value).with_message("ignore must be 0, 1, 2, or 3")
        );
    }
    Ok(n)
}

fn scan_by_column<'b>(args: &[ArgumentHandle<'_, 'b>]) -> Result<bool, ExcelError> {
    if args.len() < 3 {
        return Ok(false);
    }
    crate::coercion::to_logical(&args[2].value()?.into_literal())
}

fn include_cell(v: &LiteralValue, ignore: i64) -> bool {
    let is_blank = matches!(v, LiteralValue::Empty);
    let is_error = matches!(v, LiteralValue::Error(_));
    match ignore {
        1 => !is_blank,
        2 => !is_error,
        3 => !is_blank && !is_error,
        _ => true,
    }
}

fn flatten_array<'b>(args: &[ArgumentHandle<'_, 'b>]) -> Result<Vec<LiteralValue>, ExcelError> {
    if args.is_empty() || args.len() > 3 {
        return Err(ExcelError::new(ExcelErrorKind::Value));
    }
    let data = materialize_arg(&args[0])?;
    let ignore = ignore_mode(args)?;
    let scan_by_col = scan_by_column(args)?;

    let rows = data.len();
    let cols = data.iter().map(Vec::len).max().unwrap_or(0);
    let mut flat = Vec::with_capacity(rows.saturating_mul(cols));

    if scan_by_col {
        for c in 0..cols {
            for row in &data {
                let v = row.get(c).cloned().unwrap_or(LiteralValue::Empty);
                if include_cell(&v, ignore) {
                    flat.push(v);
                }
            }
        }
    } else {
        for row in &data {
            for c in 0..cols {
                let v = row.get(c).cloned().unwrap_or(LiteralValue::Empty);
                if include_cell(&v, ignore) {
                    flat.push(v);
                }
            }
        }
    }

    if flat.is_empty() {
        return Err(ExcelError::new(ExcelErrorKind::Calc)
            .with_message("TOCOL/TOROW returned an empty array"));
    }
    Ok(flat)
}

/// [formualizer-docgen:schema:start]
/// Name: TOCOL
/// Type: ToColFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: TOCOL(arg1: range|any@range, arg2?: number@scalar, arg3?...: logical|number@scalar)
/// Arg schema: arg1{kinds=range|any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=true}; arg3{kinds=logical|number,required=false,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=true}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ToColFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "TOCOL"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        schema()
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        match flatten_array(args) {
            Ok(flat) => Ok(collapse_if_scalar(
                flat.into_iter().map(|v| vec![v]).collect(),
                ctx.date_system(),
            )),
            Err(e) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        }
    }
}

/// [formualizer-docgen:schema:start]
/// Name: TOROW
/// Type: ToRowFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: TOROW(arg1: range|any@range, arg2?: number@scalar, arg3?...: logical|number@scalar)
/// Arg schema: arg1{kinds=range|any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=true}; arg3{kinds=logical|number,required=false,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=true}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ToRowFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "TOROW"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        schema()
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        match flatten_array(args) {
            Ok(flat) => Ok(collapse_if_scalar(vec![flat], ctx.date_system())),
            Err(e) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        }
    }
}

/// The array argument plus optional arguments that are either a single
/// value or omitted. Error values reach the function unchanged.
fn reshape_schema(optional: usize) -> Vec<ArgSchema> {
    let mut schema = vec![ArgSchema {
        kinds: smallvec::smallvec![ArgKind::Range, ArgKind::Any],
        required: true,
        by_ref: false,
        shape: ShapeKind::Range,
        coercion: CoercionPolicy::None,
        max: None,
        repeating: None,
        default: None,
    }];
    schema.push(ArgSchema::any());
    for _ in 0..optional {
        let mut arg = ArgSchema::any();
        arg.required = false;
        schema.push(arg);
    }
    schema
}

/// An optional argument's value; `None` when it is absent or left empty.
fn optional_arg<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    index: usize,
) -> Result<Option<LiteralValue>, ExcelError> {
    match args.get(index) {
        Some(arg) if !arg.is_omitted() => Ok(Some(arg.value()?.into_literal())),
        _ => Ok(None),
    }
}

/// A size argument truncated to a whole number. Errors propagate and text
/// that is not a number is `#VALUE!`.
fn size_value(value: LiteralValue) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Error(e) => Err(e),
        other => Ok(crate::coercion::to_number_argument(&other)?.trunc()),
    }
}

fn pad_value<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    index: usize,
) -> Result<LiteralValue, ExcelError> {
    Ok(optional_arg(args, index)?
        .unwrap_or_else(|| LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na))))
}

/// Largest result EXPAND and WRAPROWS/WRAPCOLS may build: a whole worksheet.
const MAX_ROWS: f64 = 1_048_576.0;
const MAX_COLS: f64 = 16_384.0;

/// Expands an array to the given rows and columns, padding new cells.
///
/// # Remarks
/// - Omitted `rows` or `columns` keep the array's own size; a size smaller
///   than the array is `#VALUE!`.
/// - `pad_with` defaults to `#N/A`.
///
/// ```yaml,sandbox
/// title: "Pad a column to three rows"
/// formula: "=EXPAND({5;7},3,1,0)"
/// expected: [[5],[7],[0]]
/// ```
#[derive(Debug)]
pub struct ExpandFn;

impl Function for ExpandFn {
    func_caps!(PURE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "EXPAND"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| reshape_schema(2));
        &SCHEMA
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let result = (|| {
            let data = materialize_arg(&args[0])?;
            let height = data.len();
            let width = data.iter().map(Vec::len).max().unwrap_or(0);
            let rows = match optional_arg(args, 1)? {
                Some(v) => size_value(v)?,
                None => height as f64,
            };
            let cols = match optional_arg(args, 2)? {
                Some(v) => size_value(v)?,
                None => width as f64,
            };
            if rows < height as f64 || cols < width as f64 {
                return Err(ExcelError::new(ExcelErrorKind::Value)
                    .with_message("EXPAND cannot shrink an array"));
            }
            if rows > MAX_ROWS || cols > MAX_COLS {
                return Err(ExcelError::new(ExcelErrorKind::Num));
            }
            let pad = pad_value(args, 3)?;
            let (rows, cols) = (rows as usize, cols as usize);
            Ok((0..rows)
                .map(|r| {
                    (0..cols)
                        .map(|c| {
                            data.get(r)
                                .and_then(|row| row.get(c))
                                .cloned()
                                .unwrap_or_else(|| pad.clone())
                        })
                        .collect()
                })
                .collect())
        })();
        match result {
            Ok(rows) => Ok(collapse_if_scalar(rows, ctx.date_system())),
            Err(e) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        }
    }
}

/// Wraps a one-row or one-column vector into rows (`by_col = false`) or
/// columns (`by_col = true`) of `wrap_count` values, padding the last one.
fn wrap_vector<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    by_col: bool,
) -> Result<Vec<Vec<LiteralValue>>, ExcelError> {
    let data = materialize_arg(&args[0])?;
    let height = data.len();
    let width = data.iter().map(Vec::len).max().unwrap_or(0);
    if height > 1 && width > 1 {
        return Err(ExcelError::new(ExcelErrorKind::Value)
            .with_message("WRAPROWS/WRAPCOLS need a single row or column"));
    }
    let values: Vec<LiteralValue> = data.into_iter().flatten().collect();
    let wrap = size_value(args[1].value()?.into_literal())?;
    if wrap < 1.0 {
        return Err(
            ExcelError::new(ExcelErrorKind::Num).with_message("wrap_count must be at least 1")
        );
    }
    let pad = pad_value(args, 2)?;
    let (max_wrap, max_lines) = if by_col {
        (MAX_ROWS, MAX_COLS)
    } else {
        (MAX_COLS, MAX_ROWS)
    };
    if wrap > max_wrap {
        return Err(ExcelError::new(ExcelErrorKind::Num));
    }
    let wrap = wrap as usize;
    let lines = values.len().div_ceil(wrap).max(1);
    if lines as f64 > max_lines {
        return Err(ExcelError::new(ExcelErrorKind::Num));
    }
    let at = |line: usize, pos: usize| {
        values
            .get(line * wrap + pos)
            .cloned()
            .unwrap_or_else(|| pad.clone())
    };
    Ok(if by_col {
        (0..wrap)
            .map(|r| (0..lines).map(|c| at(c, r)).collect())
            .collect()
    } else {
        (0..lines)
            .map(|r| (0..wrap).map(|c| at(r, c)).collect())
            .collect()
    })
}

/// Wraps a row or column of values into rows of `wrap_count` values.
///
/// # Remarks
/// - The vector must be a single row or column, otherwise `#VALUE!`.
/// - `wrap_count` below 1 is `#NUM!`; `pad_with` defaults to `#N/A`.
///
/// ```yaml,sandbox
/// title: "Wrap four values into rows of two"
/// formula: "=WRAPROWS({1,2,3,4},2)"
/// expected: [[1,2],[3,4]]
/// ```
#[derive(Debug)]
pub struct WrapRowsFn;

/// Wraps a row or column of values into columns of `wrap_count` values.
///
/// # Remarks
/// - The vector must be a single row or column, otherwise `#VALUE!`.
/// - `wrap_count` below 1 is `#NUM!`; `pad_with` defaults to `#N/A`.
///
/// ```yaml,sandbox
/// title: "Wrap four values into columns of two"
/// formula: "=WRAPCOLS({1,2,3,4},2)"
/// expected: [[1,3],[2,4]]
/// ```
#[derive(Debug)]
pub struct WrapColsFn;

macro_rules! wrap_fn {
    ($ty:ident, $name:literal, $by_col:expr) => {
        impl Function for $ty {
            func_caps!(PURE, MAY_SPILL);
            fn name(&self) -> &'static str {
                $name
            }
            fn min_args(&self) -> usize {
                2
            }
            fn arg_schema(&self) -> &'static [ArgSchema] {
                use once_cell::sync::Lazy;
                static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| reshape_schema(1));
                &SCHEMA
            }
            fn eval<'a, 'b, 'c>(
                &self,
                args: &'c [ArgumentHandle<'a, 'b>],
                ctx: &dyn FunctionContext<'b>,
            ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
                match wrap_vector(args, $by_col) {
                    Ok(rows) => Ok(collapse_if_scalar(rows, ctx.date_system())),
                    Err(e) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
                }
            }
        }
    };
}

wrap_fn!(WrapRowsFn, "WRAPROWS", false);
wrap_fn!(WrapColsFn, "WRAPCOLS", true);

pub fn register_builtins() {
    use crate::function_registry::register_builtin;
    use std::sync::Arc;

    register_builtin(Arc::new(ToColFn));
    register_builtin(Arc::new(ToRowFn));
    register_builtin(Arc::new(ExpandFn));
    register_builtin(Arc::new(WrapRowsFn));
    register_builtin(Arc::new(WrapColsFn));
}

#[cfg(test)]
mod tests {
    use crate::builtins::logical::{FalseFn, TrueFn};
    use crate::test_workbook::TestWorkbook;
    use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
    use formualizer_parse::parser::parse;
    use std::sync::Arc;

    fn eval(formula: &str) -> LiteralValue {
        let wb = TestWorkbook::new()
            .with_function(Arc::new(super::ToColFn))
            .with_function(Arc::new(super::ToRowFn))
            .with_function(Arc::new(super::ExpandFn))
            .with_function(Arc::new(super::WrapRowsFn))
            .with_function(Arc::new(super::WrapColsFn))
            .with_function(Arc::new(TrueFn))
            .with_function(Arc::new(FalseFn));
        let interp = wb.interpreter();
        let ast = parse(formula).expect("parse");
        interp.evaluate_ast(&ast).expect("eval").into_literal()
    }

    fn n(v: f64) -> LiteralValue {
        LiteralValue::Number(v)
    }

    fn na() -> LiteralValue {
        LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na))
    }

    fn error_kind(v: LiteralValue) -> ExcelErrorKind {
        match v {
            LiteralValue::Error(e) => e.kind,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn expand_pads_rows_and_columns() {
        assert_eq!(
            eval("=EXPAND({5;7},3,1,0)"),
            LiteralValue::Array(vec![vec![n(5.0)], vec![n(7.0)], vec![n(0.0)]])
        );
        assert_eq!(
            eval("=EXPAND({5},1,3,0)"),
            LiteralValue::Array(vec![vec![n(5.0), n(0.0), n(0.0)]])
        );
        assert_eq!(
            eval("=EXPAND({1,2},2)"),
            LiteralValue::Array(vec![vec![n(1.0), n(2.0)], vec![na(), na()]])
        );
        assert_eq!(
            eval("=EXPAND({1,2},,3,\"x\")"),
            LiteralValue::Array(vec![vec![n(1.0), n(2.0), LiteralValue::Text("x".into())]])
        );
    }

    #[test]
    fn expand_rejects_shrinking() {
        assert_eq!(
            error_kind(eval("=EXPAND({1;2;3},2)")),
            ExcelErrorKind::Value
        );
        assert_eq!(
            error_kind(eval("=EXPAND({1,2},1,1)")),
            ExcelErrorKind::Value
        );
    }

    #[test]
    fn wraprows_and_wrapcols_fill_along_their_axis() {
        assert_eq!(
            eval("=WRAPROWS({1,2,3,4},2)"),
            LiteralValue::Array(vec![vec![n(1.0), n(2.0)], vec![n(3.0), n(4.0)]])
        );
        assert_eq!(
            eval("=WRAPCOLS({1,2,3,4},2)"),
            LiteralValue::Array(vec![vec![n(1.0), n(3.0)], vec![n(2.0), n(4.0)]])
        );
        assert_eq!(
            eval("=WRAPROWS({1;2;3},2)"),
            LiteralValue::Array(vec![vec![n(1.0), n(2.0)], vec![n(3.0), na()]])
        );
        assert_eq!(
            eval("=WRAPCOLS({1,2,3},2,0)"),
            LiteralValue::Array(vec![vec![n(1.0), n(3.0)], vec![n(2.0), n(0.0)]])
        );
    }

    #[test]
    fn wrap_rejects_two_dimensional_input_and_small_counts() {
        assert_eq!(
            error_kind(eval("=WRAPROWS({1,2;3,4},2)")),
            ExcelErrorKind::Value
        );
        assert_eq!(
            error_kind(eval("=WRAPCOLS({1,2,3},0)")),
            ExcelErrorKind::Num
        );
    }

    #[test]
    fn tocol_flattens_rows_by_default() {
        assert_eq!(
            eval("=TOCOL({1,2;3,4})"),
            LiteralValue::Array(vec![
                vec![LiteralValue::Number(1.0)],
                vec![LiteralValue::Number(2.0)],
                vec![LiteralValue::Number(3.0)],
                vec![LiteralValue::Number(4.0)],
            ])
        );
    }

    #[test]
    fn torow_can_scan_by_column() {
        assert_eq!(
            eval("=TOROW({1,2;3,4},0,TRUE)"),
            LiteralValue::Array(vec![vec![
                LiteralValue::Number(1.0),
                LiteralValue::Number(3.0),
                LiteralValue::Number(2.0),
                LiteralValue::Number(4.0),
            ]])
        );
    }

    #[test]
    fn ignores_blanks_and_errors() {
        let value = eval("=TOROW({1,#N/A;\"\",2},2,FALSE)");
        assert_eq!(
            value,
            LiteralValue::Array(vec![vec![
                LiteralValue::Number(1.0),
                LiteralValue::Text(String::new()),
                LiteralValue::Number(2.0),
            ]])
        );

        let value = eval("=TOROW({#N/A},2)");
        assert!(matches!(value, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Calc));
    }

    #[test]
    fn rejects_invalid_ignore_mode() {
        let value = eval("=TOCOL({1,2},4)");
        assert!(matches!(
            value,
            LiteralValue::Error(ExcelError {
                kind: ExcelErrorKind::Value,
                ..
            })
        ));
    }
}
