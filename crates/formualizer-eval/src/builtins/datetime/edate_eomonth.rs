//! EDATE and EOMONTH functions for date arithmetic

use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, FunctionContext};
use chrono::NaiveDate;
use formualizer_common::{
    DateSystem, ExcelDateParts, ExcelError, LiteralValue, date_to_serial_for,
    try_serial_to_display_date_parts_for,
};
use formualizer_macros::func_caps;

fn coerce_to_serial(arg: &ArgumentHandle, system: DateSystem) -> Result<f64, ExcelError> {
    let v = arg.value()?.into_literal();
    if let LiteralValue::Error(e) = v {
        return Err(e);
    }
    crate::coercion::to_serial_lenient_in_year(&v, system, Some(arg.current_year())).map_err(|_| {
        ExcelError::new_value()
            .with_message("EDATE/EOMONTH expects numeric, date, or text-numeric arguments")
    })
}

fn coerce_to_int(arg: &ArgumentHandle) -> Result<i32, ExcelError> {
    let v = arg.value()?.into_literal();
    if let LiteralValue::Error(e) = v {
        return Err(e);
    }
    crate::coercion::to_number_argument(&v)
        .map(|f| f.trunc() as i32)
        .map_err(|_| {
            ExcelError::new_value()
                .with_message("EDATE/EOMONTH months argument is not a valid number")
        })
}

/// A month of Excel's calendar: the serial of its first day and its length.
struct ExcelMonth {
    first_serial: f64,
    days: u32,
}

impl ExcelMonth {
    /// Serial of `day` in this month. Day 0 is the day before the 1st, which
    /// only arises from a start date of January 0, 1900 (serial 0).
    fn serial_of_day(&self, day: u32) -> f64 {
        self.first_serial + day as f64 - 1.0
    }

    fn last_day_serial(&self) -> f64 {
        self.serial_of_day(self.days)
    }
}

/// Excel calendar fields of a start-date serial (fraction dropped).
///
/// In the 1900 system serial 0 is January 0, 1900 and serial 60 is
/// February 29, 1900, so month arithmetic starts from those fields rather
/// than from the nearest real date.
fn start_date_parts(system: DateSystem, serial: f64) -> Result<ExcelDateParts, ExcelError> {
    try_serial_to_display_date_parts_for(system, serial)
}

/// The month `months` after the start date's month, in Excel's calendar.
///
/// Month lengths are measured in serials, so February 1900 has 29 days in the
/// 1900 system (the leap-year compatibility day, serial 60). A month before
/// the date system's first year or after 9999 is not an Excel date: #NUM!.
fn shifted_month(
    system: DateSystem,
    start: ExcelDateParts,
    months: i32,
) -> Result<ExcelMonth, ExcelError> {
    let index = start.year as i64 * 12 + (start.month as i64 - 1) + months as i64;
    let year = index.div_euclid(12);
    let month = (index.rem_euclid(12) + 1) as u32;
    let first_year = match system {
        DateSystem::Excel1900 => 1900,
        DateSystem::Excel1904 => 1904,
    };
    if !(first_year..=9999).contains(&year) {
        return Err(ExcelError::new_num());
    }
    let year = year as i32;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_serial = first_of_month_serial(system, year, month)?;
    let next_serial = first_of_month_serial(system, next_year, next_month)?;
    Ok(ExcelMonth {
        first_serial,
        days: (next_serial - first_serial) as u32,
    })
}

fn first_of_month_serial(system: DateSystem, year: i32, month: u32) -> Result<f64, ExcelError> {
    NaiveDate::from_ymd_opt(year, month, 1)
        .map(|date| date_to_serial_for(system, &date))
        .ok_or_else(ExcelError::new_num)
}

/// Returns the serial date offset by a whole number of months from a start date.
///
/// # Remarks
/// - `months` is truncated to an integer before calculation.
/// - If the target month has fewer days, the day is clamped to that month's last valid day.
/// - Serials are interpreted and emitted with the workbook's date system (Excel 1900 or Excel 1904).
/// - In the 1900 system the start date uses Excel's calendar: serial 0 is January 0, 1900 and
///   February 1900 has 29 days (serial 60 is February 29, 1900).
/// - A result before the date system's first year or after 9999 returns `#NUM!`.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Add months to first-of-month date"
/// formula: "=EDATE(44927, 3)"
/// expected: 45017
/// ```
///
/// ```yaml,sandbox
/// title: "Clamp month-end overflow"
/// formula: "=EDATE(45322, 1)"
/// expected: 45351
/// ```
///
/// ```yaml,docs
/// related:
///   - EOMONTH
///   - DATE
///   - YEARFRAC
/// faq:
///   - q: "What happens when the start day does not exist in the target month?"
///     a: "EDATE clamps to the last valid day of the target month (for example Jan 31 + 1 month becomes Feb month-end)."
/// ```
#[derive(Debug)]
pub struct EdateFn;

