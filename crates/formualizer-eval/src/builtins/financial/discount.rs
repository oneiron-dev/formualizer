//! Discounted and interest-at-maturity securities: DISC, INTRATE, RECEIVED,
//! PRICEDISC, YIELDDISC, PRICEMAT, YIELDMAT, as Excel for Windows 16.0.20430
//! computes them (ops/excel-finance-probe-20261006.md). Their year fractions
//! are YEARFRAC's, from `coupon.rs`.

use super::coupon::{Basis, date, number_schema, number_value, numbers};
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::ExcelError;
use formualizer_macros::func_caps;

/// settlement, maturity, two amounts and [basis]: the year fraction from
/// settlement to maturity (settlement before maturity) and the amounts.
fn term_args(
    args: &[ArgumentHandle],
    ctx: &dyn FunctionContext<'_>,
) -> Result<(f64, f64, f64), ExcelError> {
    let system = ctx.date_system();
    let [settlement, maturity, first, second, basis] = numbers(args, [0.0; 5])?;
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    let basis = Basis::new(basis)?;
    if settlement >= maturity {
        return Err(ExcelError::new_num());
    }
    Ok((basis.year_fraction(settlement, maturity), first, second))
}

/// Evaluates a settlement/maturity security from its year fraction and two
/// amounts.
fn eval_term<'b>(
    args: &[ArgumentHandle<'_, 'b>],
    ctx: &dyn FunctionContext<'b>,
    value: impl FnOnce(f64, f64, f64) -> Result<f64, ExcelError>,
) -> Result<CalcValue<'b>, ExcelError> {
    Ok(number_value(term_args(args, ctx).and_then(
        |(years, first, second)| value(years, first, second),
    )))
}

/// Returns the discount rate of a security: `(redemption - pr) / redemption`
/// over the year fraction from settlement to maturity.
///
/// # Remarks
/// - `pr` and `redemption` must be positive; settlement before maturity.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/actual"
/// formula: "=DISC(DATE(2018,1,7),DATE(2048,1,1),97.975,100,1)"
/// expected: 0.0006924310378460001
/// ```
/// ```yaml,docs
/// related:
///   - PRICEDISC
///   - YIELDDISC
/// ```
#[derive(Debug)]
pub struct DiscFn;

/// [formualizer-docgen:schema:start]
/// Name: DISC
/// Type: DiscFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: DISC(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for DiscFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "DISC"
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
        eval_term(args, ctx, |years, price, redemption| {
            if price <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok((redemption - price) / redemption / years)
        })
    }
}

/// Returns the interest rate of a fully invested security:
/// `(redemption - investment) / investment` over the year fraction.
///
/// # Remarks
/// - `investment` and `redemption` must be positive; settlement before maturity.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/360"
/// formula: "=INTRATE(DATE(2008,2,15),DATE(2008,5,15),1000000,1014420,2)"
/// expected: 0.05768
/// ```
/// ```yaml,docs
/// related:
///   - RECEIVED
///   - DISC
/// ```
#[derive(Debug)]
pub struct IntrateFn;

/// [formualizer-docgen:schema:start]
/// Name: INTRATE
/// Type: IntrateFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: INTRATE(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for IntrateFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "INTRATE"
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
        eval_term(args, ctx, |years, investment, redemption| {
            if investment <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok((redemption - investment) / investment / years)
        })
    }
}

/// Returns the amount received at maturity for a fully invested security:
/// `investment / (1 - discount * year fraction)`.
///
/// # Remarks
/// - `investment` and `discount` must be positive; settlement before maturity.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/360"
/// formula: "=RECEIVED(DATE(2008,2,15),DATE(2008,5,15),1000000,0.0575,2)"
/// expected: 1014584.6544071021
/// ```
/// ```yaml,docs
/// related:
///   - INTRATE
/// ```
#[derive(Debug)]
pub struct ReceivedFn;

/// [formualizer-docgen:schema:start]
/// Name: RECEIVED
/// Type: ReceivedFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: RECEIVED(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for ReceivedFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "RECEIVED"
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
        eval_term(args, ctx, |years, investment, discount| {
            let factor = 1.0 - discount * years;
            if investment <= 0.0 || discount <= 0.0 || factor <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok(investment / factor)
        })
    }
}

/// Returns the price per 100 face value of a discounted security:
/// `redemption - discount * redemption * year fraction`.
///
/// # Remarks
/// - `discount` and `redemption` must be positive; settlement before maturity.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/360"
/// formula: "=PRICEDISC(DATE(2008,2,16),DATE(2008,3,1),0.0525,100,2)"
/// expected: 99.79583333333333
/// ```
/// ```yaml,docs
/// related:
///   - DISC
///   - YIELDDISC
/// ```
#[derive(Debug)]
pub struct PricediscFn;

/// [formualizer-docgen:schema:start]
/// Name: PRICEDISC
/// Type: PricediscFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: PRICEDISC(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for PricediscFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "PRICEDISC"
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
        eval_term(args, ctx, |years, discount, redemption| {
            if discount <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok(redemption - discount * redemption * years)
        })
    }
}

