//! Strict cache-only XLSX recalculation, without a rich document model.
//! Unsupported package/formula cases fail before any output is published.
mod calc_always;
pub use calc_always::is_excel_function;
mod package;
mod rich;
mod sheet;
mod xml;

use super::recalculate::{DEFAULT_ERROR_LOCATION_LIMIT, RecalculateStatus, RecalculateSummary};
use crate::{CalamineAdapter, IoError, SpreadsheetReader, workbook::WBResolver};
use formualizer_common::{CellAddress, DateSystem, ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_eval::engine::ingest::EngineLoadStream;
use formualizer_eval::engine::inspect::{SnapshotOptions, Staleness};
use formualizer_eval::engine::{CancelToken, CyclePolicy, Engine, EvalConfig, FormulaParsePolicy};
use std::collections::{BTreeMap, HashSet};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::ops::Range;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// Bounds apply to actual decompression, XML depth/cells and output, not only ZIP headers.
#[derive(Debug, Clone)]
pub struct XlsxRecalculateLimits {
    pub max_input_bytes: usize,
    pub max_entries: usize,
    pub max_expanded_bytes: usize,
    pub max_worksheet_bytes: usize,
    pub max_formula_cells: usize,
    pub max_output_bytes: usize,
    pub max_xml_depth: usize,
    pub max_cells: usize,
    /// Limits the width of Calamine's per-column ingestion builders.
    pub max_columns: u32,
}
impl Default for XlsxRecalculateLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 << 20,
            max_entries: 10_000,
            max_expanded_bytes: 256 << 20,
            max_worksheet_bytes: 128 << 20,
            max_formula_cells: 100_000,
            max_output_bytes: 64 << 20,
            max_xml_depth: 128,
            max_cells: 8_000_000,
            max_columns: 16_384,
        }
    }
}
/// The source date system is authoritative; other evaluation policies come from `eval_config`.
#[derive(Debug, Clone)]
pub struct XlsxRecalculateOptions {
    pub eval_config: EvalConfig,
    pub cancel: Option<CancelToken>,
    pub limits: XlsxRecalculateLimits,
    pub error_location_limit: usize,
}
impl Default for XlsxRecalculateOptions {
    fn default() -> Self {
        Self {
            eval_config: EvalConfig::default(),
            cancel: None,
            limits: XlsxRecalculateLimits::default(),
            error_location_limit: DEFAULT_ERROR_LOCATION_LIMIT,
        }
    }
}
/// `cache_cells_changed` counts physical caches (and their rich error tags)
/// actually patched, not engine deltas.
#[derive(Debug, Clone)]
pub struct XlsxRecalculateResult {
    pub bytes: Vec<u8>,
    pub summary: RecalculateSummary,
    pub formula_cells: usize,
    pub cache_cells_changed: usize,
    pub worksheet_parts_changed: usize,
}
fn unsupported(feature: impl Into<String>, context: impl Into<String>) -> IoError {
    IoError::Unsupported {
        feature: feature.into(),
        context: context.into(),
    }
}
fn checkpoint(token: &Option<CancelToken>) -> Result<(), IoError> {
    if token.as_ref().is_some_and(CancelToken::is_cancelled) {
        Err(IoError::Engine(formualizer_common::ExcelError::new(
            formualizer_common::ExcelErrorKind::Cancelled,
        )))
    } else {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
enum Cache {
    Number(f64),
    Boolean(bool),
    Text(String),
    Error(String),
    Empty,
}
impl Cache {
    fn from_value(value: LiteralValue, system: DateSystem) -> Result<Self, IoError> {
        Ok(match value {
            LiteralValue::Boolean(b) => Self::Boolean(b),
            LiteralValue::Text(text) => {
                // _xHHHH_ has application-level escape semantics that differ
                // across cached-string readers. Do not silently corrupt it.
                if text.as_bytes().windows(7).any(|w| {
                    w[0] == b'_'
                        && w[1] == b'x'
                        && w[6] == b'_'
                        && w[2..6].iter().all(u8::is_ascii_hexdigit)
                }) {
                    return Err(unsupported(
                        "escape-looking cached text",
                        "cache-only writer",
                    ));
                }
                if !text.chars().all(|c| matches!(c, '\t'|'\n'|'\r'|' '..='\u{d7ff}'|'\u{e000}'..='\u{fffd}'|'\u{10000}'..='\u{10ffff}')) {
                    return Err(unsupported("XML-invalid cached text control", "cache-only writer"));
                }
                Self::Text(text)
            }
            // Excel caches an error that has no legacy XLSX code as #VALUE!;
            // only a cell's vm rich value records the real error.
            LiteralValue::Error(error)
                if matches!(error.kind, ExcelErrorKind::Spill | ExcelErrorKind::Calc) =>
            {
                Self::Error("#VALUE!".into())
            }
            LiteralValue::Error(error) => {
                let token = error.kind.to_string();
                if !matches!(
                    token.as_str(),
                    "#DIV/0!" | "#N/A" | "#NAME?" | "#NULL!" | "#NUM!" | "#REF!" | "#VALUE!"
                ) {
                    return Err(unsupported(
                        "engine-specific error has no approved XLSX cache encoding",
                        token,
                    ));
                }
                Self::Error(token)
            }
            LiteralValue::Empty => Self::Empty,
            LiteralValue::Array(_) => {
                return Err(unsupported("array formula result", "cache-only writer"));
            }
            LiteralValue::Pending => {
                return Err(unsupported("pending formula result", "cache-only writer"));
            }
            value => {
                let mut serial = value.as_serial_number_for(system).ok_or_else(|| {
                    unsupported("unrepresentable scalar cache", "cache-only writer")
                })?;
                // The common helper retains historical whole-second duration
                // conversion; preserve the fractional remainder here as well.
                if let LiteralValue::Duration(duration) = value {
                    serial += f64::from(duration.subsec_nanos()) / 86_400_000_000_000.0;
                }
                if !serial.is_finite() {
                    return Err(unsupported("non-finite formula cache", "cache-only writer"));
                }
                Self::Number(serial)
            }
        })
    }
    fn kind(&self) -> Option<&'static str> {
        match self {
            Self::Number(_) | Self::Empty => None,
            Self::Boolean(_) => Some("b"),
            Self::Text(_) => Some("str"),
            Self::Error(_) => Some("e"),
        }
    }
    fn text(&self) -> String {
        match self {
            Self::Number(n) => n.to_string(),
            Self::Boolean(b) => if *b { "1" } else { "0" }.into(),
            Self::Text(t) | Self::Error(t) => quick_xml::escape::escape(t).replace('\r', "&#13;"),
            Self::Empty => String::new(),
        }
    }
    fn matches(&self, cell: &sheet::Cell) -> bool {
        if cell.inline.is_some() {
            return false;
        }
        let Some(v) = &cell.value else {
            return false;
        };
        match (self, cell.kind.as_deref()) {
            (Self::Number(n), None | Some("n")) => v
                .text
                .trim()
                .parse::<f64>()
                .ok()
                .is_some_and(|old| old.is_finite() && old == *n),
            (Self::Empty, None | Some("n")) => v.text.is_empty(),
            (Self::Boolean(b), Some("b")) => {
                matches!((v.text.trim(), b), ("1", true) | ("0", false))
            }
            (Self::Text(t), Some("str")) => &v.text == t,
            (Self::Error(e), Some("e")) => &v.text == e,
            _ => false,
        }
    }
}
/// The result a formula cell's cache records: a number, `t="b"` boolean,
/// `t="str"` text or `t="e"` Excel error. Inline, shared-string and date
/// caches, and absent ones, give `None`.
fn cached_result(cell: &sheet::Cell) -> Option<LiteralValue> {
    use formualizer_common::{ExcelError, ExcelErrorKind as K};
    if cell.inline.is_some() {
        return None;
    }
    let text = &cell.value.as_ref()?.text;
    match cell.kind.as_deref() {
        None | Some("n") => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(LiteralValue::Number),
        Some("b") => match text.trim() {
            "1" => Some(LiteralValue::Boolean(true)),
            "0" => Some(LiteralValue::Boolean(false)),
            _ => None,
        },
        Some("str") => Some(LiteralValue::Text(text.clone())),
        Some("e") => K::try_parse(text)
            .filter(|kind| {
                matches!(
                    kind,
                    K::Null
                        | K::Div
                        | K::Value
                        | K::Ref
                        | K::Name
                        | K::Num
                        | K::Na
                        | K::Spill
                        | K::Calc
                )
            })
            .map(|kind| LiteralValue::Error(ExcelError::new(kind))),
        _ => None,
    }
}
/// The last calculated value a formula cell's cache records: its
/// [`cached_result`], or the error a rich value tags its cached #VALUE! with
/// (a #SPILL! or #CALC!).
fn last_calculated_value(cell: &sheet::Cell, tags: &rich::RichTags) -> Option<LiteralValue> {
    if let Some((vm, _)) = &cell.value_metadata
        && cell.kind.as_deref() == Some("e")
        && let Some(error) = tags.get(*vm)
    {
        return Some(LiteralValue::Error(ExcelError::new(error.kind)));
    }
    cached_result(cell)
}
struct Patch {
    span: Range<usize>,
    replacement: Vec<u8>,
}
fn cache_patches(xml: &[u8], cell: &sheet::Cell, value: &Cache, patches: &mut Vec<Patch>) {
    let wanted = value.kind();
    let current = cell.kind.as_deref().filter(|t| *t != "n");
    if wanted != current {
        let (span, replacement) = match &cell.kind_span {
            Some(span) => (
                span.clone(),
                wanted
                    .map(|t| format!("t=\"{t}\"").into_bytes())
                    .unwrap_or_default(),
            ),
            None => (
                cell.open_end - 1..cell.open_end - 1,
                wanted
                    .map(|t| format!(" t=\"{t}\"").into_bytes())
                    .unwrap_or_default(),
            ),
        };
        patches.push(Patch { span, replacement });
    }
    if let Some(span) = &cell.inline {
        patches.push(Patch {
            span: span.clone(),
            replacement: Vec::new(),
        });
    }
    let name = child(cell, "v");
    let text = value.text();
    let replacement = if let Some(v) = &cell.value {
        let mut out = if v.empty {
            xml[v.span.start..v.span.end - 2].to_vec()
        } else {
            xml[v.span.start..v.open_end - 1].to_vec()
        };
        if matches!(value, Cache::Empty) {
            out.extend_from_slice(b"/>");
        } else {
            out.push(b'>');
            out.extend_from_slice(text.as_bytes());
            if v.empty {
                out.extend_from_slice(format!("</{}>", v.qualified).as_bytes());
            } else {
                out.extend_from_slice(&xml[v.close_start..v.span.end]);
            }
        }
        out
    } else if matches!(value, Cache::Empty) {
        format!("<{name}/>").into_bytes()
    } else {
        format!("<{name}>{text}</{name}>").into_bytes()
    };
    patches.push(Patch {
        span: cell
            .value
            .as_ref()
            .map(|v| v.span.clone())
            .unwrap_or(cell.formula_end..cell.formula_end),
        replacement,
    });
}
/// A cell child's qualified name, in the cell's namespace prefix.
fn child(cell: &sheet::Cell, local: &str) -> String {
    match cell.qualified.rsplit_once(':') {
        Some((prefix, _)) => format!("{prefix}:{local}"),
        None => local.to_owned(),
    }
}
/// The rich error Excel saves with a cached #SPILL! or #CALC! (see
/// [`rich`]): a #SPILL! counts the additional columns and rows of a dynamic
/// array's result that could not spill into cells that were not empty
/// (`blocked`: its rows and columns). A result past the sheet's edge and an
/// error read from another cell show no spill range, as Excel saves them.
fn rich_error(value: &LiteralValue, blocked: Option<(u32, u32)>) -> Option<rich::RichError> {
    let LiteralValue::Error(error) = value else {
        return None;
    };
    match error.kind {
        ExcelErrorKind::Spill => Some(rich::RichError {
            kind: error.kind,
            offsets: Some(blocked.map_or((0, 0), |(rows, cols)| {
                (cols.saturating_sub(1), rows.saturating_sub(1))
            })),
        }),
        ExcelErrorKind::Calc => Some(rich::RichError {
            kind: error.kind,
            offsets: None,
        }),
        _ => None,
    }
}
/// Excel tags a cached #SPILL! or #CALC! with its rich error (the cell's
/// `vm`) and keeps a cell's tag only while the cell holds that error. Points
/// the cell's tag at a record of `wanted`, adding the attribute or one, or
/// removes it (and the space before it); returns whether the tag changed.
fn retag(
    xml: &[u8],
    cell: &sheet::Cell,
    wanted: Option<rich::RichError>,
    tags: &mut rich::RichTags,
    patches: &mut Vec<Patch>,
) -> bool {
    let current = cell.value_metadata.as_ref();
    if current.map(|(vm, _)| tags.get(*vm)) == wanted.map(Some) {
        return false;
    }
    match (current, wanted) {
        (None, None) => return false,
        (Some((_, span)), None) => {
            let start = span.start - usize::from(xml[span.start - 1].is_ascii_whitespace());
            patches.push(Patch {
                span: start..span.end,
                replacement: Vec::new(),
            });
        }
        (Some((_, span)), Some(error)) => patches.push(Patch {
            span: span.clone(),
            replacement: format!("vm=\"{}\"", tags.tag(error)).into_bytes(),
        }),
        // The cell's start tag ends at `open_end` with `>`.
        (None, Some(error)) => patches.push(Patch {
            span: cell.open_end - 1..cell.open_end - 1,
            replacement: format!(" vm=\"{}\"", tags.tag(error)).into_bytes(),
        }),
    }
    true
}
fn apply_patches(bytes: &[u8], mut patches: Vec<Patch>, limit: usize) -> Result<Vec<u8>, IoError> {
    patches.sort_by_key(|p| p.span.start);
    let mut length = bytes.len();
    let mut previous = 0;
    for patch in &patches {
        if patch.span.start < previous
            || patch.span.end > bytes.len()
            || patch.span.start > patch.span.end
        {
            return Err(unsupported("overlapping cache edit spans", "worksheet"));
        }
        previous = patch.span.end;
        length = length
            .checked_sub(patch.span.len())
            .and_then(|n| n.checked_add(patch.replacement.len()))
            .ok_or_else(|| unsupported("cache output size overflow", "worksheet"))?;
        if length > limit {
            return Err(unsupported("worksheet output byte limit", "worksheet"));
        }
    }
    // One append pass, rather than repeatedly shifting the remainder of XML.
    let mut out = Vec::with_capacity(length);
    previous = 0;
    for patch in patches {
        out.extend_from_slice(&bytes[previous..patch.span.start]);
        out.extend_from_slice(&patch.replacement);
        previous = patch.span.end;
    }
    out.extend_from_slice(&bytes[previous..]);
    Ok(out)
}
struct BoundedOutput {
    cursor: Cursor<Vec<u8>>,
    limit: usize,
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .cursor
            .position()
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > self.limit as u64)
        {
            return Err(std::io::Error::other("XLSX output byte limit"));
        }
        self.cursor.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.cursor.flush()
    }
}
impl Seek for BoundedOutput {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(from)
    }
}

