use crate::args::{ArgSchema, CoercionPolicy, ShapeKind};
use crate::function::{FnCaps, Function, FunctionResolution};
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{ArgKind, ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::ReferenceType;

fn arg_byref_array() -> Vec<ArgSchema> {
    vec![
        // Accept both references and array literals
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
        // row_num and column_num are number parameters: a blank is 0, TRUE and
        // FALSE are 1 and 0, and numeric text converts.
        ArgSchema::number_lenient_scalar(),
        // Column is optional for 1D arrays
        ArgSchema {
            required: false,
            ..ArgSchema::number_lenient_scalar()
        },
        // area_num (reference form) defaults to area 1. It is a number
        // argument, so numeric text converts like OFFSET's numbers.
        ArgSchema {
            required: false,
            ..ArgSchema::number_lenient_scalar()
        },
    ]
}

/// OFFSET(reference, rows, cols, [height], [width]): the offsets and sizes
/// are numeric parameters, so numeric text converts like any number argument.
fn arg_byref_reference() -> Vec<ArgSchema> {
    let optional_number = || ArgSchema {
        required: false,
        ..ArgSchema::number_lenient_scalar()
    };
    vec![
        ArgSchema {
            kinds: smallvec::smallvec![ArgKind::Range],
            required: true,
            by_ref: true,
            shape: ShapeKind::Range,
            coercion: CoercionPolicy::None,
            max: None,
            repeating: None,
            default: None,
        },
        ArgSchema::number_lenient_scalar(),
        ArgSchema::number_lenient_scalar(),
        optional_number(),
        optional_number(),
    ]
}

/// Resolve a reference's concrete 1-based inclusive bounds as
/// `(sheet, start_row, start_col, end_row, end_col)`.
///
/// Fully bounded ranges use their declared bounds directly. Unbounded
/// whole-column/whole-row (or open-ended) ranges are clamped to the used
/// region via `ctx.resolve_range_view`, mirroring how MATCH/VLOOKUP resolve
/// the same references. An empty resolved view yields `#REF!`.
const EXCEL_MAX_ROW: u32 = 1_048_576;
const EXCEL_MAX_COL: u32 = 16_384;

/// The 1-based inclusive `(start_row, start_col, end_row, end_col)` INDEX gives
/// a range without consulting the sheet: the declared bounds of a bounded
/// range, and the full grid along the open axis of a whole column or row (A:A,
/// 1:1), not just the cells in use, so INDEX(A:A,65536) is the blank cell
/// A65536 and A:C is never a single row. `None` for any other open range, which
/// is clamped to the used region. The graph's static INDEX self-loop classifier
/// uses this too, so both see the same shape.
pub(crate) fn index_static_bounds(
    start_row: Option<u32>,
    start_col: Option<u32>,
    end_row: Option<u32>,
    end_col: Option<u32>,
) -> Option<(u32, u32, u32, u32)> {
    match (start_row, start_col, end_row, end_col) {
        (Some(sr), Some(sc), Some(er), Some(ec)) => Some((sr, sc, er, ec)),
        (None, Some(sc), None, Some(ec)) => Some((1, sc, EXCEL_MAX_ROW, ec)),
        (Some(sr), None, Some(er), None) => Some((sr, 1, er, EXCEL_MAX_COL)),
        _ => None,
    }
}

fn resolve_reference_bounds<'b>(
    ctx: &dyn FunctionContext<'b>,
    base: &ReferenceType,
) -> Result<(Option<String>, u32, u32, u32, u32), ExcelError> {
    match base {
        ReferenceType::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        } => {
            if let Some((sr, sc, er, ec)) =
                index_static_bounds(*start_row, *start_col, *end_row, *end_col)
            {
                return Ok((sheet.clone(), sr, sc, er, ec));
            }
            let rv = ctx.resolve_range_view(base, ctx.current_sheet())?;
            if rv.is_empty() {
                return Err(ExcelError::new(ExcelErrorKind::Ref));
            }
            // RangeView exposes absolute 0-based coordinates; ReferenceType is 1-based.
            Ok((
                sheet.clone(),
                rv.start_row() as u32 + 1,
                rv.start_col() as u32 + 1,
                rv.end_row() as u32 + 1,
                rv.end_col() as u32 + 1,
            ))
        }
        ReferenceType::Cell {
            sheet, row, col, ..
        } => Ok((sheet.clone(), *row, *col, *row, *col)),
        _ => Err(ExcelError::new(ExcelErrorKind::Ref)),
    }
}

/// The `n`th (1-based, `n >= 1`) row or column of the span `start..=end`, or
/// `None` past its end, however large `n` is.
fn nth_within(start: u32, end: u32, n: i64) -> Option<u32> {
    let offset = u32::try_from(n.checked_sub(1)?).ok()?;
    start.checked_add(offset).filter(|&at| at <= end)
}

#[derive(Debug)]
pub struct IndexFn;

impl IndexFn {
    /// A row_num, column_num or area_num: `None` when it holds several values, which
    /// dispatch lifts over. In a formula entered without the array flag a
    /// multi-cell reference here has already been intersected with the
    /// formula cell (`lift::legacy_arg`).
    fn index_argument<'a, 'b>(arg: &ArgumentHandle<'a, 'b>) -> Result<Option<i64>, ExcelError> {
        if arg.is_omitted() {
            return Ok(Some(0));
        }
        match arg.value()? {
            crate::traits::CalcValue::Range(_)
            | crate::traits::CalcValue::Scalar(LiteralValue::Array(_)) => Ok(None),
            // A blank index is 0 (the whole row or column), TRUE/FALSE and
            // numeric text convert, and an error index is the result, as in
            // value context (MATCH's #N/A).
            value => integer_parameter(value.into_literal(), arg).map(Some),
        }
    }

    /// area_num, the area of the reference INDEX selects in: 1 when omitted.
    /// It is a whole-number parameter read like row_num and column_num
    /// (`index_argument`: truncated; a blank is 0, a logical and numeric text
    /// convert, other text is `#VALUE!`, an error is itself), the same way in
    /// reference and value context. An area below 1 is no area number at all,
    /// so it is `#VALUE!`; an area past the reference's last area is the
    /// caller's `#REF!`. `None` for an array area_num, which dispatch lifts
    /// over.
    fn area_num<'a, 'b>(args: &[ArgumentHandle<'a, 'b>]) -> Option<Result<i64, ExcelError>> {
        let Some(area) = args.get(3).filter(|area| !area.is_omitted()) else {
            return Some(Ok(1));
        };
        Some(match Self::index_argument(area) {
            Ok(Some(area)) if area < 1 => Err(ExcelError::new(ExcelErrorKind::Value)),
            Ok(Some(area)) => Ok(area),
            Ok(None) => return None,
            Err(error) => Err(error),
        })
    }

    /// Checks area_num against a source that is one area (a range or an
    /// array): area 1 selects it, a higher area lies outside it (`#REF!`).
    fn check_single_area<'a, 'b>(
        args: &[ArgumentHandle<'a, 'b>],
    ) -> Option<Result<(), ExcelError>> {
        Some(Self::area_num(args)?.and_then(|area| {
            if area == 1 {
                Ok(())
            } else {
                Err(ExcelError::new(ExcelErrorKind::Ref))
            }
        }))
    }

    fn bounded_dimensions(base: &ReferenceType) -> Option<(u32, u32)> {
        match base {
            ReferenceType::Cell { .. } => Some((1, 1)),
            ReferenceType::Range {
                start_row: Some(start_row),
                start_col: Some(start_col),
                end_row: Some(end_row),
                end_col: Some(end_col),
                ..
            } => Some((
                end_row.checked_sub(*start_row)?.checked_add(1)?,
                end_col.checked_sub(*start_col)?.checked_add(1)?,
            )),
            _ => None,
        }
    }

    pub(crate) fn precise_single_cell_selection<'a, 'b>(
        args: &[ArgumentHandle<'a, 'b>],
        rows: u32,
        cols: u32,
    ) -> bool {
        if !(2..=3).contains(&args.len()) {
            return false;
        }
        let Ok(Some(position)) = Self::index_argument(&args[1]) else {
            return false;
        };
        if args.len() == 3 {
            let Ok(Some(column)) = Self::index_argument(&args[2]) else {
                return false;
            };
            if args[1].is_omitted() {
                column > 0 && rows == 1 && u32::try_from(column).is_ok_and(|col| col <= cols)
            } else if args[2].is_omitted() {
                position > 0 && cols == 1 && u32::try_from(position).is_ok_and(|row| row <= rows)
            } else {
                position > 0
                    && column > 0
                    && u32::try_from(position).is_ok_and(|row| row <= rows)
                    && u32::try_from(column).is_ok_and(|col| col <= cols)
            }
        } else if rows == 1 {
            position > 0 && u32::try_from(position).is_ok_and(|col| col <= cols)
        } else if cols == 1 {
            position > 0 && u32::try_from(position).is_ok_and(|row| row <= rows)
        } else {
            false
        }
    }

