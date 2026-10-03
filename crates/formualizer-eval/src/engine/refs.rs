//! Crate-private semantic reference classification and AST traversal.
//!
//! The collector deliberately streams references to a consumer. It preserves
//! source order and leaves graph/planner policy (range expansion, resolution,
//! placeholder creation, and error mapping) at the call site.

use crate::engine::arena::{AstNodeData, AstNodeId, DataStore};
use crate::engine::sheet_registry::SheetRegistry;
use formualizer_common::{ExcelError, ExcelErrorKind};
use formualizer_parse::parser::{
    ASTNode, ASTNodeType, ExternalReference, ReferenceType, TableReference,
};
use rustc_hash::FxHashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeclaredSheet<'a> {
    Current,
    Name(&'a str),
}

impl<'a> DeclaredSheet<'a> {
    fn from_option(sheet: Option<&'a str>) -> Self {
        match sheet {
            Some(name) => Self::Name(name),
            None => Self::Current,
        }
    }

    pub(crate) fn name(self) -> Option<&'a str> {
        match self {
            Self::Current => None,
            Self::Name(name) => Some(name),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CellReference<'a> {
    pub(crate) original: &'a ReferenceType,
    pub(crate) sheet: DeclaredSheet<'a>,
    pub(crate) row: u32,
    pub(crate) col: u32,
    pub(crate) row_abs: bool,
    pub(crate) col_abs: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RangeReference<'a> {
    pub(crate) original: &'a ReferenceType,
    pub(crate) sheet: DeclaredSheet<'a>,
    pub(crate) start_row: Option<u32>,
    pub(crate) start_col: Option<u32>,
    pub(crate) end_row: Option<u32>,
    pub(crate) end_col: Option<u32>,
    pub(crate) start_row_abs: bool,
    pub(crate) start_col_abs: bool,
    pub(crate) end_row_abs: bool,
    pub(crate) end_col_abs: bool,
}

impl RangeReference<'_> {
    pub(crate) fn finite_bounds(self) -> Option<(u32, u32, u32, u32)> {
        Some((
            self.start_row?,
            self.start_col?,
            self.end_row?,
            self.end_col?,
        ))
    }

    pub(crate) fn is_reversed(self) -> bool {
        self.finite_bounds()
            .is_some_and(|(sr, sc, er, ec)| sr > er || sc > ec)
    }

