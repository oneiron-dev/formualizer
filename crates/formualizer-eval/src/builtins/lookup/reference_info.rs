//! Reference information functions: ROW, ROWS, COLUMN, COLUMNS, AREAS
//!
//! Excel semantics:
//! - ROW([reference]) - Returns the row number of a reference
//! - ROWS(array) - Returns the number of rows in a reference
//! - COLUMN([reference]) - Returns the column number of a reference
//! - COLUMNS(array) - Returns the number of columns in a reference
//! - AREAS(reference) - Returns the number of areas in a reference
//!
//! Without arguments, ROW and COLUMN return the current cell's position

use crate::args::{ArgSchema, CoercionPolicy, ShapeKind};
use crate::function::{Function, FunctionResolution};
use crate::function_contract::{FunctionContextDependence, FunctionSemanticContract};
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{ArgKind, ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;
use formualizer_parse::parser::{ExternalRefKind, ReferenceType};

#[derive(Debug)]
pub struct AreasFn;

/// Returns the number of areas in a reference.
///
/// # Remarks
/// - A union written with the `,` reference operator, `(A1:B2,C3,D4:E5)`,
///   has one area per operand in the order written (a repeated area counts
///   each time); an intersection with one has an area per overlap.
/// - Any other reference, a whole column or a function's reference included,
///   is one area.
/// - An argument that is no reference returns its error, or `#VALUE!`
///   (Excel refuses a constant such as `AREAS(1)` at entry).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Three areas"
/// formula: '=AREAS((A1:B2,C3,D4:E5))'
/// expected: 3
/// ```
///
/// ```yaml,docs
/// related:
///   - INDEX
///   - ROWS
///   - COLUMNS
/// faq:
///   - q: "How many areas does an intersection have?"
///     a: "One per overlap: AREAS(A1:B2 B1:C3) is 1."
/// ```
impl Function for AreasFn {
    fn name(&self) -> &'static str {
        "AREAS"
    }

    fn min_args(&self) -> usize {
        1
    }

    func_caps!(PURE);

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![ArgSchema {
                kinds: smallvec::smallvec![ArgKind::Range],
                required: true,
                by_ref: true,
                shape: ShapeKind::Range,
                coercion: CoercionPolicy::None,
                max: None,
                repeating: None,
                default: None,
            }]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let count = if args.len() != 1 {
            Err(ExcelError::new(ExcelErrorKind::Value))
        } else if let Some(areas) = args[0].reference_areas() {
            areas.map(|areas| areas.len())
        } else {
            args[0].as_reference_or_eval().map(|_| 1).map_err(|error| {
                if error.kind == ExcelErrorKind::Ref {
                    ExcelError::new(ExcelErrorKind::Value)
                } else {
                    error
                }
            })
        };
        Ok(crate::traits::CalcValue::Scalar(match count {
            Ok(count) => LiteralValue::Number(count as f64),
            Err(error) => LiteralValue::Error(error),
        }))
    }
}

#[derive(Debug)]
pub struct RowFn;

