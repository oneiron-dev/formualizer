//! FORECAST.ETS: additive (AAA) exponential-smoothing forecast.

use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

/// The longest season FORECAST.ETS accepts.
const MAX_SEASONALITY: usize = 8760;

fn num_error() -> ExcelError {
    ExcelError::new(ExcelErrorKind::Num)
}

fn cells(arg: &ArgumentHandle<'_, '_>) -> Result<Vec<LiteralValue>, ExcelError> {
    let view = arg.range_view_or_scalar()?;
    let mut out = Vec::new();
    view.for_each_row(&mut |row| {
        out.extend(row.iter().cloned());
        Ok(())
    })?;
    Ok(out)
}

fn number(value: &LiteralValue) -> Result<f64, ExcelError> {
    match value {
        LiteralValue::Error(e) => Err(e.clone()),
        LiteralValue::Number(n) => Ok(*n),
        LiteralValue::Int(i) => Ok(*i as f64),
        other => other
            .as_serial_number()
            .ok_or_else(|| ExcelError::new(ExcelErrorKind::Value)),
    }
}

fn optional_int(
    args: &[ArgumentHandle<'_, '_>],
    index: usize,
    default: i64,
) -> Result<i64, ExcelError> {
    match args.get(index) {
        Some(arg) if !arg.is_omitted() => {
            let v = arg.value()?.into_literal();
            Ok(number(&v)?.trunc() as i64)
        }
        _ => Ok(default),
    }
}

/// Duplicate timeline points combine by the aggregation argument.
fn aggregate(values: &[f64], how: i64) -> f64 {
    let n = values.len() as f64;
    match how {
        2 | 3 => n,
        4 => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        5 => {
            let mut v = values.to_vec();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mid = v.len() / 2;
            if v.len() % 2 == 0 {
                (v[mid - 1] + v[mid]) / 2.0
            } else {
                v[mid]
            }
        }
        6 => values.iter().copied().fold(f64::INFINITY, f64::min),
        7 => values.iter().sum(),
        _ => values.iter().sum::<f64>() / n,
    }
}

/// The series on an evenly stepped timeline: points sorted, duplicates
/// aggregated, missing steps completed by interpolation (or zero).
struct Series {
    values: Vec<f64>,
    start: f64,
    step: f64,
}

fn build_series(
    values: &[LiteralValue],
    timeline: &[LiteralValue],
    completion: i64,
    aggregation: i64,
) -> Result<Series, ExcelError> {
    if values.len() != timeline.len() {
        return Err(ExcelError::new(ExcelErrorKind::Na));
    }
    let mut points: Vec<(f64, Option<f64>)> = Vec::with_capacity(values.len());
    for (v, t) in values.iter().zip(timeline) {
        let t = number(t)?;
        let v = match v {
            LiteralValue::Empty => None,
            other => Some(number(other)?),
        };
        points.push((t, v));
    }
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut grouped: Vec<(f64, Vec<f64>)> = Vec::new();
    for (t, v) in points {
        match grouped.last_mut() {
            Some((last, vs)) if *last == t => vs.extend(v),
            _ => grouped.push((t, v.into_iter().collect())),
        }
    }
    if grouped.len() < 3 {
        return Err(num_error());
    }
    let step = grouped
        .windows(2)
        .map(|w| w[1].0 - w[0].0)
        .fold(f64::INFINITY, f64::min);
    if step <= 0.0 || !step.is_finite() {
        return Err(num_error());
    }
    let start = grouped[0].0;
    let mut slots: Vec<Option<f64>> = Vec::new();
    for (t, vs) in &grouped {
        let pos = (t - start) / step;
        let idx = pos.round();
        if (pos - idx).abs() > 1e-9 * idx.abs().max(1.0) {
            return Err(num_error());
        }
        let idx = idx as usize;
        if idx >= 1_000_000 {
            return Err(num_error());
        }
        slots.resize(idx + 1, None);
        slots[idx] = (!vs.is_empty()).then(|| aggregate(vs, aggregation));
    }
    let known: Vec<usize> = (0..slots.len()).filter(|&i| slots[i].is_some()).collect();
    if known.len() < 2 {
        return Err(num_error());
    }
    let mut out = Vec::with_capacity(slots.len());
    for (i, slot) in slots.iter().enumerate() {
        out.push(match slot {
            Some(v) => *v,
            None if completion == 0 => 0.0,
            None => {
                // Linear interpolation between the known neighbours.
                let before = known.iter().rev().find(|&&k| k < i);
                let after = known.iter().find(|&&k| k > i);
                match (before, after) {
                    (Some(&a), Some(&b)) => {
                        let (va, vb) = (slots[a].unwrap(), slots[b].unwrap());
                        va + (vb - va) * (i - a) as f64 / (b - a) as f64
                    }
                    (Some(&a), None) => slots[a].unwrap(),
                    (None, Some(&b)) => slots[b].unwrap(),
                    (None, None) => 0.0,
                }
            }
        });
    }
    Ok(Series {
        values: out,
        start,
        step,
    })
}

