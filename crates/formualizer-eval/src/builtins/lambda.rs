use crate::function::{FnCaps, Function};
use crate::function_contract::{
    FunctionArgumentDependencyContract, FunctionArityRule, FunctionDependencyClass,
    FunctionDependencyContract,
};
use crate::interpreter::{LocalBinding, LocalEnv};
use crate::traits::{ArgumentHandle, CalcValue, CustomCallable, FunctionContext};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};
use std::collections::HashSet;
use std::sync::Arc;

fn value_error(msg: impl Into<String>) -> ExcelError {
    ExcelError::new(ExcelErrorKind::Value).with_message(msg.into())
}

fn local_name_from_ast(node: &ASTNode) -> Result<String, ExcelError> {
    match &node.node_type {
        ASTNodeType::Reference {
            reference: ReferenceType::NamedRange(name),
            ..
        } => Ok(name.clone()),
        _ => Err(value_error("Expected a local name identifier")),
    }
}

fn binding_from_calc_value(cv: CalcValue<'_>) -> LocalBinding {
    match cv {
        CalcValue::Scalar(v) | CalcValue::AnnotatedScalar(v, _) => LocalBinding::Value(v),
        CalcValue::Range(rv) => {
            let (rows, cols) = rv.dims();
            if rows == 1 && cols == 1 {
                LocalBinding::Value(rv.get_cell(0, 0))
            } else {
                let mut data = Vec::with_capacity(rows);
                let _ = rv.for_each_row(&mut |row| {
                    data.push(row.to_vec());
                    Ok(())
                });
                LocalBinding::Value(LiteralValue::Array(data))
            }
        }
        CalcValue::Callable(c) => LocalBinding::Callable(c),
    }
}

#[derive(Debug)]
pub struct LetFn;

/// Binds local names to values and evaluates a final expression with those bindings.
///
/// `LET` introduces lexical variables using name/value pairs, then returns the last expression.
///
/// # Remarks
/// - Arguments must be provided as `name, value` pairs followed by one final calculation expression.
/// - Names are resolved as local identifiers and can shadow workbook-level names.
/// - Bindings are evaluated left-to-right, so later values can reference earlier bindings.
/// - Invalid names or malformed arity return `#VALUE!`.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Bind intermediate values"
/// formula: "=LET(rate,0.08,price,125,price*(1+rate))"
/// expected: 135
/// ```
///
/// ```yaml,sandbox
/// title: "Use LET with range calculations"
/// grid:
///   A1: 10
///   A2: 4
/// formula: "=LET(total,SUM(A1:A2),total*2)"
/// expected: 28
/// ```
///
/// ```yaml,sandbox
/// title: "Nested LET supports shadowing"
/// formula: "=LET(x,2,LET(x,5,x)+x)"
/// expected: 7
/// ```
///
/// ```yaml,docs
/// related:
///   - LAMBDA
///   - IF
///   - SUM
///   - INDEX
/// faq:
///   - q: "Can a LET binding reference a name defined later in the same LET?"
///     a: "No. LET evaluates name/value pairs left-to-right, so each binding can only use earlier bindings."
///   - q: "Does LET overwrite workbook or worksheet names permanently?"
///     a: "No. LET names are lexical and local to that formula evaluation; they only shadow outer names inside the LET expression."
///   - q: "Is LET itself volatile?"
///     a: "No. LET is deterministic unless one of its bound expressions calls a volatile function such as RAND."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: LET
/// Type: LetFn
/// Min args: 3
/// Max args: variadic
/// Variadic: true
/// Signature: LET(arg1...: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, SHORT_CIRCUIT
/// [formualizer-docgen:schema:end]
impl Function for LetFn {
    fn caps(&self) -> FnCaps {
        FnCaps::PURE | FnCaps::SHORT_CIRCUIT | FnCaps::LOCAL_ENVIRONMENT | FnCaps::MAY_SPILL
    }

