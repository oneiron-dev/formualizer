//! Day counts and coupon schedules shared by Excel's securities functions,
//! as Excel for Windows 16.0.20430 computes them (probes:
//! ops/excel-finance-probe-20261006.md): COUPDAYBS, COUPDAYS, COUPDAYSNC,
//! COUPNCD, COUPNUM and COUPPCD here, and the pricing functions in
//! `bonds.rs`, `discount.rs` and `odd.rs`.
//!
//! Every argument is a number: numbers, blanks and numeric or date text
//! convert as VALUE() converts them, a logical is #VALUE!. Dates are serials
//! in the workbook's date system truncated to the day, frequency and basis
//! are truncated, errors in the arguments win before any check of the values.

use crate::args::ArgSchema;
use crate::coercion::{to_serial_lenient_in_year, to_serial_strict};
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use chrono::{Datelike, NaiveDate};
use formualizer_common::{
    DateSystem, ExcelError, ExcelErrorKind, LiteralValue, date_to_serial_for,
    try_serial_to_date_for,
};
use formualizer_macros::func_caps;
use std::sync::LazyLock;

/// A number argument of the securities and depreciation functions: a date
/// cell is its serial, text converts as VALUE() converts it, a blank is 0
/// and a logical is #VALUE! (COUPNUM(s,m,TRUE) is #VALUE!).
pub(crate) fn number(arg: &ArgumentHandle) -> Result<f64, ExcelError> {
    let value = arg.value()?.into_literal();
    match value {
        LiteralValue::Boolean(_) => Err(ExcelError::new_value()),
        LiteralValue::Text(_) => {
            to_serial_lenient_in_year(&value, arg.date_system(), Some(arg.current_year()))
        }
        other => to_serial_strict(&other, arg.date_system()),
    }
}

/// The arguments as numbers, in order so that the first error wins, with
/// `defaults` for trailing arguments the call leaves out. An empty argument
/// (`COUPNUM(s,m,2,)`) is 0.
pub(crate) fn numbers<const N: usize>(
    args: &[ArgumentHandle],
    defaults: [f64; N],
) -> Result<[f64; N], ExcelError> {
    let mut out = defaults;
    for (slot, arg) in out.iter_mut().zip(args) {
        *slot = number(arg)?;
    }
    Ok(out)
}

/// `count` number parameters, the schema every securities function shares.
pub(crate) fn number_schema(count: usize) -> &'static [ArgSchema] {
    static SCHEMA: LazyLock<Vec<ArgSchema>> =
        LazyLock::new(|| vec![ArgSchema::number_lenient_scalar(); 11]);
    &SCHEMA[..count]
}

/// A computed number as the function's value: errors become error values,
/// and a result that overflows is #NUM!.
pub(crate) fn number_value<'b>(result: Result<f64, ExcelError>) -> CalcValue<'b> {
    CalcValue::Scalar(match result {
        Ok(n) if n.is_finite() => LiteralValue::Number(n),
        Ok(_) => LiteralValue::Error(ExcelError::new_num()),
        Err(e) => LiteralValue::Error(e),
    })
}

/// A logical argument (ACCRINT's calc_method, VDB's no_switch): a logical, a
/// number (non-zero is TRUE), a blank (FALSE) or the text "TRUE"/"FALSE";
/// `default` when the call leaves it out or empty.
pub(crate) fn logical(
    args: &[ArgumentHandle],
    index: usize,
    default: bool,
) -> Result<bool, ExcelError> {
    match args.get(index) {
        Some(arg) if !arg.is_omitted() => match arg.value()?.into_literal() {
            LiteralValue::Number(n) => Ok(n != 0.0),
            LiteralValue::Int(i) => Ok(i != 0),
            other => crate::coercion::to_logical(&other),
        },
        _ => Ok(default),
    }
}

/// The calendar date of a date argument: its serial truncated to the day.
/// A negative serial or one past 9999-12-31 is #NUM!. Serial 60 of the 1900
/// system, Excel's 29 February 1900, is no calendar date; the securities
/// functions treat it as one (COUPNCD(40,60,4,0) is 60), so it is #N/IMPL!.
pub(crate) fn date(serial: f64, system: DateSystem) -> Result<NaiveDate, ExcelError> {
    if system == DateSystem::Excel1900 && serial.trunc() == 60.0 {
        return Err(ExcelError::new(ExcelErrorKind::NImpl)
            .with_message("serial 60, 29 February 1900, as a date"));
    }
    try_serial_to_date_for(system, serial)
}