/// Recalculate ordinary/shared formula caches without importing/rewriting rich
/// workbook structures. Unsupported geometry/metadata cases return an error;
/// there is no lossy fallback. Exact no-ops return the original package bytes.
pub fn recalculate_xlsx_bytes(
    bytes: &[u8],
    options: XlsxRecalculateOptions,
) -> Result<XlsxRecalculateResult, IoError> {
    let (mut archive, mut tags) = package::admit(bytes, &options)?;
    let (sheets, date_system, extension, defined_names) =
        package::discover(&mut archive, &options)?;
    let mut plans = Vec::new();
    let mut observed = 0;
    let mut logical_cells = 0u64;
    let mut formula_count = 0usize;
    for sheet in &sheets {
        checkpoint(&options.cancel)?;
        let data = package::read_part(
            &mut archive,
            &sheet.part,
            options.limits.max_worksheet_bytes,
        )?;
        let scan = sheet::scan(&data, &options, &tags, &mut observed, &mut logical_cells)?;
        formula_count = formula_count
            .checked_add(scan.cells.len())
            .ok_or_else(|| unsupported("formula count overflow", "workbook"))?;
        if formula_count > options.limits.max_formula_cells {
            return Err(unsupported("formula cell count limit", "workbook"));
        }
        plans.push((data, scan));
    }
    let empty_result = |summary| XlsxRecalculateResult {
        bytes: bytes.to_vec(),
        summary,
        formula_cells: formula_count,
        cache_cells_changed: 0,
        worksheet_parts_changed: 0,
    };
    if formula_count == 0 {
        checkpoint(&options.cancel)?;
        if bytes.len() > options.limits.max_output_bytes {
            return Err(unsupported("output byte limit", "XLSX package"));
        }
        return Ok(empty_result(RecalculateSummary::default()));
    }
    checkpoint(&options.cancel)?;
    // Calamine 0.36 cannot decode every legal/stale cache representation,
    // although formula ingestion ignores cached results. Clear those caches in a
    // bounded, transient ingestion view; the authoritative package stays intact.
    let mut view_parts = BTreeMap::new();
    for (sheet, (data, scan)) in sheets.iter().zip(&plans) {
        let mut patches = Vec::new();
        // Array members are blanked so the anchor's result can spill.
        for member in &scan.members {
            if member.cell.value.is_some() || member.cell.inline.is_some() {
                cache_patches(data, &member.cell, &Cache::Empty, &mut patches);
            }
        }
        for cell in &scan.cells {
            if !sheet::readable_scalar_cache(cell, data)
                || matches!(cell.kind.as_deref(), Some("s" | "inlineStr" | "d"))
            {
                cache_patches(data, cell, &Cache::Empty, &mut patches);
            }
        }
        if !patches.is_empty() {
            view_parts.insert(
                sheet.part.clone(),
                apply_patches(data, patches, options.limits.max_worksheet_bytes)?,
            );
        }
    }
    let ingest_bytes = if view_parts.is_empty() {
        bytes.to_vec()
    } else {
        package::rewrite(bytes, &mut archive, &view_parts, &[], &options)?
    };
    let opened = if let Some(cancel) = options.cancel.clone() {
        CalamineAdapter::open_bytes_cancellable(ingest_bytes, cancel)
    } else {
        CalamineAdapter::open_bytes(ingest_bytes)
    };
    checkpoint(&options.cancel)?;
    let mut adapter = opened.map_err(IoError::Calamine)?;
    if adapter.sheet_names().map_err(IoError::Calamine)?
        != sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
    {
        return Err(unsupported(
            "adapter/preflight sheet mapping mismatch",
            "workbook",
        ));
    }
    let mut config = options.eval_config.clone();
    config.date_system = date_system;
    // The package was saved to a file, so CELL("filename") names one; a host
    // that knows the file's name sets it in `eval_config` (the file API sets
    // the input file's). Otherwise the placeholder name carries the extension
    // Excel saves this kind of package under: workbook.xlsm for a
    // macro-enabled workbook, workbook.xltx for a template, and so on.
    config
        .workbook_file_name
        .get_or_insert_with(|| format!("workbook.{extension}"));
    // Excel spills any array that fits the grid; the package cell limit is
    // the only bound here (the default 10,000-cell cap refused SEQUENCE(30000)).
    config.spill.max_spill_cells = config
        .spill
        .max_spill_cells
        .max(u32::try_from(options.limits.max_cells).unwrap_or(u32::MAX));
    // Excel finds circular references while it calculates: a formula whose
    // references only look circular (INDEX($K$3:K9,...) picking an earlier
    // row) is not one. The file's calcPr decides whether real ones iterate.
    config.cycle.detection = formualizer_eval::engine::CycleDetection::Runtime;
    if let Some(settings) = crate::traits::SpreadsheetReader::calc_settings(&adapter) {
        config.cycle = crate::calc_pr::apply_calc_settings_to_cycle(&settings, config.cycle);
    }
    // With iterative calculation off (Excel's default), Excel cannot
    // calculate a formula on a real circular reference: it leaves it with
    // its last calculated value, the result this file caches for it.
    if config.cycle.policy == CyclePolicy::Error {
        config.cycle.policy = CyclePolicy::RetainLastValue;
    }
    // Whether the policy came from the file or from the caller, the
    // package's caches are what a retained circular formula keeps.
    let retain_last_values = config.cycle.policy == CyclePolicy::RetainLastValue;
    // XLSX dates are serial caches. Native chrono materialization cannot retain
    // Excel-1900 phantom serial 60 and can discard fractional duration precision.
    config.temporal_egress = formualizer_eval::engine::TemporalEgress::Serial;
    let mut engine: Engine<WBResolver> = Engine::new(WBResolver::default(), config);
    let mut load_limits = engine.workbook_load_limits().clone();
    load_limits.max_sheet_cols = load_limits.max_sheet_cols.min(options.limits.max_columns);
    load_limits.max_sheet_logical_cells = load_limits
        .max_sheet_logical_cells
        .min(options.limits.max_cells as u64);
    load_limits.max_formula_spool_bytes_per_sheet = load_limits
        .max_formula_spool_bytes_per_sheet
        .min(options.limits.max_expanded_bytes as u64);
    load_limits.max_formula_spool_bytes_per_workbook = load_limits
        .max_formula_spool_bytes_per_workbook
        .min(options.limits.max_expanded_bytes as u64);
    engine.set_workbook_load_limits(load_limits);
    define_tables(&mut engine, &sheets)?;
    let ingested = adapter.stream_into_engine(&mut engine);
    checkpoint(&options.cancel)?;
    ingested?;
    drop(adapter);
    checkpoint(&options.cancel)?;
    // A value cell's cached #VALUE! that a rich value tags stands for the
    // tagged error.
    for (sheet, (_, scan)) in sheets.iter().zip(&plans) {
        for &(row, col, kind) in &scan.tagged {
            engine
                .set_cell_value(
                    &sheet.name,
                    row,
                    col,
                    LiteralValue::Error(ExcelError::new(kind)),
                )
                .map_err(IoError::Engine)?;
        }
    }
    // The ingestion view may have cleared a cache Calamine cannot read; the
    // package's own caches are the last calculated values — of an array
    // formula, those of every cell of its extent.
    if retain_last_values {
        for (sheet, (_, scan)) in sheets.iter().zip(&plans) {
            for cell in &scan.cells {
                if let Some(value) = last_calculated_value(cell, &tags) {
                    engine.set_last_calculated_value(&sheet.name, cell.row, cell.col, value);
                }
            }
            for member in &scan.members {
                let Some(anchor) = scan.cells.get(member.anchor) else {
                    continue;
                };
                if let Some(value) = last_calculated_value(&member.cell, &tags) {
                    engine.set_last_calculated_array_member_value(
                        &sheet.name,
                        anchor.row,
                        anchor.col,
                        member.cell.row,
                        member.cell.col,
                        value,
                    );
                }
            }
        }
    }
    // Only array formulas produce arrays; a formula stored without the array
    // flag takes the implicit intersection of an array or range result.
    engine.use_legacy_array_semantics();
    for (sheet, (_, scan)) in sheets.iter().zip(&plans) {
        for cell in &scan.cells {
            if cell.formula_kind != "array" {
                continue;
            }
            let (r1, c1, r2, c2) = cell
                .array_extent
                .unwrap_or((cell.row, cell.col, cell.row, cell.col));
            {
                engine.declare_array_formula(
                    &sheet.name,
                    cell.row,
                    cell.col,
                    r2 - r1 + 1,
                    c2 - c1 + 1,
                    cell.dynamic_array,
                );
            }
        }
    }
    if let Some(cancel) = options.cancel.clone() {
        engine.evaluate_all_cancellable(cancel)?;
    } else {
        engine.evaluate_all()?;
    }
    checkpoint(&options.cancel)?;
    let calc_always = calc_always::calc_always(&engine, &sheets, &plans, &defined_names)?;
    checkpoint(&options.cancel)?;
    // INDIRECT text that names a workbook ('[Book.xlsx]Sheet1'!A1) reads this
    // file under the name it was saved with, or another workbook Excel has
    // open; the package records neither, so its #REF! is not Excel's value.
    if engine.text_named_workbook() {
        return Err(unsupported(
            "INDIRECT text that names a workbook",
            "workbook",
        ));
    }
    let coerced: HashSet<_> = engine
        .formula_parse_diagnostics()
        .iter()
        .filter(|d| d.policy == FormulaParsePolicy::CoerceToError)
        .map(|d| (d.sheet.clone(), d.row, d.col))
        .collect();
    let mut summary = RecalculateSummary::default();
    let mut changed = 0usize;
    let mut replacements = BTreeMap::new();
    let mut expanded = usize::try_from(
        archive
            .decompressed_size()
            .ok_or_else(|| unsupported("ZIP expanded-size overflow", "workbook"))?,
    )
    .map_err(|_| unsupported("ZIP expanded-size overflow", "workbook"))?;
    for (s, (sheet, (data, scan))) in sheets.iter().zip(plans).enumerate() {
        let mut patches = Vec::new();
        // Evaluated array extent (rows, cols) per anchor index.
        let mut array_results: BTreeMap<usize, (u32, u32)> = BTreeMap::new();
        for (index, cell) in scan.cells.iter().enumerate() {
            checkpoint(&options.cancel)?;
            let address = CellAddress::new(&sheet.name, cell.row, cell.col)
                .map_err(|e| IoError::from_backend("xlsx-coordinate", e))?;
            let snapshot = engine
                .inspect_cell(&address, &SnapshotOptions::default())
                .map_err(|e| IoError::from_backend("xlsx-inspect", e))?
                .cell;
            use formualizer_eval::engine::inspect::SpillRole;
            let spilled = match &snapshot.spill {
                None => (1, 1),
                Some(SpillRole::Anchor { extent }) => (
                    extent.end_row - extent.start_row + 1,
                    extent.end_col - extent.start_col + 1,
                ),
                Some(_) => (u32::MAX, u32::MAX),
            };
            // A spilled result must stay inside the array extent recorded
            // in the file, whose member cells receive the values.
            let fits = match cell.array_extent {
                Some((r1, c1, r2, c2)) => spilled.0 <= r2 - r1 + 1 && spilled.1 <= c2 - c1 + 1,
                None => spilled == (1, 1),
            };
            if !fits {
                return Err(unsupported(
                    "materialized multi-cell dynamic spill",
                    &sheet.name,
                ));
            }
            if cell.array_extent.is_some() {
                array_results.insert(index, spilled);
            }
            let mut value = snapshot
                .value
                .ok_or_else(|| unsupported("absent formula result", &sheet.name))?;
            if matches!(&value,LiteralValue::Array(rows) if rows.len()==1 && rows[0].len()==1) {
                value = value
                    .coerce_to_single_value()
                    .map_err(|_| unsupported("non-scalar result", "cache-only writer"))?;
            }
            if snapshot.formula.is_none()
                && !(matches!(value, LiteralValue::Error(_))
                    && coerced.contains(&(sheet.name.clone(), cell.row, cell.col)))
            {
                return Err(unsupported("source formula was not ingested", &sheet.name));
            }
            // Volatile formulas (TODAY, NOW, RAND, ...) and the formulas that
            // read them stay dirty by design; evaluate_all has just computed
            // them for this pass.
            if snapshot.staleness != Staleness::Current
                && !(snapshot.staleness == Staleness::Dirty
                    && (snapshot.volatile || engine.recomputes_each_recalc(&address)))
            {
                return Err(unsupported(
                    "formula result is not current",
                    format!("{}!{} ({:?})", sheet.name, cell.address, snapshot.staleness),
                ));
            }
            let stats = summary.sheets.entry(sheet.name.clone()).or_default();
            stats.evaluated += 1;
            summary.evaluated += 1;
            if let LiteralValue::Error(error) = &value {
                summary.errors += 1;
                stats.errors += 1;
                let errors = summary
                    .error_summary
                    .entry(error.kind.to_string())
                    .or_default();
                errors.count += 1;
                if errors.locations.len() < options.error_location_limit {
                    errors
                        .locations
                        .push(format!("{}!{}", sheet.name, cell.address));
                } else {
                    errors.locations_truncated += 1;
                }
            }
            // A formula left uncalculated on a circular reference keeps its
            // last calculated value with the rich error that tags it (the
            // spill range of a #SPILL!).
            let kept = engine
                .kept_last_calculated_value(&sheet.name, cell.row, cell.col)
                .then(|| cell.value_metadata.as_ref())
                .flatten()
                .and_then(|(vm, _)| tags.get(*vm))
                .filter(|tag| matches!(&value, LiteralValue::Error(e) if e.kind == tag.kind));
            let wanted = kept.or_else(|| {
                rich_error(
                    &value,
                    engine.blocked_spill_extent(&sheet.name, cell.row, cell.col),
                )
            });
            let cache = Cache::from_value(value, engine.config.date_system)?;
            let stale = !cache.matches(cell);
            if stale {
                cache_patches(&data, cell, &cache, &mut patches);
            }
            let retagged = retag(&data, cell, wanted, &mut tags, &mut patches);
            // A flag is not a cache: only patched values (and a value's
            // rich-error tag) count.
            changed += usize::from(stale || retagged);
            if calc_always.cells[s][index] && !cell.calc_always {
                patches.push(calc_always_patch(cell));
            }
        }
        changed += array_member_patches(
            &engine,
            sheet,
            &data,
            &scan,
            &array_results,
            &calc_always.members[s],
            &mut tags,
            &mut patches,
        )?;
        if !patches.is_empty() {
            let patched = apply_patches(&data, patches, options.limits.max_worksheet_bytes)?;
            expanded = expanded
                .checked_sub(data.len())
                .and_then(|n| n.checked_add(patched.len()))
                .ok_or_else(|| unsupported("expanded output overflow", "workbook"))?;
            if expanded > options.limits.max_expanded_bytes {
                return Err(unsupported("expanded output byte limit", "workbook"));
            }
            replacements.insert(sheet.part.clone(), patched);
        }
    }
    let worksheet_parts_changed = replacements.len();
    // The rich values of the errors newly tagged.
    let edits = tags.finish(&mut archive, &options)?;
    for (part, patched) in edits.replaced {
        let saved = archive
            .by_name(&part)
            .map_err(|e| IoError::from_backend("zip", e))?
            .size();
        expanded = usize::try_from(saved)
            .ok()
            .and_then(|saved| expanded.checked_sub(saved))
            .and_then(|n| n.checked_add(patched.len()))
            .ok_or_else(|| unsupported("expanded output overflow", "workbook"))?;
        replacements.insert(part, patched);
    }
    for (_, added) in &edits.added {
        expanded = expanded
            .checked_add(added.len())
            .ok_or_else(|| unsupported("expanded output overflow", "workbook"))?;
    }
    if expanded > options.limits.max_expanded_bytes {
        return Err(unsupported("expanded output byte limit", "workbook"));
    }
    summary.status = if summary.errors == 0 {
        RecalculateStatus::Success
    } else {
        RecalculateStatus::ErrorsFound
    };
    checkpoint(&options.cancel)?;
    if replacements.is_empty() && edits.added.is_empty() {
        if bytes.len() > options.limits.max_output_bytes {
            return Err(unsupported("output byte limit", "XLSX package"));
        }
        return Ok(empty_result(summary));
    }
    let output = package::rewrite(bytes, &mut archive, &replacements, &edits.added, &options)?;
    checkpoint(&options.cancel)?;
    Ok(XlsxRecalculateResult {
        bytes: output,
        summary,
        formula_cells: formula_count,
        cache_cells_changed: changed,
        worksheet_parts_changed,
    })
}