    fn reference_from_base<'a, 'b>(
        args: &[ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
        base: ReferenceType,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        Self::reference_from_areas(args, ctx, std::slice::from_ref(&base))
    }

    /// The reference INDEX selects: row_num and column_num pick within the
    /// area_num-th of `areas` (area 1 of a single range), and an area_num past
    /// the last area is `#REF!`.
    fn reference_from_areas<'a, 'b>(
        args: &[ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
        areas: &[ReferenceType],
    ) -> Option<Result<ReferenceType, ExcelError>> {
        let position = match Self::index_argument(&args[1]) {
            Ok(Some(position)) => position,
            Ok(None) => return None,
            Err(error) => return Some(Err(error)),
        };
        let explicit_col = if args.len() >= 3 {
            match Self::index_argument(&args[2]) {
                Ok(Some(column)) => Some(column),
                Ok(None) => return None,
                Err(error) => return Some(Err(error)),
            }
        } else {
            None
        };
        let base = match Self::area_num(args)? {
            Ok(area) => match usize::try_from(area - 1)
                .ok()
                .and_then(|area| areas.get(area))
            {
                Some(base) => base,
                None => return Some(Err(ExcelError::new(ExcelErrorKind::Ref))),
            },
            Err(error) => return Some(Err(error)),
        };

        // A structured reference is the area it selects on the table's own
        // sheet, read from the table's placement without reading a cell:
        // INDEX(Table1[Qty],2) is the second data cell of that column.
        let base = match crate::traits::reference_as_area(ctx, base.clone()) {
            Ok(base) => base,
            Err(error) => return Some(Err(error)),
        };
        let (sheet, sr, sc, er, ec) = match resolve_reference_bounds(ctx, &base) {
            Ok(bounds) => bounds,
            Err(error) => return Some(Err(error)),
        };
        // A lone index selects a column of a single-row reference; otherwise it
        // selects a row and the omitted column_num acts as 0 (the entire row).
        let (row, col) = match explicit_col {
            Some(col) => (position, col),
            None if sr == er => (1, position),
            None => (position, 0),
        };
        if row < 0 || col < 0 {
            return Some(Err(ExcelError::new(ExcelErrorKind::Ref)));
        }
        // Whole columns and rows stay open-ended (A:A, not A1:A1048576).
        let full_rows = sr == 1 && er == EXCEL_MAX_ROW;
        let full_cols = sc == 1 && ec == EXCEL_MAX_COL;
        let range_ref = |sheet, sr: u32, sc: u32, er: u32, ec: u32| {
            let rows_open = full_rows && sr == 1 && er == EXCEL_MAX_ROW;
            let cols_open = full_cols && sc == 1 && ec == EXCEL_MAX_COL;
            ReferenceType::Range {
                sheet,
                start_row: (!rows_open).then_some(sr),
                start_col: (!cols_open).then_some(sc),
                end_row: (!rows_open).then_some(er),
                end_col: (!cols_open).then_some(ec),
                start_row_abs: false,
                start_col_abs: false,
                end_row_abs: false,
                end_col_abs: false,
            }
        };
        let off_reference = || Some(Err(ExcelError::new(ExcelErrorKind::Ref)));
        if col == 0 {
            if row == 0 {
                return Some(Ok(base));
            }
            let Some(r) = nth_within(sr, er, row) else {
                return off_reference();
            };
            return Some(Ok(if sc == ec {
                ReferenceType::cell(sheet, r, sc)
            } else {
                range_ref(sheet, r, sc, r, ec)
            }));
        }
        if row == 0 {
            let Some(c) = nth_within(sc, ec, col) else {
                return off_reference();
            };
            return Some(Ok(if sr == er {
                ReferenceType::cell(sheet, sr, c)
            } else {
                range_ref(sheet, sr, c, er, c)
            }));
        }
        match (nth_within(sr, er, row), nth_within(sc, ec, col)) {
            (Some(r), Some(c)) => Some(Ok(ReferenceType::cell(sheet, r, c))),
            _ => off_reference(),
        }
    }

    fn materialize_reference<'b>(
        ctx: &dyn FunctionContext<'b>,
        reference: &ReferenceType,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let view = ctx.resolve_range_view(reference, ctx.current_sheet())?;
        let (rows, cols) = view.dims();
        if rows == 1 && cols == 1 {
            Ok(crate::traits::CalcValue::Scalar(
                view.as_1x1().unwrap_or(LiteralValue::Empty),
            ))
        } else {
            Ok(crate::traits::CalcValue::Range(
                view.with_reference_extent(reference),
            ))
        }
    }

    /// Attempt the precise single-cell fast path.
    ///
    /// Returns `None` when any gate declines, in which case the caller falls
    /// back to `validated_dispatch`. Factored out of `dispatch` (and
    /// parameterized over the dispatching function) so tests can assert which
    /// path a given argument shape takes and observe the format policy being
    /// applied to the precise result.
    pub(crate) fn precise_dispatch<'a, 'b, 'c>(
        function: &dyn Function,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Option<crate::traits::CalcValue<'b>> {
        let argument = args.first()?;
        if !argument.may_return_reference() {
            return None;
        }
        let Ok(FunctionResolution::Reference(base)) = argument.resolve_reference_or_value() else {
            return None;
        };
        let base = crate::traits::reference_as_area(ctx, base).ok()?;
        let (rows, cols) = Self::bounded_dimensions(&base)?;
        if !Self::precise_single_cell_selection(args, rows, cols) {
            return None;
        }
        let Some(Ok(reference @ ReferenceType::Cell { .. })) =
            Self::reference_from_base(args, ctx, base)
        else {
            return None;
        };
        let value = Self::materialize_reference(ctx, &reference).ok()?;
        Some(function.apply_format_propagation(value))
    }

    fn validated_dispatch<'a, 'b>(
        &self,
        args: &[ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        use crate::args::{ValidationOptions, validate_and_prepare_reading};
        // INDEX reads only the cells it selects. Validating its reference as
        // a range would read every cell of it, so INDEX(B:B,MATCH(...)) in
        // column B would read its own cell, a circular reference Excel does
        // not see. That validation never fails (any value is accepted), so
        // only the argument count and the indexes are validated.
        if let Err(error) = validate_and_prepare_reading(
            args,
            self.arg_schema(),
            ValidationOptions {
                warn_only: false,
                min_args: self.min_args(),
            },
            |index| index != 0,
        ) {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)));
        }
        self.eval(args, ctx)
            .map(|result| self.apply_format_propagation(result))
    }
}