/// A fitted additive Holt-Winters state after the last observation.
struct Fit {
    sse: f64,
    level: f64,
    trend: f64,
    /// Seasonal terms indexed by absolute period position modulo the season.
    season: Vec<f64>,
}

fn run(y: &[f64], m: usize, alpha: f64, beta: f64, gamma: f64) -> Fit {
    let (mut level, mut trend, mut season, first) = if m >= 2 {
        let mean = |r: std::ops::Range<usize>| y[r.clone()].iter().sum::<f64>() / r.len() as f64;
        let l0 = mean(0..m);
        let b0 = (mean(m..2 * m) - l0) / m as f64;
        let s: Vec<f64> = (0..m)
            .map(|i| y[i] - (l0 + (i as f64 - (m as f64 - 1.0) / 2.0) * b0))
            .collect();
        (l0 + (m as f64 - 1.0) / 2.0 * b0, b0, s, m)
    } else {
        (y[0], y[1] - y[0], Vec::new(), 1)
    };
    let mut sse = 0.0;
    for (t, &obs) in y.iter().enumerate().skip(first) {
        let s = if m >= 2 { season[t % m] } else { 0.0 };
        let forecast = level + trend + s;
        let e = obs - forecast;
        sse += e * e;
        let new_level = alpha * (obs - s) + (1.0 - alpha) * (level + trend);
        trend = beta * (new_level - level) + (1.0 - beta) * trend;
        if m >= 2 {
            season[t % m] = gamma * (obs - new_level) + (1.0 - gamma) * s;
        }
        level = new_level;
    }
    Fit {
        sse,
        level,
        trend,
        season,
    }
}

/// Smoothing parameters minimising the one-step squared error: a coarse
/// grid, then a finer grid around the best point.
fn fit(y: &[f64], m: usize) -> Fit {
    let grid = |centre: (f64, f64, f64), half: f64, steps: usize| {
        let axis = |c: f64| -> Vec<f64> {
            (0..=steps)
                .map(|i| (c - half + 2.0 * half * i as f64 / steps as f64).clamp(0.0, 1.0))
                .collect()
        };
        let gammas = if m >= 2 { axis(centre.2) } else { vec![0.0] };
        let mut best = (f64::INFINITY, centre);
        for &a in &axis(centre.0) {
            for &b in &axis(centre.1) {
                for &g in &gammas {
                    let sse = run(y, m, a, b, g).sse;
                    if sse < best.0 - 1e-12 {
                        best = (sse, (a, b, g));
                    }
                }
            }
        }
        best.1
    };
    let coarse = grid((0.5, 0.5, 0.5), 0.5, 10);
    let fine = grid(coarse, 0.05, 10);
    run(y, m, fine.0, fine.1, fine.2)
}

/// Seasonality detection: candidate periods are the autocorrelation peaks
/// of the detrended series; the model with the best information criterion
/// (non-seasonal included) wins.
fn detect_seasonality(y: &[f64]) -> usize {
    let n = y.len();
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let mx = xs.iter().sum::<f64>() / n as f64;
    let my = y.iter().sum::<f64>() / n as f64;
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    let slope = xs
        .iter()
        .zip(y)
        .map(|(x, v)| (x - mx) * (v - my))
        .sum::<f64>()
        / sxx;
    let r: Vec<f64> = (0..n)
        .map(|i| y[i] - (my + slope * (i as f64 - mx)))
        .collect();
    let var: f64 = r.iter().map(|v| v * v).sum();
    if var <= 1e-12 * y.iter().map(|v| v * v).sum::<f64>().max(1e-300) {
        return 0;
    }
    let acf = |k: usize| (0..n - k).map(|i| r[i] * r[i + k]).sum::<f64>() / var;
    let max_lag = (n / 2).min(MAX_SEASONALITY);
    let values: Vec<f64> = (0..=max_lag)
        .map(|k| if k < n { acf(k) } else { 0.0 })
        .collect();
    let mut peaks: Vec<(usize, f64)> = (2..=max_lag)
        .filter(|&k| {
            values[k] > 0.0
                && values[k] >= values[k - 1]
                && (k + 1 > max_lag || values[k] >= values[k + 1])
        })
        .map(|k| (k, values[k]))
        .collect();
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    peaks.truncate(3);
    let score = |m: usize| {
        let f = fit(y, m);
        let first = if m >= 2 { m } else { 1 };
        let count = (n - first) as f64;
        let params = if m >= 2 { 3.0 + m as f64 } else { 2.0 };
        count * (f.sse / count).max(1e-300).ln() + 2.0 * params
    };
    let mut best = (score(0), 0);
    for (m, _) in peaks {
        if n >= 2 * m + 1 {
            let s = score(m);
            if s < best.0 {
                best = (s, m);
            }
        }
    }
    best.1
}

