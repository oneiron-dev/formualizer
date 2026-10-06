use crate::engine::lookup_index_cache::{LookupAxis, LookupIndex};
use crate::engine::range_view::RangeView;
use crate::engine::row_visibility::VisibilityMaskMode;
pub use crate::function::Function;
use crate::interpreter::Interpreter;
use crate::reference::CellRef;
use formualizer_common::{
    LiteralValue,
    error::{ExcelError, ExcelErrorKind},
};
use std::any::Any;
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::Arc;

use formualizer_parse::parser::{
    ASTNode, ASTNodeType, ReferenceType, TableReference, TableSpecifier,
};

mod result_shape;
pub(crate) use result_shape::{
    ResultShape, lambda_result_wanted, note_replaced_arguments, probe_references,
    record_lambda_call, record_selection, report_lambda_result, track_own_selections,
    track_selections, want_lambda_result, written_body_key,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceInfo {
    /// Excel-style 1-based index of the first sheet covered by the reference.
    pub first_sheet_index: Option<usize>,
    /// Number of sheets covered by the reference (`1` for ordinary references, `N` for 3D refs).
    pub sheet_count: Option<usize>,
    /// Top-left / first cell addressed by the reference, when it resolves to a concrete cell.
    pub first_cell: Option<CellRef>,
}

/* ───────────────────────────── Range ───────────────────────────── */

pub trait Range: Debug + Send + Sync {
    fn get(&self, row: usize, col: usize) -> Result<LiteralValue, ExcelError>;
    fn dimensions(&self) -> (usize, usize);

    fn is_sparse(&self) -> bool {
        false
    }

    // Handle infinite ranges (A:A, 1:1)
    fn is_infinite(&self) -> bool {
        false
    }

    fn materialise(&self) -> Cow<'_, [Vec<LiteralValue>]> {
        Cow::Owned(
            (0..self.dimensions().0)
                .map(|r| {
                    (0..self.dimensions().1)
                        .map(|c| self.get(r, c).unwrap_or(LiteralValue::Empty))
                        .collect()
                })
                .collect(),
        )
    }

    fn iter_cells<'a>(&'a self) -> Box<dyn Iterator<Item = LiteralValue> + 'a> {
        let (rows, cols) = self.dimensions();
        Box::new((0..rows).flat_map(move |r| (0..cols).map(move |c| self.get(r, c).unwrap())))
    }
    fn iter_rows<'a>(&'a self) -> Box<dyn Iterator<Item = Vec<LiteralValue>> + 'a> {
        let (rows, cols) = self.dimensions();
        Box::new((0..rows).map(move |r| (0..cols).map(|c| self.get(r, c).unwrap()).collect()))
    }

    /* down-cast hook for SIMD back-ends */
    fn as_any(&self) -> &dyn Any;
}

/* blanket dyn passthrough */
impl Range for Box<dyn Range> {
    fn get(&self, r: usize, c: usize) -> Result<LiteralValue, ExcelError> {
        (**self).get(r, c)
    }
    fn dimensions(&self) -> (usize, usize) {
        (**self).dimensions()
    }
    fn is_sparse(&self) -> bool {
        (**self).is_sparse()
    }
    fn materialise(&self) -> Cow<'_, [Vec<LiteralValue>]> {
        (**self).materialise()
    }
    fn iter_cells<'a>(&'a self) -> Box<dyn Iterator<Item = LiteralValue> + 'a> {
        (**self).iter_cells()
    }
    fn iter_rows<'a>(&'a self) -> Box<dyn Iterator<Item = Vec<LiteralValue>> + 'a> {
        (**self).iter_rows()
    }
    fn as_any(&self) -> &dyn Any {
        (**self).as_any()
    }
}

/* ────────────────────── ArgumentHandle helpers ───────────────────── */

pub type CowValue<'a> = Cow<'a, LiteralValue>;

pub trait CustomCallable: Send + Sync {
    fn arity(&self) -> usize;

    /// Whether the callable can be invoked with `count` arguments.
    fn accepts(&self, count: usize) -> bool {
        self.arity() == count
    }

    fn invoke<'ctx>(
        &self,
        interp: &Interpreter<'ctx>,
        args: &[LiteralValue],
    ) -> Result<CalcValue<'ctx>, ExcelError>;

    /// Invokes with the arguments of a call written in a formula
    /// (`LAMBDA(area,ROWS(area))(A:A)`), where an argument written as a reference is
    /// bound as that reference. The default passes each argument's value.
    fn invoke_bindings<'ctx>(
        &self,
        interp: &Interpreter<'ctx>,
        args: Vec<crate::interpreter::LocalBinding>,
    ) -> Result<CalcValue<'ctx>, ExcelError> {
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(interp.binding_value(arg)?.into_literal());
        }
        self.invoke(interp, &values)
    }

    /// [`Self::invoke_bindings`] with the shape each argument value has to
    /// Excel, when it is known ([`ResultShape`]): a LAMBDA parameter bound to
    /// a one-element array held as its single value is an array, one bound
    /// to a single value is not. The default ignores the shapes, and reports
    /// no result shape to the call site (a LAMBDA it calls in turn does not
    /// report one for it either).
    #[doc(hidden)]
    fn invoke_shaped_bindings<'ctx>(
        &self,
        interp: &Interpreter<'ctx>,
        args: Vec<(crate::interpreter::LocalBinding, Option<ResultShape>)>,
    ) -> Result<CalcValue<'ctx>, ExcelError> {
        let _ = result_shape::lambda_result_wanted();
        self.invoke_bindings(interp, args.into_iter().map(|(arg, _)| arg).collect())
    }

    /// The parameters, body and captured local names of a callable written
    /// as a LAMBDA, so that the shape of its result (an array or a single
    /// value) can be read from its body. `None` for other callables.
    #[doc(hidden)]
    fn lambda_parts(&self) -> Option<(&[String], &ASTNode, &crate::interpreter::LocalEnv)> {
        None
    }

    /// For a LAMBDA written in a formula, the node of the body it was
    /// written with (hashed), which its copy of the body stands for when the
    /// shape of a call's result is read. `None` for other callables.
    #[doc(hidden)]
    fn written_body(&self) -> Option<u64> {
        None
    }
}

#[derive(Clone)]
pub enum CalcValue<'a> {
    Scalar(LiteralValue),
    /// Scalar carrying an eval-internal number-format annotation.
    AnnotatedScalar(LiteralValue, crate::format::FormatId),
    Range(RangeView<'a>),
    Callable(Arc<dyn CustomCallable>),
}

/// The result of resolving an argument where either a reference or a value is accepted.
///
/// Reference-shaped syntax is resolved without first evaluating it as a value.
/// All other syntax is evaluated through [`ArgumentHandle::value`] and retains
/// its `CalcValue` discriminant.
#[derive(Clone)]
pub(crate) enum ResolvedArgument<'a> {
    Range(RangeView<'a>),
    ReferenceError(ExcelError),
    Value(CalcValue<'a>),
}

impl std::fmt::Debug for ResolvedArgument<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Range(view) => f.debug_tuple("Range").field(view).finish(),
            Self::ReferenceError(error) => f.debug_tuple("ReferenceError").field(error).finish(),
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
        }
    }
}

impl<'a> std::fmt::Debug for CalcValue<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalcValue::Scalar(v) => f.debug_tuple("Scalar").field(v).finish(),
            CalcValue::AnnotatedScalar(v, format) => f
                .debug_tuple("AnnotatedScalar")
                .field(v)
                .field(format)
                .finish(),
            CalcValue::Range(rv) => {
                let (r, c) = rv.dims();
                f.debug_tuple("Range").field(&(r, c)).finish()
            }
            CalcValue::Callable(_) => f.write_str("Callable(<opaque>)"),
        }
    }
}

impl<'a> CalcValue<'a> {
    pub fn into_literal(self) -> LiteralValue {
        match self {
            CalcValue::Scalar(s) | CalcValue::AnnotatedScalar(s, _) => s,
            CalcValue::Range(rv) => {
                let (rows, cols) = rv.dims();
                if rows == 1 && cols == 1 {
                    rv.get_cell(0, 0)
                } else {
                    let mut data = Vec::with_capacity(rows);
                    for row_idx in 0..rows {
                        let mut row = Vec::with_capacity(cols);
                        for col_idx in 0..cols {
                            row.push(rv.get_cell(row_idx, col_idx));
                        }
                        data.push(row);
                    }
                    LiteralValue::Array(data)
                }
            }
            CalcValue::Callable(_) => LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Calc).with_message("LAMBDA value must be invoked"),
            ),
        }
    }

    pub fn as_scalar(&self) -> Option<&LiteralValue> {
        match self {
            CalcValue::Scalar(s) | CalcValue::AnnotatedScalar(s, _) => Some(s),
            _ => None,
        }
    }

    pub fn format_id(&self) -> Option<crate::format::FormatId> {
        match self {
            CalcValue::AnnotatedScalar(_, format) => Some(*format),
            _ => None,
        }
    }

    pub fn with_format(self, format: Option<crate::format::FormatId>) -> Self {
        let format = format.filter(|id| *id != crate::format::FormatId::GENERAL);
        match (self, format) {
            (CalcValue::Scalar(value) | CalcValue::AnnotatedScalar(value, _), Some(id)) => {
                CalcValue::AnnotatedScalar(value, id)
            }
            (CalcValue::AnnotatedScalar(value, _), None) => CalcValue::Scalar(value),
            (other, _) => other,
        }
    }

    pub fn into_scalar_parts(self) -> (LiteralValue, Option<crate::format::FormatId>) {
        match self {
            CalcValue::Scalar(value) => (value, None),
            CalcValue::AnnotatedScalar(value, format) => (value, Some(format)),
            other => (other.into_literal(), None),
        }
    }

    pub fn as_range(&self) -> Option<&RangeView<'a>> {
        match self {
            CalcValue::Range(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_callable(&self) -> Option<&Arc<dyn CustomCallable>> {
        match self {
            CalcValue::Callable(c) => Some(c),
            _ => None,
        }
    }

    pub fn into_owned(self) -> LiteralValue {
        self.into_literal()
    }
}

impl From<CalcValue<'_>> for LiteralValue {
    fn from(val: CalcValue<'_>) -> Self {
        val.into_literal()
    }
}

impl<'a> PartialEq<LiteralValue> for CalcValue<'a> {
    fn eq(&self, other: &LiteralValue) -> bool {
        match self {
            CalcValue::Scalar(s) | CalcValue::AnnotatedScalar(s, _) => s == other,
            CalcValue::Range(rv) => match other {
                LiteralValue::Array(arr) => {
                    let (rows, cols) = rv.dims();
                    if arr.len() != rows {
                        return false;
                    }
                    for (r, row) in arr.iter().enumerate() {
                        if row.len() != cols {
                            return false;
                        }
                        for (c, cell) in row.iter().enumerate() {
                            if &rv.get_cell(r, c) != cell {
                                return false;
                            }
                        }
                    }
                    true
                }
                _ => {
                    let (rows, cols) = rv.dims();
                    rows == 1 && cols == 1 && &rv.get_cell(0, 0) == other
                }
            },
            CalcValue::Callable(_) => false,
        }
    }
}

impl<'a> PartialEq<CalcValue<'a>> for LiteralValue {
    fn eq(&self, other: &CalcValue<'a>) -> bool {
        other == self
    }
}

pub enum EvaluatedArg<'a> {
    LiteralValue(CowValue<'a>),
    Range(Box<dyn Range>),
}

#[derive(Clone, Copy)]
enum ArgumentExpr<'a> {
    Ast(&'a ASTNode),
    Arena {
        id: crate::engine::arena::AstNodeId,
        data_store: &'a crate::engine::arena::DataStore,
        sheet_registry: &'a crate::engine::sheet_registry::SheetRegistry,
    },
}

pub struct ArgumentHandle<'a, 'b> {
    expr: ArgumentExpr<'a>,
    interp: &'a Interpreter<'b>,
    cached_ast: std::cell::OnceCell<ASTNode>,
    cached_ref: std::cell::OnceCell<ReferenceType>,
    cached_reference_or_value:
        std::cell::OnceCell<Result<crate::function::FunctionResolution<'b>, ExcelError>>,
    cached_resolved: std::cell::OnceCell<Result<ResolvedArgument<'b>, ExcelError>>,
    /// Memoized result of [`Self::value`]. `Function::dispatch` evaluates
    /// every argument once during schema validation and the function's `eval`
    /// evaluates it again — without this cache that re-entry compounds to
    /// 2^depth evaluations of the innermost node for nested non-short-circuit
    /// calls (measured: depth 12 ⇒ 4096 evaluations). The handle is created
    /// per call site and per evaluation, so the memo can never go stale
    /// across recalcs. `value_with_env` is intentionally NOT memoized (the
    /// local env changes the result).
    cached_value: std::cell::OnceCell<Result<crate::traits::CalcValue<'b>, ExcelError>>,
}

