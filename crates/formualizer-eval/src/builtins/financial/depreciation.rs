//! Depreciation functions: SLN, SYD, DB, DDB, VDB, AMORLINC, AMORDEGRC

use super::coupon::{Basis, date, logical, number_schema, number_value, numbers};
use crate::args::ArgSchema;
use crate::coercion::to_serial_strict;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use chrono::{Datelike, NaiveDate};
use formualizer_common::ExcelErrorKind;
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_macros::func_caps;

/// Numeric coercion for depreciation arguments. Shares the crate-central
/// value -> number policy with the rest of the financial builtins so a date
/// cell resolves to its serial instead of `#VALUE!`.
///
/// These are number parameters, so text converts as VALUE() converts it:
/// numeric text, and date or time text in the workbook's date system
/// (`"12:00"` is 0.5), as the argument validation already reads it.
fn coerce_num(arg: &ArgumentHandle) -> Result<f64, ExcelError> {
    let v = arg.value()?.into_literal();
    match v {
        LiteralValue::Text(_) => crate::coercion::to_serial_lenient_in_year(
            &v,
            arg.date_system(),
            Some(arg.current_year()),
        ),
        other => to_serial_strict(&other, arg.date_system()),
    }
}

/// Returns straight-line depreciation for a single period.
///
/// `SLN` spreads the depreciable amount (`cost - salvage`) evenly across `life` periods.
///
/// # Remarks
/// - Formula: `(cost - salvage) / life`.
/// - `life` must be non-zero; `life = 0` returns `#DIV/0!`.
/// - This function returns the algebraic result: if `salvage > cost`, depreciation is negative.
/// - Inputs are interpreted as scalar numeric values in matching currency/period units.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Straight-line yearly depreciation"
/// formula: "=SLN(10000, 1000, 9)"
/// expected: 1000
/// ```
///
/// ```yaml,sandbox
/// title: "Negative depreciation when salvage exceeds cost"
/// formula: "=SLN(1000, 1200, 2)"
/// expected: -100
/// ```
/// ```yaml,docs
/// related:
///   - SYD
///   - DB
///   - DDB
/// faq:
///   - q: "Can `SLN` return a negative value?"
///     a: "Yes. If `salvage > cost`, `(cost - salvage) / life` is negative."
///   - q: "What happens when `life` is zero?"
///     a: "`SLN` returns `#DIV/0!`."
/// ```
#[derive(Debug)]
pub struct SlnFn;
/// [formualizer-docgen:schema:start]
/// Name: SLN
/// Type: SlnFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: SLN(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for SlnFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "SLN"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static SCHEMA: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            vec![
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
            ]
        });
        &SCHEMA[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let cost = coerce_num(&args[0])?;
        let salvage = coerce_num(&args[1])?;
        let life = coerce_num(&args[2])?;

        if life == 0.0 {
            return Ok(CalcValue::Scalar(
                LiteralValue::Error(ExcelError::new_div()),
            ));
        }

        let depreciation = (cost - salvage) / life;
        Ok(CalcValue::Scalar(LiteralValue::Number(depreciation)))
    }
}

