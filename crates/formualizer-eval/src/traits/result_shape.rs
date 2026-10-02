//! Whether an argument evaluates to an array or to a single value.
//!
//! Excel keeps an array an array whatever its size (MS-XLS distinguishes the
//! ARRAY and VALUE types of a formula's parts): `{1}`, `SEQUENCE(1)`, `{1}+0`,
//! `ABS({-1})` and a LET name bound to one of them are one-element arrays,
//! while a function that consumes an array returns a single value
//! (`SUM({1})`, `TEXTJOIN("",TRUE,{"x"})`), and IF or CHOOSE return the value
//! of the argument they select, whatever the others are. The engine holds a
//! one-element array as its single value, so the shape is read from the
//! expression, the way Excel types each part of it:
//!
//! - an array constant is an array; a constant, a cell or range reference
//!   (in an operator it is a single value or is intersected to one; a range
//!   that stays whole is a multi-cell array value of its own) is not;
//! - an operator, and a function over a parameter that takes a single value
//!   ([`crate::lift::legacy_arg`]), return an array when an operand or such
//!   an argument is one: Excel evaluates them once per element;
//! - a function that returns arrays (SEQUENCE, SORT, TRANSPOSE, MMULT, ...)
//!   returns one; any other function returns a single value;
//! - IF and CHOOSE return the argument their single-value test or index
//!   selects, IFERROR, IFNA, IFS and SWITCH one of their value arguments,
//!   and LET its calculation, with each name the shape of its value; a
//!   LAMBDA called with arguments returns its body's shape with each
//!   parameter the shape of its argument.
//!
//! Where that depends on more than the expression (INDEX with a row or column
//! of 0, the item XLOOKUP returns, the nodes FILTERXML finds, REDUCE's
//! accumulator, a LAMBDA parameter at run time) the shape is unknown.

use super::{ArgumentExpr, ArgumentHandle, selected_branch};
use crate::engine::arena::{
    AstNodeData, AstNodeId, CALL_EXPRESSION_NAME, CompactRefType, DataStore, ValueType,
};
use crate::engine::sheet_registry::SheetRegistry;
use crate::interpreter::LocalBinding;
use crate::lift::LegacyArg;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};

/// What an expression evaluates to in Excel's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResultShape {
    /// A single value or a reference.
    Single,
    /// An array, a one-element array included.
    Array,
    /// Not known from the expression.
    Unknown,
}

impl ResultShape {
    /// The shape of an element-wise result over parts of these shapes: an
    /// array when any of them is one.
    fn elementwise(self, other: Self) -> Self {
        match (self, other) {
            (Self::Array, _) | (_, Self::Array) => Self::Array,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            _ => Self::Single,
        }
    }

    /// The shape of a result that is one of several alternatives.
    fn either(self, other: Self) -> Self {
        if self == other { self } else { Self::Unknown }
    }
}

/// Nested calls deeper than this are of unknown shape.
const MAX_DEPTH: usize = 256;

pub(super) fn of(arg: &ArgumentHandle<'_, '_>) -> ResultShape {
    Typer::default().shape(arg)
}

/// A name bound inside the expression being typed.
#[derive(Clone)]
enum Local<'a, 'b> {
    Shape(ResultShape),
    /// A LAMBDA: its parameter names and body, and how many names of the
    /// scope it sees (those in scope where it is written).
    Lambda {
        params: Vec<&'a str>,
        body: Box<ArgumentHandle<'a, 'b>>,
        scope: usize,
    },
}