/// A clone shares whatever this handle has already evaluated or resolved.
impl<'a, 'b> Clone for ArgumentHandle<'a, 'b> {
    fn clone(&self) -> Self {
        Self {
            expr: self.expr,
            interp: self.interp,
            cached_ast: self.cached_ast.clone(),
            cached_ref: self.cached_ref.clone(),
            cached_reference_or_value: self.cached_reference_or_value.clone(),
            cached_resolved: self.cached_resolved.clone(),
            cached_value: self.cached_value.clone(),
        }
    }
}

/// The reference argument an intersection forms from the areas it holds
/// (`Interpreter::evaluate_ast_as_areas`): `Some(Some(Ok(area)))` for a single
/// area, `Some(Some(Err(_)))` for its error and `Some(None)` for several areas,
/// which are not one reference. `None` when no side can hold several areas, so
/// the intersection resolves as a plain reference.
fn single_area(
    areas: Option<Result<Vec<ReferenceType>, ExcelError>>,
) -> Option<Option<Result<ReferenceType, ExcelError>>> {
    match areas? {
        Ok(mut areas) if areas.len() == 1 => Some(Some(Ok(areas.remove(0)))),
        Ok(_) => Some(None),
        Err(error) => Some(Some(Err(error))),
    }
}

/// The argument IF or CHOOSE selects with a single condition or index.
fn selected_branch(name: &str, args: &[ArgumentHandle<'_, '_>]) -> Option<usize> {
    let selector = args.first()?.value().ok()?.into_literal();
    if name.eq_ignore_ascii_case("IF") {
        if !(2..=3).contains(&args.len()) {
            return None;
        }
        if crate::builtins::logical::if_condition(selector).ok()? {
            Some(1)
        } else {
            (args.len() == 3).then_some(2)
        }
    } else {
        let index = match selector {
            LiteralValue::Number(n) => n as i64,
            LiteralValue::Int(i) => i,
            _ => return None,
        };
        (index >= 1 && (index as usize) < args.len()).then_some(index as usize)
    }
}

