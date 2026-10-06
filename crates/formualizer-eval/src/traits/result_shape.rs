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
//!   ([`crate::lift::legacy_arg`]) or that it evaluates once per element of
//!   an array ([`crate::lift::lift_spec`]: N and T too), return an array when
//!   an operand or such an argument is one;
//! - a function that returns arrays (SEQUENCE, SORT, TRANSPOSE, MMULT, ...)
//!   returns one, XMATCH only for an array of lookup values; INDEX and
//!   XLOOKUP over a reference or a single value return a reference or a
//!   value; any other function returns a single value;
//! - IF and CHOOSE return the argument their single-value test or index
//!   selected, IFERROR and IFNA their value or its replacement, as their
//!   evaluation recorded it (see [`track_selections`]): the shape is read
//!   without evaluating anything again, so no test is calculated twice and
//!   no branch the formula did not take is calculated at all. IFS and SWITCH
//!   return one of their value arguments, and LET its calculation, with each
//!   name the shape of its value; a LAMBDA called with arguments returns its
//!   body's shape with each parameter the shape of its argument.
//!
//! Where that depends on more than the expression (INDEX with a row or column
//! of 0 over an array, the item XLOOKUP returns from an array, the nodes
//! FILTERXML finds, REDUCE's accumulator, an IF whose selection was not
//! recorded) the shape is unknown.

use super::{ArgumentExpr, ArgumentHandle};
use crate::engine::arena::{
    AstNodeData, AstNodeId, CALL_EXPRESSION_NAME, CompactRefType, DataStore, ValueType,
};
use crate::engine::sheet_registry::SheetRegistry;
use crate::interpreter::LocalBinding;
use crate::lift::LegacyArg;
use formualizer_common::LiteralValue;
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};

/// A node of a formula, by identity: a node of a parsed tree by its address,
/// an arena node by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum NodeKey {
    Ast(usize),
    Arena(AstNodeId),
}

/// What a recorded call calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Callee {
    /// A builtin (IF, CHOOSE, IFERROR, IFNA, IFS, SWITCH), by name.
    Function(&'static str),
    /// A LET name or LAMBDA parameter bound to a LAMBDA, by its name in
    /// upper case, hashed.
    Name(u64),
    /// A LAMBDA written as the callee of `callee(args)`.
    Expression(NodeKey),
    /// A LAMBDA, by the closure called (see
    /// [`crate::traits::CustomCallable`]'s identity).
    Callable(usize),
    /// A LAMBDA, by the body node it was written with, hashed.
    Written(u64),
}

/// The key of the body node a LAMBDA is written with (see
/// [`crate::traits::CustomCallable::written_body`]).
pub(crate) fn written_body_key(body: &ArgumentHandle<'_, '_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    handle_key(body).hash(&mut hasher);
    hasher.finish()
}

fn callable_key(callable: &std::sync::Arc<dyn crate::traits::CustomCallable>) -> usize {
    std::sync::Arc::as_ptr(callable).cast::<()>() as usize
}

/// A call in a formula: what it calls and its arguments (their nodes,
/// hashed, and their count). A formula's arena shares the nodes of equal
/// expressions, so equal calls written twice are one call here: when they
/// record different things the record is [`Recorded::Ambiguous`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CallKey {
    callee: Callee,
    args: u64,
    count: usize,
}

impl CallKey {
    fn new(callee: Callee, args: impl Iterator<Item = NodeKey>) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        let mut count = 0;
        for arg in args {
            arg.hash(&mut hasher);
            count += 1;
        }
        CallKey {
            callee,
            args: hasher.finish(),
            count,
        }
    }
}

/// What a call recorded about its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recorded {
    /// The argument a selecting call returned: `Some(index)` for the
    /// argument whose value it returned, `None` for a single value of its
    /// own (an error, FALSE).
    Selection(Option<usize>),
    /// The shape of a LAMBDA's result, read from its body as it ran.
    Shape(ResultShape),
    /// Different records for one call key (equal calls under different LET
    /// bindings, say): nothing is known.
    Ambiguous,
}