    fn name(&self) -> &'static str {
        "LET"
    }

    fn min_args(&self) -> usize {
        3
    }

    fn variadic(&self) -> bool {
        true
    }

    fn dependency_contract(&self, arity: usize) -> Option<FunctionDependencyContract> {
        FunctionDependencyContract {
            class: FunctionDependencyClass::StaticScalarAllArgs,
            arity: FunctionArityRule::OddAtLeast(3),
            arguments: FunctionArgumentDependencyContract::LocalBindingPairs,
        }
        .for_arity(arity)
    }

    fn arg_schema(&self) -> &'static [crate::args::ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<crate::args::ArgSchema>> =
            std::sync::LazyLock::new(|| vec![crate::args::ArgSchema::any()]);
        &SCHEMA
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
        if args.len() < 3 || args.len().is_multiple_of(2) {
            return Ok(CalcValue::Scalar(LiteralValue::Error(value_error(
                "LET expects name/value pairs followed by a final expression",
            ))));
        }

        let mut env: LocalEnv = args[0].current_env();

        for pair_idx in (0..args.len() - 1).step_by(2) {
            let name = match local_name_from_ast(args[pair_idx].ast()) {
                Ok(name) => name,
                Err(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            };

            let bound = args[pair_idx + 1].value_with_env(env.clone())?;
            env = env.with_binding(&name, binding_from_calc_value(bound));
        }

        args[args.len() - 1].value_with_env(env)
    }
}

#[derive(Clone)]
struct LambdaClosure {
    params: Vec<String>,
    body: ASTNode,
    captured_env: LocalEnv,
}

impl CustomCallable for LambdaClosure {
    fn arity(&self) -> usize {
        self.params.len()
    }

    fn invoke<'ctx>(
        &self,
        interp: &crate::interpreter::Interpreter<'ctx>,
        args: &[LiteralValue],
    ) -> Result<CalcValue<'ctx>, ExcelError> {
        if args.len() != self.arity() {
            return Ok(CalcValue::Scalar(LiteralValue::Error(value_error(
                format!(
                    "LAMBDA expected {} argument(s), got {}",
                    self.arity(),
                    args.len()
                ),
            ))));
        }

        let mut env = self.captured_env.clone();
        for (name, value) in self.params.iter().zip(args.iter()) {
            env = env.with_binding(name, LocalBinding::Value(value.clone()));
        }

        let scoped = interp.with_local_env(env);
        scoped.evaluate_ast(&self.body)
    }
}

#[derive(Debug)]
pub struct LambdaFn;

/// Creates an anonymous callable that can be invoked with spreadsheet arguments.
///
/// `LAMBDA` captures its defining local scope and returns a reusable function value.
///
/// # Remarks
/// - All arguments except the last are parameter names; the last argument is the body expression.
/// - Parameter names must be unique (case-insensitive), or `#VALUE!` is returned.
/// - Invocation arity must exactly match the declared parameter count.
/// - Returning an uninvoked lambda as a final cell value yields a `#CALC!` in evaluation.
///
/// # Examples
///
/// ```yaml,sandbox
/// title: "Inline lambda invocation"
/// formula: "=LAMBDA(x,x+1)(41)"
/// expected: 42
/// ```
///
/// ```yaml,sandbox
/// title: "Lambda captures outer LET bindings"
/// formula: "=LET(k,10,addk,LAMBDA(n,n+k),addk(5))"
/// expected: 15
/// ```
///
/// ```yaml,sandbox
/// title: "Duplicate parameter names are invalid"
/// formula: "=LAMBDA(x,x,x+1)"
/// expected: "#VALUE!"
/// ```
///
/// ```yaml,docs
/// related:
///   - LET
///   - IF
///   - SUM
/// faq:
///   - q: "Why does =LAMBDA(x,x+1) return #CALC! instead of a number?"
///     a: "LAMBDA returns a callable value. In a cell result position, it must be invoked, for example =LAMBDA(x,x+1)(1)."
///   - q: "Does a LAMBDA read outer LET variables at call time or definition time?"
///     a: "Definition time. The closure captures its lexical environment when created."
///   - q: "Can I call a LAMBDA with fewer or extra arguments?"
///     a: "No. Invocation arity must match the declared parameter count exactly, or #VALUE! is returned."
/// ```
///
/// [formualizer-docgen:schema:start]
/// Name: LAMBDA
/// Type: LambdaFn
/// Min args: 1
/// Max args: variadic
/// Variadic: true
/// Signature: LAMBDA(arg1...: any@scalar)
/// Arg schema: arg1{kinds=any,required=true,shape=scalar,by_ref=false,coercion=None,max=None,repeating=None,default=false}
/// Caps: PURE, SHORT_CIRCUIT
/// [formualizer-docgen:schema:end]
impl Function for LambdaFn {
    fn caps(&self) -> FnCaps {
        FnCaps::PURE | FnCaps::SHORT_CIRCUIT | FnCaps::LOCAL_ENVIRONMENT | FnCaps::MAY_SPILL
    }