/// Returns the row number of a reference, or of the current cell when omitted.
///
/// `ROW` returns a 1-based row index.
///
/// # Remarks
/// - With a multi-row range argument, `ROW` returns the vertical array of its row numbers.
/// - Without arguments, it uses the row of the formula cell.
/// - Full-column references such as `A:A` return `1` as a single value; as an
///   array, the row numbers of the rows read for whole columns (through the
///   sheet's last used row), aligned with the column's values.
/// - Invalid references return an error (`#REF!`/`#VALUE!` depending on context).
/// - A computed value instead of a reference (`IF(A1:C1<>"",A1:C1)` in an array
///   formula) gives, element by element, the element's error or `#VALUE!`.
/// - An array of references (`OFFSET(A1,{0;1},0)`, `INDIRECT({"A1";"C3"})`)
///   gives one result per reference; an element that is no reference keeps its
///   error (`#REF!` for an offset off the sheet).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Row of a single-cell reference"
/// formula: '=ROW(B5)'
/// expected: 5
/// ```
///
/// ```yaml,sandbox
/// title: "Row of a single-row range"
/// formula: '=ROW(C3:E3)'
/// expected: 3
/// ```
///
/// ```yaml,docs
/// related:
///   - ROWS
///   - COLUMN
///   - ADDRESS
/// faq:
///   - q: "What does ROW return for a multi-cell reference?"
///     a: "ROW returns the array of every row number in the reference (its first row when used as a single value)."
///   - q: "What if ROW() is called without arguments?"
///     a: "It uses the formula cell position; if no current cell context exists, it returns #VALUE!."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: ROW
/// Type: RowFn
/// Min args: 0
/// Max args: 1
/// Variadic: false
/// Signature: ROW(arg1?: range@range)
/// Arg schema: arg1{kinds=range,required=false,shape=range,by_ref=true,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for RowFn {
    fn name(&self) -> &'static str {
        "ROW"
    }

    fn min_args(&self) -> usize {
        0
    }

    func_caps!(PURE);

    // The argument may be an array of references or a computed array rather
    // than a reference; see `dispatch_position`.
    fn dispatch<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        dispatch_position(self, args, ctx)
    }

    fn semantic_contract(&self, arity: usize) -> Option<FunctionSemanticContract> {
        let mut contract = FunctionSemanticContract::trusted_builtin_default(None);
        if arity == 0 {
            contract.context = FunctionContextDependence::PlacementDependent;
        }
        Some(contract)
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![
                // Optional reference
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Range],
                    required: false,
                    by_ref: true,
                    shape: ShapeKind::Range,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
            ]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.is_empty() {
            // Return current cell's row (1-based) if available
            if let Some(cell_ref) = ctx.current_cell() {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(
                    cell_ref.coord.row() as i64 + 1,
                )));
            }
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Get reference
        let reference = match args[0].as_reference_or_eval() {
            Ok(r) => r,
            Err(e) if e.kind == ExcelErrorKind::Cancelled => return Err(e),
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        let position = match linked_position(&reference, ctx) {
            Ok(position) => position,
            Err(e) if e.kind == ExcelErrorKind::Cancelled => return Err(e),
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        // Row numbers (1-based) spanned by the reference.
        let (first, last) = match position.as_ref().unwrap_or(&reference) {
            ReferenceType::Cell { row, .. } => (*row as i64, *row as i64),
            // A whole column (A:A) starts at row 1 and ends where it is read.
            ReferenceType::Range {
                start_row, end_row, ..
            } => {
                let first = start_row.map_or(1, i64::from);
                (
                    first,
                    open_axis_end(&args[0], &reference, *end_row, first, true, ctx)?,
                )
            }
            // Fallback: resolve the reference and use the view extent
            _ => match ctx.resolve_range_view(&reference, ctx.current_sheet()) {
                Ok(view) => {
                    if view.is_empty() {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                            ExcelError::new(ExcelErrorKind::Ref),
                        )));
                    }
                    let first = view.start_row() as i64 + 1;
                    (first, first + view.dims().0 as i64 - 1)
                }
                Err(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
            },
        };

        // Unless evaluated as an array, a formula entered without the array
        // flag takes the first row only.
        let last = if args[0].in_legacy_value_context() {
            first
        } else {
            last
        };
        Ok(index_sequence(first, last, true, ctx))
    }
}

#[derive(Debug)]
pub struct RowsFn;