/// Flag a formula calculated always. Excel writes `ca="1"` after the
/// formula's type and extent and before its shared-formula index.
fn calc_always_patch(cell: &sheet::Cell) -> Patch {
    let (span, replacement): (_, &[u8]) = match (&cell.calc_always_attr, cell.shared_index_attr) {
        (Some(span), _) => (span.clone(), b"ca=\"1\""),
        (None, Some(at)) => (at..at, b"ca=\"1\" "),
        (None, None) => (cell.formula_attrs_end..cell.formula_attrs_end, b" ca=\"1\""),
    };
    Patch {
        span,
        replacement: replacement.to_vec(),
    }
}

/// Write evaluated array results into the member caches of each multi-cell
/// array formula. A dynamic array's members outside its current spill are
/// blank. A legacy (CSE) array fills its whole extent: a one-row or
/// one-column result repeats and positions beyond the result are #N/A.
/// Excel writes an empty `<f ca="1"/>` before the cache of each member that
/// is calculated always (`flags`, see [`calc_always`]).
fn array_member_patches(
    engine: &Engine<WBResolver>,
    sheet: &package::Sheet,
    data: &[u8],
    scan: &sheet::Scan,
    results: &BTreeMap<usize, (u32, u32)>,
    flags: &[bool],
    tags: &mut rich::RichTags,
    patches: &mut Vec<Patch>,
) -> Result<usize, IoError> {
    let mut changed = 0;
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    let value_at = |index: usize, row: u32, col: u32| -> LiteralValue {
        let anchor = &scan.cells[index];
        let (r1, c1, _, _) = anchor.array_extent.expect("array anchor");
        let (height, width) = results[&index];
        let (i, j) = (row - r1, col - c1);
        let (i, j) = if anchor.dynamic_array {
            (i, j)
        } else {
            (
                if height == 1 { 0 } else { i },
                if width == 1 { 0 } else { j },
            )
        };
        if i >= height || j >= width {
            return if anchor.dynamic_array {
                LiteralValue::Empty
            } else {
                LiteralValue::Error(formualizer_common::ExcelError::new(
                    formualizer_common::ExcelErrorKind::Na,
                ))
            };
        }
        match engine.get_cell_value(&sheet.name, r1 + i, c1 + j) {
            Some(LiteralValue::Array(rows)) if rows.len() == 1 && rows[0].len() == 1 => {
                rows[0][0].clone()
            }
            value => value.unwrap_or(LiteralValue::Empty),
        }
    };
    for (member, &flag) in scan.members.iter().zip(flags) {
        seen.insert((member.cell.row, member.cell.col));
        let value = value_at(member.anchor, member.cell.row, member.cell.col);
        let wanted = rich_error(&value, None);
        let cache = Cache::from_value(value, engine.config.date_system)?;
        let mark = flag && !member.marked;
        if mark {
            // <f> is the cell's first child; it precedes an inserted <v>.
            let open = member.cell.open_end;
            patches.push(Patch {
                span: open..open,
                replacement: format!("<{} ca=\"1\"/>", child(&member.cell, "f")).into_bytes(),
            });
        }
        let stale = !cache.matches(&member.cell);
        if stale {
            cache_patches(data, &member.cell, &cache, patches);
        }
        let retagged = retag(data, &member.cell, wanted, tags, patches);
        // A marker alone is not a cache: only patched values (and a value's
        // rich-error tag) count.
        changed += usize::from(stale || retagged);
    }
    // Positions without a cell element cannot receive a value.
    for &index in results.keys() {
        let (r1, c1, r2, c2) = scan.cells[index].array_extent.expect("array anchor");
        for row in r1..=r2 {
            for col in c1..=c2 {
                if (row, col) == (r1, c1) || seen.contains(&(row, col)) {
                    continue;
                }
                if !matches!(value_at(index, row, col), LiteralValue::Empty) {
                    return Err(unsupported("array result cell is absent", &sheet.name));
                }
            }
        }
    }
    Ok(changed)
}

