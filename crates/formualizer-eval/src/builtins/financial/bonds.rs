//! Coupon bonds and Treasury bills: ACCRINT, ACCRINTM, PRICE, YIELD,
//! DURATION, MDURATION, TBILLEQ, TBILLPRICE, TBILLYIELD, on the day counts
//! and coupon schedule of `coupon.rs`, as Excel for Windows 16.0.20430
//! computes them (ops/excel-finance-probe-20261006.md).

use super::coupon::{
    Basis, Period, add_months, coupon_date, date, days_us_30_360, frequency, logical,
    number_schema, number_value, numbers, period, periods_back,
};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use chrono::{Datelike, NaiveDate};
use formualizer_common::ExcelError;
use formualizer_macros::func_caps;

/// The par value ACCRINT and ACCRINTM take when the argument is left empty.
const DEFAULT_PAR: f64 = 1000.0;

/// ACCRINT's par: an empty argument (`ACCRINT(i,f,s,0.1,,2)`) is 1,000.
fn par_arg(args: &[ArgumentHandle], index: usize, value: f64) -> f64 {
    if args.get(index).is_some_and(|arg| arg.is_omitted()) {
        DEFAULT_PAR
    } else {
        value
    }
}

/// Accrued interest of a security paying periodic interest, Excel's way.
///
/// The accrual is counted in the quasi-coupon periods of `first_interest`'s
/// schedule. It ends in the period that starts on the coupon date on or
/// before settlement when settlement is after the first interest date and
/// `from_issue` is set; otherwise in the period that ends on the first
/// interest date, so that a settlement before that period counts negative
/// days and a settlement after it counts every day since in that one period.
/// Each earlier period back to issue adds a whole period when `from_issue` is
/// set and nothing when it is not, and the period holding issue adds the
/// days from issue to its end.
fn accrued_interest(
    issue: NaiveDate,
    first_interest: NaiveDate,
    settlement: NaiveDate,
    frequency: u32,
    basis: Basis,
    from_issue: bool,
) -> f64 {
    let months = 12 / frequency as i32;
    let mut k = if settlement > first_interest && from_issue {
        periods_back(first_interest, settlement, months)
    } else {
        1
    };
    let pcd = coupon_date(first_interest, k * months);
    let start = issue.max(pcd);
    let length = basis.period_days(
        &period_containing(pcd, first_interest, frequency),
        frequency,
    );
    let mut periods = basis.days(start, settlement) / length;

    let mut end = pcd;
    while end > issue {
        k += 1;
        let begin = coupon_date(first_interest, k * months);
        periods += if issue <= begin {
            if from_issue { 1.0 } else { 0.0 }
        } else {
            let (days, length) = match basis {
                Basis::Us30 => (
                    days_us_30_360(issue, end, false) as f64,
                    days_us_30_360(begin, end, true) as f64,
                ),
                Basis::Actual | Basis::Euro30 => (basis.days(issue, end), basis.days(begin, end)),
                _ => (
                    basis.days(issue, end),
                    basis.period_days(&period_containing(begin, end, frequency), frequency),
                ),
            };
            days / length
        };
        end = begin;
    }
    periods
}

/// The coupon period holding `date` on `anchor`'s schedule; the period ending
/// on `anchor` when `date` is `anchor`.
fn period_containing(date: NaiveDate, anchor: NaiveDate, frequency: u32) -> Period {
    let months = 12 / frequency as i32;
    let k = if date == anchor {
        1
    } else {
        periods_back(anchor, date, months)
    };
    Period {
        pcd: coupon_date(anchor, k * months),
        ncd: coupon_date(anchor, (k - 1) * months),
        coupons: None,
    }
}