    fn name(&self) -> &'static str {
        "LAMBDA"
    }

    fn min_args(&self) -> usize {
        1
    }

    fn variadic(&self) -> bool {
        true
    }

    fn dependency_contract(&self, arity: usize) -> Option<FunctionDependencyContract> {
        FunctionDependencyContract {
            class: FunctionDependencyClass::StaticScalarAllArgs,
            arity: FunctionArityRule::AtLeast(1),
            arguments: FunctionArgumentDependencyContract::LambdaParameters,
        }
        .for_arity(arity)
    }

    fn arg_schema(&self) -> &'static [crate::args::ArgSchema] {
        static SCHEMA: std::sync::LazyLock<Vec<crate::args::ArgSchema>> =
            std::sync::LazyLock::new(|| vec![crate::args::ArgSchema::any()]);
        &SCHEMA
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
        if args.is_empty() {
            return Ok(CalcValue::Scalar(LiteralValue::Error(value_error(
                "LAMBDA requires at least a calculation expression",
            ))));
        }

        let mut params = Vec::new();
        let mut seen = HashSet::new();
        for arg in &args[..args.len() - 1] {
            let name = match local_name_from_ast(arg.ast()) {
                Ok(name) => name,
                Err(e) => return Ok(CalcValue::Scalar(LiteralValue::Error(e))),
            };
            let key = name.to_ascii_uppercase();
            if !seen.insert(key) {
                return Ok(CalcValue::Scalar(LiteralValue::Error(value_error(
                    "LAMBDA parameter names must be unique",
                ))));
            }
            params.push(name);
        }

        let closure = LambdaClosure {
            params,
            body: args[args.len() - 1].ast().clone(),
            captured_env: args[0].current_env(),
        };

        Ok(CalcValue::Callable(Arc::new(closure)))
    }
}

/* ───────────────────── LAMBDA helper functions ───────────────────── */

type Grid = Vec<Vec<LiteralValue>>;

fn error_value(kind: ExcelErrorKind, msg: &str) -> LiteralValue {
    LiteralValue::Error(ExcelError::new(kind).with_message(msg.to_string()))
}

fn scalar<'b>(value: LiteralValue) -> CalcValue<'b> {
    CalcValue::Scalar(value)
}

/// The LAMBDA a helper receives as its last argument. `Err` carries the
/// value to return instead: an error argument propagates, and anything else
/// that is not a LAMBDA, or a LAMBDA with the wrong number of parameters,
/// is `#VALUE!`.
fn lambda_arg(
    arg: &ArgumentHandle<'_, '_>,
    arity: usize,
) -> Result<Result<Arc<dyn CustomCallable>, LiteralValue>, ExcelError> {
    Ok(match arg.value()? {
        CalcValue::Callable(callable) if callable.arity() == arity => Ok(callable),
        CalcValue::Callable(_) => Err(error_value(
            ExcelErrorKind::Value,
            "LAMBDA has the wrong number of parameters",
        )),
        other => match other.into_literal() {
            error @ LiteralValue::Error(_) => Err(error),
            _ => Err(error_value(ExcelErrorKind::Value, "Expected a LAMBDA")),
        },
    })
}

/// An array argument as rows of values; a single value is a 1x1 array.
fn grid_arg(arg: &ArgumentHandle<'_, '_>) -> Result<Grid, ExcelError> {
    Ok(match arg.value()?.into_literal() {
        LiteralValue::Array(rows) => rows,
        other => vec![vec![other]],
    })
}