/// Returns the number of rows in a reference or array.
///
/// `ROWS` reports height, not data density.
///
/// # Remarks
/// - For a single cell reference, returns `1`.
/// - For full-column references (for example `A:A`), returns `1048576`.
/// - For array literals, returns the outer array length.
/// - Invalid references return an error.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Count rows in a contiguous range"
/// formula: '=ROWS(B2:D10)'
/// expected: 9
/// ```
///
/// ```yaml,sandbox
/// title: "Count rows in a full column reference"
/// formula: '=ROWS(A:A)'
/// expected: 1048576
/// ```
///
/// ```yaml,docs
/// related:
///   - ROW
///   - COLUMNS
///   - INDEX
/// faq:
///   - q: "Does ROWS count populated rows or reference height?"
///     a: "ROWS returns reference height only; blanks inside the range do not reduce the count."
///   - q: "How does ROWS behave for full-column references?"
///     a: "A full-column reference (like A:A) returns the sheet row limit, 1048576."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: ROWS
/// Type: RowsFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: ROWS(arg1: any@range)
/// Arg schema: arg1{kinds=any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for RowsFn {
    fn name(&self) -> &'static str {
        "ROWS"
    }

    fn min_args(&self) -> usize {
        1
    }

    func_caps!(PURE);

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![
                // Required reference/range or array
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Any],
                    required: true,
                    by_ref: false,
                    shape: ShapeKind::Range,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
            ]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        const EXCEL_MAX_ROWS: i64 = 1_048_576;

        if args.is_empty() {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Try to get reference first, fall back to array literal
        if let Ok(reference) = args[0].as_reference_or_eval() {
            let position = match linked_position(&reference, ctx) {
                Ok(position) => position,
                Err(e) if e.kind == ExcelErrorKind::Cancelled => return Err(e),
                Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
            };
            // Calculate number of rows
            let rows = match position.as_ref().unwrap_or(&reference) {
                ReferenceType::Cell { .. } => 1,
                ReferenceType::Range {
                    start_row: Some(sr),
                    end_row: Some(er),
                    ..
                } => {
                    if *er >= *sr {
                        (*er - *sr + 1) as i64
                    } else {
                        1
                    }
                }
                // Full-column references like A:A
                ReferenceType::Range {
                    start_row: None,
                    end_row: None,
                    ..
                } => EXCEL_MAX_ROWS,
                // Open-ended tail like A5:A
                ReferenceType::Range {
                    start_row: Some(sr),
                    end_row: None,
                    ..
                } => EXCEL_MAX_ROWS.saturating_sub(*sr as i64).saturating_add(1),
                // Open-ended head like A:A10 (treated as A1:A10)
                ReferenceType::Range {
                    start_row: None,
                    end_row: Some(er),
                    ..
                } => *er as i64,
                // Fallback for named ranges, table refs, etc.
                _ => match ctx.resolve_range_view(&reference, ctx.current_sheet()) {
                    Ok(view) => view.dims().0 as i64,
                    Err(e) => {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                    }
                },
            };
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(rows)))
        } else {
            // Handle array literal
            let v = args[0].value()?.into_literal();
            let rows = match v {
                LiteralValue::Array(arr) => arr.len() as i64,
                LiteralValue::Error(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
                _ => 1,
            };
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(rows)))
        }
    }
}

#[derive(Debug)]
pub struct ColumnFn;

/// Returns the column number of a reference, or of the current cell when omitted.
///
/// `COLUMN` returns a 1-based column index (`A` = 1).
///
/// # Remarks
/// - With a range argument, `COLUMN` returns the first column in that reference.
/// - Without arguments, it uses the column of the formula cell.
/// - Full-row references such as `5:5` return `1` as a single value; as an
///   array, the column numbers of the columns read for whole rows (through the
///   sheet's last used column), aligned with the row's values.
/// - Invalid references return an error (`#REF!`/`#VALUE!` depending on context).
/// - A computed value instead of a reference (`IF(A1:C1<>"",A1:C1)` in an array
///   formula) gives, element by element, the element's error or `#VALUE!`.
/// - An array of references (`OFFSET(A1,{0;1},0)`, `INDIRECT({"A1";"C3"})`)
///   gives one result per reference; an element that is no reference keeps its
///   error (`#REF!` for an offset off the sheet).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Column of a single-cell reference"
/// formula: '=COLUMN(C5)'
/// expected: 3
/// ```
///
/// ```yaml,sandbox
/// title: "Column of a range"
/// formula: '=COLUMN(B2:D4)'
/// expected: 2
/// ```
///
/// ```yaml,docs
/// related:
///   - COLUMNS
///   - ROW
///   - ADDRESS
/// faq:
///   - q: "What does COLUMN return for a range like B2:D4?"
///     a: "COLUMN returns the first column index of the reference (2 for column B)."
///   - q: "What if COLUMN() is used without a reference?"
///     a: "It returns the formula cell's column number, or #VALUE! if current-cell context is unavailable."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: COLUMN
/// Type: ColumnFn
/// Min args: 0
/// Max args: 1
/// Variadic: false
/// Signature: COLUMN(arg1?: range@range)
/// Arg schema: arg1{kinds=range,required=false,shape=range,by_ref=true,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ColumnFn {
    fn name(&self) -> &'static str {
        "COLUMN"
    }

    fn min_args(&self) -> usize {
        0
    }

    func_caps!(PURE);

    // The argument may be an array of references or a computed array rather
    // than a reference; see `dispatch_position`.
    fn dispatch<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        dispatch_position(self, args, ctx)
    }

    fn semantic_contract(&self, arity: usize) -> Option<FunctionSemanticContract> {
        let mut contract = FunctionSemanticContract::trusted_builtin_default(None);
        if arity == 0 {
            contract.context = FunctionContextDependence::PlacementDependent;
        }
        Some(contract)
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![
                // Optional reference
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Range],
                    required: false,
                    by_ref: true,
                    shape: ShapeKind::Range,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
            ]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if args.is_empty() {
            // Return current cell's column (1-based) if available
            if let Some(cell_ref) = ctx.current_cell() {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(
                    cell_ref.coord.col() as i64 + 1,
                )));
            }
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Get reference
        let reference = match args[0].as_reference_or_eval() {
            Ok(r) => r,
            Err(e) if e.kind == ExcelErrorKind::Cancelled => return Err(e),
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        let position = match linked_position(&reference, ctx) {
            Ok(position) => position,
            Err(e) if e.kind == ExcelErrorKind::Cancelled => return Err(e),
            Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
        };

        // Column numbers (1-based) spanned by the reference.
        let (first, last) = match position.as_ref().unwrap_or(&reference) {
            ReferenceType::Cell { col, .. } => (*col as i64, *col as i64),
            // A whole row (5:5) starts at column 1 and ends where it is read.
            ReferenceType::Range {
                start_col, end_col, ..
            } => {
                let first = start_col.map_or(1, i64::from);
                (
                    first,
                    open_axis_end(&args[0], &reference, *end_col, first, false, ctx)?,
                )
            }
            // Fallback: resolve the reference and use the view extent
            _ => match ctx.resolve_range_view(&reference, ctx.current_sheet()) {
                Ok(view) => {
                    if view.is_empty() {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                            ExcelError::new(ExcelErrorKind::Ref),
                        )));
                    }
                    let first = view.start_col() as i64 + 1;
                    (first, first + view.dims().1 as i64 - 1)
                }
                Err(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
            },
        };

        // Unless evaluated as an array, a formula entered without the array
        // flag takes the first column only.
        let last = if args[0].in_legacy_value_context() {
            first
        } else {
            last
        };
        Ok(index_sequence(first, last, false, ctx))
    }
}