/// The first day of the date system, serial 0 (1900-01-00, which is
/// 1899-12-31, or 1904-01-01). Excel reads a coupon date before it as this day.
pub(crate) fn day_zero(system: DateSystem) -> NaiveDate {
    try_serial_to_date_for(system, 0.0).expect("serial 0 is a date")
}

/// The serial of a date.
pub(crate) fn serial(date: NaiveDate, system: DateSystem) -> f64 {
    date_to_serial_for(system, &date)
}

/// Coupons per year: 1, 2 or 4 after truncation, otherwise #NUM!.
pub(crate) fn frequency(value: f64) -> Result<u32, ExcelError> {
    let frequency = value.trunc();
    if frequency == 1.0 || frequency == 2.0 || frequency == 4.0 {
        Ok(frequency as u32)
    } else {
        Err(ExcelError::new_num())
    }
}

/// The day-count basis argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Basis {
    /// 0, US (NASD) 30/360.
    Us30,
    /// 1, actual/actual.
    Actual,
    /// 2, actual/360.
    Actual360,
    /// 3, actual/365.
    Actual365,
    /// 4, European 30/360.
    Euro30,
}

impl Basis {
    /// 0 to 4 after truncation, otherwise #NUM!.
    pub(crate) fn new(value: f64) -> Result<Self, ExcelError> {
        let basis = value.trunc();
        if !(0.0..=4.0).contains(&basis) {
            return Err(ExcelError::new_num());
        }
        Ok(match basis as u8 {
            0 => Basis::Us30,
            1 => Basis::Actual,
            2 => Basis::Actual360,
            3 => Basis::Actual365,
            _ => Basis::Euro30,
        })
    }

    /// Days from `start` to `end` as the basis counts them: 30/360 (US with
    /// the start-date rule, or European) or actual days.
    pub(crate) fn days(self, start: NaiveDate, end: NaiveDate) -> f64 {
        match self {
            Basis::Us30 => days_us_30_360(start, end, false) as f64,
            Basis::Euro30 => days_euro_30_360(start, end) as f64,
            _ => actual_days(start, end) as f64,
        }
    }

    /// The days of a year from `start` to `end` (`start <= end`): 360, 365,
    /// or for actual/actual YEARFRAC's year length.
    pub(crate) fn year_days(self, start: NaiveDate, end: NaiveDate) -> f64 {
        match self {
            Basis::Actual => crate::builtins::datetime::yearfrac_actual_year_length(start, end),
            Basis::Actual365 => 365.0,
            _ => 360.0,
        }
    }

    /// The fraction of a year from `start` to `end` (`start <= end`), as
    /// YEARFRAC counts it.
    pub(crate) fn year_fraction(self, start: NaiveDate, end: NaiveDate) -> f64 {
        if start == end {
            return 0.0;
        }
        self.days(start, end) / self.year_days(start, end)
    }

    /// COUPDAYBS: days from the start of settlement's coupon period to
    /// settlement.
    pub(crate) fn days_before(self, period: &Period, settlement: NaiveDate) -> f64 {
        self.days(period.pcd, settlement)
    }

    /// COUPDAYSNC: days from settlement to the next coupon date. US 30/360
    /// counts the whole period with both ends moved and takes away the days
    /// before settlement.
    pub(crate) fn days_after(self, period: &Period, settlement: NaiveDate) -> f64 {
        match self {
            Basis::Us30 => {
                (days_us_30_360(period.pcd, period.ncd, true)
                    - days_us_30_360(period.pcd, settlement, false)) as f64
            }
            _ => self.days(settlement, period.ncd),
        }
    }

    /// The days of settlement's coupon period as PRICE, YIELD and DURATION
    /// count them (E): 360/frequency, 365/frequency for actual/365, the actual
    /// days from the previous to the next coupon date for actual/actual.
    pub(crate) fn period_days(self, period: &Period, frequency: u32) -> f64 {
        match self {
            Basis::Actual => actual_days(period.pcd, period.ncd) as f64,
            Basis::Actual365 => 365.0 / frequency as f64,
            _ => 360.0 / frequency as f64,
        }
    }
}

