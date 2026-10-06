//! Securities with an odd first or last period: ODDFPRICE, ODDFYIELD,
//! ODDLPRICE, ODDLYIELD, as Excel for Windows 16.0.20430 computes them
//! (ops/excel-finance-probe-20261006.md).
//!
//! Unlike the COUP* functions, these walk the coupon schedule one period at a
//! time from the previous date, so a day lost to a short month stays lost
//! (back from 30 August: 28 February, then 28 August), unless the anchor is
//! the last day of its month, when every step is a month end. Excel's own
//! results follow that walk, the quasi-coupon periods of the odd period with
//! it, and count negative day spans as 0.

use super::coupon::{
    Basis, add_months, date, days_us_30_360, frequency, number_schema, number_value, numbers,
};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use chrono::{Datelike, NaiveDate};
use formualizer_common::ExcelError;
use formualizer_macros::func_caps;

fn is_month_end(date: NaiveDate) -> bool {
    date.succ_opt()
        .is_none_or(|next| next.month() != date.month())
}

/// One step of `months` months from `date`, then to the month's last day
/// when `month_end`.
fn step(date: NaiveDate, months: i32, month_end: bool) -> NaiveDate {
    let next = add_months(date, months);
    if month_end {
        add_months(next.with_day(1).expect("first of month"), 1)
            .pred_opt()
            .expect("month end")
    } else {
        next
    }
}

/// Walks from `from` by `months` (negative: back) until reaching or passing
/// `to`; returns the date reached and the one before it.
fn walk(from: NaiveDate, to: NaiveDate, months: i32, month_end: bool) -> (NaiveDate, NaiveDate) {
    let (mut front, mut trailing) = (from, to);
    while if months > 0 { front < to } else { front > to } {
        trailing = front;
        front = step(front, months, month_end);
    }
    (front, trailing)
}

/// The coupon dates on or before and after `settlement`, walked back from
/// `maturity`.
fn coupon_dates(
    settlement: NaiveDate,
    maturity: NaiveDate,
    frequency: u32,
) -> (NaiveDate, NaiveDate) {
    walk(
        maturity,
        settlement,
        -(12 / frequency as i32),
        is_month_end(maturity),
    )
}

/// The days of settlement's coupon period on the walked schedule.
fn coupon_days(basis: Basis, settlement: NaiveDate, maturity: NaiveDate, frequency: u32) -> f64 {
    match basis {
        Basis::Actual => {
            let (pcd, ncd) = coupon_dates(settlement, maturity, frequency);
            (ncd - pcd).num_days() as f64
        }
        Basis::Actual365 => 365.0 / frequency as f64,
        _ => 360.0 / frequency as f64,
    }
}

/// The coupons from settlement to maturity on the walked schedule.
fn coupon_count(settlement: NaiveDate, maturity: NaiveDate, frequency: u32) -> f64 {
    let (pcd, _) = coupon_dates(settlement, maturity, frequency);
    let months = (maturity.year() - pcd.year()) * 12 + maturity.month() as i32 - pcd.month() as i32;
    f64::from(months) * frequency as f64 / 12.0
}

/// Days from `start` to `end` by the basis, 0 when `end` is earlier.
fn days(basis: Basis, start: NaiveDate, end: NaiveDate) -> f64 {
    basis.days(start, end).max(0.0)
}

/// The whole quasi-coupon periods from settlement to the first coupon, as
/// Excel counts them: from settlement (moved to its month's end when the
/// first coupon is a month end, which counts one period), one step at a time.
fn quasi_periods_to_first(first_coupon: NaiveDate, settlement: NaiveDate, months: i32) -> f64 {
    let month_end =
        if !is_month_end(first_coupon) && first_coupon.month() != 2 && first_coupon.day() > 28 {
            is_month_end(settlement)
        } else {
            is_month_end(first_coupon)
        };
    let start = step(settlement, 0, month_end);
    let mut count = if settlement < start { 1.0 } else { 0.0 };
    let mut front = step(start, months, month_end);
    while front < first_coupon {
        front = step(front, months, month_end);
        count += 1.0;
    }
    count
}

/// A bond with an odd first period: issue < settlement < first coupon <
/// maturity, the first coupon a step of maturity's schedule.
struct OddFirst {
    settlement: NaiveDate,
    maturity: NaiveDate,
    issue: NaiveDate,
    first_coupon: NaiveDate,
    frequency: u32,
    basis: Basis,
}

