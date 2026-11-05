//! GROUPBY and PIVOTBY: aggregate values by row (and column) keys with a
//! LAMBDA or a function named as a value, such as `SUM`.

use super::super::utils::collapse_if_scalar;
use crate::builtins::lambda::{
    cancellation_checkpoint, element_value, function_arg, invoke, invoke_cancellable,
};
use crate::engine::CancelToken;
use crate::function::{FnCaps, Function};
use crate::traits::{ArgumentHandle, CalcValue, CustomCallable, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

type Grid = Vec<Vec<LiteralValue>>;

#[derive(Debug)]
enum GroupFailure {
    Value(ExcelError),
    Native(ExcelError),
}

impl From<ExcelError> for GroupFailure {
    fn from(error: ExcelError) -> Self {
        Self::Value(error)
    }
}

fn raw_argument_failure(error: ExcelError) -> GroupFailure {
    if error.kind == ExcelErrorKind::Cancelled {
        GroupFailure::Native(error)
    } else {
        GroupFailure::Value(error)
    }
}

fn value_error(msg: &str) -> ExcelError {
    ExcelError::new(ExcelErrorKind::Value).with_message(msg.to_string())
}

fn blank() -> LiteralValue {
    LiteralValue::Text(String::new())
}

fn grid_arg(
    arg: &ArgumentHandle<'_, '_>,
    cancel: Option<&CancelToken>,
) -> Result<Grid, GroupFailure> {
    let view = arg.range_view_or_scalar().map_err(|error| {
        if error.kind == ExcelErrorKind::Cancelled && !arg.resolved_as_scalar_error() {
            GroupFailure::Native(error)
        } else {
            GroupFailure::Value(error)
        }
    })?;
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let mut rows = Vec::new();
    view.for_each_row(&mut |row| {
        cancellation_checkpoint(cancel)?;
        rows.push(row.to_vec());
        Ok(())
    })
    .map_err(raw_argument_failure)?;
    Ok(rows)
}

fn width(grid: &Grid) -> usize {
    grid.iter().map(Vec::len).max().unwrap_or(0)
}

fn cell(grid: &Grid, r: usize, c: usize) -> LiteralValue {
    grid.get(r)
        .and_then(|row| row.get(c))
        .cloned()
        .unwrap_or(LiteralValue::Empty)
}

/// An optional argument's value; `None` when absent or left empty.
fn optional_arg(
    args: &[ArgumentHandle<'_, '_>],
    index: usize,
) -> Result<Option<LiteralValue>, GroupFailure> {
    match args.get(index) {
        Some(arg) if !arg.is_omitted() => {
            match arg.value().map_err(raw_argument_failure)?.into_literal() {
                LiteralValue::Error(error) => Err(GroupFailure::Value(error)),
                value => Ok(Some(value)),
            }
        }
        _ => Ok(None),
    }
}

fn optional_int(
    args: &[ArgumentHandle<'_, '_>],
    index: usize,
) -> Result<Option<i64>, GroupFailure> {
    optional_arg(args, index)?
        .map(|value| {
            crate::coercion::to_number_argument(&value)
                .map(|number| number.trunc() as i64)
                .map_err(GroupFailure::Value)
        })
        .transpose()
}

/// A sort order: one column number or a vector of them, negative for
/// descending.
fn sort_spec(
    args: &[ArgumentHandle<'_, '_>],
    index: usize,
) -> Result<Option<Vec<i64>>, GroupFailure> {
    let Some(value) = optional_arg(args, index)? else {
        return Ok(None);
    };
    let items: Vec<LiteralValue> = match value {
        LiteralValue::Array(rows) => rows.into_iter().flatten().collect(),
        v => vec![v],
    };
    items
        .iter()
        .map(|v| match v {
            LiteralValue::Error(error) => Err(GroupFailure::Value(error.clone())),
            value => crate::coercion::to_number_argument(value)
                .map(|number| number.trunc() as i64)
                .map_err(GroupFailure::Value),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Excel's sort order: numbers, then text (ignoring case), then logicals,
/// then errors; blanks always last.
fn type_rank(v: &LiteralValue) -> u8 {
    match v {
        LiteralValue::Number(_)
        | LiteralValue::Int(_)
        | LiteralValue::Date(_)
        | LiteralValue::DateTime(_)
        | LiteralValue::Time(_)
        | LiteralValue::Duration(_) => 0,
        LiteralValue::Text(s) if s.is_empty() => 4,
        LiteralValue::Text(_) => 1,
        LiteralValue::Boolean(_) => 2,
        LiteralValue::Error(_) => 3,
        _ => 4,
    }
}

fn number(v: &LiteralValue) -> f64 {
    match v {
        LiteralValue::Number(n) => *n,
        LiteralValue::Int(i) => *i as f64,
        other => other.as_serial_number().unwrap_or(0.0),
    }
}

fn compare_values(a: &LiteralValue, b: &LiteralValue) -> Ordering {
    let (ra, rb) = (type_rank(a), type_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (LiteralValue::Text(x), LiteralValue::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (LiteralValue::Boolean(x), LiteralValue::Boolean(y)) => x.cmp(y),
        (LiteralValue::Error(x), LiteralValue::Error(y)) => {
            x.kind.to_string().cmp(&y.kind.to_string())
        }
        _ if ra == 0 => number(a).partial_cmp(&number(b)).unwrap_or(Ordering::Equal),
        _ => Ordering::Equal,
    }
}

/// Grouping identity of a key value: text ignores case.
fn key_part(v: &LiteralValue) -> String {
    match v {
        LiteralValue::Text(s) if s.is_empty() => "E".to_string(),
        LiteralValue::Text(s) => format!("T{}", s.to_lowercase()),
        LiteralValue::Boolean(b) => format!("B{b}"),
        LiteralValue::Error(e) => format!("X{}", e.kind),
        LiteralValue::Empty => "E".to_string(),
        // Excel has no negative zero: a -0 key (`ROUND(-0.4,0)`) is the 0 group.
        other => format!(
            "N{}",
            crate::coercion::normalize_zero(number(other)).to_bits()
        ),
    }
}

/// The distinct keys in first-appearance order, and each row's key index.
fn distinct_keys(
    keys: &[Vec<LiteralValue>],
    cancel: Option<&CancelToken>,
) -> Result<(Vec<Vec<LiteralValue>>, Vec<usize>), GroupFailure> {
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let mut index: HashMap<Vec<String>, usize> = HashMap::new();
    let mut distinct = Vec::new();
    let mut of_row = Vec::with_capacity(keys.len());
    for key in keys {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        let id: Vec<String> = key
            .iter()
            .map(|value| {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                Ok(key_part(value))
            })
            .collect::<Result<Vec<_>, GroupFailure>>()?;
        let next = distinct.len();
        let k = *index.entry(id).or_insert(next);
        if k == next {
            distinct.push(key.clone());
        }
        of_row.push(k);
    }
    Ok((distinct, of_row))
}

fn same_prefix(a: &[LiteralValue], b: &[LiteralValue], len: usize) -> bool {
    a.iter()
        .zip(b)
        .take(len)
        .all(|(x, y)| key_part(x) == key_part(y))
}

/// One line of a result axis: a group, a subtotal of the groups sharing the
/// first `prefix` key columns, or the grand total (`prefix == 0`).
struct Entry {
    label: Vec<LiteralValue>,
    keys: Vec<usize>,
}

/// Sorts the distinct keys. `spec` numbers the key columns from 1; a number
/// past them sorts by `extra(key, n)` (an aggregate). Without a spec, keys
/// sort ascending column by column. Groups sharing a parent key stay
/// together so subtotals can follow them.
fn order_keys(
    distinct: &[Vec<LiteralValue>],
    spec: Option<&[i64]>,
    extra: &dyn Fn(usize, usize) -> LiteralValue,
    columns: usize,
    cancel: Option<&CancelToken>,
) -> Result<Vec<usize>, GroupFailure> {
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let mut order: Vec<usize> = (0..distinct.len()).collect();
    let default: Vec<i64> = (1..=columns as i64).collect();
    let spec = spec.unwrap_or(&default);
    order.sort_by(|&a, &b| {
        for &s in spec {
            let col = s.unsigned_abs() as usize - 1;
            let (x, y) = if col < columns {
                (distinct[a][col].clone(), distinct[b][col].clone())
            } else {
                (extra(a, col - columns), extra(b, col - columns))
            };
            let ord = compare_values(&x, &y);
            let ord = if s < 0 { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    if columns > 1 {
        // Keep each parent group contiguous, in order of first appearance.
        let position: Vec<usize> = {
            let mut p = vec![0; distinct.len()];
            for (i, &k) in order.iter().enumerate() {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                p[k] = i;
            }
            p
        };
        let first = |k: usize, len: usize| {
            order
                .iter()
                .filter(|&&o| same_prefix(&distinct[o], &distinct[k], len))
                .map(|&o| position[o])
                .min()
                .unwrap_or(0)
        };
        let rank: Vec<Vec<usize>> = (0..distinct.len())
            .map(|k| {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                let mut r: Vec<usize> = (1..columns)
                    .map(|len| {
                        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                        Ok(first(k, len))
                    })
                    .collect::<Result<Vec<_>, GroupFailure>>()?;
                r.push(position[k]);
                Ok(r)
            })
            .collect::<Result<Vec<_>, GroupFailure>>()?;
        order.sort_by(|&a, &b| rank[a].cmp(&rank[b]));
    }
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    Ok(order)
}

/// Lays out an axis: the sorted groups, subtotals for the first
/// `|depth| - 1` key levels and a grand total when `depth != 0`; negative
/// depths put totals before the groups they sum.
fn axis_entries(
    distinct: &[Vec<LiteralValue>],
    order: &[usize],
    depth: i64,
    columns: usize,
    cancel: Option<&CancelToken>,
) -> Result<Vec<Entry>, GroupFailure> {
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let top = depth < 0;
    let levels = (depth.unsigned_abs() as usize)
        .saturating_sub(1)
        .min(columns.saturating_sub(1));
    let label = |k: usize, len: usize| -> Result<Vec<LiteralValue>, GroupFailure> {
        (0..columns)
            .map(|c| {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                Ok(if c < len {
                    distinct[k][c].clone()
                } else {
                    blank()
                })
            })
            .collect()
    };
    let members = |k: usize, len: usize| -> Result<Vec<usize>, GroupFailure> {
        let mut members = Vec::new();
        for &o in order {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            if same_prefix(&distinct[o], &distinct[k], len) {
                members.push(o);
            }
        }
        Ok(members)
    };
    let total = || -> Result<Entry, GroupFailure> {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        Ok(Entry {
            label: (0..columns)
                .map(|c| {
                    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                    Ok(if c == 0 {
                        LiteralValue::Text("Total".into())
                    } else {
                        blank()
                    })
                })
                .collect::<Result<Vec<_>, GroupFailure>>()?,
            keys: order.to_vec(),
        })
    };
    let mut entries = Vec::new();
    if top && depth != 0 {
        entries.push(total()?);
    }
    for (i, &k) in order.iter().enumerate() {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        if top {
            for len in 1..=levels {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                if i == 0 || !same_prefix(&distinct[order[i - 1]], &distinct[k], len) {
                    entries.push(Entry {
                        label: label(k, len)?,
                        keys: members(k, len)?,
                    });
                }
            }
        }
        entries.push(Entry {
            label: label(k, columns)?,
            keys: vec![k],
        });
        if !top {
            for len in (1..=levels).rev() {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                if i + 1 == order.len() || !same_prefix(&distinct[order[i + 1]], &distinct[k], len)
                {
                    entries.push(Entry {
                        label: label(k, len)?,
                        keys: members(k, len)?,
                    });
                }
            }
        }
    }
    if !top && depth != 0 {
        entries.push(total()?);
    }
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    Ok(entries)
}

/// Header detection when field_headers is omitted: the values have a header
/// when their first value is text and the second a number.
fn detect_headers(values: &Grid) -> bool {
    matches!(cell(values, 0, 0), LiteralValue::Text(ref s) if !s.is_empty())
        && matches!(
            cell(values, 1, 0),
            LiteralValue::Number(_) | LiteralValue::Int(_)
        )
}

/// `(has_headers, show_headers)` from the field_headers argument.
fn header_mode(field_headers: Option<i64>, values: &Grid) -> Result<(bool, bool), ExcelError> {
    Ok(match field_headers {
        None => {
            let has = detect_headers(values);
            (has, has)
        }
        Some(0) => (false, false),
        Some(1) => (true, false),
        Some(2) => (false, true),
        Some(3) => (true, true),
        Some(_) => return Err(value_error("field_headers must be 0-3")),
    })
}

/// Data rows kept by the header setting and the filter array.
fn kept_rows(
    total_rows: usize,
    start: usize,
    filter: Option<LiteralValue>,
    cancel: Option<&CancelToken>,
) -> Result<Vec<usize>, GroupFailure> {
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let keep: Option<Vec<LiteralValue>> = filter.map(|f| match f {
        LiteralValue::Array(rows) => rows.into_iter().flatten().collect(),
        v => vec![v],
    });
    let mut rows = Vec::new();
    for r in start..total_rows {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        let include = match &keep {
            None => true,
            Some(flags) => {
                let idx = if flags.len() == total_rows {
                    r
                } else {
                    r - start
                };
                match flags.get(idx) {
                    Some(LiteralValue::Error(e)) => return Err(GroupFailure::Value(e.clone())),
                    Some(v) => crate::coercion::to_logical(v).unwrap_or(false),
                    None => return Err(value_error("filter_array does not match the data").into()),
                }
            }
        };
        if include {
            rows.push(r);
        }
    }
    if rows.is_empty() {
        return Err(ExcelError::new(ExcelErrorKind::Calc)
            .with_message("No rows to group")
            .into());
    }
    Ok(rows)
}

/// The function argument, which takes the group's values (and, for
/// PERCENTOF-style functions, the values it is relative to).
fn aggregator(arg: &ArgumentHandle<'_, '_>) -> Result<Arc<dyn CustomCallable>, GroupFailure> {
    match function_arg(arg).map_err(raw_argument_failure)? {
        Ok(callable) if callable.accepts(1) || callable.accepts(2) => Ok(callable),
        Ok(_) => Err(GroupFailure::Value(value_error(
            "The function must take one or two arguments",
        ))),
        Err(LiteralValue::Error(error)) => Err(GroupFailure::Value(error)),
        Err(_) => Err(GroupFailure::Value(value_error("Expected a function"))),
    }
}

struct Aggregate<'h, 'a, 'b> {
    arg: &'h ArgumentHandle<'a, 'b>,
    callable: Arc<dyn CustomCallable>,
    values: &'h Grid,
}

impl Aggregate<'_, '_, '_> {
    fn column(&self, rows: &[usize], col: usize) -> LiteralValue {
        LiteralValue::Array(
            rows.iter()
                .map(|&r| vec![cell(self.values, r, col)])
                .collect(),
        )
    }

    fn apply(&self, rows: &[usize], relative: &[usize], col: usize) -> LiteralValue {
        let result = if self.callable.accepts(1) {
            invoke(self.arg, &self.callable, &[self.column(rows, col)])
        } else {
            invoke(
                self.arg,
                &self.callable,
                &[self.column(rows, col), self.column(relative, col)],
            )
        };
        element_value(result).unwrap_or_else(|| {
            LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Calc)
                    .with_message("Nested arrays are not supported"),
            )
        })
    }

    fn column_cancellable(
        &self,
        rows: &[usize],
        col: usize,
        cancel: Option<&CancelToken>,
    ) -> Result<LiteralValue, ExcelError> {
        cancellation_checkpoint(cancel)?;
        let values = rows
            .iter()
            .map(|&r| {
                cancellation_checkpoint(cancel)?;
                Ok(vec![cell(self.values, r, col)])
            })
            .collect::<Result<Vec<_>, ExcelError>>()?;
        cancellation_checkpoint(cancel)?;
        Ok(LiteralValue::Array(values))
    }

    fn apply_cancellable(
        &self,
        rows: &[usize],
        relative: &[usize],
        col: usize,
        cancel: Option<&CancelToken>,
    ) -> Result<LiteralValue, ExcelError> {
        let result = if self.callable.accepts(1) {
            invoke_cancellable(
                self.arg,
                &self.callable,
                &[self.column_cancellable(rows, col, cancel)?],
            )?
        } else {
            invoke_cancellable(
                self.arg,
                &self.callable,
                &[
                    self.column_cancellable(rows, col, cancel)?,
                    self.column_cancellable(relative, col, cancel)?,
                ],
            )?
        };
        Ok(element_value(result).unwrap_or_else(|| {
            LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Calc)
                    .with_message("Nested arrays are not supported"),
            )
        }))
    }
}

fn rows_of(
    keys: &[usize],
    rows_by_key: &[Vec<usize>],
    cancel: Option<&CancelToken>,
) -> Result<Vec<usize>, GroupFailure> {
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let mut rows = Vec::new();
    for &key in keys {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        for &row in &rows_by_key[key] {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            rows.push(row);
        }
    }
    rows.sort_unstable();
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    Ok(rows)
}

fn validate_spec(spec: &Option<Vec<i64>>, limit: usize) -> Result<(), ExcelError> {
    if let Some(spec) = spec
        && spec
            .iter()
            .any(|&s| s == 0 || s.unsigned_abs() as usize > limit)
    {
        return Err(value_error("sort_order is out of range"));
    }
    Ok(())
}

/// `GROUPBY(row_fields, values, function, [field_headers], [total_depth],
/// [sort_order], [filter_array], [field_relationship])`
fn groupby(
    args: &[ArgumentHandle<'_, '_>],
    cancel: Option<&CancelToken>,
) -> Result<Grid, GroupFailure> {
    let fields = grid_arg(&args[0], cancel)?;
    let values = grid_arg(&args[1], cancel)?;
    if fields.len() != values.len() {
        return Err(value_error("row_fields and values must have the same rows").into());
    }
    let aggregate = Aggregate {
        arg: &args[2],
        callable: aggregator(&args[2])?,
        values: &values,
    };
    let (key_cols, value_cols) = (width(&fields), width(&values));
    let (has_headers, show_headers) = header_mode(optional_int(args, 3)?, &values)?;
    let table = optional_int(args, 7)? == Some(1);
    let depth = optional_int(args, 4)?.unwrap_or(if table { 1 } else { key_cols as i64 });
    let depth = if table { depth.clamp(-1, 1) } else { depth };
    let spec = sort_spec(args, 5)?;
    validate_spec(&spec, key_cols + value_cols)?;
    let rows = kept_rows(
        fields.len(),
        usize::from(has_headers),
        optional_arg(args, 6)?,
        cancel,
    )?;

    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let keys: Vec<Vec<LiteralValue>> = rows
        .iter()
        .map(|&r| {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            (0..key_cols)
                .map(|c| {
                    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                    Ok(cell(&fields, r, c))
                })
                .collect::<Result<Vec<_>, GroupFailure>>()
        })
        .collect::<Result<Vec<_>, GroupFailure>>()?;
    let (distinct, key_of) = distinct_keys(&keys, cancel)?;
    let mut rows_by_key = vec![Vec::new(); distinct.len()];
    for (i, &r) in rows.iter().enumerate() {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        rows_by_key[key_of[i]].push(r);
    }
    let detail: Vec<Vec<LiteralValue>> = (0..distinct.len())
        .map(|k| {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            (0..value_cols)
                .map(|c| {
                    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                    aggregate
                        .apply_cancellable(&rows_by_key[k], &rows, c, cancel)
                        .map_err(GroupFailure::Native)
                })
                .collect::<Result<Vec<_>, GroupFailure>>()
        })
        .collect::<Result<Vec<_>, GroupFailure>>()?;
    let order = order_keys(
        &distinct,
        spec.as_deref(),
        &|k, c| detail[k][c].clone(),
        key_cols,
        cancel,
    )?;

    let mut out = Vec::new();
    if show_headers {
        let mut header: Vec<LiteralValue> = (0..key_cols)
            .map(|c| {
                if has_headers {
                    cell(&fields, 0, c)
                } else {
                    LiteralValue::Text(format!("Row Field {}", c + 1))
                }
            })
            .collect();
        header.extend((0..value_cols).map(|c| {
            if has_headers {
                cell(&values, 0, c)
            } else {
                LiteralValue::Text(format!("Value {}", c + 1))
            }
        }));
        out.push(header);
    }
    for entry in axis_entries(&distinct, &order, depth, key_cols, cancel)? {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        let mut line = entry.label;
        if entry.keys.len() == 1 {
            line.extend(detail[entry.keys[0]].iter().cloned());
        } else {
            let members = rows_of(&entry.keys, &rows_by_key, cancel)?;
            for c in 0..value_cols {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                line.push(
                    aggregate
                        .apply_cancellable(&members, &rows, c, cancel)
                        .map_err(GroupFailure::Native)?,
                );
            }
        }
        out.push(line);
    }
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    Ok(out)
}

/// `PIVOTBY(row_fields, col_fields, values, function, [field_headers],
/// [row_total_depth], [row_sort_order], [col_total_depth], [col_sort_order],
/// [filter_array], [relative_to])`
fn pivotby(
    args: &[ArgumentHandle<'_, '_>],
    cancel: Option<&CancelToken>,
) -> Result<Grid, GroupFailure> {
    let row_fields = grid_arg(&args[0], cancel)?;
    let col_fields = grid_arg(&args[1], cancel)?;
    let values = grid_arg(&args[2], cancel)?;
    if row_fields.len() != values.len() || col_fields.len() != values.len() {
        return Err(
            value_error("row_fields, col_fields and values must have the same rows").into(),
        );
    }
    let aggregate = Aggregate {
        arg: &args[3],
        callable: aggregator(&args[3])?,
        values: &values,
    };
    let (row_cols, col_cols, value_cols) = (width(&row_fields), width(&col_fields), width(&values));
    let (has_headers, show_headers) = header_mode(optional_int(args, 4)?, &values)?;
    let row_depth = optional_int(args, 5)?.unwrap_or(row_cols as i64);
    let row_spec = sort_spec(args, 6)?;
    let col_depth = optional_int(args, 7)?.unwrap_or(col_cols as i64);
    let col_spec = sort_spec(args, 8)?;
    validate_spec(&row_spec, row_cols + value_cols)?;
    validate_spec(&col_spec, col_cols + value_cols)?;
    let rows = kept_rows(
        values.len(),
        usize::from(has_headers),
        optional_arg(args, 9)?,
        cancel,
    )?;
    let relative_to = optional_int(args, 10)?.unwrap_or(0);
    if !(0..=4).contains(&relative_to) {
        return Err(value_error("relative_to must be 0-4").into());
    }

    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    let axis = |fields: &Grid, cols: usize| {
        let keys: Vec<Vec<LiteralValue>> = rows
            .iter()
            .map(|&r| {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                (0..cols)
                    .map(|c| {
                        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                        Ok(cell(fields, r, c))
                    })
                    .collect::<Result<Vec<_>, GroupFailure>>()
            })
            .collect::<Result<Vec<_>, GroupFailure>>()?;
        let (distinct, key_of) = distinct_keys(&keys, cancel)?;
        let mut rows_by_key = vec![Vec::new(); distinct.len()];
        for (i, &r) in rows.iter().enumerate() {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            rows_by_key[key_of[i]].push(r);
        }
        Ok::<_, GroupFailure>((distinct, rows_by_key))
    };
    let (row_keys, rows_by_row_key) = axis(&row_fields, row_cols)?;
    let (col_keys, rows_by_col_key) = axis(&col_fields, col_cols)?;
    let row_totals = |k: usize, c: usize| aggregate.apply(&rows_by_row_key[k], &rows, c);
    let col_totals = |k: usize, c: usize| aggregate.apply(&rows_by_col_key[k], &rows, c);
    let row_order = order_keys(
        &row_keys,
        row_spec.as_deref(),
        &row_totals,
        row_cols,
        cancel,
    )?;
    let col_order = order_keys(
        &col_keys,
        col_spec.as_deref(),
        &col_totals,
        col_cols,
        cancel,
    )?;
    let row_entries = axis_entries(&row_keys, &row_order, row_depth, row_cols, cancel)?;
    let col_entries = axis_entries(&col_keys, &col_order, col_depth, col_cols, cancel)?;
    let col_members: Vec<Vec<usize>> = col_entries
        .iter()
        .map(|e| {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            rows_of(&e.keys, &rows_by_col_key, cancel)
        })
        .collect::<Result<Vec<_>, GroupFailure>>()?;

    let mut out = Vec::new();
    for level in 0..col_cols {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        let mut line = vec![blank(); row_cols];
        if show_headers && level + 1 == col_cols && value_cols == 1 {
            for (c, slot) in line.iter_mut().enumerate() {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                *slot = if has_headers {
                    cell(&row_fields, 0, c)
                } else {
                    LiteralValue::Text(format!("Row Field {}", c + 1))
                };
            }
        }
        for entry in &col_entries {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            for v in 0..value_cols {
                line.push(if v == 0 {
                    entry.label[level].clone()
                } else {
                    blank()
                });
            }
        }
        out.push(line);
    }
    if value_cols > 1 {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        let mut line = vec![blank(); row_cols];
        for _ in &col_entries {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            for v in 0..value_cols {
                line.push(if has_headers {
                    cell(&values, 0, v)
                } else {
                    LiteralValue::Text(format!("Value {}", v + 1))
                });
            }
        }
        out.push(line);
    }
    for entry in row_entries {
        cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
        let members = rows_of(&entry.keys, &rows_by_row_key, cancel)?;
        let mut line = entry.label;
        for cols in &col_members {
            cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
            let mut both = Vec::new();
            for &member in &members {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                if cols.binary_search(&member).is_ok() {
                    both.push(member);
                }
            }
            let relative = match relative_to {
                1 | 4 => members.clone(),
                2 => rows.clone(),
                _ => cols.clone(),
            };
            for v in 0..value_cols {
                cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
                line.push(if both.is_empty() {
                    blank()
                } else {
                    aggregate
                        .apply_cancellable(&both, &relative, v, cancel)
                        .map_err(GroupFailure::Native)?
                });
            }
        }
        out.push(line);
    }
    cancellation_checkpoint(cancel).map_err(GroupFailure::Native)?;
    Ok(out)
}

macro_rules! group_fn {
    ($ty:ident, $name:literal, $min:expr, $max:expr, $eval:ident) => {
        #[derive(Debug)]
        pub struct $ty;

        impl Function for $ty {
            fn caps(&self) -> FnCaps {
                FnCaps::PURE | FnCaps::MAY_SPILL
            }
            fn name(&self) -> &'static str {
                $name
            }
            fn min_args(&self) -> usize {
                $min
            }
            fn variadic(&self) -> bool {
                true
            }
            fn arg_schema(&self) -> &'static [crate::args::ArgSchema] {
                static SCHEMA: std::sync::LazyLock<Vec<crate::args::ArgSchema>> =
                    std::sync::LazyLock::new(|| vec![crate::args::ArgSchema::any()]);
                &SCHEMA
            }
            // The function argument is a LAMBDA or a bare function name, which
            // schema validation would reject, so dispatch straight to eval.
            fn dispatch<'a, 'b, 'c>(
                &self,
                args: &'c [ArgumentHandle<'a, 'b>],
                ctx: &dyn FunctionContext<'b>,
            ) -> Result<CalcValue<'b>, ExcelError> {
                self.eval(args, ctx)
            }
            fn eval<'a, 'b, 'c>(
                &self,
                args: &'c [ArgumentHandle<'a, 'b>],
                ctx: &dyn FunctionContext<'b>,
            ) -> Result<CalcValue<'b>, ExcelError> {
                let result = if args.len() < $min || args.len() > $max {
                    Err(GroupFailure::Value(value_error(concat!(
                        "Wrong number of arguments to ",
                        $name
                    ))))
                } else {
                    let cancel = ctx.cancellation_token();
                    $eval(args, cancel.as_ref())
                };
                Ok(match result {
                    Ok(rows) => collapse_if_scalar(rows, ctx.date_system()),
                    Err(GroupFailure::Value(error)) => {
                        CalcValue::Scalar(LiteralValue::Error(error))
                    }
                    Err(GroupFailure::Native(error)) => return Err(error),
                })
            }
        }
    };
}

group_fn!(GroupByFn, "GROUPBY", 3, 8, groupby);
group_fn!(PivotByFn, "PIVOTBY", 4, 11, pivotby);

pub fn register_builtins() {
    use crate::function_registry::register_builtin;
    register_builtin(Arc::new(GroupByFn));
    register_builtin(Arc::new(PivotByFn));
}
