//! TRIMRANGE and the trim-reference operators.
//!
//! Excel stores the trim-reference operators of a typed formula (`A1:.D6`,
//! `A1.:D6`, `A1.:.D6`) as the functions `_xlfn._TRO_TRAILING`,
//! `_xlfn._TRO_LEADING` and `_xlfn._TRO_ALL` around the range.

use crate::args::ArgSchema;
use crate::function::{Function, FunctionResolution};
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;
use formualizer_parse::parser::ReferenceType;

/// Which edges of a range to trim, as TRIMRANGE numbers them: 0 none,
/// 1 leading, 2 trailing, 3 both.
#[derive(Clone, Copy, Debug)]
struct Edges {
    leading: bool,
    trailing: bool,
}

impl Edges {
    fn from_mode(mode: u8) -> Self {
        Self {
            leading: mode & 1 != 0,
            trailing: mode & 2 != 0,
        }
    }
}

/// The reference left when the empty rows and columns at the trimmed edges
/// of `reference` are removed. A cell is empty only when it holds nothing; a
/// formula returning "" or 0 keeps its row and column. A range with nothing
/// left to keep is `#REF!`.
fn trim_reference(
    ctx: &dyn FunctionContext<'_>,
    reference: ReferenceType,
    rows: Edges,
    cols: Edges,
) -> Result<ReferenceType, ExcelError> {
    let area = crate::traits::reference_as_area(ctx, reference)?;
    let (sheet, sr, sc, er, ec) =
        crate::builtins::reference_fns::resolve_reference_bounds(ctx, &area)?;
    // The bounds of the cells in use, absolute and 1-based.
    let mut used: Option<(u32, u32, u32, u32)> = None;
    let view = ctx.resolve_range_view(&area, ctx.current_sheet())?;
    let (top, left) = (view.start_row() as u32 + 1, view.start_col() as u32 + 1);
    let mut row = top;
    view.for_each_row(&mut |cells| {
        for (offset, cell) in cells.iter().enumerate() {
            if matches!(cell, LiteralValue::Empty) {
                continue;
            }
            let col = left + offset as u32;
            used = Some(match used {
                None => (row, col, row, col),
                Some((r0, c0, r1, c1)) => (r0.min(row), c0.min(col), r1.max(row), c1.max(col)),
            });
        }
        row += 1;
        Ok(())
    })?;
    let (sr, sc, er, ec) = match used {
        Some((r0, c0, r1, c1)) => (
            if rows.leading { r0 } else { sr },
            if cols.leading { c0 } else { sc },
            if rows.trailing { r1 } else { er },
            if cols.trailing { c1 } else { ec },
        ),
        None if rows.leading || rows.trailing || cols.leading || cols.trailing => {
            return Err(ExcelError::new(ExcelErrorKind::Ref));
        }
        None => (sr, sc, er, ec),
    };
    Ok(if sr == er && sc == ec {
        ReferenceType::cell(sheet, sr, sc)
    } else {
        ReferenceType::range(sheet, Some(sr), Some(sc), Some(er), Some(ec))
    })
}

/// The reference a trimming function's first argument holds: `None` when it
/// holds a value instead (an array or a single value, returned as it is).
fn base_reference(arg: &ArgumentHandle<'_, '_>) -> Option<Result<ReferenceType, ExcelError>> {
    if let Some(areas) = arg.reference_areas() {
        return Some(match areas {
            Ok(mut areas) if areas.len() == 1 => Ok(areas.remove(0)),
            Ok(_) => Err(ExcelError::new(ExcelErrorKind::Value)
                .with_message("TRIMRANGE takes a single area")),
            Err(e) => Err(e),
        });
    }
    match arg.resolve_reference_or_value() {
        Ok(FunctionResolution::Reference(reference)) => Some(Ok(reference)),
        Ok(FunctionResolution::ReferenceError(e)) => Some(Err(e)),
        Ok(FunctionResolution::Value(_)) | Err(_) => None,
    }
}

fn materialize<'b>(
    ctx: &dyn FunctionContext<'b>,
    reference: Result<ReferenceType, ExcelError>,
) -> CalcValue<'b> {
    let error = |e: ExcelError| CalcValue::Scalar(LiteralValue::Error(e));
    let reference = match reference {
        Ok(reference) => reference,
        Err(e) => return error(e),
    };
    match ctx.resolve_range_view(&reference, ctx.current_sheet()) {
        Ok(view) if view.dims() == (1, 1) => {
            CalcValue::Scalar(view.as_1x1().unwrap_or(LiteralValue::Empty))
        }
        Ok(view) => CalcValue::Range(view.with_reference_extent(&reference)),
        Err(e) => error(e),
    }
}

/// TRIMRANGE's trim_rows or trim_cols: omitted is 3; otherwise a number
/// argument that must be the whole number 0, 1, 2 or 3 (2.5 is `#VALUE!`).
fn trim_mode(arg: Option<&ArgumentHandle<'_, '_>>) -> Result<u8, ExcelError> {
    let Some(arg) = arg.filter(|arg| !arg.is_omitted()) else {
        return Ok(3);
    };
    let n = match arg.value()?.into_literal() {
        LiteralValue::Error(e) => return Err(e),
        other => crate::builtins::utils::coerce_num(&other)?,
    };
    if n.fract() == 0.0 && (0.0..=3.0).contains(&n) {
        Ok(n as u8)
    } else {
        Err(ExcelError::new(ExcelErrorKind::Value))
    }
}