/// Returns the accrued interest for a security that pays periodic interest.
///
/// # Remarks
/// - Accrues in the quasi-coupon periods of `first_interest`'s schedule. `calc_method` TRUE
///   (the default) counts every whole period from issue; FALSE counts only the days from issue
///   to the first period and the days in settlement's stretch of it.
/// - `rate` and `par` must be positive (an empty `par` is 1,000); settlement must be after issue.
/// - `frequency` is 1, 2 or 4 and `basis` 0 to 4 after truncation, otherwise `#NUM!`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Accrued from issue, 30/360"
/// formula: "=ACCRINT(DATE(2008,3,1),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0)"
/// expected: 16.666666666666664
/// ```
///
/// ```yaml,sandbox
/// title: "Issue more than a period before the first interest date"
/// formula: "=ACCRINT(DATE(2007,4,5),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0,TRUE)"
/// expected: 107.50000000000001
/// ```
/// ```yaml,docs
/// related:
///   - ACCRINTM
///   - COUPPCD
///   - PRICE
/// faq:
///   - q: "Does `calc_method` matter when settlement is before the first interest date?"
///     a: "Yes, when issue is more than a period before it: FALSE leaves out the whole periods between issue and the first interest period."
/// ```
#[derive(Debug)]
pub struct AccrintFn;

/// [formualizer-docgen:schema:start]
/// Name: ACCRINT
/// Type: AccrintFn
/// Min args: 6
/// Max args: variadic
/// Variadic: true
/// Signature: ACCRINT(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7: number@scalar, arg8...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg8{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for AccrintFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ACCRINT"
    }
    fn min_args(&self) -> usize {
        6
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
        let system = ctx.date_system();
        let value = (|| {
            let [issue, first_interest, settlement, rate, par, freq, basis] =
                numbers(&args[..args.len().min(7)], [0.0; 7])?;
            let from_issue = logical(args, 7, true)?;
            let par = par_arg(args, 4, par);
            let issue = date(issue, system)?;
            let first_interest = date(first_interest, system)?;
            let settlement = date(settlement, system)?;
            let frequency = frequency(freq)?;
            let basis = Basis::new(basis)?;
            if rate <= 0.0 || par <= 0.0 || settlement <= issue {
                return Err(ExcelError::new_num());
            }
            let periods = accrued_interest(
                issue,
                first_interest,
                settlement,
                frequency,
                basis,
                from_issue,
            );
            Ok(par * rate / frequency as f64 * periods)
        })();
        Ok(number_value(value))
    }
}

/// Returns the accrued interest for a security that pays interest at
/// maturity.
///
/// # Remarks
/// - `par * rate * YEARFRAC(issue, settlement, basis)`; an empty `par` is 1,000.
/// - `rate` and `par` must be positive; settlement before issue is `#NUM!`, on issue 0.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Actual/365"
/// formula: "=ACCRINTM(DATE(2008,4,1),DATE(2008,6,15),0.1,1000,3)"
/// expected: 20.54794520547945
/// ```
/// ```yaml,docs
/// related:
///   - ACCRINT
///   - YEARFRAC
/// ```
#[derive(Debug)]
pub struct AccrintmFn;

/// [formualizer-docgen:schema:start]
/// Name: ACCRINTM
/// Type: AccrintmFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: ACCRINTM(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for AccrintmFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "ACCRINTM"
    }
    fn min_args(&self) -> usize {
        4
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(5)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let system = ctx.date_system();
        let value = (|| {
            let [issue, settlement, rate, par, basis] = numbers(args, [0.0; 5])?;
            let par = par_arg(args, 3, par);
            let issue = date(issue, system)?;
            let settlement = date(settlement, system)?;
            let basis = Basis::new(basis)?;
            if rate <= 0.0 || par <= 0.0 || settlement < issue {
                return Err(ExcelError::new_num());
            }
            Ok(par * rate * basis.year_fraction(issue, settlement))
        })();
        Ok(number_value(value))
    }
}

/// The price per 100 face value at an annual yield: the remaining coupons and
/// the redemption discounted from settlement, less the interest accrued since
/// the previous coupon date. The days to the next coupon are the period's days
/// less the days before settlement, whatever the basis.
pub(crate) fn bond_price(
    settlement: NaiveDate,
    period: &Period,
    coupons: i32,
    rate: f64,
    yld: f64,
    redemption: f64,
    frequency: u32,
    basis: Basis,
) -> f64 {
    let f = frequency as f64;
    let e = basis.period_days(period, frequency);
    let a = basis.days_before(period, settlement);
    let dsc = e - a;
    let coupon = 100.0 * rate / f;
    if coupons == 1 {
        return (redemption + coupon) / (1.0 + dsc / e * yld / f) - a / e * coupon;
    }
    let x = 1.0 + yld / f;
    let t = dsc / e;
    let mut value = redemption / x.powf(coupons as f64 - 1.0 + t);
    for k in 1..=coupons {
        value += coupon / x.powf(k as f64 - 1.0 + t);
    }
    value - a / e * coupon
}