/// Actual days from `start` to `end`, as the difference of their 1900-system
/// serials: a span over February 1900 counts Excel's 29 February 1900.
pub(crate) fn actual_days(start: NaiveDate, end: NaiveDate) -> i64 {
    let phantom = NaiveDate::from_ymd_opt(1900, 3, 1).expect("1900-03-01");
    let crossings = i64::from(end >= phantom) - i64::from(start >= phantom);
    (end - start).num_days() + crossings
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (y, m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|d| d.pred_opt())
        .map_or(31, |d| d.day())
}

pub(crate) fn is_month_end(date: NaiveDate) -> bool {
    date.day() == days_in_month(date.year(), date.month())
}

fn last_of_february(date: NaiveDate) -> bool {
    date.month() == 2 && is_month_end(date)
}

fn days_360(start: (i32, u32, u32), end: (i32, u32, u32)) -> i64 {
    360 * i64::from(end.0 - start.0)
        + 30 * (i64::from(end.1) - i64::from(start.1))
        + (i64::from(end.2) - i64::from(start.2))
}

/// US (NASD) 30/360 days. A start on the 31st or the last day of February
/// counts as the 30th; an end on the 31st counts as the 30th when the start
/// is the 30th or 31st, and an end on the last day of February when the start
/// is one too. `whole_period` moves both ends whatever the start, as Excel
/// counts the days of a coupon period for COUPDAYSNC.
pub(crate) fn days_us_30_360(start: NaiveDate, end: NaiveDate, whole_period: bool) -> i64 {
    let (mut sd, mut ed) = (start.day(), end.day());
    if last_of_february(end) && (last_of_february(start) || whole_period) {
        ed = 30;
    }
    if ed == 31 && (sd >= 30 || whole_period) {
        ed = 30;
    }
    if sd == 31 || last_of_february(start) {
        sd = 30;
    }
    days_360(
        (start.year(), start.month(), sd),
        (end.year(), end.month(), ed),
    )
}

/// European 30/360 days: the 31st counts as the 30th at either end.
pub(crate) fn days_euro_30_360(start: NaiveDate, end: NaiveDate) -> i64 {
    days_360(
        (start.year(), start.month(), start.day().min(30)),
        (end.year(), end.month(), end.day().min(30)),
    )
}

/// The coupon date `months` months before `anchor` on `anchor`'s schedule:
/// on `anchor`'s day of the month, or the month's last day when the month is
/// shorter, and on the last day of every month when `anchor` is the last day
/// of its month. Negative `months` count forward.
pub(crate) fn coupon_date(anchor: NaiveDate, months: i32) -> NaiveDate {
    let total = anchor.year() * 12 + anchor.month0() as i32 - months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let last = days_in_month(year, month);
    let day = if is_month_end(anchor) {
        last
    } else {
        anchor.day().min(last)
    };
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid coupon date")
}

/// `date` plus `months` months, on the same day of the month or the month's
/// last day when the month is shorter (EDATE).
pub(crate) fn add_months(date: NaiveDate, months: i32) -> NaiveDate {
    let total = date.year() * 12 + date.month0() as i32 + months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).expect("a valid date")
}

/// The number of periods of `months` months from the coupon date on or
/// before `date` to `anchor` on `anchor`'s schedule: 0 when `date` is
/// `anchor` or later in its first period after it, negative further on.
pub(crate) fn periods_back(anchor: NaiveDate, date: NaiveDate, months: i32) -> i32 {
    let diff = (anchor.year() - date.year()) * 12 + anchor.month() as i32 - date.month() as i32;
    let mut k = diff.div_euclid(months);
    while coupon_date(anchor, k * months) > date {
        k += 1;
    }
    while coupon_date(anchor, (k - 1) * months) <= date {
        k -= 1;
    }
    k
}

/// Settlement's coupon period on maturity's schedule.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Period {
    /// The coupon date on or before settlement (COUPPCD), day 0 of the date
    /// system when it would fall before it.
    pub(crate) pcd: NaiveDate,
    /// The coupon date after settlement (COUPNCD).
    pub(crate) ncd: NaiveDate,
    /// Coupons from settlement to maturity (COUPNUM); `None` when the
    /// previous coupon date fell before day 0, where Excel's count is #NUM!.
    pub(crate) coupons: Option<i32>,
}