/// Register worksheet tables before formula ingestion so structured
/// references resolve (sheet registration is idempotent for the adapter).
fn define_tables(
    engine: &mut Engine<WBResolver>,
    sheets: &[package::Sheet],
) -> Result<(), IoError> {
    use formualizer_eval::reference::{CellRef, Coord, RangeRef};
    if sheets.iter().all(|sheet| sheet.tables.is_empty()) {
        return Ok(());
    }
    engine
        .adopt_file_sheets(sheets.iter().map(|sheet| sheet.name.as_str()))
        .map_err(IoError::Engine)?;
    for sheet in sheets {
        let sheet_id = engine
            .sheet_id(&sheet.name)
            .ok_or_else(|| unsupported("table sheet was not registered", &sheet.name))?;
        for table in &sheet.tables {
            let (r1, c1, r2, c2) = table.area;
            let range = RangeRef::new(
                CellRef::new(sheet_id, Coord::from_excel(r1, c1, true, true)),
                CellRef::new(sheet_id, Coord::from_excel(r2, c2, true, true)),
            );
            engine
                .define_table(
                    &table.name,
                    range,
                    table.header_row,
                    table.columns.clone(),
                    table.totals_row,
                )
                .map_err(IoError::Engine)?;
        }
    }
    Ok(())
}