/// The shape of TRIMRANGE's result when trim_rows or trim_cols holds several
/// values: Excel evaluates the function once per element, and an element
/// cannot hold the array each evaluation returns, so every element is
/// `#VALUE!` (`#N/A` where the two shapes do not broadcast).
fn mode_array<'b>(args: &[ArgumentHandle<'_, 'b>]) -> Result<Option<CalcValue<'b>>, ExcelError> {
    let mut dims = Vec::new();
    for arg in args.iter().skip(1).take(2) {
        if arg.is_omitted() {
            continue;
        }
        if let CalcValue::Range(view) = arg.value()?
            && view.dims() != (1, 1)
        {
            dims.push(view.dims());
        }
    }
    if dims.is_empty() {
        return Ok(None);
    }
    let rows = dims.iter().map(|d| d.0).max().unwrap();
    let cols = dims.iter().map(|d| d.1).max().unwrap();
    let grid = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let fits = dims
                        .iter()
                        .all(|&(h, w)| (h == 1 || r < h) && (w == 1 || c < w));
                    LiteralValue::Error(ExcelError::new(if fits {
                        ExcelErrorKind::Value
                    } else {
                        ExcelErrorKind::Na
                    }))
                })
                .collect()
        })
        .collect();
    Ok(Some(crate::lift::array_result(grid, args[0].date_system())))
}

#[derive(Debug)]
pub struct TrimRangeFn;
/// Excludes the empty rows and columns at the outer edges of a range.
///
/// `TRIMRANGE(range, [trim_rows], [trim_cols])`: each mode is 0 (keep), 1
/// (trim leading), 2 (trim trailing) or 3 (both, the default).
///
/// # Remarks
/// - A cell counts as empty only when it holds nothing: a 0, an error or a
///   formula returning "" stays.
/// - The result is a reference; with nothing left to keep it is `#REF!`.
/// - A mode other than the whole numbers 0 to 3 returns `#VALUE!`; a union
///   of several areas returns `#VALUE!`. An array or single value is returned
///   as it is.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Rows of a trimmed empty range"
/// formula: '=TRIMRANGE(A1:D6)'
/// expected: "#REF!"
/// ```
///
/// ```yaml,docs
/// related:
///   - TAKE
///   - DROP
///   - FILTER
/// faq:
///   - q: "Is a formula returning empty text trimmed?"
///     a: "No. Only cells that hold nothing are empty."
/// ```
impl Function for TrimRangeFn {
    func_caps!(PURE, RETURNS_REFERENCE, MAY_SPILL);
    fn name(&self) -> &'static str {
        "TRIMRANGE"
    }
    fn min_args(&self) -> usize {
        1
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
            std::sync::LazyLock::new(|| vec![ArgSchema::any()]);
        &SCHEMA
    }
    fn eval_reference<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        if !(1..=3).contains(&args.len()) {
            return Some(Err(ExcelError::new(ExcelErrorKind::Value)));
        }
        let modes = trim_mode(args.get(1)).and_then(|rows| Ok((rows, trim_mode(args.get(2))?)));
        let base = base_reference(&args[0])?;
        Some(modes.and_then(|(rows, cols)| {
            trim_reference(ctx, base?, Edges::from_mode(rows), Edges::from_mode(cols))
        }))
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        if let Some(array) = mode_array(args)? {
            return Ok(array);
        }
        match self.eval_reference(args, ctx) {
            Some(reference) => Ok(materialize(ctx, reference)),
            None => {
                if let Err(e) = trim_mode(args.get(1)).and_then(|_| trim_mode(args.get(2))) {
                    return Ok(CalcValue::Scalar(LiteralValue::Error(e)));
                }
                args[0].value()
            }
        }
    }
}

/// A trim-reference operator: `_TRO_LEADING`, `_TRO_TRAILING` or `_TRO_ALL`
/// around a range trims its rows and columns at those edges.
#[derive(Debug)]
pub struct TrimReferenceFn {
    name: &'static str,
    edges: Edges,
}

impl Function for TrimReferenceFn {
    func_caps!(PURE, RETURNS_REFERENCE, MAY_SPILL);
    fn name(&self) -> &'static str {
        self.name
    }
    fn min_args(&self) -> usize {
        1
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
            std::sync::LazyLock::new(|| vec![ArgSchema::any()]);
        &SCHEMA
    }
    fn eval_reference<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        if args.len() != 1 {
            return Some(Err(ExcelError::new(ExcelErrorKind::Value)));
        }
        let base = base_reference(&args[0]).unwrap_or_else(|| {
            Err(ExcelError::new(ExcelErrorKind::Value)
                .with_message("A trim operator takes a range"))
        });
        Some(base.and_then(|base| trim_reference(ctx, base, self.edges, self.edges)))
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let reference = self
            .eval_reference(args, ctx)
            .unwrap_or_else(|| Err(ExcelError::new(ExcelErrorKind::Value)));
        Ok(materialize(ctx, reference))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(TrimRangeFn));
    for (name, mode) in [("_TRO_LEADING", 1), ("_TRO_TRAILING", 2), ("_TRO_ALL", 3)] {
        crate::function_registry::register_builtin(Arc::new(TrimReferenceFn {
            name,
            edges: Edges::from_mode(mode),
        }));
    }
}
