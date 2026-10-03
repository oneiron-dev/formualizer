//! DATE and TIME functions

use super::serial::create_date_normalized;
use crate::args::ArgSchema;
use crate::function::Function;
use crate::traits::{ArgumentHandle, FunctionContext};
use chrono::NaiveTime;
use formualizer_common::{ExcelError, LiteralValue, date_to_serial_for, time_to_fraction};
use formualizer_macros::func_caps;

/// A DATE or TIME argument as the whole number `whole` makes of it: DATE's
/// year, month and day snap within 2^-22 below a whole number
/// (`coercion::snapped_whole_number`, so `DATE(2026,2-1E-7,1)` is February),
/// TIME's parts truncate.
fn coerce_to_int(arg: &ArgumentHandle, whole: fn(f64) -> f64) -> Result<i32, ExcelError> {
    let v = arg.value()?.into_literal();
    match v {
        // Saturate rather than wrap, so a value outside i32 stays out of the
        // 16-bit argument ranges below instead of wrapping back into them.
        LiteralValue::Int(i) => Ok(i.clamp(i32::MIN.into(), i32::MAX.into()) as i32),
        LiteralValue::Number(f) => Ok(whole(f) as i32),
        // Text coerces as VALUE() does: numeric text, or date/time text such as
        // "Oct 21" read as its serial (year-less dates fall in the clock's year).
        LiteralValue::Text(_) => crate::coercion::to_serial_lenient_in_year(
            &v,
            arg.date_system(),
            Some(arg.current_year()),
        )
        .map(|f| whole(f) as i32)
        .map_err(|_| {
            ExcelError::new_value().with_message("DATE/TIME argument is not a valid number")
        }),
        LiteralValue::Boolean(b) => Ok(if b { 1 } else { 0 }),
        LiteralValue::Empty => Ok(0),
        LiteralValue::Error(e) => Err(e),
        _ => Err(ExcelError::new_value()
            .with_message("DATE/TIME expects numeric or text-numeric arguments")),
    }
}

/// DATE's month argument (after truncation) must lie in this range, or DATE is
/// #NUM! before any roll-over. Excel reads it as a signed 16-bit integer; the
/// references that give the edges put the top at 32766, one below `i16::MAX`.
const DATE_MONTH_RANGE: std::ops::RangeInclusive<i32> = -32768..=32766;

/// TIME's hour, minute and second (after truncation) must each lie in this
/// range, or TIME is #NUM!. Microsoft documents each as at most 32767; Excel
/// reads them as signed 16-bit integers, so a negative component that keeps
/// the total time non-negative (`TIME(1,-1,0)`) still rolls back.
const TIME_ARG_RANGE: std::ops::RangeInclusive<i32> = i16::MIN as i32..=i16::MAX as i32;

fn num_error<'b>() -> Result<crate::traits::CalcValue<'b>, ExcelError> {
    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
        ExcelError::new_num(),
    )))
}

/// Returns the serial number for a calendar date from year, month, and day.
///
/// `DATE` normalizes out-of-range month and day values to produce a valid calendar date.
///
/// # Remarks
/// - Years in the range `0..=1899` are interpreted as `1900..=3799` for Excel compatibility.
/// - A year, month or day within 2^-22 below a whole number is that number, as in Excel
///   (`DATE(2026,2-1E-7,1)` is February 1); other fractions round down (`DATE(2026,2.9999,1)`
///   is February 1, `DATE(2026,-0.5,1)` is month -1, November 1 2025).
/// - The returned serial is date-system aware and depends on the active workbook system (`1900` vs `1904`).
/// - In the `1900` system, serial mapping preserves Excel's historical phantom `1900-02-29` behavior.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Build a standard date"
/// formula: "=DATE(2024, 1, 15)"
/// expected: 45306
/// ```
///
/// ```yaml,sandbox
/// title: "Normalize overflowing month input"
/// formula: "=DATE(2024, 13, 5)"
/// expected: 45662
/// ```
///
/// ```yaml,docs
/// related:
///   - DATEVALUE
///   - YEAR
///   - EDATE
/// faq:
///   - q: "Does DATE follow the workbook 1900/1904 date system?"
///     a: "Yes. DATE emits a serial in the active workbook date system, so the same calendar date can map to different serials across 1900 vs 1904 mode."
/// ```
#[derive(Debug)]
pub struct DateFn;