impl<'a, 'b> ArgumentHandle<'a, 'b> {
    /// A handle for a literal element, evaluated by the same interpreter.
    /// Used when a scalar parameter is lifted over an array argument.
    pub(crate) fn literal<'n>(&self, node: &'n ASTNode) -> ArgumentHandle<'n, 'b>
    where
        'a: 'n,
    {
        ArgumentHandle::new(node, self.interp)
    }

    pub(crate) fn new(node: &'a ASTNode, interp: &'a Interpreter<'b>) -> Self {
        Self {
            expr: ArgumentExpr::Ast(node),
            interp,
            cached_ast: std::cell::OnceCell::new(),
            cached_ref: std::cell::OnceCell::new(),
            cached_reference_or_value: std::cell::OnceCell::new(),
            cached_resolved: std::cell::OnceCell::new(),
            cached_value: std::cell::OnceCell::new(),
        }
    }

    pub(crate) fn new_arena(
        id: crate::engine::arena::AstNodeId,
        interp: &'a Interpreter<'b>,
        data_store: &'a crate::engine::arena::DataStore,
        sheet_registry: &'a crate::engine::sheet_registry::SheetRegistry,
    ) -> Self {
        Self {
            expr: ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            },
            interp,
            cached_ast: std::cell::OnceCell::new(),
            cached_ref: std::cell::OnceCell::new(),
            cached_reference_or_value: std::cell::OnceCell::new(),
            cached_resolved: std::cell::OnceCell::new(),
            cached_value: std::cell::OnceCell::new(),
        }
    }

    /// Workbook date system in force for the evaluation this argument belongs to.
    ///
    /// Lets value-collecting helpers resolve date literals to serials without
    /// threading a `DateSystem` (or the whole `FunctionContext`) through every
    /// call site.
    pub(crate) fn date_system(&self) -> crate::engine::DateSystem {
        self.interp.context.date_system()
    }

    /// The evaluation clock's year, which Excel uses for date text that omits
    /// the year (`Jan 3`).
    pub(crate) fn current_year(&self) -> i32 {
        use chrono::Datelike;
        self.interp.context.clock().today().year()
    }

    /// Returns whether this handle represents an explicitly omitted argument slot.
    ///
    /// This is false for absent arguments, explicit empty text, and blank references.
    pub fn is_omitted(&self) -> bool {
        match &self.expr {
            ArgumentExpr::Ast(node) => matches!(node.node_type, ASTNodeType::Omitted),
            ArgumentExpr::Arena { id, data_store, .. } => matches!(
                data_store.get_node(*id),
                Some(crate::engine::arena::AstNodeData::Omitted)
            ),
        }
    }

    /// Returns whether this argument resolves as a spreadsheet reference rather than a value.
    ///
    /// This uses the interpreter's reference-resolution path, so reference-returning functions
    /// are included only when they actually produce a reference. A computed array remains a value
    /// even though both it and a cell range are represented by [`CalcValue::Range`].
    pub(crate) fn has_reference_semantics(&self) -> bool {
        // The range and intersection operators always yield a reference (or
        // its error), which needs no second evaluation of their sides to
        // tell: a selector such as `INDEX(A1:A4,n) A1:A4` or `INDEX(A1:A4,n):A4`
        // runs only for the argument's value.
        self.is_reference_operator() || self.reference_attempt().is_some()
    }

    /// Whether this argument is written with the `:` range or the space
    /// intersection operator.
    fn is_reference_operator(&self) -> bool {
        match &self.expr {
            ArgumentExpr::Ast(node) => {
                matches!(&node.node_type, ASTNodeType::BinaryOp { op, .. } if op == ":" || op == " ")
            }
            ArgumentExpr::Arena { id, data_store, .. } => matches!(
                data_store.get_node(*id),
                Some(crate::engine::arena::AstNodeData::BinaryOp { op_id, .. })
                    if matches!(data_store.resolve_ast_string(*op_id), ":" | " ")
            ),
        }
    }

    /// The reference into a closed linked workbook (`[1]Data!A1:B9`) this
    /// argument is written as, placed for the formula cell; `None` for any
    /// other argument. It reads the values saved with the link.
    pub(crate) fn linked_book_reference(&self) -> Option<ReferenceType> {
        let reference = match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference {
                    reference: reference @ ReferenceType::External(_),
                    ..
                } => reference.clone(),
                _ => return None,
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => match data_store.get_node(*id) {
                Some(crate::engine::arena::AstNodeData::Reference {
                    ref_type: ref_type @ crate::engine::arena::CompactRefType::External { .. },
                    ..
                }) => data_store.reconstruct_reference_type_for_eval(ref_type, sheet_registry),
                _ => return None,
            },
        };
        match &reference {
            ReferenceType::External(ext)
                if crate::engine::external_book::is_link_index(ext.book.token()) =>
            {
                self.interp.reference_for_current_offset(&reference).ok()
            }
            _ => None,
        }
    }

    /// Whether this argument is written as a reference into a linked workbook,
    /// which evaluates from the values saved with the link while it is closed.
    pub fn is_external_reference(&self) -> bool {
        match &self.expr {
            ArgumentExpr::Ast(node) => matches!(
                &node.node_type,
                ASTNodeType::Reference {
                    reference: ReferenceType::External(_),
                    ..
                }
            ),
            ArgumentExpr::Arena { id, data_store, .. } => matches!(
                data_store.get_node(*id),
                Some(crate::engine::arena::AstNodeData::Reference {
                    ref_type: crate::engine::arena::CompactRefType::External { .. },
                    ..
                })
            ),
        }
    }

    /// Return whether this argument's syntax may produce a spreadsheet reference.
    ///
    /// Unlike [`Self::has_reference_semantics`], this does not evaluate a
    /// reference-returning function to discover which arm it selects.
    pub(crate) fn may_return_reference(&self) -> bool {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { reference, .. } => {
                    !matches!(reference, ReferenceType::NamedRange(name) if self.interp.is_local_value_name(name))
                }
                ASTNodeType::BinaryOp { op, .. } => op == ":" || op == " ",
                ASTNodeType::Function { name, .. } => self
                    .interp
                    .context
                    .function_capabilities("", name)
                    .is_some_and(|caps| caps.contains(crate::function::FnCaps::RETURNS_REFERENCE)),
                _ => false,
            },
            ArgumentExpr::Arena { id, data_store, .. } => match data_store.get_node(*id) {
                Some(crate::engine::arena::AstNodeData::Reference { ref_type, .. }) => !matches!(
                    ref_type,
                    crate::engine::arena::CompactRefType::NamedRange(name_id)
                        if self
                            .interp
                            .is_local_value_name(data_store.resolve_ast_string(*name_id))
                ),
                Some(crate::engine::arena::AstNodeData::BinaryOp { op_id, .. }) => {
                    matches!(data_store.resolve_ast_string(*op_id), ":" | " ")
                }
                Some(crate::engine::arena::AstNodeData::Function { name_id, .. }) => self
                    .interp
                    .context
                    .function_capabilities("", data_store.resolve_ast_string(*name_id))
                    .is_some_and(|caps| caps.contains(crate::function::FnCaps::RETURNS_REFERENCE)),
                _ => false,
            },
        }
    }

    /// Whether this argument evaluates to an array or to a single value, as
    /// Excel types it (see [`ResultShape`]). The engine holds a one-element
    /// array as its single value, so a function that treats the two
    /// differently (MATCH searches an array but not a single value) reads the
    /// shape here.
    pub(crate) fn result_shape(&self) -> ResultShape {
        result_shape::of(self)
    }

    /// [`Self::result_shape`] without resolving defined names (a name not
    /// bound by a LET or LAMBDA is of unknown shape), so that reading it
    /// calculates nothing.
    pub(crate) fn result_shape_without_names(&self) -> ResultShape {
        result_shape::of_without_names(self)
    }

    pub fn value(&self) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        self.cached_value
            .get_or_init(|| self.compute_value())
            .clone()
    }

    /// Resolves a scalar that is about to be coerced to text.
    ///
    /// Omitted arguments materialize as numeric zero through `value()`, which is
    /// correct for Any/numeric consumers and aggregates. Text consumers must use
    /// this boundary so omission becomes empty text without changing explicit 0.
    ///
    /// An IF written here reads through to the argument it selects, so an
    /// empty slot that IF selects is empty text as well: Excel gives "ok" for
    /// `IF(FALSE,"not ",)&"ok"`. Every other consumer, aggregates and lookups
    /// included, still sees the 0 that Microsoft documents for IF.
    pub(crate) fn value_for_text(&self) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        if self.is_omitted() {
            return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Text(
                String::new(),
            )));
        }
        if let Some(value) = self.if_branch_value_for_text() {
            return value;
        }
        self.value()
    }

    /// The text value of an IF written as this argument that may select an
    /// empty slot (see [`Self::value_for_text`]); `None` for any other
    /// argument, which reads as its plain value. The `&` operator uses this so
    /// its other operands evaluate exactly as before.
    pub(crate) fn if_branch_value_for_text(
        &self,
    ) -> Option<Result<crate::traits::CalcValue<'b>, ExcelError>> {
        self.with_if_branch_for_text(|branch| {
            // IF's value is the selected argument's; an evaluation error there
            // is IF's error value, as the interpreter makes it for any call.
            match branch.value_for_text() {
                Err(error) if error.kind != ExcelErrorKind::Cancelled => {
                    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)))
                }
                other => other,
            }
        })
    }

    /// [`Self::resolve_once`] for a text consumer, with the empty slots that
    /// [`Self::value_for_text`] reads as empty text.
    pub(crate) fn resolve_once_for_text(&self) -> Result<ResolvedArgument<'b>, ExcelError> {
        if self.is_omitted() {
            return Ok(ResolvedArgument::Value(crate::traits::CalcValue::Scalar(
                LiteralValue::Text(String::new()),
            )));
        }
        if let Some(resolved) =
            self.with_if_branch_for_text(|branch| branch.resolve_once_for_text())
        {
            return resolved;
        }
        self.resolve_once()
    }

    /// For an IF written as this argument that may select an empty slot, `f`
    /// applied to the argument its single-value condition selects; a text
    /// consumer reads that argument in place of IF's value. The argument
    /// handles are the ones IF itself is evaluated with, so in a formula
    /// without the array flag a range condition is intersected as it is for
    /// IF. The condition is evaluated here only when an empty slot is among
    /// IF's value arguments (directly or in an IF written there), so any
    /// other IF evaluates once, as a call.
    fn with_if_branch_for_text<R>(
        &self,
        f: impl FnOnce(&ArgumentHandle<'_, 'b>) -> R,
    ) -> Option<R> {
        if !self.is_if_with_empty_slot() {
            return None;
        }
        let fun = self
            .interp
            .context
            .get_function("", self.function_name()?)?;
        self.with_call_handles(fun.as_ref(), |handles| {
            let index = crate::builtins::logical::if_selected_argument(handles)?;
            Some(f(&handles[index]))
        })
        .flatten()
    }

    /// Whether this argument is a call to IF with an empty slot among its
    /// value arguments, directly or in an IF written there. Syntax only:
    /// nothing is evaluated.
    fn is_if_with_empty_slot(&self) -> bool {
        let may_be_empty =
            |arg: ArgumentHandle<'_, 'b>| arg.is_omitted() || arg.is_if_with_empty_slot();
        match self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Function { name, args } if name.eq_ignore_ascii_case("IF") => args
                    .iter()
                    .skip(1)
                    .any(|arg| may_be_empty(ArgumentHandle::new(arg, self.interp))),
                _ => false,
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => match data_store.get_node(id) {
                Some(crate::engine::arena::AstNodeData::Function { name_id, .. })
                    if data_store
                        .resolve_ast_string(*name_id)
                        .eq_ignore_ascii_case("IF") =>
                {
                    data_store.get_args(id).is_some_and(|args| {
                        args.iter().skip(1).any(|&arg| {
                            may_be_empty(ArgumentHandle::new_arena(
                                arg,
                                self.interp,
                                data_store,
                                sheet_registry,
                            ))
                        })
                    })
                }
                _ => false,
            },
        }
    }

    fn compute_value(&self) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Literal(v) => Ok(crate::traits::CalcValue::Scalar(v.clone())),
                // With no schema-level text policy, Number(0) is the neutral Any-policy
                // materialization. Text consumers resolve through `value_for_text`.
                ASTNodeType::Omitted => {
                    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0)))
                }
                _ => self.interp.evaluate_ast(node),
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                if matches!(
                    data_store.get_node(*id),
                    Some(crate::engine::arena::AstNodeData::Omitted)
                ) {
                    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0)))
                } else {
                    self.interp
                        .evaluate_arena_ast(*id, data_store, sheet_registry)
                }
            }
        }
    }

    pub fn value_with_env(
        &self,
        env: crate::interpreter::LocalEnv,
    ) -> Result<crate::traits::CalcValue<'b>, ExcelError> {
        let scoped = self.interp.with_local_env(env);
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Literal(v) => Ok(crate::traits::CalcValue::Scalar(v.clone())),
                ASTNodeType::Omitted => {
                    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0)))
                }
                _ => scoped.evaluate_ast(node),
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                if matches!(
                    data_store.get_node(*id),
                    Some(crate::engine::arena::AstNodeData::Omitted)
                ) {
                    Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0)))
                } else {
                    scoped.evaluate_arena_ast(*id, data_store, sheet_registry)
                }
            }
        }
    }

    pub fn current_env(&self) -> crate::interpreter::LocalEnv {
        self.interp.local_env().clone()
    }

    /// The interpreter evaluating this argument; helpers such as MAP invoke
    /// a LAMBDA argument through it.
    pub(crate) fn interpreter(&self) -> &'a Interpreter<'b> {
        self.interp
    }

    /// This handle with its value already evaluated.
    pub(crate) fn with_value(
        self,
        value: Result<crate::traits::CalcValue<'b>, ExcelError>,
    ) -> Self {
        let _ = self.cached_value.set(value);
        self
    }

    /// Whether a formula entered without the array flag evaluates this
    /// argument as a single value rather than as an array.
    pub(crate) fn in_legacy_value_context(&self) -> bool {
        self.interp.in_legacy_value_context()
    }

    pub fn inline_array_literal(&self) -> Result<Option<Vec<Vec<LiteralValue>>>, ExcelError> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Literal(LiteralValue::Array(arr)) => Ok(Some(arr.clone())),
                _ => Ok(None),
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let node = data_store.get_node(*id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                })?;
                match node {
                    crate::engine::arena::AstNodeData::Literal(vref) => {
                        match data_store.retrieve_value(*vref) {
                            LiteralValue::Array(arr) => Ok(Some(arr)),
                            _ => Ok(None),
                        }
                    }
                    _ => {
                        // preserve existing behavior: only a literal array (not a computed array)
                        // is treated as "inline array literal".
                        let _ = sheet_registry;
                        Ok(None)
                    }
                }
            }
        }
    }

    fn reference_for_eval(&self) -> Result<ReferenceType, ExcelError> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { reference, .. } => {
                    self.interp.reference_for_current_offset(reference)
                }
                ASTNodeType::Function { .. } | ASTNodeType::BinaryOp { .. } => {
                    self.interp.evaluate_ast_as_reference(node)
                }
                _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                    .with_message("Expected a reference (by-ref argument)")),
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let node = data_store.get_node(*id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                })?;
                match node {
                    crate::engine::arena::AstNodeData::Reference { ref_type, .. } => {
                        let reference = data_store
                            .reconstruct_reference_type_for_eval(ref_type, sheet_registry);
                        self.interp.reference_for_current_offset(&reference)
                    }
                    crate::engine::arena::AstNodeData::Function { .. }
                    | crate::engine::arena::AstNodeData::BinaryOp { .. } => self
                        .interp
                        .evaluate_arena_ast_as_reference(*id, data_store, sheet_registry),
                    _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                        .with_message("Expected a reference (by-ref argument)")),
                }
            }
        }
    }

    fn function_resolution_attempt(
        &self,
    ) -> Option<Result<crate::function::FunctionResolution<'b>, ExcelError>> {
        match &self.expr {
            ArgumentExpr::Ast(node) => {
                let ASTNodeType::Function { name, args } = &node.node_type else {
                    return None;
                };
                if !self
                    .interp
                    .context
                    .function_capabilities("", name)
                    .is_some_and(|caps| caps.contains(crate::function::FnCaps::RETURNS_REFERENCE))
                {
                    return None;
                }
                let fun = match self.interp.context.get_function("", name) {
                    Some(fun) => fun,
                    None => {
                        return Some(Err(ExcelError::new(ExcelErrorKind::Name)
                            .with_message(format!("Unknown function: {name}"))));
                    }
                };
                let handles: Vec<_> = args
                    .iter()
                    .map(|arg| ArgumentHandle::new(arg, self.interp))
                    .collect();
                let ctx = DefaultFunctionContext::new_with_sheet(
                    self.interp.context,
                    self.interp.current_cell(),
                    self.interp.current_sheet(),
                );
                let _call_dates = self.interp.enter_function_call();
                Some(fun.resolve_reference_or_value(&handles, &ctx, &|| self.value()))
            }
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let node = match data_store.get_node(*id) {
                    Some(node) => node,
                    None => {
                        return Some(Err(
                            ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                        ));
                    }
                };
                let crate::engine::arena::AstNodeData::Function { name_id, .. } = node else {
                    return None;
                };
                let name = data_store.resolve_ast_string(*name_id);
                if !self
                    .interp
                    .context
                    .function_capabilities("", name)
                    .is_some_and(|caps| caps.contains(crate::function::FnCaps::RETURNS_REFERENCE))
                {
                    return None;
                }
                let fun = match self.interp.context.get_function("", name) {
                    Some(fun) => fun,
                    None => {
                        return Some(Err(ExcelError::new(ExcelErrorKind::Name)
                            .with_message(format!("Unknown function: {name}"))));
                    }
                };
                let args = match data_store.get_args(*id) {
                    Some(args) => args,
                    None => {
                        return Some(Err(ExcelError::new(ExcelErrorKind::Value)
                            .with_message("Missing function args")));
                    }
                };
                let ctx = DefaultFunctionContext::new_with_sheet(
                    self.interp.context,
                    self.interp.current_cell(),
                    self.interp.current_sheet(),
                );
                let _call_dates = self.interp.enter_function_call();
                Some(self.interp.with_arena_call_handles(
                    fun.as_ref(),
                    args,
                    data_store,
                    sheet_registry,
                    |handles| fun.resolve_reference_or_value(handles, &ctx, &|| self.value()),
                ))
            }
        }
    }

    fn reference_attempt(&self) -> Option<Result<ReferenceType, ExcelError>> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { reference, .. } => {
                    // A closed linked workbook yields values, not a reference.
                    if matches!(reference, ReferenceType::External(_)) {
                        return None;
                    }
                    self.written_reference(reference)
                }
                ASTNodeType::BinaryOp { op, .. } if op == ":" => {
                    Some(self.interp.evaluate_ast_as_reference(node))
                }
                // The intersection operator yields a reference as well: the
                // cells both sides hold, so `NPV(r,A2 A2,...)` and
                // `SUM(A2 A2)` skip the text or logical A2 holds as they do
                // for A2. An intersection that holds several areas (through a
                // union or a multi-area name) stays a value.
                ASTNodeType::BinaryOp { op, .. } if op == " " => {
                    single_area(self.interp.evaluate_ast_as_areas(node))
                        .unwrap_or_else(|| Some(self.interp.evaluate_ast_as_reference(node)))
                }
                ASTNodeType::Function { name, .. }
                    if self
                        .interp
                        .context
                        .function_capabilities("", name)
                        .is_some_and(|caps| {
                            caps.contains(crate::function::FnCaps::RETURNS_REFERENCE)
                        }) =>
                {
                    self.interp.try_evaluate_ast_as_reference(node)
                }
                _ => None,
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let node = match data_store.get_node(*id) {
                    Some(node) => node,
                    None => {
                        return Some(Err(
                            ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                        ));
                    }
                };
                match node {
                    crate::engine::arena::AstNodeData::Reference { ref_type, .. } => {
                        // Same rules as the AST branch above.
                        if matches!(
                            ref_type,
                            crate::engine::arena::CompactRefType::External { .. }
                        ) {
                            return None;
                        }
                        let reference = data_store
                            .reconstruct_reference_type_for_eval(ref_type, sheet_registry);
                        self.written_reference(&reference)
                    }
                    crate::engine::arena::AstNodeData::BinaryOp { op_id, .. }
                        if data_store.resolve_ast_string(*op_id) == ":" =>
                    {
                        Some(self.interp.evaluate_arena_ast_as_reference(
                            *id,
                            data_store,
                            sheet_registry,
                        ))
                    }
                    // Same rules as the AST branch above.
                    crate::engine::arena::AstNodeData::BinaryOp { op_id, .. }
                        if data_store.resolve_ast_string(*op_id) == " " =>
                    {
                        single_area(self.interp.evaluate_arena_ast_as_areas(
                            *id,
                            data_store,
                            sheet_registry,
                        ))
                        .unwrap_or_else(|| {
                            Some(self.interp.evaluate_arena_ast_as_reference(
                                *id,
                                data_store,
                                sheet_registry,
                            ))
                        })
                    }
                    crate::engine::arena::AstNodeData::Function { name_id, .. } => {
                        let name = data_store.resolve_ast_string(*name_id);
                        if self
                            .interp
                            .context
                            .function_capabilities("", name)
                            .is_some_and(|caps| {
                                caps.contains(crate::function::FnCaps::RETURNS_REFERENCE)
                            })
                        {
                            self.interp.try_evaluate_arena_ast_as_reference(
                                *id,
                                data_store,
                                sheet_registry,
                            )
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
        }
    }

    /// The references this argument evaluates to when it is an array of
    /// references: a reference-returning call lifted over an array
    /// (`OFFSET(A1,{0;1},0)`, also as OFFSET's own reference), the branch IF or
    /// CHOOSE selects, a LET binding or a defined name holding one. Such an
    /// argument has no value of its own (it reads as `#VALUE!`). `None` for
    /// any other argument.
    pub(crate) fn reference_array(
        &self,
    ) -> Result<Option<crate::lift::ReferenceArray>, ExcelError> {
        if let Some(name) = self.name_reference() {
            return self.name_reference_array(name);
        }
        let Some(name) = self.function_name() else {
            return Ok(None);
        };
        let spec = crate::lift::reference_array_spec(name);
        let branches = name.eq_ignore_ascii_case("IF") || name.eq_ignore_ascii_case("CHOOSE");
        // Only a call that fails to resolve to a single reference can be one.
        if (spec.is_none() && !branches) || !self.reads_as_error()? {
            return Ok(None);
        }
        let Some(fun) = self.interp.context.get_function("", name) else {
            return Ok(None);
        };
        let ctx = DefaultFunctionContext::new_with_sheet(
            self.interp.context,
            self.interp.current_cell(),
            self.interp.current_sheet(),
        );
        let _call_dates = self.interp.enter_function_call();
        self.with_call_handles(fun.as_ref(), |handles| match spec {
            Some(spec) => {
                crate::lift::lift_reference(spec, handles, |call| fun.eval_reference(call, &ctx))
            }
            None => match selected_branch(name, handles) {
                Some(index) => handles[index].reference_array(),
                None => Ok(None),
            },
        })
        .unwrap_or(Ok(None))
    }

    /// Runs `f` on this argument as evaluated with a LET scope's bindings.
    pub(crate) fn with_env<R>(
        &self,
        env: crate::interpreter::LocalEnv,
        f: impl FnOnce(&ArgumentHandle<'_, 'b>) -> R,
    ) -> R {
        let scoped = self.interp.with_local_env(env);
        let handle = match self.expr {
            ArgumentExpr::Ast(node) => ArgumentHandle::new(node, &scoped),
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => ArgumentHandle::new_arena(id, &scoped, data_store, sheet_registry),
        };
        f(&handle)
    }

    /// The cell or range reference this argument evaluates to, which a LET
    /// name or LAMBDA parameter bound to it keeps: a reference written here, a
    /// `:` range, a name bound to a reference, or the reference a function such
    /// as INDEX, OFFSET or XLOOKUP returns. `None` for values, for references
    /// that fail, and for other references (tables, external, 3D), which are
    /// bound by value.
    pub(crate) fn bindable_reference(&self) -> Result<Option<ReferenceType>, ExcelError> {
        match self.resolve_reference_or_value() {
            Ok(crate::function::FunctionResolution::Reference(
                reference @ (ReferenceType::Cell { .. } | ReferenceType::Range { .. }),
            )) => Ok(Some(reference)),
            Err(error) if error.kind == ExcelErrorKind::Cancelled => Err(error),
            _ => Ok(None),
        }
    }

    /// `reference`, written as this argument, resolved for the formula cell.
    /// A LET/LAMBDA local shadows any workbook name of the same spelling: a
    /// local bound to a reference is that reference. A name that holds a value
    /// is not a reference (`None`): a local bound to a value, or a defined name
    /// whose formula does not evaluate to a reference (`={1,2,3}`,
    /// `=Sheet1!$B$1:$B$3*2`, `=Konst` for such a name, `=IF(TRUE,42)`). Such a
    /// name resolves only on the value path.
    fn written_reference(
        &self,
        reference: &ReferenceType,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        if let ReferenceType::NamedRange(name) = reference {
            if let Some(binding) = self.interp.local_binding(name) {
                return match binding {
                    crate::interpreter::LocalBinding::Reference(bound) => Some(Ok(bound.clone())),
                    _ => None,
                };
            }
            let sheet = self.interp.current_sheet();
            if let Some(resolved) = self.interp.context.resolve_name_reference(name, sheet) {
                return Some(resolved);
            }
            if self.interp.context.is_value_name(name, sheet) {
                return None;
            }
        }
        Some(self.interp.reference_for_current_offset(reference))
    }

    /// The array of references a LET binding or a defined name holds.
    fn name_reference_array(
        &self,
        name: &str,
    ) -> Result<Option<crate::lift::ReferenceArray>, ExcelError> {
        if let Some(binding) = self.interp.resolve_local_name(name) {
            return Ok(match binding {
                crate::interpreter::LocalBinding::References(references) => {
                    Some(references.as_ref().clone())
                }
                _ => None,
            });
        }
        if !self.reads_as_error()? {
            return Ok(None);
        }
        match self
            .interp
            .context
            .resolve_name_reference_array(name, self.interp.current_sheet())
        {
            Some(Ok(references)) => Ok(Some(references)),
            Some(Err(error)) if error.kind == ExcelErrorKind::Cancelled => Err(error),
            _ => Ok(None),
        }
    }

    /// Whether this argument resolves to neither a reference nor a value
    /// other than an error, as an array of references does.
    fn reads_as_error(&self) -> Result<bool, ExcelError> {
        use crate::function::FunctionResolution;
        match self.resolve_reference_or_value() {
            Ok(FunctionResolution::ReferenceError(_))
            | Ok(FunctionResolution::Value(CalcValue::Scalar(LiteralValue::Error(_)))) => Ok(true),
            Err(error) if error.kind == ExcelErrorKind::Cancelled => Err(error),
            _ => Ok(false),
        }
    }

    /// The name of a defined name or LET binding written as this argument.
    fn name_reference(&self) -> Option<&'a str> {
        match self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference {
                    reference: ReferenceType::NamedRange(name),
                    ..
                } => Some(name.as_str()),
                _ => None,
            },
            ArgumentExpr::Arena { id, data_store, .. } => match data_store.get_node(id) {
                Some(crate::engine::arena::AstNodeData::Reference {
                    ref_type: crate::engine::arena::CompactRefType::NamedRange(name_id),
                    ..
                }) => Some(data_store.resolve_ast_string(*name_id)),
                _ => None,
            },
        }
    }

    /// Whether this argument is a LET name or LAMBDA parameter bound to a
    /// value rather than to a reference: it reads as that value.
    pub(crate) fn is_local_value_name(&self) -> bool {
        self.name_reference()
            .is_some_and(|name| self.interp.is_local_value_name(name))
    }

    /// The function name of a call written as this argument.
    fn function_name(&self) -> Option<&'a str> {
        match self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Function { name, .. } => Some(name.as_str()),
                _ => None,
            },
            ArgumentExpr::Arena { id, data_store, .. } => match data_store.get_node(id) {
                Some(crate::engine::arena::AstNodeData::Function { name_id, .. }) => {
                    Some(data_store.resolve_ast_string(*name_id))
                }
                _ => None,
            },
        }
    }

    /// Run `f` on the argument handles of the call to the builtin `fun`
    /// written as this argument. They are the handles the call itself is
    /// evaluated with: in a formula entered without the array flag each
    /// argument gets the context of its parameter class, so a range in a
    /// single-value position (OFFSET's rows, IF's test) is intersected with
    /// the formula cell there and kept whole inside an array argument.
    fn with_call_handles<R>(
        &self,
        fun: &dyn crate::function::Function,
        f: impl FnOnce(&[ArgumentHandle<'_, 'b>]) -> R,
    ) -> Option<R> {
        match self.expr {
            ArgumentExpr::Ast(node) => {
                let ASTNodeType::Function { args, .. } = &node.node_type else {
                    return None;
                };
                let handles: Vec<_> = args
                    .iter()
                    .map(|arg| ArgumentHandle::new(arg, self.interp))
                    .collect();
                Some(f(&handles))
            }
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let args = data_store.get_args(id)?;
                Some(
                    self.interp
                        .with_arena_call_handles(fun, args, data_store, sheet_registry, f),
                )
            }
        }
    }

    pub(crate) fn resolve_reference_or_value(
        &self,
    ) -> Result<crate::function::FunctionResolution<'b>, ExcelError> {
        let resolved = self
            .cached_reference_or_value
            .get_or_init(|| self.compute_reference_or_value())
            .clone();
        if let Ok(crate::function::FunctionResolution::Value(value)) = &resolved {
            let _ = self.cached_value.set(Ok(value.clone()));
        }
        resolved
    }

    /// Whether this argument, once resolved ([`Self::resolve_once`] or
    /// [`Self::resolve_reference_or_value`]), is a reference: a cell, a range,
    /// a name for one, a function returning one or an expression whose value
    /// reads a sheet's cells (an intersection, a name bound by LET), rather
    /// than a value such as a number, a computed result or an array. False
    /// before resolution.
    pub(crate) fn resolved_as_reference(&self) -> bool {
        match self.cached_reference_or_value.get() {
            Some(Ok(crate::function::FunctionResolution::Reference(_))) => true,
            Some(Ok(crate::function::FunctionResolution::Value(CalcValue::Range(view)))) => {
                view.is_sheet_backed()
            }
            _ => false,
        }
    }

    fn compute_reference_or_value(
        &self,
    ) -> Result<crate::function::FunctionResolution<'b>, ExcelError> {
        if let Some(result) = self.function_resolution_attempt() {
            return result;
        }
        if let Some(reference) = self.reference_attempt() {
            return Ok(match reference {
                Ok(reference) => crate::function::FunctionResolution::Reference(reference),
                Err(error) => crate::function::FunctionResolution::ReferenceError(error),
            });
        }
        self.value().map(crate::function::FunctionResolution::Value)
    }

    /// Resolve this argument once without using a failed range conversion as type dispatch.
    ///
    /// Direct references and the `:` operator take the reference path. Functions
    /// with `RETURNS_REFERENCE` first attempt reference evaluation, but fall back
    /// to their cached value when `eval_reference` returns `None`.
    pub(crate) fn resolve_once(&self) -> Result<ResolvedArgument<'b>, ExcelError> {
        self.cached_resolved
            .get_or_init(|| self.compute_resolved_argument())
            .clone()
    }

    pub(crate) fn with_context_cancel_token(&self, view: RangeView<'b>) -> RangeView<'b> {
        match self.interp.context.cancellation_token() {
            Some(token) => view.with_cancel_token(Some(token)),
            None => view,
        }
    }

    fn compute_resolved_argument(&self) -> Result<ResolvedArgument<'b>, ExcelError> {
        let value = match self.resolve_reference_or_value()? {
            crate::function::FunctionResolution::Reference(reference) => {
                return match self
                    .interp
                    .context
                    .resolve_range_view(&reference, self.interp.current_sheet())
                {
                    Ok(view) => Ok(ResolvedArgument::Range(
                        self.with_context_cancel_token(view),
                    )),
                    Err(error) if error.kind == ExcelErrorKind::Cancelled => Err(error),
                    Err(error) => Ok(ResolvedArgument::ReferenceError(error)),
                };
            }
            crate::function::FunctionResolution::ReferenceError(error)
                if error.kind == ExcelErrorKind::Cancelled =>
            {
                return Err(error);
            }
            crate::function::FunctionResolution::ReferenceError(error) => {
                return Ok(ResolvedArgument::ReferenceError(error));
            }
            crate::function::FunctionResolution::Value(value) => value,
        };

        match value {
            CalcValue::Range(view) => Ok(ResolvedArgument::Range(
                self.with_context_cancel_token(view),
            )),
            CalcValue::Scalar(LiteralValue::Array(rows)) => {
                let view = RangeView::try_from_owned_rows(
                    rows,
                    self.interp.context.date_system(),
                    self.interp.context.cancellation_token(),
                )?;
                Ok(ResolvedArgument::Range(view))
            }
            other => Ok(ResolvedArgument::Value(other)),
        }
    }

    pub fn range(&self) -> Result<Box<dyn Range>, ExcelError> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { reference, .. } => {
                    // Prefer RangeView since it has explicit current-sheet context.
                    let reference = self.interp.reference_for_current_offset(reference)?;
                    let view = self
                        .interp
                        .context
                        .resolve_range_view(&reference, self.interp.current_sheet())?;
                    let (rows, cols) = view.dims();
                    let mut out: Vec<Vec<LiteralValue>> = Vec::with_capacity(rows);
                    view.for_each_row(&mut |row| {
                        let row_data: Vec<LiteralValue> = (0..cols)
                            .map(|c| row.get(c).cloned().unwrap_or(LiteralValue::Empty))
                            .collect();
                        out.push(row_data);
                        Ok(())
                    })?;
                    Ok(Box::new(InMemoryRange::new(out)))
                }
                ASTNodeType::Function { .. } | ASTNodeType::BinaryOp { .. } => {
                    let reference = self.reference_for_eval()?;
                    let view = self
                        .interp
                        .context
                        .resolve_range_view(&reference, self.interp.current_sheet())?;
                    let (rows, cols) = view.dims();
                    let mut out: Vec<Vec<LiteralValue>> = Vec::with_capacity(rows);
                    view.for_each_row(&mut |row| {
                        let row_data: Vec<LiteralValue> = (0..cols)
                            .map(|c| row.get(c).cloned().unwrap_or(LiteralValue::Empty))
                            .collect();
                        out.push(row_data);
                        Ok(())
                    })?;
                    Ok(Box::new(InMemoryRange::new(out)))
                }
                ASTNodeType::Array(rows) => {
                    let mut materialized = Vec::new();
                    for row in rows {
                        let mut materialized_row = Vec::new();
                        for cell in row {
                            materialized_row.push(self.interp.evaluate_ast(cell)?.into_literal());
                        }
                        materialized.push(materialized_row);
                    }
                    Ok(Box::new(InMemoryRange::new(materialized)))
                }
                _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                    .with_message(format!("Expected a range, got {:?}", node.node_type))),
            },
            ArgumentExpr::Arena { id, data_store, .. } => {
                let node = data_store.get_node(*id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                })?;

                match node {
                    crate::engine::arena::AstNodeData::Reference { .. }
                    | crate::engine::arena::AstNodeData::Function { .. }
                    | crate::engine::arena::AstNodeData::BinaryOp { .. } => {
                        let reference = self.reference_for_eval()?;
                        let view = self
                            .interp
                            .context
                            .resolve_range_view(&reference, self.interp.current_sheet())?;
                        let (rows, cols) = view.dims();
                        let mut out: Vec<Vec<LiteralValue>> = Vec::with_capacity(rows);
                        view.for_each_row(&mut |row| {
                            let row_data: Vec<LiteralValue> = (0..cols)
                                .map(|c| row.get(c).cloned().unwrap_or(LiteralValue::Empty))
                                .collect();
                            out.push(row_data);
                            Ok(())
                        })?;
                        Ok(Box::new(InMemoryRange::new(out)))
                    }
                    crate::engine::arena::AstNodeData::Array { .. } => {
                        let (rows, cols, elements) =
                            data_store.get_array_elems(*id).ok_or_else(|| {
                                ExcelError::new(ExcelErrorKind::Value).with_message("Invalid array")
                            })?;
                        let rows_usize = rows as usize;
                        let cols_usize = cols as usize;
                        let mut materialized: Vec<Vec<LiteralValue>> =
                            Vec::with_capacity(rows_usize);
                        for r in 0..rows_usize {
                            let mut row = Vec::with_capacity(cols_usize);
                            for c in 0..cols_usize {
                                let idx = r * cols_usize + c;
                                let elem_id = elements.get(idx).copied().ok_or_else(|| {
                                    ExcelError::new(ExcelErrorKind::Value)
                                        .with_message("Invalid array")
                                })?;
                                let v = self.interp.evaluate_arena_ast(
                                    elem_id,
                                    data_store,
                                    self.sheet_registry(),
                                )?;
                                row.push(v.into_literal());
                            }
                            materialized.push(row);
                        }
                        Ok(Box::new(InMemoryRange::new(materialized)))
                    }
                    _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                        .with_message("Argument cannot be interpreted as a range.")),
                }
            }
        }
    }

    fn sheet_registry(&self) -> &crate::engine::sheet_registry::SheetRegistry {
        match &self.expr {
            ArgumentExpr::Ast(_) => {
                // Not needed; used only in arena flows.
                unreachable!("sheet_registry only used for arena ArgumentHandle")
            }
            ArgumentExpr::Arena { sheet_registry, .. } => sheet_registry,
        }
    }

    /// Resolve this argument to a [`RangeView`].
    ///
    /// Delegates to [`Self::resolve_once`] so reference-shaped and computed
    /// arguments share one cached resolution path. A reference keeps its lazy
    /// view, while a computed argument (`B1:B3="x"`, `SEQUENCE(3)`, `{1,2}`)
    /// resolves through the same single evaluation the rest of argument
    /// preparation uses instead of being rejected for not being a reference.
    pub fn range_view(&self) -> Result<RangeView<'b>, ExcelError> {
        match self.resolve_once()? {
            ResolvedArgument::Range(view) => Ok(view),
            // A genuine reference failure (`OFFSET(A1,-1,0)`) stays an error
            // rather than being masked by re-evaluating the node as a value.
            ResolvedArgument::ReferenceError(error) => Err(error),
            // `resolve_once` already folds range-shaped and array values into
            // `Range`, so these two arms are defensive. They still apply the
            // cancellation token so the invariant changing could never silently
            // drop cancellation on this path.
            ResolvedArgument::Value(CalcValue::Range(view)) => {
                Ok(self.with_context_cancel_token(view))
            }
            ResolvedArgument::Value(CalcValue::Scalar(LiteralValue::Array(rows))) => {
                RangeView::try_from_owned_rows(
                    rows,
                    self.interp.context.date_system(),
                    self.interp.context.cancellation_token(),
                )
            }
            ResolvedArgument::Value(_) => Err(ExcelError::new(ExcelErrorKind::Ref)
                .with_message("Argument cannot be interpreted as a range.")),
        }
    }

    /// Resolve this argument to a [`RangeView`], promoting a scalar to a 1x1 view.
    ///
    /// Excel treats a scalar handed to a range-consuming function as a 1x1 array,
    /// so `=TRANSPOSE(2)` is `2` rather than an error. [`Self::range_view`] rejects
    /// scalars, and a function that wants the Excel behaviour opts in here.
    ///
    /// This is deliberately *not* the behaviour of `range_view` itself. Several
    /// builtins use a `range_view` failure as type dispatch, where "scalar" and
    /// "1x1 range" mean genuinely different things -- `MEDIAN(TRUE)` is `1`
    /// because a direct scalar is coerced while a range cell of the same type is
    /// skipped, and a D-function's scalar criteria argument is an error rather
    /// than an empty criteria block that matches every row. Promoting inside
    /// `range_view` would silently change those answers. The rule for choosing
    /// between the two: a function that distinguishes a scalar argument from a
    /// 1x1 range keeps `range_view`.
    ///
    /// An error scalar propagates as an error rather than becoming a 1x1 view
    /// containing it, so `=TRANSPOSE(NA())` is `#N/A` instead of being masked as
    /// `#REF!`.
    pub fn range_view_or_scalar(&self) -> Result<RangeView<'b>, ExcelError> {
        match self.resolve_once()? {
            ResolvedArgument::Range(view) => Ok(view),
            ResolvedArgument::ReferenceError(error) => Err(error),
            ResolvedArgument::Value(CalcValue::Range(view)) => {
                Ok(self.with_context_cancel_token(view))
            }
            ResolvedArgument::Value(CalcValue::Scalar(LiteralValue::Array(rows))) => {
                RangeView::try_from_owned_rows(
                    rows,
                    self.interp.context.date_system(),
                    self.interp.context.cancellation_token(),
                )
            }
            // Preserve the argument's own error instead of reporting the shape
            // mismatch that rejecting it would produce.
            ResolvedArgument::Value(CalcValue::Scalar(LiteralValue::Error(error))) => Err(error),
            ResolvedArgument::Value(
                CalcValue::Scalar(scalar) | CalcValue::AnnotatedScalar(scalar, _),
            ) => RangeView::try_from_owned_rows(
                vec![vec![scalar]],
                self.interp.context.date_system(),
                self.interp.context.cancellation_token(),
            ),
            // A lambda is not a value that can stand in for a 1x1 array.
            ResolvedArgument::Value(CalcValue::Callable(_)) => {
                Err(ExcelError::new(ExcelErrorKind::Ref)
                    .with_message("Argument cannot be interpreted as a range."))
            }
        }
    }

    pub fn value_or_range(&self) -> Result<EvaluatedArg<'_>, ExcelError> {
        self.range().map(EvaluatedArg::Range).or_else(|_| {
            self.value()
                .map(|cv| EvaluatedArg::LiteralValue(Cow::Owned(cv.into_literal())))
        })
    }

    /// Lazily iterate values for this argument in row-major expansion order.
    /// - Reference: stream via RangeView (row-major)
    /// - Array literal: evaluate each element lazily per cell
    /// - Scalar/other expressions: a single value
    pub fn lazy_values_owned(
        &'a self,
    ) -> Result<Box<dyn Iterator<Item = LiteralValue> + 'a>, ExcelError> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { .. } => {
                    let view = self.range_view()?;
                    let mut values: Vec<LiteralValue> = Vec::new();
                    view.for_each_cell(&mut |v| {
                        values.push(v.clone());
                        Ok(())
                    })?;
                    Ok(Box::new(values.into_iter()))
                }
                ASTNodeType::Array(rows) => {
                    struct ArrayEvalIter<'a, 'b> {
                        rows: &'a [Vec<ASTNode>],
                        r: usize,
                        c: usize,
                        interp: &'a Interpreter<'b>,
                    }
                    impl<'a, 'b> Iterator for ArrayEvalIter<'a, 'b> {
                        type Item = LiteralValue;
                        fn next(&mut self) -> Option<Self::Item> {
                            if self.rows.is_empty() {
                                return None;
                            }
                            let rows = self.rows;
                            let mut r = self.r;
                            let mut c = self.c;
                            if r >= rows.len() {
                                return None;
                            }
                            let node = &rows[r][c];
                            // advance indices
                            c += 1;
                            if c >= rows[r].len() {
                                r += 1;
                                c = 0;
                            }
                            self.r = r;
                            self.c = c;
                            match self.interp.evaluate_ast(node) {
                                Ok(cv) => Some(cv.into_literal()),
                                Err(e) => Some(LiteralValue::Error(e)),
                            }
                        }
                    }
                    let it = ArrayEvalIter {
                        rows,
                        r: 0,
                        c: 0,
                        interp: self.interp,
                    };
                    Ok(Box::new(it))
                }
                _ => {
                    // Single value expression
                    let v = self.value()?.into_literal();
                    Ok(Box::new(std::iter::once(v)))
                }
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let node = data_store.get_node(*id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                })?;

                match node {
                    crate::engine::arena::AstNodeData::Reference { .. } => {
                        let view = self.range_view()?;
                        let mut values: Vec<LiteralValue> = Vec::new();
                        view.for_each_cell(&mut |v| {
                            values.push(v.clone());
                            Ok(())
                        })?;
                        Ok(Box::new(values.into_iter()))
                    }
                    crate::engine::arena::AstNodeData::Array { .. } => {
                        let (rows, cols, elements) =
                            data_store.get_array_elems(*id).ok_or_else(|| {
                                ExcelError::new(ExcelErrorKind::Value).with_message("Invalid array")
                            })?;

                        struct ArenaArrayEvalIter<'a, 'b> {
                            elements: &'a [crate::engine::arena::AstNodeId],
                            idx: usize,
                            interp: &'a Interpreter<'b>,
                            data_store: &'a crate::engine::arena::DataStore,
                            sheet_registry: &'a crate::engine::sheet_registry::SheetRegistry,
                        }

                        impl<'a, 'b> Iterator for ArenaArrayEvalIter<'a, 'b> {
                            type Item = LiteralValue;

                            fn next(&mut self) -> Option<Self::Item> {
                                let id = self.elements.get(self.idx).copied()?;
                                self.idx += 1;
                                match self.interp.evaluate_arena_ast(
                                    id,
                                    self.data_store,
                                    self.sheet_registry,
                                ) {
                                    Ok(cv) => Some(cv.into_literal()),
                                    Err(e) => Some(LiteralValue::Error(e)),
                                }
                            }
                        }

                        let _ = (rows, cols);
                        let it = ArenaArrayEvalIter {
                            elements,
                            idx: 0,
                            interp: self.interp,
                            data_store,
                            sheet_registry,
                        };
                        Ok(Box::new(it))
                    }
                    _ => {
                        let v = self
                            .interp
                            .evaluate_arena_ast(*id, data_store, sheet_registry)?;
                        Ok(Box::new(std::iter::once(v.into_literal())))
                    }
                }
            }
        }
    }

    pub fn ast(&self) -> &ASTNode {
        match &self.expr {
            ArgumentExpr::Ast(node) => node,
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => self.cached_ast.get_or_init(|| {
                data_store
                    .retrieve_ast(*id, sheet_registry)
                    .unwrap_or_else(|| ASTNode {
                        node_type: ASTNodeType::Literal(LiteralValue::Error(
                            ExcelError::new(ExcelErrorKind::Value)
                                .with_message("Missing formula AST"),
                        )),
                        source_token: None,
                        contains_volatile: false,
                    })
            }),
        }
    }

    /// Returns the raw reference from the AST when this argument is a reference.
    /// This does not evaluate the reference or materialize values.
    pub fn as_reference(&self) -> Result<&ReferenceType, ExcelError> {
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { reference, .. } => Ok(reference),
                _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                    .with_message("Expected a reference (by-ref argument)")),
            },
            ArgumentExpr::Arena { .. } => {
                let reference = self.reference_for_eval()?;
                Ok(self.cached_ref.get_or_init(|| reference))
            }
        }
    }

    /// Returns a `ReferenceType` if this argument is a reference or a function that
    /// can yield a reference via `eval_reference`. Materializes no values.
    ///
    /// A name that holds a value (a LET or LAMBDA local bound to a value, a
    /// defined name such as `={1,2,3}`) is not a reference: `#VALUE!`.
    pub fn as_reference_or_eval(&self) -> Result<ReferenceType, ExcelError> {
        let written = |reference: &ReferenceType| {
            self.written_reference(reference).unwrap_or_else(|| {
                Err(ExcelError::new(ExcelErrorKind::Value)
                    .with_message("The name holds a value, not a reference"))
            })
        };
        match &self.expr {
            ArgumentExpr::Ast(node) => match &node.node_type {
                ASTNodeType::Reference { reference, .. } => written(reference),
                ASTNodeType::Function { .. } | ASTNodeType::BinaryOp { .. } => {
                    self.interp.evaluate_ast_as_reference(node)
                }
                _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                    .with_message("Argument is not a reference")),
            },
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => {
                let node = data_store.get_node(*id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                })?;

                match node {
                    crate::engine::arena::AstNodeData::Reference { ref_type, .. } => written(
                        &data_store.reconstruct_reference_type_for_eval(ref_type, sheet_registry),
                    ),
                    crate::engine::arena::AstNodeData::Function { .. }
                    | crate::engine::arena::AstNodeData::BinaryOp { .. } => self
                        .interp
                        .evaluate_arena_ast_as_reference(*id, data_store, sheet_registry),
                    _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                        .with_message("Argument is not a reference")),
                }
            }
        }
    }

    /// The areas of a multi-area reference argument, numbered in the order
    /// written: a union such as `(A1:B2,D1:E2)`, a name defined as one, or an
    /// intersection with such a reference (see
    /// [`Interpreter::evaluate_ast_as_areas`]). `None` for any other
    /// argument, a single range included.
    pub(crate) fn reference_areas(&self) -> Option<Result<Vec<ReferenceType>, ExcelError>> {
        match &self.expr {
            ArgumentExpr::Ast(node) => self.interp.evaluate_ast_as_areas(node),
            // The arena form keeps a legacy formula's argument contexts for a
            // function inside the union.
            ArgumentExpr::Arena {
                id,
                data_store,
                sheet_registry,
            } => self
                .interp
                .evaluate_arena_ast_as_areas(*id, data_store, sheet_registry),
        }
    }

    /* tiny validator helper for macro */
    pub fn matches_kind(&self, k: formualizer_common::ArgKind) -> Result<bool, ExcelError> {
        Ok(match k {
            formualizer_common::ArgKind::Any => true,
            formualizer_common::ArgKind::Range => self.range().is_ok(),
            formualizer_common::ArgKind::Number => matches!(
                self.value()?.into_literal(),
                LiteralValue::Number(_) | LiteralValue::Int(_)
            ),
            formualizer_common::ArgKind::Text => {
                matches!(self.value()?.into_literal(), LiteralValue::Text(_))
            }
            formualizer_common::ArgKind::Logical => {
                matches!(self.value()?.into_literal(), LiteralValue::Boolean(_))
            }
        })
    }
}