/// Returns sum-of-years'-digits depreciation for a requested period.
///
/// `SYD` applies accelerated depreciation by weighting earlier periods more heavily.
///
/// # Remarks
/// - Formula: `(cost - salvage) * (life - per + 1) / (life * (life + 1) / 2)`.
/// - `life` and `per` must satisfy: `life > 0`, `per > 0`, and `per <= life`; otherwise returns `#NUM!`.
/// - The function uses the provided numeric values directly (no integer-only enforcement).
/// - Result sign follows `(cost - salvage)`: positive for typical depreciation expense, negative if `salvage > cost`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "First SYD period"
/// formula: "=SYD(10000, 1000, 5, 1)"
/// expected: 3000
/// ```
///
/// ```yaml,sandbox
/// title: "Final SYD period"
/// formula: "=SYD(10000, 1000, 5, 5)"
/// expected: 600
/// ```
/// ```yaml,docs
/// related:
///   - SLN
///   - DB
///   - DDB
/// faq:
///   - q: "Does `SYD` require integer `life` and `per`?"
///     a: "No strict integer check is enforced; it uses provided numeric values directly after domain validation."
///   - q: "Which period values are valid?"
///     a: "`per` must satisfy `0 < per <= life`, and `life` must be positive; otherwise `#NUM!` is returned."
/// ```
#[derive(Debug)]
pub struct SydFn;
/// [formualizer-docgen:schema:start]
/// Name: SYD
/// Type: SydFn
/// Min args: 4
/// Max args: 4
/// Variadic: false
/// Signature: SYD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for SydFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "SYD"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static SCHEMA: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            vec![
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
            ]
        });
        &SCHEMA[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let cost = coerce_num(&args[0])?;
        let salvage = coerce_num(&args[1])?;
        let life = coerce_num(&args[2])?;
        let per = coerce_num(&args[3])?;

        if life <= 0.0 || per <= 0.0 || per > life {
            return Ok(CalcValue::Scalar(
                LiteralValue::Error(ExcelError::new_num()),
            ));
        }

        // Sum of years = life * (life + 1) / 2
        let sum_of_years = life * (life + 1.0) / 2.0;

        // SYD = (cost - salvage) * (life - per + 1) / sum_of_years
        let depreciation = (cost - salvage) * (life - per + 1.0) / sum_of_years;

        Ok(CalcValue::Scalar(LiteralValue::Number(depreciation)))
    }
}

/// Returns fixed-declining-balance depreciation for a specified period.
///
/// `DB` computes per-period depreciation using a declining-balance rate and an optional
/// first-year month proration.
///
/// # Remarks
/// - Parameters: `cost`, `salvage`, `life`, `period`, and optional `month` (default `12`).
/// - `month` must be in `1..=12`; `life` and `period` must be positive; invalid values return `#NUM!`.
/// - `life` and `period` are truncated to integers for period checks and iteration.
/// - The declining rate is rounded to three decimals; if `cost <= 0` or `salvage <= 0`, this implementation uses a rate of `1.0`.
/// - Returned value is the period depreciation amount (generally positive expense, but sign follows provided inputs).
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "First full-year DB period"
/// formula: "=DB(10000, 1000, 5, 1)"
/// expected: 3690
/// ```
///
/// ```yaml,sandbox
/// title: "Fractional period input is truncated"
/// formula: "=DB(10000, 1000, 5, 2.9)"
/// expected: 2328.39
/// ```
/// ```yaml,docs
/// related:
///   - DDB
///   - SYD
///   - SLN
/// faq:
///   - q: "How is `month` used in `DB`?"
///     a: "`month` prorates the first-year depreciation; if omitted it defaults to `12`."
///   - q: "Why can fractional `period` inputs behave like integers?"
///     a: "`DB` truncates `life` and `period` to integers for iteration and period bounds."
/// ```
#[derive(Debug)]
pub struct DbFn;
/// [formualizer-docgen:schema:start]
/// Name: DB
/// Type: DbFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: DB(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for DbFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "DB"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static SCHEMA: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            vec![
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
            ]
        });
        &SCHEMA[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let cost = coerce_num(&args[0])?;
        let salvage = coerce_num(&args[1])?;
        let life = coerce_num(&args[2])?;
        let period = coerce_num(&args[3])?;
        let month = if args.len() > 4 {
            coerce_num(&args[4])?
        } else {
            12.0
        };

        if life <= 0.0 || period <= 0.0 || !(1.0..=12.0).contains(&month) {
            return Ok(CalcValue::Scalar(
                LiteralValue::Error(ExcelError::new_num()),
            ));
        }

        let life_int = life.trunc() as i32;
        let period_int = period.trunc() as i32;
        // The partial year after the last whole one (a huge life saturates
        // rather than overflowing).
        let last_period = life_int.saturating_add(1);

        if period_int < 1 || period_int > last_period {
            return Ok(CalcValue::Scalar(
                LiteralValue::Error(ExcelError::new_num()),
            ));
        }

        // Calculate rate (rounded to 3 decimal places)
        let rate = if cost <= 0.0 || salvage <= 0.0 {
            1.0
        } else {
            let r = 1.0 - (salvage / cost).powf(1.0 / life);
            (r * 1000.0).round() / 1000.0
        };

        let mut total_depreciation = 0.0;
        let value = cost;

        for p in 1..=period_int {
            let depreciation = if p == 1 {
                // First period: prorated
                value * rate * month / 12.0
            } else if p == last_period {
                // Last period (if partial year): remaining value minus salvage
                (value - total_depreciation - salvage)
                    .max(0.0)
                    .min(value - total_depreciation)
            } else {
                (value - total_depreciation) * rate
            };

            if p == period_int {
                return Ok(CalcValue::Scalar(LiteralValue::Number(depreciation)));
            }

            total_depreciation += depreciation;
        }

        Ok(CalcValue::Scalar(LiteralValue::Number(0.0)))
    }
}

