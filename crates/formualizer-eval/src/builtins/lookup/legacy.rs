//! Legacy LOOKUP function (vector and array forms).
//!
//! Excel's classic `LOOKUP` always performs approximate matching (largest value
//! less-than-or-equal to the lookup value) against data that must be sorted in
//! ascending order.  Two calling conventions exist:
//!
//! **Vector form** – `LOOKUP(lookup_value, lookup_vector, [result_vector])`
//!   Searches `lookup_vector` (a single row or column) and returns the
//!   corresponding element from `result_vector`.  If `result_vector` is
//!   omitted the match is returned from `lookup_vector` itself.
//!
//! **Array form** – `LOOKUP(lookup_value, array)`
//!   When given a 2-D array with no result vector:
//!   * If width > height  → search the first *row*, return from the last *row*.
//!   * Otherwise          → search the first *column*, return from the last *column*.

use super::lookup_utils::{SearchedVector, excel_approximate_search, reference_extent};
use crate::args::{ArgSchema, CoercionPolicy, ShapeKind};
use crate::engine::DateSystem;
use crate::function::Function;
use crate::traits::{ArgumentHandle, CalcValue, FunctionContext};
use formualizer_common::{ArgKind, ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_macros::func_caps;

/// Approximate match (largest value <= needle) over a lookup vector of `len`
/// cells, `slice` being the ones read from it. LOOKUP runs the same bisection
/// as MATCH and VLOOKUP ([`excel_approximate_search`]): inclusive bounds, a
/// floor midpoint, an exact hit kept over the entries probed after it, and
/// unsorted data searched rather than rejected. Errors, blanks and entries of
/// another type are skipped, so `LOOKUP(2,1/(cond),result)` finds the last
/// position where `cond` holds.
fn approx_match_ascending(
    slice: &[LiteralValue],
    needle: &LiteralValue,
    len: usize,
    date_system: DateSystem,
) -> Option<usize> {
    let searched = SearchedVector::new(slice, needle, date_system).ok()?;
    excel_approximate_search(&searched, len, needle, false)
}

/// Searches for a value and returns a corresponding value from another range.
///
/// `LOOKUP` always performs approximate matching against ascending-sorted data.
///
/// # Remarks
/// - **Vector form** `LOOKUP(value, lookup_vec, result_vec)`: searches `lookup_vec`,
///   returns corresponding position from `result_vec`.
/// - **Array form** `LOOKUP(value, array)`: if width > height searches first row
///   and returns from last row; otherwise searches first column and returns from
///   last column.
/// - Data should be sorted ascending. Like Excel, LOOKUP bisects without checking
///   the order, so unsorted data gives whichever entry the search reaches.
/// - Returns `#N/A` when the lookup value is smaller than every value in the
///   search range.
///
/// # Examples
/// ```excel
/// =LOOKUP(2,A1:A3,B1:B3)
/// ```
///
/// ```yaml,sandbox
/// title: "Vector form exact hit"
/// grid:
///   A1: 1
///   A2: 2
///   A3: 3
///   B1: "a"
///   B2: "b"
///   B3: "c"
/// formula: '=LOOKUP(2,A1:A3,B1:B3)'
/// expected: "b"
/// ```
///
/// ```yaml,sandbox
/// title: "Vector form approximate"
/// grid:
///   A1: 1
///   A2: 2
///   A3: 3
///   A4: 4
///   A5: 5
/// formula: '=LOOKUP(3.5,A1:A5)'
/// expected: 3
/// ```
///
/// ```yaml,docs
/// related:
///   - VLOOKUP
///   - HLOOKUP
///   - MATCH
/// faq:
///   - q: "Does LOOKUP support exact matching?"
///     a: "No. LOOKUP always performs approximate matching (largest <= lookup value)."
///   - q: "What happens with unsorted data?"
///     a: "LOOKUP runs Excel's binary search without checking the order, as MATCH and VLOOKUP do, so the result follows the search path and may be a wrong entry or #N/A."
/// ```
/// [formualizer-docgen:schema:start]
/// Name: LOOKUP
/// Type: LookupFn
/// Min args: 2
/// Max args: 3
/// Variadic: false
/// Signature: LOOKUP(arg1: any@scalar, arg2: any@range, arg3?: any@range)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg3{kinds=any,required=false,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, LOOKUP
/// [formualizer-docgen:schema:end]
#[derive(Debug)]
pub struct LookupFn;
/// [formualizer-docgen:schema:start]
/// Name: LOOKUP
/// Type: LookupFn
/// Min args: 2
/// Max args: 3
/// Variadic: false
/// Signature: LOOKUP(arg1: any@scalar, arg2: any@range, arg3?: any@range)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg2{kinds=any,required=true,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}; arg3{kinds=any,required=false,shape=range,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, LOOKUP
/// [formualizer-docgen:schema:end]
impl Function for LookupFn {
    fn name(&self) -> &'static str {
        "LOOKUP"
    }
    fn min_args(&self) -> usize {
        2
    }
    func_caps!(PURE, LOOKUP);
    fn arg_schema(&self) -> &'static [ArgSchema] {
        use once_cell::sync::Lazy;
        static SCHEMA: Lazy<Vec<ArgSchema>> = Lazy::new(|| {
            vec![
                // lookup_value (any scalar)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Any],
                    required: true,
                    by_ref: false,
                    shape: ShapeKind::Scalar,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
                // lookup_vector / array (range)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Any],
                    required: true,
                    by_ref: false,
                    shape: ShapeKind::Range,
                    coercion: CoercionPolicy::None,
                    max: None,
                    repeating: None,
                    default: None,
                },
                // result_vector (optional range)
                ArgSchema {
                    kinds: smallvec::smallvec![ArgKind::Any],
                    required: false,
                    by_ref: false,
                    shape: ShapeKind::Range,
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
    ) -> Result<CalcValue<'b>, ExcelError> {
        if args.len() < 2 {
            return Ok(CalcValue::Scalar(LiteralValue::Error(ExcelError::new(
                ExcelErrorKind::Na,
            ))));
        }

        let lookup_value = args[0].value()?.into_literal();
        if let LiteralValue::Error(e) = lookup_value {
            return Ok(CalcValue::Scalar(LiteralValue::Error(e)));
        }

        let has_result_vector = args.len() >= 3;

        // --- Materialise lookup vector / array ---
        let (lookup_data, written) = materialise_range(&args[1], ctx)?;
        let (l_rows, l_cols) = dims(&lookup_data);
        // The result vector is resolved before the search, so an error passed in
        // its place is the result whether or not the lookup value is found.
        let result_data = if has_result_vector {
            Some(materialise_range(&args[2], ctx)?)
        } else {
            None
        };

        // Determine search orientation and build the search slice.
        let (search_vec, is_row_search) = if has_result_vector {
            // Vector form: lookup_data must be 1-D
            flatten_1d(&lookup_data, l_rows, l_cols)
        } else if l_rows == 1 && l_cols == 1 {
            // Single cell – trivially a column search
            (vec![lookup_data[0][0].clone()], false)
        } else if l_cols > l_rows {
            // Array form: wider than tall → search first row
            (lookup_data[0].clone(), true)
        } else {
            // Array form: tall or square → search first column
            (
                lookup_data.iter().map(|r| r[0].clone()).collect::<Vec<_>>(),
                false,
            )
        };

        // Approximate match – largest <= needle, searched over the vector as
        // written: a whole column is 1,048,576 rows though only its used rows
        // were read.
        let len = match written {
            Some((rows, 1)) => rows,
            Some((1, cols)) => cols,
            Some((rows, cols)) if !has_result_vector => {
                if is_row_search {
                    cols
                } else {
                    rows
                }
            }
            _ => search_vec.len(),
        };
        let match_idx = approx_match_ascending(&search_vec, &lookup_value, len, ctx.date_system());
        let match_idx = match match_idx {
            Some(i) => i,
            None => {
                return Ok(CalcValue::Scalar(LiteralValue::Error(ExcelError::new(
                    ExcelErrorKind::Na,
                ))));
            }
        };

        // --- Retrieve result ---
        if let Some((result_data, _)) = result_data {
            let (r_rows, r_cols) = dims(&result_data);
            let result_vec = flatten_1d_vec(&result_data, r_rows, r_cols);
            let val = result_vec
                .get(match_idx)
                .cloned()
                .unwrap_or(LiteralValue::Empty);
            Ok(CalcValue::Scalar(materialise_empty(val)))
        } else if l_rows == 1 && l_cols == 1 {
            Ok(CalcValue::Scalar(materialise_empty(
                lookup_data[0][0].clone(),
            )))
        } else if is_row_search {
            // Return from last row at matched column
            let last_row = l_rows - 1;
            let val = lookup_data
                .get(last_row)
                .and_then(|r| r.get(match_idx))
                .cloned()
                .unwrap_or(LiteralValue::Empty);
            Ok(CalcValue::Scalar(materialise_empty(val)))
        } else {
            // Return from last column at matched row
            let last_col = l_cols - 1;
            let val = lookup_data
                .get(match_idx)
                .and_then(|r| r.get(last_col))
                .cloned()
                .unwrap_or(LiteralValue::Empty);
            Ok(CalcValue::Scalar(materialise_empty(val)))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Materialise a range argument into a 2-D Vec grid, with the rows and
/// columns a reference spans as written (see [`reference_extent`]). An error
/// passed in place of the vector (`#NAME?` from an unknown function, `#N/A`) is
/// not a 1x1 vector to search: Excel returns it as the result, so it comes
/// back as `Err` and the caller propagates it.
fn materialise_range<'a, 'b>(
    arg: &ArgumentHandle<'a, 'b>,
    ctx: &dyn FunctionContext<'b>,
) -> Result<(Vec<Vec<LiteralValue>>, Option<(usize, usize)>), ExcelError> {
    if let Ok(r) = arg.as_reference_or_eval() {
        let current_sheet = ctx.current_sheet();
        let rv = ctx.resolve_range_view(&r, current_sheet)?;
        let (rows, cols) = rv.dims();
        let mut data = Vec::with_capacity(rows);
        rv.for_each_row(&mut |row| {
            let mut owned = Vec::with_capacity(cols);
            owned.extend_from_slice(row);
            data.push(owned);
            Ok(())
        })?;
        Ok((data, reference_extent(&r)))
    } else {
        let v = arg.value()?.into_literal();
        match v {
            LiteralValue::Error(e) => Err(e),
            LiteralValue::Array(rows) => Ok((rows, None)),
            other => Ok((vec![vec![other]], None)),
        }
    }
}

fn dims(data: &[Vec<LiteralValue>]) -> (usize, usize) {
    let rows = data.len();
    let cols = data.first().map(|r| r.len()).unwrap_or(0);
    (rows, cols)
}

/// Flatten a 2-D grid into a 1-D vector.  If only one row → use it; if only
/// one column → extract first element of each row; otherwise flatten row-major.
fn flatten_1d(data: &[Vec<LiteralValue>], rows: usize, cols: usize) -> (Vec<LiteralValue>, bool) {
    if rows == 1 {
        (data[0].clone(), true)
    } else if cols == 1 {
        (data.iter().map(|r| r[0].clone()).collect(), false)
    } else {
        // Multi-dimensional – flatten row-major (uncommon for LOOKUP but
        // required for robustness).
        (data.iter().flat_map(|r| r.iter().cloned()).collect(), false)
    }
}

/// Like `flatten_1d` but returns just the vec (for result vector).
fn flatten_1d_vec(data: &[Vec<LiteralValue>], rows: usize, cols: usize) -> Vec<LiteralValue> {
    flatten_1d(data, rows, cols).0
}

/// Excel materialises empty lookup results as 0.
fn materialise_empty(v: LiteralValue) -> LiteralValue {
    match v {
        LiteralValue::Empty => LiteralValue::Number(0.0),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use crate::traits::ArgumentHandle;
    use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};
    use std::sync::Arc;

    fn lit(v: LiteralValue) -> ASTNode {
        ASTNode::new(ASTNodeType::Literal(v), None)
    }

    // -- approx_match_ascending unit tests --------------------------------

    #[test]
    fn approx_empty_slice() {
        assert_eq!(
            approx_match_ascending(&[], &LiteralValue::Int(1), 0, DateSystem::Excel1900),
            None
        );
    }

    #[test]
    fn approx_below_minimum() {
        let vals = vec![
            LiteralValue::Int(10),
            LiteralValue::Int(20),
            LiteralValue::Int(30),
        ];
        assert_eq!(
            approx_match_ascending(
                &vals,
                &LiteralValue::Int(5),
                vals.len(),
                DateSystem::Excel1900
            ),
            None
        );
    }

    #[test]
    fn approx_exact_hit() {
        let vals = vec![
            LiteralValue::Int(10),
            LiteralValue::Int(20),
            LiteralValue::Int(30),
        ];
        assert_eq!(
            approx_match_ascending(
                &vals,
                &LiteralValue::Int(20),
                vals.len(),
                DateSystem::Excel1900
            ),
            Some(1)
        );
    }

    #[test]
    fn approx_between_values() {
        let vals = vec![
            LiteralValue::Int(10),
            LiteralValue::Int(20),
            LiteralValue::Int(30),
        ];
        assert_eq!(
            approx_match_ascending(
                &vals,
                &LiteralValue::Int(25),
                vals.len(),
                DateSystem::Excel1900
            ),
            Some(1)
        );
    }

    #[test]
    fn approx_above_max() {
        let vals = vec![
            LiteralValue::Int(10),
            LiteralValue::Int(20),
            LiteralValue::Int(30),
        ];
        assert_eq!(
            approx_match_ascending(
                &vals,
                &LiteralValue::Int(100),
                vals.len(),
                DateSystem::Excel1900
            ),
            Some(2)
        );
    }

    #[test]
    fn approx_skips_errors_blanks_and_other_types() {
        let na = || LiteralValue::Error(ExcelError::new(ExcelErrorKind::Na));
        let value = || LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
        let d = DateSystem::Excel1900;
        // SEARCH(list, text) with a single hit in the middle.
        let hits = vec![
            value(),
            value(),
            LiteralValue::Int(8),
            value(),
            value(),
            value(),
        ];
        assert_eq!(
            approx_match_ascending(&hits, &LiteralValue::Int(99999), hits.len(), d),
            Some(2)
        );
        // 1/(cond): the last position where cond holds.
        let last = vec![
            na(),
            LiteralValue::Int(1),
            na(),
            LiteralValue::Int(1),
            na(),
            na(),
            na(),
            na(),
        ];
        assert_eq!(
            approx_match_ascending(&last, &LiteralValue::Int(2), last.len(), d),
            Some(3)
        );
        let tail = vec![na(), na(), na(), na(), LiteralValue::Int(1)];
        assert_eq!(
            approx_match_ascending(&tail, &LiteralValue::Int(2), tail.len(), d),
            Some(4)
        );
        let mixed = vec![
            LiteralValue::Int(1),
            LiteralValue::Empty,
            LiteralValue::Text("x".into()),
            LiteralValue::Int(3),
            na(),
        ];
        assert_eq!(
            approx_match_ascending(&mixed, &LiteralValue::Int(5), mixed.len(), d),
            Some(3)
        );
        assert_eq!(
            approx_match_ascending(&[na(), na()], &LiteralValue::Int(2), 2, d),
            None
        );
    }

    #[test]
    fn approx_bisects_with_inclusive_bounds_and_keeps_an_exact_hit() {
        let d = DateSystem::Excel1900;
        let ints = |v: &[i64]| v.iter().map(|&i| LiteralValue::Int(i)).collect::<Vec<_>>();
        // Inclusive bounds and a floor midpoint: 30 is probed first, then 10.
        let unsorted = ints(&[10, 30, 20, 40]);
        assert_eq!(
            approx_match_ascending(&unsorted, &LiteralValue::Int(25), 4, d),
            Some(0)
        );
        // Equal keys: the last of the run.
        let dups = ints(&[1, 5, 7, 7, 7, 9]);
        assert_eq!(
            approx_match_ascending(&dups, &LiteralValue::Int(7), 6, d),
            Some(4)
        );
        // The first probe is an exact hit; the smaller entries probed after
        // it do not replace it.
        let titles: Vec<LiteralValue> = [
            "Winter Guard #1",
            "Iron man Annual 2021",
            "Infinity War",
            "Deadpool Kills the Marvel Universe AGAIN",
            "Deadpool Kills the Marvel Universe",
            "Black Cat Annual 2021",
        ]
        .into_iter()
        .map(|t| LiteralValue::Text(t.into()))
        .collect();
        assert_eq!(
            approx_match_ascending(&titles, &LiteralValue::Text("Infinity War".into()), 6, d),
            Some(2)
        );
        // Every probe is above "Black Cat Annual 2021".
        assert_eq!(
            approx_match_ascending(
                &titles,
                &LiteralValue::Text("Black Cat Annual 2021".into()),
                6,
                d
            ),
            None
        );
    }

    #[test]
    fn lookup_bisects_the_vector_as_written() {
        use crate::engine::{Engine, EvalConfig};
        use formualizer_parse::parser::parse;
        let mut engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        for (i, title) in [
            "Winter Guard #1",
            "Iron man Annual 2021",
            "Infinity War",
            "Deadpool Kills the Marvel Universe AGAIN",
            "Deadpool Kills the Marvel Universe",
            "Black Cat Annual 2021",
        ]
        .into_iter()
        .enumerate()
        {
            let row = i as u32 + 4;
            engine
                .set_cell_value("Sheet1", row, 1, LiteralValue::Text(title.into()))
                .unwrap();
            engine
                .set_cell_value("Sheet1", row, 8, LiteralValue::Int(i as i64 + 1))
                .unwrap();
        }
        let mut eval = |formula: &str| {
            engine
                .set_cell_formula("Sheet1", 3, 12, parse(formula).unwrap())
                .unwrap();
            engine.evaluate_all().unwrap();
            engine.get_cell_value("Sheet1", 3, 12)
        };
        // A$4:A$1048576 is bisected over all its rows: the probes come down
        // the blank tail to "Infinity War", an exact hit.
        for formula in [
            "=LOOKUP(\"Infinity War\",A$4:A$1048576,H$4:H$1048576)",
            "=LOOKUP(\"Infinity War\",A4:A9,H4:H9)",
        ] {
            assert_eq!(eval(formula), Some(LiteralValue::Number(3.0)), "{formula}");
        }
        let na = eval("=LOOKUP(\"Black Cat Annual 2021\",A$4:A$1048576,H$4:H$1048576)");
        assert!(
            matches!(na, Some(LiteralValue::Error(ref e)) if e.kind == ExcelErrorKind::Na),
            "{na:?}"
        );
    }

    // -- LOOKUP vector form (cell references) -----------------------------

    #[test]
    fn lookup_vector_exact_match() {
        let wb = TestWorkbook::new()
            .with_function(Arc::new(LookupFn))
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Int(3))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Text("a".into()))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Text("b".into()))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Text("c".into()));
        let ctx = wb.interpreter();

        let lookup_vec = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:A3".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(3), Some(1)),
            },
            None,
        );
        let result_vec = ASTNode::new(
            ASTNodeType::Reference {
                original: "B1:B3".into(),
                reference: ReferenceType::range(None, Some(1), Some(2), Some(3), Some(2)),
            },
            None,
        );

        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let needle = lit(LiteralValue::Int(2));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&lookup_vec, &ctx),
            ArgumentHandle::new(&result_vec, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Text("b".into()));
    }

    #[test]
    fn lookup_vector_approximate() {
        let wb = TestWorkbook::new()
            .with_function(Arc::new(LookupFn))
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Int(3))
            .with_cell_a1("Sheet1", "A4", LiteralValue::Int(4))
            .with_cell_a1("Sheet1", "A5", LiteralValue::Int(5));
        let ctx = wb.interpreter();

        let lookup_vec = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:A5".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(5), Some(1)),
            },
            None,
        );

        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let needle = lit(LiteralValue::Number(3.5));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&lookup_vec, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        // Approximate: largest <= 3.5 is 3
        assert_eq!(v, LiteralValue::Number(3.0));
    }

    #[test]
    fn lookup_vector_below_min_returns_na() {
        let wb = TestWorkbook::new()
            .with_function(Arc::new(LookupFn))
            .with_cell_a1("Sheet1", "A1", LiteralValue::Int(10))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Int(20));
        let ctx = wb.interpreter();

        let lookup_vec = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:A2".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(2), Some(1)),
            },
            None,
        );

        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let needle = lit(LiteralValue::Int(5));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&lookup_vec, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert!(matches!(v, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Na));
    }

    // -- LOOKUP array form (array literals) --------------------------------

    #[test]
    fn lookup_array_column_search() {
        // 3 rows x 2 cols => searches first column, returns from last column
        let wb = TestWorkbook::new().with_function(Arc::new(LookupFn));
        let ctx = wb.interpreter();

        let arr = lit(LiteralValue::Array(vec![
            vec![LiteralValue::Int(1), LiteralValue::Text("a".into())],
            vec![LiteralValue::Int(2), LiteralValue::Text("b".into())],
            vec![LiteralValue::Int(3), LiteralValue::Text("c".into())],
        ]));

        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let needle = lit(LiteralValue::Int(2));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&arr, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Text("b".into()));
    }

    #[test]
    fn lookup_array_row_search() {
        // 2 rows x 3 cols => wider, searches first row, returns from last row
        let wb = TestWorkbook::new().with_function(Arc::new(LookupFn));
        let ctx = wb.interpreter();

        let arr = lit(LiteralValue::Array(vec![
            vec![
                LiteralValue::Int(1),
                LiteralValue::Int(2),
                LiteralValue::Int(3),
            ],
            vec![
                LiteralValue::Text("x".into()),
                LiteralValue::Text("y".into()),
                LiteralValue::Text("z".into()),
            ],
        ]));

        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let needle = lit(LiteralValue::Int(2));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&arr, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Text("y".into()));
    }

    #[test]
    fn lookup_error_propagation() {
        let wb = TestWorkbook::new().with_function(Arc::new(LookupFn));
        let ctx = wb.interpreter();

        let arr = lit(LiteralValue::Array(vec![vec![
            LiteralValue::Int(1),
            LiteralValue::Int(2),
        ]]));
        let needle = lit(LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value)));
        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&arr, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert!(matches!(v, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Value));
    }

    #[test]
    fn lookup_single_element() {
        let wb = TestWorkbook::new().with_function(Arc::new(LookupFn));
        let ctx = wb.interpreter();

        let arr = lit(LiteralValue::Array(vec![vec![LiteralValue::Int(5)]]));
        let f = ctx.context.get_function("", "LOOKUP").unwrap();

        // Exact single-element match
        let needle = lit(LiteralValue::Int(5));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&arr, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Int(5));

        // Below single element → #N/A
        let needle_lo = lit(LiteralValue::Int(3));
        let args2 = vec![
            ArgumentHandle::new(&needle_lo, &ctx),
            ArgumentHandle::new(&arr, &ctx),
        ];
        let v2 = f
            .dispatch(&args2, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert!(matches!(v2, LiteralValue::Error(e) if e.kind == ExcelErrorKind::Na));
    }

    #[test]
    fn lookup_text_values() {
        // Text search in ascending order
        let wb = TestWorkbook::new()
            .with_function(Arc::new(LookupFn))
            .with_cell_a1("Sheet1", "A1", LiteralValue::Text("apple".into()))
            .with_cell_a1("Sheet1", "A2", LiteralValue::Text("banana".into()))
            .with_cell_a1("Sheet1", "A3", LiteralValue::Text("cherry".into()))
            .with_cell_a1("Sheet1", "B1", LiteralValue::Int(1))
            .with_cell_a1("Sheet1", "B2", LiteralValue::Int(2))
            .with_cell_a1("Sheet1", "B3", LiteralValue::Int(3));
        let ctx = wb.interpreter();

        let lookup_vec = ASTNode::new(
            ASTNodeType::Reference {
                original: "A1:A3".into(),
                reference: ReferenceType::range(None, Some(1), Some(1), Some(3), Some(1)),
            },
            None,
        );
        let result_vec = ASTNode::new(
            ASTNodeType::Reference {
                original: "B1:B3".into(),
                reference: ReferenceType::range(None, Some(1), Some(2), Some(3), Some(2)),
            },
            None,
        );

        let f = ctx.context.get_function("", "LOOKUP").unwrap();
        let needle = lit(LiteralValue::Text("banana".into()));
        let args = vec![
            ArgumentHandle::new(&needle, &ctx),
            ArgumentHandle::new(&lookup_vec, &ctx),
            ArgumentHandle::new(&result_vec, &ctx),
        ];
        let v = f
            .dispatch(&args, &ctx.function_context(None))
            .unwrap()
            .into_literal();
        assert_eq!(v, LiteralValue::Number(2.0));
    }
}