/// The arguments of a call.
#[derive(Clone, Copy)]
enum Args<'a> {
    Ast(&'a [ASTNode]),
    Arena(&'a [AstNodeId], &'a DataStore, &'a SheetRegistry),
}

impl<'a> Args<'a> {
    fn len(&self) -> usize {
        match self {
            Args::Ast(nodes) => nodes.len(),
            Args::Arena(ids, ..) => ids.len(),
        }
    }

    /// Argument `index`, evaluated by `at`'s interpreter.
    fn get<'b>(&self, at: &ArgumentHandle<'a, 'b>, index: usize) -> ArgumentHandle<'a, 'b> {
        match *self {
            Args::Ast(nodes) => ArgumentHandle::new(&nodes[index], at.interp),
            Args::Arena(ids, data_store, sheet_registry) => {
                ArgumentHandle::new_arena(ids[index], at.interp, data_store, sheet_registry)
            }
        }
    }
}

/// The parts of an expression that its shape depends on.
enum Node<'a, 'b> {
    Single,
    Array,
    Name(&'a str),
    /// A value operator (not `@`, `:` or the space) and its operands.
    Operator(Vec<ArgumentHandle<'a, 'b>>),
    Function(&'a str, Args<'a>),
    /// `callee(args)`, the callee written as an expression.
    Call(Box<ArgumentHandle<'a, 'b>>, Args<'a>),
}

fn node<'a, 'b>(arg: &ArgumentHandle<'a, 'b>) -> Node<'a, 'b> {
    match arg.expr {
        ArgumentExpr::Ast(ast) => match &ast.node_type {
            ASTNodeType::Array(_) | ASTNodeType::Literal(LiteralValue::Array(_)) => Node::Array,
            ASTNodeType::Literal(_) | ASTNodeType::Omitted => Node::Single,
            ASTNodeType::Reference {
                reference: ReferenceType::NamedRange(name),
                ..
            } => Node::Name(name),
            ASTNodeType::Reference { .. } => Node::Single,
            ASTNodeType::UnaryOp { op, expr } if op != "@" => {
                Node::Operator(vec![ArgumentHandle::new(expr, arg.interp)])
            }
            ASTNodeType::BinaryOp { op, left, right } if !matches!(op.as_str(), ":" | " ") => {
                Node::Operator(vec![
                    ArgumentHandle::new(left, arg.interp),
                    ArgumentHandle::new(right, arg.interp),
                ])
            }
            ASTNodeType::UnaryOp { .. } | ASTNodeType::BinaryOp { .. } => Node::Single,
            ASTNodeType::Function { name, args } => Node::Function(name, Args::Ast(args)),
            ASTNodeType::Call { callee, args } => Node::Call(
                Box::new(ArgumentHandle::new(callee, arg.interp)),
                Args::Ast(args),
            ),
        },
        ArgumentExpr::Arena {
            id,
            data_store,
            sheet_registry,
        } => {
            let child = |id| ArgumentHandle::new_arena(id, arg.interp, data_store, sheet_registry);
            match data_store.get_node(id) {
                None => Node::Single,
                Some(AstNodeData::Literal(value)) if value.value_type() == ValueType::Array => {
                    Node::Array
                }
                Some(AstNodeData::Array { .. }) => Node::Array,
                Some(AstNodeData::Literal(_) | AstNodeData::Omitted) => Node::Single,
                Some(AstNodeData::Reference {
                    ref_type: CompactRefType::NamedRange(name_id),
                    ..
                }) => Node::Name(data_store.resolve_ast_string(*name_id)),
                Some(AstNodeData::Reference { .. }) => Node::Single,
                Some(AstNodeData::UnaryOp { op_id, expr_id })
                    if data_store.resolve_ast_string(*op_id) != "@" =>
                {
                    Node::Operator(vec![child(*expr_id)])
                }
                Some(AstNodeData::BinaryOp {
                    op_id,
                    left_id,
                    right_id,
                }) if !matches!(data_store.resolve_ast_string(*op_id), ":" | " ") => {
                    Node::Operator(vec![child(*left_id), child(*right_id)])
                }
                Some(AstNodeData::UnaryOp { .. } | AstNodeData::BinaryOp { .. }) => Node::Single,
                Some(AstNodeData::Function { name_id, .. }) => {
                    let name = data_store.resolve_ast_string(*name_id);
                    let args = data_store.get_args(id).unwrap_or(&[]);
                    match args.split_first() {
                        Some((callee, args)) if name == CALL_EXPRESSION_NAME => Node::Call(
                            Box::new(child(*callee)),
                            Args::Arena(args, data_store, sheet_registry),
                        ),
                        _ => Node::Function(name, Args::Arena(args, data_store, sheet_registry)),
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct Typer<'a, 'b> {
    /// The names bound inside the expression, innermost last.
    scope: Vec<(&'a str, Local<'a, 'b>)>,
    depth: usize,
}

impl<'a, 'b> Typer<'a, 'b> {
    fn shape(&mut self, arg: &ArgumentHandle<'a, 'b>) -> ResultShape {
        if self.depth >= MAX_DEPTH {
            return ResultShape::Unknown;
        }
        self.depth += 1;
        let shape = match node(arg) {
            Node::Single => ResultShape::Single,
            Node::Array => ResultShape::Array,
            Node::Name(name) => self.name(arg, name),
            Node::Operator(operands) => {
                operands.iter().fold(ResultShape::Single, |shape, operand| {
                    shape.elementwise(self.shape(operand))
                })
            }
            Node::Function(name, args) => self.function(arg, name, args),
            Node::Call(callee, args) => self.call(&callee, args),
        };
        self.depth -= 1;
        shape
    }

    fn local(&self, name: &str) -> Option<&Local<'a, 'b>> {
        self.scope
            .iter()
            .rev()
            .find(|(bound, _)| bound.eq_ignore_ascii_case(name))
            .map(|(_, local)| local)
    }

    /// A name: one bound in the expression, a LET name or LAMBDA parameter of
    /// the formula around it, or a defined name.
    fn name(&self, at: &ArgumentHandle<'a, 'b>, name: &str) -> ResultShape {
        if let Some(local) = self.local(name) {
            return match local {
                Local::Shape(shape) => *shape,
                Local::Lambda { .. } => ResultShape::Single,
            };
        }
        if let Some(shape) = at.interp.local_env().binding_shape(name) {
            return shape;
        }
        // A name for a cell or a range is a reference; a name for a constant
        // or a formula may hold an array.
        match at
            .interp
            .context
            .resolve_name_reference(name, at.interp.current_sheet())
        {
            Some(Ok(_)) => ResultShape::Single,
            _ => ResultShape::Unknown,
        }
    }

    fn function(
        &mut self,
        at: &ArgumentHandle<'a, 'b>,
        name: &'a str,
        args: Args<'a>,
    ) -> ResultShape {
        let Some(fun) = at.interp.context.get_function("", name) else {
            // A LET name or LAMBDA parameter called as a function.
            return self.call_name(at, name, args);
        };
        let count = args.len();
        let arg = |index: usize| args.get(at, index);
        match fun.name() {
            "LET" => self.let_shape(at, args),
            // A LAMBDA that is not called is a function, not an array.
            "LAMBDA" => ResultShape::Single,
            "IF" | "CHOOSE" => {
                if count < 2 {
                    return ResultShape::Single;
                }
                let selector = self.shape(&arg(0));
                if selector == ResultShape::Array {
                    return ResultShape::Array;
                }
                let mut results: Vec<ResultShape> =
                    (1..count).map(|i| self.shape(&arg(i))).collect();
                if fun.name() == "IF" && count == 2 {
                    // FALSE when the test fails and value_if_false is missing.
                    results.push(ResultShape::Single);
                }
                if results.iter().all(|shape| *shape == results[0]) {
                    return results[0];
                }
                if selector != ResultShape::Single || self.mentions_local(&arg(0)) {
                    return ResultShape::Unknown;
                }
                // The argument the test or index selects, evaluated as the
                // call evaluates it.
                match at
                    .with_call_handles(fun.as_ref(), |handles| selected_branch(fun.name(), handles))
                    .flatten()
                {
                    Some(index) => results[index - 1],
                    None => ResultShape::Unknown,
                }
            }
            "IFERROR" | "IFNA" => {
                if count != 2 {
                    return ResultShape::Single;
                }
                match self.shape(&arg(0)) {
                    // Each element of an array, or its replacement.
                    ResultShape::Array => ResultShape::Array,
                    value => value.either(self.shape(&arg(1))),
                }
            }
            "IFS" => {
                if count < 2 {
                    return ResultShape::Single;
                }
                let tests = (0..count).step_by(2).fold(ResultShape::Single, |shape, i| {
                    shape.elementwise(self.shape(&arg(i)))
                });
                if tests == ResultShape::Array {
                    return ResultShape::Array;
                }
                let values = (1..count)
                    .step_by(2)
                    .map(|i| self.shape(&arg(i)))
                    .reduce(ResultShape::either)
                    .unwrap_or(ResultShape::Single);
                tests.elementwise(values)
            }
            "SWITCH" => {
                if count < 3 {
                    return ResultShape::Single;
                }
                let expression = self.shape(&arg(0));
                if expression == ResultShape::Array {
                    return ResultShape::Array;
                }
                // The results follow each value; a default ends an even count.
                let mut results: Vec<usize> = (2..count).step_by(2).collect();
                if count.is_multiple_of(2) {
                    results.push(count - 1);
                }
                let results = results
                    .into_iter()
                    .map(|i| self.shape(&arg(i)))
                    .reduce(ResultShape::either)
                    .unwrap_or(ResultShape::Single);
                expression.elementwise(results)
            }
            _ => {
                // Evaluated once per element of an array given for a single value.
                let mut lifted = ResultShape::Single;
                for index in 0..count {
                    if matches!(
                        crate::lift::legacy_arg(fun.as_ref(), index),
                        LegacyArg::Value | LegacyArg::ForcedValue
                    ) {
                        lifted = lifted.elementwise(self.shape(&arg(index)));
                        if lifted == ResultShape::Array {
                            return ResultShape::Array;
                        }
                    }
                }
                match fun.name() {
                    // A value, a reference or an array, as the data has it.
                    "INDEX" | "XLOOKUP" | "FILTERXML" | "REDUCE" => ResultShape::Unknown,
                    // The functions that return arrays.
                    _ if fun.caps().contains(crate::function::FnCaps::MAY_SPILL) => {
                        ResultShape::Array
                    }
                    _ => lifted,
                }
            }
        }
    }

    /// `LET(name1, value1, ..., calculation)`: the calculation's shape, with
    /// each name the shape of its value.
    fn let_shape(&mut self, at: &ArgumentHandle<'a, 'b>, args: Args<'a>) -> ResultShape {
        let count = args.len();
        if count < 3 || count.is_multiple_of(2) {
            return ResultShape::Single;
        }
        let outer = self.scope.len();
        for pair in (0..count - 1).step_by(2) {
            let Some(name) = args.get(at, pair).name_reference() else {
                self.scope.truncate(outer);
                return ResultShape::Single;
            };
            let local = self.bound(&args.get(at, pair + 1));
            self.scope.push((name, local));
        }
        let shape = self.shape(&args.get(at, count - 1));
        self.scope.truncate(outer);
        shape
    }

    /// What a LET name is bound to: a LAMBDA written there, or a value.
    fn bound(&mut self, value: &ArgumentHandle<'a, 'b>) -> Local<'a, 'b> {
        if let Some((params, body)) = self.lambda(value) {
            return Local::Lambda {
                params,
                body: Box::new(body),
                scope: self.scope.len(),
            };
        }
        Local::Shape(self.shape(value))
    }

    /// The parameter names and body of a LAMBDA written as `value`.
    fn lambda(
        &self,
        value: &ArgumentHandle<'a, 'b>,
    ) -> Option<(Vec<&'a str>, ArgumentHandle<'a, 'b>)> {
        let Node::Function(name, args) = node(value) else {
            return None;
        };
        let fun = value.interp.context.get_function("", name)?;
        if fun.name() != "LAMBDA" || args.len() == 0 {
            return None;
        }
        let params = (0..args.len() - 1)
            .map(|i| args.get(value, i).name_reference())
            .collect::<Option<Vec<_>>>()?;
        Some((params, args.get(value, args.len() - 1)))
    }

    /// `callee(args)`: a LAMBDA written there, or a name bound to one.
    fn call(&mut self, callee: &ArgumentHandle<'a, 'b>, args: Args<'a>) -> ResultShape {
        if let Some((params, body)) = self.lambda(callee) {
            let scope = self.scope.len();
            return self.lambda_result(callee, &params, &body, scope, args);
        }
        match node(callee) {
            Node::Name(name) => self.call_name(callee, name, args),
            _ => ResultShape::Unknown,
        }
    }

    /// A call to the LAMBDA a name is bound to.
    fn call_name(
        &mut self,
        at: &ArgumentHandle<'a, 'b>,
        name: &str,
        args: Args<'a>,
    ) -> ResultShape {
        if let Some(local) = self.local(name) {
            return match local.clone() {
                Local::Lambda {
                    params,
                    body,
                    scope,
                } => self.lambda_result(at, &params, &body, scope, args),
                Local::Shape(_) => ResultShape::Unknown,
            };
        }
        let Some(LocalBinding::Callable(callable)) = at.interp.resolve_local_name(name) else {
            return ResultShape::Unknown;
        };
        let Some((params, body, captured)) = callable.lambda_parts() else {
            return ResultShape::Unknown;
        };
        if params.len() != args.len() {
            return ResultShape::Unknown;
        }
        let shapes: Vec<ResultShape> = (0..args.len())
            .map(|i| self.shape(&args.get(at, i)))
            .collect();
        // The body sees the names captured where the LAMBDA was written.
        let interp = at.interp.with_local_env(captured.clone());
        let body = ArgumentHandle::new(body, &interp);
        let mut typer = Typer {
            scope: params
                .iter()
                .map(String::as_str)
                .zip(shapes.into_iter().map(Local::Shape))
                .collect(),
            depth: self.depth,
        };
        typer.shape(&body)
    }

    /// The shape of a LAMBDA's body called with `args`, seeing the first
    /// `scope` names bound in the expression and its parameters.
    fn lambda_result(
        &mut self,
        at: &ArgumentHandle<'a, 'b>,
        params: &[&'a str],
        body: &ArgumentHandle<'a, 'b>,
        scope: usize,
        args: Args<'a>,
    ) -> ResultShape {
        if params.len() != args.len() {
            return ResultShape::Unknown;
        }
        let shapes: Vec<ResultShape> = (0..args.len())
            .map(|i| self.shape(&args.get(at, i)))
            .collect();
        let mut inner = self.scope[..scope.min(self.scope.len())].to_vec();
        inner.extend(
            params
                .iter()
                .copied()
                .zip(shapes.into_iter().map(Local::Shape)),
        );
        let outer = std::mem::replace(&mut self.scope, inner);
        let shape = self.shape(body);
        self.scope = outer;
        shape
    }

    /// Whether `arg` reads a name bound inside the expression, which has no
    /// value outside its evaluation.
    fn mentions_local(&self, arg: &ArgumentHandle<'a, 'b>) -> bool {
        if self.scope.is_empty() {
            return false;
        }
        match node(arg) {
            Node::Single | Node::Array => false,
            Node::Name(name) => self.local(name).is_some(),
            Node::Operator(operands) => operands.iter().any(|operand| self.mentions_local(operand)),
            Node::Function(name, args) => {
                self.local(name).is_some()
                    || (0..args.len()).any(|i| self.mentions_local(&args.get(arg, i)))
            }
            Node::Call(callee, args) => {
                self.mentions_local(&callee)
                    || (0..args.len()).any(|i| self.mentions_local(&args.get(arg, i)))
            }
        }
    }
}