/// Returns declining-balance depreciation for a period using a configurable acceleration factor.
///
/// `DDB` defaults to the double-declining method (`factor = 2`) and applies a salvage floor so
/// book value does not fall below `salvage`.
///
/// # Remarks
/// - Parameters: `cost`, `salvage`, `life`, `period`, and optional `factor` (default `2`).
/// - Input constraints: `cost >= 0`, `salvage >= 0`, `life > 0`, `factor > 0`, and `0 < period <= life`; violations return `#NUM!`.
/// - Per-period rate is `factor / life`, at most 1.
/// - `period` may be fractional: the depreciation is the book value after `period - 1` periods (the cost for a period up to 1) times the rate, as Excel computes it.
/// - Result is the period depreciation amount; with valid inputs above it is non-negative.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Default double-declining first period"
/// formula: "=DDB(10000, 1000, 5, 1)"
/// expected: 4000
/// ```
///
/// ```yaml,sandbox
/// title: "Using a custom factor"
/// formula: "=DDB(10000, 1000, 5, 1, 1.5)"
/// expected: 3000
/// ```
///
/// ```yaml,sandbox
/// title: "Fractional period"
/// formula: "=DDB(10000, 1000, 5, 1.9)"
/// expected: 2525.7834699574214
/// ```
/// ```yaml,docs
/// related:
///   - DB
///   - SYD
///   - SLN
/// faq:
///   - q: "What does the optional `factor` control?"
///     a: "It sets the per-period declining rate as `factor / life`; `2` gives double-declining balance."
///   - q: "When does `DDB` return `#NUM!`?"
///     a: "Invalid non-positive inputs (`life`, `period`, `factor`), negative `cost`/`salvage`, or `period > life`."
///   - q: "What happens with a fractional `period`?"
///     a: "Excel does not truncate it: `DDB(c,s,l,1.5)` is the book value after half a period times the rate."
/// ```
#[derive(Debug)]
pub struct DdbFn;
/// [formualizer-docgen:schema:start]
/// Name: DDB
/// Type: DdbFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: DDB(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for DdbFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "DDB"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static SCHEMA: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            vec![
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
            ]
        });
        &SCHEMA[..]
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let cost = coerce_num(&args[0])?;
        let salvage = coerce_num(&args[1])?;
        let life = coerce_num(&args[2])?;
        let period = coerce_num(&args[3])?;
        let factor = if args.len() > 4 {
            coerce_num(&args[4])?
        } else {
            2.0
        };

        if cost < 0.0
            || salvage < 0.0
            || life <= 0.0
            || period <= 0.0
            || factor <= 0.0
            || period > life
        {
            return Ok(CalcValue::Scalar(
                LiteralValue::Error(ExcelError::new_num()),
            ));
        }

        Ok(CalcValue::Scalar(LiteralValue::Number(
            declining_balance(cost, salvage, life, period, factor).max(0.0),
        )))
    }
}

