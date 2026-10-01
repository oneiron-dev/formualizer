use crate::{
    CellRef,
    broadcast::{broadcast_shape, project_index},
    coercion,
    traits::{ArgumentHandle, DefaultFunctionContext, EvaluationContext},
};
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ASTNode, ASTNodeType, ReferenceType};
use rustc_hash::FxHashMap;
use std::{borrow::Cow, sync::Arc};

use crate::engine::arena::ast::SheetKey;
use crate::engine::arena::{AstNodeData, AstNodeId, CompactRefType, DataStore};
use crate::engine::sheet_registry::SheetRegistry;
use crate::engine::used_extent::{
    ExtentPolicy, OpenRangeBounds, resolve_used_extent_with_fallback,
};
use crate::formula_plane::template_canonical::LiteralSlotId;

pub(crate) fn probe_range_dimensions<C: EvaluationContext + ?Sized>(
    context: &C,
    current_sheet: &str,
    reference: &ReferenceType,
) -> Option<(u32, u32)> {
    match reference {
        ReferenceType::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        } => {
            let sheet_name = sheet.as_deref().unwrap_or(current_sheet);
            let extent = resolve_used_extent_with_fallback(
                OpenRangeBounds {
                    start_row: *start_row,
                    start_column: *start_col,
                    end_row: *end_row,
                    end_column: *end_col,
                },
                ExtentPolicy::EvaluationCompat {
                    fallback_row: None,
                    fallback_column: None,
                },
                || context.sheet_bounds(sheet_name).map(|bounds| bounds.0),
                || context.sheet_bounds(sheet_name).map(|bounds| bounds.1),
                |first, last| context.used_rows_for_columns(sheet_name, first, last),
                |first, last| context.used_cols_for_rows(sheet_name, first, last),
            );
            let Some(extent) = extent else {
                return Some((0, 0));
            };
            Some((
                extent.end_row - extent.start_row + 1,
                extent.end_column - extent.start_column + 1,
            ))
        }
        ReferenceType::Cell { .. } => Some((1, 1)),
        _ => None,
    }
}

#[derive(Clone)]
pub enum LocalBinding {
    Value(LiteralValue),
    /// A reference (`LET(c,A:A,...)`, a LAMBDA called with `B1:B3`). As in
    /// Excel, the name stays that reference: it reads as the referenced cells,
    /// and a parameter that takes a reference (SUMIFS's ranges, ROW, OFFSET)
    /// receives the reference itself, a whole column at its full height.
    Reference(ReferenceType),
    Callable(Arc<dyn crate::traits::CustomCallable>),
    /// An array of references (`LET(r,OFFSET(A1,{0;1},0),...)`). It has no
    /// value of its own (`#VALUE!`); reference parameters and N/T read each
    /// reference (see `ArgumentHandle::reference_array`).
    #[allow(clippy::type_complexity)]
    References(Arc<Vec<Vec<Result<ReferenceType, ExcelError>>>>),
}

#[derive(Clone, Default)]
pub struct LocalEnv {
    head: Option<Arc<EnvFrame>>,
}

#[derive(Clone)]
struct EnvFrame {
    parent: Option<Arc<EnvFrame>>,
    bindings: FxHashMap<String, LocalBinding>,
}

impl LocalEnv {
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    fn norm(name: &str) -> String {
        name.to_ascii_uppercase()
    }

    pub fn lookup(&self, name: &str) -> Option<LocalBinding> {
        self.head.as_ref()?;
        let key = Self::norm(name);
        let mut cur = self.head.as_ref().cloned();
        while let Some(frame) = cur {
            if let Some(v) = frame.bindings.get(&key) {
                return Some(v.clone());
            }
            cur = frame.parent.clone();
        }
        None
    }

    /// [`Self::lookup`] without copying the binding.
    pub(crate) fn get(&self, name: &str) -> Option<&LocalBinding> {
        let key = Self::norm(name);
        let mut frame = self.head.as_deref();
        while let Some(current) = frame {
            if let Some(binding) = current.bindings.get(&key) {
                return Some(binding);
            }
            frame = current.parent.as_deref();
        }
        None
    }

    pub fn with_binding(&self, name: &str, value: LocalBinding) -> Self {
        let mut bindings = FxHashMap::default();
        bindings.insert(Self::norm(name), value);
        Self {
            head: Some(Arc::new(EnvFrame {
                parent: self.head.clone(),
                bindings,
            })),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct InterpreterParameterBindings<'a> {
    pub(crate) literal_slots_by_node: &'a FxHashMap<AstNodeId, LiteralSlotId>,
    pub(crate) literal_values: &'a [LiteralValue],
}

pub struct Interpreter<'a> {
    pub context: &'a dyn EvaluationContext,
    current_sheet: &'a str,
    current_cell: Option<crate::CellRef>,
    local_env: LocalEnv,
    reference_row_delta: i64,
    reference_col_delta: i64,
    disable_ast_planner: bool,
    parameter_bindings: Option<InterpreterParameterBindings<'a>>,
    /// Set while evaluating a formula entered without the array flag (see
    /// [`LegacyContext`]); `None` evaluates every expression as an array.
    legacy: Option<LegacyContext>,
}

/// How a formula entered without the array flag evaluates the expression at
/// hand. Excel evaluates such a formula as a value: a range in a single-value
/// position is implicitly intersected with the formula cell, unless the
/// position belongs to an argument that Excel evaluates as an array
/// (SUMPRODUCT, INDEX's array, ...). See [`crate::lift::LegacyArg`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyContext {
    Value,
    Array,
}

/// A function's error is its value: ISNUMBER(SEARCH("x",#REF!)) is FALSE and
/// IF(FALSE,...) never sees it. Only cancellation aborts the formula.
fn error_as_value<'a>(
    result: Result<crate::traits::CalcValue<'a>, ExcelError>,
) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
    match result {
        Err(error) if error.kind != ExcelErrorKind::Cancelled => {
            Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)))
        }
        other => other,
    }
}

impl<'a> Interpreter<'a> {
    pub fn new(context: &'a dyn EvaluationContext, current_sheet: &'a str) -> Self {
        Self {
            context,
            current_sheet,
            current_cell: None,
            local_env: LocalEnv::default(),
            reference_row_delta: 0,
            reference_col_delta: 0,
            disable_ast_planner: false,
            parameter_bindings: None,
            legacy: None,
        }
    }

    pub fn new_with_cell(
        context: &'a dyn EvaluationContext,
        current_sheet: &'a str,
        cell: crate::CellRef,
    ) -> Self {
        Self {
            context,
            current_sheet,
            current_cell: Some(cell),
            local_env: LocalEnv::default(),
            reference_row_delta: 0,
            reference_col_delta: 0,
            disable_ast_planner: false,
            parameter_bindings: None,
            legacy: None,
        }
    }