impl Period {
    /// COUPNUM, #NUM! where Excel has none.
    pub(crate) fn count(&self) -> Result<i32, ExcelError> {
        self.coupons.ok_or_else(ExcelError::new_num)
    }
}

/// Settlement's coupon period for a security maturing on `maturity`
/// (`settlement < maturity`).
pub(crate) fn period(
    settlement: NaiveDate,
    maturity: NaiveDate,
    frequency: u32,
    system: DateSystem,
) -> Period {
    let months = 12 / frequency as i32;
    let k = periods_back(maturity, settlement, months);
    let pcd = coupon_date(maturity, k * months);
    let ncd = coupon_date(maturity, (k - 1) * months);
    let first = day_zero(system);
    if pcd < first {
        Period {
            pcd: first,
            ncd,
            coupons: None,
        }
    } else {
        Period {
            pcd,
            ncd,
            coupons: Some(k),
        }
    }
}

/// The arguments every COUP* function takes: settlement, maturity,
/// frequency, [basis]. Settlement must be before maturity.
fn coupon_args(
    args: &[ArgumentHandle],
    system: DateSystem,
) -> Result<(NaiveDate, NaiveDate, Period, u32, Basis), ExcelError> {
    let [settlement, maturity, frequency_arg, basis_arg] = numbers(args, [0.0, 0.0, 0.0, 0.0])?;
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    let frequency = frequency(frequency_arg)?;
    let basis = Basis::new(basis_arg)?;
    if settlement >= maturity {
        return Err(ExcelError::new_num());
    }
    Ok((
        settlement,
        maturity,
        period(settlement, maturity, frequency, system),
        frequency,
        basis,
    ))
}

/// COUPDAYS: the days of settlement's coupon period. On actual/actual Excel
/// counts from the previous coupon date to the same day of the month one
/// period later (COUPDAYS(DATE(2024,5,15),DATE(2030,10,31),2,1) is 183, from
/// 2024-04-30 to 2024-10-30, where the coupon dates are 184 days apart), or to
/// the next coupon date once settlement has reached that day. For a maturity
/// after the 28th whose schedule has a February coupon, and for a previous
/// coupon date before day 0, its count follows no rule the probes pinned
/// down: #N/IMPL!, so the workbook is recalculated elsewhere.
fn coupon_days(c: &Coupon) -> Result<f64, ExcelError> {
    if c.basis != Basis::Actual {
        return Ok(c.basis.period_days(&c.period, c.frequency));
    }
    let months = 12 / c.frequency as i32;
    let february = (0..c.frequency as i32)
        .any(|k| (c.maturity.month0() as i32 - k * months).rem_euclid(12) == 1);
    let maturity_month_end = c.maturity.day() > 28 && c.maturity.month() != 2;
    if (february && maturity_month_end) || c.period.coupons.is_none() {
        return Err(ExcelError::new(ExcelErrorKind::NImpl).with_message(
            "COUPDAYS on actual/actual for a maturity after the 28th with a February coupon",
        ));
    }
    let a_period_on = add_months(c.period.pcd, months);
    let end = if a_period_on > c.settlement {
        a_period_on
    } else {
        c.period.ncd
    };
    Ok(actual_days(c.period.pcd, end) as f64)
}

/// A COUP* call: settlement, maturity, settlement's coupon period, frequency
/// and basis.
struct Coupon {
    settlement: NaiveDate,
    maturity: NaiveDate,
    period: Period,
    frequency: u32,
    basis: Basis,
}

/// Evaluates a COUP* function from settlement's coupon period.
fn eval_coupon<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    ctx: &dyn FunctionContext<'b>,
    value: impl FnOnce(Coupon, DateSystem) -> Result<f64, ExcelError>,
) -> Result<CalcValue<'b>, ExcelError> {
    let system = ctx.date_system();
    Ok(number_value(coupon_args(args, system).and_then(
        |(settlement, maturity, period, frequency, basis)| {
            value(
                Coupon {
                    settlement,
                    maturity,
                    period,
                    frequency,
                    basis,
                },
                system,
            )
        },
    )))
}