/// ROW/COLUMN dispatch. A reference gives its row or column numbers. Any
/// other argument fails that path, and then ROW/COLUMN apply once to each
/// reference of an array of references (`COLUMN(OFFSET(A1,0,{0,1,2}))` is
/// `{1,2,3}`, an element that is no reference keeping its error, as
/// `ROW(OFFSET(A1,{-1;0},0))` is `{#REF!;1}`) or to each element of a
/// computed value (see `non_reference_result`).
fn dispatch_position<'b>(
    fun: &dyn Function,
    args: &[ArgumentHandle<'_, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
    let result = fun.dispatch_scalar(args, ctx)?;
    if !matches!(
        result,
        crate::traits::CalcValue::Scalar(LiteralValue::Error(_))
    ) {
        return Ok(result);
    }
    if let Some(lifted) =
        crate::lift::lift_call(fun.name(), args, |call| dispatch_position(fun, call, ctx))?
    {
        return Ok(lifted);
    }
    non_reference_result(args, result, ctx)
}

/// ROW/COLUMN of a computed value instead of a reference, as in the array
/// formula `COLUMN(IF(A1:C1<>"",A1:C1))`: Excel applies the function to each
/// element of the value, so an error element keeps its error and any other
/// element is #VALUE!. `result` is the reference path's result, an error
/// whenever the argument is not a reference (a name that holds a value
/// included). Cancellation aborts the formula.
fn non_reference_result<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    result: crate::traits::CalcValue<'b>,
    ctx: &dyn FunctionContext<'b>,
) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
    match &result {
        crate::traits::CalcValue::Scalar(LiteralValue::Error(error))
            if error.kind == ExcelErrorKind::Cancelled =>
        {
            return Err(error.clone());
        }
        crate::traits::CalcValue::Scalar(LiteralValue::Error(_)) => {}
        _ => return Ok(result),
    }
    let [arg] = args else {
        return Ok(result);
    };
    let value = match arg.resolve_reference_or_value() {
        Ok(FunctionResolution::Value(value)) => value,
        Err(error) if error.kind == ExcelErrorKind::Cancelled => return Err(error),
        _ => return Ok(result),
    };
    let element = |value: LiteralValue| match value {
        LiteralValue::Error(error) => LiteralValue::Error(error),
        _ => LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value)),
    };
    Ok(match value.into_literal() {
        // Excel has no empty array; an empty result is #CALC!.
        LiteralValue::Array(rows) if rows.first().is_none_or(Vec::is_empty) => {
            crate::traits::CalcValue::Scalar(LiteralValue::Error(ExcelError::new(
                ExcelErrorKind::Calc,
            )))
        }
        LiteralValue::Array(rows) => crate::lift::array_result(
            rows.into_iter()
                .map(|row| row.into_iter().map(element).collect())
                .collect(),
            ctx.date_system(),
        ),
        other => crate::traits::CalcValue::Scalar(element(other)),
    })
}