/// Declining-balance depreciation in `period`, which may be fractional: the
/// book value after `period - 1` periods (the cost before the first) times
/// the rate `factor / life` (at most 1), no further than salvage. Negative
/// when the book value is already below salvage.
fn declining_balance(cost: f64, salvage: f64, life: f64, period: f64, factor: f64) -> f64 {
    let rate = (factor / life).min(1.0);
    let book = cost * (1.0 - rate).powf((period - 1.0).max(0.0));
    (book * rate).min(book - salvage)
}

/// One VDB term: DDB in whole period `period` (1-based) of an asset worth
/// `cost`, never below salvage and never negative (LibreOffice's ScGetDDB,
/// which Excel's VDB follows).
fn vdb_ddb(cost: f64, salvage: f64, life: f64, period: f64, factor: f64) -> f64 {
    let rate = factor / life;
    let (rate, old) = if rate >= 1.0 {
        (1.0, if period == 1.0 { cost } else { 0.0 })
    } else {
        (rate, cost * (1.0 - rate).powf(period - 1.0))
    };
    let new = cost * (1.0 - rate).powf(period);
    let ddb = if new < salvage {
        old - salvage
    } else {
        old - new
    };
    ddb.max(0.0)
}

/// Depreciation of an asset worth `cost` over its first `period` periods
/// (the last one partly when `period` is fractional), declining balance until
/// straight line over the `life1` periods left is larger, and whether it
/// switched to straight line.
fn vdb_from_start(
    cost: f64,
    salvage: f64,
    life: f64,
    life1: f64,
    period: f64,
    factor: f64,
) -> (f64, bool) {
    let end = period.ceil();
    let last = end as u64;
    let mut depreciable = cost - salvage;
    let (mut total, mut sln, mut straight) = (0.0, 0.0, false);
    for i in 1..=last {
        let mut term = if straight {
            sln
        } else {
            let ddb = vdb_ddb(cost, salvage, life, i as f64, factor);
            sln = depreciable / (life1 - (i - 1) as f64);
            if sln > ddb {
                straight = true;
                sln
            } else {
                depreciable -= ddb;
                ddb
            }
        };
        if i == last {
            term *= period + 1.0 - end;
        }
        total += term;
    }
    (total, straight)
}

/// Returns the depreciation of an asset for any period, including partial
/// periods, by the declining-balance method switching to straight line.
///
/// # Remarks
/// - The asset is first depreciated to `start_period`; from there it is a new schedule of
///   periods starting at `start_period`, the rate still `factor / life` and the straight line
///   over the `life - start_period` periods left, up to `end_period`.
/// - `no_switch` TRUE keeps declining balance throughout, each whole period's term scaled by
///   the part of it inside the range.
/// - `#NUM!` for a negative cost, start or factor, an end before the start or past `life`;
///   `life` 0 is `#DIV/0!`. Logicals are numbers (`TRUE` is 1).
/// - A salvage above cost puts the whole negative amount in the first period.
/// - From a fractional `start_period` into the straight-line part Excel's result follows a
///   rule the probes did not pin down: `#N/IMPL!`, so the workbook is recalculated elsewhere.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Months 6 to 18 of a 10-year asset"
/// formula: "=VDB(2400,300,10*12,6,18)"
/// expected: 396.30605326475785
/// ```
///
/// ```yaml,sandbox
/// title: "A partial first year"
/// formula: "=VDB(2400,300,10,0,0.875,1.5)"
/// expected: 315
/// ```
/// ```yaml,docs
/// related:
///   - DDB
///   - SLN
/// ```
#[derive(Debug)]
pub struct VdbFn;