/// The settlement and coupon arguments PRICE, YIELD and DURATION share:
/// settlement before maturity, frequency 1, 2 or 4, basis 0 to 4.
struct Bond {
    settlement: NaiveDate,
    period: Period,
    coupons: i32,
    frequency: u32,
    basis: Basis,
}

fn bond(
    settlement: f64,
    maturity: f64,
    freq: f64,
    basis: f64,
    ctx: &dyn FunctionContext<'_>,
) -> Result<Bond, ExcelError> {
    let system = ctx.date_system();
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    let frequency = frequency(freq)?;
    let basis = Basis::new(basis)?;
    if settlement >= maturity {
        return Err(ExcelError::new_num());
    }
    let period = period(settlement, maturity, frequency, system);
    let coupons = period.count()?;
    Ok(Bond {
        settlement,
        period,
        coupons,
        frequency,
        basis,
    })
}

/// Returns the price per 100 face value of a security that pays periodic
/// interest.
///
/// # Remarks
/// - `rate` and `yld` must be at least 0 and `redemption` positive, otherwise `#NUM!`.
/// - The coupon schedule is counted back from maturity (see COUPPCD); the days to the next
///   coupon are COUPDAYS - COUPDAYBS, as Excel counts them, for every basis.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Semiannual bond, 30/360"
/// formula: "=PRICE(DATE(2008,2,15),DATE(2017,11,15),0.0575,0.065,100,2,0)"
/// expected: 94.63436162132213
/// ```
/// ```yaml,docs
/// related:
///   - YIELD
///   - DURATION
///   - COUPDAYS
/// ```
#[derive(Debug)]
pub struct PriceFn;

/// [formualizer-docgen:schema:start]
/// Name: PRICE
/// Type: PriceFn
/// Min args: 6
/// Max args: variadic
/// Variadic: true
/// Signature: PRICE(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for PriceFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "PRICE"
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
        let value = (|| {
            let [settlement, maturity, rate, yld, redemption, freq, basis] =
                numbers(args, [0.0; 7])?;
            let b = bond(settlement, maturity, freq, basis, ctx)?;
            if rate < 0.0 || yld < 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok(bond_price(
                b.settlement,
                &b.period,
                b.coupons,
                rate,
                yld,
                redemption,
                b.frequency,
                b.basis,
            ))
        })();
        Ok(number_value(value))
    }
}

/// YIELD with one coupon left: the simple yield over the days from settlement
/// to the coupon, counted by the basis; the period is 360/frequency days on
/// the 30/360 bases and its actual days on the others.
fn single_coupon_yield(b: &Bond, rate: f64, price: f64, redemption: f64) -> f64 {
    let f = b.frequency as f64;
    let a = b.basis.days_before(&b.period, b.settlement);
    let dsr = b.basis.days(b.settlement, b.period.ncd);
    let e = match b.basis {
        Basis::Us30 | Basis::Euro30 => 360.0 / f,
        _ => Basis::Actual.days(b.period.pcd, b.period.ncd),
    };
    let paid = redemption / 100.0 + rate / f;
    let cost = price / 100.0 + a / e * rate / f;
    (paid - cost) / cost * f * e / dsr
}

/// The yield at which `bond_price` is `price`: Newton's method on the price,
/// which is convex and falling in the yield, so the iterates approach the root
/// from below once the first step is taken.
fn solve_yield(b: &Bond, rate: f64, price: f64, redemption: f64) -> Option<f64> {
    let f = b.frequency as f64;
    let at = |y: f64| {
        bond_price(
            b.settlement,
            &b.period,
            b.coupons,
            rate,
            y,
            redemption,
            b.frequency,
            b.basis,
        )
    };
    let e = b.basis.period_days(&b.period, b.frequency);
    let t = (e - b.basis.days_before(&b.period, b.settlement)) / e;
    let coupon = 100.0 * rate / f;
    let slope = |y: f64| {
        let x = 1.0 + y / f;
        let n = b.coupons as f64;
        let mut d = -(n - 1.0 + t) * redemption / x.powf(n + t) / f;
        for k in 1..=b.coupons {
            let p = k as f64 - 1.0 + t;
            d -= p * coupon / x.powf(p + 1.0) / f;
        }
        d
    };
    let floor = -f;
    let mut y = 0.05_f64.max(rate);
    for _ in 0..200 {
        let d = slope(y);
        if !d.is_finite() || d == 0.0 {
            return None;
        }
        let mut next = y - (at(y) - price) / d;
        if next <= floor {
            next = (y + floor) / 2.0;
        }
        if (next - y).abs() <= 1e-15 * next.abs().max(1e-10) {
            return Some(next);
        }
        y = next;
    }
    ((at(y) - price).abs() <= 1e-9 * price).then_some(y)
}