/// [formualizer-docgen:schema:start]
/// Name: EDATE
/// Type: EdateFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: EDATE(arg1: number@scalar, arg2: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for EdateFn {
    func_caps!(PURE);

    fn name(&self) -> &'static str {
        "EDATE"
    }

    fn min_args(&self) -> usize {
        2
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static TWO: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            vec![
                // start_date serial (numeric lenient)
                ArgSchema::number_lenient_scalar(),
                // months offset (numeric lenient)
                ArgSchema::number_lenient_scalar(),
            ]
        });
        &TWO[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let system = ctx.date_system();
        let start_serial = coerce_to_serial(&args[0], system)?;
        let months = coerce_to_int(&args[1])?;

        let start = start_date_parts(system, start_serial)?;
        let target = shifted_month(system, start, months)?;

        // Keep the same day, clamped to the target month's last day.
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(
            target.serial_of_day(start.day.min(target.days)),
        )))
    }
}

/// Returns the serial for the last day of the month at a month offset from a start date.
///
/// # Remarks
/// - `months` is truncated to an integer before offset calculation.
/// - The returned date is always the month-end date for the target month.
/// - Serials are interpreted and returned using the workbook's date system (Excel 1900 or Excel 1904).
/// - In the 1900 system the start date uses Excel's calendar: serial 0 is January 0, 1900 and
///   February 1900 ends on the 29th (serial 60).
/// - A target month before the date system's first year or after 9999 returns `#NUM!`.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Get end of current month"
/// formula: "=EOMONTH(44927, 0)"
/// expected: 44957
/// ```
///
/// ```yaml,sandbox
/// title: "Get end of month two months ahead"
/// formula: "=EOMONTH(45322, 2)"
/// expected: 45382
/// ```
///
/// ```yaml,docs
/// related:
///   - EDATE
///   - DATE
///   - DAY
/// faq:
///   - q: "Does EOMONTH always return a month-end date?"
///     a: "Yes. Regardless of the start day, EOMONTH returns the final calendar day of the target month after offset."
/// ```
#[derive(Debug)]
pub struct EomonthFn;