/// [formualizer-docgen:schema:start]
/// Name: DATE
/// Type: DateFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: DATE(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for DateFn {
    fn propagate_format(
        &self,
        _result: &crate::traits::CalcValue<'_>,
    ) -> Option<crate::format::FormatId> {
        Some(crate::format::FormatId::DATE)
    }

    func_caps!(PURE);

    fn name(&self) -> &'static str {
        "DATE"
    }

    fn min_args(&self) -> usize {
        3
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        // DATE(year, month, day) – all scalar, numeric lenient (allow text numbers)
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
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let whole = crate::coercion::snapped_whole_number;
        let year = coerce_to_int(&args[0], whole)?;
        let month = coerce_to_int(&args[1], whole)?;
        let day = coerce_to_int(&args[2], whole)?;

        // A month outside Excel's 16-bit range is #NUM!, even when the rolled-over
        // date would be valid. Date text used as the month (a serial such as
        // 44211 for "1/15/2021") lands here.
        if !DATE_MONTH_RANGE.contains(&month) {
            return num_error();
        }

        // Excel interprets years 0-1899 as 1900-3799
        let adjusted_year = if (0..=1899).contains(&year) {
            year + 1900
        } else {
            year
        };

        // Excel's dates run from serial 0 to 9999-12-31; anything else is #NUM!.
        if !(0..=9999).contains(&adjusted_year) {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_num(),
            )));
        }
        let date = create_date_normalized(adjusted_year, month, day)?;
        let serial = date_to_serial_for(ctx.date_system(), &date);
        if chrono::Datelike::year(&date) > 9999 || serial < 0.0 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new_num(),
            )));
        }

        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(
            serial,
        )))
    }
}

/// Returns the fractional-day serial for a time built from hour, minute, and second.
///
/// `TIME` normalizes overflowing and negative components by wrapping across day boundaries.
///
/// # Remarks
/// - The result is always in the range `0.0..1.0` and represents only a time-of-day fraction.
/// - Values are normalized like Excel (for example, `25` hours becomes `01:00:00`).
/// - Time fractions are date-system independent because they do not include a date component.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Create noon"
/// formula: "=TIME(12, 0, 0)"
/// expected: 0.5
/// ```
///
/// ```yaml,sandbox
/// title: "Wrap overflowing hour"
/// formula: "=TIME(25, 0, 0)"
/// expected: 0.0416666667
/// ```
///
/// ```yaml,docs
/// related:
///   - TIMEVALUE
///   - HOUR
///   - NOW
/// faq:
///   - q: "Can TIME return values greater than 1 day?"
///     a: "No. TIME wraps overflow and always returns a fraction in [0,1), so extra days are discarded."
/// ```
#[derive(Debug)]
pub struct TimeFn;

