//! ADDRESS function - creates a cell reference as text
//!
//! Excel semantics:
//! - ADDRESS(row_num, column_num, [abs_num], [a1], [sheet_text])
//! - abs_num: 1 = absolute ($A$1), 2 = abs row (A$1), 3 = abs col ($A1), 4 = relative (A1)
//! - a1: TRUE = A1 style, FALSE = R1C1 style
//! - sheet_text: optional sheet name to include

use crate::args::{ArgSchema, CoercionPolicy, ShapeKind};
use crate::function::Function;
use crate::traits::{ArgumentHandle, FunctionContext};
use formualizer_common::{
    ArgKind, DateSystem, ExcelError, ExcelErrorKind, LiteralValue, col_letters_from_1based,
};
use formualizer_macros::func_caps;

/// Convert a column number to Excel letter notation (1 = A, 27 = AA, etc.)
fn column_to_letters(col: u32) -> String {
    col_letters_from_1based(col).unwrap_or_default()
}

#[derive(Debug)]
pub struct AddressFn;

/// ADDRESS's row, column or abs_num as a whole number, read like INDEX's
/// positions (`coercion::snapped_whole_number`) from what a number argument
/// takes: a number, a logical (`ADDRESS(TRUE,TRUE)` is `$A$1`), a date (its
/// serial in the workbook's date `system`), a blank cell (0) or text that
/// reads as a number (`ADDRESS(1,1,"1")` is `$A$1`). `None` for anything else.
fn whole_position(value: &LiteralValue, system: DateSystem) -> Option<i64> {
    value
        .as_serial_number_for(system)
        .or_else(|| crate::coercion::to_number_argument(value).ok())
        .map(|n| crate::coercion::snapped_whole_number(n) as i64)
}