/// Returns the value or reference at a 1-based row and column within an array or range.
///
/// `INDEX` can operate on both references and array literals. When the first argument is
/// a reference, this implementation resolves a referenced cell and materializes its value in
/// value context.
///
/// # Remarks
/// - Indexing is 1-based for both `row_num` and `column_num`.
/// - If `column_num` is omitted for a single-row or single-column input, `row_num` selects the
///   position along that 1D vector.
/// - For inputs with more than one row and column, `row_num` with `column_num` omitted
///   selects the entire row, like `column_num` = `0` (Excel behavior).
/// - A `row_num` or `column_num` of `0` selects the entire column or row respectively
///   (both `0` selects the whole range), matching Excel.
/// - Negative or out-of-bounds indexes return `#REF!`, however large.
/// - A structured reference is the cells it selects: `INDEX(Table1[Qty],2)` is the second
///   data cell of that column.
/// - `row_num`, `column_num` and `area_num` are numbers: a blank cell is 0, TRUE and FALSE
///   are 1 and 0, and numeric text converts. Other text returns `#VALUE!`, and an error
///   index returns that error.
/// - An array `row_num`, `column_num` or `area_num` returns an array of the selected values,
///   paired and broadcast element by element like any single-value parameter.
/// - In a workbook formula entered without the array flag, a range `row_num`, `column_num` or
///   `area_num` is implicitly intersected with the formula cell (`#VALUE!` when they do not
///   cross).
/// - `area_num` (reference form) picks the area of a multi-area reference in which `row_num`
///   and `column_num` select: a union such as `(A1:B2,D1:E2)`, or a name defined as one,
///   numbers its areas in the order written, and the intersection of such a reference with
///   another, as in `(A1:B2,D1:E2) A2:E2`, has each area's overlap in that order. It
///   defaults to 1, and a single range or array is area 1 only. An area below 1 returns
///   `#VALUE!`, an area past the last one returns `#REF!`, and a union whose areas lie on
///   different sheets returns `#VALUE!`. Only the selected area is read.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Pick a value from a 2D table"
/// grid:
///   A1: "Item"
///   B1: "Price"
///   A2: "Pen"
///   B2: 2.5
///   A3: "Book"
///   B3: 8
/// formula: '=INDEX(A1:B3,3,2)'
/// expected: 8
/// ```
///
/// ```yaml,sandbox
/// title: "Index into a 1D vector"
/// grid:
///   A1: "Q1"
///   A2: "Q2"
///   A3: "Q3"
/// formula: '=INDEX(A1:A3,2)'
/// expected: "Q2"
/// ```
///
/// ```yaml,docs
/// related:
///   - MATCH
///   - XLOOKUP
///   - OFFSET
/// faq:
///   - q: "How does INDEX behave when column_num is omitted?"
///     a: "For single-row or single-column inputs, row_num selects the position along that vector; for 2D inputs, an omitted column_num returns the entire row, like column_num 0."
///   - q: "Which errors indicate bad indexes?"
///     a: "A blank index is 0, TRUE and FALSE are 1 and 0, and numeric text converts; other text returns #VALUE! and an error index returns that error. A 0 row_num/column_num selects an entire column/row (Excel behavior); negative or out-of-bounds indexes return #REF!."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: INDEX
/// Type: IndexFn
/// Min args: 2
/// Max args: 4
/// Variadic: false
/// Signature: INDEX(arg1: any@range, arg2: number@scalar, arg3?: number@scalar, arg4?: number@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE, RETURNS_REFERENCE
/// [formualizer-docgen:schema:end]
impl Function for IndexFn {
    fn caps(&self) -> FnCaps {
        FnCaps::PURE | FnCaps::RETURNS_REFERENCE
    }
    fn name(&self) -> &'static str {
        "INDEX"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(arg_byref_array);
        &SCHEMA
    }

    fn dispatch<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if let Some(value) = Self::precise_dispatch(self, args, ctx) {
            return Ok(value);
        }
        // An array row_num, column_num or area_num selects one value per element:
        // INDEX(B1:B6,{1;3;6}) is {B1;B3;B6}, with #REF! for an element out of range.
        if let Some(lifted) = crate::lift::lift_call(self.name(), args, |call| {
            match Self::precise_dispatch(self, call, ctx) {
                Some(value) => Ok(value),
                None => self.validated_dispatch(call, ctx),
            }
        })? {
            return Ok(lifted);
        }
        self.validated_dispatch(args, ctx)
    }

    fn eval_reference<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        if !(2..=4).contains(&args.len()) {
            return Some(Err(ExcelError::new(ExcelErrorKind::Value)));
        }
        // A multi-area reference, a union like (A1:B2,D1:E2) or a name defined
        // as one: area_num picks the area row_num and column_num select in.
        // An array index is `None` here, as for a single range, so the caller
        // takes the value path, where dispatch lifts over it.
        if let Some(areas) = args[0].reference_areas() {
            return match areas {
                Ok(areas) => Self::reference_from_areas(args, ctx, &areas),
                Err(error) => Some(Err(error)),
            };
        }
        let base = match args[0].resolve_reference_or_value() {
            Ok(FunctionResolution::Reference(reference)) => reference,
            Ok(FunctionResolution::ReferenceError(_) | FunctionResolution::Value(_)) | Err(_) => {
                return None;
            }
        };
        Self::reference_from_base(args, ctx, base)
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        // First try to handle as a reference
        if let Some(result) = self.eval_reference(args, ctx) {
            match result {
                Ok(reference) => Self::materialize_reference(ctx, &reference).or_else(|error| {
                    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)))
                }),
                Err(e) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
            }
        } else {
            // Handle array literal. A multi-area reference has no value of its
            // own: an index dispatch did not lift is #VALUE!, as for a range.
            if args.len() < 2
                || matches!(args[0].reference_areas(), Some(Ok(areas)) if areas.len() > 1)
            {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new(ExcelErrorKind::Value),
                )));
            }
            let v = args[0].value()?.into_literal();
            let table: Vec<Vec<LiteralValue>> = match v {
                LiteralValue::Array(rows) => rows,
                other => vec![vec![other]],
            };
            // Defensive: value() currently materializes omitted indexes as Number(0), so these
            // row/column checks are redundant while documenting whole-row/column intent.
            let index = if args[1].is_omitted() {
                0
            } else {
                match integer_parameter(args[1].value()?.into_literal(), &args[1]) {
                    Ok(index) => index,
                    Err(error) => {
                        return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)));
                    }
                }
            };

            // Optional explicit column_num (third argument).
            let explicit_col = if args.len() >= 3 {
                Some(if args[2].is_omitted() {
                    0
                } else {
                    match integer_parameter(args[2].value()?.into_literal(), &args[2]) {
                        Ok(column) => column,
                        Err(error) => {
                            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                                error,
                            )));
                        }
                    }
                })
            } else {
                None
            };

            let nrows = table.len();
            let ncols = table.iter().map(|r| r.len()).max().unwrap_or(0);
            let single_row = nrows == 1;

            // Map (index, optional column) to (row, col) exactly like eval_reference:
            // for a single-row input the lone index selects the column, otherwise it
            // selects the row and the omitted column acts as 0 (the entire row).
            let (row, col) = match explicit_col {
                Some(c) => (index, c),
                None if single_row => (1, index),
                None => (index, 0),
            };

            // Negative indices are #REF!. A 0 selects the entire row (column_num == 0)
            // or entire column (row_num == 0); 0 for both yields the whole array.
            // This mirrors the reference path so the two don't drift apart.
            let ref_err = || {
                crate::traits::CalcValue::Scalar(LiteralValue::Error(ExcelError::new(
                    ExcelErrorKind::Ref,
                )))
            };
            // area_num is checked before the positions, as in the reference path.
            match Self::check_single_area(args) {
                Some(Ok(())) => {}
                Some(Err(error)) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)));
                }
                None => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                        ExcelError::new(ExcelErrorKind::Value),
                    )));
                }
            }
            if row < 0 || col < 0 {
                return Ok(ref_err());
            }
            // A position beyond the address space is past the array's end.
            let (Ok(row), Ok(col)) = (usize::try_from(row), usize::try_from(col)) else {
                return Ok(ref_err());
            };

            // Wrap a multi-cell array result in a RangeView so aggregations
            // (SUM, etc.) iterate it, matching how the reference path returns
            // CalcValue::Range. A literal LiteralValue::Array would otherwise be
            // strict-coerced as a single scalar by numeric callers.
            let as_range = |rows: Vec<Vec<LiteralValue>>| {
                crate::traits::CalcValue::Range(
                    crate::engine::range_view::RangeView::from_owned_rows(rows, ctx.date_system()),
                )
            };

            if col == 0 {
                if row == 0 {
                    // INDEX(array, 0, 0) -> the whole array.
                    return Ok(as_range(table));
                }
                // INDEX(array, r, 0) -> the entire row r (scalar for a single-column array).
                if row > nrows {
                    return Ok(ref_err());
                }
                let r = &table[row - 1];
                if ncols == 1 {
                    return Ok(crate::traits::CalcValue::Scalar(
                        r.first().cloned().unwrap_or(LiteralValue::Empty),
                    ));
                }
                return Ok(as_range(vec![r.clone()]));
            }
            if row == 0 {
                // INDEX(array, 0, c) -> the entire column c (scalar for a single-row array).
                if col > ncols {
                    return Ok(ref_err());
                }
                let cidx = col - 1;
                if single_row {
                    return Ok(crate::traits::CalcValue::Scalar(
                        table[0].get(cidx).cloned().unwrap_or(LiteralValue::Empty),
                    ));
                }
                let column: Vec<Vec<LiteralValue>> = table
                    .iter()
                    .map(|r| vec![r.get(cidx).cloned().unwrap_or(LiteralValue::Empty)])
                    .collect();
                return Ok(as_range(column));
            }

            // 1-based positive indexing.
            if row > nrows || col > ncols {
                return Ok(ref_err());
            }
            let val = table
                .get(row - 1)
                .and_then(|r| r.get(col - 1))
                .cloned()
                .unwrap_or_else(|| LiteralValue::Error(ExcelError::new(ExcelErrorKind::Ref)));
            Ok(crate::traits::CalcValue::Scalar(val))
        }
    }
}

#[derive(Debug)]
pub struct OffsetFn;

/// Returns a reference shifted from a starting reference by rows and columns.
///
/// `OFFSET` is volatile and returns a reference that can point to a single cell or a resized
/// range, depending on the optional `height` and `width` arguments.
///
/// # Remarks
/// - `rows` and `cols` shift from the top-left of `reference`.
/// - If omitted, `height` and `width` default to the original reference size.
/// - A target that starts before row/column 1, a height or width below 1, or a resized
///   reference that reaches past row 1,048,576 or column 16,384 returns `#REF!`.
/// - Offsets and sizes are numbers: a blank cell is 0, logicals and numeric text convert,
///   and other text returns `#VALUE!`.
/// - In value context, a 1x1 result returns a scalar; larger results spill as an array.
/// - An array offset/size returns an array of references, one per element: reference
///   parameters such as SUBTOTAL's or SUMIF's evaluate once per reference, and `N`
///   reads each one. On its own it has no value (`#VALUE!`).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Move one row down and one column right"
/// grid:
///   A1: 10
///   B2: 42
/// formula: '=OFFSET(A1,1,1)'
/// expected: 42
/// ```
///
/// ```yaml,sandbox
/// title: "Offset and resize a range"
/// grid:
///   A1: 1
///   A2: 2
///   A3: 3
///   B1: 4
///   B2: 5
///   B3: 6
/// formula: '=SUM(OFFSET(A1,1,0,2,2))'
/// expected: 16
/// ```
///
/// ```yaml,docs
/// related:
///   - INDEX
///   - INDIRECT
///   - ADDRESS
/// faq:
///   - q: "What defaults are used when height and width are omitted?"
///     a: "OFFSET keeps the source reference size, then applies the row/column shift to that same-sized block."
///   - q: "When does OFFSET return #REF!?"
///     a: "It returns #REF! if the shifted start goes to row/column <= 0, if requested height/width are non-positive, or if the result reaches past the last row or column of the sheet."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: OFFSET
/// Type: OffsetFn
/// Min args: 3
/// Max args: 5
/// Variadic: false
/// Signature: OFFSET(arg1: range@range, arg2: number@scalar, arg3: number@scalar, arg4?: number@scalar, arg5?: number@scalar)
/// Arg schema: arg1{kinds=range,required=true,shape=range,by_ref=true,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE, VOLATILE, RETURNS_REFERENCE, DYNAMIC_DEPENDENCY
/// [formualizer-docgen:schema:end]
impl Function for OffsetFn {
    fn caps(&self) -> FnCaps {
        // OFFSET is volatile in Excel semantics and has runtime-dynamic dependencies.
        FnCaps::PURE | FnCaps::RETURNS_REFERENCE | FnCaps::VOLATILE | FnCaps::DYNAMIC_DEPENDENCY
    }
    fn name(&self) -> &'static str {
        "OFFSET"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(arg_byref_reference);
        &SCHEMA
    }

    fn eval_reference<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        Some(offset_reference(args, ctx))
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        match self.eval_reference(args, ctx) {
            Some(Ok(r)) => {
                let current_sheet = ctx.current_sheet();
                match ctx.resolve_range_view(&r, current_sheet) {
                    Ok(rv) => {
                        let (rows, cols) = rv.dims();
                        if rows == 1 && cols == 1 {
                            Ok(crate::traits::CalcValue::Scalar(
                                rv.as_1x1().unwrap_or(LiteralValue::Empty),
                            ))
                        } else {
                            Ok(crate::traits::CalcValue::Range(
                                rv.with_reference_extent(&r),
                            ))
                        }
                    }
                    Err(e) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
                }
            }
            // An array offset or size makes an array of references, which has
            // no value (#VALUE!); reference-taking callers lift over it.
            Some(Err(e)) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
            None => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Ref),
            ))),
        }
    }
}