/// [formualizer-docgen:schema:start]
/// Name: TIME
/// Type: TimeFn
/// Min args: 3
/// Max args: 3
/// Variadic: false
/// Signature: TIME(arg1: number@scalar, arg2: number@scalar, arg3: number@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}; arg3{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberLenientText,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for TimeFn {
    fn propagate_format(
        &self,
        _result: &crate::traits::CalcValue<'_>,
    ) -> Option<crate::format::FormatId> {
        Some(crate::format::FormatId::TIME)
    }

    func_caps!(PURE);

    fn name(&self) -> &'static str {
        "TIME"
    }

    fn min_args(&self) -> usize {
        3
    }

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use std::sync::LazyLock;
        // TIME(hour, minute, second) – scalar numeric lenient
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
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let hour = coerce_to_int(&args[0], f64::trunc)?;
        let minute = coerce_to_int(&args[1], f64::trunc)?;
        let second = coerce_to_int(&args[2], f64::trunc)?;

        // An argument outside the 16-bit range is #NUM! (date text such as
        // "1/15/2021" is a serial far above 32767). The bound also keeps the
        // total below inside i32: 32767 * 3661 < 2^31.
        if [hour, minute, second]
            .iter()
            .any(|v| !TIME_ARG_RANGE.contains(v))
        {
            return num_error();
        }

        // Excel normalizes time values
        let total_seconds = hour * 3600 + minute * 60 + second;

        // Handle negative time by wrapping
        let normalized_seconds = if total_seconds < 0 {
            let days_back = (-total_seconds - 1) / 86400 + 1;
            total_seconds + days_back * 86400
        } else {
            total_seconds
        };

        // Get just the time portion (modulo full days)
        let time_seconds = normalized_seconds % 86400;
        let hours = (time_seconds / 3600) as u32;
        let minutes = ((time_seconds % 3600) / 60) as u32;
        let seconds = (time_seconds % 60) as u32;

        match NaiveTime::from_hms_opt(hours, minutes, seconds) {
            Some(time) => {
                let fraction = time_to_fraction(&time);
                Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(
                    fraction,
                )))
            }
            None => Err(ExcelError::new_num()),
        }
    }
}