/// The last row (`rows`) or column index of `reference`, a range whose axis
/// ends at `end` or, for a whole column (row) such as `A:A` (`5:5`), runs to
/// the sheet's end. Evaluated as an array, such an axis gives the indexes of
/// the cells the engine reads for it: whole columns of a sheet all end at the
/// sheet's last used row (whole rows at its last used column), so `ROW(A:A)`
/// lines up element by element with `A:A`'s values, as in
/// `SMALL(IF(A:A="x",ROW(A:A)),2)`. As a single value only `first` is needed,
/// as it is when the context cannot read the whole axis. Only cancellation
/// is an error.
fn open_axis_end(
    arg: &ArgumentHandle<'_, '_>,
    reference: &ReferenceType,
    end: Option<u32>,
    first: i64,
    rows: bool,
    ctx: &dyn FunctionContext<'_>,
) -> Result<i64, ExcelError> {
    if let Some(end) = end {
        return Ok(i64::from(end));
    }
    if arg.in_legacy_value_context() {
        return Ok(first);
    }
    let view = match ctx.resolve_range_view(reference, ctx.current_sheet()) {
        Ok(view) if !view.is_empty() => view,
        Err(error) if error.kind == ExcelErrorKind::Cancelled => return Err(error),
        _ => return Ok(first),
    };
    let (start, len) = if rows {
        (view.start_row(), view.dims().0)
    } else {
        (view.start_col(), view.dims().1)
    };
    // A linked workbook's saved values come as a view of their own, which
    // starts at the reference's first row (column).
    let start = match reference {
        ReferenceType::External(_) => first - 1,
        _ => start as i64,
    };
    Ok((start + len as i64).max(first))
}

/// A reference into a closed linked workbook (`[1]Data!K2:K9`) evaluates to
/// the values saved with the link, but keeps the position it is written with:
/// `ROW([1]Data!K2:K9)` is `{2;...;9}`, as for a local range. Gives the local
/// reference of the same shape once the linked sheet is known to be readable
/// (a sheet the link does not name is `#REF!`), and `None` for any other
/// reference.
fn linked_position(
    reference: &ReferenceType,
    ctx: &dyn FunctionContext<'_>,
) -> Result<Option<ReferenceType>, ExcelError> {
    let ReferenceType::External(ext) = reference else {
        return Ok(None);
    };
    ctx.resolve_range_view(reference, ctx.current_sheet())?;
    Ok(Some(match ext.kind {
        ExternalRefKind::Cell { row, col, .. } => ReferenceType::cell(None, row, col),
        ExternalRefKind::Range {
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        } => {
            let (start_row, end_row) = crate::engine::external_book::in_order(start_row, end_row);
            let (start_col, end_col) = crate::engine::external_book::in_order(start_col, end_col);
            ReferenceType::range(None, start_row, start_col, end_row, end_col)
        }
    }))
}

/// ROW/COLUMN result: a single index, or for a multi-row (multi-column)
/// reference the vertical (horizontal) array of every index, as Excel returns.
fn index_sequence<'b>(
    first: i64,
    last: i64,
    vertical: bool,
    ctx: &dyn FunctionContext<'b>,
) -> crate::traits::CalcValue<'b> {
    if last <= first {
        return crate::traits::CalcValue::Scalar(LiteralValue::Int(first));
    }
    let values = (first..=last).map(LiteralValue::Int);
    let rows = if vertical {
        values.map(|v| vec![v]).collect()
    } else {
        vec![values.collect()]
    };
    crate::traits::CalcValue::Range(crate::engine::range_view::RangeView::from_owned_rows(
        rows,
        ctx.date_system(),
    ))
}

#[derive(Debug)]
pub struct ColumnsFn;