    pub fn current_sheet(&self) -> &'a str {
        self.current_sheet
    }

    /// The cell whose formula is being evaluated, when there is one.
    pub(crate) fn current_cell(&self) -> Option<crate::CellRef> {
        self.current_cell
    }

    /// `reference` as the cells it selects: a workbook table's structured
    /// reference is their A1 reference, with `#This Row` at the formula's row,
    /// so it composes with `:` and ` ` like the range it names.
    fn reference_as_area(&self, reference: ReferenceType) -> Result<ReferenceType, ExcelError> {
        if let ReferenceType::Table(table) = &reference
            && let Some(area) =
                self.context
                    .structured_reference_area(table, self.current_sheet, self.current_cell)
        {
            return area;
        }
        Ok(reference)
    }

    /// The range operator `a:b`: the smallest area holding both references.
    fn combine_reference_areas(
        &self,
        a: ReferenceType,
        b: ReferenceType,
    ) -> Result<ReferenceType, ExcelError> {
        crate::reference::combine_references(
            &self.reference_as_area(a)?,
            &self.reference_as_area(b)?,
            self.current_sheet,
        )
    }

    /// The intersection operator `a b`: the cells both references hold.
    fn intersect_reference_areas(
        &self,
        a: ReferenceType,
        b: ReferenceType,
    ) -> Result<Option<ReferenceType>, ExcelError> {
        crate::reference::intersect_references(
            &self.reference_as_area(a)?,
            &self.reference_as_area(b)?,
        )
    }

    pub fn local_env(&self) -> &LocalEnv {
        &self.local_env
    }

    pub(crate) fn with_current_cell(&self, cell: crate::CellRef) -> Self {
        Self {
            context: self.context,
            current_sheet: self.current_sheet,
            current_cell: Some(cell),
            local_env: self.local_env.clone(),
            reference_row_delta: self.reference_row_delta,
            reference_col_delta: self.reference_col_delta,
            disable_ast_planner: self.disable_ast_planner,
            parameter_bindings: self.parameter_bindings,
            legacy: self.legacy,
        }
    }

    pub fn with_local_env(&self, env: LocalEnv) -> Self {
        Self {
            context: self.context,
            current_sheet: self.current_sheet,
            current_cell: self.current_cell,
            local_env: env,
            reference_row_delta: self.reference_row_delta,
            reference_col_delta: self.reference_col_delta,
            disable_ast_planner: self.disable_ast_planner,
            parameter_bindings: self.parameter_bindings,
            legacy: self.legacy,
        }
    }

    pub(crate) fn with_parameter_bindings(
        &self,
        bindings: InterpreterParameterBindings<'a>,
    ) -> Self {
        Self {
            context: self.context,
            current_sheet: self.current_sheet,
            current_cell: self.current_cell,
            local_env: self.local_env.clone(),
            reference_row_delta: self.reference_row_delta,
            reference_col_delta: self.reference_col_delta,
            disable_ast_planner: self.disable_ast_planner,
            parameter_bindings: Some(bindings),
            legacy: self.legacy,
        }
    }

    /// Evaluate as the formula of a cell entered without the array flag.
    pub(crate) fn as_legacy_formula(mut self) -> Self {
        self.legacy = Some(LegacyContext::Value);
        self
    }

    fn with_legacy_context(&self, legacy: Option<LegacyContext>) -> Self {
        Self {
            context: self.context,
            current_sheet: self.current_sheet,
            current_cell: self.current_cell,
            local_env: self.local_env.clone(),
            reference_row_delta: self.reference_row_delta,
            reference_col_delta: self.reference_col_delta,
            disable_ast_planner: self.disable_ast_planner,
            parameter_bindings: self.parameter_bindings,
            legacy,
        }
    }

    /// Whether a legacy formula evaluates the expression at hand as a single
    /// value (see [`LegacyContext`]).
    pub(crate) fn in_legacy_value_context(&self) -> bool {
        self.legacy == Some(LegacyContext::Value)
    }

    fn effective_reference<'r>(
        &self,
        reference: &'r ReferenceType,
    ) -> Result<Cow<'r, ReferenceType>, ExcelError> {
        if self.reference_row_delta == 0 && self.reference_col_delta == 0 {
            return Ok(Cow::Borrowed(reference));
        }

        Ok(Cow::Owned(relocate_reference_for_offset(
            reference,
            self.reference_row_delta,
            self.reference_col_delta,
        )?))
    }

    fn resolve_local_reference(
        &self,
        reference: &ReferenceType,
    ) -> Option<Result<crate::traits::CalcValue<'a>, ExcelError>> {
        if self.local_env.is_empty() {
            return None;
        }
        let name = match reference {
            ReferenceType::NamedRange(name) => name,
            _ => return None,
        };
        Some(self.binding_value(self.local_env.lookup(name)?))
    }

    /// The value a local binding reads as. A bound reference reads exactly
    /// like the same reference written in its place.
    pub(crate) fn binding_value(
        &self,
        binding: LocalBinding,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        match binding {
            LocalBinding::Value(v) => Ok(crate::traits::CalcValue::Scalar(v)),
            LocalBinding::Reference(reference) => self.eval_reference_to_calc(&reference),
            LocalBinding::Callable(c) => Ok(crate::traits::CalcValue::Callable(c)),
            LocalBinding::References(_) => Ok(crate::traits::CalcValue::Scalar(
                LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value)),
            )),
        }
    }

    fn resolve_local_callable(&self, name: &str) -> Option<Arc<dyn crate::traits::CustomCallable>> {
        if self.local_env.is_empty() {
            return None;
        }
        match self.local_env.lookup(name)? {
            LocalBinding::Callable(c) => Some(c),
            LocalBinding::Value(_) | LocalBinding::Reference(_) | LocalBinding::References(_) => {
                None
            }
        }
    }

    pub fn resolve_local_name(&self, name: &str) -> Option<LocalBinding> {
        self.local_env.lookup(name)
    }

    /// Whether `name` is a LET name or LAMBDA parameter bound to something
    /// other than a reference. Such a name resolves only on the value path.
    pub(crate) fn is_local_value_name(&self, name: &str) -> bool {
        self.local_binding(name)
            .is_some_and(|binding| !matches!(binding, LocalBinding::Reference(_)))
    }

    /// The LET/LAMBDA local `name` is bound to, without copying it.
    pub(crate) fn local_binding(&self, name: &str) -> Option<&LocalBinding> {
        if self.local_env.is_empty() {
            return None;
        }
        self.local_env.get(name)
    }

    pub fn resolve_range_view<'c>(
        &'c self,
        reference: &ReferenceType,
        current_sheet: &str,
    ) -> Result<crate::engine::range_view::RangeView<'c>, ExcelError> {
        self.context.resolve_range_view(reference, current_sheet)
    }

    /// Evaluate an AST node in a reference context and return a ReferenceType.
    /// This is used for range combinators (e.g., ":"), by-ref argument flows,
    /// and spill planning. Functions that can return references must set
    /// `FnCaps::RETURNS_REFERENCE` and override `eval_reference`.
    pub fn evaluate_ast_as_reference(&self, node: &ASTNode) -> Result<ReferenceType, ExcelError> {
        match &node.node_type {
            ASTNodeType::Reference { reference, .. } => {
                self.reference_for_current_offset(reference)
            }
            ASTNodeType::Function { name, args } => {
                if let Some(fun) = self.context.get_function("", name) {
                    // Build handles; allow function to decide reference semantics
                    let handles: Vec<ArgumentHandle> =
                        args.iter().map(|n| ArgumentHandle::new(n, self)).collect();
                    let fctx = DefaultFunctionContext::new_with_sheet(
                        self.context,
                        self.current_cell,
                        self.current_sheet,
                    );
                    let _call_dates = self.enter_function_call();
                    if let Some(res) = fun.eval_reference(&handles, &fctx) {
                        res
                    } else {
                        Err(ExcelError::new(ExcelErrorKind::Ref)
                            .with_message("Function does not return a reference"))
                    }
                } else {
                    Err(ExcelError::new(ExcelErrorKind::Name)
                        .with_message(format!("Unknown function: {name}")))
                }
            }
            ASTNodeType::BinaryOp { op, left, right } if op == " " => {
                let lref = self.evaluate_ast_as_reference(left)?;
                let rref = self.evaluate_ast_as_reference(right)?;
                self.intersect_reference_areas(lref, rref)?
                    .ok_or_else(|| ExcelError::new(ExcelErrorKind::Null))
            }
            ASTNodeType::BinaryOp { op, left, right } if op == ":" => {
                let lref = self.evaluate_ast_as_reference(left)?;
                let rref = self.evaluate_ast_as_reference(right)?;
                self.combine_reference_areas(lref, rref)
            }
            ASTNodeType::Array(_)
            | ASTNodeType::UnaryOp { .. }
            | ASTNodeType::BinaryOp { .. }
            | ASTNodeType::Call { .. }
            | ASTNodeType::Literal(_)
            | ASTNodeType::Omitted => Err(ExcelError::new(ExcelErrorKind::Ref)
                .with_message("Expression cannot be used as a reference")),
        }
    }

    pub(crate) fn try_evaluate_ast_as_reference(
        &self,
        node: &ASTNode,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        let ASTNodeType::Function { name, args } = &node.node_type else {
            return Some(self.evaluate_ast_as_reference(node));
        };
        let fun = match self.context.get_function("", name) {
            Some(fun) => fun,
            None => {
                return Some(Err(ExcelError::new(ExcelErrorKind::Name)
                    .with_message(format!("Unknown function: {name}"))));
            }
        };
        let handles: Vec<ArgumentHandle> = args
            .iter()
            .map(|arg| ArgumentHandle::new(arg, self))
            .collect();
        let fctx = DefaultFunctionContext::new_with_sheet(
            self.context,
            self.current_cell,
            self.current_sheet,
        );
        let _call_dates = self.enter_function_call();
        fun.eval_reference(&handles, &fctx)
    }

    pub(crate) fn evaluate_arena_ast_as_reference(
        &self,
        node_id: AstNodeId,
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
    ) -> Result<ReferenceType, ExcelError> {
        let node = data_store.get_node(node_id).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
        })?;

        match node {
            AstNodeData::Reference { ref_type, .. } => {
                let reference =
                    data_store.reconstruct_reference_type_for_eval(ref_type, sheet_registry);
                self.reference_for_current_offset(&reference)
            }
            AstNodeData::Function { name_id, .. } => {
                let name = data_store.resolve_ast_string(*name_id);
                let fun = self.context.get_function("", name).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Name)
                        .with_message(format!("Unknown function: {name}"))
                })?;

                let args = data_store.get_args(node_id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing function args")
                })?;

                let fctx = DefaultFunctionContext::new_with_sheet(
                    self.context,
                    self.current_cell,
                    self.current_sheet,
                );
                let _call_dates = self.enter_function_call();

                self.with_arena_call_handles(
                    fun.as_ref(),
                    args,
                    data_store,
                    sheet_registry,
                    |handles| fun.eval_reference(handles, &fctx),
                )
                .ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Ref)
                        .with_message("Function does not return a reference")
                })?
            }
            AstNodeData::BinaryOp {
                op_id,
                left_id,
                right_id,
            } => {
                let op = data_store.resolve_ast_string(*op_id);
                if op != ":" && op != " " {
                    return Err(ExcelError::new(ExcelErrorKind::Ref)
                        .with_message("Expression cannot be used as a reference"));
                }
                let lref =
                    self.evaluate_arena_ast_as_reference(*left_id, data_store, sheet_registry)?;
                let rref =
                    self.evaluate_arena_ast_as_reference(*right_id, data_store, sheet_registry)?;
                if op == " " {
                    return self
                        .intersect_reference_areas(lref, rref)?
                        .ok_or_else(|| ExcelError::new(ExcelErrorKind::Null));
                }
                self.combine_reference_areas(lref, rref)
            }
            _ => Err(ExcelError::new(ExcelErrorKind::Ref)
                .with_message("Expression cannot be used as a reference")),
        }
    }

    pub(crate) fn try_evaluate_arena_ast_as_reference(
        &self,
        node_id: AstNodeId,
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
    ) -> Option<Result<ReferenceType, ExcelError>> {
        let node = match data_store.get_node(node_id) {
            Some(node) => node,
            None => {
                return Some(Err(
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
                ));
            }
        };
        let AstNodeData::Function { name_id, .. } = node else {
            return Some(self.evaluate_arena_ast_as_reference(node_id, data_store, sheet_registry));
        };
        let name = data_store.resolve_ast_string(*name_id);
        let fun = match self.context.get_function("", name) {
            Some(fun) => fun,
            None => {
                return Some(Err(ExcelError::new(ExcelErrorKind::Name)
                    .with_message(format!("Unknown function: {name}"))));
            }
        };
        let args = match data_store.get_args(node_id) {
            Some(args) => args,
            None => {
                return Some(Err(
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing function args")
                ));
            }
        };
        let fctx = DefaultFunctionContext::new_with_sheet(
            self.context,
            self.current_cell,
            self.current_sheet,
        );
        let _call_dates = self.enter_function_call();
        self.with_arena_call_handles(fun.as_ref(), args, data_store, sheet_registry, |handles| {
            fun.eval_reference(handles, &fctx)
        })
    }

    /// Run `f` on the argument handles of a call to the builtin `fun`.
    ///
    /// In a legacy formula each argument is evaluated in the context Excel
    /// gives its position ([`crate::lift::legacy_arg`]): a range in a
    /// single-value position of a value context is implicitly intersected
    /// with the formula cell, array positions evaluate their expression as an
    /// array, and the test of IF is a single value even inside an array.
    pub(crate) fn with_arena_call_handles<R>(
        &self,
        fun: &dyn crate::function::Function,
        args: &[AstNodeId],
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
        f: impl FnOnce(&[ArgumentHandle<'_, 'a>]) -> R,
    ) -> R {
        let Some(context) = self.legacy else {
            let handles: Vec<ArgumentHandle> = args
                .iter()
                .map(|&id| ArgumentHandle::new_arena(id, self, data_store, sheet_registry))
                .collect();
            return f(&handles);
        };
        let other = self.with_legacy_context(Some(match context {
            LegacyContext::Value => LegacyContext::Array,
            LegacyContext::Array => LegacyContext::Value,
        }));
        let interp_for = |arg_context: LegacyContext| {
            if arg_context == context { self } else { &other }
        };
        // Per argument: its context, the intersected value of a range in a
        // single-value position, and the value of a reference-capable call
        // already evaluated there.
        type Position<'v> = (
            LegacyContext,
            Option<ASTNode>,
            Option<Result<crate::traits::CalcValue<'v>, ExcelError>>,
        );
        let positions: Vec<Position<'a>> = args
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                use crate::lift::LegacyArg;
                let arg = crate::lift::legacy_arg(fun, index);
                let arg_context = match arg {
                    LegacyArg::Value | LegacyArg::Reference => context,
                    LegacyArg::ForcedValue | LegacyArg::Choice => LegacyContext::Value,
                    LegacyArg::Array => LegacyContext::Array,
                };
                let single = matches!(arg, LegacyArg::Value | LegacyArg::ForcedValue);
                let mut evaluated = None;
                let intersected = if single && arg_context == LegacyContext::Value {
                    match self.arena_range_reference(id, data_store, sheet_registry) {
                        Ok(Some(reference)) => {
                            Some(self.implicit_intersection_from_reference(&reference))
                        }
                        Ok(None) if self.arena_may_return_reference(id, data_store) => {
                            // A range returned by INDEX, OFFSET, IF, ... intersects too.
                            let value = interp_for(arg_context).evaluate_arena_ast(
                                id,
                                data_store,
                                sheet_registry,
                            );
                            match value {
                                Ok(crate::traits::CalcValue::Range(view))
                                    if Self::is_sheet_range(&view) =>
                                {
                                    Some(self.eval_implicit_intersection_calc(
                                        crate::traits::CalcValue::Range(view),
                                    ))
                                }
                                other => {
                                    evaluated = Some(other);
                                    None
                                }
                            }
                        }
                        Ok(None) => None,
                        Err(error) => Some(LiteralValue::Error(error)),
                    }
                } else {
                    None
                };
                let intersected =
                    intersected.map(|value| ASTNode::new(ASTNodeType::Literal(value), None));
                (arg_context, intersected, evaluated)
            })
            .collect();
        let handles: Vec<ArgumentHandle> = args
            .iter()
            .zip(positions.iter())
            .map(|(&id, (arg_context, intersected, evaluated))| {
                let interp = interp_for(*arg_context);
                match (intersected, evaluated) {
                    (Some(node), _) => ArgumentHandle::new(node, interp),
                    (None, Some(value)) => {
                        ArgumentHandle::new_arena(id, interp, data_store, sheet_registry)
                            .with_value(value.clone())
                    }
                    (None, None) => {
                        ArgumentHandle::new_arena(id, interp, data_store, sheet_registry)
                    }
                }
            })
            .collect();
        f(&handles)
    }

    /// The reference written at `node_id` when it can span several cells (a
    /// range, a whole row or column, a name or a table, or a LET/LAMBDA local
    /// bound to a range); `None` for anything else, including single cells and
    /// locals bound to values.
    fn arena_range_reference(
        &self,
        node_id: AstNodeId,
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
    ) -> Result<Option<ReferenceType>, ExcelError> {
        let Some(AstNodeData::Reference { ref_type, .. }) = data_store.get_node(node_id) else {
            return Ok(None);
        };
        if !matches!(
            ref_type,
            CompactRefType::Range { .. }
                | CompactRefType::NamedRange(_)
                | CompactRefType::Table { .. }
        ) {
            return Ok(None);
        }
        let reference = data_store.reconstruct_reference_type_for_eval(ref_type, sheet_registry);
        let reference = self.effective_reference(&reference)?.into_owned();
        // A LET name or LAMBDA parameter bound to a range is that range written
        // here; any other local is a value.
        if let ReferenceType::NamedRange(name) = &reference
            && let Some(binding) = self.resolve_local_name(name)
        {
            return Ok(match binding {
                LocalBinding::Reference(bound @ ReferenceType::Range { .. }) => Some(bound),
                _ => None,
            });
        }
        // A name for a range intersects through that range; a name for a cell,
        // a constant or a computed value evaluates as usual.
        if let ReferenceType::NamedRange(name) = &reference {
            let named = self
                .context
                .resolve_name_reference(name, self.current_sheet);
            return Ok(match named {
                Some(Ok(named)) if !matches!(named, ReferenceType::Cell { .. }) => Some(named),
                _ => None,
            });
        }
        Ok(Some(reference))
    }

    /// Whether a function call or reference operator at `node_id` may yield a
    /// reference (INDEX, OFFSET, INDIRECT, IF, CHOOSE, `:`).
    fn arena_may_return_reference(&self, node_id: AstNodeId, data_store: &DataStore) -> bool {
        match data_store.get_node(node_id) {
            Some(AstNodeData::Function { name_id, .. }) => self
                .context
                .function_capabilities("", data_store.resolve_ast_string(*name_id))
                .is_some_and(|caps| caps.contains(crate::function::FnCaps::RETURNS_REFERENCE)),
            Some(AstNodeData::BinaryOp { op_id, .. }) => {
                matches!(data_store.resolve_ast_string(*op_id), ":" | " ")
            }
            _ => false,
        }
    }

    /// Whether `view` is cells of a sheet spanning more than one cell, rather
    /// than a computed array.
    fn is_sheet_range(view: &crate::engine::range_view::RangeView<'_>) -> bool {
        view.sheet_name() != "__tmp" && !view.is_empty() && view.dims() != (1, 1)
    }

    /// An operand of a value operator. In a legacy formula's value context a
    /// range operand, or a function's range result, is implicitly intersected
    /// with the formula cell; arrays stay arrays.
    fn evaluate_arena_operand(
        &self,
        node_id: AstNodeId,
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        if !self.in_legacy_value_context() {
            return self.evaluate_arena_ast(node_id, data_store, sheet_registry);
        }
        if let Some(reference) = self.arena_range_reference(node_id, data_store, sheet_registry)? {
            return Ok(crate::traits::CalcValue::Scalar(
                self.implicit_intersection_from_reference(&reference),
            ));
        }
        self.evaluate_arena_ast(node_id, data_store, sheet_registry)
            .map(|value| self.legacy_operand_value(value))
    }

    /// The computed value of an operand of a value operator: in a legacy
    /// formula's value context a range it yields is implicitly intersected
    /// with the formula cell; arrays and other values stay as they are.
    fn legacy_operand_value(
        &self,
        value: crate::traits::CalcValue<'a>,
    ) -> crate::traits::CalcValue<'a> {
        match value {
            crate::traits::CalcValue::Range(view)
                if self.in_legacy_value_context() && Self::is_sheet_range(&view) =>
            {
                crate::traits::CalcValue::Scalar(
                    self.eval_implicit_intersection_calc(crate::traits::CalcValue::Range(view)),
                )
            }
            other => other,
        }
    }

    /* ===================  public  =================== */
    pub fn evaluate_ast(&self, node: &ASTNode) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        self.evaluate_ast_uncached(node)
    }

    pub(crate) fn evaluate_ast_with_offset(
        &self,
        node: &ASTNode,
        row_delta: i64,
        col_delta: i64,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        let offset = Self {
            context: self.context,
            current_sheet: self.current_sheet,
            current_cell: self.current_cell,
            local_env: self.local_env.clone(),
            reference_row_delta: row_delta,
            reference_col_delta: col_delta,
            disable_ast_planner: true,
            parameter_bindings: self.parameter_bindings,
            legacy: self.legacy,
        };
        offset.evaluate_ast_uncached(node)
    }

    pub(crate) fn reference_for_current_offset(
        &self,
        reference: &ReferenceType,
    ) -> Result<ReferenceType, ExcelError> {
        if let ReferenceType::NamedRange(name) = reference {
            match self.resolve_local_name(name) {
                // Already resolved (and offset) where it was bound.
                Some(LocalBinding::Reference(bound)) => return Ok(bound),
                Some(_) => {}
                None => {
                    if let Some(resolved) = self
                        .context
                        .resolve_name_reference(name, self.current_sheet)
                    {
                        return resolved;
                    }
                }
            }
        }
        self.effective_reference(reference)
            .map(|reference| reference.into_owned())
    }

    pub(crate) fn evaluate_arena_ast_with_offset(
        &self,
        node_id: AstNodeId,
        row_delta: i64,
        col_delta: i64,
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        let offset = Self {
            context: self.context,
            current_sheet: self.current_sheet,
            current_cell: self.current_cell,
            local_env: self.local_env.clone(),
            reference_row_delta: row_delta,
            reference_col_delta: col_delta,
            disable_ast_planner: true,
            parameter_bindings: self.parameter_bindings,
            legacy: self.legacy,
        };
        offset.evaluate_arena_ast(node_id, data_store, sheet_registry)
    }

    fn annotate_cell_value(
        &self,
        sheet: Option<&str>,
        row: u32,
        col: u32,
        value: LiteralValue,
    ) -> crate::traits::CalcValue<'a> {
        match self
            .context
            .resolve_cell_format(sheet, row, col, self.current_sheet)
        {
            Some(format) => crate::traits::CalcValue::AnnotatedScalar(value, format),
            None => crate::traits::CalcValue::Scalar(value),
        }
    }

    fn binary_format(
        &self,
        op: char,
        left: Option<crate::format::FormatId>,
        right: Option<crate::format::FormatId>,
    ) -> Option<crate::format::FormatId> {
        use formualizer_common::numfmt::FormatClass;
        let class =
            |id: Option<crate::format::FormatId>| id.and_then(|id| self.context.format_class(id));
        let left = class(left);
        let right = class(right);
        let is_plain = |class: &Option<FormatClass>| {
            matches!(
                class,
                None | Some(FormatClass::General | FormatClass::Number { .. })
            )
        };
        // This table is intentionally closed. LibreOffice measurement establishes
        // Date+Time and Date+Percent; unlisted pairs (including Date+Date,
        // Duration+Date, Date+Currency, DateTime+Time, and Date+Text) drop the
        // annotation rather than guessing a display class.
        match (op, left.as_ref(), right.as_ref()) {
            ('+', Some(FormatClass::Date), Some(FormatClass::Time))
            | ('+', Some(FormatClass::Time), Some(FormatClass::Date)) => {
                Some(crate::format::FormatId::DATETIME)
            }
            ('+', Some(FormatClass::Date), Some(FormatClass::Percent { .. }))
            | ('+', Some(FormatClass::Percent { .. }), Some(FormatClass::Date)) => {
                Some(crate::format::FormatId::DATE)
            }
            ('+' | '-', Some(FormatClass::Date), r) if is_plain(&r.cloned()) => {
                Some(crate::format::FormatId::DATE)
            }
            ('+', l, Some(FormatClass::Date)) if is_plain(&l.cloned()) => {
                Some(crate::format::FormatId::DATE)
            }
            ('+' | '-', Some(FormatClass::Time), r) if is_plain(&r.cloned()) => {
                Some(crate::format::FormatId::TIME)
            }
            ('+', l, Some(FormatClass::Time)) if is_plain(&l.cloned()) => {
                Some(crate::format::FormatId::TIME)
            }
            ('+' | '-', Some(FormatClass::DateTime), r) if is_plain(&r.cloned()) => {
                Some(crate::format::FormatId::DATETIME)
            }
            ('+', l, Some(FormatClass::DateTime)) if is_plain(&l.cloned()) => {
                Some(crate::format::FormatId::DATETIME)
            }
            ('+' | '-', Some(FormatClass::Duration), r) if is_plain(&r.cloned()) => {
                Some(crate::format::FormatId::DURATION)
            }
            ('+', l, Some(FormatClass::Duration)) if is_plain(&l.cloned()) => {
                Some(crate::format::FormatId::DURATION)
            }
            _ => None,
        }
    }

    fn annotate_numeric_result(
        &self,
        value: LiteralValue,
        format: Option<crate::format::FormatId>,
    ) -> crate::traits::CalcValue<'a> {
        match (value, format) {
            (value @ LiteralValue::Number(_), Some(format)) => {
                crate::traits::CalcValue::AnnotatedScalar(value, format)
            }
            (value, _) => crate::traits::CalcValue::Scalar(value),
        }
    }

    pub(crate) fn evaluate_arena_ast(
        &self,
        node_id: AstNodeId,
        data_store: &DataStore,
        sheet_registry: &SheetRegistry,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        let node = data_store.get_node(node_id).ok_or_else(|| {
            ExcelError::new(ExcelErrorKind::Value).with_message("Missing AST node")
        })?;

        match node {
            AstNodeData::Literal(vref) => {
                if let Some(bindings) = self.parameter_bindings
                    && let Some(slot_id) = bindings.literal_slots_by_node.get(&node_id)
                    && let Some(value) = bindings.literal_values.get(slot_id.0 as usize)
                {
                    return Ok(crate::traits::CalcValue::Scalar(value.clone()));
                }
                Ok(crate::traits::CalcValue::Scalar(
                    data_store.retrieve_value(*vref),
                ))
            }
            AstNodeData::Omitted => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0))),
            AstNodeData::Reference { ref_type, .. } => {
                if self.local_env.is_empty()
                    && let CompactRefType::Cell {
                        sheet,
                        row,
                        col,
                        row_abs,
                        col_abs,
                    } = ref_type
                    && *row > 0
                    && *col > 0
                {
                    let sheet_name = match sheet {
                        Some(SheetKey::Id(id)) => Some(sheet_registry.name(*id)),
                        Some(SheetKey::Name(name_id)) => {
                            Some(data_store.resolve_ast_string(*name_id))
                        }
                        None => None,
                    };
                    let row = shift_axis_for_offset(*row, self.reference_row_delta, *row_abs)?;
                    let col = shift_axis_for_offset(*col, self.reference_col_delta, *col_abs)?;
                    let value = self.context.resolve_cell_reference_value(
                        sheet_name,
                        row,
                        col,
                        self.current_sheet,
                    )?;
                    Ok(self.annotate_cell_value(sheet_name, row, col, value))
                } else {
                    let reference =
                        data_store.reconstruct_reference_type_for_eval(ref_type, sheet_registry);
                    let reference = self.effective_reference(&reference)?;
                    if let Some(local) = self.resolve_local_reference(&reference) {
                        return local;
                    }
                    self.eval_reference_to_calc(&reference)
                }
            }
            AstNodeData::UnaryOp { op_id, expr_id } => {
                let op = data_store.resolve_ast_string(*op_id);
                let expr = if op == "@" {
                    self.evaluate_arena_ast(*expr_id, data_store, sheet_registry)?
                } else {
                    self.evaluate_arena_operand(*expr_id, data_store, sheet_registry)?
                };

                if op == "@" {
                    // Prefer reference-aware implicit intersection so we don't depend on
                    // RangeView absolute coordinates (important for lightweight test contexts).
                    if let Some(AstNodeData::Reference { ref_type, .. }) =
                        data_store.get_node(*expr_id)
                    {
                        let reference = data_store
                            .reconstruct_reference_type_for_eval(ref_type, sheet_registry);
                        let v = self.implicit_intersection_from_reference(&reference);
                        return Ok(crate::traits::CalcValue::Scalar(v));
                    }

                    let v = self.eval_implicit_intersection_calc(expr);
                    return Ok(crate::traits::CalcValue::Scalar(v));
                }
                // For now, materialize for operators. Future: virtual range ops.
                let v = expr.into_literal();
                match v {
                    LiteralValue::Array(arr) => self
                        .map_array(arr, |cell| self.eval_unary_scalar(op, cell))
                        .map(crate::traits::CalcValue::Scalar),
                    other => self
                        .eval_unary_scalar(op, other)
                        .map(crate::traits::CalcValue::Scalar),
                }
            }
            AstNodeData::BinaryOp {
                op_id,
                left_id,
                right_id,
            } => {
                let op = data_store.resolve_ast_string(*op_id);
                if op == " " {
                    let intersection = self
                        .evaluate_arena_ast_as_reference(*left_id, data_store, sheet_registry)
                        .and_then(|lref| {
                            let rref = self.evaluate_arena_ast_as_reference(
                                *right_id,
                                data_store,
                                sheet_registry,
                            )?;
                            self.intersect_reference_areas(lref, rref)
                        });
                    return self.intersection_value(intersection);
                }
                if op == ":" {
                    let range = self
                        .evaluate_arena_ast_as_reference(*left_id, data_store, sheet_registry)
                        .and_then(|lref| {
                            let rref = self.evaluate_arena_ast_as_reference(
                                *right_id,
                                data_store,
                                sheet_registry,
                            )?;
                            self.combine_reference_areas(lref, rref)
                        });
                    return self.range_value(range);
                }

                // `&` reads its operands as text, where an empty slot that IF
                // selects is "" rather than IF's 0.
                let operand = |id: AstNodeId| {
                    if op == "&"
                        && let Some(value) =
                            ArgumentHandle::new_arena(id, self, data_store, sheet_registry)
                                .if_branch_value_for_text()
                    {
                        return value.map(|value| self.legacy_operand_value(value));
                    }
                    self.evaluate_arena_operand(id, data_store, sheet_registry)
                };
                let left_calc = operand(*left_id)?;
                let left_format = left_calc.format_id();
                let left = left_calc.into_literal();
                let right_calc = operand(*right_id)?;
                let right_format = right_calc.format_id();
                let right = right_calc.into_literal();

                if matches!(op, "=" | "<>" | ">" | "<" | ">=" | "<=") {
                    return self
                        .compare(op, left, right)
                        .map(crate::traits::CalcValue::Scalar);
                }

                match op {
                    "+" => self.numeric_binary(left, right, |a, b| a + b).map(|value| {
                        self.annotate_numeric_result(
                            value,
                            self.binary_format('+', left_format, right_format),
                        )
                    }),
                    "-" => self.numeric_binary(left, right, |a, b| a - b).map(|value| {
                        self.annotate_numeric_result(
                            value,
                            self.binary_format('-', left_format, right_format),
                        )
                    }),
                    "*" => self
                        .numeric_binary(left, right, |a, b| a * b)
                        .map(crate::traits::CalcValue::Scalar),
                    "/" => self
                        .divide(left, right)
                        .map(crate::traits::CalcValue::Scalar),
                    "^" => self
                        .power(left, right)
                        .map(crate::traits::CalcValue::Scalar),
                    "&" => self
                        .concat(left, right)
                        .map(crate::traits::CalcValue::Scalar),
                    _ => Err(ExcelError::new(ExcelErrorKind::NImpl)
                        .with_message(format!("Binary op '{op}'"))),
                }
            }
            AstNodeData::Array { .. } => {
                let (rows, cols, elements) =
                    data_store.get_array_elems(node_id).ok_or_else(|| {
                        ExcelError::new(ExcelErrorKind::Value).with_message("Invalid array")
                    })?;

                let rows_usize = rows as usize;
                let cols_usize = cols as usize;
                let mut out: Vec<Vec<LiteralValue>> = Vec::with_capacity(rows_usize);
                for r in 0..rows_usize {
                    let mut row = Vec::with_capacity(cols_usize);
                    for c in 0..cols_usize {
                        let idx = r * cols_usize + c;
                        if let Some(&elem_id) = elements.get(idx) {
                            row.push(
                                self.evaluate_arena_ast(elem_id, data_store, sheet_registry)?
                                    .into_literal(),
                            );
                        }
                    }
                    out.push(row);
                }

                Ok(crate::traits::CalcValue::Range(
                    crate::engine::range_view::RangeView::from_owned_rows(
                        out,
                        self.context.date_system(),
                    ),
                ))
            }
            AstNodeData::Function { name_id, .. } => {
                let name = data_store.resolve_ast_string(*name_id);
                let args = data_store.get_args(node_id).ok_or_else(|| {
                    ExcelError::new(ExcelErrorKind::Value).with_message("Missing function args")
                })?;

                if name == crate::engine::arena::CALL_EXPRESSION_NAME
                    && let Some((callee_id, call_args)) = args.split_first()
                {
                    let callee = self.evaluate_arena_ast(*callee_id, data_store, sheet_registry)?;
                    let mut bindings = Vec::with_capacity(call_args.len());
                    for arg_id in call_args {
                        bindings.push(self.call_argument(&ArgumentHandle::new_arena(
                            *arg_id,
                            self,
                            data_store,
                            sheet_registry,
                        ))?);
                    }
                    return self.invoke_call_bindings(callee, bindings);
                }

                if let Some(fun) = self.context.get_function("", name) {
                    let fctx = DefaultFunctionContext::new_with_sheet(
                        self.context,
                        self.current_cell,
                        self.current_sheet,
                    );
                    let _call_dates = self.enter_function_call();

                    return error_as_value(self.with_arena_call_handles(
                        fun.as_ref(),
                        args,
                        data_store,
                        sheet_registry,
                        |handles| fun.dispatch(handles, &fctx),
                    ));
                }

                if let Some(callable) = self.resolve_local_callable(name) {
                    let mut bindings = Vec::with_capacity(args.len());
                    for arg_id in args {
                        bindings.push(self.call_argument(&ArgumentHandle::new_arena(
                            *arg_id,
                            self,
                            data_store,
                            sheet_registry,
                        ))?);
                    }
                    return callable.invoke_bindings(self, bindings);
                }

                // An unknown function is a #NAME? value at the call site, as on the
                // tree path, so IFERROR, ISERROR or a criteria argument sees it like
                // any other error instead of the whole formula aborting.
                Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new(ExcelErrorKind::Name)
                        .with_message(format!("Unknown function: {name}")),
                )))
            }
        }
    }

    fn evaluate_ast_uncached(
        &self,
        node: &ASTNode,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        // Tree formulas (LAMBDA bodies, names) are not cell formulas: they keep
        // array evaluation even when invoked from a legacy formula.
        if self.legacy.is_some() {
            return self.with_legacy_context(None).evaluate_ast_uncached(node);
        }
        if self.disable_ast_planner {
            return self.eval_tree_uncached(node);
        }

        // Plan-aware evaluation: build a plan for this node and execute accordingly.
        // Provide the planner with a lightweight range-dimension probe and function lookup
        // so it can select chunked reduction and arg-parallel strategies where appropriate.
        let current_sheet = self.current_sheet.to_string();
        let range_probe = |reference: &ReferenceType| {
            probe_range_dimensions(self.context, &current_sheet, reference)
        };
        let fn_lookup = |ns: &str, name: &str| self.context.get_function(ns, name);

        let mut planner = crate::planner::Planner::new(crate::planner::PlanConfig::default())
            .with_range_probe(&range_probe)
            .with_function_lookup(&fn_lookup);
        let plan = planner.plan(node);
        self.eval_with_plan(node, &plan.root)
    }

    fn eval_tree_uncached(
        &self,
        node: &ASTNode,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        match &node.node_type {
            ASTNodeType::Literal(v) => Ok(crate::traits::CalcValue::Scalar(v.clone())),
            ASTNodeType::Omitted => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0))),
            ASTNodeType::Reference { reference, .. } => self.eval_ast_reference_to_calc(reference),
            ASTNodeType::UnaryOp { op, expr } => self
                .eval_unary(op, expr)
                .map(crate::traits::CalcValue::Scalar),
            ASTNodeType::BinaryOp { op, left, right } => self.eval_binary(op, left, right),
            ASTNodeType::Function { name, args } => self.eval_function_to_calc(name, args),
            ASTNodeType::Call { callee, args } => self.eval_call_to_calc(callee, args),
            ASTNodeType::Array(rows) => self.eval_array_literal_to_calc(rows),
        }
    }

    fn eval_with_plan(
        &self,
        node: &ASTNode,
        plan_node: &crate::planner::PlanNode,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        match &node.node_type {
            ASTNodeType::Literal(v) => Ok(crate::traits::CalcValue::Scalar(v.clone())),
            ASTNodeType::Omitted => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Number(0.0))),
            ASTNodeType::Reference { reference, .. } => self.eval_ast_reference_to_calc(reference),
            ASTNodeType::UnaryOp { op, expr } => {
                // For now, reuse existing unary implementation (which recurses).
                // In a later phase, we can map plan_node.children[0].
                self.eval_unary(op, expr)
                    .map(crate::traits::CalcValue::Scalar)
            }
            ASTNodeType::BinaryOp { op, left, right } => self.eval_binary(op, left, right),
            ASTNodeType::Function { name, args } => {
                let strategy = plan_node.strategy;
                if let Some(fun) = self.context.get_function("", name) {
                    use crate::function::FnCaps;
                    use crate::planner::ExecStrategy;
                    let caps = fun.caps();

                    // Short-circuit or volatile: always sequential
                    if caps.contains(FnCaps::SHORT_CIRCUIT) || caps.contains(FnCaps::VOLATILE) {
                        return self.eval_function_to_calc(name, args);
                    }

                    // Windowed/chunked strategies are handled by the unified `eval()` path.

                    // Arg-parallel: prewarm subexpressions and then dispatch
                    if matches!(strategy, ExecStrategy::ArgParallel)
                        && caps.contains(FnCaps::PARALLEL_ARGS)
                    {
                        // Sequential prewarm of subexpressions (safe without Sync bounds)
                        for arg in args {
                            match &arg.node_type {
                                ASTNodeType::Reference { reference, .. } => {
                                    if let Ok(reference) = self.effective_reference(reference) {
                                        let _ = self
                                            .context
                                            .resolve_range_view(&reference, self.current_sheet);
                                    }
                                }
                                _ => {
                                    let _ = self.evaluate_ast(arg);
                                }
                            }
                        }
                        return self.eval_function_to_calc(name, args);
                    }

                    // Default path
                    return self.eval_function_to_calc(name, args);
                }
                self.eval_function_to_calc(name, args)
            }
            ASTNodeType::Call { callee, args } => self.eval_call_to_calc(callee, args),
            ASTNodeType::Array(rows) => self.eval_array_literal_to_calc(rows),
        }
    }

    /* ===================  reference  =================== */
    fn eval_ast_reference_to_calc(
        &self,
        reference: &ReferenceType,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        if !self.local_env.is_empty() {
            let reference = self.effective_reference(reference)?;
            if let Some(local) = self.resolve_local_reference(&reference) {
                return local;
            }
            return self.eval_reference_to_calc(&reference);
        }

        if let ReferenceType::Cell {
            sheet,
            row,
            col,
            row_abs,
            col_abs,
        } = reference
        {
            let row = shift_axis_for_offset(*row, self.reference_row_delta, *row_abs)?;
            let col = shift_axis_for_offset(*col, self.reference_col_delta, *col_abs)?;
            let value = self.context.resolve_cell_reference_value(
                sheet.as_deref(),
                row,
                col,
                self.current_sheet,
            )?;
            return Ok(self.annotate_cell_value(sheet.as_deref(), row, col, value));
        }

        let reference = self.effective_reference(reference)?;
        self.eval_reference_to_calc(&reference)
    }

    fn eval_reference_to_calc(
        &self,
        reference: &ReferenceType,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        if let ReferenceType::Cell {
            sheet, row, col, ..
        } = reference
        {
            let value = self.context.resolve_cell_reference_value(
                sheet.as_deref(),
                *row,
                *col,
                self.current_sheet,
            )?;
            return Ok(self.annotate_cell_value(sheet.as_deref(), *row, *col, value));
        }

        let view = match self
            .context
            .resolve_range_view(reference, self.current_sheet)
        {
            Ok(view) => view,
            // An undefined name is a #NAME? value that ISERROR, IFERROR,
            // ERROR.TYPE and TYPE can inspect, not a failed evaluation.
            Err(error)
                if error.kind == ExcelErrorKind::Name
                    && matches!(reference, ReferenceType::NamedRange(_)) =>
            {
                return Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error)));
            }
            Err(error) => return Err(error),
        }
        .with_cancel_token(self.context.cancellation_token());
        Ok(crate::traits::CalcValue::Range(view))
    }

    /// The value of a space-operator intersection: the shared cells, or
    /// `#NULL!` when the references do not overlap.
    fn intersection_value(
        &self,
        intersection: Result<Option<ReferenceType>, ExcelError>,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        match intersection {
            Ok(Some(reference)) => self.eval_reference_to_calc(&reference),
            Ok(None) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Null),
            ))),
            Err(error) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error))),
        }
    }

    /// The value of a `:` range: its cells, read exactly like a literal range
    /// of the same area, or the error that kept the range from forming or
    /// being read (an error value, so ISERROR and IFERROR see it).
    fn range_value(
        &self,
        range: Result<ReferenceType, ExcelError>,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        match range.and_then(|reference| self.eval_reference_to_calc(&reference)) {
            Err(error) if error.kind == ExcelErrorKind::Cancelled => Err(error),
            Err(error) => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(error))),
            ok => ok,
        }
    }

    fn eval_reference(&self, reference: &ReferenceType) -> Result<LiteralValue, ExcelError> {
        self.eval_reference_to_calc(reference)
            .map(|cv| cv.into_literal())
    }

    /* ===================  unary ops  =================== */
    fn eval_unary(&self, op: &str, expr: &ASTNode) -> Result<LiteralValue, ExcelError> {
        if op == "@" {
            if let ASTNodeType::Reference { reference, .. } = &expr.node_type {
                let reference = self.effective_reference(reference)?;
                return Ok(self.implicit_intersection_from_reference(&reference));
            }

            let cv = self.evaluate_ast(expr)?;
            return Ok(self.eval_implicit_intersection_calc(cv));
        }

        let v = self.evaluate_ast(expr)?.into_literal();
        match v {
            LiteralValue::Array(arr) => {
                self.map_array(arr, |cell| self.eval_unary_scalar(op, cell))
            }
            other => self.eval_unary_scalar(op, other),
        }
    }

    fn eval_unary_scalar(&self, op: &str, v: LiteralValue) -> Result<LiteralValue, ExcelError> {
        match op {
            // Excel/LibreOffice treat unary `+` as a pass-through (identity) operator,
            // not as a numeric coercion. `=+"2014F"` returns the text "2014F"; only the
            // unary `-` form coerces operands to numbers. The `=+A1` idiom is common in
            // finance models (Lotus 1-2-3 carry-over) and must preserve text labels.
            "+" => Ok(v),
            "-" => self.apply_number_unary(v, |n| -n),
            "%" => self.apply_number_unary(v, |n| n / 100.0),
            _ => {
                Err(ExcelError::new(ExcelErrorKind::NImpl).with_message(format!("Unary op '{op}'")))
            }
        }
    }

    pub(crate) fn eval_implicit_intersection_calc(
        &self,
        cv: crate::traits::CalcValue<'a>,
    ) -> LiteralValue {
        let (cur_r0, cur_c0) = match self.current_cell {
            Some(cell) => (cell.coord.row() as usize, cell.coord.col() as usize),
            None => (0usize, 0usize),
        };

        match cv {
            crate::traits::CalcValue::Scalar(v)
            | crate::traits::CalcValue::AnnotatedScalar(v, _) => match v {
                LiteralValue::Array(arr) => {
                    if arr.is_empty() || arr.first().map(|r| r.is_empty()).unwrap_or(true) {
                        return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                    }
                    arr[0][0].clone()
                }
                other => other,
            },
            crate::traits::CalcValue::Range(rv) => {
                if rv.is_empty() {
                    return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                }

                // Array results (array literals and many dynamic-array functions) are materialized
                // into an owned RangeView with a temporary backing sheet ("__tmp").
                // For explicit @, interpret these as anchored at the formula cell and select the
                // top-left element.
                if rv.sheet_name() == "__tmp" {
                    return rv.get_cell(0, 0);
                }

                if let Some(v) = rv.as_1x1() {
                    return v;
                }

                let (rows, cols) = rv.dims();
                let sr = rv.start_row();
                let sc = rv.start_col();
                let er = rv.end_row();
                let ec = rv.end_col();

                // Excel-compatible implicit intersection (simplified):
                // - Nx1: pick by row
                // - 1xM: pick by column
                // - NxM: pick by (row,col)
                if cols == 1 {
                    if cur_r0 < sr || cur_r0 > er {
                        return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                    }
                    let rel_r = cur_r0 - sr;
                    return rv.get_cell(rel_r, 0);
                }

                if rows == 1 {
                    if cur_c0 < sc || cur_c0 > ec {
                        return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                    }
                    let rel_c = cur_c0 - sc;
                    return rv.get_cell(0, rel_c);
                }

                if cur_r0 < sr || cur_r0 > er || cur_c0 < sc || cur_c0 > ec {
                    return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                }
                let rel_r = cur_r0 - sr;
                let rel_c = cur_c0 - sc;
                rv.get_cell(rel_r, rel_c)
            }
            crate::traits::CalcValue::Callable(_) => LiteralValue::Error(
                ExcelError::new(ExcelErrorKind::Calc).with_message("LAMBDA value must be invoked"),
            ),
        }
    }

    fn implicit_intersection_from_reference(&self, reference: &ReferenceType) -> LiteralValue {
        let (cur_r1, cur_c1) = match self.current_cell {
            Some(cell) => (
                cell.coord.row().saturating_add(1),
                cell.coord.col().saturating_add(1),
            ),
            None => (1u32, 1u32),
        };

        match reference {
            ReferenceType::Cell {
                sheet, row, col, ..
            } => {
                let sheet_name = sheet.as_deref().unwrap_or(self.current_sheet);
                // A blank cell stays blank, as through a range (`@A4&"x"` is "x").
                match self.context.resolve_cell_reference_value(
                    Some(sheet_name),
                    *row,
                    *col,
                    self.current_sheet,
                ) {
                    Ok(v) => v,
                    Err(e) => LiteralValue::Error(e),
                }
            }
            ReferenceType::Range {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
                ..
            } => {
                let sheet_name = sheet.as_deref().unwrap_or(self.current_sheet);

                // A whole column or row (A:A, 3:3, A5:A) spans the sheet on its open
                // axis, so it intersects every row or column of the sheet.
                let (sr, sc, er, ec) = (
                    start_row.unwrap_or(1),
                    start_col.unwrap_or(1),
                    end_row.unwrap_or(1_048_576),
                    end_col.unwrap_or(16_384),
                );

                // Normalize bounds (A10:A1 is legal syntax; treat as swapped).
                let (mut sr, mut er) = (sr, er);
                let (mut sc, mut ec) = (sc, ec);
                if sr > er {
                    std::mem::swap(&mut sr, &mut er);
                }
                if sc > ec {
                    std::mem::swap(&mut sc, &mut ec);
                }

                let pick = if sr == er && sc == ec {
                    // A single cell needs no intersection.
                    (sr, sc)
                } else if sc == ec {
                    // Column vector: intersect by row
                    if cur_r1 < sr || cur_r1 > er {
                        return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                    }
                    (cur_r1, sc)
                } else if sr == er {
                    // Row vector: intersect by column
                    if cur_c1 < sc || cur_c1 > ec {
                        return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                    }
                    (sr, cur_c1)
                } else {
                    // 2D: require both axes
                    if cur_r1 < sr || cur_r1 > er || cur_c1 < sc || cur_c1 > ec {
                        return LiteralValue::Error(ExcelError::new(ExcelErrorKind::Value));
                    }
                    (cur_r1, cur_c1)
                };

                // A blank cell stays blank (not 0) for the consumer to coerce.
                match self.context.resolve_cell_reference_value(
                    Some(sheet_name),
                    pick.0,
                    pick.1,
                    self.current_sheet,
                ) {
                    Ok(v) => v,
                    Err(e) => LiteralValue::Error(e),
                }
            }
            // Named ranges / tables / external: fall back to materializing and intersecting.
            other => {
                let cv = match self.eval_reference_to_calc(other) {
                    Ok(cv) => cv,
                    Err(e) => return LiteralValue::Error(e),
                };
                self.eval_implicit_intersection_calc(cv)
            }
        }
    }

    fn apply_number_unary<F>(&self, v: LiteralValue, f: F) -> Result<LiteralValue, ExcelError>
    where
        F: Fn(f64) -> f64,
    {
        match crate::coercion::to_arithmetic_number_with_locale(
            &v,
            &self.context.locale(),
            self.context.date_system(),
            self.current_year(),
        ) {
            Ok(n) => match crate::coercion::sanitize_numeric(f(n)) {
                Ok(n2) => Ok(LiteralValue::Number(n2)),
                Err(e) => Ok(LiteralValue::Error(e)),
            },
            Err(e) => Ok(LiteralValue::Error(e)),
        }
    }

    /// Year for date text that omits it (`Jan 3`): Excel uses its clock's year.
    fn current_year(&self) -> Option<i32> {
        use chrono::Datelike;
        Some(self.context.clock().today().year())
    }

    /// Enter the date context of a builtin call made by this interpreter:
    /// until the guard drops, date text in the call's number arguments reads
    /// in this workbook's date system and the clock's year
    /// ([`crate::coercion::to_number_argument`]). Every builtin call the
    /// interpreter makes (value, reference or lifted) enters it here, before
    /// the function runs, so a function that overrides `dispatch` reads date
    /// text in the same context as any other.
    pub(crate) fn enter_function_call(&self) -> crate::coercion::ArgumentDateContextGuard {
        crate::coercion::enter_argument_date_context(
            self.context.date_system(),
            self.current_year(),
        )
    }

    /* ===================  binary ops  =================== */
    fn eval_binary(
        &self,
        op: &str,
        left_node: &ASTNode,
        right_node: &ASTNode,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        if op == " " {
            let intersection = self.evaluate_ast_as_reference(left_node).and_then(|lref| {
                let rref = self.evaluate_ast_as_reference(right_node)?;
                self.intersect_reference_areas(lref, rref)
            });
            return self.intersection_value(intersection);
        }
        if op == ":" {
            let range = self.evaluate_ast_as_reference(left_node).and_then(|lref| {
                let rref = self.evaluate_ast_as_reference(right_node)?;
                self.combine_reference_areas(lref, rref)
            });
            return self.range_value(range);
        }
        // `&` reads its operands as text, where an empty slot that IF selects
        // is "" rather than IF's 0.
        let operand = |node: &ASTNode| {
            if op == "&"
                && let Some(value) = ArgumentHandle::new(node, self).if_branch_value_for_text()
            {
                return value;
            }
            self.evaluate_ast(node)
        };
        let left_calc = operand(left_node)?;
        let left_format = left_calc.format_id();
        let left = left_calc.into_literal();
        let right_calc = operand(right_node)?;
        let right_format = right_calc.format_id();
        let right = right_calc.into_literal();
        if matches!(op, "=" | "<>" | ">" | "<" | ">=" | "<=") {
            return self
                .compare(op, left, right)
                .map(crate::traits::CalcValue::Scalar);
        }
        match op {
            "+" => self.numeric_binary(left, right, |a, b| a + b).map(|value| {
                self.annotate_numeric_result(
                    value,
                    self.binary_format('+', left_format, right_format),
                )
            }),
            "-" => self.numeric_binary(left, right, |a, b| a - b).map(|value| {
                self.annotate_numeric_result(
                    value,
                    self.binary_format('-', left_format, right_format),
                )
            }),
            "*" => self
                .numeric_binary(left, right, |a, b| a * b)
                .map(crate::traits::CalcValue::Scalar),
            "/" => self
                .divide(left, right)
                .map(crate::traits::CalcValue::Scalar),
            "^" => self
                .power(left, right)
                .map(crate::traits::CalcValue::Scalar),
            "&" => self
                .concat(left, right)
                .map(crate::traits::CalcValue::Scalar),
            _ => {
                Err(ExcelError::new(ExcelErrorKind::NImpl)
                    .with_message(format!("Binary op '{op}'")))
            }
        }
    }

    /* ===================  function calls  =================== */
    /// Postfix call such as `LAMBDA(x,x+1)(5)`: evaluate the callee, then the
    /// arguments, then invoke.
    fn eval_call_to_calc(
        &self,
        callee: &ASTNode,
        args: &[ASTNode],
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        let callee = self.evaluate_ast(callee)?;
        let mut bindings = Vec::with_capacity(args.len());
        for arg in args {
            bindings.push(self.call_argument(&ArgumentHandle::new(arg, self))?);
        }
        self.invoke_call_bindings(callee, bindings)
    }

    /// What a LAMBDA parameter receives for a call argument: a reference stays
    /// a reference (`LAMBDA(r,ROWS(r))(A:A)` is 1048576), anything else passes
    /// its value.
    fn call_argument(&self, arg: &ArgumentHandle<'_, 'a>) -> Result<LocalBinding, ExcelError> {
        Ok(match arg.bindable_reference()? {
            Some(reference) => LocalBinding::Reference(reference),
            None => LocalBinding::Value(arg.value()?.into_literal()),
        })
    }

    /// Invokes an evaluated callee. An error callee propagates; any other
    /// non-callable value cannot be called and yields `#VALUE!`.
    fn invoke_call_bindings(
        &self,
        callee: crate::traits::CalcValue<'a>,
        args: Vec<LocalBinding>,
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        match callee {
            crate::traits::CalcValue::Callable(callable) => callable.invoke_bindings(self, args),
            other => match other.into_literal() {
                error @ LiteralValue::Error(_) => Ok(crate::traits::CalcValue::Scalar(error)),
                _ => Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
                    ExcelError::new(ExcelErrorKind::Value)
                        .with_message("Only a LAMBDA value can be called"),
                ))),
            },
        }
    }

    fn eval_function_to_calc(
        &self,
        name: &str,
        args: &[ASTNode],
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        if let Some(fun) = self.context.get_function("", name) {
            let handles: Vec<ArgumentHandle> =
                args.iter().map(|n| ArgumentHandle::new(n, self)).collect();
            // Use the function's built-in dispatch method with a narrow FunctionContext
            let fctx = DefaultFunctionContext::new_with_sheet(
                self.context,
                self.current_cell,
                self.current_sheet,
            );
            let _call_dates = self.enter_function_call();
            return error_as_value(fun.dispatch(&handles, &fctx));
        }

        if let Some(callable) = self.resolve_local_callable(name) {
            let mut bindings = Vec::with_capacity(args.len());
            for arg in args {
                bindings.push(self.call_argument(&ArgumentHandle::new(arg, self))?);
            }
            return callable.invoke_bindings(self, bindings);
        }

        // Include the function name in the error message for better debugging
        Ok(crate::traits::CalcValue::Scalar(LiteralValue::Error(
            ExcelError::new(ExcelErrorKind::Name).with_message(format!("Unknown function: {name}")),
        )))
    }

    fn eval_function(&self, name: &str, args: &[ASTNode]) -> Result<LiteralValue, ExcelError> {
        self.eval_function_to_calc(name, args)
            .map(|cv| cv.into_literal())
    }

    pub fn function_context(&self, cell_ref: Option<&CellRef>) -> DefaultFunctionContext<'_> {
        DefaultFunctionContext::new_with_sheet(self.context, cell_ref.cloned(), self.current_sheet)
    }

    /* ===================  array literal  =================== */
    fn eval_array_literal_to_calc(
        &self,
        rows: &[Vec<ASTNode>],
    ) -> Result<crate::traits::CalcValue<'a>, ExcelError> {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mut r = Vec::with_capacity(row.len());
            for cell in row {
                r.push(self.evaluate_ast(cell)?.into_literal());
            }
            out.push(r);
        }
        Ok(crate::traits::CalcValue::Range(
            crate::engine::range_view::RangeView::from_owned_rows(out, self.context.date_system()),
        ))
    }

    fn eval_array_literal(&self, rows: &[Vec<ASTNode>]) -> Result<LiteralValue, ExcelError> {
        self.eval_array_literal_to_calc(rows)
            .map(|cv| cv.into_literal())
    }

    fn numeric_binary<F>(
        &self,
        left: LiteralValue,
        right: LiteralValue,
        f: F,
    ) -> Result<LiteralValue, ExcelError>
    where
        F: Fn(f64, f64) -> f64 + Copy,
    {
        self.broadcast_apply(left, right, |l, r| {
            let a = crate::coercion::to_arithmetic_number_with_locale(
                &l,
                &self.context.locale(),
                self.context.date_system(),
                self.current_year(),
            );
            let b = crate::coercion::to_arithmetic_number_with_locale(
                &r,
                &self.context.locale(),
                self.context.date_system(),
                self.current_year(),
            );
            match (a, b) {
                (Ok(a), Ok(b)) => match crate::coercion::sanitize_numeric(f(a, b)) {
                    Ok(n2) => Ok(LiteralValue::Number(n2)),
                    Err(e) => Ok(LiteralValue::Error(e)),
                },
                (Err(e), _) | (_, Err(e)) => Ok(LiteralValue::Error(e)),
            }
        })
    }

    fn divide(&self, left: LiteralValue, right: LiteralValue) -> Result<LiteralValue, ExcelError> {
        self.broadcast_apply(left, right, |l, r| {
            let ln = crate::coercion::to_arithmetic_number_with_locale(
                &l,
                &self.context.locale(),
                self.context.date_system(),
                self.current_year(),
            );
            let rn = crate::coercion::to_arithmetic_number_with_locale(
                &r,
                &self.context.locale(),
                self.context.date_system(),
                self.current_year(),
            );
            let (a, b) = match (ln, rn) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => return Ok(LiteralValue::Error(e)),
            };
            if b == 0.0 {
                return Ok(LiteralValue::Error(ExcelError::from_error_string(
                    "#DIV/0!",
                )));
            }
            match crate::coercion::sanitize_numeric(a / b) {
                Ok(n) => Ok(LiteralValue::Number(n)),
                Err(e) => Ok(LiteralValue::Error(e)),
            }
        })
    }

    fn power(&self, left: LiteralValue, right: LiteralValue) -> Result<LiteralValue, ExcelError> {
        self.broadcast_apply(left, right, |l, r| {
            let ln = crate::coercion::to_arithmetic_number_with_locale(
                &l,
                &self.context.locale(),
                self.context.date_system(),
                self.current_year(),
            );
            let rn = crate::coercion::to_arithmetic_number_with_locale(
                &r,
                &self.context.locale(),
                self.context.date_system(),
                self.current_year(),
            );
            let (a, b) = match (ln, rn) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => return Ok(LiteralValue::Error(e)),
            };
            match crate::coercion::excel_power(a, b) {
                Ok(n) => Ok(LiteralValue::Number(n)),
                Err(e) => Ok(LiteralValue::Error(e)),
            }
        })
    }

    fn map_array<F>(&self, arr: Vec<Vec<LiteralValue>>, f: F) -> Result<LiteralValue, ExcelError>
    where
        F: Fn(LiteralValue) -> Result<LiteralValue, ExcelError> + Copy,
    {
        let mut out = Vec::with_capacity(arr.len());
        for row in arr {
            let mut new_row = Vec::with_capacity(row.len());
            for cell in row {
                new_row.push(match f(cell) {
                    Ok(v) => v,
                    Err(e) => LiteralValue::Error(e),
                });
            }
            out.push(new_row);
        }
        Ok(LiteralValue::Array(out))
    }

    fn combine_arrays<F>(
        &self,
        l: Vec<Vec<LiteralValue>>,
        r: Vec<Vec<LiteralValue>>,
        f: F,
    ) -> Result<LiteralValue, ExcelError>
    where
        F: Fn(LiteralValue, LiteralValue) -> Result<LiteralValue, ExcelError> + Copy,
    {
        // Excel's array expansion: the result takes the larger size in each
        // dimension; a single row or column repeats, and positions past the
        // end of a longer-but-not-single dimension are #N/A.
        let (rows, cols) = crate::lift::broadcast_dims([&l, &r]);
        let mut out = Vec::with_capacity(rows);
        for i in 0..rows {
            let mut row = Vec::with_capacity(cols);
            for j in 0..cols {
                let lv = crate::lift::broadcast_get(&l, i, j);
                let rv = crate::lift::broadcast_get(&r, i, j);
                row.push(match f(lv, rv) {
                    Ok(v) => v,
                    Err(e) => LiteralValue::Error(e),
                });
            }
            out.push(row);
        }
        Ok(LiteralValue::Array(out))
    }

    fn broadcast_apply<F>(
        &self,
        left: LiteralValue,
        right: LiteralValue,
        f: F,
    ) -> Result<LiteralValue, ExcelError>
    where
        F: Fn(LiteralValue, LiteralValue) -> Result<LiteralValue, ExcelError> + Copy,
    {
        use LiteralValue::*;
        match (left, right) {
            (Array(l), Array(r)) => self.combine_arrays(l, r, f),
            (Array(arr), v) => {
                let shape_l = (arr.len(), arr.first().map(|r| r.len()).unwrap_or(0));
                let shape_r = (1usize, 1usize);
                let target = match broadcast_shape(&[shape_l, shape_r]) {
                    Ok(s) => s,
                    Err(e) => return Ok(LiteralValue::Error(e)),
                };
                let mut out = Vec::with_capacity(target.0);
                for i in 0..target.0 {
                    let mut row = Vec::with_capacity(target.1);
                    for j in 0..target.1 {
                        let (li, lj) = project_index((i, j), shape_l);
                        let lv = arr
                            .get(li)
                            .and_then(|r| r.get(lj))
                            .cloned()
                            .unwrap_or(LiteralValue::Empty);
                        row.push(match f(lv, v.clone()) {
                            Ok(vv) => vv,
                            Err(e) => LiteralValue::Error(e),
                        });
                    }
                    out.push(row);
                }
                Ok(LiteralValue::Array(out))
            }
            (v, Array(arr)) => {
                let shape_l = (1usize, 1usize);
                let shape_r = (arr.len(), arr.first().map(|r| r.len()).unwrap_or(0));
                let target = match broadcast_shape(&[shape_l, shape_r]) {
                    Ok(s) => s,
                    Err(e) => return Ok(LiteralValue::Error(e)),
                };
                let mut out = Vec::with_capacity(target.0);
                for i in 0..target.0 {
                    let mut row = Vec::with_capacity(target.1);
                    for j in 0..target.1 {
                        let (ri, rj) = project_index((i, j), shape_r);
                        let rv = arr
                            .get(ri)
                            .and_then(|r| r.get(rj))
                            .cloned()
                            .unwrap_or(LiteralValue::Empty);
                        row.push(match f(v.clone(), rv) {
                            Ok(vv) => vv,
                            Err(e) => LiteralValue::Error(e),
                        });
                    }
                    out.push(row);
                }
                Ok(LiteralValue::Array(out))
            }
            (l, r) => f(l, r),
        }
    }

    /// `&`: text concatenation, element-wise over arrays. An error operand
    /// (element) propagates, before any text coercion.
    fn concat(&self, left: LiteralValue, right: LiteralValue) -> Result<LiteralValue, ExcelError> {
        fn join(left: LiteralValue, right: LiteralValue) -> Result<LiteralValue, ExcelError> {
            Ok(match (left, right) {
                (LiteralValue::Error(error), _) | (_, LiteralValue::Error(error)) => {
                    LiteralValue::Error(error)
                }
                (left, right) => LiteralValue::Text(format!(
                    "{}{}",
                    crate::coercion::to_text_invariant(&left),
                    crate::coercion::to_text_invariant(&right)
                )),
            })
        }
        self.broadcast_apply(left, right, join)
    }

    /* ---------- coercion helpers ---------- */
    fn coerce_number(&self, v: &LiteralValue) -> Result<f64, ExcelError> {
        coercion::to_number_lenient(v)
    }

    fn coerce_text(&self, v: &LiteralValue) -> String {
        coercion::to_text_invariant(v)
    }

    /* ---------- comparison ---------- */
    fn compare(
        &self,
        op: &str,
        left: LiteralValue,
        right: LiteralValue,
    ) -> Result<LiteralValue, ExcelError> {
        use LiteralValue::*;
        if matches!(left, Error(_)) {
            return Ok(left);
        }
        if matches!(right, Error(_)) {
            return Ok(right);
        }

        // arrays: element‑wise with broadcasting
        match (left, right) {
            (Array(l), Array(r)) => self.combine_arrays(l, r, |a, b| self.compare(op, a, b)),
            (Array(arr), v) => self.broadcast_apply(Array(arr), v, |a, b| self.compare(op, a, b)),
            (v, Array(arr)) => self.broadcast_apply(v, Array(arr), |a, b| self.compare(op, a, b)),
            (l, r) => {
                // Excel orders values by type first: numbers < text < logicals,
                // with no text-to-number coercion ("4" > 5). A blank operand
                // takes the other operand's type: 0, "" or FALSE.
                enum Key {
                    Number(f64),
                    Text(String),
                    Logical(bool),
                    Blank,
                }
                let system = self.context.date_system();
                let key = |v: &LiteralValue| match v {
                    Number(n) => Key::Number(*n),
                    Int(i) => Key::Number(*i as f64),
                    Boolean(b) => Key::Logical(*b),
                    Text(t) => Key::Text(t.clone()),
                    Empty => Key::Blank,
                    other => other
                        .as_serial_number_for(system)
                        .map(Key::Number)
                        .unwrap_or_else(|| Key::Text(crate::coercion::to_text_invariant(other))),
                };
                let rank = |k: &Key| match k {
                    Key::Number(_) => 0,
                    Key::Text(_) => 1,
                    Key::Logical(_) => 2,
                    Key::Blank => 3,
                };
                let (a, b) = match (key(&l), key(&r)) {
                    (Key::Blank, Key::Blank) => (Key::Number(0.0), Key::Number(0.0)),
                    (Key::Blank, other) | (other, Key::Blank) => {
                        let blank = match other {
                            Key::Number(_) => Key::Number(0.0),
                            Key::Text(_) => Key::Text(String::new()),
                            _ => Key::Logical(false),
                        };
                        if matches!(l, Empty) {
                            (blank, other)
                        } else {
                            (other, blank)
                        }
                    }
                    pair => pair,
                };
                let res = match (&a, &b) {
                    (Key::Number(x), Key::Number(y)) => self.cmp_f64(*x, *y, op),
                    (Key::Text(x), Key::Text(y)) => self.cmp_text(x, y, op),
                    (Key::Logical(x), Key::Logical(y)) => {
                        self.cmp_f64(f64::from(u8::from(*x)), f64::from(u8::from(*y)), op)
                    }
                    _ => self.cmp_f64(f64::from(rank(&a)), f64::from(rank(&b)), op),
                };
                Ok(LiteralValue::Boolean(res))
            }
        }
    }

    fn cmp_f64(&self, a: f64, b: f64, op: &str) -> bool {
        // Excel compares numbers to 15 significant digits: 0.1+0.2=0.3.
        let (a, b) = if a != b && same_to_15_digits(a, b) {
            (a, a)
        } else {
            (a, b)
        };
        match op {
            "=" => a == b,
            "<>" => a != b,
            ">" => a > b,
            "<" => a < b,
            ">=" => a >= b,
            "<=" => a <= b,
            _ => unreachable!(),
        }
    }
    fn cmp_text(&self, a: &str, b: &str, op: &str) -> bool {
        let loc = self.context.locale();
        let (a, b) = (loc.fold_case_invariant(a), loc.fold_case_invariant(b));
        self.cmp_f64(
            a.cmp(&b) as i32 as f64,
            0.0,
            match op {
                "=" => "=",
                "<>" => "<>",
                ">" => ">",
                "<" => "<",
                ">=" => ">=",
                "<=" => "<=",
                _ => unreachable!(),
            },
        )
    }
}