/// Returns a cell reference as text from row and column numbers.
///
/// `ADDRESS` can emit either A1 or R1C1 notation and optionally prefix the address with a
/// sheet name.
///
/// # Remarks
/// - `abs_num` defaults to `1` (`$A$1`) and supports values `1..4`.
/// - `a1` defaults to `TRUE`; `FALSE` returns R1C1-style text.
/// - Valid row range is `1..1048576`; valid column range is `1..16384`.
/// - Out-of-range row/column values or invalid `abs_num` return `#VALUE!`.
/// - A row, column or `abs_num` within 2^-22 below a whole number is that number, as in Excel
///   (`ADDRESS(2-1E-7,1)` is `$A$2`); other fractions truncate (`ADDRESS(2.9999,1)` is `$A$2`).
///   Text that reads as a number converts first (`ADDRESS("2",1)` is `$A$2`), a logical is 1 or
///   0 (`ADDRESS(TRUE,TRUE)` is `$A$1`) and a blank cell is 0 (as `abs_num`, `#VALUE!`).
/// - `a1` is read as a logical: a blank cell is `FALSE`, any number but 0 is `TRUE`, text
///   `"TRUE"` or `"FALSE"` is that logical and other text (numeric text too) is `#VALUE!`.
/// - An empty `abs_num` or `a1` argument (`ADDRESS(1,1,)`, `ADDRESS(1,1,,)`) is the default.
/// - If `sheet_text` contains spaces or special characters, it is quoted.
///
/// # Examples
/// ```yaml,sandbox
/// title: "Absolute A1 reference"
/// formula: '=ADDRESS(2,3)'
/// expected: "$C$2"
/// ```
///
/// ```yaml,sandbox
/// title: "Relative R1C1 reference with sheet"
/// formula: '=ADDRESS(5,3,4,FALSE,"Data Sheet")'
/// expected: "'Data Sheet'!R[5]C[3]"
/// ```
///
/// ```yaml,docs
/// related:
///   - INDEX
///   - OFFSET
///   - INDIRECT
/// faq:
///   - q: "What happens when abs_num is outside 1..4?"
///     a: "ADDRESS returns #VALUE!; only 1 (absolute), 2 (absolute row), 3 (absolute column), and 4 (relative) are valid."
///   - q: "Why does sheet_text sometimes add single quotes?"
///     a: "Sheet names containing spaces or special characters are quoted and internal apostrophes are escaped to keep a valid reference string."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: ADDRESS
/// Type: AddressFn
/// Min args: 2
/// Max args: 5
/// Variadic: false
/// Signature: ADDRESS(arg1: number@scalar, arg2: number@scalar, arg3?: number@scalar, arg4?: logical@scalar, arg5?: text@scalar)
/// Arg schema: arg1{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberStrict,max=None,repeating=None,default=false}; arg2{kinds=number,required=true,shape=scalar,by_ref=false,coercion=NumberStrict,max=None,repeating=None,default=false}; arg3{kinds=number,required=false,shape=scalar,by_ref=false,coercion=NumberStrict,max=None,repeating=None,default=true}; arg4{kinds=logical,required=false,shape=scalar,by_ref=false,coercion=Logical,max=None,repeating=None,default=true}; arg5{kinds=text,required=false,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE
/// [formualizer-docgen:schema:end]
impl Function for AddressFn {
    fn name(&self) -> &'static str {
        "ADDRESS"
    }

    fn min_args(&self) -> usize {
        2
    }

    func_caps!(PURE);

    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![
                // row_num (required, a number; numeric text converts)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Number],
                    required: true,
                    by_ref: false,
                    shape: ShapeKind::Scalar,
                    coercion: CoercionPolicy::NumberLenientText,
                    max: None,
                    repeating: None,
                    default: None,
                },
                // column_num (required, a number; numeric text converts)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Number],
                    required: true,
                    by_ref: false,
                    shape: ShapeKind::Scalar,
                    coercion: CoercionPolicy::NumberLenientText,
                    max: None,
                    repeating: None,
                    default: None,
                },
                // abs_num (optional, default 1; numeric text converts)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Number],
                    required: false,
                    by_ref: false,
                    shape: ShapeKind::Scalar,
                    coercion: CoercionPolicy::NumberLenientText,
                    max: None,
                    repeating: None,
                    default: Some(LiteralValue::Int(1)),
                },
                // a1 (optional, default TRUE; read in eval, where a date is a
                // number and so TRUE)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Logical],
                    required: false,
                    by_ref: false,
                    shape: ShapeKind::Scalar,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: Some(LiteralValue::Boolean(true)),
                },
                // sheet_text (optional)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Text],
                    required: false,
                    by_ref: false,
                    shape: ShapeKind::Scalar,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
            ]
        });
        &SCHEMA
    }

    fn eval<'a, 'b, 'c>(
        &self,
        args: &'c [ArgumentHandle<'a, 'b>],
        ctx: &dyn FunctionContext<'b>,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let system = ctx.date_system();
        if args.len() < 2 {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Get row number
        let row_val = args[0].value()?.into_literal();
        if let LiteralValue::Error(e) = row_val {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
        }
        let row = match whole_position(&row_val, system) {
            Some(row) => row,
            None => {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new(ExcelErrorKind::Value),
                )));
            }
        };

        if !(1..=1_048_576).contains(&row) {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Get column number
        let col_val = args[1].value()?.into_literal();
        if let LiteralValue::Error(e) = col_val {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
        }
        let col = match whole_position(&col_val, system) {
            Some(col) => col,
            None => {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new(ExcelErrorKind::Value),
                )));
            }
        };

        if !(1..=16384).contains(&col) {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Get abs_num (default 1 = absolute). An empty argument, ADDRESS(1,1,),
        // is the default; a blank cell is 0, #VALUE!.
        let abs_num = if args.len() > 2 && !args[2].is_omitted() {
            let abs_val = args[2].value()?.into_literal();
            if let LiteralValue::Error(e) = abs_val {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
            }
            whole_position(&abs_val, system).unwrap_or(0)
        } else {
            1
        };

        if !(1..=4).contains(&abs_num) {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Value),
            )));
        }

        // Get a1 (default TRUE = A1 notation). An empty argument, ADDRESS(1,1,,),
        // is the default; otherwise a logical: a blank cell is FALSE, a number is
        // TRUE unless 0, text "TRUE" or "FALSE" is that logical and other text,
        // numeric text included, is #VALUE!.
        let a1_style = if args.len() > 3 && !args[3].is_omitted() {
            let a1_val = args[3].value()?.into_literal();
            if let LiteralValue::Error(e) = a1_val {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
            }
            // A number, logical or date (its serial in the workbook's date
            // system) is TRUE unless 0; blank and text read as a logical.
            let a1 = match a1_val.as_serial_number_for(system) {
                Some(n) => Ok(n != 0.0),
                None => crate::coercion::to_logical(&a1_val),
            };
            match a1 {
                Ok(a1) => a1,
                Err(_) => {
                    return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                        ExcelError::new(ExcelErrorKind::Value),
                    )));
                }
            }
        } else {
            true
        };

        // Get sheet name (optional). An empty argument, ADDRESS(1,1,1,TRUE,), is
        // no sheet; a blank cell is the empty name, so ADDRESS(1,1,1,TRUE,A1)
        // with A1 blank is "!$A$1", as `""` is.
        let sheet_name = if args.len() > 4 && !args[4].is_omitted() {
            let sheet_val = args[4].value()?.into_literal();
            if let LiteralValue::Error(e) = sheet_val {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(e)));
            }
            match sheet_val {
                LiteralValue::Text(s) => Some(s),
                LiteralValue::Empty => Some(String::new()),
                _ => None,
            }
        } else {
            None
        };

        // Build the address
        let address = if a1_style {
            // A1 notation
            let col_letters = column_to_letters(col as u32);
            let (col_abs, row_abs) = match abs_num {
                1 => (true, true),   // $A$1
                2 => (false, true),  // A$1
                3 => (true, false),  // $A1
                4 => (false, false), // A1
                _ => (true, true),
            };

            let col_str = if col_abs {
                format!("${col_letters}")
            } else {
                col_letters
            };
            let row_str = if row_abs {
                format!("${row}")
            } else {
                row.to_string()
            };
            format!("{col_str}{row_str}")
        } else {
            // R1C1 notation
            let (col_abs, row_abs) = match abs_num {
                1 => (true, true),
                2 => (false, true),
                3 => (true, false),
                4 => (false, false),
                _ => (true, true),
            };

            let row_str = if row_abs {
                format!("R{row}")
            } else {
                format!("R[{row}]")
            };
            let col_str = if col_abs {
                format!("C{col}")
            } else {
                format!("C[{col}]")
            };
            format!("{row_str}{col_str}")
        };

        // Add sheet name if provided
        let final_address = if let Some(sheet) = sheet_name {
            // Quote sheet name if it contains spaces or special characters
            if sheet.contains(' ') || sheet.contains('!') || sheet.contains('\'') {
                format!("'{}'!{address}", sheet.replace('\'', "''"))
            } else {
                format!("{sheet}!{address}")
            }
        } else {
            address
        };

        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Text(
            final_address,
        )))
    }
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
    fn test_column_to_letters() {
        assert_eq!(column_to_letters(1), "A");
        assert_eq!(column_to_letters(26), "Z");
        assert_eq!(column_to_letters(27), "AA");
        assert_eq!(column_to_letters(52), "AZ");
        assert_eq!(column_to_letters(53), "BA");
        assert_eq!(column_to_letters(702), "ZZ");
        assert_eq!(column_to_letters(703), "AAA");
    }

    #[test]
    fn address_basic() {
        let wb = TestWorkbook::new().with_function(Arc::new(AddressFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ADDRESS").unwrap();

        // ADDRESS(2, 3) -> "$C$2" (default absolute)
        let two = lit(LiteralValue::Int(2));
        let three = lit(LiteralValue::Int(3));

        let args = vec![
            ArgumentHandle::new(&two, &ctx),
            ArgumentHandle::new(&three, &ctx),
        ];

        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(result, LiteralValue::Text("$C$2".into()));
    }

    #[test]
    fn address_abs_variations() {
        let wb = TestWorkbook::new().with_function(Arc::new(AddressFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ADDRESS").unwrap();

        let row = lit(LiteralValue::Int(5));
        let col = lit(LiteralValue::Int(4)); // Column D

        // abs_num = 1: $D$5
        let abs1 = lit(LiteralValue::Int(1));
        let args1 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs1, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args1, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("$D$5".into())
        );

        // abs_num = 2: D$5
        let abs2 = lit(LiteralValue::Int(2));
        let args2 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs2, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args2, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("D$5".into())
        );

        // abs_num = 3: $D5
        let abs3 = lit(LiteralValue::Int(3));
        let args3 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs3, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args3, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("$D5".into())
        );

        // abs_num = 4: D5
        let abs4 = lit(LiteralValue::Int(4));
        let args4 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs4, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args4, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("D5".into())
        );
    }

    #[test]
    fn address_with_sheet() {
        let wb = TestWorkbook::new().with_function(Arc::new(AddressFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ADDRESS").unwrap();

        let row = lit(LiteralValue::Int(1));
        let col = lit(LiteralValue::Int(1));
        let abs_num = lit(LiteralValue::Int(1));
        let a1_style = lit(LiteralValue::Boolean(true));

        // Simple sheet name
        let sheet1 = lit(LiteralValue::Text("Sheet1".into()));
        let args1 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs_num, &ctx),
            ArgumentHandle::new(&a1_style, &ctx),
            ArgumentHandle::new(&sheet1, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args1, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("Sheet1!$A$1".into())
        );

        // Sheet name with spaces (needs quoting)
        let sheet2 = lit(LiteralValue::Text("My Sheet".into()));
        let args2 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs_num, &ctx),
            ArgumentHandle::new(&a1_style, &ctx),
            ArgumentHandle::new(&sheet2, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args2, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("'My Sheet'!$A$1".into())
        );
    }

    #[test]
    fn address_r1c1_style() {
        let wb = TestWorkbook::new().with_function(Arc::new(AddressFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ADDRESS").unwrap();

        let row = lit(LiteralValue::Int(5));
        let col = lit(LiteralValue::Int(3));
        let abs1 = lit(LiteralValue::Int(1));
        let r1c1 = lit(LiteralValue::Boolean(false));

        // R1C1 absolute
        let args = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs1, &ctx),
            ArgumentHandle::new(&r1c1, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args, &ctx.function_context(None))
                .unwrap()
                .into_literal(),
            LiteralValue::Text("R5C3".into())
        );

        // R1C1 relative
        let abs4 = lit(LiteralValue::Int(4));
        let args2 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&col, &ctx),
            ArgumentHandle::new(&abs4, &ctx),
            ArgumentHandle::new(&r1c1, &ctx),
        ];
        assert_eq!(
            f.dispatch(&args2, &ctx.function_context(None)).unwrap(),
            LiteralValue::Text("R[5]C[3]".into())
        );
    }

    #[test]
    fn address_edge_cases() {
        let wb = TestWorkbook::new().with_function(Arc::new(AddressFn));
        let ctx = wb.interpreter();
        let f = ctx.context.get_function("", "ADDRESS").unwrap();

        // Row too large
        let big_row = lit(LiteralValue::Int(1_048_577));
        let col = lit(LiteralValue::Int(1));
        let args = vec![
            ArgumentHandle::new(&big_row, &ctx),
            ArgumentHandle::new(&col, &ctx),
        ];
        let result = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert!(matches!(result, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value));

        // Column too large
        let row = lit(LiteralValue::Int(1));
        let big_col = lit(LiteralValue::Int(16385));
        let args2 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&big_col, &ctx),
        ];
        let result2 = f
            .dispatch(&args2, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert!(matches!(result2, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value));

        // Invalid abs_num
        let abs5 = lit(LiteralValue::Int(5));
        let normal_col = lit(LiteralValue::Int(1));
        let args3 = vec![
            ArgumentHandle::new(&row, &ctx),
            ArgumentHandle::new(&normal_col, &ctx),
            ArgumentHandle::new(&abs5, &ctx),
        ];
        let result3 = f
            .dispatch(&args3, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert!(matches!(result3, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value));
    }
}