/// Returns the number of columns in a reference or array.
///
/// `COLUMNS` reports width, not data density.
///
/// # Remarks
/// - For a single cell reference, returns `1`.
/// - For full-row references (for example `1:1`), returns `16384`.
/// - For array literals, returns the first row width.
/// - Invalid references return an error.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Count columns in a rectangular range"
/// formula: '=COLUMNS(B2:D10)'
/// expected: 3
/// ```
///
/// ```yaml,sandbox
/// title: "Count columns in a full row reference"
/// formula: '=COLUMNS(1:1)'
/// expected: 16384
/// ```
///
/// ```yaml,docs
/// related:
///   - COLUMN
///   - ROWS
///   - CHOOSECOLS
/// faq:
///   - q: "Does COLUMNS count non-empty cells?"
///     a: "No. COLUMNS returns the width of the referenced array/range, including blank cells."
///   - q: "What does COLUMNS return for a full-row reference like 1:1?"
///     a: "It returns the sheet column limit, 16384."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: COLUMNS
/// Type: ColumnsFn
/// Min args: 1
/// Max args: 1
/// Variadic: false
/// Signature: COLUMNS(arg1: any@range)
/// Arg schema: arg1{kinds=any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ColumnsFn {
    fn name(&self) -> &'static str {
        "COLUMNS"
    }

    fn min_args(&self) -> usize {
        1
    }

    func_caps!(PURE);

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![
                // Required reference/range or array
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Any],
                    required: true,
                    by_ref: false,
                    shape: ShapeKind::Range,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
            ]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        const EXCEL_MAX_COLS: i64 = 16_384;

        if args.is_empty() {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Try to get reference first, fall back to array literal
        if let Ok(reference) = args[0].as_reference_or_eval() {
            let position = match linked_position(&reference, ctx) {
                Ok(position) => position,
                Err(e) if e.kind == ExcelErrorKind::Cancelled => return Err(e),
                Err(e) => return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
            };
            // Calculate number of columns
            let cols = match position.as_ref().unwrap_or(&reference) {
                ReferenceType::Cell { .. } => 1,
                ReferenceType::Range {
                    start_col: Some(sc),
                    end_col: Some(ec),
                    ..
                } => {
                    if *ec >= *sc {
                        (*ec - *sc + 1) as i64
                    } else {
                        1
                    }
                }
                // Full-row references like 1:1
                ReferenceType::Range {
                    start_col: None,
                    end_col: None,
                    ..
                } => EXCEL_MAX_COLS,
                // Open-ended tail where start_col is known and end_col is omitted
                ReferenceType::Range {
                    start_col: Some(sc),
                    end_col: None,
                    ..
                } => EXCEL_MAX_COLS.saturating_sub(*sc as i64).saturating_add(1),
                // Open-ended head like :F (or equivalent parsed form)
                ReferenceType::Range {
                    start_col: None,
                    end_col: Some(ec),
                    ..
                } => *ec as i64,
                // Fallback for named ranges, table refs, etc.
                _ => match ctx.resolve_range_view(&reference, ctx.current_sheet()) {
                    Ok(view) => view.dims().1 as i64,
                    Err(e) => {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                    }
                },
            };
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(cols)))
        } else {
            // Handle array literal
            let v = args[0].value()?.into_literal();
            let cols = match v {
                LiteralValue::Array(arr) => arr.first().map(|r| r.len()).unwrap_or(0) as i64,
                LiteralValue::Error(e) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
                }
                _ => 1,
            };
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Int(cols)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::{CellRef, Coord};
    use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};
    use std::sync::Arc;

    #[test]
    fn row_with_reference() {
        let wb = TestWorkbook::new().with_function(Arc::new(RowFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ROW").unwrap();

        // ROW(B5) -> 5
        let b5_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "B5".into(),
                reference: ReferenceType::cell(None, 5, 2),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&b5_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(5));

        // ROW(A1:C3) -> {1;2;3}
        let range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:C3".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(3), Some(3)),
            },
            None,
        );

        let args2 = vec![ArgumentHandle::new(&range_ref, &ctx)];
        let result2 = f
            .dispatch(&args2, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(
            result2,
            LiteralValue::Array(vec![
                vec![LiteralValue::Number(1.0)],
                vec![LiteralValue::Number(2.0)],
                vec![LiteralValue::Number(3.0)],
            ])
        );
    }

    #[test]
    fn row_no_arg_uses_current_cell_1_based() {
        let wb = TestWorkbook::new().with_function(Arc::new(RowFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ROW").unwrap();

        let current = CellRef::new(0, Coord::from_excel(7, 4, false, false));
        let result = f
            .dispatch(&[], &ctx.function_context(Some(&current)))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(7));
    }

    #[test]
    fn row_full_column_reference_returns_first_row_as_a_single_value() {
        let wb = TestWorkbook::new().with_function(Arc::new(RowFn));
        let ctx = wb.interpreter().as_legacy_formula();
        let f = ctx.context.get_function("", "ROW").unwrap();

        // ROW(A:A) -> 1 in a formula entered without the array flag
        let col_range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A:A".into(),
                reference: ReferenceType::range(None, None, Some(1), None, Some(1)),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&col_range_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(1));
    }

    #[test]
    fn row_named_range_falls_back_to_resolved_range_view() {
        let wb = TestWorkbook::new()
            .with_named_range("MyRow", vec![vec![LiteralValue::Int(42)]])
            .with_function(Arc::new(RowFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ROW").unwrap();

        let named_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "MyRow".into(),
                reference: ReferenceType::NamedRange("MyRow".into()),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&named_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(1));
    }

    #[test]
    fn rows_function() {
        let wb = TestWorkbook::new().with_function(Arc::new(RowsFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ROWS").unwrap();

        // ROWS(A1:A5) -> 5
        let range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:A5".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(5), Some(1)),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&range_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(5));

        // ROWS(B2:D10) -> 9
        let range_ref2 = ASTNode::new(
            ASTNodeType::Reference {
                original: "B2:D10".into(),
                reference: ReferenceType::range(None, Some(2), Some(2), Some(10), Some(4)),
            },
            None,
        );

        let args2 = vec![ArgumentHandle::new(&range_ref2, &ctx)];
        let result2 = f
            .dispatch(&args2, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result2, LiteralValue::Int(9));

        // ROWS(A1) -> 1 (single cell)
        let cell_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1".into(),
                reference: ReferenceType::cell(None, 1, 1),
            },
            None,
        );

        let args3 = vec![ArgumentHandle::new(&cell_ref, &ctx)];
        let result3 = f
            .dispatch(&args3, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result3, LiteralValue::Int(1));
    }

    #[test]
    fn rows_full_column_reference_returns_sheet_height() {
        let wb = TestWorkbook::new().with_function(Arc::new(RowsFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ROWS").unwrap();

        // ROWS(A:A) -> 1048576
        let col_range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A:A".into(),
                reference: ReferenceType::range(None, None, Some(1), None, Some(1)),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&col_range_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(1_048_576));
    }

    #[test]
    fn rows_named_range_falls_back_to_resolved_range_view() {
        let wb = TestWorkbook::new()
            .with_named_range(
                "MyRows",
                vec![
                    vec![LiteralValue::Int(1)],
                    vec![LiteralValue::Int(2)],
                    vec![LiteralValue::Int(3)],
                ],
            )
            .with_function(Arc::new(RowsFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ROWS").unwrap();

        let named_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "MyRows".into(),
                reference: ReferenceType::NamedRange("MyRows".into()),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&named_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(3));
    }

    #[test]
    fn column_with_reference() {
        let wb = TestWorkbook::new().with_function(Arc::new(ColumnFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "COLUMN").unwrap();

        // COLUMN(C5) -> 3
        let c5_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "C5".into(),
                reference: ReferenceType::cell(None, 5, 3),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&c5_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(3));

        // COLUMN(B2:D4) -> {2,3,4}
        let range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "B2:D4".into(),
                reference: ReferenceType::range(None, Some(2), Some(2), Some(4), Some(4)),
            },
            None,
        );

        let args2 = vec![ArgumentHandle::new(&range_ref, &ctx)];
        let result2 = f
            .dispatch(&args2, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(
            result2,
            LiteralValue::Array(vec![vec![
                LiteralValue::Number(2.0),
                LiteralValue::Number(3.0),
                LiteralValue::Number(4.0),
            ]])
        );
    }

    #[test]
    fn column_no_arg_uses_current_cell_1_based() {
        let wb = TestWorkbook::new().with_function(Arc::new(ColumnFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "COLUMN").unwrap();

        let current = CellRef::new(0, Coord::from_excel(7, 4, false, false));
        let result = f
            .dispatch(&[], &ctx.function_context(Some(&current)))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(4));
    }

    #[test]
    fn column_full_row_reference_returns_first_column_as_a_single_value() {
        let wb = TestWorkbook::new().with_function(Arc::new(ColumnFn));
        let ctx = wb.interpreter().as_legacy_formula();
        let f = ctx.context.get_function("", "COLUMN").unwrap();

        // COLUMN(5:5) -> 1 in a formula entered without the array flag
        let row_range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "5:5".into(),
                reference: ReferenceType::range(None, Some(5), None, Some(5), None),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&row_range_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(1));
    }

    #[test]
    fn column_named_range_falls_back_to_resolved_range_view() {
        let wb = TestWorkbook::new()
            .with_named_range("MyRange", vec![vec![LiteralValue::Int(42)]])
            .with_function(Arc::new(ColumnFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "COLUMN").unwrap();

        let named_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "MyRange".into(),
                reference: ReferenceType::NamedRange("MyRange".into()),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&named_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(1));
    }

    #[test]
    fn columns_function() {
        let wb = TestWorkbook::new().with_function(Arc::new(ColumnsFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "COLUMNS").unwrap();

        // COLUMNS(A1:E1) -> 5
        let range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:E1".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(1), Some(5)),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&range_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(5));

        // COLUMNS(B2:D10) -> 3
        let range_ref2 = ASTNode::new(
            ASTNodeType::Reference {
                original: "B2:D10".into(),
                reference: ReferenceType::range(None, Some(2), Some(2), Some(10), Some(4)),
            },
            None,
        );

        let args2 = vec![ArgumentHandle::new(&range_ref2, &ctx)];
        let result2 = f
            .dispatch(&args2, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result2, LiteralValue::Int(3));

        // COLUMNS(A1) -> 1 (single cell)
        let cell_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1".into(),
                reference: ReferenceType::cell(None, 1, 1),
            },
            None,
        );

        let args3 = vec![ArgumentHandle::new(&cell_ref, &ctx)];
        let result3 = f
            .dispatch(&args3, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result3, LiteralValue::Int(1));
    }

    #[test]
    fn columns_full_row_reference_returns_sheet_width() {
        let wb = TestWorkbook::new().with_function(Arc::new(ColumnsFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "COLUMNS").unwrap();

        // COLUMNS(1:1) -> 16384
        let row_range_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "1:1".into(),
                reference: ReferenceType::range(None, Some(1), None, Some(1), None),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&row_range_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(16_384));
    }

    #[test]
    fn columns_named_range_falls_back_to_resolved_range_view() {
        let wb = TestWorkbook::new()
            .with_named_range(
                "MyCols",
                vec![
                    vec![
                        LiteralValue::Int(1),
                        LiteralValue::Int(2),
                        LiteralValue::Int(3),
                    ],
                    vec![
                        LiteralValue::Int(4),
                        LiteralValue::Int(5),
                        LiteralValue::Int(6),
                    ],
                ],
            )
            .with_function(Arc::new(ColumnsFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "COLUMNS").unwrap();

        let named_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "MyCols".into(),
                reference: ReferenceType::NamedRange("MyCols".into()),
            },
            None,
        );

        let args = vec![ArgumentHandle::new(&named_ref, &ctx)];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Int(3));
    }

    #[test]
    fn rows_columns_reversed_range() {
        // A5:A1 (start_row > end_row) should treat as 1 row / 1 column per current implementation fallback
        let wb = TestWorkbook::new()
            .with_function(Arc::new(RowsFn))
            .with_function(Arc::new(ColumnsFn));
        let ctx = wb.interpreter();
        let rows_f = ctx.context.get_function("", "ROWS").unwrap();
        let cols_f = ctx.context.get_function("", "COLUMNS").unwrap();
        let rev_range = ASTNode::new(
            ASTNodeType::Reference {
                original: "A5:A1".into(),
                reference: ReferenceType::range(None, Some(5), Some(1), Some(1), Some(1)),
            },
            None,
        );
        let args = vec![ArgumentHandle::new(&rev_range, &ctx)];
        let r_count = rows_f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        let c_count = cols_f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(r_count, LiteralValue::Int(1));
        assert_eq!(c_count, LiteralValue::Int(1));
    }
}