/// Returns the yield of a security that pays periodic interest.
///
/// # Remarks
/// - With one coupon left the yield is computed directly; otherwise it is the yield at which
///   PRICE gives `pr`, found by Newton's method. It may be negative.
/// - `rate` must be at least 0, `pr` and `redemption` positive.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Semiannual bond, 30/360"
/// formula: "=YIELD(DATE(2008,2,15),DATE(2016,11,15),0.0575,95.04287,100,2,0)"
/// expected: 0.06500000688073
/// ```
/// ```yaml,docs
/// related:
///   - PRICE
/// ```
#[derive(Debug)]
pub struct YieldFn;

/// [formualizer-docgen:schema:start]
/// Name: YIELD
/// Type: YieldFn
/// Min args: 6
/// Max args: variadic
/// Variadic: true
/// Signature: YIELD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6: number@scalar, arg7...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg7{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for YieldFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "YIELD"
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
        let value = (|| {
            let [settlement, maturity, rate, price, redemption, freq, basis] =
                numbers(args, [0.0; 7])?;
            let b = bond(settlement, maturity, freq, basis, ctx)?;
            if rate < 0.0 || price <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            if b.coupons <= 1 {
                return Ok(single_coupon_yield(&b, rate, price, redemption));
            }
            solve_yield(&b, rate, price, redemption).ok_or_else(ExcelError::new_num)
        })();
        Ok(number_value(value))
    }
}

/// Macaulay duration in years: the cash flows' times, from settlement in
/// coupon periods (the first one COUPDAYS - COUPDAYBS over COUPDAYS),
/// weighted by their discounted values.
fn macaulay_duration(b: &Bond, coupon_rate: f64, yld: f64) -> f64 {
    let f = b.frequency as f64;
    let e = b.basis.period_days(&b.period, b.frequency);
    let dsc = e - b.basis.days_before(&b.period, b.settlement);
    let x1 = dsc / e;
    let x3 = yld / f + 1.0;
    let n = b.coupons as f64;
    let x2 = x1 + n - 1.0;
    let x4 = x3.powf(x2);
    let mut weighted = x2 * 100.0 / x4;
    let mut value = 100.0 / x4;
    for k in 1..=b.coupons {
        let t = k as f64 - 1.0 + x1;
        let cash = (100.0 * coupon_rate / f) / x3.powf(t);
        weighted += cash * t;
        value += cash;
    }
    weighted / value / f
}

fn duration_args(
    args: &[ArgumentHandle],
    ctx: &dyn FunctionContext<'_>,
) -> Result<(Bond, f64, f64), ExcelError> {
    let [settlement, maturity, coupon, yld, freq, basis] = numbers(args, [0.0; 6])?;
    let b = bond(settlement, maturity, freq, basis, ctx)?;
    if coupon < 0.0 || yld < 0.0 {
        return Err(ExcelError::new_num());
    }
    Ok((b, coupon, yld))
}

/// Returns the Macaulay duration of a security that pays periodic interest.
///
/// # Remarks
/// - `coupon` and `yld` must be at least 0.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "30-year semiannual bond"
/// formula: "=DURATION(DATE(2018,7,1),DATE(2048,1,1),0.08,0.09,2,1)"
/// expected: 10.919145281591932
/// ```
/// ```yaml,docs
/// related:
///   - MDURATION
///   - PRICE
/// ```
#[derive(Debug)]
pub struct DurationFn;