/// [formualizer-docgen:schema:start]
/// Name: EOMONTH
/// Type: EomonthFn
/// Min args: 2
/// Max args: 2
/// Variadic: false
/// Signature: EOMONTH(arg1: number@scalar, arg2: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for EomonthFn {
    func_caps!(PURE);

    fn name(&self) -> &'static str {
        "EOMONTH"
    }

    fn min_args(&self) -> usize {
        2
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        static TWO: LazyLock<Vec<ArgSchema>> = LazyLock::new(|| {
            vec![
                ArgSchema::number_lenient_scalar(),
                ArgSchema::number_lenient_scalar(),
            ]
        });
        &TWO[..]
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let system = ctx.date_system();
        let start_serial = coerce_to_serial(&args[0], system)?;
        let months = coerce_to_int(&args[1])?;

        let start = start_date_parts(system, start_serial)?;
        let target = shifted_month(system, start, months)?;

        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(
            target.last_day_serial(),
        )))
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(EdateFn));
    crate::function_registry::register_builtin(Arc::new(EomonthFn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use formualizer_parse::parser::{ASTNode, ASTNodeType};
    use std::sync::Arc;

    fn lit(v: LiteralValue) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(v), None)
    }

    #[test]
    fn test_edate_basic() {
        let wb = TestWorkbook::new().with_function(Arc::new(EdateFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "EDATE").unwrap();

        // Test adding months
        // Use a known date serial (e.g., 44927 = 2023-01-01)
        let start = lit(LiteralValue::Number(44927.0));
        let months = lit(LiteralValue::Int(3));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&start, &ctx),
                    ArgumentHandle::new(&months, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        // Should return a date 3 months later
        assert!(matches!(result, LiteralValue::Number(_)));
    }

    #[test]
    fn test_edate_negative_months() {
        let wb = TestWorkbook::new().with_function(Arc::new(EdateFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "EDATE").unwrap();

        // Test subtracting months
        let start = lit(LiteralValue::Number(44927.0)); // 2023-01-01
        let months = lit(LiteralValue::Int(-2));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&start, &ctx),
                    ArgumentHandle::new(&months, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        // Should return a date 2 months earlier
        assert!(matches!(result, LiteralValue::Number(_)));
    }

    #[test]
    fn test_eomonth_basic() {
        let wb = TestWorkbook::new().with_function(Arc::new(EomonthFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "EOMONTH").unwrap();

        // Test end of month
        let start = lit(LiteralValue::Number(44927.0)); // 2023-01-01
        let months = lit(LiteralValue::Int(0));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&start, &ctx),
                    ArgumentHandle::new(&months, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        // Should return Jan 31, 2023
        assert!(matches!(result, LiteralValue::Number(_)));
    }

    #[test]
    fn test_eomonth_february() {
        let wb = TestWorkbook::new().with_function(Arc::new(EomonthFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "EOMONTH").unwrap();

        // Test February (checking leap year handling)
        let start = lit(LiteralValue::Number(44927.0)); // 2023-01-01
        let months = lit(LiteralValue::Int(1)); // Move to February

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&start, &ctx),
                    ArgumentHandle::new(&months, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        // Should return Feb 28, 2023 (not a leap year)
        assert!(matches!(result, LiteralValue::Number(_)));
    }

    fn eval_month_offset_formula(system: crate::engine::DateSystem, formula: &str) -> LiteralValue {
        use crate::engine::{Engine, EvalConfig};
        use crate::interpreter::Interpreter;
        use formualizer_parse::parser::parse;

        let wb = TestWorkbook::new()
            .with_function(Arc::new(EdateFn))
            .with_function(Arc::new(EomonthFn));
        let engine = Engine::new(wb, EvalConfig::default().with_date_system(system));
        let interpreter = Interpreter::new(&engine, "Sheet1");
        match interpreter.evaluate_ast(&parse(formula).expect("formula should parse")) {
            Ok(value) => value.into_literal(),
            Err(e) => LiteralValue::Error(e),
        }
    }

    fn assert_numbers(system: crate::engine::DateSystem, cases: &[(&str, f64)]) {
        for &(formula, want) in cases {
            assert_eq!(
                eval_month_offset_formula(system, formula),
                LiteralValue::Number(want),
                "{formula} under {system:?}"
            );
        }
    }

    /// EOMONTH reads the start serial with Excel's 1900 calendar fields:
    /// serial 0 is January 0, 1900 (so its month is January 1900), and
    /// February 1900 has 29 days, ending on serial 60 (KB 214326).
    #[test]
    fn eomonth_uses_excel_1900_calendar_fields() {
        use crate::engine::DateSystem;

        assert_numbers(
            DateSystem::Excel1900,
            &[
                ("=EOMONTH(0,0)", 31.0),
                ("=EOMONTH(0.75,0)", 31.0),
                ("=EOMONTH(0,1)", 60.0),
                ("=EOMONTH(0,2)", 91.0),
                ("=EOMONTH(0,12)", 397.0),
                // A blank start cell is serial 0.
                ("=EOMONTH(Z99,0)", 31.0),
                ("=EOMONTH(Z99,1)", 60.0),
                ("=EOMONTH(Z99,11)", 366.0),
                // February 1900 ends on the 29th.
                ("=EOMONTH(32,0)", 60.0),
                ("=EOMONTH(59,0)", 60.0),
                ("=EOMONTH(60,0)", 60.0),
                ("=EOMONTH(31,1)", 60.0),
                ("=EOMONTH(60,-1)", 31.0),
                ("=EOMONTH(60,1)", 91.0),
                ("=EOMONTH(60,12)", 425.0),
                // Real dates either side are unchanged.
                ("=EOMONTH(1,0)", 31.0),
                ("=EOMONTH(61,0)", 91.0),
                ("=EOMONTH(444,-3)", 366.0),
                ("=EOMONTH(444,3)", 547.0),
            ],
        );
    }

    /// EDATE keeps the start date's Excel calendar day and clamps it to the
    /// target month's length, where February 1900 has 29 days.
    #[test]
    fn edate_uses_excel_1900_calendar_fields() {
        use crate::engine::DateSystem;

        assert_numbers(
            DateSystem::Excel1900,
            &[
                // February 29, 1900 is a start date in its own right.
                ("=EDATE(60,0)", 60.0),
                ("=EDATE(60.5,0)", 60.0),
                ("=EDATE(60,-1)", 29.0),
                ("=EDATE(60,1)", 89.0),
                ("=EDATE(60,12)", 425.0),
                // Day 29 or later in January lands on February 29, 1900.
                ("=EDATE(29,1)", 60.0),
                ("=EDATE(31,1)", 60.0),
                ("=EDATE(59,0)", 59.0),
                ("=EDATE(59,1)", 88.0),
                ("=EDATE(61,-1)", 32.0),
                // January 0, 1900 shifts to the day before each month's 1st.
                ("=EDATE(0,0)", 0.0),
                ("=EDATE(Z99,0)", 0.0),
                ("=EDATE(0,1)", 31.0),
                ("=EDATE(0,2)", 60.0),
                ("=EDATE(0,12)", 366.0),
                ("=EDATE(2.4,1)", 33.0),
            ],
        );
    }

    /// A shifted month before the date system's first year or after 9999 is
    /// not an Excel date, so EDATE and EOMONTH return #NUM!.
    #[test]
    fn edate_eomonth_outside_excel_calendar_are_num() {
        use crate::engine::DateSystem;
        use formualizer_common::ExcelErrorKind;

        for (system, formula) in [
            (DateSystem::Excel1900, "=EOMONTH(0,-1)"),
            (DateSystem::Excel1900, "=EOMONTH(1,-1)"),
            (DateSystem::Excel1900, "=EDATE(0,-1)"),
            (DateSystem::Excel1900, "=EDATE(1,-1)"),
            (DateSystem::Excel1900, "=EDATE(31,-1)"),
            (DateSystem::Excel1900, "=EDATE(2958465,1)"),
            (DateSystem::Excel1900, "=EOMONTH(2958465,1)"),
            (DateSystem::Excel1900, "=EDATE(-1,0)"),
            (DateSystem::Excel1904, "=EOMONTH(0,-1)"),
            (DateSystem::Excel1904, "=EDATE(0,-1)"),
        ] {
            let got = eval_month_offset_formula(system, formula);
            assert!(
                matches!(&got, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Num),
                "{formula} under {system:?}: {got:?}"
            );
        }

        assert_numbers(
            DateSystem::Excel1900,
            &[
                ("=EOMONTH(2958465,0)", 2958465.0),
                ("=EDATE(2958465,0)", 2958465.0),
            ],
        );
    }

    /// The 1904 system has no pseudo-dates: serial 0 is 1904-01-01.
    #[test]
    fn edate_eomonth_1904_serial_zero_is_a_real_date() {
        use crate::engine::DateSystem;

        assert_numbers(
            DateSystem::Excel1904,
            &[
                ("=EOMONTH(0,0)", 30.0),
                ("=EOMONTH(0,1)", 59.0),
                ("=EDATE(0,0)", 0.0),
                ("=EDATE(0,1)", 31.0),
            ],
        );
    }

    /// EDATE round-trips serial -> date -> shifted date -> serial, so the
    /// workbook date system must be used on both ends.
    #[test]
    fn edate_follows_workbook_date_system_1900_and_1904() {
        use crate::engine::DateSystem;
        use formualizer_common::date_to_serial_for;

        // 2023-01-31 + 1 month clamps to 2023-02-28 in either date system.
        let start = chrono::NaiveDate::from_ymd_opt(2023, 1, 31).unwrap();
        let expected_date = chrono::NaiveDate::from_ymd_opt(2023, 2, 28).unwrap();

        for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
            let start_serial = date_to_serial_for(system, &start);
            assert_eq!(
                eval_month_offset_formula(system, &format!("=EDATE({start_serial},1)")),
                LiteralValue::Number(date_to_serial_for(system, &expected_date)),
                "EDATE under {system:?}"
            );
        }
    }

    #[test]
    fn eomonth_follows_workbook_date_system_1900_and_1904() {
        use crate::engine::DateSystem;
        use formualizer_common::date_to_serial_for;

        // 2024-02-15 with a zero offset is the leap-year month end 2024-02-29.
        let start = chrono::NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
        let expected_date = chrono::NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();

        for system in [DateSystem::Excel1900, DateSystem::Excel1904] {
            let start_serial = date_to_serial_for(system, &start);
            assert_eq!(
                eval_month_offset_formula(system, &format!("=EOMONTH({start_serial},0)")),
                LiteralValue::Number(date_to_serial_for(system, &expected_date)),
                "EOMONTH under {system:?}"
            );
        }
    }
}