/// The reference OFFSET returns. A reference whose rows/cols offset or
/// height/width reach past the edge of the sheet (row 1 to 1,048,576, column 1
/// to 16,384) is `#REF!`, as is a height or width below 1.
fn offset_reference<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<ReferenceType, ExcelError> {
    if args.len() < 3 {
        return Err(ExcelError::new(ExcelErrorKind::Value));
    }
    // A structured reference is the area it selects, taken from the table's
    // placement: only the cells of the result are read, so OFFSET(Table1[Qty],
    // 0,0,1,1) depends on the first data cell, not the whole column.
    let base = crate::traits::reference_as_area(ctx, args[0].as_reference_or_eval()?)?;
    let rows = offset_number(&args[1])?;
    let cols = offset_number(&args[2])?;

    // A whole column or row (B:B, 2:2) spans the full grid; other open-ended
    // ranges are clamped to the used region.
    let (sheet, sr, sc, er, ec) = resolve_reference_bounds(ctx, &base)?;

    // An omitted height or width keeps the reference's own size.
    let size = |index: usize, own: i64| match args.get(index) {
        Some(arg) if !arg.is_omitted() => offset_number(arg),
        _ => Ok(own),
    };
    let height = size(3, i64::from(er) - i64::from(sr) + 1)?;
    let width = size(4, i64::from(ec) - i64::from(sc) + 1)?;

    let off_grid = || ExcelError::new(ExcelErrorKind::Ref);
    if height < 1 || width < 1 {
        return Err(off_grid());
    }
    let top = i64::from(sr).checked_add(rows).ok_or_else(off_grid)?;
    let left = i64::from(sc).checked_add(cols).ok_or_else(off_grid)?;
    let bottom = top.checked_add(height - 1).ok_or_else(off_grid)?;
    let right = left.checked_add(width - 1).ok_or_else(off_grid)?;
    let on_grid = |first: i64, last: i64, max: u32| first >= 1 && last <= i64::from(max);
    if !on_grid(top, bottom, EXCEL_MAX_ROW) || !on_grid(left, right, EXCEL_MAX_COL) {
        return Err(off_grid());
    }
    // Every bound is now within 1..=1,048,576.
    let (top, left, bottom, right) = (top as u32, left as u32, bottom as u32, right as u32);
    Ok(if height == 1 && width == 1 {
        ReferenceType::cell(sheet, top, left)
    } else {
        ReferenceType::range(sheet, Some(top), Some(left), Some(bottom), Some(right))
    })
}

/// An OFFSET offset or size, which is a numeric parameter: a blank is 0,
/// logicals and numeric text convert like any number argument, other text is
/// `#VALUE!` and an error is itself. The number truncates toward zero
/// (saturating, so a huge offset is off the sheet). An array is lifted by the
/// caller (`ArgumentHandle::reference_array`) and is `#VALUE!` here.
fn offset_number(arg: &ArgumentHandle<'_, '_>) -> Result<i64, ExcelError> {
    integer_parameter(arg.value()?.into_literal(), arg)
}

/// A whole-number parameter (INDEX's row_num and column_num, OFFSET's offsets
/// and sizes): a blank is 0, TRUE and FALSE are 1 and 0, and numeric or date
/// text converts; other text is #VALUE!, an error is itself, and a fraction
/// truncates.
fn integer_parameter(value: LiteralValue, arg: &ArgumentHandle<'_, '_>) -> Result<i64, ExcelError> {
    match value {
        LiteralValue::Error(e) => Err(e),
        LiteralValue::Array(_) => Err(ExcelError::new(ExcelErrorKind::Value)),
        value => crate::coercion::to_serial_lenient_in_year(
            &value,
            arg.date_system(),
            Some(arg.current_year()),
        )
        .map(|n| n.trunc() as i64),
    }
}

fn arg_indirect() -> Vec<ArgSchema> {
    vec![
        ArgSchema {
            kinds: smallvec::smallvec![ArgKind::Text],
            required: true,
            by_ref: false,
            shape: ShapeKind::Scalar,
            coercion: CoercionPolicy::None,
            max: None,
            repeating: None,
            default: None,
        },
        ArgSchema {
            kinds: smallvec::smallvec![ArgKind::Logical, ArgKind::Number],
            required: false,
            by_ref: false,
            shape: ShapeKind::Scalar,
            coercion: CoercionPolicy::Logical,
            max: None,
            repeating: None,
            default: Some(LiteralValue::Boolean(true)),
        },
    ]
}

#[derive(Debug)]
pub struct IndirectFn;

/// Converts text into a reference and returns the referenced value or range.
///
/// `INDIRECT` lets formulas build references dynamically from strings such as `"A1"` or
/// `"Sheet2!B3:C5"`.
///
/// # Remarks
/// - `a1_style` defaults to `TRUE` (A1 style parsing).
/// - `a1_style=FALSE` (R1C1 parsing) is currently not implemented and returns `#N/IMPL!`.
/// - Invalid or unresolved references return `#REF!`; so does a number, logical or blank `ref_text`.
/// - A defined name must be defined as a reference: a name that holds a value
///   (`={1,2,3}`, `=5`, a formula that evaluates to no reference) is `#REF!`.
/// - An error in `ref_text` or `a1_style` is returned unchanged.
/// - The function is volatile because target references can change without direct dependency links.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Resolve a direct cell reference"
/// grid:
///   A1: 99
/// formula: '=INDIRECT("A1")'
/// expected: 99
/// ```
///
/// ```yaml,sandbox
/// title: "Resolve a range and aggregate it"
/// grid:
///   A1: 5
///   A2: 7
///   A3: 9
/// formula: '=SUM(INDIRECT("A1:A3"))'
/// expected: 21
/// ```
///
/// ```yaml,docs
/// related:
///   - ADDRESS
///   - INDEX
///   - OFFSET
/// faq:
///   - q: "What happens if a1_style is FALSE?"
///     a: "R1C1 parsing is not implemented here yet, so INDIRECT(...,FALSE) returns #N/IMPL!."
///   - q: "How are bad reference strings reported?"
///     a: "If the text cannot be parsed or resolved to a valid reference, INDIRECT returns #REF!."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: INDIRECT
/// Type: IndirectFn
/// Min args: 1
/// Max args: 2
/// Variadic: false
/// Signature: INDIRECT(arg1: text@scalar, arg2?: logical|number@scalar)
/// Arg schema: arg1{kinds=text,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=logical|number,required=false,shape=scalar,by_ref=false,coercion=Logical,max=None,repeating=None,default=true}
/// Caps: PURE, VOLATILE, RETURNS_REFERENCE, DYNAMIC_DEPENDENCY
/// [formualizer-docgen:schema:end]
impl Function for IndirectFn {
    fn caps(&self) -> FnCaps {
        FnCaps::PURE | FnCaps::RETURNS_REFERENCE | FnCaps::VOLATILE | FnCaps::DYNAMIC_DEPENDENCY
    }
    fn name(&self) -> &'static str {
        "INDIRECT"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(arg_indirect);
        &SCHEMA
    }

    fn eval_reference<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        // A structured reference spelled at run time is the cells it selects,
        // `#This Row` at the formula's row, like one written in the formula.
        // Excel takes the name of a name defined as a reference; a name that
        // holds a value has no cells to refer to.
        let holds_value = |name: &str| {
            args[0]
                .interpreter()
                .context
                .is_value_name(name, ctx.current_sheet())
        };
        Some(
            indirect_text_reference(args)
                .and_then(|reference| crate::traits::reference_as_area(ctx, reference))
                .and_then(|reference| indirect_reference_exists(reference, ctx, holds_value)),
        )
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        match self.eval_reference(args, ctx) {
            Some(Ok(r)) => {
                let current_sheet = ctx.current_sheet();
                match ctx.resolve_range_view(&r, current_sheet) {
                    Ok(rv) => {
                        let (rows, cols) = rv.dims();
                        if rows == 1 && cols == 1 {
                            Ok(crate::traits::CalcValue::Scalar(
                                rv.as_1x1().unwrap_or(LiteralValue::Empty),
                            ))
                        } else {
                            Ok(crate::traits::CalcValue::Range(
                                rv.with_reference_extent(&r),
                            ))
                        }
                    }
                    Err(e) => {
                        let mapped = if e.kind == ExcelErrorKind::Name {
                            ExcelError::new(ExcelErrorKind::Ref)
                        } else {
                            e
                        };
                        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                            mapped,
                        )))
                    }
                }
            }
            Some(Err(e)) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e))),
            None => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Ref),
            ))),
        }
    }
}

