//! FORECAST.ETS and its CONFINT, SEASONALITY and STAT companions: additive
//! (AAA) exponential smoothing, computed where Excel's result does not depend
//! on its optimizer.

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
    // Excel forecasts from two points (FORECAST.ETS(3,{10,20},{1,2}) is 30)
    // and from a timeline whose steps are not one size
    // (FORECAST.ETS(7,{10,...,60},{1,...,5,6.5}) is 62.52); neither, nor a
    // single point, is computed here.
    if grouped.len() < 2 {
        return Err(not_computed());
    }
    let step = grouped
        .windows(2)
        .map(|w| w[1].0 - w[0].0)
        .fold(f64::INFINITY, f64::min);
    if step <= 0.0 || !step.is_finite() {
        return Err(not_computed());
    }
    let start = grouped[0].0;
    let mut slots: Vec<Option<f64>> = Vec::new();
    for (t, vs) in &grouped {
        let pos = (t - start) / step;
        let idx = pos.round();
        if (pos - idx).abs() > 1e-9 * idx.abs().max(1.0) {
            return Err(not_computed());
        }
        let idx = idx as usize;
        if idx >= 1_000_000 {
            return Err(not_computed());
        }
        slots.resize(idx + 1, None);
        slots[idx] = (!vs.is_empty()).then(|| aggregate(vs, aggregation));
    }
    let known: Vec<usize> = (0..slots.len()).filter(|&i| slots[i].is_some()).collect();
    if known.len() < 2 {
        return Err(not_computed());
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

/// The exact additive structure of a series, when it has one: `y[t]` is a
/// line plus a season of `period` steps (`period` 0: a line alone). Excel's
/// AAA exponential smoothing fits such a series without error whatever
/// smoothing parameters its optimizer picks, so its forecasts, a zero
/// confidence interval and zero error statistics follow from the series
/// alone. For any other series they depend on Excel's optimizer and initial
/// state, which this engine does not reproduce: those calls are not computed
/// here (`#N/IMPL!`) rather than given a value Excel would not give
/// (Excel for Windows 16.0.20430: `FORECAST.ETS(25,...)` of the first 24
/// AirPassengers values is 138.69607025814878, a grid-search Holt-Winters fit
/// gives 142.9).
struct Exact {
    period: usize,
    /// The trend per step.
    slope: f64,
}

fn close(a: f64, b: f64, scale: f64) -> bool {
    (a - b).abs() <= 1e-12 * scale
}

/// Whether `y` is a line plus a season of `period` steps (`period` 0: a line),
/// to 1E-12 of its largest value; with `strict`, every step exactly the same
/// in binary (Excel's own fit of 0.1, 0.2, ..., 0.6 leaves residues of
/// 1E-16, which its error statistics and confidence interval show).
fn exact_fit(y: &[f64], period: usize, strict: bool) -> Option<Exact> {
    let scale = y.iter().fold(1.0f64, |m, v| m.max(v.abs()));
    let same = |a: f64, b: f64| if strict { a == b } else { close(a, b, scale) };
    let lag = period.max(1);
    if y.len() < lag + 1 || (period > 0 && y.len() < 2 * period) {
        return None;
    }
    let cycle = y[lag] - y[0];
    let exact = (0..y.len() - lag).all(|t| same(y[t + lag] - y[t], cycle))
        && (strict || period > 0 || (0..y.len()).all(|t| same(y[t], y[0] + cycle * t as f64)));
    exact.then(|| Exact {
        period,
        slope: cycle / lag as f64,
    })
}

/// The season Excel detects (seasonality 1, the default), known only for an
/// exact line: none. For a series with a season Excel's choice is not
/// reproduced (a 4-step season plus a trend is detected as 2).
fn detect_exact(y: &[f64], strict: bool) -> Option<Exact> {
    exact_fit(y, 0, strict)
}

fn not_computed() -> ExcelError {
    ExcelError::new(ExcelErrorKind::NImpl).with_message(
        "FORECAST.ETS of a series that is not exactly a line plus a season depends on \
         Excel's optimizer, which is not reproduced",
    )
}

/// The arguments the ETS functions share after the target (or before the
/// statistic): values, timeline, seasonality, data completion, aggregation.
struct EtsInput {
    series: Series,
    exact: Exact,
}

fn ets_input(args: &[ArgumentHandle<'_, '_>], first: usize) -> Result<EtsInput, ExcelError> {
    ets_input_with(args, first, first + 2, false)
}

impl EtsInput {
    /// The forecast `steps` steps after the last point: along the line, or
    /// from the same season of the last full cycle (whole steps only:
    /// Excel's value between the points of a season is not reproduced).
    fn forecast(&self, steps: f64) -> Result<f64, ExcelError> {
        let y = &self.series.values;
        let n = y.len();
        if self.exact.period == 0 {
            return Ok(y[n - 1] + self.exact.slope * steps);
        }
        if (steps - steps.round()).abs() > 1e-9 {
            return Err(not_computed());
        }
        let m = self.exact.period;
        let target = n - 1 + steps.round() as usize;
        let base = n - m + (target - (n - m)) % m;
        Ok(y[base] + self.exact.slope * (target - base) as f64)
    }

    /// Steps from the end of the timeline to `target` (Excel: 7.5 on a
    /// timeline 1 to 6 is 1.5 steps); `#NUM!` before the end.
    fn steps_to(&self, target: f64) -> Result<f64, ExcelError> {
        let y = &self.series.values;
        let last = self.series.start + self.series.step * (y.len() - 1) as f64;
        if target < last {
            return Err(num_error());
        }
        let h = (target - last) / self.series.step;
        if h > 1e7 {
            return Err(not_computed());
        }
        Ok(h)
    }
}

fn ets_schema() -> &'static [ArgSchema] {
    static SCHEMA: std::sync::LazyLock<Vec<ArgSchema>> =
        std::sync::LazyLock::new(|| vec![ArgSchema::any()]);
    &SCHEMA
}