fn odd_first(
    [settlement, maturity, issue, first_coupon]: [f64; 4],
    freq: f64,
    basis: f64,
    ctx: &dyn FunctionContext<'_>,
) -> Result<OddFirst, ExcelError> {
    let system = ctx.date_system();
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    let issue = date(issue, system)?;
    let first_coupon = date(first_coupon, system)?;
    let frequency = frequency(freq)?;
    let basis = Basis::new(basis)?;
    if !(issue < settlement && settlement < first_coupon && first_coupon < maturity) {
        return Err(ExcelError::new_num());
    }
    let months = 12 / frequency as i32;
    let month_end = is_month_end(maturity);
    let (on_schedule, _) = walk(
        step(maturity, -months, month_end),
        first_coupon,
        -months,
        month_end,
    );
    if on_schedule != first_coupon {
        return Err(ExcelError::new_num());
    }
    Ok(OddFirst {
        settlement,
        maturity,
        issue,
        first_coupon,
        frequency,
        basis,
    })
}

/// ODDFPRICE: the coupons and the redemption discounted from settlement, plus
/// the odd first coupon, less the interest accrued since issue. A first
/// period shorter than settlement's coupon period is one period; a longer one
/// is counted in quasi-coupon periods walked back from the first coupon.
fn odd_first_price(b: &OddFirst, rate: f64, yld: f64, redemption: f64) -> f64 {
    let f = b.frequency as f64;
    let months = 12 / b.frequency as i32;
    let basis = b.basis;
    let x = yld / f + 1.0;
    let coupon = 100.0 * rate / f;
    let e = coupon_days(basis, b.settlement, b.first_coupon, b.frequency);
    let dfc = days(basis, b.issue, b.first_coupon);
    if dfc < e {
        let n = coupon_count(b.settlement, b.maturity, b.frequency);
        let dsc = days(basis, b.settlement, b.first_coupon);
        let a = days(basis, b.issue, b.settlement);
        let y = dsc / e;
        let mut value = redemption / x.powf(n - 1.0 + y) + coupon * dfc / e / x.powf(y);
        for k in 2..=n as i64 {
            value += coupon / x.powf(k as f64 - 1.0 + y);
        }
        return value - a / e * rate / f * 100.0;
    }

    let quasi = coupon_count(b.issue, b.first_coupon, b.frequency) as i64;
    let (mut dc, mut accrued) = (0.0, 0.0);
    let mut late = b.first_coupon;
    for index in (1..=quasi).rev() {
        let early = step(late, -months, false);
        let nl = if basis == Basis::Actual {
            days(basis, early, late)
        } else {
            e
        };
        let dci = if index > 1 {
            nl
        } else {
            days(basis, b.issue, late)
        };
        let a = days(basis, b.issue.max(early), b.settlement.min(late));
        late = early;
        dc += dci / nl;
        accrued += a / nl;
    }
    let dsc = match basis {
        Basis::Actual360 | Basis::Actual365 => {
            let (_, ncd) = coupon_dates(b.settlement, b.first_coupon, b.frequency);
            days(basis, b.settlement, ncd)
        }
        _ => {
            let (pcd, _) = coupon_dates(b.settlement, b.first_coupon, b.frequency);
            e - basis.days(pcd, b.settlement)
        }
    };
    let nq = quasi_periods_to_first(b.first_coupon, b.settlement, months);
    let n = coupon_count(b.first_coupon, b.maturity, b.frequency);
    let y = dsc / e;
    let mut value = redemption / x.powf(y + nq + n) + coupon * dc / x.powf(nq + y);
    for k in 1..=n as i64 {
        value += coupon / x.powf(k as f64 + nq + y);
    }
    value - coupon * accrued
}

/// The yield at which `price_at` gives `price`: Newton's method with a
/// central-difference slope, from 5%, kept above -frequency.
fn solve_rate(price: f64, price_at: impl Fn(f64) -> f64, floor: f64) -> Option<f64> {
    let mut y = 0.05;
    for _ in 0..300 {
        let p = price_at(y);
        let h = 1e-7 * y.abs().max(1e-3);
        let slope = (price_at(y + h) - price_at(y - h)) / (2.0 * h);
        if !slope.is_finite() || slope == 0.0 {
            return None;
        }
        let mut next = y - (p - price) / slope;
        if next <= floor {
            next = (y + floor) / 2.0;
        }
        if (next - y).abs() <= 1e-15 * next.abs().max(1e-10) {
            return Some(next);
        }
        y = next;
    }
    ((price_at(y) - price).abs() <= 1e-9 * price).then_some(y)
}

