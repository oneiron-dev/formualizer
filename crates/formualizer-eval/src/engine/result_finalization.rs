use formualizer_common::LiteralValue;

/// Finalize a formula result immediately before it is published to the grid.
///
/// Excel exposes a blank-cell passthrough as numeric zero once it becomes a
/// formula cell's result. `Number(0.0)` matches the evaluator's existing
/// coercion results; stored blank cells remain `Empty` because only formula
/// publication calls this function. Excel has no negative zero and no
/// denormalized numbers, so a `-0` from a function (`ROUND(-0.4,0)`) or an
/// underflowed result is published as `0`.
pub(crate) fn finalize_formula_result(value: LiteralValue) -> LiteralValue {
    match value {
        LiteralValue::Empty => LiteralValue::Number(0.0),
        LiteralValue::Number(n) => LiteralValue::Number(crate::coercion::underflow_to_zero(n)),
        LiteralValue::Array(rows) => LiteralValue::Array(
            rows.into_iter()
                .map(|row| row.into_iter().map(finalize_formula_result).collect())
                .collect(),
        ),
        other => other,
    }
}