/// The reference INDIRECT's text spells (A1 style), before checking that it exists.
fn indirect_text_reference(args: &[ArgumentHandle<'_, '_>]) -> Result<ReferenceType, ExcelError> {
    if args.is_empty() {
        return Err(ExcelError::new(ExcelErrorKind::Value));
    }

    // ref_text is read as text: an error value propagates unchanged, and a
    // number, logical or blank becomes text that names no reference (#REF!).
    let ref_text = match args[0].value()?.into_literal() {
        LiteralValue::Text(s) => Some(s),
        LiteralValue::Error(e) => return Err(e),
        LiteralValue::Array(_) | LiteralValue::Pending => {
            return Err(ExcelError::new(ExcelErrorKind::Value));
        }
        _ => None,
    };

    // a1 is a logical argument: errors propagate, a blank cell is FALSE and
    // "TRUE"/"FALSE" text converts.
    let a1_style = if args.len() >= 2 {
        crate::coercion::to_logical(&args[1].value()?.into_literal())?
    } else {
        true
    };

    let Some(ref_text) = ref_text else {
        return Err(ExcelError::new(ExcelErrorKind::Ref));
    };

    if !a1_style {
        // The A1/R1C1 flag does not apply to defined names or tables (they are
        // neither A1 nor R1C1 syntax). Excel resolves `INDIRECT(name, FALSE)`
        // exactly like `INDIRECT(name)`, so handle those before refusing R1C1.
        // Real R1C1 cell/range text remains unsupported.
        return match ReferenceType::from_string(&ref_text) {
            Ok(reference @ (ReferenceType::NamedRange(_) | ReferenceType::Table(_))) => {
                Ok(reference)
            }
            _ => Err(ExcelError::new(ExcelErrorKind::NImpl).with_message(
                "INDIRECT with R1C1 style (second argument FALSE) is not yet supported",
            )),
        };
    }

    let sheet_name = |sheet: formualizer_common::SheetLocator<'_>| match sheet {
        formualizer_common::SheetLocator::Current => None,
        formualizer_common::SheetLocator::Name(name) => Some(name.to_string()),
        formualizer_common::SheetLocator::Id(_) => None,
    };
    match ReferenceType::parse_sheet_ref(&ref_text) {
        Ok(formualizer_common::SheetRef::Cell(cell)) => Ok(ReferenceType::Cell {
            sheet: sheet_name(cell.sheet),
            row: cell.coord.row() + 1,
            col: cell.coord.col() + 1,
            row_abs: cell.coord.row_abs(),
            col_abs: cell.coord.col_abs(),
        }),
        Ok(formualizer_common::SheetRef::Range(range)) => Ok(ReferenceType::Range {
            sheet: sheet_name(range.sheet),
            start_row: range.start_row.map(|b| b.index + 1),
            start_col: range.start_col.map(|b| b.index + 1),
            end_row: range.end_row.map(|b| b.index + 1),
            end_col: range.end_col.map(|b| b.index + 1),
            start_row_abs: range.start_row.map(|b| b.abs).unwrap_or(false),
            start_col_abs: range.start_col.map(|b| b.abs).unwrap_or(false),
            end_row_abs: range.end_row.map(|b| b.abs).unwrap_or(false),
            end_col_abs: range.end_col.map(|b| b.abs).unwrap_or(false),
        }),
        Err(_) => match ReferenceType::from_string(&ref_text) {
            Ok(reference @ (ReferenceType::NamedRange(_) | ReferenceType::Table(_))) => {
                Ok(reference)
            }
            _ => Err(ExcelError::new(ExcelErrorKind::Ref)),
        },
    }
}

/// INDIRECT's reference when it exists: an address past row 1,048,576 or
/// column 16,384, or a name or table that is not defined, is `#REF!` (not
/// `#NAME?`) wherever INDIRECT is used. So is a name that holds a value
/// (`holds_value`) rather than a reference.
fn indirect_reference_exists(
    reference: ReferenceType,
    ctx: &dyn FunctionContext<'_>,
    holds_value: impl Fn(&str) -> bool,
) -> Result<ReferenceType, ExcelError> {
    let off_grid = |row: Option<u32>, col: Option<u32>| {
        row.is_some_and(|row| row == 0 || row > EXCEL_MAX_ROW)
            || col.is_some_and(|col| col == 0 || col > EXCEL_MAX_COL)
    };
    let exists = match &reference {
        ReferenceType::Cell { row, col, .. } => !off_grid(Some(*row), Some(*col)),
        ReferenceType::Range {
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        } => !off_grid(*start_row, *start_col) && !off_grid(*end_row, *end_col),
        ReferenceType::NamedRange(name) if holds_value(name) => false,
        ReferenceType::NamedRange(_) | ReferenceType::Table(_) => {
            match ctx.resolve_range_view(&reference, ctx.current_sheet()) {
                Err(error) if error.kind == ExcelErrorKind::Cancelled => return Err(error),
                Err(error) => error.kind != ExcelErrorKind::Name,
                Ok(_) => true,
            }
        }
        _ => true,
    };
    if exists {
        Ok(reference)
    } else {
        Err(ExcelError::new(ExcelErrorKind::Ref))
    }
}

#[derive(Debug)]
pub struct HyperlinkFn;

/// Returns the friendly name of a hyperlink, or its link location when no name is given.
///
/// The returned value is `friendly_name` when the second argument is present,
/// otherwise `link_location`.
///
/// As in Excel, the friendly name keeps its type: numbers and booleans pass
/// through, so `=ISNUMBER(HYPERLINK("x",1))` is TRUE.
///
/// ```yaml,sandbox
/// title: "Hyperlink with a friendly name"
/// formula: '=HYPERLINK("https://example.com","Example")'
/// expected: "Example"
/// ```
///
/// ```yaml,sandbox
/// title: "Hyperlink without a friendly name"
/// formula: '=HYPERLINK("https://example.com")'
/// expected: "https://example.com"
/// ```
///
/// ```yaml,docs
/// related:
///   - INDIRECT
/// faq:
///   - q: "What does HYPERLINK return?"
///     a: "The friendly name when provided (keeping its type), otherwise the link location as text."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: HYPERLINK
/// Type: HyperlinkFn
/// Min args: 1
/// Max args: 2
/// Variadic: false
/// Signature: HYPERLINK(arg1: any@scalar, arg2?: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=any,required=false,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for HyperlinkFn {
    fn caps(&self) -> FnCaps {
        FnCaps::PURE
    }
    fn name(&self) -> &'static str {
        "HYPERLINK"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static SCHEMA: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            let mut optional = ArgSchema::any();
            optional.required = false;
            vec![ArgSchema::any(), optional]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let link = hyperlink_value(&args[0], true)?;
        if args.len() < 2 {
            return Ok(link);
        }
        // The friendly name shows as the value it is: 42 stays a number.
        hyperlink_value(&args[1], false)
    }
}

/// A HYPERLINK argument's display value: the link location as text, the
/// friendly name as the value it is (a blank cell shows 0).
///
/// Errors propagate as the argument's own error. Multi-cell references and
/// array constants cannot name a hyperlink target, so they surface `#VALUE!`
/// instead of leaking a debug-formatted array literal into the cell text.
/// A 1x1 array collapses to its single element.
fn hyperlink_value<'a, 'b>(
    arg: &ArgumentHandle<'a, 'b>,
    as_text: bool,
) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
    let lit = match arg.value()? {
        crate::traits::CalcValue::Scalar(lit) => lit,
        crate::traits::CalcValue::AnnotatedScalar(lit, _) => lit,
        crate::traits::CalcValue::Range(view) => {
            let (rows, cols) = view.dims();
            if rows == 1 && cols == 1 {
                view.get_cell(0, 0)
            } else {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new_value(),
                )));
            }
        }
        crate::traits::CalcValue::Callable(_) => {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Calc).with_message("LAMBDA value must be invoked"),
            )));
        }
    };
    let lit = match lit {
        LiteralValue::Array(arr) if arr.len() == 1 && arr[0].len() == 1 => arr[0][0].clone(),
        other => other,
    };
    Ok(crate::traits::CalcValue::Scalar(match lit {
        LiteralValue::Error(e) => LiteralValue::Error(e),
        LiteralValue::Array(_) => LiteralValue::Error(ExcelError::new_value()),
        other if as_text => LiteralValue::Text(crate::coercion::to_text_invariant(&other)),
        LiteralValue::Empty => LiteralValue::Number(0.0),
        other => other,
    }))
}