/// [formualizer-docgen:schema:start]
/// Name: DURATION
/// Type: DurationFn
/// Min args: 5
/// Max args: variadic
/// Variadic: true
/// Signature: DURATION(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for DurationFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "DURATION"
    }
    fn min_args(&self) -> usize {
        5
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(6)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value =
            duration_args(args, ctx).map(|(b, coupon, yld)| macaulay_duration(&b, coupon, yld));
        Ok(number_value(value))
    }
}

/// Returns the modified duration of a security that pays periodic interest:
/// DURATION / (1 + yld / frequency).
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "8-year semiannual bond"
/// formula: "=MDURATION(DATE(2008,1,1),DATE(2016,1,1),0.08,0.09,2,1)"
/// expected: 5.735669813918838
/// ```
/// ```yaml,docs
/// related:
///   - DURATION
/// ```
#[derive(Debug)]
pub struct MdurationFn;

/// [formualizer-docgen:schema:start]
/// Name: MDURATION
/// Type: MdurationFn
/// Min args: 5
/// Max args: variadic
/// Variadic: true
/// Signature: MDURATION(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for MdurationFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "MDURATION"
    }
    fn min_args(&self) -> usize {
        5
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(6)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = duration_args(args, ctx).map(|(b, coupon, yld)| {
            macaulay_duration(&b, coupon, yld) / (yld / b.frequency as f64 + 1.0)
        });
        Ok(number_value(value))
    }
}

/// The days from settlement to maturity of a Treasury bill: maturity after
/// settlement and no more than a year after it.
fn bill_days(
    settlement: f64,
    maturity: f64,
    ctx: &dyn FunctionContext<'_>,
) -> Result<f64, ExcelError> {
    let system = ctx.date_system();
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    // A year after 29 February is 1 March.
    let year_later =
        NaiveDate::from_ymd_opt(settlement.year() + 1, settlement.month(), settlement.day())
            .unwrap_or_else(|| add_months(settlement, 12).succ_opt().expect("a date"));
    if maturity <= settlement || maturity > year_later {
        return Err(ExcelError::new_num());
    }
    Ok((maturity - settlement).num_days() as f64)
}

fn bill_args(
    args: &[ArgumentHandle],
    ctx: &dyn FunctionContext<'_>,
) -> Result<(f64, f64), ExcelError> {
    let [settlement, maturity, third] = numbers(args, [0.0; 3])?;
    Ok((bill_days(settlement, maturity, ctx)?, third))
}

/// Returns the bond-equivalent yield for a Treasury bill.
///
/// # Remarks
/// - Maturity must be after settlement and at most one year after it; `discount` positive,
///   and the bill's price, 100 * (1 - discount * DSM / 360), positive.
/// - Up to 182 days: `365 * discount / (360 - discount * DSM)`; beyond, the root of the
///   coupon-equivalent quadratic.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Bill of 62 days"
/// formula: "=TBILLEQ(DATE(2008,3,31),DATE(2008,6,1),0.0914)"
/// expected: 0.09415149356594302
/// ```
/// ```yaml,docs
/// related:
///   - TBILLPRICE
///   - TBILLYIELD
/// ```
#[derive(Debug)]
pub struct TbilleqFn;

/// [formualizer-docgen:schema:start]
/// Name: TBILLEQ
/// Type: TbilleqFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: TBILLEQ(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TbilleqFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "TBILLEQ"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(3)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = bill_args(args, ctx).and_then(|(dsm, discount)| {
            let price = 1.0 - discount * dsm / 360.0;
            if discount <= 0.0 || price <= 0.0 {
                return Err(ExcelError::new_num());
            }
            if dsm <= 182.0 {
                return Ok(365.0 * discount / (360.0 - discount * dsm));
            }
            // A bill of 366 days is a year of 366 days.
            let a = dsm / if dsm == 366.0 { 366.0 } else { 365.0 };
            let inner = a * a - (2.0 * a - 1.0) * (1.0 - 1.0 / price);
            if inner < 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok((-2.0 * a + 2.0 * inner.sqrt()) / (2.0 * a - 1.0))
        });
        Ok(number_value(value))
    }
}

/// Returns the price per 100 face value of a Treasury bill:
/// `100 * (1 - discount * DSM / 360)`.
///
/// # Remarks
/// - Maturity must be after settlement and at most one year after it; `discount` and the
///   price must be positive.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Bill of 62 days"
/// formula: "=TBILLPRICE(DATE(2008,3,31),DATE(2008,6,1),0.09)"
/// expected: 98.45
/// ```
/// ```yaml,docs
/// related:
///   - TBILLEQ
///   - TBILLYIELD
/// ```
#[derive(Debug)]
pub struct TbillpriceFn;