/// Returns the price per 100 face value of a security with an odd first
/// period.
///
/// # Remarks
/// - Dates must satisfy issue < settlement < first_coupon < maturity, with first_coupon on
///   maturity's coupon schedule; `rate` and `yld` at least 0, `redemption` positive.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Short first period, actual/actual"
/// formula: "=ODDFPRICE(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0785,0.0625,100,2,1)"
/// expected: 113.59771747407883
/// ```
/// ```yaml,docs
/// related:
///   - ODDFYIELD
///   - PRICE
/// ```
#[derive(Debug)]
pub struct OddfpriceFn;

/// [formualizer-docgen:schema:start]
/// Name: ODDFPRICE
/// Type: OddfpriceFn
/// Min args: 8
/// Max args: variadic
/// Variadic: true
/// Signature: ODDFPRICE(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7: number@scalar, arg8: number@scalar, arg9...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg8{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg9{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for OddfpriceFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ODDFPRICE"
    }
    fn min_args(&self) -> usize {
        8
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(9)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = (|| {
            let [s, m, i, fc, rate, yld, redemption, freq, basis] = numbers(args, [0.0; 9])?;
            let b = odd_first([s, m, i, fc], freq, basis, ctx)?;
            if rate < 0.0 || yld < 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok(odd_first_price(&b, rate, yld, redemption))
        })();
        Ok(number_value(value))
    }
}

/// Returns the yield of a security with an odd first period: the yield at
/// which ODDFPRICE gives `pr`.
///
/// # Remarks
/// - Dates as for ODDFPRICE; `rate` at least 0, `pr` and `redemption` positive.
/// - Solved by Newton's method to full precision; Excel stops its own iteration a little
///   earlier, so the last digits can differ (within 1E-9 relative on the probes).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Short first period, 30/360"
/// formula: "=ODDFYIELD(DATE(2008,11,11),DATE(2021,3,1),DATE(2008,10,15),DATE(2009,3,1),0.0575,84.5,100,2,0)"
/// expected: 0.07724554159729888
/// ```
/// ```yaml,docs
/// related:
///   - ODDFPRICE
///   - YIELD
/// ```
#[derive(Debug)]
pub struct OddfyieldFn;

/// [formualizer-docgen:schema:start]
/// Name: ODDFYIELD
/// Type: OddfyieldFn
/// Min args: 8
/// Max args: variadic
/// Variadic: true
/// Signature: ODDFYIELD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7: number@scalar, arg8: number@scalar, arg9...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg8{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg9{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for OddfyieldFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ODDFYIELD"
    }
    fn min_args(&self) -> usize {
        8
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(9)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = (|| {
            let [s, m, i, fc, rate, price, redemption, freq, basis] = numbers(args, [0.0; 9])?;
            let b = odd_first([s, m, i, fc], freq, basis, ctx)?;
            if rate < 0.0 || price <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            let floor = -(b.frequency as f64);
            solve_rate(price, |y| odd_first_price(&b, rate, y, redemption), floor)
                .ok_or_else(ExcelError::new_num)
        })();
        Ok(number_value(value))
    }
}

/// The odd last period of a bond in quasi-coupon periods walked forward from
/// the last interest date: the fractions of a period the last coupon covers,
/// that have accrued at settlement, and that remain from settlement to
/// maturity.
struct OddLast {
    covered: f64,
    accrued: f64,
    remaining: f64,
}