/// Whether two numbers agree when rounded to 15 significant digits.
fn same_to_15_digits(a: f64, b: f64) -> bool {
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    let scale = a.abs().max(b.abs());
    if (a - b).abs() > scale * 1e-14 {
        return false;
    }
    format!("{a:.14e}") == format!("{b:.14e}")
}

fn relocate_reference_for_offset(
    reference: &ReferenceType,
    row_delta: i64,
    col_delta: i64,
) -> Result<ReferenceType, ExcelError> {
    match reference {
        ReferenceType::Cell {
            sheet,
            row,
            col,
            row_abs,
            col_abs,
        } => Ok(ReferenceType::Cell {
            sheet: sheet.clone(),
            row: shift_axis_for_offset(*row, row_delta, *row_abs)?,
            col: shift_axis_for_offset(*col, col_delta, *col_abs)?,
            row_abs: *row_abs,
            col_abs: *col_abs,
        }),
        ReferenceType::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            start_row_abs,
            start_col_abs,
            end_row_abs,
            end_col_abs,
        } => Ok(ReferenceType::Range {
            sheet: sheet.clone(),
            start_row: shift_optional_axis_for_offset(*start_row, row_delta, *start_row_abs)?,
            start_col: shift_optional_axis_for_offset(*start_col, col_delta, *start_col_abs)?,
            end_row: shift_optional_axis_for_offset(*end_row, row_delta, *end_row_abs)?,
            end_col: shift_optional_axis_for_offset(*end_col, col_delta, *end_col_abs)?,
            start_row_abs: *start_row_abs,
            start_col_abs: *start_col_abs,
            end_row_abs: *end_row_abs,
            end_col_abs: *end_col_abs,
        }),
        // Defined names are placement-invariant: a relocated copy of the
        // formula references the same name, resolved at evaluation time.
        ReferenceType::NamedRange(name) => Ok(ReferenceType::NamedRange(name.clone())),
        ReferenceType::Table(_)
        | ReferenceType::Cell3D { .. }
        | ReferenceType::Range3D { .. }
        | ReferenceType::External(_) => Err(unsupported_reference_relocation_error()),
    }
}