    pub(crate) fn saturating_area(self) -> Option<u64> {
        self.finite_bounds().map(|(sr, sc, er, ec)| {
            u64::from(er.saturating_sub(sr) + 1) * u64::from(ec.saturating_sub(sc) + 1)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SemanticReference<'a> {
    Cell(CellReference<'a>),
    FiniteRange(RangeReference<'a>),
    OpenRange(RangeReference<'a>),
    Name(&'a str),
    Table(&'a TableReference),
    ExternalSource(&'a ExternalReference),
    ThreeDimensional(&'a ReferenceType),
    #[allow(dead_code)]
    Unsupported(&'a ReferenceType),
}

pub(crate) fn classify(reference: &ReferenceType) -> SemanticReference<'_> {
    match reference {
        ReferenceType::Cell {
            sheet,
            row,
            col,
            row_abs,
            col_abs,
        } => SemanticReference::Cell(CellReference {
            original: reference,
            sheet: DeclaredSheet::from_option(sheet.as_deref()),
            row: *row,
            col: *col,
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
        } => {
            let range = RangeReference {
                original: reference,
                sheet: DeclaredSheet::from_option(sheet.as_deref()),
                start_row: *start_row,
                start_col: *start_col,
                end_row: *end_row,
                end_col: *end_col,
                start_row_abs: *start_row_abs,
                start_col_abs: *start_col_abs,
                end_row_abs: *end_row_abs,
                end_col_abs: *end_col_abs,
            };
            if range.finite_bounds().is_some() {
                SemanticReference::FiniteRange(range)
            } else {
                SemanticReference::OpenRange(range)
            }
        }
        ReferenceType::NamedRange(name) => SemanticReference::Name(name),
        ReferenceType::Table(table) => SemanticReference::Table(table),
        ReferenceType::External(external) => SemanticReference::ExternalSource(external),
        ReferenceType::Cell3D { .. } | ReferenceType::Range3D { .. } => {
            SemanticReference::ThreeDimensional(reference)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LocalBindingStyle {
    #[default]
    None,
    LocalBindingPairs,
    LambdaParameters,
}

/// Arguments Excel reads only as a reference (its sheet, position or shape),
/// never for their values. They are not calculation dependencies, so a
/// formula such as `=ROWS(A$1:A5)` in A5 is not circular. `cell_info` is the
/// CELL info_type when it is a literal; the contents/type forms read values.
pub(crate) fn reference_only_argument(name: &str, index: usize, cell_info: Option<&str>) -> bool {
    let name = name.strip_prefix("_xlfn.").unwrap_or(name);
    if name.eq_ignore_ascii_case("CELL") {
        return index == 1
            && cell_info.is_some_and(|info| {
                !info.eq_ignore_ascii_case("contents") && !info.eq_ignore_ascii_case("type")
            });
    }
    index == 0
        && [
            "ROW", "ROWS", "COLUMN", "COLUMNS", "AREAS", "ISREF", "SHEET",
        ]
        .iter()
        .any(|f| name.eq_ignore_ascii_case(f))
}

/// A constant INDEX selector (row_num, column_num or area_num) as the number
/// INDEX reads: a number, a logical or numeric text. `None` for anything that
/// is not such a constant (a cell, an array, date text, ...): its value is
/// known only when the formula runs.
fn static_number_value(value: &formualizer_common::LiteralValue) -> Option<f64> {
    use formualizer_common::LiteralValue;
    match value {
        LiteralValue::Int(_)
        | LiteralValue::Number(_)
        | LiteralValue::Boolean(_)
        | LiteralValue::Text(_) => crate::coercion::to_number_lenient(value)
            .ok()
            .filter(|number| number.is_finite()),
        _ => None,
    }
}

/// [`static_number_value`] of a selector written in a formula, with any
/// leading signs.
fn static_number_tree(node: &ASTNode) -> Option<f64> {
    match &node.node_type {
        ASTNodeType::Literal(value) => static_number_value(value),
        ASTNodeType::UnaryOp { op, expr } if op == "+" => static_number_tree(expr),
        ASTNodeType::UnaryOp { op, expr } if op == "-" => static_number_tree(expr).map(|n| -n),
        _ => None,
    }
}

pub(crate) fn static_number_arena(store: &DataStore, id: AstNodeId) -> Option<f64> {
    match store.get_node(id)? {
        AstNodeData::Literal(value) => static_number_value(&store.retrieve_value(*value)),
        AstNodeData::UnaryOp { op_id, expr_id } => match store.resolve_ast_string(*op_id) {
            "+" => static_number_arena(store, *expr_id),
            "-" => static_number_arena(store, *expr_id).map(|n| -n),
            _ => None,
        },
        _ => None,
    }
}

/// A constant selector written in a formula, as the whole-number position
/// INDEX reads it ([`crate::coercion::snapped_whole_number`]).
pub(crate) fn static_index_tree(node: &ASTNode) -> Option<i64> {
    static_number_tree(node).map(|n| crate::coercion::snapped_whole_number(n) as i64)
}

fn static_index_arena(store: &DataStore, id: AstNodeId) -> Option<i64> {
    static_number_arena(store, id).map(|n| crate::coercion::snapped_whole_number(n) as i64)
}

/// The area INDEX selects when its area_num is constant: area 1 when absent
/// or omitted.
fn static_index_area(name: &str, arg_count: usize, area: Option<Option<i64>>) -> Option<i64> {
    if !name.eq_ignore_ascii_case("INDEX") || !(2..=4).contains(&arg_count) {
        return None;
    }
    area.unwrap_or(Some(1))
}

/// Numbers the operands of INDEX's reference argument (those of its `,`
/// unions, in the order written) and keeps the ones INDEX may read. Areas are
/// numbered while each operand is a cell or a range, one area each; an
/// operand whose area is known and is not `area` is never read, because INDEX
/// reads only the area it selects. From the first operand whose number of
/// areas the formula alone does not settle (a name, which may be defined as
/// several areas and be redefined later, a function, ...), every operand is
/// kept. `single_area` gives a cell or range operand's sheet (see
/// [`operand_sheet_key`]); the areas of a union lie on one sheet (it is
/// `#VALUE!` otherwise), so operands are left out only when every cell or
/// range lies on the same sheet, which the selected area then still ties the
/// formula to. `None` when every operand is kept.
fn index_read_operands<T: Copy, S: PartialEq>(
    operands: &[T],
    area: i64,
    single_area: impl Fn(T) -> Option<S>,
) -> Option<Vec<T>> {
    let sheets: Vec<Option<S>> = operands
        .iter()
        .map(|&operand| single_area(operand))
        .collect();
    let mut written = sheets.iter().flatten();
    if let Some(first) = written.next()
        && written.any(|sheet| sheet != first)
    {
        return None;
    }
    let mut number = Some(1i64);
    let mut read = Vec::with_capacity(operands.len());
    for (&operand, sheet) in operands.iter().zip(&sheets) {
        match number {
            Some(current) if sheet.is_some() => {
                if current == area {
                    read.push(operand);
                }
                number = current.checked_add(1);
            }
            _ => {
                read.push(operand);
                number = None;
            }
        }
    }
    (read.len() < operands.len()).then_some(read)
}

/// The sheet a cell or range operand of INDEX's union lies on, as a key that
/// matches whatever the case of its name: an unqualified operand lies on the
/// formula's sheet, `current_sheet`, so `(A1,Sheet1!D1)` on Sheet1 is one
/// sheet. `None` for an unqualified operand when the formula's sheet is not
/// known, which then matches only other unqualified operands.
fn operand_sheet_key(sheet: Option<&str>, current_sheet: Option<&str>) -> Option<String> {
    sheet.or(current_sheet).map(str::to_lowercase)
}

/// The operands of INDEX's reference argument it may read (see
/// [`index_read_operands`]); `None` when every one may be read.
fn index_read_operands_tree<'a>(
    name: &str,
    args: &'a [ASTNode],
    current_sheet: Option<&str>,
) -> Option<Vec<&'a ASTNode>> {
    let area = static_index_area(
        name,
        args.len(),
        args.get(3).map(|area| match area.node_type {
            ASTNodeType::Omitted => Some(1),
            _ => static_index_tree(area),
        }),
    )?;
    fn flatten<'a>(node: &'a ASTNode, out: &mut Vec<&'a ASTNode>) {
        match &node.node_type {
            ASTNodeType::BinaryOp { op, left, right } if op == "," => {
                flatten(left, out);
                flatten(right, out);
            }
            _ => out.push(node),
        }
    }
    let mut operands = Vec::new();
    flatten(&args[0], &mut operands);
    index_read_operands(&operands, area, |operand: &ASTNode| {
        match &operand.node_type {
            ASTNodeType::Reference {
                reference: ReferenceType::Cell { sheet, .. } | ReferenceType::Range { sheet, .. },
                ..
            } => Some(operand_sheet_key(sheet.as_deref(), current_sheet)),
            _ => None,
        }
    })
}

/// The arena form of [`index_read_operands_tree`].
fn index_read_operands_arena(
    store: &DataStore,
    sheet_registry: &SheetRegistry,
    name: &str,
    args: &[AstNodeId],
    current_sheet: Option<&str>,
) -> Option<Vec<AstNodeId>> {
    let area = static_index_area(
        name,
        args.len(),
        args.get(3).map(|&area| match store.get_node(area) {
            Some(AstNodeData::Omitted) => Some(1),
            _ => static_index_arena(store, area),
        }),
    )?;
    fn flatten(store: &DataStore, id: AstNodeId, out: &mut Vec<AstNodeId>) {
        match store.get_node(id) {
            Some(AstNodeData::BinaryOp {
                op_id,
                left_id,
                right_id,
                ..
            }) if store.resolve_ast_string(*op_id) == "," => {
                let (left, right) = (*left_id, *right_id);
                flatten(store, left, out);
                flatten(store, right, out);
            }
            _ => out.push(id),
        }
    }
    let mut operands = Vec::new();
    flatten(store, args[0], &mut operands);
    index_read_operands(&operands, area, |operand| match store.get_node(operand) {
        Some(AstNodeData::Reference {
            ref_type:
                ref_type @ (crate::engine::arena::CompactRefType::Cell { .. }
                | crate::engine::arena::CompactRefType::Range { .. }),
            ..
        }) => match store.reconstruct_reference_type_for_eval(ref_type, sheet_registry) {
            ReferenceType::Cell { sheet, .. } | ReferenceType::Range { sheet, .. } => {
                Some(operand_sheet_key(sheet.as_deref(), current_sheet))
            }
            _ => None,
        },
        _ => None,
    })
}

fn tree_cell_info(name: &str, args: &[ASTNode]) -> Option<String> {
    if !name.eq_ignore_ascii_case("CELL") {
        return None;
    }
    match args.first().map(|arg| &arg.node_type) {
        Some(ASTNodeType::Literal(formualizer_common::LiteralValue::Text(info))) => {
            Some(info.clone())
        }
        _ => None,
    }
}

/// Streams the references `ast` reads to `visitor`, in source order.
/// `current_sheet` gives the sheet the formula lies on, when known, which an
/// unqualified reference means.
pub(crate) fn visit_tree_references<C>(
    ast: &ASTNode,
    context: &mut C,
    local_binding_style: fn(&C, &str, usize) -> LocalBindingStyle,
    current_sheet: fn(&C) -> Option<&str>,
    visitor: fn(&mut C, SemanticReference<'_>) -> Result<(), ExcelError>,
) -> Result<(), ExcelError> {
    enum Frame<'a> {
        Node(&'a ASTNode),
        AddBinding(&'a str),
        ExitScope,
    }

    let mut local_scopes: Vec<FxHashSet<String>> = Vec::new();
    let mut stack = vec![Frame::Node(ast)];
    while let Some(frame) = stack.pop() {
        let ast = match frame {
            Frame::AddBinding(name) => {
                if let Some(scope) = local_scopes.last_mut() {
                    scope.insert(name.to_ascii_uppercase());
                }
                continue;
            }
            Frame::ExitScope => {
                local_scopes.pop();
                continue;
            }
            Frame::Node(ast) => ast,
        };

        match &ast.node_type {
            ASTNodeType::Reference { reference, .. } => {
                if let ReferenceType::NamedRange(name) = reference
                    && !local_scopes.is_empty()
                {
                    let key = name.to_ascii_uppercase();
                    if local_scopes.iter().rev().any(|scope| scope.contains(&key)) {
                        continue;
                    }
                }
                visitor(context, classify(reference))?;
            }
            ASTNodeType::BinaryOp { left, right, .. } => {
                stack.push(Frame::Node(right));
                stack.push(Frame::Node(left));
            }
            ASTNodeType::UnaryOp { expr, .. } => stack.push(Frame::Node(expr)),
            ASTNodeType::Function { name, args } => {
                match local_binding_style(context, name, args.len()) {
                    LocalBindingStyle::LocalBindingPairs
                        if args.len() >= 3 && args.len() % 2 == 1 =>
                    {
                        local_scopes.push(FxHashSet::default());
                        stack.push(Frame::ExitScope);
                        stack.push(Frame::Node(&args[args.len() - 1]));
                        for pair_idx in (0..args.len() - 1).step_by(2).rev() {
                            if let ASTNodeType::Reference {
                                reference: ReferenceType::NamedRange(local_name),
                                ..
                            } = &args[pair_idx].node_type
                            {
                                stack.push(Frame::AddBinding(local_name));
                            }
                            stack.push(Frame::Node(&args[pair_idx + 1]));
                        }
                    }
                    LocalBindingStyle::LambdaParameters => {
                        if let Some(body) = args.last() {
                            let mut scope = FxHashSet::default();
                            for parameter in &args[..args.len().saturating_sub(1)] {
                                if let ASTNodeType::Reference {
                                    reference: ReferenceType::NamedRange(name),
                                    ..
                                } = &parameter.node_type
                                {
                                    scope.insert(name.to_ascii_uppercase());
                                }
                            }
                            local_scopes.push(scope);
                            stack.push(Frame::ExitScope);
                            stack.push(Frame::Node(body));
                        }
                    }
                    _ => {
                        let cell_info = tree_cell_info(name, args);
                        // INDEX reads only the area it selects.
                        let index_operands =
                            index_read_operands_tree(name, args, current_sheet(context));
                        for (index, arg) in args.iter().enumerate().rev() {
                            if index == 0
                                && let Some(operands) = &index_operands
                            {
                                for operand in operands.iter().rev() {
                                    stack.push(Frame::Node(operand));
                                }
                                continue;
                            }
                            // Names and tables keep their definition dependency.
                            if matches!(
                                arg.node_type,
                                ASTNodeType::Reference {
                                    reference: ReferenceType::Cell { .. }
                                        | ReferenceType::Range { .. },
                                    ..
                                }
                            ) && reference_only_argument(name, index, cell_info.as_deref())
                            {
                                continue;
                            }
                            stack.push(Frame::Node(arg));
                        }
                    }
                }
            }
            ASTNodeType::Call { callee, args } => {
                for arg in args.iter().rev() {
                    stack.push(Frame::Node(arg));
                }
                stack.push(Frame::Node(callee));
            }
            ASTNodeType::Array(rows) => {
                for item in rows.iter().rev().flat_map(|row| row.iter().rev()) {
                    stack.push(Frame::Node(item));
                }
            }
            ASTNodeType::Literal(_) | ASTNodeType::Omitted => {}
        }
    }
    Ok(())
}

/// The arena form of [`visit_tree_references`].
pub(crate) fn visit_arena_references<C>(
    ast_id: AstNodeId,
    context: &mut C,
    data_store: fn(&C) -> &DataStore,
    sheet_registry: fn(&C) -> &SheetRegistry,
    current_sheet: fn(&C) -> Option<&str>,
    visitor: fn(&mut C, SemanticReference<'_>) -> Result<(), ExcelError>,
) -> Result<(), ExcelError> {
    let node = data_store(context)
        .get_node(ast_id)
        .cloned()
        .ok_or_else(missing_ast_error)?;

    match node {
        AstNodeData::Reference { ref_type, .. } => {
            let reference = data_store(context)
                .reconstruct_reference_type_for_eval(&ref_type, sheet_registry(context));
            visitor(context, classify(&reference))
        }
        AstNodeData::UnaryOp { expr_id, .. } => visit_arena_references(
            expr_id,
            context,
            data_store,
            sheet_registry,
            current_sheet,
            visitor,
        ),
        AstNodeData::BinaryOp {
            left_id, right_id, ..
        } => {
            visit_arena_references(
                left_id,
                context,
                data_store,
                sheet_registry,
                current_sheet,
                visitor,
            )?;
            visit_arena_references(
                right_id,
                context,
                data_store,
                sheet_registry,
                current_sheet,
                visitor,
            )
        }
        AstNodeData::Function { name_id, .. } => {
            let store = data_store(context);
            let name = store.resolve_ast_string(name_id).to_owned();
            let arg_count = store.get_args(ast_id).map_or(0, <[_]>::len);
            let cell_info = if name.eq_ignore_ascii_case("CELL") {
                store
                    .get_args(ast_id)
                    .and_then(|args| args.first().copied())
                    .and_then(|first| match store.get_node(first) {
                        Some(AstNodeData::Literal(value)) => match store.retrieve_value(*value) {
                            formualizer_common::LiteralValue::Text(info) => Some(info),
                            _ => None,
                        },
                        _ => None,
                    })
            } else {
                None
            };
            // INDEX reads only the area it selects.
            let index_operands = store.get_args(ast_id).and_then(|args| {
                index_read_operands_arena(
                    store,
                    sheet_registry(context),
                    &name,
                    args,
                    current_sheet(context),
                )
            });
            for index in 0..arg_count {
                if index == 0
                    && let Some(operands) = &index_operands
                {
                    for &operand in operands {
                        visit_arena_references(
                            operand,
                            context,
                            data_store,
                            sheet_registry,
                            current_sheet,
                            visitor,
                        )?;
                    }
                    continue;
                }
                let store = data_store(context);
                let child = store.get_args(ast_id).expect("args disappeared")[index];
                if matches!(
                    store.get_node(child),
                    Some(AstNodeData::Reference {
                        ref_type: crate::engine::arena::CompactRefType::Cell { .. }
                            | crate::engine::arena::CompactRefType::Range { .. },
                        ..
                    })
                ) && reference_only_argument(&name, index, cell_info.as_deref())
                {
                    continue;
                }
                visit_arena_references(
                    child,
                    context,
                    data_store,
                    sheet_registry,
                    current_sheet,
                    visitor,
                )?;
            }
            Ok(())
        }
        AstNodeData::Array { .. } => {
            let element_count = data_store(context)
                .get_array_elems(ast_id)
                .map_or(0, |(_, _, elements)| elements.len());
            for index in 0..element_count {
                let child = data_store(context)
                    .get_array_elems(ast_id)
                    .expect("array elements disappeared")
                    .2[index];
                visit_arena_references(
                    child,
                    context,
                    data_store,
                    sheet_registry,
                    current_sheet,
                    visitor,
                )?;
            }
            Ok(())
        }
        AstNodeData::Literal(_) | AstNodeData::Omitted => Ok(()),
    }
}

fn missing_ast_error() -> ExcelError {
    ExcelError::new(ExcelErrorKind::Value).with_message("Missing interned formula AST")
}

#[cfg(test)]
mod tests {
    use super::*;
    use formualizer_parse::parse;

    #[derive(Default)]
    struct Seen(Vec<String>);

    fn no_bindings(_: &Seen, _: &str, _: usize) -> LocalBindingStyle {
        LocalBindingStyle::None
    }

    fn no_sheet<C>(_: &C) -> Option<&str> {
        None
    }

    fn sheet1<C>(_: &C) -> Option<&str> {
        Some("Sheet1")
    }

    fn record(seen: &mut Seen, reference: SemanticReference<'_>) -> Result<(), ExcelError> {
        let label = match reference {
            SemanticReference::Cell(cell) => format!(
                "cell:{:?}:{}:{}:{}:{}",
                cell.sheet, cell.row, cell.col, cell.row_abs, cell.col_abs
            ),
            SemanticReference::FiniteRange(range) => format!(
                "finite:{:?}:{:?}:{}:{}:{}:{}",
                range.sheet,
                range.finite_bounds(),
                range.start_row_abs,
                range.start_col_abs,
                range.end_row_abs,
                range.end_col_abs
            ),
            SemanticReference::OpenRange(range) => format!(
                "open:{:?}:{:?}:{:?}:{:?}:{:?}",
                range.sheet, range.start_row, range.start_col, range.end_row, range.end_col
            ),
            SemanticReference::Name(name) => format!("name:{name}"),
            SemanticReference::Table(table) => format!("table:{}", table.name),
            SemanticReference::ExternalSource(external) => format!("external:{}", external.raw),
            SemanticReference::ThreeDimensional(_) => "3d".to_string(),
            SemanticReference::Unsupported(_) => "unsupported".to_string(),
        };
        seen.0.push(label);
        Ok(())
    }

    #[test]
    fn classifies_in_source_order_without_expanding_ranges() {
        let ast = parse(
            "=SUM($A1,Sheet2!B$2,$C3:D$4,A1:A,A:A,1:1,NamedThing,Table1[#Data],[book]Sheet!A1,Sheet1:Sheet3!E5)",
        )
        .unwrap();
        let mut seen = Seen::default();
        visit_tree_references(&ast, &mut seen, no_bindings, no_sheet, record).unwrap();

        assert_eq!(seen.0.len(), 10);
        assert!(seen.0[0].starts_with("cell:Current:1:1:false:true"));
        assert!(seen.0[1].starts_with("cell:Name(\"Sheet2\"):2:2:true:false"));
        assert_eq!(
            seen.0[2],
            "finite:Current:Some((3, 3, 4, 4)):false:true:true:false"
        );
        assert!(seen.0[3].starts_with("open:Current:Some(1):Some(1):None:Some(1)"));
        assert!(seen.0[6].starts_with("name:NamedThing"));
        assert!(seen.0[7].starts_with("table:Table1"));
        assert!(seen.0[8].starts_with("external:"));
        assert_eq!(seen.0[9], "3d");
    }

    struct Arena<'s> {
        seen: Seen,
        store: &'s DataStore,
        sheets: &'s SheetRegistry,
    }

    fn arena_store<'a>(arena: &'a Arena<'_>) -> &'a DataStore {
        arena.store
    }

    fn arena_sheets<'a>(arena: &'a Arena<'_>) -> &'a SheetRegistry {
        arena.sheets
    }

    fn record_arena(
        arena: &mut Arena<'_>,
        reference: SemanticReference<'_>,
    ) -> Result<(), ExcelError> {
        record(&mut arena.seen, reference)
    }

    #[test]
    fn index_reads_only_the_area_it_selects() {
        // With a constant area_num (1 when absent), an area of INDEX's union
        // that is numbered and not selected is no dependency. A name may hold
        // several areas, so the areas from it on stay dependencies; so do all
        // of them when area_num is known only at run time. The formula lies
        // on Sheet1, which its unqualified areas mean.
        let cases: [(&str, &[&str]); 16] = [
            ("=INDEX((A1,B1:B2,C1),1,1,2)", &["finite"]),
            ("=INDEX((A1,B1:B2,C1),1,1,\"3\")", &["cell:Current:1:3"]),
            ("=INDEX((A1,B1:B2,C1),1,1,TRUE)", &["cell:Current:1:1"]),
            ("=INDEX((A1,B1),1,1)", &["cell:Current:1:1"]),
            ("=INDEX((A1,B1),1,1,)", &["cell:Current:1:1"]),
            ("=INDEX(((A1,B1),C1),1,1,-(-3))", &["cell:Current:1:3"]),
            ("=INDEX(A1:A3,1,1,2)", &[]),
            (
                "=INDEX((A1,Areas,C1),1,1,2)",
                &["name:Areas", "cell:Current:1:3"],
            ),
            (
                "=INDEX((A1,B1),1,1,F1)",
                &["cell:Current:1:1", "cell:Current:1:2", "cell:Current:1:6"],
            ),
            (
                "=INDEX(((A1,B1) A1:B1),1,1,2)",
                &["cell:Current:1:1", "cell:Current:1:2", "finite"],
            ),
            (
                "=SUM(A1,B1)+INDEX((C1,D1),1,1,2)",
                &["cell", "cell", "cell:Current:1:4"],
            ),
            // Areas on different sheets make the union #VALUE!: all are kept.
            ("=INDEX((A1,Data!B1),1,1,1)", &["cell:Current", "cell:Name"]),
            (
                "=INDEX((Data!A1,Data!B1),1,1,1)",
                &["cell:Name(\"Data\"):1:1"],
            ),
            // An unqualified area lies on the formula's sheet, whatever the
            // case the other areas spell it in.
            (
                "=INDEX((A1,Sheet1!D1),1,1,2)",
                &["cell:Name(\"Sheet1\"):1:4"],
            ),
            ("=INDEX((A1,SHEET1!D1),1,1,1)", &["cell:Current:1:1"]),
            (
                "=INDEX((sheet1!A1,Sheet1!D1),1,1,2)",
                &["cell:Name(\"Sheet1\"):1:4"],
            ),
        ];
        let sheets = SheetRegistry::new();
        let mut store = DataStore::new();
        for (formula, expected) in cases {
            let ast = parse(formula).unwrap();
            let mut seen = Seen::default();
            visit_tree_references(&ast, &mut seen, no_bindings, sheet1, record).unwrap();
            assert_eq!(seen.0.len(), expected.len(), "{formula}: {:?}", seen.0);
            for (label, prefix) in seen.0.iter().zip(expected) {
                assert!(label.starts_with(prefix), "{formula}: {label} vs {prefix}");
            }

            let id = store.store_ast(&ast, &sheets);
            let mut arena = Arena {
                seen: Seen::default(),
                store: &store,
                sheets: &sheets,
            };
            visit_arena_references(
                id,
                &mut arena,
                arena_store,
                arena_sheets,
                sheet1,
                record_arena,
            )
            .unwrap();
            assert_eq!(arena.seen.0.len(), expected.len(), "arena {formula}");
            for (label, prefix) in arena.seen.0.iter().zip(expected) {
                assert!(label.starts_with(prefix), "arena {formula}: {label}");
            }
        }

        // Without the formula's sheet, an unqualified area matches only other
        // unqualified ones: every area is kept.
        let ast = parse("=INDEX((A1,Sheet1!D1),1,1,2)").unwrap();
        let mut seen = Seen::default();
        visit_tree_references(&ast, &mut seen, no_bindings, no_sheet, record).unwrap();
        assert_eq!(seen.0.len(), 2, "{:?}", seen.0);
    }

    #[test]
    fn finite_range_helpers_expose_reversal_and_saturating_area() {
        let ast = parse("=D4:B2").unwrap();
        let mut observed = None;
        fn capture(
            observed: &mut Option<(bool, Option<u64>)>,
            reference: SemanticReference<'_>,
        ) -> Result<(), ExcelError> {
            let SemanticReference::FiniteRange(range) = reference else {
                panic!("expected finite range")
            };
            *observed = Some((range.is_reversed(), range.saturating_area()));
            Ok(())
        }
        fn none(_: &Option<(bool, Option<u64>)>, _: &str, _: usize) -> LocalBindingStyle {
            LocalBindingStyle::None
        }
        visit_tree_references(&ast, &mut observed, none, no_sheet, capture).unwrap();
        assert_eq!(observed, Some((true, Some(1))));
    }

    #[test]
    fn finite_range_area_uses_u64_at_and_above_u32_boundary() {
        fn capture(
            observed: &mut Option<u64>,
            reference: SemanticReference<'_>,
        ) -> Result<(), ExcelError> {
            let SemanticReference::FiniteRange(range) = reference else {
                panic!("expected finite range")
            };
            *observed = range.saturating_area();
            Ok(())
        }
        fn none(_: &Option<u64>, _: &str, _: usize) -> LocalBindingStyle {
            LocalBindingStyle::None
        }

        for (formula, expected) in [
            ("=A1:FLA983055", 4_294_967_295),
            ("=A1:XFD262144", 4_294_967_296),
            ("=A1:XFD1048576", 17_179_869_184),
        ] {
            let ast = parse(formula).unwrap();
            let mut observed = None;
            visit_tree_references(&ast, &mut observed, none, no_sheet, capture).unwrap();
            assert_eq!(observed, Some(expected), "{formula}");
        }
    }
}