/// Native bounded snapshot + same-directory temporary + atomic replace. This
/// is not CAS against unrelated writers; callers retain their source authority.
/// Symlink destinations are rejected. No failure/cancellation publishes bytes.
/// CELL("filename") names the input file in its folder
/// (`/data/[Budget.xlsm]Sheet1`) unless `eval_config.workbook_file_name`
/// gives another name (with `workbook_directory` its folder, if any).
#[cfg(not(target_arch = "wasm32"))]
pub fn recalculate_xlsx_file(
    input: &Path,
    output: Option<&Path>,
    mut options: XlsxRecalculateOptions,
) -> Result<XlsxRecalculateResult, IoError> {
    checkpoint(&options.cancel)?;
    if options.eval_config.workbook_file_name.is_none() {
        options.eval_config.workbook_file_name = input
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        // The absolute folder with its trailing separator, as Excel prints
        // the folder it opened the file from.
        options.eval_config.workbook_directory = std::path::absolute(input)
            .ok()
            .and_then(|path| Some(path.parent()?.to_str()?.to_owned()))
            .map(|mut folder| {
                if !folder.ends_with(std::path::MAIN_SEPARATOR) {
                    folder.push(std::path::MAIN_SEPARATOR);
                }
                folder
            });
    }
    let mut source = Vec::new();
    std::fs::File::open(input)?
        .take((options.limits.max_input_bytes as u64).saturating_add(1))
        .read_to_end(&mut source)?;
    let result = recalculate_xlsx_bytes(&source, options.clone())?;
    let dest = output.unwrap_or(input);
    let metadata = match std::fs::symlink_metadata(dest) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if metadata
        .as_ref()
        .is_some_and(|m| m.file_type().is_symlink())
    {
        return Err(unsupported("symlink destination", "atomic XLSX output"));
    }
    checkpoint(&options.cancel)?;
    if dest == input && result.bytes == source {
        return Ok(result);
    }
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    for chunk in result.bytes.chunks(64 * 1024) {
        checkpoint(&options.cancel)?;
        temp.write_all(chunk)?;
    }
    if let Some(metadata) = metadata {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.as_file().sync_all()?;
    checkpoint(&options.cancel)?;
    temp.persist(dest).map_err(|e| IoError::Io(e.error))?;
    Ok(result)
}