/// [formualizer-docgen:schema:start]
/// Name: VDB
/// Type: VdbFn
/// Min args: 5
/// Max args: variadic
/// Variadic: true
/// Signature: VDB(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for VdbFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "VDB"
    }
    fn min_args(&self) -> usize {
        5
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(7)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        _ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = (|| {
            let mut numbers = [0.0, 0.0, 0.0, 0.0, 0.0, 2.0];
            for (slot, arg) in numbers.iter_mut().zip(args) {
                *slot = coerce_num(arg)?;
            }
            let [cost, salvage, life, start, end, factor] = numbers;
            let no_switch = logical(args, 6, false)?;
            if start < 0.0 || end < start || end > life || cost < 0.0 || factor < 0.0 || life < 0.0
            {
                return Err(ExcelError::new_num());
            }
            if life == 0.0 {
                return Err(ExcelError::new_div());
            }
            if no_switch {
                let (first, last) = (start.floor(), end.ceil());
                let mut total = 0.0;
                for i in (first as u64 + 1)..=(last as u64) {
                    let mut term = vdb_ddb(cost, salvage, life, i as f64, factor);
                    if i == first as u64 + 1 {
                        term *= end.min(first + 1.0) - start;
                    } else if i == last as u64 {
                        term *= end + 1.0 - last;
                    }
                    total += term;
                }
                return Ok(total);
            }
            if salvage > cost {
                // The whole (negative) amount goes in the first period.
                let to = |period: f64| (cost - salvage) * period.min(1.0);
                return Ok(to(end) - to(start));
            }
            let book = cost - vdb_from_start(cost, salvage, life, life, start, factor).0;
            let (value, straight) =
                vdb_from_start(book, salvage, life, life - start, end - start, factor);
            if straight && start.fract() != 0.0 {
                return Err(ExcelError::new(ExcelErrorKind::NImpl).with_message(
                    "VDB from a fractional start_period into the straight-line part",
                ));
            }
            Ok(value)
        })();
        Ok(number_value(value))
    }
}

/// The arguments AMORLINC and AMORDEGRC share: cost, date_purchased,
/// first_period, salvage, period (truncated), rate, [basis], and the
/// fraction of a year from purchase to the end of the first period.
struct Amortization {
    cost: f64,
    salvage: f64,
    period: f64,
    rate: f64,
    first_year: f64,
    purchased_on_first: bool,
}

fn amortization(
    args: &[ArgumentHandle],
    ctx: &dyn FunctionContext<'_>,
) -> Result<Amortization, ExcelError> {
    let system = ctx.date_system();
    let [cost, purchased, first, salvage, period, rate, basis] = numbers(args, [0.0; 7])?;
    let purchased = date(purchased, system)?;
    let first = date(first, system)?;
    let basis = Basis::new(basis)?;
    if cost <= 0.0
        || salvage < 0.0
        || salvage > cost
        || period < 0.0
        || rate <= 0.0
        || first < purchased
        || basis == Basis::Actual360
    {
        return Err(ExcelError::new_num());
    }
    Ok(Amortization {
        cost,
        salvage,
        period: period.trunc(),
        rate,
        first_year: amortization_year_fraction(purchased, first, basis),
        purchased_on_first: purchased == first,
    })
}

/// The year fraction AMORLINC and AMORDEGRC depreciate the first period by:
/// 30/360 days over 360, or for actual/actual and actual/365 the actual days
/// (29 February read as the 28th) over the purchase year's days or 365.
fn amortization_year_fraction(purchased: NaiveDate, first: NaiveDate, basis: Basis) -> f64 {
    let feb_28 = |d: NaiveDate| {
        if d.month() == 2 && d.day() == 29 {
            d.with_day(28).expect("28 February")
        } else {
            d
        }
    };
    match basis {
        Basis::Actual => {
            let year = if NaiveDate::from_ymd_opt(purchased.year(), 2, 29).is_some() {
                366.0
            } else {
                365.0
            };
            (feb_28(first) - feb_28(purchased)).num_days() as f64 / year
        }
        Basis::Actual365 => (feb_28(first) - feb_28(purchased)).num_days() as f64 / 365.0,
        _ => basis.days(purchased, first) / 360.0,
    }
}

/// Returns the depreciation for an accounting period on the French
/// linear system.
///
/// # Remarks
/// - The first period depreciates `cost * rate` times the year fraction from purchase to the
///   end of the first period, then `cost * rate` a period until `cost - salvage` is reached.
/// - `#NUM!` for cost, rate at most 0, a negative period or salvage, salvage above cost, the
///   first period before purchase, or basis 2.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Actual/actual"
/// formula: "=AMORLINC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)"
/// expected: 360
/// ```
/// ```yaml,docs
/// related:
///   - AMORDEGRC
///   - SLN
/// ```
#[derive(Debug)]
pub struct AmorlincFn;