pub fn register_builtins() {
    use std::sync::Arc;
    crate::function_registry::register_builtin(Arc::new(DateFn));
    crate::function_registry::register_builtin(Arc::new(TimeFn));
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
    fn test_date_basic() {
        let wb = TestWorkbook::new().with_function(Arc::new(DateFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "DATE").unwrap();

        // DATE(2024, 1, 15)
        let year = lit(LiteralValue::Int(2024));
        let month = lit(LiteralValue::Int(1));
        let day = lit(LiteralValue::Int(15));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&year, &ctx),
                    ArgumentHandle::new(&month, &ctx),
                    ArgumentHandle::new(&day, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        match result {
            LiteralValue::Number(n) => {
                // Should be a positive serial number
                assert!(n > 0.0);
                // Should be an integer (no time component)
                assert_eq!(n.trunc(), n);
            }
            _ => panic!("DATE should return a number"),
        }
    }

    #[test]
    fn test_date_normalization() {
        let wb = TestWorkbook::new().with_function(Arc::new(DateFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "DATE").unwrap();

        // DATE(2024, 13, 5) should normalize to 2025-01-05
        let year = lit(LiteralValue::Int(2024));
        let month = lit(LiteralValue::Int(13));
        let day = lit(LiteralValue::Int(5));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&year, &ctx),
                    ArgumentHandle::new(&month, &ctx),
                    ArgumentHandle::new(&day, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap();

        // Just verify it returns a valid number
        assert!(matches!(result.into_literal(), LiteralValue::Number(_)));
    }

    #[test]
    fn test_date_system_1900_vs_1904() {
        use crate::engine::{Engine, EvalConfig};
        use crate::interpreter::Interpreter;

        // Engine with default 1900 system
        let cfg_1900 = EvalConfig::default();
        let eng_1900 = Engine::new(TestWorkbook::new(), cfg_1900.clone());
        let interp_1900 = Interpreter::new(&eng_1900, "Sheet1");
        let f = interp_1900.context.get_function("", "DATE").unwrap();
        let y = lit(LiteralValue::Int(1904));
        let m = lit(LiteralValue::Int(1));
        let d = lit(LiteralValue::Int(1));
        let args = [
            crate::traits::ArgumentHandle::new(&y, &interp_1900),
            crate::traits::ArgumentHandle::new(&m, &interp_1900),
            crate::traits::ArgumentHandle::new(&d, &interp_1900),
        ];
        let v1900 = f
            .dispatch(&args, &interp_1900.function_context(None))
            .unwrap()
            .into_literal();

        // Engine with 1904 system
        let cfg_1904 = EvalConfig {
            date_system: crate::engine::DateSystem::Excel1904,
            ..Default::default()
        };
        let eng_1904 = Engine::new(TestWorkbook::new(), cfg_1904);
        let interp_1904 = Interpreter::new(&eng_1904, "Sheet1");
        let f2 = interp_1904.context.get_function("", "DATE").unwrap();
        let args2 = [
            crate::traits::ArgumentHandle::new(&y, &interp_1904),
            crate::traits::ArgumentHandle::new(&m, &interp_1904),
            crate::traits::ArgumentHandle::new(&d, &interp_1904),
        ];
        let v1904 = f2
            .dispatch(&args2, &interp_1904.function_context(None))
            .unwrap()
            .into_literal();

        match (v1900, v1904) {
            (LiteralValue::Number(a), LiteralValue::Number(b)) => {
                // 1904-01-01 is 1462 in 1900 system, 0 in 1904 system
                assert!((a - 1462.0).abs() < 1e-9, "expected 1462, got {a}");
                assert!(b.abs() < 1e-9, "expected 0, got {b}");
            }
            other => panic!("Unexpected results: {other:?}"),
        }
    }

    #[test]
    fn test_time_basic() {
        let wb = TestWorkbook::new().with_function(Arc::new(TimeFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TIME").unwrap();

        // TIME(12, 0, 0) = noon = 0.5
        let hour = lit(LiteralValue::Int(12));
        let minute = lit(LiteralValue::Int(0));
        let second = lit(LiteralValue::Int(0));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&hour, &ctx),
                    ArgumentHandle::new(&minute, &ctx),
                    ArgumentHandle::new(&second, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        match result {
            LiteralValue::Number(n) => {
                assert!((n - 0.5).abs() < 1e-10);
            }
            _ => panic!("TIME should return a number"),
        }
    }

    #[test]
    fn test_time_normalization() {
        let wb = TestWorkbook::new().with_function(Arc::new(TimeFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "TIME").unwrap();

        // TIME(25, 0, 0) = 1:00 AM next day = 1/24
        let hour = lit(LiteralValue::Int(25));
        let minute = lit(LiteralValue::Int(0));
        let second = lit(LiteralValue::Int(0));

        let result = f
            .dispatch(
                &[
                    ArgumentHandle::new(&hour, &ctx),
                    ArgumentHandle::new(&minute, &ctx),
                    ArgumentHandle::new(&second, &ctx),
                ],
                &ctx.function_context(None),
            )
            .unwrap()
            .into_literal();

        match result {
            LiteralValue::Number(n) => {
                // Should wrap to 1:00 AM = 1/24
                assert!((n - 1.0 / 24.0).abs() < 1e-10);
            }
            _ => panic!("TIME should return a number"),
        }
    }

    fn eval_formula(formula: &str) -> LiteralValue {
        use crate::engine::{Engine, EvalConfig};
        use crate::interpreter::Interpreter;
        use formualizer_parse::parser::parse;

        let wb = TestWorkbook::new()
            .with_function(Arc::new(DateFn))
            .with_function(Arc::new(TimeFn))
            .with_function(Arc::new(crate::builtins::info::IsErrorFn))
            .with_function(Arc::new(crate::builtins::logical_ext::IfErrorFn));
        let engine = Engine::new(wb, EvalConfig::default());
        let interpreter = Interpreter::new(&engine, "Sheet1");
        match interpreter.evaluate_ast(&parse(formula).expect("formula should parse")) {
            Ok(v) => v.into_literal(),
            Err(e) => LiteralValue::Error(e),
        }
    }

    fn is_num_error(v: &LiteralValue) -> bool {
        matches!(v, LiteralValue::Error(e) if e.kind == formualizer_common::ExcelErrorKind::Num)
    }

    /// DATE and TIME coerce text arguments as VALUE() does, so date/time text
    /// becomes its serial instead of #VALUE!.
    #[test]
    fn date_time_text_arguments_coerce_like_value() {
        // "Oct 21" is 21 October of the clock's year, a serial far above
        // 9999 in any recent year, so as the year it is #NUM!, not #VALUE!.
        let v = eval_formula("=DATE(\"Oct 21\",1,1)");
        assert!(is_num_error(&v), "DATE(\"Oct 21\",1,1) gave {v:?}");
        // "1/15/1950" is serial 18278: 18277 days after 2000-01-01.
        assert_eq!(
            eval_formula("=DATE(2000,1,\"1/15/1950\")"),
            LiteralValue::Number(36526.0 + 18277.0)
        );
        // "12:00" is 0.5, which truncates to day 0: 2020-12-31.
        assert_eq!(
            eval_formula("=DATE(2021,1,\"12:00\")"),
            LiteralValue::Number(44196.0)
        );
        // "1/30/1900" is serial 30, read as 30 minutes.
        match eval_formula("=TIME(0,\"1/30/1900\",0)") {
            LiteralValue::Number(n) => assert!((n - 30.0 / 1440.0).abs() < 1e-12),
            other => panic!("TIME with date-text minutes gave {other:?}"),
        }
    }

    /// Numeric text keeps working and text that is neither a number nor a
    /// date stays #VALUE!.
    #[test]
    fn date_time_non_date_text_arguments_unchanged() {
        assert_eq!(
            eval_formula("=DATE(\"2021\",\"3\",\"15\")"),
            eval_formula("=DATE(2021,3,15)")
        );
        match eval_formula("=TIME(\"12\",\"30\",\"0\")") {
            LiteralValue::Number(n) => assert!((n - 12.5 / 24.0).abs() < 1e-12),
            other => panic!("TIME with numeric text gave {other:?}"),
        }
        for formula in [
            "=DATE(2021,\"abc\",1)",
            "=DATE(2021,\"\",1)",
            "=TIME(\"noon\",0,0)",
        ] {
            let v = eval_formula(formula);
            assert!(
                matches!(&v, LiteralValue::Error(e) if e.kind == formualizer_common::ExcelErrorKind::Value),
                "{formula} gave {v:?}"
            );
        }
    }

    fn serial_1900(y: i32, m: u32, d: u32) -> f64 {
        date_to_serial_for(
            formualizer_common::DateSystem::Excel1900,
            &chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap(),
        )
    }

    fn assert_time(formula: &str, seconds: f64) {
        match eval_formula(formula) {
            LiteralValue::Number(n) => assert!(
                (n - seconds / 86400.0).abs() < 1e-12,
                "{formula} gave {n}, expected {}",
                seconds / 86400.0
            ),
            other => panic!("{formula} gave {other:?}"),
        }
    }

    /// Date text read as a DATE month or a TIME component is a serial far above
    /// the 16-bit argument range, so it is #NUM! (an error, as in Excel) and
    /// error-agnostic callers such as ISERROR and IFERROR see an error.
    #[test]
    fn date_time_date_text_outside_16_bit_range_is_num_error() {
        for formula in [
            "=DATE(2021,\"Oct 21\",1)",
            "=DATE(2021,\"1/15/2021\",1)",
            "=DATE(2000,\"12/31/9999\",1)",
            "=TIME(0,\"1/15/2021\",0)",
            "=TIME(\"1/15/2021\",0,0)",
            "=TIME(0,0,\"1/15/2021\")",
            "=TIME(\"12/31/9999\",0,0)",
        ] {
            let v = eval_formula(formula);
            assert!(is_num_error(&v), "{formula} gave {v:?}");
        }
        assert_eq!(
            eval_formula("=ISERROR(DATE(2021,\"Oct 21\",1))"),
            LiteralValue::Boolean(true)
        );
        assert_eq!(
            eval_formula("=IFERROR(DATE(2021,\"Oct 21\",1),\"bad\")"),
            LiteralValue::Text("bad".into())
        );
        assert_eq!(
            eval_formula("=IFERROR(TIME(0,\"1/15/2021\",0),\"bad\")"),
            LiteralValue::Text("bad".into())
        );
        assert_eq!(
            eval_formula("=ISERROR(TIME(\"1/15/2021\",0,0))"),
            LiteralValue::Boolean(true)
        );
        // Date text whose serial is inside the range still rolls over:
        // "1/30/1900" is 30, so month 30 of 2000 is June 2002.
        assert_eq!(
            eval_formula("=DATE(2000,\"1/30/1900\",1)"),
            LiteralValue::Number(serial_1900(2002, 6, 1))
        );
    }

    /// A DATE month outside -32768..=32766 (after truncation) is #NUM!, even
    /// when the rolled-over date would be a valid serial.
    #[test]
    fn date_month_outside_16_bit_range_is_num_error() {
        for formula in [
            "=DATE(21,32767,1)",
            "=DATE(21,46316,0)",
            "=DATE(2000,32767.5,1)",
            "=DATE(5000,-32769,1)",
            "=DATE(2000,1E10,1)",
            "=DATE(9999,-1E10,1)",
            "=DATE(21,\"32768\",1)",
        ] {
            let v = eval_formula(formula);
            assert!(is_num_error(&v), "{formula} gave {v:?}");
        }
        // Months at the edges still roll over: 2000 + 32765 months is June
        // 4730, 5000 - 32769 months is April 2269.
        assert_eq!(
            eval_formula("=DATE(2000,32766,1)"),
            LiteralValue::Number(serial_1900(4730, 6, 1))
        );
        assert_eq!(
            eval_formula("=DATE(2000,32766.9,1)"),
            LiteralValue::Number(serial_1900(4730, 6, 1))
        );
        assert_eq!(
            eval_formula("=DATE(5000,-32768,1)"),
            LiteralValue::Number(serial_1900(2269, 4, 1))
        );
        // An argument error still comes before the range check.
        let v = eval_formula("=DATE(21,32768,\"x\")");
        assert!(
            matches!(&v, LiteralValue::Error(e) if e.kind == formualizer_common::ExcelErrorKind::Value),
            "DATE(21,32768,\"x\") gave {v:?}"
        );
    }

    /// A TIME argument above 32767 or below -32768 (after truncation) is #NUM!,
    /// and huge values no longer overflow the seconds total.
    #[test]
    fn time_argument_outside_16_bit_range_is_num_error() {
        for formula in [
            "=TIME(32768,0,0)",
            "=TIME(0,32768,0)",
            "=TIME(0,0,32768)",
            "=TIME(600000,0,0)",
            "=TIME(1E10,0,0)",
            "=TIME(0,1E300,0)",
            "=TIME(-32769,0,0)",
            "=TIME(\"32768\",0,0)",
        ] {
            let v = eval_formula(formula);
            assert!(is_num_error(&v), "{formula} gave {v:?}");
        }
        // Up to 32767 each component still rolls over into the time of day,
        // and a negative component that keeps the total positive rolls back.
        assert_time("=TIME(32767,0,0)", 7.0 * 3600.0);
        assert_time("=TIME(0,32767,0)", (18.0 * 60.0 + 7.0) * 60.0);
        assert_time("=TIME(0,0,32767)", 32767.0);
        assert_time("=TIME(32767.9,0,0)", 7.0 * 3600.0);
        assert_time("=TIME(1,-1,0)", 59.0 * 60.0);
    }

    /// An integer argument beyond i32 saturates instead of wrapping back into
    /// the 16-bit range: 2^32 + 1 would otherwise wrap to 1.
    #[test]
    fn date_time_huge_integer_arguments_do_not_wrap() {
        let wb = TestWorkbook::new()
            .with_function(Arc::new(DateFn))
            .with_function(Arc::new(TimeFn));
        let ctx = wb.interpreter();
        let huge = lit(LiteralValue::Int((1_i64 << 32) + 1));
        let one = lit(LiteralValue::Int(1));
        let year = lit(LiteralValue::Int(2000));
        for (name, args) in [
            ("DATE", [&year, &huge, &one]),
            ("TIME", [&huge, &one, &one]),
            ("TIME", [&one, &one, &huge]),
        ] {
            let f = ctx.context.get_function("", name).unwrap();
            let handles: Vec<_> = args.iter().map(|a| ArgumentHandle::new(a, &ctx)).collect();
            let v = f
                .dispatch(&handles, &ctx.function_context(None))
                .unwrap()
                .into_literal();
            assert!(is_num_error(&v), "{name} with 2^32 + 1 gave {v:?}");
        }
    }
}