thread_local! {
    /// The records of each tracking scope on this thread, innermost last;
    /// a scope that ends passes its records to the one around it.
    static RECORDS: RefCell<Vec<FxHashMap<CallKey, Recorded>>> = const { RefCell::new(Vec::new()) };
    /// Arguments that run replaced by a node of their own (a range in a
    /// single-value position of a formula entered without the array flag,
    /// replaced by its intersection with the formula cell): the replacement
    /// node's address and the arena node it stands for.
    static REPLACED: RefCell<Vec<(usize, AstNodeId)>> = const { RefCell::new(Vec::new()) };
    /// Whether the call site invoking a LAMBDA now records its result shape.
    static LAMBDA_RESULT_WANTED: Cell<bool> = const { Cell::new(false) };
    /// How many reference-only probes are running: what they calculate is
    /// discarded, so nothing they run is recorded.
    static PROBING: Cell<usize> = const { Cell::new(0) };
    /// The shape of the result of the LAMBDA such a call site invoked.
    static LAMBDA_RESULT: Cell<Option<ResultShape>> = const { Cell::new(None) };
}

/// Record which argument the selecting calls evaluated from now on return,
/// and the shape of the results of the LAMBDAs called, until the returned
/// guard drops, so that the shape of their result can be read afterwards
/// without evaluating anything again. A caller that reads
/// [`ArgumentHandle::result_shape`] holds one while it evaluates the
/// argument. Records are kept while any scope is open: the records of a
/// scope that ends go to the scope around it.
pub(crate) fn track_selections() -> SelectionTracking {
    RECORDS.with(|records| {
        if let Ok(mut records) = records.try_borrow_mut() {
            records.push(FxHashMap::default());
        }
    });
    SelectionTracking(())
}

/// See [`track_selections`].
pub(crate) struct SelectionTracking(());

impl Drop for SelectionTracking {
    fn drop(&mut self) {
        let outermost = RECORDS.with(|records| {
            let Ok(mut records) = records.try_borrow_mut() else {
                return false;
            };
            let Some(ended) = records.pop() else {
                return true;
            };
            match records.last_mut() {
                Some(outer) => {
                    for (key, recorded) in ended {
                        insert_into(outer, key, recorded);
                    }
                    false
                }
                None => true,
            }
        });
        if outermost {
            LAMBDA_RESULT.with(|shape| shape.set(None));
            LAMBDA_RESULT_WANTED.with(|wanted| wanted.set(false));
        }
    }
}

/// While something tracks selections, track those of a body evaluated now
/// that is a copy made for this evaluation only (a LAMBDA's, called): its
/// records are its own and go with the returned guard, not to the scope
/// around it, since the copy's nodes go too and others may take their
/// places. `None` when nothing tracks.
pub(crate) fn track_own_selections() -> Option<OwnSelections> {
    if !tracking() {
        return None;
    }
    RECORDS.with(|records| {
        let mut records = records.try_borrow_mut().ok()?;
        records.push(FxHashMap::default());
        Some(OwnSelections(()))
    })
}

/// See [`track_own_selections`].
pub(crate) struct OwnSelections(());

impl Drop for OwnSelections {
    fn drop(&mut self) {
        RECORDS.with(|records| {
            if let Ok(mut records) = records.try_borrow_mut() {
                records.pop();
            }
        });
    }
}

fn tracking() -> bool {
    RECORDS.with(|records| {
        records
            .try_borrow()
            .is_ok_and(|records| !records.is_empty())
    })
}

fn insert_into(records: &mut FxHashMap<CallKey, Recorded>, key: CallKey, recorded: Recorded) {
    records
        .entry(key)
        .and_modify(|held| {
            if *held != recorded {
                *held = Recorded::Ambiguous;
            }
        })
        .or_insert(recorded);
}

/// While the returned guard lives, a reference is resolved and any value
/// found on the way is discarded (MATCH trying its lookup_array as a
/// reference, IF/CHOOSE/IFS resolved as one): what runs records nothing, as
/// the value used comes from another evaluation.
pub(crate) fn probe_references() -> ReferenceProbe {
    PROBING.with(|depth| depth.set(depth.get() + 1));
    ReferenceProbe(())
}