fn odd_last(
    [settlement, maturity, last_interest]: [f64; 3],
    freq: f64,
    basis: f64,
    ctx: &dyn FunctionContext<'_>,
) -> Result<(OddLast, f64), ExcelError> {
    let system = ctx.date_system();
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    let last = date(last_interest, system)?;
    let frequency = frequency(freq)?;
    let basis = Basis::new(basis)?;
    if !(last < settlement && settlement < maturity) {
        return Err(ExcelError::new_num());
    }
    let months = 12 / frequency as i32;
    // US 30/360 counts the quasi-coupon periods with both ends moved.
    let period_days = |start: NaiveDate, end: NaiveDate| match basis {
        Basis::Us30 => (days_us_30_360(start, end, true) as f64).max(0.0),
        _ => days(basis, start, end),
    };
    let quasi = coupon_count(last, maturity, frequency) as i64;
    let mut odd = OddLast {
        covered: 0.0,
        accrued: 0.0,
        remaining: 0.0,
    };
    let mut early = last;
    for index in 1..=quasi {
        let late = step(early, months, false);
        let nl = period_days(early, late);
        let dci = if index < quasi {
            nl
        } else {
            period_days(early, maturity)
        };
        let a = if late < settlement {
            dci
        } else if early < settlement {
            days(basis, early, settlement)
        } else {
            0.0
        };
        let dsc = days(basis, settlement.max(early), maturity.min(late));
        early = late;
        odd.covered += dci / nl;
        odd.accrued += a / nl;
        odd.remaining += dsc / nl;
    }
    Ok((odd, frequency as f64))
}

/// Returns the price per 100 face value of a security with an odd last
/// period.
///
/// # Remarks
/// - Dates must satisfy last_interest < settlement < maturity; `rate` and `yld` at least 0,
///   `redemption` positive.
///
/// # Examples
/// ```yaml,sandbox
/// title: "30/360"
/// formula: "=ODDLPRICE(DATE(2008,2,7),DATE(2008,6,15),DATE(2007,10,15),0.0375,0.0405,100,2,0)"
/// expected: 99.87828601472134
/// ```
/// ```yaml,docs
/// related:
///   - ODDLYIELD
///   - PRICE
/// ```
#[derive(Debug)]
pub struct OddlpriceFn;

/// [formualizer-docgen:schema:start]
/// Name: ODDLPRICE
/// Type: OddlpriceFn
/// Min args: 7
/// Max args: variadic
/// Variadic: true
/// Signature: ODDLPRICE(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7: number@scalar, arg8...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg8{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for OddlpriceFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ODDLPRICE"
    }
    fn min_args(&self) -> usize {
        7
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(8)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = (|| {
            let [s, m, l, rate, yld, redemption, freq, basis] = numbers(args, [0.0; 8])?;
            let (odd, f) = odd_last([s, m, l], freq, basis, ctx)?;
            if rate < 0.0 || yld < 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            let coupon = 100.0 * rate / f;
            Ok(
                (odd.covered * coupon + redemption) / (odd.remaining * yld / f + 1.0)
                    - odd.accrued * coupon,
            )
        })();
        Ok(number_value(value))
    }
}

/// Returns the yield of a security with an odd last period.
///
/// # Remarks
/// - Dates as for ODDLPRICE; `rate` at least 0, `pr` and `redemption` positive.
///
/// # Examples
/// ```yaml,sandbox
/// title: "30/360"
/// formula: "=ODDLYIELD(DATE(2008,4,20),DATE(2008,6,15),DATE(2007,12,24),0.0375,99.875,100,2,0)"
/// expected: 0.04519223562916916
/// ```
/// ```yaml,docs
/// related:
///   - ODDLPRICE
///   - YIELD
/// ```
#[derive(Debug)]
pub struct OddlyieldFn;

/// [formualizer-docgen:schema:start]
/// Name: ODDLYIELD
/// Type: OddlyieldFn
/// Min args: 7
/// Max args: variadic
/// Variadic: true
/// Signature: ODDLYIELD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7: number@scalar, arg8...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg8{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for OddlyieldFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ODDLYIELD"
    }
    fn min_args(&self) -> usize {
        7
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(8)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = (|| {
            let [s, m, l, rate, price, redemption, freq, basis] = numbers(args, [0.0; 8])?;
            let (odd, f) = odd_last([s, m, l], freq, basis, ctx)?;
            if rate < 0.0 || price <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            let coupon = 100.0 * rate / f;
            let paid = odd.covered * coupon + redemption;
            let cost = odd.accrued * coupon + price;
            Ok((paid - cost) / cost * (f / odd.remaining))
        })();
        Ok(number_value(value))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(OddfpriceFn));
    crate::function_registry::register_builtin(Arc::new(OddfyieldFn));
    crate::function_registry::register_builtin(Arc::new(OddlpriceFn));
    crate::function_registry::register_builtin(Arc::new(OddlyieldFn));
}