/// Returns the number of days from the beginning of the coupon period to the
/// settlement date.
///
/// # Remarks
/// - Dates are serials truncated to the day; settlement must be before maturity.
/// - `frequency` is 1, 2 or 4 and `basis` 0 to 4 after truncation, otherwise `#NUM!`.
/// - The coupon dates are counted back from maturity on maturity's day of the month (the
///   month's last day when the month is shorter, every month's last day when maturity is a
///   month end).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/actual"
/// formula: "=COUPDAYBS(DATE(2011,1,25),DATE(2011,11,15),2,1)"
/// expected: 71
/// ```
/// ```yaml,docs
/// related:
///   - COUPDAYS
///   - COUPDAYSNC
///   - COUPPCD
/// ```
#[derive(Debug)]
pub struct CoupdaybsFn;

/// [formualizer-docgen:schema:start]
/// Name: COUPDAYBS
/// Type: CoupdaybsFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: COUPDAYBS(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CoupdaybsFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "COUPDAYBS"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(4)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        eval_coupon(args, ctx, |c, _| {
            Ok(c.basis.days_before(&c.period, c.settlement))
        })
    }
}

/// Returns the number of days in the coupon period that contains the
/// settlement date.
///
/// # Remarks
/// - 360/frequency for bases 0, 2 and 4, 365/frequency for basis 3.
/// - Actual/actual (basis 1) counts from the previous coupon date to the same day of the
///   month one period later, or to the next coupon date once settlement has reached that day.
/// - On actual/actual a maturity after the 28th with a February coupon in its schedule, or a
///   previous coupon date before day 0, is `#N/IMPL!` (Excel's count there is not reproduced).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/actual"
/// formula: "=COUPDAYS(DATE(2011,1,25),DATE(2011,11,15),2,1)"
/// expected: 181
/// ```
/// ```yaml,docs
/// related:
///   - COUPDAYBS
///   - COUPDAYSNC
/// ```
#[derive(Debug)]
pub struct CoupdaysFn;

/// [formualizer-docgen:schema:start]
/// Name: COUPDAYS
/// Type: CoupdaysFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: COUPDAYS(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CoupdaysFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "COUPDAYS"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(4)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        eval_coupon(args, ctx, |c, _| coupon_days(&c))
    }
}

/// Returns the number of days from the settlement date to the next coupon
/// date.
///
/// # Remarks
/// - US 30/360 (basis 0) counts the coupon period with both of its ends moved to the 30th
///   and takes away COUPDAYBS.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/actual"
/// formula: "=COUPDAYSNC(DATE(2011,1,25),DATE(2011,11,15),2,1)"
/// expected: 110
/// ```
/// ```yaml,docs
/// related:
///   - COUPDAYBS
///   - COUPNCD
/// ```
#[derive(Debug)]
pub struct CoupdaysncFn;

/// [formualizer-docgen:schema:start]
/// Name: COUPDAYSNC
/// Type: CoupdaysncFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: COUPDAYSNC(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CoupdaysncFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "COUPDAYSNC"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(4)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        eval_coupon(args, ctx, |c, _| {
            Ok(c.basis.days_after(&c.period, c.settlement))
        })
    }
}

/// Returns the next coupon date after the settlement date, as a serial.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Semiannual coupon"
/// formula: "=COUPNCD(DATE(2011,1,25),DATE(2011,11,15),2,1)"
/// expected: 40678
/// ```
/// ```yaml,docs
/// related:
///   - COUPPCD
///   - COUPNUM
/// ```
#[derive(Debug)]
pub struct CoupncdFn;

/// [formualizer-docgen:schema:start]
/// Name: COUPNCD
/// Type: CoupncdFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: COUPNCD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CoupncdFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "COUPNCD"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(4)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        eval_coupon(args, ctx, |c, system| Ok(serial(c.period.ncd, system)))
    }
}

/// Returns the number of coupons payable between the settlement date and
/// maturity.
///
/// # Remarks
/// - `#NUM!` when the previous coupon date falls before day 0 of the date system (a blank
///   settlement, for one).
///
/// # Examples
/// ```yaml,sandbox
/// title: "Semiannual coupon"
/// formula: "=COUPNUM(DATE(2011,1,25),DATE(2011,11,15),2,1)"
/// expected: 2
/// ```
/// ```yaml,docs
/// related:
///   - COUPNCD
///   - COUPPCD
/// ```
#[derive(Debug)]
pub struct CoupnumFn;