/* simple Vec-backed range */
#[derive(Debug, Clone)]
pub struct InMemoryRange {
    data: Vec<Vec<LiteralValue>>,
}
impl InMemoryRange {
    pub fn new(d: Vec<Vec<LiteralValue>>) -> Self {
        Self { data: d }
    }
}
impl Range for InMemoryRange {
    fn get(&self, r: usize, c: usize) -> Result<LiteralValue, ExcelError> {
        Ok(self
            .data
            .get(r)
            .and_then(|row| row.get(c))
            .cloned()
            .unwrap_or(LiteralValue::Empty))
    }
    fn dimensions(&self) -> (usize, usize) {
        (self.data.len(), self.data.first().map_or(0, |r| r.len()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/* ───────────────────────── Table abstraction ───────────────────────── */

pub trait Table: Debug + Send + Sync {
    fn get_cell(&self, row: usize, column: &str) -> Result<LiteralValue, ExcelError>;
    fn get_column(&self, column: &str) -> Result<Box<dyn Range>, ExcelError>;
    /// Ordered list of column names
    fn columns(&self) -> Vec<String> {
        vec![]
    }
    /// Number of data rows (excluding headers/totals)
    fn data_height(&self) -> usize {
        0
    }
    /// Whether the table has a header row
    fn has_headers(&self) -> bool {
        false
    }
    /// Whether the table has a totals row
    fn has_totals(&self) -> bool {
        false
    }
    /// Headers row as a 1xW range
    fn headers_row(&self) -> Option<Box<dyn Range>> {
        None
    }
    /// Totals row as a 1xW range, if present
    fn totals_row(&self) -> Option<Box<dyn Range>> {
        None
    }
    /// Entire data body as HxW range
    fn data_body(&self) -> Option<Box<dyn Range>> {
        None
    }
    fn clone_box(&self) -> Box<dyn Table>;
}
impl Table for Box<dyn Table> {
    fn get_cell(&self, r: usize, c: &str) -> Result<LiteralValue, ExcelError> {
        (**self).get_cell(r, c)
    }
    fn get_column(&self, c: &str) -> Result<Box<dyn Range>, ExcelError> {
        (**self).get_column(c)
    }
    fn columns(&self) -> Vec<String> {
        (**self).columns()
    }
    fn data_height(&self) -> usize {
        (**self).data_height()
    }
    fn has_headers(&self) -> bool {
        (**self).has_headers()
    }
    fn has_totals(&self) -> bool {
        (**self).has_totals()
    }
    fn headers_row(&self) -> Option<Box<dyn Range>> {
        (**self).headers_row()
    }
    fn totals_row(&self) -> Option<Box<dyn Range>> {
        (**self).totals_row()
    }
    fn data_body(&self) -> Option<Box<dyn Range>> {
        (**self).data_body()
    }
    fn clone_box(&self) -> Box<dyn Table> {
        (**self).clone_box()
    }
}

/* ─────────────────────── Resolver super-trait ─────────────────────── */

pub trait ReferenceResolver: Send + Sync {
    fn resolve_cell_reference(
        &self,
        sheet: Option<&str>,
        row: u32,
        col: u32,
    ) -> Result<LiteralValue, ExcelError>;
}
pub trait RangeResolver: Send + Sync {
    fn resolve_range_reference(
        &self,
        sheet: Option<&str>,
        sr: Option<u32>,
        sc: Option<u32>,
        er: Option<u32>,
        ec: Option<u32>,
    ) -> Result<Box<dyn Range>, ExcelError>;
}
pub trait NamedRangeResolver: Send + Sync {
    fn resolve_named_range_reference(
        &self,
        name: &str,
    ) -> Result<Vec<Vec<LiteralValue>>, ExcelError>;
}
pub trait TableResolver: Send + Sync {
    fn resolve_table_reference(
        &self,
        tref: &formualizer_parse::parser::TableReference,
    ) -> Result<Box<dyn Table>, ExcelError>;
}

pub trait SourceResolver: Send + Sync {
    fn source_scalar_version(&self, _name: &str) -> Option<u64> {
        None
    }

    fn resolve_source_scalar(&self, name: &str) -> Result<LiteralValue, ExcelError> {
        Err(ExcelError::new(ExcelErrorKind::NImpl)
            .with_message(format!("Source scalar not supported: {name}")))
    }

    fn source_table_version(&self, _name: &str) -> Option<u64> {
        None
    }

    fn resolve_source_table(&self, name: &str) -> Result<Box<dyn Table>, ExcelError> {
        Err(ExcelError::new(ExcelErrorKind::NImpl)
            .with_message(format!("Source table not supported: {name}")))
    }
}

pub trait Resolver: ReferenceResolver + RangeResolver + NamedRangeResolver + TableResolver {
    fn resolve_range_like(&self, r: &ReferenceType) -> Result<Box<dyn Range>, ExcelError> {
        match r {
            ReferenceType::Range {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
                ..
            } => self.resolve_range_reference(
                sheet.as_deref(),
                *start_row,
                *start_col,
                *end_row,
                *end_col,
            ),
            ReferenceType::External(_) => Err(ExcelError::new(ExcelErrorKind::NImpl)
                .with_message("External references are not supported by Resolver".to_string())),
            ReferenceType::Table(tref) => {
                let t = self.resolve_table_reference(tref)?;
                match &tref.specifier {
                    Some(TableSpecifier::Column(c)) => t.get_column(c),
                    Some(TableSpecifier::ColumnRange(start, end)) => {
                        // Build a rectangular range from start..=end columns in table order
                        let cols = t.columns();
                        let start_key = start.to_lowercase();
                        let end_key = end.to_lowercase();
                        let start_idx = cols.iter().position(|n| n.to_lowercase() == start_key);
                        let end_idx = cols.iter().position(|n| n.to_lowercase() == end_key);
                        if let (Some(mut si), Some(mut ei)) = (start_idx, end_idx) {
                            if si > ei {
                                std::mem::swap(&mut si, &mut ei);
                            }
                            // Materialize by stacking columns into a 2D array
                            let h = t.data_height();
                            let w = ei - si + 1;
                            let mut rows = vec![vec![LiteralValue::Empty; w]; h];
                            for (offset, ci) in (si..=ei).enumerate() {
                                let cname = &cols[ci];
                                let col_range = t.get_column(cname)?;
                                let (rh, _) = col_range.dimensions();
                                for (r, row) in rows.iter_mut().enumerate().take(h.min(rh)) {
                                    row[offset] = col_range.get(r, 0)?;
                                }
                            }
                            Ok(Box::new(InMemoryRange::new(rows)))
                        } else {
                            Err(ExcelError::new(ExcelErrorKind::Ref).with_message(
                                "Column range refers to unknown column(s)".to_string(),
                            ))
                        }
                    }
                    Some(TableSpecifier::SpecialItem(
                        formualizer_parse::parser::SpecialItem::Headers,
                    )) => {
                        if let Some(h) = t.headers_row() {
                            Ok(h)
                        } else {
                            Ok(Box::new(InMemoryRange::new(vec![])))
                        }
                    }
                    Some(TableSpecifier::SpecialItem(
                        formualizer_parse::parser::SpecialItem::Totals,
                    )) => {
                        if let Some(tr) = t.totals_row() {
                            Ok(tr)
                        } else {
                            Ok(Box::new(InMemoryRange::new(vec![])))
                        }
                    }
                    Some(TableSpecifier::SpecialItem(
                        formualizer_parse::parser::SpecialItem::Data,
                    )) => {
                        if let Some(body) = t.data_body() {
                            Ok(body)
                        } else {
                            Ok(Box::new(InMemoryRange::new(vec![])))
                        }
                    }
                    Some(TableSpecifier::SpecialItem(
                        formualizer_parse::parser::SpecialItem::All,
                    )) => {
                        // Equivalent to TableSpecifier::All handling
                        let mut out: Vec<Vec<LiteralValue>> = Vec::new();
                        if let Some(h) = t.headers_row() {
                            out.extend(h.iter_rows());
                        }
                        if let Some(body) = t.data_body() {
                            out.extend(body.iter_rows());
                        }
                        if let Some(tr) = t.totals_row() {
                            out.extend(tr.iter_rows());
                        }
                        Ok(Box::new(InMemoryRange::new(out)))
                    }
                    Some(TableSpecifier::SpecialItem(
                        formualizer_parse::parser::SpecialItem::ThisRow,
                    )) => Err(ExcelError::new(ExcelErrorKind::NImpl).with_message(
                        "@ (This Row) requires table-aware context; not yet supported".to_string(),
                    )),
                    Some(TableSpecifier::All) => {
                        // Concatenate headers (if any), data, totals (if any)
                        let mut out: Vec<Vec<LiteralValue>> = Vec::new();
                        if let Some(h) = t.headers_row() {
                            out.extend(h.iter_rows());
                        }
                        if let Some(body) = t.data_body() {
                            out.extend(body.iter_rows());
                        }
                        if let Some(tr) = t.totals_row() {
                            out.extend(tr.iter_rows());
                        }
                        Ok(Box::new(InMemoryRange::new(out)))
                    }
                    Some(TableSpecifier::Data) => {
                        if let Some(body) = t.data_body() {
                            Ok(body)
                        } else {
                            Ok(Box::new(InMemoryRange::new(vec![])))
                        }
                    }
                    // Defer complex combinations and row selectors for tranche 1
                    Some(TableSpecifier::Combination(_)) => Err(ExcelError::new(
                        ExcelErrorKind::NImpl,
                    )
                    .with_message("Complex structured references not yet supported".to_string())),
                    Some(TableSpecifier::Row(_)) => Err(ExcelError::new(ExcelErrorKind::NImpl)
                        .with_message("Row selectors (@/index) not yet supported".to_string())),
                    Some(TableSpecifier::Headers) | Some(TableSpecifier::Totals) => {
                        Err(ExcelError::new(ExcelErrorKind::NImpl).with_message(
                            "Legacy Headers/Totals variants not used; use SpecialItem".to_string(),
                        ))
                    }
                    None => Err(ExcelError::new(ExcelErrorKind::Ref).with_message(
                        "Table reference without specifier is unsupported".to_string(),
                    )),
                }
            }
            ReferenceType::NamedRange(n) => {
                let v = self.resolve_named_range_reference(n)?;
                Ok(Box::new(InMemoryRange::new(v)))
            }
            ReferenceType::Cell {
                sheet, row, col, ..
            } => {
                let v = self.resolve_cell_reference(sheet.as_deref(), *row, *col)?;
                Ok(Box::new(InMemoryRange::new(vec![vec![v]])))
            }
            ReferenceType::Cell3D { .. } | ReferenceType::Range3D { .. } => {
                Err(ExcelError::new(ExcelErrorKind::NImpl)
                    .with_message("3D references are not yet supported".to_string()))
            }
        }
    }
}

/* ───────────────────── EvaluationContext = Resolver+Fns ───────────── */

pub trait FunctionProvider: Send + Sync {
    fn get_function(&self, ns: &str, name: &str) -> Option<Arc<dyn Function>>;

    #[doc(hidden)]
    fn get_function_for_planning(&self, _ns: &str, _name: &str) -> Option<Arc<dyn Function>> {
        None
    }

    /// Monotonic revision for runtime function resolution and semantics used by
    /// compressed planning. Providers that cannot supply one fail closed.
    #[doc(hidden)]
    fn planning_semantic_revision(&self) -> Option<u64> {
        None
    }

    #[doc(hidden)]
    fn function_capabilities(&self, ns: &str, name: &str) -> Option<crate::function::FnCaps> {
        self.get_function(ns, name).map(|function| function.caps())
    }

    fn function_semantic_identity(
        &self,
        ns: &str,
        name: &str,
        arity: usize,
    ) -> Option<crate::function_contract::FunctionSemanticIdentity> {
        crate::function_registry::resolve_semantic_identity(self, ns, name, arity)
    }
}

pub trait EvaluationContext: Resolver + FunctionProvider + SourceResolver {
    /// Get access to the shared thread pool for parallel evaluation
    /// Returns None if parallel evaluation is disabled or unavailable
    fn thread_pool(&self) -> Option<&Arc<rayon::ThreadPool>> {
        None
    }

    /// Returns the optional shared cancellation handle for this evaluation.
    ///
    /// Custom context authors may return a clone: clones share the same signal
    /// without allocating. Consumers should retrieve the handle once before a
    /// hot loop and poll [`crate::engine::CancelToken::is_cancelled`]
    /// periodically.
    fn cancellation_token(&self) -> Option<crate::engine::CancelToken> {
        None
    }

    /// Optional chunk size hint for streaming visitors.
    fn chunk_hint(&self) -> Option<usize> {
        None
    }

    /// Resolve a reference into a `RangeView` with clear bounds.
    /// Implementations should resolve un/partially bounded references using used-region.
    fn resolve_range_view<'c>(
        &'c self,
        _reference: &ReferenceType,
        _current_sheet: &str,
    ) -> Result<RangeView<'c>, ExcelError> {
        Err(ExcelError::new(ExcelErrorKind::NImpl))
    }

    /// The reference a name defined by a reference-returning formula
    /// (`OFFSET(...)`, `A1:INDEX(...)`) evaluates to. `None` when `name` is not
    /// such a name, so callers keep the name itself.
    fn resolve_name_reference(
        &self,
        _name: &str,
        _current_sheet: &str,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        None
    }

    /// The array of references a name defined by a reference-returning formula
    /// lifted over an array (`OFFSET($B$1,{0;1;2},0)`) holds, one reference
    /// (or the error in its place) per element. `None` when `name` is not such
    /// a name.
    #[allow(clippy::type_complexity)]
    fn resolve_name_reference_array(
        &self,
        _name: &str,
        _current_sheet: &str,
    ) -> Option<Result<Vec<Vec<Result<ReferenceType, ExcelError>>>, ExcelError>> {
        None
    }

    /// Whether `name` holds a value rather than a reference: a constant, or a
    /// formula that does not evaluate to a reference (`{0,1,2}`, `MATCH(...)`,
    /// `days+1`, `IF(TRUE,42)`, a name for such a name). Arguments that accept
    /// a reference or a value take its value.
    fn is_value_name(&self, _name: &str, _current_sheet: &str) -> bool {
        false
    }

    /// The areas of a name defined as a multi-area reference
    /// (`Sheet1!$A$1:$B$2,Sheet1!$D$1:$E$2`), in the order written. `None`
    /// when `name` is not such a name.
    fn resolve_name_areas(
        &self,
        _name: &str,
        _current_sheet: &str,
    ) -> Option<Result<Vec<ReferenceType>, ExcelError>> {
        None
    }

    /// Resolve a single-cell reference as a scalar value.
    ///
    /// Default implementation preserves existing reference semantics by routing through
    /// `resolve_range_view` and extracting a 1x1 value.
    fn resolve_cell_reference_value(
        &self,
        sheet: Option<&str>,
        row: u32,
        col: u32,
        current_sheet: &str,
    ) -> Result<LiteralValue, ExcelError> {
        let reference = ReferenceType::Cell {
            sheet: sheet.map(str::to_string),
            row,
            col,
            row_abs: true,
            col_abs: true,
        };
        let view = self.resolve_range_view(&reference, current_sheet)?;
        Ok(view.as_1x1().unwrap_or(LiteralValue::Empty))
    }

    /// Resolve the effective number-format annotation of a scalar cell read.
    fn resolve_cell_format(
        &self,
        _sheet: Option<&str>,
        _row: u32,
        _col: u32,
        _current_sheet: &str,
    ) -> Option<crate::format::FormatId> {
        None
    }

    /// Resolve an interned format id to its reported class.
    fn format_class(
        &self,
        format: crate::format::FormatId,
    ) -> Option<formualizer_common::numfmt::FormatClass> {
        formualizer_common::numfmt::NumberFormat::builtin(format.0)
            .map(|format| format.class().clone())
    }

    /// Record a formula cell's derived scalar format during alternate scalar evaluation paths.
    fn record_cell_derived_format(
        &self,
        _sheet: &str,
        _row: u32,
        _col: u32,
        _format: Option<crate::format::FormatId>,
    ) {
    }

    /// Locale provider: invariant by default
    fn locale(&self) -> crate::locale::Locale {
        crate::locale::Locale::invariant()
    }

    /// Number of active sheets in the workbook, if known.
    fn workbook_sheet_count(&self) -> Option<usize> {
        None
    }

    /// File name of the workbook, when it was read from a file.
    fn workbook_file_name(&self) -> Option<String> {
        None
    }

    /// Excel-style 1-based active-sheet index for a sheet name, if known.
    fn sheet_index_by_name(&self, _sheet: &str) -> Option<usize> {
        None
    }

    /// Excel-style 1-based active-sheet index for the current formula sheet, if known.
    fn current_sheet_index(&self, current_sheet: &str) -> Option<usize> {
        self.sheet_index_by_name(current_sheet)
    }

    /// Inspect reference metadata without materializing referenced values.
    fn inspect_reference(
        &self,
        _reference: &ReferenceType,
        _current_sheet: &str,
    ) -> Result<Option<ReferenceInfo>, ExcelError> {
        Ok(None)
    }

    /// The A1 reference for the cells a workbook table's structured reference
    /// selects, on the table's own sheet (named when that is not
    /// `current_sheet`): `Table1[Qty]` is that column's data cells and
    /// `#This Row` the row of `current_cell`, the formula's cell. It reads no
    /// cell, so it records no dependency. `None` when `table` names no
    /// workbook table here (a source table, or a context without tables).
    fn structured_reference_area(
        &self,
        _table: &TableReference,
        _current_sheet: &str,
        _current_cell: Option<CellRef>,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        None
    }

    /// Retrieve formula text for a concrete cell, if that cell stores a formula.
    fn formula_text_at_cell(&self, _cell: CellRef) -> Result<Option<String>, ExcelError> {
        Ok(None)
    }

    /// Clock provider for volatile date/time builtins.
    ///
    /// Default when `system-clock` feature is enabled: `SystemClock(Local)` for
    /// Excel-compatible wall-clock behaviour.
    ///
    /// Default when `system-clock` is **disabled** (portable wasm profile): a
    /// UTC epoch `FixedClock`. Implementors that need real wall-clock time should
    /// override this method and inject an appropriate `ClockProvider`.
    fn clock(&self) -> &dyn crate::timezone::ClockProvider {
        #[cfg(feature = "system-clock")]
        {
            static DEFAULT_CLOCK: std::sync::OnceLock<crate::timezone::SystemClock> =
                std::sync::OnceLock::new();
            DEFAULT_CLOCK.get_or_init(|| {
                crate::timezone::SystemClock::new(crate::timezone::TimeZoneSpec::default())
            })
        }
        #[cfg(not(feature = "system-clock"))]
        {
            static DEFAULT_CLOCK: std::sync::OnceLock<crate::timezone::FixedClock> =
                std::sync::OnceLock::new();
            DEFAULT_CLOCK.get_or_init(|| {
                crate::timezone::FixedClock::new(
                    chrono::DateTime::UNIX_EPOCH,
                    crate::timezone::TimeZoneSpec::Utc,
                )
            })
        }
    }

    /// Timezone spec for date/time functions.
    ///
    /// Default: derived from `clock()`.
    fn timezone(&self) -> &crate::timezone::TimeZoneSpec {
        self.clock().timezone()
    }

    /// Volatile granularity. Default Always for backwards compatibility.
    fn volatile_level(&self) -> VolatileLevel {
        VolatileLevel::Always
    }

    /// A stable workbook seed for RNG composition.
    fn workbook_seed(&self) -> u64 {
        0xF0F0_D0D0_AAAA_5555
    }

    /// Recalc epoch that increments on each full recalc when appropriate.
    fn recalc_epoch(&self) -> u64 {
        0
    }

    /* ─────────────── Future-proof IO/backends hooks (default no-op) ─────────────── */

    /// Optional: Return the min/max used rows for a set of columns on a sheet.
    /// When None, the backend does not provide used-region hints.
    fn used_rows_for_columns(
        &self,
        _sheet: &str,
        _start_col: u32,
        _end_col: u32,
    ) -> Option<(u32, u32)> {
        None
    }

    /// Optional: Return the min/max used columns for a set of rows on a sheet.
    /// When None, the backend does not provide used-region hints.
    fn used_cols_for_rows(
        &self,
        _sheet: &str,
        _start_row: u32,
        _end_row: u32,
    ) -> Option<(u32, u32)> {
        None
    }

    /// Optional: Physical sheet bounds (max rows, max cols) if known.
    fn sheet_bounds(&self, _sheet: &str) -> Option<(u32, u32)> {
        None
    }

    /// Monotonic identifier for the current data snapshot; increments on mutation.
    fn data_snapshot_id(&self) -> u64 {
        0
    }

    /// Backend capability advertisement for IO/adapters.
    fn backend_caps(&self) -> BackendCaps {
        BackendCaps::default()
    }

    // Flats removed

    /// Workbook date system selection (1900 vs 1904).
    /// Defaults to 1900 for compatibility.
    fn date_system(&self) -> crate::engine::DateSystem {
        crate::engine::DateSystem::Excel1900
    }

    /// Optional: Build or fetch an exact-match lookup index over an Arrow-backed view.
    /// Implementations should return None if not supported or unsafe.
    fn build_lookup_index(
        &self,
        _view: &RangeView<'_>,
        _axis: LookupAxis,
    ) -> Option<std::sync::Arc<LookupIndex>> {
        None
    }

    /// Optional: Build or fetch a cached boolean mask for a criterion over an Arrow-backed view.
    /// Implementations should return None if not supported.
    fn build_criteria_mask(
        &self,
        _view: &RangeView<'_>,
        _col_in_view: usize,
        _pred: &crate::args::CriteriaPredicate,
    ) -> Option<std::sync::Arc<arrow_array::BooleanArray>> {
        None
    }

    /// Optional: Build row-visibility mask aligned to `view` rows.
    /// Returns None if not supported by the underlying context.
    fn build_row_visibility_mask(
        &self,
        _view: &RangeView<'_>,
        _mode: VisibilityMaskMode,
    ) -> Option<std::sync::Arc<arrow_array::BooleanArray>> {
        None
    }

    /// Optional: `(row, column)` offsets within `view` of the cells whose
    /// formulas call SUBTOTAL or AGGREGATE, which those functions skip.
    /// Returns None if not supported by the underlying context.
    fn nested_aggregate_cells(
        &self,
        _view: &RangeView<'_>,
    ) -> Option<std::collections::HashSet<(usize, usize)>> {
        None
    }
}

/// Minimal backend capability descriptor for planning and adapters.
#[derive(Copy, Clone, Debug, Default)]
pub struct BackendCaps {
    /// Provides lazy access (// TODO REMOVE?)
    pub streaming: bool,
    /// Can compute used-region for rows/columns
    pub used_region: bool,
    /// Supports write-back mutations via external sink
    pub write: bool,
    /// Provides table metadata/streaming beyond basic column access
    pub tables: bool,
    /// May provide asynchronous/lazy remote streams (reserved)
    pub async_stream: bool,
}

/* ───────────────────── FunctionContext (narrow) ───────────────────── */

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VolatileLevel {
    /// Value can change at any edit; seed excludes recalc_epoch by default.
    Always,
    /// Value changes per recalculation; seed should include recalc_epoch.
    OnRecalc,
    /// Value changes per open; seed uses only workbook_seed.
    OnOpen,
}

/// Minimal context exposed to functions (no engine/graph APIs)
pub trait FunctionContext<'ctx> {
    fn locale(&self) -> crate::locale::Locale;
    fn timezone(&self) -> &crate::timezone::TimeZoneSpec;
    fn clock(&self) -> &dyn crate::timezone::ClockProvider;
    fn thread_pool(&self) -> Option<&std::sync::Arc<rayon::ThreadPool>>;
    /// Returns the optional shared cancellation handle for this evaluation.
    ///
    /// Custom function authors should retrieve this once before a hot loop and
    /// poll [`crate::engine::CancelToken::is_cancelled`] periodically. Cloning
    /// the handle shares the same signal without allocating.
    fn cancellation_token(&self) -> Option<crate::engine::CancelToken>;
    fn chunk_hint(&self) -> Option<usize>;

    /// Current formula sheet name.
    fn current_sheet(&self) -> &str;

    fn workbook_sheet_count(&self) -> Option<usize> {
        None
    }

    fn workbook_file_name(&self) -> Option<String> {
        None
    }

    fn sheet_index_by_name(&self, _sheet: &str) -> Option<usize> {
        None
    }

    fn current_sheet_index(&self) -> Option<usize> {
        self.sheet_index_by_name(self.current_sheet())
    }

    fn inspect_reference(
        &self,
        _reference: &ReferenceType,
    ) -> Result<Option<ReferenceInfo>, ExcelError> {
        Ok(None)
    }

    /// The A1 reference for the cells a workbook table's structured reference
    /// selects, with `#This Row` at the formula's row; see
    /// [`EvaluationContext::structured_reference_area`].
    fn structured_reference_area(
        &self,
        _table: &TableReference,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        None
    }

    fn formula_text_at_cell(&self, _cell: CellRef) -> Result<Option<String>, ExcelError> {
        Ok(None)
    }

    fn volatile_level(&self) -> VolatileLevel;
    fn workbook_seed(&self) -> u64;
    fn recalc_epoch(&self) -> u64;
    fn current_cell(&self) -> Option<CellRef>;

    /// Resolve a reference into a RangeView using the underlying engine context.
    fn resolve_range_view(
        &self,
        _reference: &ReferenceType,
        _current_sheet: &str,
    ) -> Result<RangeView<'ctx>, ExcelError>;

    // Flats removed

    /// Deterministic RNG seeded for the current evaluation site and function salt.
    fn rng_for_current(&self, fn_salt: u64) -> rand::rngs::SmallRng {
        use crate::rng::{compose_seed, small_rng_from_lanes};
        let (sheet_id, row, col) = self
            .current_cell()
            .map(|c| (c.sheet_id as u32, c.coord.row(), c.coord.col()))
            .unwrap_or((0, 0, 0));
        // Include epoch only for OnRecalc
        let epoch = match self.volatile_level() {
            VolatileLevel::OnRecalc => self.recalc_epoch(),
            _ => 0,
        };
        let (l0, l1) = compose_seed(self.workbook_seed(), sheet_id, row, col, fn_salt, epoch);
        small_rng_from_lanes(l0, l1)
    }

    /// Workbook date system selection (1900 vs 1904).
    fn date_system(&self) -> crate::engine::DateSystem {
        crate::engine::DateSystem::Excel1900
    }

    /// Optional: Build or fetch an exact-match lookup index over an Arrow-backed view.
    /// Returns None if not supported by the underlying context.
    fn get_lookup_index(
        &self,
        _view: &RangeView<'_>,
        _axis: LookupAxis,
    ) -> Option<std::sync::Arc<LookupIndex>> {
        None
    }

    /// Optional: Build or fetch a cached boolean mask for a criterion over an Arrow-backed view.
    /// Returns None if not supported by the underlying context.
    fn get_criteria_mask(
        &self,
        _view: &RangeView<'_>,
        _col_in_view: usize,
        _pred: &crate::args::CriteriaPredicate,
    ) -> Option<std::sync::Arc<arrow_array::BooleanArray>> {
        None
    }

    /// Optional: Build row-visibility mask aligned to `view` rows.
    fn get_row_visibility_mask(
        &self,
        _view: &RangeView<'_>,
        _mode: VisibilityMaskMode,
    ) -> Option<std::sync::Arc<arrow_array::BooleanArray>> {
        None
    }

    /// Optional: offsets within `view` of cells whose formulas call SUBTOTAL
    /// or AGGREGATE (see [`EvaluationContext::nested_aggregate_cells`]).
    fn nested_aggregate_cells(
        &self,
        _view: &RangeView<'_>,
    ) -> Option<std::collections::HashSet<(usize, usize)>> {
        None
    }
}

/// `reference` as the cells it selects: a workbook table's structured
/// reference becomes their A1 reference (see
/// [`FunctionContext::structured_reference_area`]), so `Table1[Qty]` is read,
/// offset and combined like the range it names; any other reference is
/// returned as it is. The error a structured reference selecting no cell
/// evaluates to (`#This Row` off the data body, hidden headers) is the result.
pub(crate) fn reference_as_area(
    ctx: &dyn FunctionContext<'_>,
    reference: ReferenceType,
) -> Result<ReferenceType, ExcelError> {
    if let ReferenceType::Table(table) = &reference
        && let Some(area) = ctx.structured_reference_area(table)
    {
        return area;
    }
    Ok(reference)
}

/// Default adapter that wraps an EvaluationContext and provides the narrow FunctionContext.
pub struct DefaultFunctionContext<'a> {
    pub base: &'a dyn EvaluationContext,
    pub current: Option<CellRef>,
    pub current_sheet: &'a str,
}

