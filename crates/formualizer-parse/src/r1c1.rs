//! R1C1 reference text, as Excel's INDIRECT reads it with `a1 = FALSE`.
use crate::parser::{ReferenceType, SheetSpec};
use crate::types::ParsingError;

const MAX_ROW: u32 = 1_048_576;
const MAX_COL: u32 = 16_384;

/// One row or column number of R1C1 text.
#[derive(Clone, Copy)]
enum Axis {
    /// `R2`: row 2.
    Absolute(u64),
    /// `R[-1]`, `R[]` or a bare `R`: that many rows from the origin's.
    Relative(i64),
}

impl ReferenceType {
    /// The reference R1C1 text names, as Excel for Windows reads it in
    /// `INDIRECT(text, FALSE)` (ops/excel-context-probe-20261006.md).
    ///
    /// `R2C3` is absolute; `R[-1]C[2]` is relative to `origin`, the 1-based
    /// row and column of the formula's cell; a bare `R` or `C`, or `[]`, is
    /// the origin's own row or column. `R2` and `R[1]` are whole rows, `C3`
    /// and `C[-1]` whole columns, and two cells, two rows or two columns
    /// joined by `:` a range, in either order. A sheet may lead (`Sheet1!`,
    /// `'My sheet'!`). Letters take either case, numbers leading zeros and an
    /// offset a `+`. Spaces may trail the text, follow the sheet's `!` and
    /// precede the `:`, and follow the `:` between two cells. A relative row
    /// or column past the grid's edge wraps around it. Anything else is an
    /// error: a number outside the grid, an offset of a whole grid or more, a
    /// leading space, a space inside a part, a 3-D sheet span, mixed kinds, a
    /// third part.
    pub fn parse_r1c1(text: &str, origin: Option<(u32, u32)>) -> Result<Self, ParsingError> {
        let invalid = || ParsingError::InvalidReference(format!("Invalid R1C1 reference: {text}"));
        let (spec, area) = Self::extract_sheet_spec(text.trim_end_matches(' '));
        let (sheet, area) = match spec {
            SheetSpec::None => (None, area.as_str()),
            SheetSpec::Single(name) => (Some(name), area.trim_start_matches(' ')),
            SheetSpec::Range { .. } => return Err(invalid()),
        };
        let mut parts = area.split(':');
        let first = parts
            .next()
            .and_then(|text| part(text.trim_end_matches(' ')))
            .ok_or_else(invalid)?;
        let second = match parts.next() {
            Some(text) => {
                let trimmed = text.trim_start_matches(' ');
                let second = part(trimmed).ok_or_else(invalid)?;
                let cell = |p: (Option<Axis>, Option<Axis>)| p.0.is_some() && p.1.is_some();
                if trimmed.len() != text.len() && !(cell(first) && cell(second)) {
                    return Err(invalid());
                }
                Some(second)
            }
            None => None,
        };
        if parts.next().is_some() {
            return Err(invalid());
        }
        let resolve = |axis: Option<Axis>, origin: Option<u32>, max: u32| match axis {
            None => Ok(None),
            Some(axis) => resolve(axis, origin, max).map(Some).ok_or_else(invalid),
        };
        let row = |p: (Option<Axis>, Option<Axis>)| resolve(p.0, origin.map(|o| o.0), MAX_ROW);
        let col = |p: (Option<Axis>, Option<Axis>)| resolve(p.1, origin.map(|o| o.1), MAX_COL);
        let (start_row, start_col) = (row(first)?, col(first)?);
        let (end_row, end_col) = match second {
            Some(second) => (row(second)?, col(second)?),
            None => (start_row, start_col),
        };
        // Both ends name the same kind: cells, rows or columns.
        if start_row.is_some() != end_row.is_some() || start_col.is_some() != end_col.is_some() {
            return Err(invalid());
        }
        let ordered = |a: Option<(u32, bool)>, b: Option<(u32, bool)>| match (a, b) {
            (Some(a), Some(b)) if a.0 > b.0 => (Some(b), Some(a)),
            other => other,
        };
        let (start_row, end_row) = ordered(start_row, end_row);
        let (start_col, end_col) = ordered(start_col, end_col);
        if second.is_none()
            && let (Some((row, row_abs)), Some((col, col_abs))) = (start_row, start_col)
        {
            return Ok(Self::Cell {
                sheet,
                row,
                col,
                row_abs,
                col_abs,
            });
        }
        let index = |bound: Option<(u32, bool)>| bound.map(|(index, _)| index);
        let absolute = |bound: Option<(u32, bool)>| bound.is_some_and(|(_, abs)| abs);
        Ok(Self::Range {
            sheet,
            start_row: index(start_row),
            start_col: index(start_col),
            end_row: index(end_row),
            end_col: index(end_col),
            start_row_abs: absolute(start_row),
            start_col_abs: absolute(start_col),
            end_row_abs: absolute(end_row),
            end_col_abs: absolute(end_col),
        })
    }
}

/// `R…C…`, `R…` or `C…`: the row and column axes it names.
fn part(text: &str) -> Option<(Option<Axis>, Option<Axis>)> {
    let mut rest = text;
    let mut take = |letter: u8| -> Option<Option<Axis>> {
        match rest.as_bytes().first() {
            Some(first) if first.eq_ignore_ascii_case(&letter) => {
                let (axis, after) = axis(&rest[1..])?;
                rest = after;
                Some(Some(axis))
            }
            _ => Some(None),
        }
    };
    let row = take(b'R')?;
    let col = take(b'C')?;
    (rest.is_empty() && (row.is_some() || col.is_some())).then_some((row, col))
}

/// The number after `R` or `C` and the text after it: digits, `[offset]`, or
/// nothing (offset 0).
fn axis(text: &str) -> Option<(Axis, &str)> {
    if let Some(inner) = text.strip_prefix('[') {
        let (offset, after) = inner.split_once(']')?;
        let digits = offset
            .strip_prefix('-')
            .or_else(|| offset.strip_prefix('+'))
            .unwrap_or(offset);
        let offset = if offset.is_empty() {
            0
        } else if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            let magnitude = digits.parse::<i64>().unwrap_or(i64::MAX);
            if offset.starts_with('-') {
                -magnitude
            } else {
                magnitude
            }
        } else {
            return None;
        };
        return Some((Axis::Relative(offset), after));
    }
    let end = text.bytes().take_while(u8::is_ascii_digit).count();
    let (digits, after) = text.split_at(end);
    let axis = if digits.is_empty() {
        Axis::Relative(0)
    } else {
        Axis::Absolute(digits.parse::<u64>().unwrap_or(u64::MAX))
    };
    Some((axis, after))
}

/// The 1-based index `axis` names on an axis of `max`, from `origin`, and
/// whether it is absolute. A relative index wraps around the axis.
fn resolve(axis: Axis, origin: Option<u32>, max: u32) -> Option<(u32, bool)> {
    match axis {
        Axis::Absolute(index) => (1..=u64::from(max))
            .contains(&index)
            .then_some((index as u32, true)),
        Axis::Relative(offset) => {
            let max = i64::from(max);
            if offset.unsigned_abs() >= max as u64 {
                return None;
            }
            let index = (i64::from(origin?) - 1 + offset).rem_euclid(max) + 1;
            Some((index as u32, false))
        }
    }
}