/// Predicts a future value with additive (AAA) exponential smoothing.
///
/// # Remarks
/// - `seasonality`: omitted or 1 detects the season length, 0 turns
///   seasonality off, 2 to 8760 sets it.
/// - The timeline must advance in a constant step; duplicates combine with
///   `aggregation` (default average) and missing points are interpolated
///   (`data_completion` 1, the default) or zero (0).
/// - A target before the end of the timeline is `#NUM!`.
///
/// ```yaml,sandbox
/// title: "Continue a linear trend"
/// formula: "=FORECAST.ETS(7,{10,20,30,40,50,60},{1,2,3,4,5,6})"
/// expected: 70
/// ```
#[derive(Debug)]
pub struct ForecastEtsFn;

impl Function for ForecastEtsFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "FORECAST.ETS"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
            std::sync::LazyLock::new(|| vec![ArgSchema::any()]);
        &SCHEMA
    }
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
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let result = (|| -> Result<f64, ExcelError> {
            if args.len() < 3 || args.len() > 6 {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let target = number(&args[0].value()?.into_literal())?;
            let seasonality = optional_int(args, 3, 1)?;
            let completion = optional_int(args, 4, 1)?;
            let aggregation = optional_int(args, 5, 1)?;
            if !(0..=MAX_SEASONALITY as i64).contains(&seasonality)
                || !(0..=1).contains(&completion)
                || !(1..=7).contains(&aggregation)
            {
                return Err(num_error());
            }
            let series = build_series(
                &cells(&args[1])?,
                &cells(&args[2])?,
                completion,
                aggregation,
            )?;
            let y = &series.values;
            let last = series.start + series.step * (y.len() - 1) as f64;
            if target < last {
                return Err(num_error());
            }
            let m = match seasonality {
                1 => detect_seasonality(y),
                m => m as usize,
            };
            let m = if m >= 2 && y.len() >= 2 * m { m } else { 0 };
            let state = fit(y, m);
            let h = (target - last) / series.step;
            let season = if m >= 2 {
                let at = (y.len() - 1) as f64 + h.round();
                state.season[(at as usize) % m]
            } else {
                0.0
            };
            Ok(state.level + h * state.trend + season)
        })();
        Ok(CalcValue::Scalar(match result {
            Ok(v) => LiteralValue::Number(v),
            Err(e) => LiteralValue::Error(e),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use formualizer_parse::parser::parse;
    use std::sync::Arc;

    fn eval(formula: &str) -> LiteralValue {
        let wb = TestWorkbook::new().with_function(Arc::new(ForecastEtsFn));
        let interp = wb.interpreter();
        interp
            .evaluate_ast(&parse(formula).unwrap())
            .unwrap()
            .into_literal()
    }

    fn close(v: LiteralValue, want: f64, tol: f64) {
        match v {
            LiteralValue::Number(n) => assert!((n - want).abs() <= tol, "{n} != {want}"),
            other => panic!("expected {want}, got {other:?}"),
        }
    }

    #[test]
    fn continues_a_linear_trend() {
        close(
            eval("=FORECAST.ETS(7,{10,20,30,40,50,60},{1,2,3,4,5,6})"),
            70.0,
            1e-9,
        );
        close(
            eval("=FORECAST.ETS(9,{10,20,30,40,50,60},{1,2,3,4,5,6})"),
            90.0,
            1e-9,
        );
    }

    #[test]
    fn unsorted_timelines_and_gaps_are_accepted() {
        close(
            eval("=FORECAST.ETS(7,{30,10,20,60,50},{3,1,2,6,5})"),
            70.0,
            1e-9,
        );
    }

    #[test]
    fn repeats_a_clean_season() {
        let values = "{1,5,9,1,5,9,1,5,9,1,5,9}";
        let timeline = "{1,2,3,4,5,6,7,8,9,10,11,12}";
        close(
            eval(&format!("=FORECAST.ETS(13,{values},{timeline})")),
            1.0,
            1e-6,
        );
        close(
            eval(&format!("=FORECAST.ETS(14,{values},{timeline},3)")),
            5.0,
            1e-6,
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for f in [
            "=FORECAST.ETS(2,{10,20,30},{1,2,3})",
            "=FORECAST.ETS(7,{10,20,30},{1,2,3},-1)",
            "=FORECAST.ETS(7,{10,20,30},{1,2,3.5})",
        ] {
            match eval(f) {
                LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Num, "{f}"),
                other => panic!("{f}: expected #NUM!, got {other:?}"),
            }
        }
    }
}