impl<'a> DefaultFunctionContext<'a> {
    pub fn new(
        base: &'a dyn EvaluationContext,
        current: Option<CellRef>,
        current_sheet: &'a str,
    ) -> Self {
        Self {
            base,
            current,
            current_sheet,
        }
    }

    pub fn new_with_sheet(
        base: &'a dyn EvaluationContext,
        current: Option<CellRef>,
        current_sheet: &'a str,
    ) -> Self {
        Self::new(base, current, current_sheet)
    }
}

impl<'a> FunctionContext<'a> for DefaultFunctionContext<'a> {
    fn locale(&self) -> crate::locale::Locale {
        self.base.locale()
    }

    fn current_sheet(&self) -> &str {
        self.current_sheet
    }

    fn workbook_sheet_count(&self) -> Option<usize> {
        self.base.workbook_sheet_count()
    }

    fn workbook_file_name(&self) -> Option<String> {
        self.base.workbook_file_name()
    }

    fn sheet_index_by_name(&self, sheet: &str) -> Option<usize> {
        self.base.sheet_index_by_name(sheet)
    }

    fn current_sheet_index(&self) -> Option<usize> {
        self.base.current_sheet_index(self.current_sheet)
    }

    fn inspect_reference(
        &self,
        reference: &ReferenceType,
    ) -> Result<Option<ReferenceInfo>, ExcelError> {
        self.base.inspect_reference(reference, self.current_sheet)
    }

    fn structured_reference_area(
        &self,
        table: &TableReference,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        self.base
            .structured_reference_area(table, self.current_sheet, self.current)
    }

    fn formula_text_at_cell(&self, cell: CellRef) -> Result<Option<String>, ExcelError> {
        self.base.formula_text_at_cell(cell)
    }

    fn timezone(&self) -> &crate::timezone::TimeZoneSpec {
        self.base.timezone()
    }

    fn clock(&self) -> &dyn crate::timezone::ClockProvider {
        self.base.clock()
    }
    fn thread_pool(&self) -> Option<&std::sync::Arc<rayon::ThreadPool>> {
        self.base.thread_pool()
    }
    fn cancellation_token(&self) -> Option<crate::engine::CancelToken> {
        self.base.cancellation_token()
    }
    fn chunk_hint(&self) -> Option<usize> {
        self.base.chunk_hint()
    }

    fn volatile_level(&self) -> VolatileLevel {
        self.base.volatile_level()
    }
    fn workbook_seed(&self) -> u64 {
        self.base.workbook_seed()
    }
    fn recalc_epoch(&self) -> u64 {
        self.base.recalc_epoch()
    }
    fn current_cell(&self) -> Option<CellRef> {
        self.current
    }

    fn resolve_range_view(
        &self,
        reference: &ReferenceType,
        current_sheet: &str,
    ) -> Result<RangeView<'a>, ExcelError> {
        self.base.resolve_range_view(reference, current_sheet)
    }

    // Flats removed

    fn date_system(&self) -> crate::engine::DateSystem {
        self.base.date_system()
    }

    fn get_lookup_index(
        &self,
        view: &RangeView<'_>,
        axis: LookupAxis,
    ) -> Option<std::sync::Arc<LookupIndex>> {
        self.base.build_lookup_index(view, axis)
    }

    fn get_criteria_mask(
        &self,
        view: &RangeView<'_>,
        col_in_view: usize,
        pred: &crate::args::CriteriaPredicate,
    ) -> Option<std::sync::Arc<arrow_array::BooleanArray>> {
        self.base.build_criteria_mask(view, col_in_view, pred)
    }

    fn get_row_visibility_mask(
        &self,
        view: &RangeView<'_>,
        mode: VisibilityMaskMode,
    ) -> Option<std::sync::Arc<arrow_array::BooleanArray>> {
        self.base.build_row_visibility_mask(view, mode)
    }

    fn nested_aggregate_cells(
        &self,
        view: &RangeView<'_>,
    ) -> Option<std::collections::HashSet<(usize, usize)>> {
        self.base.nested_aggregate_cells(view)
    }
}