/// Returns the annual yield of a discounted security:
/// `(redemption - pr) / pr` over the year fraction.
///
/// # Remarks
/// - `pr` and `redemption` must be positive; settlement before maturity.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Actual/360"
/// formula: "=YIELDDISC(DATE(2008,2,16),DATE(2008,3,1),99.795,100,2)"
/// expected: 0.05282257198685834
/// ```
/// ```yaml,docs
/// related:
///   - PRICEDISC
///   - DISC
/// ```
#[derive(Debug)]
pub struct YielddiscFn;

/// [formualizer-docgen:schema:start]
/// Name: YIELDDISC
/// Type: YielddiscFn
/// Min args: 4
/// Max args: variadic
/// Variadic: true
/// Signature: YIELDDISC(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for YielddiscFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "YIELDDISC"
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
        eval_term(args, ctx, |years, price, redemption| {
            if price <= 0.0 || redemption <= 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok((redemption - price) / price / years)
        })
    }
}

/// settlement, maturity, issue, two rates and [basis] of a security paying
/// interest at maturity: the year fractions from issue to maturity, issue to
/// settlement and settlement to maturity. Their days come from one count:
/// the last is the first less the second (US 30/360 counts from issue), over
/// the year of issue to settlement.
fn maturity_args(
    args: &[ArgumentHandle],
    ctx: &dyn FunctionContext<'_>,
) -> Result<([f64; 3], f64, f64), ExcelError> {
    let system = ctx.date_system();
    let [settlement, maturity, issue, rate, second, basis] = numbers(args, [0.0; 6])?;
    let settlement = date(settlement, system)?;
    let maturity = date(maturity, system)?;
    let issue = date(issue, system)?;
    let basis = Basis::new(basis)?;
    if settlement >= maturity || issue >= settlement || rate < 0.0 {
        return Err(ExcelError::new_num());
    }
    let year = basis.year_days(issue, settlement);
    let dim = basis.days(issue, maturity);
    let a = basis.days(issue, settlement);
    Ok(([dim / year, a / year, (dim - a) / year], rate, second))
}

/// Returns the price per 100 face value of a security that pays interest at
/// maturity.
///
/// # Remarks
/// - Settlement must be after issue and before maturity; `rate` and `yld` at least 0.
///
/// # Examples
/// ```yaml,sandbox
/// title: "30/360"
/// formula: "=PRICEMAT(DATE(2008,2,15),DATE(2008,4,13),DATE(2007,11,11),0.061,0.061,0)"
/// expected: 99.98449887555694
/// ```
/// ```yaml,docs
/// related:
///   - YIELDMAT
///   - ACCRINTM
/// ```
#[derive(Debug)]
pub struct PricematFn;

/// [formualizer-docgen:schema:start]
/// Name: PRICEMAT
/// Type: PricematFn
/// Min args: 5
/// Max args: variadic
/// Variadic: true
/// Signature: PRICEMAT(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for PricematFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "PRICEMAT"
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
        let value = maturity_args(args, ctx).and_then(|([dim, a, dsm], rate, yld)| {
            if yld < 0.0 {
                return Err(ExcelError::new_num());
            }
            Ok((100.0 + dim * rate * 100.0) / (1.0 + dsm * yld) - a * rate * 100.0)
        });
        Ok(number_value(value))
    }
}

/// Returns the annual yield of a security that pays interest at maturity.
///
/// # Remarks
/// - Settlement must be after issue and before maturity; `rate` at least 0, `pr` positive.
///
/// # Examples
/// ```yaml,sandbox
/// title: "30/360"
/// formula: "=YIELDMAT(DATE(2008,3,15),DATE(2008,11,3),DATE(2007,11,11),0.0625,100.0123,0)"
/// expected: 0.06095433369153867
/// ```
/// ```yaml,docs
/// related:
///   - PRICEMAT
/// ```
#[derive(Debug)]
pub struct YieldmatFn;

/// [formualizer-docgen:schema:start]
/// Name: YIELDMAT
/// Type: YieldmatFn
/// Min args: 5
/// Max args: variadic
/// Variadic: true
/// Signature: YIELDMAT(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar, arg4: number@scalar, arg5: number@scalar, arg6...: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg4{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg5{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg6{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for YieldmatFn {
    func_caps!(PURE);
    fn name(&self) -> &'static str {
        "YIELDMAT"
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
        let value = maturity_args(args, ctx).and_then(|([dim, a, dsm], rate, price)| {
            if price <= 0.0 {
                return Err(ExcelError::new_num());
            }
            let cost = price / 100.0 + a * rate;
            Ok(((1.0 + dim * rate) - cost) / cost / dsm)
        });
        Ok(number_value(value))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(DiscFn));
    crate::function_registry::register_builtin(Arc::new(IntrateFn));
    crate::function_registry::register_builtin(Arc::new(ReceivedFn));
    crate::function_registry::register_builtin(Arc::new(PricediscFn));
    crate::function_registry::register_builtin(Arc::new(YielddiscFn));
    crate::function_registry::register_builtin(Arc::new(PricematFn));
    crate::function_registry::register_builtin(Arc::new(YieldmatFn));
}