fn number_result<'b>(result: Result<f64, ExcelError>) -> Result<CalcValue<'b>, ExcelError> {
    Ok(CalcValue::Scalar(match result {
        Ok(v) => LiteralValue::Number(v),
        Err(e) => LiteralValue::Error(e),
    }))
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
/// - Computed only where Excel's result does not depend on its optimizer: a
///   series that is exactly a line (any seasonality setting but 2 and up), or
///   a line plus a season of the length given (at least two cycles), on an
///   evenly stepped timeline of two points or more. Elsewhere `#N/IMPL!`.
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
        ets_schema()
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
        number_result((|| {
            if !(3..=6).contains(&args.len()) {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let target = number(&args[0].value()?.into_literal())?;
            let input = ets_input(args, 1)?;
            input.forecast(input.steps_to(target)?)
        })())
    }
}

/// Returns the confidence interval of a FORECAST.ETS forecast.
///
/// `FORECAST.ETS.CONFINT(target_date, values, timeline, [confidence_level],
/// [seasonality], [data_completion], [aggregation])`; confidence_level is
/// 0.95 by default and must lie strictly between 0 and 1.
///
/// # Remarks
/// - Computed only for a series whose every step is exactly the same (with
///   the season given), which Excel fits without error: the interval is 0.
///   Elsewhere `#N/IMPL!`.
///
/// ```yaml,sandbox
/// title: "An exact trend has no uncertainty"
/// formula: "=FORECAST.ETS.CONFINT(7,{10,20,30,40,50,60},{1,2,3,4,5,6})"
/// expected: 0
/// ```
#[derive(Debug)]
pub struct ForecastEtsConfintFn;

impl Function for ForecastEtsConfintFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "FORECAST.ETS.CONFINT"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        ets_schema()
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
        number_result((|| {
            if !(3..=7).contains(&args.len()) {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let target = number(&args[0].value()?.into_literal())?;
            let level = match args.get(3) {
                Some(arg) if !arg.is_omitted() => number(&arg.value()?.into_literal())?,
                _ => 0.95,
            };
            if !(level > 0.0 && level < 1.0) {
                return Err(num_error());
            }
            // The values and timeline follow the target; the options follow
            // the confidence level.
            let input = ets_input_with(args, 1, 4, true)?;
            input.steps_to(target)?;
            Ok(0.0)
        })())
    }
}

/// [`ets_input`] when the options start at `options` rather than right after
/// the timeline.
fn ets_input_with(
    args: &[ArgumentHandle<'_, '_>],
    first: usize,
    options: usize,
    strict: bool,
) -> Result<EtsInput, ExcelError> {
    let seasonality = optional_int(args, options, 1)?;
    let completion = optional_int(args, options + 1, 1)?;
    let aggregation = optional_int(args, options + 2, 1)?;
    if !(0..=MAX_SEASONALITY as i64).contains(&seasonality)
        || !(0..=1).contains(&completion)
        || !(1..=7).contains(&aggregation)
    {
        return Err(num_error());
    }
    let series = build_series(
        &cells(&args[first])?,
        &cells(&args[first + 1])?,
        completion,
        aggregation,
    )?;
    let exact = match seasonality {
        1 => detect_exact(&series.values, strict),
        m => exact_fit(&series.values, m as usize, strict),
    }
    .ok_or_else(not_computed)?;
    Ok(EtsInput { series, exact })
}