/// See [`probe_references`].
pub(crate) struct ReferenceProbe(());

impl Drop for ReferenceProbe {
    fn drop(&mut self) {
        PROBING.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn insert_record(key: CallKey, recorded: Recorded) {
    if PROBING.with(Cell::get) > 0 {
        return;
    }
    RECORDS.with(|records| {
        if let Ok(mut records) = records.try_borrow_mut()
            && let Some(innermost) = records.last_mut()
        {
            insert_into(innermost, key, recorded);
        }
    });
}

/// While tracked, the arguments of the call about to run that were
/// replaced by a node of their own (`(replacement, original)`), so that the
/// call is recorded under the nodes written in the formula. Forgotten when
/// the returned guard drops.
pub(crate) fn note_replaced_arguments<'n>(
    replaced: impl Iterator<Item = (&'n ASTNode, AstNodeId)>,
) -> ReplacedArguments {
    let mut count = 0;
    if tracking() {
        REPLACED.with(|noted| {
            if let Ok(mut noted) = noted.try_borrow_mut() {
                for (node, id) in replaced {
                    noted.push((std::ptr::from_ref(node) as usize, id));
                    count += 1;
                }
            }
        });
    }
    ReplacedArguments(count)
}

/// See [`note_replaced_arguments`].
pub(crate) struct ReplacedArguments(usize);

impl Drop for ReplacedArguments {
    fn drop(&mut self) {
        if self.0 > 0 {
            REPLACED.with(|noted| {
                if let Ok(mut noted) = noted.try_borrow_mut() {
                    let keep = noted.len().saturating_sub(self.0);
                    noted.truncate(keep);
                }
            });
        }
    }
}

fn handle_key(arg: &ArgumentHandle<'_, '_>) -> NodeKey {
    match arg.expr {
        ArgumentExpr::Ast(node) => {
            let address = std::ptr::from_ref(node) as usize;
            let original = REPLACED.with(|noted| {
                noted.try_borrow().ok().and_then(|noted| {
                    noted
                        .iter()
                        .rev()
                        .find(|(replacement, _)| *replacement == address)
                        .map(|(_, id)| *id)
                })
            });
            match original {
                Some(id) => NodeKey::Arena(id),
                None => NodeKey::Ast(address),
            }
        }
        ArgumentExpr::Arena { id, .. } => NodeKey::Arena(id),
    }
}

/// The selecting builtin `function` (IF, CHOOSE, IFERROR, IFNA, IFS or SWITCH),
/// evaluated with `args`, returned the value of argument `returned` (`None`:
/// a single value of its own). Kept only while a caller tracks selections.
pub(crate) fn record_selection(
    function: &'static str,
    args: &[ArgumentHandle<'_, '_>],
    returned: Option<usize>,
) {
    if !tracking() {
        return;
    }
    insert_record(
        CallKey::new(Callee::Function(function), args.iter().map(handle_key)),
        Recorded::Selection(returned),
    );
}

/// A call site written in a formula is about to invoke a callable: a LAMBDA
/// reports the shape of its result for it (see [`record_lambda_call`]).
pub(crate) fn want_lambda_result() {
    if tracking() {
        LAMBDA_RESULT.with(|result| result.set(None));
        LAMBDA_RESULT_WANTED.with(|wanted| wanted.set(true));
    }
}

/// Whether the LAMBDA being invoked reports the shape of its result: only
/// to a call site written in a formula that records it (not to MAP, REDUCE,
/// ... calling it per element). Asked once, as the LAMBDA starts.
pub(crate) fn lambda_result_wanted() -> bool {
    LAMBDA_RESULT_WANTED.with(|wanted| wanted.replace(false))
}

/// A LAMBDA whose result shape is wanted reports it, read from its body
/// (`body`, evaluated with its parameters bound) with the selections its
/// evaluation recorded.
pub(crate) fn report_lambda_result(body: &ArgumentHandle<'_, '_>) {
    if tracking() {
        let shape = of_without_names(body);
        LAMBDA_RESULT.with(|result| result.set(Some(shape)));
    }
}

fn name_hash(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    name.to_ascii_uppercase().hash(&mut hasher);
    hasher.finish()
}

/// The call written as `name(args)` (`callee` `None`) or `callee(args)` just
/// invoked `callable` with `args`: if it was a LAMBDA, record the shape of
/// its result it reported, under the call as written and under the closure
/// called and the body it was written with (equal calls written twice may
/// call different LAMBDAs: `f(x)` under two LET bindings of `f`).
pub(crate) fn record_lambda_call(
    name: Option<&str>,
    callee: Option<&ArgumentHandle<'_, '_>>,
    callable: Option<&std::sync::Arc<dyn crate::traits::CustomCallable>>,
    args: &[ArgumentHandle<'_, '_>],
) {
    if !tracking() {
        return;
    }
    LAMBDA_RESULT_WANTED.with(|wanted| wanted.set(false));
    let Some(shape) = LAMBDA_RESULT.with(|result| result.take()) else {
        return;
    };
    let written = match (name, callee) {
        (Some(name), _) => Some(Callee::Name(name_hash(name))),
        (None, Some(callee)) => Some(Callee::Expression(handle_key(callee))),
        (None, None) => None,
    };
    let callees = written
        .into_iter()
        .chain(callable.into_iter().flat_map(|callable| {
            std::iter::once(Callee::Callable(callable_key(callable)))
                .chain(callable.written_body().map(Callee::Written))
        }));
    for callee in callees {
        insert_record(
            CallKey::new(callee, args.iter().map(handle_key)),
            Recorded::Shape(shape),
        );
    }
}

/// What an expression evaluates to in Excel's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResultShape {
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

/// An expression with more parts than this (counting the body of each
/// LAMBDA it calls once per distinct call) is of unknown shape: a bound no
/// formula's text reaches.
const MAX_VISITS: usize = 1 << 20;

/// A LAMBDA call already typed: the LAMBDA (the node of its body, or the
/// callable), the shapes of its arguments and of the names it sees.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TypedCall {
    lambda: TypedLambda,
    args: Vec<ResultShape>,
    scope: u64,
}

/// The parameter names of a LAMBDA, in upper case, hashed.
fn params_signature<'p>(params: impl Iterator<Item = &'p str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    for param in params {
        param.to_ascii_uppercase().hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TypedLambda {
    /// A LAMBDA written in the formula: its body and its parameter names.
    Body(NodeKey, u64),
    Callable(usize),
}

pub(super) fn of(arg: &ArgumentHandle<'_, '_>) -> ResultShape {
    Typer::default().shape(arg)
}

/// [`of`] without resolving defined names, which may calculate their
/// formulas: a name not bound in the formula is of unknown shape.
pub(super) fn of_without_names(arg: &ArgumentHandle<'_, '_>) -> ResultShape {
    Typer {
        without_names: true,
        ..Typer::default()
    }
    .shape(arg)
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

    /// What the call of `callee` with these arguments recorded when it last
    /// ran (see [`record_selection`], [`record_lambda_call`]), in the
    /// innermost tracking scope that holds a record of it; `None` when
    /// nothing is recorded.
    fn recorded(&self, callee: Callee) -> Option<Recorded> {
        let key = match *self {
            Args::Ast(nodes) => CallKey::new(
                callee,
                nodes
                    .iter()
                    .map(|node| NodeKey::Ast(std::ptr::from_ref(node) as usize)),
            ),
            Args::Arena(ids, ..) => CallKey::new(callee, ids.iter().map(|&id| NodeKey::Arena(id))),
        };
        RECORDS.with(|records| {
            let records = records.try_borrow().ok()?;
            records
                .iter()
                .rev()
                .find_map(|scope| scope.get(&key).copied())
        })
    }

    /// The argument the selecting builtin `function` returned when it last
    /// ran with these arguments (see [`record_selection`]).
    fn recorded_selection(&self, function: &'static str) -> Option<Option<usize>> {
        match self.recorded(Callee::Function(function))? {
            Recorded::Selection(returned) => Some(returned),
            Recorded::Shape(_) | Recorded::Ambiguous => None,
        }
    }

    /// The shape of the result of the LAMBDA the call of `callee` with these
    /// arguments last invoked (see [`record_lambda_call`]), when known.
    fn recorded_lambda_result(&self, callee: Callee) -> Option<ResultShape> {
        match self.recorded(callee)? {
            Recorded::Shape(ResultShape::Unknown) => None,
            Recorded::Shape(shape) => Some(shape),
            Recorded::Selection(_) | Recorded::Ambiguous => None,
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
                    ..
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
    /// The parts typed so far (bodies of the LAMBDAs called included).
    visits: usize,
    /// The LAMBDA calls typed so far: a body is typed once per distinct
    /// call, not once per time it is written.
    calls: FxHashMap<TypedCall, ResultShape>,
    /// Defined names are of unknown shape (their formulas are not
    /// calculated to resolve them).
    without_names: bool,
}

impl<'a, 'b> Typer<'a, 'b> {
    fn shape(&mut self, arg: &ArgumentHandle<'a, 'b>) -> ResultShape {
        if self.depth >= MAX_DEPTH || self.visits >= MAX_VISITS {
            return ResultShape::Unknown;
        }
        self.depth += 1;
        self.visits += 1;
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
        if self.without_names {
            return ResultShape::Unknown;
        }
        // A name for a cell or a range is a reference; a name for a constant
        // or a formula may hold an array. Only its shape is read here: what
        // resolving it drew is drawn again where it is evaluated.
        let resolved = crate::interpreter::EvaluationScope::probe(
            || {
                at.interp
                    .context
                    .resolve_name_reference(name, at.interp.current_sheet())
            },
            |_| false,
        );
        match resolved {
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
                // The argument the test or index selected: a constant one
                // here, else as the call recorded it when it ran. Nothing
                // is evaluated to find it, and only that argument is typed.
                let selected = match literal(&arg(0)) {
                    Some(selector) => Some(constant_selection(fun.name(), selector, count)),
                    None => args.recorded_selection(fun.name()),
                };
                match selected {
                    Some(Some(index)) if index < count => return self.shape(&arg(index)),
                    // FALSE when the test fails and value_if_false is
                    // missing, or an error of the call's own.
                    Some(_) => return ResultShape::Single,
                    None => {}
                }
                // Not known: the shape all of them share, if any.
                let mut results: Vec<ResultShape> =
                    (1..count).map(|i| self.shape(&arg(i))).collect();
                if fun.name() == "IF" && count == 2 {
                    results.push(ResultShape::Single);
                }
                if results.iter().all(|shape| *shape == results[0]) {
                    results[0]
                } else {
                    ResultShape::Unknown
                }
            }
            "IFERROR" | "IFNA" => {
                if count != 2 {
                    return ResultShape::Single;
                }
                let value = match self.shape(&arg(0)) {
                    // Each element of an array, or its replacement.
                    ResultShape::Array => return ResultShape::Array,
                    value => value,
                };
                // The value or its replacement, as the call recorded it.
                match args.recorded_selection(fun.name()) {
                    Some(Some(0)) => value,
                    Some(Some(1)) => self.shape(&arg(1)),
                    _ => value.either(self.shape(&arg(1))),
                }
            }
            "IFS" => {
                if count < 2 {
                    return ResultShape::Single;
                }
                // The value the call returned, as it recorded it, after the
                // tests it calculated: an array among them makes the call
                // element-wise.
                if let Some(selected) = args.recorded_selection(fun.name()) {
                    let tested = selected.map_or(count, |index| index);
                    let tests = (0..tested)
                        .step_by(2)
                        .fold(ResultShape::Single, |shape, i| {
                            shape.elementwise(self.shape(&arg(i)))
                        });
                    return match (tests, selected) {
                        (ResultShape::Array, _) => ResultShape::Array,
                        (tests, Some(index)) if index < count => {
                            tests.elementwise(self.shape(&arg(index)))
                        }
                        (tests, _) => tests,
                    };
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
                // The result the call returned, as it recorded it, unless
                // the expression is an array (the call is element-wise).
                let expression = self.shape(&arg(0));
                if expression == ResultShape::Array {
                    return ResultShape::Array;
                }
                match args.recorded_selection(fun.name()) {
                    Some(Some(index)) if index < count => {
                        return expression.elementwise(self.shape(&arg(index)));
                    }
                    Some(_) => return expression,
                    None => {}
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
                // Evaluated once per element of an array given for a single
                // value, or for a parameter the call is lifted over (N's and
                // T's reference: `N({1})` is a one-element array).
                let lifts = crate::lift::lift_spec(fun.name());
                let mut lifted = ResultShape::Single;
                for index in 0..count {
                    if matches!(
                        crate::lift::legacy_arg(fun.as_ref(), index),
                        LegacyArg::Value | LegacyArg::ForcedValue
                    ) || lifts.is_some_and(|spec| spec.lifts(index))
                    {
                        lifted = lifted.elementwise(self.shape(&arg(index)));
                        if lifted == ResultShape::Array {
                            return ResultShape::Array;
                        }
                    }
                }
                match fun.name() {
                    // A reference (or a value) when the array argument is a
                    // reference (or a value): `INDEX(A1:A3,1)+0` is a single
                    // value. Over an array, a value or an array as the data
                    // and the indexes have it.
                    "INDEX" if count >= 1 && self.reference_or_value(&arg(0)) => {
                        ResultShape::Single
                    }
                    // The return array's item, a reference into it when it is
                    // one, or the value given for no match.
                    "XLOOKUP" if count >= 3 => {
                        let found = if self.reference_or_value(&arg(2)) {
                            ResultShape::Single
                        } else {
                            ResultShape::Unknown
                        };
                        match count {
                            3 => found,
                            _ => found.either(self.shape(&arg(3))),
                        }
                    }
                    "INDEX" | "XLOOKUP" | "FILTERXML" | "REDUCE" => ResultShape::Unknown,
                    // A position per lookup value: an array of them only for
                    // an array of lookup values.
                    "XMATCH" => lifted,
                    // The functions that return arrays.
                    _ if fun.caps().contains(crate::function::FnCaps::MAY_SPILL) => {
                        ResultShape::Array
                    }
                    _ => lifted,
                }
            }
        }
    }

    /// Whether `arg` is written as a reference or a single value, or is a
    /// name for one: not an expression, which INDEX's and XLOOKUP's array
    /// parameters evaluate as an array.
    fn reference_or_value(&mut self, arg: &ArgumentHandle<'a, 'b>) -> bool {
        match node(arg) {
            Node::Single => true,
            Node::Name(_) => self.shape(arg) == ResultShape::Single,
            _ => false,
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

    /// `callee(args)`: the shape its result had when it ran, if recorded (the
    /// LAMBDA ran with a copy of the body written here), else that of a
    /// LAMBDA written there, or of a name bound to one.
    fn call(&mut self, callee: &ArgumentHandle<'a, 'b>, args: Args<'a>) -> ResultShape {
        if let Some(shape) = args.recorded_lambda_result(Callee::Expression(handle_key(callee))) {
            return shape;
        }
        if let Some((params, body)) = self.lambda(callee) {
            let scope = self.scope.len();
            return self.lambda_result(callee, &params, &body, scope, args);
        }
        match node(callee) {
            Node::Name(name) => self.call_name(callee, name, args),
            _ => ResultShape::Unknown,
        }
    }

    /// A call to the LAMBDA a name is bound to: the shape its result had when
    /// it ran, if recorded (the LAMBDA ran with its own copy of the body, so
    /// the selections in it are recorded there), else its body's shape.
    fn call_name(
        &mut self,
        at: &ArgumentHandle<'a, 'b>,
        name: &str,
        args: Args<'a>,
    ) -> ResultShape {
        // The LAMBDA the name is bound to, as the call that ran called it.
        let lambda = match self.local(name) {
            Some(Local::Lambda { body, .. }) => Some(Callee::Written(written_body_key(body))),
            Some(Local::Shape(_)) => None,
            None => match at.interp.resolve_local_name(name) {
                Some(LocalBinding::Callable(callable)) => {
                    Some(Callee::Callable(callable_key(&callable)))
                }
                _ => None,
            },
        };
        let recorded = lambda
            .and_then(|lambda| args.recorded_lambda_result(lambda))
            .or_else(|| args.recorded_lambda_result(Callee::Name(name_hash(name))));
        if let Some(shape) = recorded {
            return shape;
        }
        self.call_name_body(at, name, args)
    }

    /// [`Self::call_name`] read from the body of the LAMBDA the name is bound to.
    fn call_name_body(
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
        let typed = TypedCall {
            lambda: TypedLambda::Callable(std::sync::Arc::as_ptr(&callable).cast::<()>() as usize),
            args: shapes.clone(),
            scope: 0,
        };
        if let Some(shape) = self.calls.get(&typed) {
            return *shape;
        }
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
            visits: self.visits,
            calls: std::mem::take(&mut self.calls),
            without_names: self.without_names,
        };
        let shape = typer.shape(&body);
        self.visits = typer.visits;
        self.calls = std::mem::take(&mut typer.calls);
        self.calls.insert(typed, shape);
        shape
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
        let visible = &self.scope[..scope.min(self.scope.len())];
        let typed = TypedCall {
            lambda: TypedLambda::Body(handle_key(body), params_signature(params.iter().copied())),
            args: shapes.clone(),
            scope: scope_signature(visible),
        };
        if let Some(shape) = self.calls.get(&typed) {
            return *shape;
        }
        let mut inner = visible.to_vec();
        inner.extend(
            params
                .iter()
                .copied()
                .zip(shapes.into_iter().map(Local::Shape)),
        );
        let outer = std::mem::replace(&mut self.scope, inner);
        let shape = self.shape(body);
        self.scope = outer;
        self.calls.insert(typed, shape);
        shape
    }
}

/// The constant written as `arg`, if it is one (not an array).
fn literal(arg: &ArgumentHandle<'_, '_>) -> Option<LiteralValue> {
    let value = match arg.expr {
        ArgumentExpr::Ast(ast) => match &ast.node_type {
            ASTNodeType::Literal(value) => value.clone(),
            _ => return None,
        },
        ArgumentExpr::Arena { id, data_store, .. } => match data_store.get_node(id) {
            Some(AstNodeData::Literal(value)) => data_store.retrieve_value(*value),
            _ => return None,
        },
    };
    (!matches!(value, LiteralValue::Array(_))).then_some(value)
}

/// The argument IF or CHOOSE with `count` arguments returns for the constant
/// test or index `selector`, as [`record_selection`] records it.
fn constant_selection(name: &str, selector: LiteralValue, count: usize) -> Option<usize> {
    if name == "IF" {
        return match crate::builtins::logical::if_condition(selector) {
            Ok(true) => Some(1),
            Ok(false) => Some(2),
            Err(_) => None,
        };
    }
    let index = match selector {
        LiteralValue::Number(n) => n as i64,
        LiteralValue::Int(i) => i,
        _ => return None,
    };
    (index >= 1 && (index as usize) < count).then_some(index as usize)
}

/// The names a LAMBDA body sees (`scope`), as a call of it is typed: each
/// name with the shape of its value or the body of its LAMBDA.
fn scope_signature(scope: &[(&str, Local<'_, '_>)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    for (name, local) in scope {
        name.to_ascii_uppercase().hash(&mut hasher);
        match local {
            Local::Shape(shape) => shape.hash(&mut hasher),
            Local::Lambda {
                params,
                body,
                scope,
            } => {
                handle_key(body).hash(&mut hasher);
                params_signature(params.iter().copied()).hash(&mut hasher);
                scope.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}