pub fn register_builtins() {
    crate::function_registry::register_builtin(std::sync::Arc::new(IndexFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(OffsetFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(IndirectFn));
    crate::function_registry::register_builtin(std::sync::Arc::new(HyperlinkFn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::lookup::MatchFn;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_common::error::{ExcelError, ExcelErrorKind};
    use formualizer_parse::parser::{ASTNode, ASTNodeType, Parser};

    fn interp(wb: &TestWorkbook) -> crate::interpreter::Interpreter<'_> {
        wb.interpreter()
    }

    fn evaluate_formula(formula: &str, wb: &TestWorkbook) -> Result<LiteralValue, ExcelError> {
        let mut parser = Parser::new(formula).unwrap();
        let ast = parser
            .parse()
            .map_err(|e| ExcelError::new(ExcelErrorKind::Error).with_message(e.message.clone()))?;
        Ok(interp(wb).evaluate_ast(&ast)?.into_literal())
    }

    #[test]
    fn index_returns_reference_and_materializes_in_value_context() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(42))
            .with_function(std::sync::Arc::new(IndexFn));
        let ctx = interp(&wb);

        // Build INDEX(A1:C3,2,2) expecting B2
        let array_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:C3".into(),
                reference: ReferenceType::Range {
                    sheet: None,
                    start_row: Some(1),
                    start_col: Some(1),
                    end_row: Some(3),
                    end_col: Some(3),
                    start_row_abs: false,
                    start_col_abs: false,
                    end_row_abs: false,
                    end_col_abs: false,
                },
            },
            None,
        );
        let row = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(2)), None);
        let col = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(2)), None);
        let call = ASTNode::new(
            ASTNodeType::Function {
                name: "INDEX".into(),
                args: vec![array_ref.clone(), row.clone(), col.clone()],
            },
            None,
        );

        // Reference context
        let r = ctx.evaluate_ast_as_reference(&call).expect("ref ok");
        match r {
            ReferenceType::Cell { row, col, .. } => {
                assert_eq!((row, col), (2, 2));
            }
            _ => panic!(),
        }

        // Value context (scalar materialization)
        let args = vec![
            ArgumentHandle::new(&array_ref, &ctx),
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
        ];
        let f = ctx.context.get_function("", "INDEX").unwrap();
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Number(42.0));
    }

    #[test]
    fn index_single_row_reference_uses_omitted_col_as_horizontal_position() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "C1", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn));
        let ctx = interp(&wb);

        let array_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:C1".into(),
                reference: ReferenceType::Range {
                    sheet: None,
                    start_row: Some(1),
                    start_col: Some(1),
                    end_row: Some(1),
                    end_col: Some(3),
                    start_row_abs: false,
                    start_col_abs: false,
                    end_row_abs: false,
                    end_col_abs: false,
                },
            },
            None,
        );
        let index = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(2)), None);
        let call = ASTNode::new(
            ASTNodeType::Function {
                name: "INDEX".into(),
                args: vec![array_ref.clone(), index.clone()],
            },
            None,
        );

        let r = ctx.evaluate_ast_as_reference(&call).expect("ref ok");
        match r {
            ReferenceType::Cell { row, col, .. } => assert_eq!((row, col), (1, 2)),
            _ => panic!(),
        }

        let args = vec![
            ArgumentHandle::new(&array_ref, &ctx),
            ArgumentHandle::new(&index, &ctx),
        ];
        let f = ctx.context.get_function("", "INDEX").unwrap();
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Number(20.0));
    }

    #[test]
    fn index_single_column_reference_keeps_omitted_col_as_vertical_position() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn));
        let ctx = interp(&wb);

        let array_ref = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:A3".into(),
                reference: ReferenceType::Range {
                    sheet: None,
                    start_row: Some(1),
                    start_col: Some(1),
                    end_row: Some(3),
                    end_col: Some(1),
                    start_row_abs: false,
                    start_col_abs: false,
                    end_row_abs: false,
                    end_col_abs: false,
                },
            },
            None,
        );
        let index = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(2)), None);
        let args = vec![
            ArgumentHandle::new(&array_ref, &ctx),
            ArgumentHandle::new(&index, &ctx),
        ];
        let f = ctx.context.get_function("", "INDEX").unwrap();
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Number(20.0));
    }

    #[test]
    fn index_rectangular_reference_omitted_col_returns_entire_row() {
        // INDEX(A1:B2, 2) on a 2-D range -> the entire row 2 (A2:B2), like column_num 0.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(200))
            .with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX(A1:B2,2)", &wb).unwrap();
        let LiteralValue::Array(rows) = value else {
            panic!("expected the 1x2 row, got {value:?}");
        };
        assert_eq!(rows.len(), 1);
        let flat: Vec<f64> = rows[0].iter().map(as_number).collect();
        assert_eq!(flat, vec![20.0, 200.0]);

        let ast = Parser::new("=INDEX(A1:B2,2)").unwrap().parse().unwrap();
        match interp(&wb).evaluate_ast_as_reference(&ast).expect("ref ok") {
            ReferenceType::Range {
                start_row,
                start_col,
                end_row,
                end_col,
                ..
            } => assert_eq!(
                (start_row, start_col, end_row, end_col),
                (Some(2), Some(1), Some(2), Some(2))
            ),
            other => panic!("expected A2:B2, got {other:?}"),
        }
    }

    #[test]
    fn index_static_bounds_spans_full_grid_on_whole_axes() {
        // A1:C3 keeps its declared bounds.
        assert_eq!(
            index_static_bounds(Some(1), Some(1), Some(3), Some(3)),
            Some((1, 1, 3, 3))
        );
        // A:C spans every row, so it is never one row whatever cells are in use.
        assert_eq!(
            index_static_bounds(None, Some(1), None, Some(3)),
            Some((1, 1, EXCEL_MAX_ROW, 3))
        );
        // 2:4 spans every column.
        assert_eq!(
            index_static_bounds(Some(2), None, Some(4), None),
            Some((2, 1, 4, EXCEL_MAX_COL))
        );
        // Other open ranges clamp to the used region instead.
        assert_eq!(index_static_bounds(Some(1), Some(1), None, Some(3)), None);
        assert_eq!(index_static_bounds(None, None, None, None), None);
    }

    #[test]
    fn index_rectangular_reference_omitted_col_matches_zero_col() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "C1", LiteralValue::Int(3))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(4))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(5))
            .with_cell_a1("Sheet1", "C2", LiteralValue::Int(6))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Int(7))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(MatchFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));

        for (formula, expected) in [
            ("=SUM(INDEX(A1:C3,2))", 15.0),
            ("=SUM(INDEX(A1:C3,2,))", 15.0),
            ("=SUM(INDEX(A1:C3,2,0))", 15.0),
            // Both selectors 0 (row 0, column omitted) -> the whole range.
            ("=SUM(INDEX(A1:C3,0))", 28.0),
            // The selected row is a reference, so it can end a range.
            ("=SUM(A1:INDEX(A1:C3,2))", 21.0),
            ("=MATCH(6,INDEX(A1:C3,2),0)", 3.0),
        ] {
            let value = evaluate_formula(formula, &wb).unwrap();
            assert_eq!(as_number(&value), expected, "{formula}");
        }

        let value = evaluate_formula("=INDEX(A1:C3,4)", &wb).unwrap();
        match value {
            LiteralValue::Error(err) => assert_eq!(err.kind, ExcelErrorKind::Ref),
            other => panic!("expected #REF!, got {other:?}"),
        }
    }

    #[test]
    fn index_single_row_reference_match_position_materializes_value() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "C1", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(MatchFn));

        let value = evaluate_formula("=INDEX(A1:C1,MATCH(20,A1:C1,0))", &wb).unwrap();
        assert_eq!(value, LiteralValue::Number(20.0));
    }

    #[test]
    fn index_single_row_reference_out_of_bounds_is_ref() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "C1", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX(A1:C1,4)", &wb).unwrap();
        match value {
            LiteralValue::Error(err) => assert_eq!(err.kind, ExcelErrorKind::Ref),
            other => panic!("expected #REF!, got {other:?}"),
        }
    }

    #[test]
    fn index_zero_column_degenerates_to_cell_in_single_column_range() {
        // INDEX(B1:B5, 2, 0) -> entire row 2 of a single-column range = B2.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX(B1:B5,2,0)", &wb).unwrap();
        assert_eq!(value, LiteralValue::Number(20.0));
    }

    #[test]
    fn index_zero_row_degenerates_to_cell_in_single_row_range() {
        // INDEX(A1:C1, 0, 2) -> entire column 2 of a single-row range = B1.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "C1", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX(A1:C1,0,2)", &wb).unwrap();
        assert_eq!(value, LiteralValue::Number(20.0));
    }

    #[test]
    fn index_zero_column_returns_entire_row_range() {
        // INDEX(A1:C3, 2, 0) -> entire row 2 (A2:C2); SUM materializes it.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "C2", LiteralValue::Int(3))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));

        let value = evaluate_formula("=SUM(INDEX(A1:C3,2,0))", &wb).unwrap();
        assert_eq!(value, LiteralValue::Number(6.0));
    }

    #[test]
    fn index_zero_row_returns_entire_column_range() {
        // INDEX(A1:C3, 0, 2) -> entire column 2 (B1:B3); SUM materializes it.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(4))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(5))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Int(6))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));

        let value = evaluate_formula("=SUM(INDEX(A1:C3,0,2))", &wb).unwrap();
        assert_eq!(value, LiteralValue::Number(15.0));
    }

    #[test]
    fn index_negative_index_is_ref() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX(A1:C3,-1,2)", &wb).unwrap();
        match value {
            LiteralValue::Error(err) => assert_eq!(err.kind, ExcelErrorKind::Ref),
            other => panic!("expected #REF!, got {other:?}"),
        }
    }

    fn as_number(v: &LiteralValue) -> f64 {
        match v {
            LiteralValue::Number(n) => *n,
            LiteralValue::Int(i) => *i as f64,
            other => panic!("expected number, got {other:?}"),
        }
    }

    #[test]
    fn index_array_constant_zero_column_returns_entire_row() {
        // INDEX({1,2,3},0) over an array constant -> the whole row {1,2,3}.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        let raw = evaluate_formula("=INDEX({1,2,3},0)", &wb).unwrap();
        let LiteralValue::Array(rows) = raw else {
            panic!("expected a 1x3 array, got {raw:?}");
        };
        assert_eq!(rows.len(), 1);
        let flat: Vec<f64> = rows[0].iter().map(as_number).collect();
        assert_eq!(flat, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn index_array_constant_zero_row_returns_entire_column() {
        // INDEX({1,2;3,4},0,2) over an array constant -> the whole column {2;4}.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        let raw = evaluate_formula("=INDEX({1,2;3,4},0,2)", &wb).unwrap();
        let LiteralValue::Array(rows) = raw else {
            panic!("expected a 2x1 array, got {raw:?}");
        };
        let flat: Vec<f64> = rows.iter().map(|r| as_number(&r[0])).collect();
        assert_eq!(flat, vec![2.0, 4.0]);
    }

    #[test]
    fn index_array_constant_zero_zero_returns_whole_array() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        let raw = evaluate_formula("=INDEX({1,2;3,4},0,0)", &wb).unwrap();
        let LiteralValue::Array(rows) = raw else {
            panic!("expected the whole 2x2 array, got {raw:?}");
        };
        let flat: Vec<f64> = rows.iter().flatten().map(as_number).collect();
        assert_eq!(flat, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn index_array_constant_row_zero_for_single_row_degenerates_to_scalar() {
        // INDEX({1,2,3},0,2): single-row array, entire column 2 -> scalar 2.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX({1,2,3},0,2)", &wb).unwrap();
        assert_eq!(as_number(&value), 2.0);
    }

    #[test]
    fn index_array_constant_omitted_column_on_2d_returns_entire_row() {
        // INDEX({1,2,3;4,5,6},2) -> the whole row {4,5,6}, like column_num 0.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        for formula in ["=INDEX({1,2,3;4,5,6},2)", "=INDEX({1,2,3;4,5,6},2,)"] {
            let raw = evaluate_formula(formula, &wb).unwrap();
            let LiteralValue::Array(rows) = raw else {
                panic!("{formula}: expected a 1x3 array, got {raw:?}");
            };
            assert_eq!(rows.len(), 1, "{formula}");
            let flat: Vec<f64> = rows[0].iter().map(as_number).collect();
            assert_eq!(flat, vec![4.0, 5.0, 6.0], "{formula}");
        }

        // Row 0 with the column omitted selects the whole array.
        let raw = evaluate_formula("=INDEX({1,2;3,4},0)", &wb).unwrap();
        let LiteralValue::Array(rows) = raw else {
            panic!("expected the whole 2x2 array, got {raw:?}");
        };
        let flat: Vec<f64> = rows.iter().flatten().map(as_number).collect();
        assert_eq!(flat, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn index_array_constant_vector_with_omitted_column_selects_element() {
        // One-row and one-column arrays keep the lone index as a position.
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        assert_eq!(
            as_number(&evaluate_formula("=INDEX({1,2,3},2)", &wb).unwrap()),
            2.0
        );
        assert_eq!(
            as_number(&evaluate_formula("=INDEX({1;2;3},2)", &wb).unwrap()),
            2.0
        );
        assert_eq!(
            as_number(&evaluate_formula("=INDEX({1,2;3,4},2,1)", &wb).unwrap()),
            3.0
        );
    }

    #[test]
    fn index_array_constant_negative_is_ref() {
        let wb = TestWorkbook::new().with_function(std::sync::Arc::new(IndexFn));

        let value = evaluate_formula("=INDEX({1,2,3},-1)", &wb).unwrap();
        match value {
            LiteralValue::Error(err) => assert_eq!(err.kind, ExcelErrorKind::Ref),
            other => panic!("expected #REF!, got {other:?}"),
        }
    }

    #[test]
    fn index_area_num_selects_the_single_area() {
        // INDEX(reference,row_num,column_num,area_num): a single range (or an
        // array) is area 1; any higher area lies outside it.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));
        let value = |formula: &str| evaluate_formula(formula, &wb).unwrap();
        let error_kind = |formula: &str| match value(formula) {
            LiteralValue::Error(err) => err.kind,
            other => panic!("{formula}: expected an error, got {other:?}"),
        };

        assert_eq!(as_number(&value("=INDEX(A1:B3,2,2,1)")), 20.0);
        assert_eq!(as_number(&value("=INDEX(A1:B3,2,2,1.9)")), 20.0);
        assert_eq!(as_number(&value("=INDEX(A1:B3,2,2,)")), 20.0);
        assert_eq!(as_number(&value("=INDEX(B1:B3,3,,1)")), 30.0);
        assert_eq!(as_number(&value("=SUM(INDEX(A1:B3,0,2,1))")), 50.0);
        assert_eq!(as_number(&value("=SUM(INDEX(A1:B3,2,2,1):B3)")), 50.0);
        assert_eq!(as_number(&value("=INDEX({1,2;3,4},2,2,1)")), 4.0);

        assert_eq!(error_kind("=INDEX(A1:B3,2,2,2)"), ExcelErrorKind::Ref);
        assert_eq!(error_kind("=SUM(INDEX(A1:B3,2,2,2))"), ExcelErrorKind::Ref);
        assert_eq!(error_kind("=INDEX({1,2;3,4},2,2,2)"), ExcelErrorKind::Ref);
        assert_eq!(error_kind("=INDEX(A1:B3,2,2,1/0)"), ExcelErrorKind::Div);
        assert_eq!(
            error_kind("=SUM(INDEX(A1:B3,2,2,1/0))"),
            ExcelErrorKind::Div
        );
        assert_eq!(error_kind("=INDEX(A1:B3,2,2,\"x\")"), ExcelErrorKind::Value);
        assert_eq!(
            error_kind("=SUM(INDEX(A1:B3,2,2,\"x\"))"),
            ExcelErrorKind::Value
        );
        assert_eq!(error_kind("=INDEX(A1:B3,1,1,1,1)"), ExcelErrorKind::Value);
        assert_eq!(
            error_kind("=SUM(INDEX(A1:B3,1,1,1,1))"),
            ExcelErrorKind::Value
        );
    }

    #[test]
    fn index_area_num_below_one_is_value_error() {
        // An area_num below 1 after truncation is no area number at all
        // (#VALUE!); only an area above the reference's one area is #REF!.
        // (TestWorkbook reads a cell it does not hold as #REF!, so the blank
        // C1 is set explicitly.)
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Int(30))
            .with_cell_a1("Sheet1", "C1", LiteralValue::Empty)
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));
        let error_kind = |formula: &str| match evaluate_formula(formula, &wb).unwrap() {
            LiteralValue::Error(err) => err.kind,
            other => panic!("{formula}: expected an error, got {other:?}"),
        };

        // C1 is blank, which reads as area 0.
        for area in ["0", "-1", "0.5", "0.9999999999", "-0.5", "C1", "FALSE"] {
            for formula in [
                format!("=INDEX(A1:B3,2,2,{area})"),
                format!("=SUM(INDEX(A1:B3,2,2,{area}))"),
                format!("=SUM(INDEX(A1:B3,2,2,{area}):B3)"),
                format!("=INDEX({{1,2;3,4}},2,2,{area})"),
            ] {
                assert_eq!(error_kind(&formula), ExcelErrorKind::Value, "{formula}");
            }
        }
        for area in ["2", "2.5", "3", "1E300"] {
            for formula in [
                format!("=INDEX(A1:B3,2,2,{area})"),
                format!("=SUM(INDEX(A1:B3,2,2,{area}))"),
                format!("=INDEX({{1,2;3,4}},2,2,{area})"),
            ] {
                assert_eq!(error_kind(&formula), ExcelErrorKind::Ref, "{formula}");
            }
        }
        // area_num is checked before the positions in both the reference and
        // the array path.
        assert_eq!(error_kind("=INDEX(A1:B3,-1,2,0)"), ExcelErrorKind::Value);
        assert_eq!(
            error_kind("=INDEX({1,2;3,4},-1,2,0)"),
            ExcelErrorKind::Value
        );
        assert_eq!(error_kind("=INDEX(A1:B3,9,9,0)"), ExcelErrorKind::Value);
        assert_eq!(
            error_kind("=INDEX({1,2;3,4},-1,2,1/0)"),
            ExcelErrorKind::Div
        );
    }

    #[test]
    fn index_area_num_converts_numeric_text() {
        // area_num is a number argument: numeric text converts, in a literal
        // and in a referenced cell, while other text stays #VALUE!.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Int(30))
            .with_cell_a1("Sheet1", "C2", LiteralValue::Text("1".into()))
            .with_cell_a1("Sheet1", "C3", LiteralValue::Text("2".into()))
            .with_cell_a1("Sheet1", "C4", LiteralValue::Text("x".into()))
            .with_cell_a1("Sheet1", "C5", LiteralValue::Boolean(true))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));
        let value = |formula: &str| evaluate_formula(formula, &wb).unwrap();
        let error_kind = |formula: &str| match value(formula) {
            LiteralValue::Error(err) => err.kind,
            other => panic!("{formula}: expected an error, got {other:?}"),
        };

        for area in ["\"1\"", "\"1.5\"", "C2", "TRUE", "C5"] {
            assert_eq!(
                as_number(&value(&format!("=INDEX(A1:B3,2,2,{area})"))),
                20.0,
                "{area}"
            );
            assert_eq!(
                as_number(&value(&format!("=SUM(INDEX(A1:B3,0,2,{area}))"))),
                50.0,
                "{area}"
            );
            assert_eq!(
                as_number(&value(&format!("=INDEX({{1,2;3,4}},2,2,{area})"))),
                4.0,
                "{area}"
            );
        }
        assert_eq!(error_kind("=INDEX(A1:B3,2,2,\"0\")"), ExcelErrorKind::Value);
        assert_eq!(error_kind("=INDEX(A1:B3,2,2,\"2\")"), ExcelErrorKind::Ref);
        assert_eq!(error_kind("=INDEX(A1:B3,2,2,C3)"), ExcelErrorKind::Ref);
        assert_eq!(error_kind("=SUM(INDEX(A1:B3,2,2,C3))"), ExcelErrorKind::Ref);
        assert_eq!(error_kind("=INDEX(A1:B3,2,2,C4)"), ExcelErrorKind::Value);
        assert_eq!(
            error_kind("=SUM(INDEX(A1:B3,2,2,C4))"),
            ExcelErrorKind::Value
        );
        assert_eq!(
            error_kind("=INDEX({1,2;3,4},2,2,\"x\")"),
            ExcelErrorKind::Value
        );
    }

    #[test]
    fn index_oversized_row_or_column_is_ref_error() {
        // A row_num or column_num past the selected area is #REF!, however
        // large: it never wraps onto a cell of the grid (2^32 + 1 is not row
        // 1) and never overflows, in a union area, a single range, a whole
        // column and the whole-row/whole-column (0) forms.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(3))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(4))
            .with_cell_a1("Sheet1", "D1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "D2", LiteralValue::Int(30))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));
        let error_kind = |formula: &str| match evaluate_formula(formula, &wb).unwrap() {
            LiteralValue::Error(err) => err.kind,
            other => panic!("{formula}: expected an error, got {other:?}"),
        };
        for formula in [
            "=INDEX((A1:B2,D1:E2),4294967297,1,2)",
            "=INDEX((A1:B2,D1:E2),1,4294967297,2)",
            "=INDEX((A2:B2,D2:E2),4294967295,1,2)",
            "=INDEX((A2:B2,D2:E2),1,4294967295,2)",
            "=INDEX(A1:B2,4294967297,1)",
            "=INDEX(A1:B2,1,4294967297)",
            "=INDEX(A1:B2,4294967296,2)",
            "=INDEX(A1:B2,4294967297)",
            "=INDEX(A1:A2,4294967297)",
            "=INDEX(A1:B1,4294967297)",
            "=INDEX(A:A,4294967295)",
            "=INDEX(1:1,1,4294967295)",
            "=INDEX(A1:B2,1E300,1)",
            "=SUM(INDEX(A1:B2,0,4294967297))",
            "=SUM(INDEX(A1:B2,4294967297,0))",
            "=SUM(INDEX((A1:B2,D1:E2),0,4294967297,2))",
        ] {
            assert_eq!(error_kind(formula), ExcelErrorKind::Ref, "{formula}");
        }
        // The largest in-range indexes still select.
        let value = |formula: &str| as_number(&evaluate_formula(formula, &wb).unwrap());
        assert_eq!(value("=INDEX((A1:B2,D1:E2),2,1,2)"), 30.0);
        assert_eq!(value("=INDEX(A1:B2,2,2)"), 4.0);
    }

    #[test]
    fn index_area_num_selects_an_area_of_an_intersection() {
        // Space is the intersection operator: (A1:B2,D1:E2) A2:E2 is the
        // reference with the areas A2:B2 and D2:E2, each area's overlap with
        // the other operand in order, and INDEX's area_num numbers those.
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(3))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(4))
            .with_cell_a1("Sheet1", "D1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "E1", LiteralValue::Int(20))
            .with_cell_a1("Sheet1", "D2", LiteralValue::Int(30))
            .with_cell_a1("Sheet1", "E2", LiteralValue::Int(40))
            .with_function(std::sync::Arc::new(IndexFn))
            .with_function(std::sync::Arc::new(crate::builtins::math::aggregate::SumFn));
        let value = |formula: &str| as_number(&evaluate_formula(formula, &wb).unwrap());
        let error_kind = |formula: &str| match evaluate_formula(formula, &wb).unwrap() {
            LiteralValue::Error(err) => err.kind,
            other => panic!("{formula}: expected an error, got {other:?}"),
        };
        assert_eq!(value("=INDEX(((A1:B2,D1:E2) A2:E2),1,1,2)"), 30.0);
        assert_eq!(value("=INDEX(((A1:B2,D1:E2) A2:E2),1,2,1)"), 4.0);
        assert_eq!(value("=INDEX((A2:E2 (A1:B2,D1:E2)),1,2,2)"), 40.0);
        assert_eq!(value("=SUM(INDEX(((A1:B2,D1:E2) A1:E1),0,0,2))"), 30.0);
        // An area that does not overlap leaves no area behind.
        assert_eq!(value("=INDEX(((A1:B2,D1:E2) D1:E2),2,2,1)"), 40.0);
        assert_eq!(
            error_kind("=INDEX(((A1:B2,D1:E2) A2:E2),1,1,3)"),
            ExcelErrorKind::Ref
        );
        assert_eq!(
            error_kind("=INDEX(((A1:B2,D1:E2) G1:G2),1,1)"),
            ExcelErrorKind::Null
        );
    }

    #[test]
    fn offset_returns_reference_and_materializes() {
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(5))
            .with_function(std::sync::Arc::new(OffsetFn));
        let ctx = interp(&wb);

        let base = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1".into(),
                reference: ReferenceType::Cell {
                    sheet: None,
                    row: 1,
                    col: 1,
                    row_abs: false,
                    col_abs: false,
                },
            },
            None,
        );
        let dr = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(1)), None);
        let dc = ASTNode::new(ASTNodeType::Literal(LiteralValue::Int(1)), None);
        let call = ASTNode::new(
            ASTNodeType::Function {
                name: "OFFSET".into(),
                args: vec![base.clone(), dr.clone(), dc.clone()],
            },
            None,
        );

        let r = ctx.evaluate_ast_as_reference(&call).expect("ref ok");
        match r {
            ReferenceType::Cell { row, col, .. } => assert_eq!((row, col), (2, 2)),
            _ => panic!(),
        }

        let args = vec![
            ArgumentHandle::new(&base, &ctx),
            ArgumentHandle::new(&dr, &ctx),
            ArgumentHandle::new(&dc, &ctx),
        ];
        let f = ctx.context.get_function("", "OFFSET").unwrap();
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Number(5.0));
    }

    #[test]
    fn offset_with_array_offsets_is_an_array_of_references() {
        crate::builtins::load_builtins();
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Int(4));
        let eval = |formula: &str| evaluate_formula(formula, &wb).unwrap();
        assert_eq!(
            eval("=SUMPRODUCT(SUBTOTAL(9,OFFSET(A1,{0;1;2},0))*{1;10;100})"),
            LiteralValue::Number(421.0)
        );
        assert_eq!(
            eval("=SUM(SUMIF(OFFSET(A1,0,0,{1,2,3}),\">0\"))"),
            LiteralValue::Number(11.0)
        );
        assert_eq!(
            eval("=SUMPRODUCT(N(OFFSET(A1,{2;0},0)))"),
            LiteralValue::Number(5.0)
        );
        assert!(matches!(
            eval("=OFFSET(A1,{0;1},0)"),
            LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value
        ));
    }

    #[test]
    fn offset_and_indirect_off_the_sheet_are_ref_and_offsets_coerce() {
        crate::builtins::load_builtins();
        let wb = TestWorkbook::new()
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Int(4));
        let eval = |formula: &str| evaluate_formula(formula, &wb).unwrap();
        let is_ref = |value: LiteralValue| matches!(value, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Ref);
        assert!(is_ref(eval("=OFFSET(A1,1048576,0)")));
        assert!(is_ref(eval("=OFFSET(A1,0,0,1,16385)")));
        assert!(is_ref(eval("=OFFSET(A1,4294967296,0)")));
        assert_eq!(
            eval("=SUMPRODUCT(IFERROR(SUBTOTAL(9,OFFSET(A1,{0;1048576},0)),100))"),
            LiteralValue::Number(101.0)
        );
        // Offsets are numbers: logicals and numeric text convert.
        assert_eq!(eval("=OFFSET(A1,TRUE,0)"), LiteralValue::Number(2.0));
        assert_eq!(eval("=OFFSET(A1,\"2\",0)"), LiteralValue::Number(4.0));
        // N reads the first cell of each reference.
        assert_eq!(
            eval("=SUMPRODUCT(N(OFFSET(A1,{0;1},0,2)))"),
            LiteralValue::Number(3.0)
        );
        assert_eq!(
            eval("=SUMPRODUCT(SUBTOTAL(9,IF(1,OFFSET(A1,{0;1},0))))"),
            LiteralValue::Number(3.0)
        );
        assert!(is_ref(eval("=SUBTOTAL(9,INDIRECT(\"XFE1\"))")));
    }
}