/// Returns the season length FORECAST.ETS detects for a series.
///
/// `FORECAST.ETS.SEASONALITY(values, timeline, [data_completion],
/// [aggregation])`.
///
/// # Remarks
/// - Computed only for a series that is exactly a line (0); Excel's season
///   detection is not reproduced, so any other series is `#N/IMPL!`.
///
/// ```yaml,sandbox
/// title: "A line has no season"
/// formula: "=FORECAST.ETS.SEASONALITY({10,20,30,40,50,60},{1,2,3,4,5,6})"
/// expected: 0
/// ```
#[derive(Debug)]
pub struct ForecastEtsSeasonalityFn;

impl Function for ForecastEtsSeasonalityFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "FORECAST.ETS.SEASONALITY"
    }
    fn min_args(&self) -> usize {
        2
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        ets_schema()
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
        number_result((|| {
            if !(2..=4).contains(&args.len()) {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let completion = optional_int(args, 2, 1)?;
            let aggregation = optional_int(args, 3, 1)?;
            if !(0..=1).contains(&completion) || !(1..=7).contains(&aggregation) {
                return Err(num_error());
            }
            let series = build_series(
                &cells(&args[0])?,
                &cells(&args[1])?,
                completion,
                aggregation,
            )?;
            let exact = detect_exact(&series.values, false).ok_or_else(not_computed)?;
            Ok(exact.period as f64)
        })())
    }
}

/// Returns a statistic of the FORECAST.ETS model of a series.
///
/// `FORECAST.ETS.STAT(values, timeline, statistic_type, [seasonality],
/// [data_completion], [aggregation])`: 1 alpha, 2 beta, 3 gamma, 4 MASE,
/// 5 SMAPE, 6 MAE, 7 RMSE, 8 the step of the timeline.
///
/// # Remarks
/// - A statistic_type outside 1 to 8 is `#NUM!`.
/// - The error statistics (4 to 7) are 0 for a series whose every step is
///   exactly the same, and the step (8) is the timeline's for a series that
///   is a line (or a line plus the season given); the smoothing parameters
///   (1 to 3) are those Excel's optimizer picks, and every statistic of any
///   other series depends on it: `#N/IMPL!`.
///
/// ```yaml,sandbox
/// title: "Step of the timeline"
/// formula: "=FORECAST.ETS.STAT({10,20,30,40,50,60},{1,2,3,4,5,6},8)"
/// expected: 1
/// ```
#[derive(Debug)]
pub struct ForecastEtsStatFn;

impl Function for ForecastEtsStatFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "FORECAST.ETS.STAT"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        ets_schema()
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
        number_result((|| {
            if !(3..=6).contains(&args.len()) {
                return Err(ExcelError::new(ExcelErrorKind::Value));
            }
            let statistic = number(&args[2].value()?.into_literal())?.trunc();
            if !(1.0..=8.0).contains(&statistic) {
                return Err(num_error());
            }
            // The error statistics need the strict fit; the step does not.
            let strict = (4.0..=7.0).contains(&statistic);
            let input = ets_input_with(args, 0, 3, strict)?;
            match statistic as u8 {
                4..=7 => Ok(0.0),
                8 => Ok(input.series.step),
                _ => Err(not_computed()),
            }
        })())
    }
}

pub(crate) fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(ForecastEtsFn));
    crate::function_registry::register_builtin(Arc::new(ForecastEtsConfintFn));
    crate::function_registry::register_builtin(Arc::new(ForecastEtsSeasonalityFn));
    crate::function_registry::register_builtin(Arc::new(ForecastEtsStatFn));
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
            eval(&format!("=FORECAST.ETS(13,{values},{timeline},3)")),
            1.0,
            1e-6,
        );
        // Excel's season detection is not reproduced for a seasonal series.
        match eval(&format!("=FORECAST.ETS(13,{values},{timeline})")) {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::NImpl),
            other => panic!("expected #N/IMPL!, got {other:?}"),
        }
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
        ] {
            match eval(f) {
                LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Num, "{f}"),
                other => panic!("{f}: expected #NUM!, got {other:?}"),
            }
        }
        // Excel forecasts from an unevenly stepped timeline; that is not
        // reproduced.
        match eval("=FORECAST.ETS(7,{10,20,30},{1,2,3.5})") {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::NImpl),
            other => panic!("expected #N/IMPL!, got {other:?}"),
        }
    }
}