/// [formualizer-docgen:schema:start]
/// Name: COUPNUM
/// Type: CoupnumFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: COUPNUM(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CoupnumFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "COUPNUM"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(4)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        eval_coupon(args, ctx, |c, _| Ok(f64::from(c.period.count()?)))
    }
}

/// Returns the coupon date on or before the settlement date, as a serial.
///
/// # Remarks
/// - A coupon date before day 0 of the date system is 0.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Semiannual coupon"
/// formula: "=COUPPCD(DATE(2011,1,25),DATE(2011,11,15),2,1)"
/// expected: 40497
/// ```
/// ```yaml,docs
/// related:
///   - COUPNCD
///   - COUPDAYBS
/// ```
#[derive(Debug)]
pub struct CouppcdFn;

/// [formualizer-docgen:schema:start]
/// Name: COUPPCD
/// Type: CouppcdFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: COUPPCD(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for CouppcdFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "COUPPCD"
    }
    fn min_args(&self) -> usize {
        3
    }
    fn variadic(&self) -> bool {
        true
    }
    fn arg_schema(&self) -> &'static [ArgSchema] {
        number_schema(4)
    }
    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<CalcValue<'b>, ExcelError> {
        eval_coupon(args, ctx, |c, system| Ok(serial(c.period.pcd, system)))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(CoupdaybsFn));
    crate::function_registry::register_builtin(Arc::new(CoupdaysFn));
    crate::function_registry::register_builtin(Arc::new(CoupdaysncFn));
    crate::function_registry::register_builtin(Arc::new(CoupncdFn));
    crate::function_registry::register_builtin(Arc::new(CoupnumFn));
    crate::function_registry::register_builtin(Arc::new(CouppcdFn));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// US 30/360: a start on the last of February or the 31st is the 30th; an
    /// end on the 31st is the 30th only when the start is then the 30th.
    #[test]
    fn us_30_360_moves_the_ends_as_excel_does() {
        assert_eq!(days_us_30_360(d(2023, 2, 28), d(2023, 8, 31), false), 181);
        assert_eq!(days_us_30_360(d(2023, 2, 28), d(2023, 8, 31), true), 180);
        assert_eq!(days_us_30_360(d(2024, 8, 29), d(2025, 2, 28), true), 181);
        assert_eq!(days_us_30_360(d(2024, 2, 29), d(2024, 5, 1), false), 61);
        assert_eq!(days_euro_30_360(d(2024, 2, 29), d(2024, 8, 31)), 181);
    }

    /// A maturity at a month end puts every coupon at a month end; otherwise
    /// each coupon keeps maturity's day, or the month's last when shorter.
    #[test]
    fn coupon_dates_follow_maturity() {
        assert_eq!(coupon_date(d(2030, 8, 31), 6 * 13), d(2024, 2, 29));
        assert_eq!(coupon_date(d(2030, 2, 28), 6), d(2029, 8, 31));
        assert_eq!(coupon_date(d(2030, 8, 30), 6 * 13), d(2024, 2, 29));
        assert_eq!(coupon_date(d(2030, 8, 30), 6 * 12), d(2024, 8, 30));
        let p = period(d(2024, 8, 30), d(2030, 8, 30), 2, DateSystem::Excel1900);
        assert_eq!(
            (p.pcd, p.ncd, p.coupons),
            (d(2024, 8, 30), d(2025, 2, 28), Some(12))
        );
    }

    /// Serial 0 is the first day; a coupon date before it is read as it and
    /// leaves no coupon count.
    #[test]
    fn coupon_dates_before_day_zero() {
        let p = period(d(1899, 12, 31), d(2011, 11, 15), 2, DateSystem::Excel1900);
        assert_eq!(p.pcd, d(1899, 12, 31));
        assert_eq!(p.ncd, d(1900, 5, 15));
        assert!(p.count().is_err());
    }

    /// Actual days are serial differences: Excel's 29 February 1900 counts.
    #[test]
    fn actual_days_count_the_1900_leap_day() {
        assert_eq!(actual_days(d(1899, 12, 31), d(1900, 5, 15)), 136);
        assert_eq!(actual_days(d(1900, 3, 1), d(1900, 5, 15)), 75);
        assert_eq!(actual_days(d(2024, 2, 28), d(2024, 3, 1)), 2);
    }
}