/// A 1x1 array passes to a LAMBDA as its single value.
fn array_value(rows: Grid) -> LiteralValue {
    if rows.len() == 1 && rows[0].len() == 1 {
        rows.into_iter().next().unwrap().into_iter().next().unwrap()
    } else {
        LiteralValue::Array(rows)
    }
}

fn invoke(
    arg: &ArgumentHandle<'_, '_>,
    callable: &Arc<dyn CustomCallable>,
    values: &[LiteralValue],
) -> LiteralValue {
    match callable.invoke(arg.interpreter(), values) {
        Ok(result) => result.into_literal(),
        Err(error) => LiteralValue::Error(error),
    }
}

/// One element of a helper's result array. A LAMBDA result that is itself a
/// multi-cell array cannot nest, so the whole helper returns `#CALC!`; a blank
/// result reads as 0, like a formula that refers to an empty cell.
fn element_value(value: LiteralValue) -> Option<LiteralValue> {
    match value {
        LiteralValue::Array(rows) => {
            if rows.len() == 1 && rows[0].len() == 1 {
                element_value(rows.into_iter().next().unwrap().into_iter().next().unwrap())
            } else {
                None
            }
        }
        LiteralValue::Empty => Some(LiteralValue::Number(0.0)),
        other => Some(other),
    }
}

fn nested_array_error() -> LiteralValue {
    error_value(ExcelErrorKind::Calc, "Nested arrays are not supported")
}

fn array_result<'b>(rows: Grid, ctx: &dyn FunctionContext<'b>) -> CalcValue<'b> {
    if rows.len() == 1 && rows[0].len() == 1 {
        scalar(rows.into_iter().next().unwrap().into_iter().next().unwrap())
    } else {
        CalcValue::Range(crate::engine::range_view::RangeView::from_owned_rows(
            rows,
            ctx.date_system(),
        ))
    }
}

/// Element `(r, c)` of an array broadcast to a larger result: a single row or
/// column repeats, and positions past the array's edge are `#N/A`.
fn broadcast_get(rows: &Grid, r: usize, c: usize) -> LiteralValue {
    let height = rows.len();
    let width = rows.first().map_or(0, Vec::len);
    let r = if height == 1 { 0 } else { r };
    let c = if width == 1 { 0 } else { c };
    rows.get(r)
        .and_then(|row| row.get(c))
        .cloned()
        .unwrap_or_else(|| error_value(ExcelErrorKind::Na, "Array too small"))
}

macro_rules! lambda_helper {
    ($ty:ident, $name:literal, $min:expr, $variadic:expr, $eval:ident) => {
        #[derive(Debug)]
        pub struct $ty;

        impl Function for $ty {
            fn caps(&self) -> FnCaps {
                FnCaps::PURE | FnCaps::MAY_SPILL
            }

            fn name(&self) -> &'static str {
                $name
            }

            fn min_args(&self) -> usize {
                $min
            }

            fn variadic(&self) -> bool {
                $variadic
            }

            fn arg_schema(&self) -> &'static [crate::args::ArgSchema] {
                static SCHEMA: std::sync::LazyLock<Vec<crate::args::ArgSchema>> =
                    std::sync::LazyLock::new(|| vec![crate::args::ArgSchema::any()]);
                &SCHEMA
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
                ctx: &dyn FunctionContext<'b>,
            ) -> Result<CalcValue<'b>, ExcelError> {
                if args.len() < $min || (!$variadic && args.len() > $min) {
                    return Ok(scalar(error_value(
                        ExcelErrorKind::Value,
                        concat!("Wrong number of arguments to ", $name),
                    )));
                }
                $eval(args, ctx)
            }
        }
    };
}