/// [formualizer-docgen:schema:start]
/// Name: TBILLPRICE
/// Type: TbillpriceFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: TBILLPRICE(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TbillpriceFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "TBILLPRICE"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(3)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = bill_args(args, ctx).and_then(|(dsm, discount)| {
            let price = 100.0 * (1.0 - discount * dsm / 360.0);
            if discount <= 0.0 || price <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok(price)
        });
        Ok(number_value(value))
    }
}

/// Returns the yield of a Treasury bill: `(100 - pr) / pr * 360 / DSM`.
///
/// # Remarks
/// - Maturity must be after settlement and at most one year after it; `pr` positive.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Bill of 62 days"
/// formula: "=TBILLYIELD(DATE(2008,3,31),DATE(2008,6,1),98.45)"
/// expected: 0.09141696292534264
/// ```
/// ```yaml,docs
/// related:
///   - TBILLEQ
///   - TBILLPRICE
/// ```
#[derive(Debug)]
pub struct TbillyieldFn;

/// [formualizer-docgen:schema:start]
/// Name: TBILLYIELD
/// Type: TbillyieldFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: TBILLYIELD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TbillyieldFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "TBILLYIELD"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(3)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        let value = bill_args(args, ctx).and_then(|(dsm, price)| {
            if price <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok((100.0 - price) / price * 360.0 / dsm)
        });
        Ok(number_value(value))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(AccrintFn));
    crate::function_registry::register_builtin(Arc::new(AccrintmFn));
    crate::function_registry::register_builtin(Arc::new(PriceFn));
    crate::function_registry::register_builtin(Arc::new(YieldFn));
    crate::function_registry::register_builtin(Arc::new(DurationFn));
    crate::function_registry::register_builtin(Arc::new(MdurationFn));
    crate::function_registry::register_builtin(Arc::new(TbilleqFn));
    crate::function_registry::register_builtin(Arc::new(TbillpriceFn));
    crate::function_registry::register_builtin(Arc::new(TbillyieldFn));
}

#[cfg(test)]
mod tests {
    use formualizer_common::LiteralValue;

    fn eval_in(system: crate::engine::DateSystem, formula: &str) -> LiteralValue {
        use crate::engine::{Engine, EvalConfig};
        use crate::interpreter::Interpreter;
        use crate::test_workbook::TestWorkbook;
        use formualizer_parse::parser::parse;

        let engine = Engine::new(
            TestWorkbook::new(),
            EvalConfig::default().with_date_system(system),
        );
        let interpreter = Interpreter::new(&engine, "Sheet1");
        interpreter
            .evaluate_ast(&parse(formula).expect("formula should parse"))
            .expect("formula should evaluate")
            .into_literal()
    }

    /// Bond day counts depend on the calendar dates the serials denote, so the
    /// same issue/settlement pair must accrue identically in both date systems.
    /// The dates are in 1904: read as 1900 serials they land on other dates.
    #[test]
    fn accrintm_follows_workbook_date_system_1900_and_1904() {
        use crate::engine::DateSystem;
        use chrono::{Datelike, NaiveDate};
        use formualizer_common::date_to_serial_for;

        let issue = NaiveDate::from_ymd_opt(1904, 1, 1).unwrap();
        let settlement = NaiveDate::from_ymd_opt(1904, 7, 1).unwrap();
        // Actual/360 over 182 days: 1000 * 0.1 * 182 / 360.
        let expected = 1000.0 * 0.1 * 182.0 / 360.0;
        for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
            let i = date_to_serial_for(system, &issue);
            let s = date_to_serial_for(system, &settlement);
            match eval_in(system, &format!("=ACCRINTM({i},{s},0.1,1000,2)")) {
                LiteralValue::Number(n) => assert!(
                    (n - expected).abs() < 1e-9,
                    "ACCRINTM under {system:?}: expected {expected}, got {n}"
                ),
                other => panic!("expected numeric ACCRINTM under {system:?}, got {other:?}"),
            }
        }
    }
}