fn shift_optional_axis_for_offset(
    value: Option<u32>,
    delta: i64,
    is_absolute: bool,
) -> Result<Option<u32>, ExcelError> {
    value
        .map(|value| shift_axis_for_offset(value, delta, is_absolute))
        .transpose()
}

fn shift_axis_for_offset(value: u32, delta: i64, is_absolute: bool) -> Result<u32, ExcelError> {
    if is_absolute {
        return Ok(value);
    }
    let shifted = i64::from(value) + delta;
    if shifted < 1 || shifted > i64::from(u32::MAX) {
        return Err(unsupported_reference_relocation_error());
    }
    Ok(shifted as u32)
}

fn unsupported_reference_relocation_error() -> ExcelError {
    ExcelError::new(ExcelErrorKind::Ref)
        .with_message("Unsupported reference relocation for FormulaPlane span evaluation")
}

#[cfg(test)]
mod format_algebra_tests {
    use super::*;
    use crate::engine::{EvalConfig, eval::Engine};
    use crate::format::FormatId;
    use crate::test_workbook::TestWorkbook;

    #[test]
    fn temporal_binary_format_algebra_pins_positive_and_negative_cases() {
        let engine = Engine::new(TestWorkbook::new(), EvalConfig::default());
        let interpreter = Interpreter::new(&engine, "Sheet1");

        assert_eq!(
            interpreter.binary_format('+', Some(FormatId::DATE), Some(FormatId::TIME)),
            Some(FormatId::DATETIME)
        );
        assert_eq!(
            interpreter.binary_format('+', Some(FormatId::DATE), Some(FormatId(9))),
            Some(FormatId::DATE),
            "Date + Percent follows the measured temporal-wins rule"
        );
        assert_eq!(
            interpreter.binary_format('-', Some(FormatId::DATE), Some(FormatId::DATE)),
            None,
            "Date - Date is an unformatted duration in days"
        );
        assert_eq!(
            interpreter.binary_format('+', Some(FormatId::DATE), Some(FormatId(49))),
            None,
            "Date + Text must not acquire a temporal annotation"
        );
        for (left, right) in [
            (FormatId::DATE, FormatId::DATE),
            (FormatId::DURATION, FormatId::DATE),
            (FormatId::DATE, FormatId(5)),
            (FormatId::DATETIME, FormatId::TIME),
        ] {
            assert_eq!(
                interpreter.binary_format('+', Some(left), Some(right)),
                None
            );
        }
    }
}