/// `MAP(array1, [array2, ...], lambda)`: applies the LAMBDA to each element,
/// taking one parameter per array.
fn eval_map<'a, 'b>(
    args: &[ArgumentHandle<'a, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<CalcValue<'b>, ExcelError> {
    let (lambda, arrays) = args.split_last().unwrap();
    let callable = match lambda_arg(lambda, arrays.len())? {
        Ok(callable) => callable,
        Err(value) => return Ok(scalar(value)),
    };
    let grids = arrays.iter().map(grid_arg).collect::<Result<Vec<_>, _>>()?;
    let height = grids.iter().map(Vec::len).max().unwrap_or(0);
    let width = grids
        .iter()
        .map(|g| g.first().map_or(0, Vec::len))
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(height);
    for r in 0..height {
        let mut row = Vec::with_capacity(width);
        for c in 0..width {
            let values: Vec<LiteralValue> = grids.iter().map(|g| broadcast_get(g, r, c)).collect();
            match element_value(invoke(lambda, &callable, &values)) {
                Some(value) => row.push(value),
                None => return Ok(scalar(nested_array_error())),
            }
        }
        out.push(row);
    }
    Ok(array_result(out, ctx))
}

/// The starting accumulator of REDUCE/SCAN; an omitted one is blank.
fn initial_value(arg: &ArgumentHandle<'_, '_>) -> Result<LiteralValue, ExcelError> {
    if arg.is_omitted() {
        Ok(LiteralValue::Empty)
    } else {
        Ok(arg.value()?.into_literal())
    }
}

/// `REDUCE([initial_value], array, lambda(accumulator, value))`: folds the
/// array in row-major order and returns the final accumulator, which may be
/// an array.
fn eval_reduce<'a, 'b>(
    args: &[ArgumentHandle<'a, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<CalcValue<'b>, ExcelError> {
    let (array_arg, lambda) = (&args[1], &args[2]);
    let callable = match lambda_arg(lambda, 2)? {
        Ok(callable) => callable,
        Err(value) => return Ok(scalar(value)),
    };
    let mut acc = initial_value(&args[0])?;
    for row in grid_arg(array_arg)? {
        for value in row {
            acc = invoke(lambda, &callable, &[acc, value]);
        }
    }
    Ok(match acc {
        LiteralValue::Array(rows) => array_result(rows, ctx),
        LiteralValue::Empty => scalar(LiteralValue::Number(0.0)),
        other => scalar(other),
    })
}

/// `SCAN([initial_value], array, lambda(accumulator, value))`: like REDUCE,
/// returning every intermediate accumulator in the shape of the array.
fn eval_scan<'a, 'b>(
    args: &[ArgumentHandle<'a, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<CalcValue<'b>, ExcelError> {
    let (array_arg, lambda) = (&args[1], &args[2]);
    let callable = match lambda_arg(lambda, 2)? {
        Ok(callable) => callable,
        Err(value) => return Ok(scalar(value)),
    };
    let mut acc = initial_value(&args[0])?;
    let grid = grid_arg(array_arg)?;
    let mut out = Vec::with_capacity(grid.len());
    for row in grid {
        let mut out_row = Vec::with_capacity(row.len());
        for value in row {
            acc = invoke(lambda, &callable, &[acc, value]);
            match element_value(acc.clone()) {
                Some(value) => out_row.push(value),
                None => return Ok(scalar(nested_array_error())),
            }
        }
        out.push(out_row);
    }
    Ok(array_result(out, ctx))
}

/// `BYROW(array, lambda(row))`: one result per row, as a column.
fn eval_byrow<'a, 'b>(
    args: &[ArgumentHandle<'a, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<CalcValue<'b>, ExcelError> {
    let callable = match lambda_arg(&args[1], 1)? {
        Ok(callable) => callable,
        Err(value) => return Ok(scalar(value)),
    };
    let mut out = Vec::new();
    for row in grid_arg(&args[0])? {
        match element_value(invoke(&args[1], &callable, &[array_value(vec![row])])) {
            Some(value) => out.push(vec![value]),
            None => return Ok(scalar(nested_array_error())),
        }
    }
    Ok(array_result(out, ctx))
}

/// `BYCOL(array, lambda(column))`: one result per column, as a row.
fn eval_bycol<'a, 'b>(
    args: &[ArgumentHandle<'a, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<CalcValue<'b>, ExcelError> {
    let callable = match lambda_arg(&args[1], 1)? {
        Ok(callable) => callable,
        Err(value) => return Ok(scalar(value)),
    };
    let grid = grid_arg(&args[0])?;
    let width = grid.first().map_or(0, Vec::len);
    let mut out = Vec::with_capacity(width);
    for c in 0..width {
        let column: Grid = grid
            .iter()
            .map(|row| vec![row.get(c).cloned().unwrap_or(LiteralValue::Empty)])
            .collect();
        match element_value(invoke(&args[1], &callable, &[array_value(column)])) {
            Some(value) => out.push(value),
            None => return Ok(scalar(nested_array_error())),
        }
    }
    Ok(array_result(vec![out], ctx))
}

/// A MAKEARRAY dimension: a number truncated to an integer from 1 up to the
/// sheet's size; anything else is `#VALUE!`.
fn dimension_arg(
    arg: &ArgumentHandle<'_, '_>,
    max: f64,
) -> Result<Result<usize, LiteralValue>, ExcelError> {
    let value = arg.value()?.into_literal();
    if let LiteralValue::Error(_) = value {
        return Ok(Err(value));
    }
    Ok(match crate::coercion::to_number_lenient(&value) {
        Ok(n) if n.trunc() >= 1.0 && n.trunc() <= max => Ok(n.trunc() as usize),
        _ => Err(error_value(
            ExcelErrorKind::Value,
            "Invalid MAKEARRAY dimension",
        )),
    })
}

/// `MAKEARRAY(rows, columns, lambda(row, column))`: builds an array from the
/// LAMBDA applied to each 1-based row and column index.
fn eval_makearray<'a, 'b>(
    args: &[ArgumentHandle<'a, 'b>],
    ctx: &dyn FunctionContext<'b>,
) -> Result<CalcValue<'b>, ExcelError> {
    let height = match dimension_arg(&args[0], 1_048_576.0)? {
        Ok(n) => n,
        Err(value) => return Ok(scalar(value)),
    };
    let width = match dimension_arg(&args[1], 16_384.0)? {
        Ok(n) => n,
        Err(value) => return Ok(scalar(value)),
    };
    let callable = match lambda_arg(&args[2], 2)? {
        Ok(callable) => callable,
        Err(value) => return Ok(scalar(value)),
    };
    let mut out = Vec::with_capacity(height);
    for r in 1..=height {
        let mut row = Vec::with_capacity(width);
        for c in 1..=width {
            let values = [
                LiteralValue::Number(r as f64),
                LiteralValue::Number(c as f64),
            ];
            match element_value(invoke(&args[2], &callable, &values)) {
                Some(value) => row.push(value),
                None => return Ok(scalar(nested_array_error())),
            }
        }
        out.push(row);
    }
    Ok(array_result(out, ctx))
}

lambda_helper!(MapFn, "MAP", 2, true, eval_map);
lambda_helper!(ReduceFn, "REDUCE", 3, false, eval_reduce);
lambda_helper!(ScanFn, "SCAN", 3, false, eval_scan);
lambda_helper!(ByRowFn, "BYROW", 2, false, eval_byrow);
lambda_helper!(ByColFn, "BYCOL", 2, false, eval_bycol);
lambda_helper!(MakeArrayFn, "MAKEARRAY", 3, false, eval_makearray);

pub fn register_builtins() {
    crate::function_registry::register_builtin(Arc::new(LetFn));
    crate::function_registry::register_builtin(Arc::new(LambdaFn));
    crate::function_registry::register_builtin(Arc::new(MapFn));
    crate::function_registry::register_builtin(Arc::new(ReduceFn));
    crate::function_registry::register_builtin(Arc::new(ScanFn));
    crate::function_registry::register_builtin(Arc::new(ByRowFn));
    crate::function_registry::register_builtin(Arc::new(ByColFn));
    crate::function_registry::register_builtin(Arc::new(MakeArrayFn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_workbook::TestWorkbook;
    use formualizer_parse::parser::parse;

    fn test_wb() -> TestWorkbook {
        TestWorkbook::new()
            .with_function(Arc::new(LetFn))
            .with_function(Arc::new(LambdaFn))
    }

    fn eval(src: &str) -> LiteralValue {
        eval_result(src).expect("eval")
    }

    fn eval_result(src: &str) -> Result<LiteralValue, ExcelError> {
        eval_result_with_wb(src, test_wb())
    }

    fn eval_with_wb(src: &str, wb: TestWorkbook) -> LiteralValue {
        eval_result_with_wb(src, wb).expect("eval")
    }

    fn eval_result_with_wb(src: &str, wb: TestWorkbook) -> Result<LiteralValue, ExcelError> {
        let interp = wb.interpreter();
        let ast = parse(src).expect("parse");
        interp.evaluate_ast(&ast).map(|v| v.into_literal())
    }

    #[test]
    fn let_binds_values() {
        assert_eq!(eval("=LET(x,2,x+3)"), LiteralValue::Number(5.0));
    }

    #[test]
    fn let_nested_shadowing() {
        assert_eq!(eval("=LET(x,2,LET(x,5,x)+x)"), LiteralValue::Number(7.0));
    }

    #[test]
    fn lambda_can_be_bound_and_invoked() {
        assert_eq!(
            eval("=LET(inc,LAMBDA(n,n+1),inc(41))"),
            LiteralValue::Number(42.0)
        );
    }

    #[test]
    fn lambda_closure_captures_outer_bindings() {
        assert_eq!(
            eval("=LET(k,10,addk,LAMBDA(n,n+k),addk(5))"),
            LiteralValue::Number(15.0)
        );
    }

    #[test]
    fn lambda_arity_errors() {
        let v = eval("=LET(inc,LAMBDA(n,n+1),inc(1,2))");
        match v {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Value),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn lambda_value_requires_invocation() {
        let v = eval("=LAMBDA(x,x+1)");
        match v {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Calc),
            other => panic!("expected #CALC!, got {other:?}"),
        }
    }

    #[test]
    fn let_rejects_non_identifier_name() {
        let v = eval("=LET(A1,2,A1)");
        match v {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Value),
            other => panic!("expected #VALUE!, got {other:?}"),
        }
    }

    #[test]
    fn lambda_rejects_duplicate_params() {
        let v = eval("=LAMBDA(x,x,x+1)");
        match v {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Value),
            other => panic!("expected #VALUE!, got {other:?}"),
        }
    }

    #[test]
    fn let_and_lambda_names_are_case_insensitive() {
        assert_eq!(eval("=LET(x,1,X+1)"), LiteralValue::Number(2.0));
        assert_eq!(
            eval("=LET(F,LAMBDA(n,n+1),f(1))"),
            LiteralValue::Number(2.0)
        );
    }

    #[test]
    fn let_shadows_workbook_named_range() {
        let wb = test_wb().with_named_range("x", vec![vec![LiteralValue::Number(100.0)]]);
        assert_eq!(eval_with_wb("=LET(X,1,x+1)", wb), LiteralValue::Number(2.0));
    }

    #[test]
    fn lambda_param_shadows_outer_scope() {
        assert_eq!(
            eval("=LET(n,5,f,LAMBDA(n,n+1),f(10))"),
            LiteralValue::Number(11.0)
        );
    }

    #[test]
    fn lambda_closure_snapshot_semantics() {
        assert_eq!(
            eval("=LET(k,1,f,LAMBDA(x,x+k),k,2,f(0))"),
            LiteralValue::Number(1.0)
        );
    }

    #[test]
    fn let_undefined_symbol_before_binding_errors() {
        let err = eval_result("=LET(x,y,y,2,x)").expect_err("expected #NAME?");
        assert_eq!(err.kind, ExcelErrorKind::Name);
    }

    #[test]
    fn non_invoked_lambda_in_let_is_calc_error() {
        let v = eval("=LET(f,LAMBDA(x,x+1),f)");
        match v {
            LiteralValue::Error(e) => assert_eq!(e.kind, ExcelErrorKind::Calc),
            other => panic!("expected #CALC!, got {other:?}"),
        }
    }
}