/// [formualizer-docgen:schema:start]
/// Name: AMORLINC
/// Type: AmorlincFn
/// Min args: 6
/// Max args: variadic
/// Variadic: true
/// Signature: AMORLINC(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for AmorlincFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "AMORLINC"
    }
    fn min_args(&self) -> usize {
        6
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(7)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = amortization(args, ctx).map(|a| {
            let one = a.cost * a.rate;
            let first = if a.purchased_on_first {
                one
            } else {
                a.first_year * a.rate * a.cost
            };
            let full = ((a.cost - a.salvage - first) / one).trunc();
            let result = if a.period == 0.0 {
                first
            } else if a.period <= full {
                one
            } else if a.period == full + 1.0 {
                a.cost - a.salvage - one * full - first
            } else {
                0.0
            };
            result.max(0.0)
        });
        Ok(number_value(value))
    }
}

/// Returns the depreciation for an accounting period on the French
/// degressive system.
///
/// # Remarks
/// - The rate is `rate` times a coefficient from the life `1/rate`: 1.5 up to 4 years, 2 up
///   to 6, 2.5 beyond; a life of 2 years or less is `#NUM!`. Each period's depreciation is
///   rounded to a whole number.
/// - The period two before the end of the life (1/rate rounded up, one less when bought on
///   the first period's end) takes half the remaining value and the next the rest; nothing is
///   depreciated below salvage. Only the result is rounded; the book value keeps every digit.
/// - Arguments as for AMORLINC.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Actual/actual"
/// formula: "=AMORDEGRC(2400,DATE(2008,8,19),DATE(2008,12,31),300,1,0.15,1)"
/// expected: 776
/// ```
/// ```yaml,docs
/// related:
///   - AMORLINC
///   - DDB
/// ```
#[derive(Debug)]
pub struct AmordegrcFn;

/// [formualizer-docgen:schema:start]
/// Name: AMORDEGRC
/// Type: AmordegrcFn
/// Min args: 6
/// Max args: variadic
/// Variadic: true
/// Signature: AMORDEGRC(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for AmordegrcFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "AMORDEGRC"
    }
    fn min_args(&self) -> usize {
        6
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(7)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = amortization(args, ctx).and_then(|a| {
            let life = 1.0 / a.rate;
            let coefficient = if life <= 2.0 {
                return Err(ExcelError::new_num());
            } else if life <= 4.0 {
                1.5
            } else if life <= 6.0 {
                2.0
            } else {
                2.5
            };
            let mut rate = a.rate * coefficient;
            let first = if a.purchased_on_first {
                (a.cost * rate).round()
            } else {
                (a.cost * rate * a.first_year).round()
            };
            if a.period == 0.0 {
                return Ok(first);
            }
            // Bought on the first period's end, the life holds one period less.
            let periods = life.ceil() - if a.purchased_on_first { 1.0 } else { 0.0 };
            // Only the result is rounded; the book value keeps every digit.
            let mut remaining = a.cost - first;
            let mut depreciation = 0.0;
            let mut counted = 0.0;
            while counted < a.period {
                counted += 1.0;
                depreciation = if periods - counted == 2.0 {
                    rate = 1.0;
                    remaining * 0.5
                } else {
                    rate * remaining
                };
                if remaining < a.salvage {
                    depreciation = 0.0;
                }
                remaining -= depreciation;
            }
            Ok(depreciation.round())
        });
        Ok(number_value(value))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(VdbFn));
    crate::function_registry::register_builtin(Arc::new(AmorlincFn));
    crate::function_registry::register_builtin(Arc::new(AmordegrcFn));
    crate::function_registry::register_builtin(Arc::new(SlnFn));
    crate::function_registry::register_builtin(Arc::new(SydFn));
    crate::function_registry::register_builtin(Arc::new(DbFn));
    crate::function_registry::register_builtin(Arc::new(DdbFn));
}
