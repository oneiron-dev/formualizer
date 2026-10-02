//! Shared helpers for lookup-family functions (MATCH, VLOOKUP, HLOOKUP, XLOOKUP)
//! Provides unified coercion, comparison and approximate-mode selection logic.

use crate::engine::{DateSystem, range_view::RangeView};
use arrow_array::Array;
use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use formualizer_parse::parser::{ExternalRefKind, ReferenceType};

/// Coerce a value to f64 with Excel-like rules for numeric comparisons:
/// - Number / Int: numeric
/// - Text: parsed if it looks numeric (lenient)
/// - Boolean: TRUE=1, FALSE=0
/// - Date / DateTime / Time: the Excel serial number. Excel has no date type
///   at the formula level -- a date cell holds a serial carrying a date number
///   format -- so a temporal value must order against plain numerics and
///   against other temporal values exactly as two numbers do.
/// - Empty: treated as 0
pub fn value_to_f64_lenient(v: &LiteralValue, date_system: DateSystem) -> Option<f64> {
    match v {
        LiteralValue::Number(n) => Some(*n),
        LiteralValue::Int(i) => Some(*i as f64),
        LiteralValue::Text(s) => s.parse::<f64>().ok(),
        LiteralValue::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        LiteralValue::Date(_) | LiteralValue::DateTime(_) | LiteralValue::Time(_) => {
            v.as_serial_number_for(date_system)
        }
        LiteralValue::Empty => Some(0.0),
        _ => None,
    }
}

/// Case-insensitive text equality (no wildcards).
pub fn text_equal_ci(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Compare two values for ordering using lenient numeric coercion first, fallback to case-insensitive text.
/// Returns Some(ordering) where ordering <0, 0, >0 similar to cmp, or None if incomparable.
pub fn cmp_for_lookup(a: &LiteralValue, b: &LiteralValue, date_system: DateSystem) -> Option<i32> {
    if let (Some(x), Some(y)) = (
        value_to_f64_lenient(a, date_system),
        value_to_f64_lenient(b, date_system),
    ) {
        if (x - y).abs() < 1e-12 {
            return Some(0);
        }
        return Some(if x < y { -1 } else { 1 });
    }
    match (a, b) {
        (LiteralValue::Text(x), LiteralValue::Text(y)) => {
            let xl = x.to_lowercase();
            let yl = y.to_lowercase();
            Some(match xl.cmp(&yl) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            })
        }
        (LiteralValue::Boolean(x), LiteralValue::Boolean(y)) => {
            let xv = if *x { 1 } else { 0 };
            let yv = if *y { 1 } else { 0 };
            Some(xv.cmp(&yv) as i32)
        }
        _ => None,
    }
}

enum PreparedTextMatcher {
    Exact { folded_needle: String },
    Wildcard { compiled: CompiledWildcardPattern },
}

pub(crate) struct PreparedLookupMatcher<'a> {
    needle: &'a LiteralValue,
    text: Option<PreparedTextMatcher>,
    date_system: DateSystem,
}

fn is_numeric_exact_value(value: &LiteralValue) -> bool {
    matches!(
        value,
        LiteralValue::Number(_)
            | LiteralValue::Int(_)
            | LiteralValue::Date(_)
            | LiteralValue::DateTime(_)
            | LiteralValue::Time(_)
    )
}

impl<'a> PreparedLookupMatcher<'a> {
    pub(crate) fn new(needle: &'a LiteralValue, wildcard: bool, date_system: DateSystem) -> Self {
        let text = match needle {
            LiteralValue::Text(s) => {
                let folded = s.to_lowercase();
                if wildcard && (s.contains('*') || s.contains('?') || s.contains('~')) {
                    Some(PreparedTextMatcher::Wildcard {
                        compiled: CompiledWildcardPattern::from_folded(&folded),
                    })
                } else {
                    Some(PreparedTextMatcher::Exact {
                        folded_needle: folded,
                    })
                }
            }
            _ => None,
        };
        Self {
            needle,
            text,
            date_system,
        }
    }

    pub(crate) fn matches(&self, candidate: &LiteralValue) -> bool {
        // Exact lookup eligibility is asymmetric: blank needles still coerce
        // to zero, but a blank range entry can never satisfy a match (#319).
        // Numeric and temporal needles share one candidate class; unlike the
        // lenient comparison helper, they do not admit boolean or text values.
        if matches!(candidate, LiteralValue::Empty) {
            return false;
        }
        if matches!(self.needle, LiteralValue::Empty) || is_numeric_exact_value(self.needle) {
            return is_numeric_exact_value(candidate)
                && cmp_for_lookup(self.needle, candidate, self.date_system) == Some(0);
        }
        match (&self.text, candidate) {
            (
                Some(PreparedTextMatcher::Exact { folded_needle }),
                LiteralValue::Text(candidate_text),
            ) => candidate_text.to_lowercase() == *folded_needle,
            (
                Some(PreparedTextMatcher::Wildcard { compiled }),
                LiteralValue::Text(candidate_text),
            ) => {
                let folded_candidate = candidate_text.to_lowercase();
                compiled.matches_folded(&folded_candidate)
            }
            // Excel exact lookups never match a text needle against a
            // non-text candidate: "20" does not find the number 20.
            (Some(_), _) => false,
            _ => cmp_for_lookup(self.needle, candidate, self.date_system)
                .map(|o| o == 0)
                .unwrap_or(false),
        }
    }
}

/// Exact equality leveraging cmp_for_lookup plus wildcard option (pattern side may have * or ?).
pub fn equals_maybe_wildcard(
    pattern: &LiteralValue,
    candidate: &LiteralValue,
    wildcard: bool,
    date_system: DateSystem,
) -> bool {
    PreparedLookupMatcher::new(pattern, wildcard, date_system).matches(candidate)
}

/// Compare a lookup-vector entry with the lookup value in an approximate
/// lookup, or `None` when Excel's search skips the entry.
///
/// Excel compares a lookup value only with entries of its own type: numbers
/// (dates and times are serial numbers), text, or logicals. A logical is not
/// the number 0 or 1 here and numeric-looking text is not a number, so a
/// numeric search skips both, as it skips blanks and errors. A blank lookup
/// value searches as the number 0.
pub fn cmp_for_approximate(
    value: &LiteralValue,
    needle: &LiteralValue,
    date_system: DateSystem,
) -> Option<i32> {
    match (value, needle) {
        (LiteralValue::Text(a), LiteralValue::Text(b)) => {
            Some(a.to_lowercase().cmp(&b.to_lowercase()) as i32)
        }
        (LiteralValue::Boolean(a), LiteralValue::Boolean(b)) => Some(a.cmp(b) as i32),
        (v, n) if is_numeric_exact_value(v) && searches_numbers(n) => {
            cmp_for_lookup(v, n, date_system)
        }
        _ => None,
    }
}

/// Whether an approximate lookup for `needle` searches the numbers (a blank
/// lookup value searches as the number 0).
pub fn searches_numbers(needle: &LiteralValue) -> bool {
    is_numeric_exact_value(needle) || matches!(needle, LiteralValue::Empty)
}

/// Whether Excel's approximate search visits `value` when looking for `needle`.
///
/// The legacy approximate lookups (`MATCH` with `match_type` 1/-1,
/// `VLOOKUP`/`HLOOKUP` with `range_lookup` TRUE, `LOOKUP`) consider only
/// entries of the needle's type (see [`cmp_for_approximate`]). A blank cell,
/// error cell, or entry of another type such as a text header above a column
/// of numbers or a FALSE left by `IF` is skipped: it is neither out-of-order
/// data nor a matchable position.
pub fn is_searchable_for_approximate(
    value: &LiteralValue,
    needle: &LiteralValue,
    date_system: DateSystem,
) -> bool {
    cmp_for_approximate(value, needle, date_system).is_some()
}

/// A lookup vector projected onto the entries an approximate search visits.
///
/// Positions are only materialized when something is actually skipped, so the
/// common case — a vector that is entirely in the needle's class — borrows the
/// original slice and allocates nothing. Indices returned by a search over this
/// projection are mapped back with [`SearchedVector::original_position`],
/// because Excel counts the answer from the top of the *original* range.
pub struct SearchedVector<'a> {
    values: &'a [LiteralValue],
    positions: Option<Vec<usize>>,
    date_system: DateSystem,
}

impl<'a> SearchedVector<'a> {
    pub fn new(
        values: &'a [LiteralValue],
        needle: &LiteralValue,
        date_system: DateSystem,
    ) -> Result<Self, ExcelError> {
        let first_skipped = values
            .iter()
            .position(|v| !is_searchable_for_approximate(v, needle, date_system));
        let positions = first_skipped.map(|skip| {
            let mut positions: Vec<usize> = (0..skip).collect();
            positions.extend(
                ((skip + 1)..values.len())
                    .filter(|&i| is_searchable_for_approximate(&values[i], needle, date_system)),
            );
            positions
        });
        Ok(Self {
            values,
            positions,
            date_system,
        })
    }

    pub fn len(&self) -> usize {
        match &self.positions {
            Some(positions) => positions.len(),
            None => self.values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> &'a LiteralValue {
        &self.values[self.original_position(index)]
    }

    /// Map an index in this projection back to its row in the original range.
    pub fn original_position(&self, index: usize) -> usize {
        match &self.positions {
            Some(positions) => positions[index],
            None => index,
        }
    }

    /// The first searched entry at or after `position` in the original
    /// vector, as an index into this projection.
    fn first_at_or_after(&self, position: usize) -> Option<usize> {
        let index = match &self.positions {
            Some(positions) => positions.partition_point(|&p| p < position),
            None => position,
        };
        (index < self.len()).then_some(index)
    }
}

/// Excel's approximate search (`MATCH` with `match_type` 1/-1,
/// `VLOOKUP`/`HLOOKUP` with `range_lookup` TRUE) over a lookup vector of
/// `len` cells, of which `searched` holds the materialized prefix; cells past
/// it (the unused tail of a whole-column reference) are blank. Returns the
/// position in the original vector.
///
/// Excel bisects the reference as written, with inclusive bounds and a floor
/// midpoint, and never checks that the data is sorted: on unsorted data the
/// answer is wherever the probes lead, possibly `#N/A`. A probe that lands on
/// an entry the search skips (see [`cmp_for_approximate`]) moves forward to
/// the next searched entry; when there is none up to the upper bound, the
/// search continues in the lower half. An exact hit ends the bisection:
/// Excel walks on from it through the run of equal entries next to it and
/// returns the run's last entry for an ascending search, its first for a
/// descending one, so `MATCH(1,{1,1,1,1,1,0,1},1)` is 5 (the first probe hits
/// position 4, the run ends at the 0) and not the 1 beyond it. Otherwise the
/// answer is the last probed entry below (ascending) or above (descending)
/// the lookup value.
pub fn excel_approximate_search(
    searched: &SearchedVector<'_>,
    len: usize,
    needle: &LiteralValue,
    descending: bool,
) -> Option<usize> {
    let len = len.max(searched.values.len());
    if searched.is_empty() || len == 0 {
        return None;
    }
    let (mut lo, mut hi) = (0usize, len - 1);
    let mut nearest = None;
    let equal = |index: usize| {
        cmp_for_approximate(searched.get(index), needle, searched.date_system) == Some(0)
    };
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let probe = searched
            .first_at_or_after(mid)
            .map(|index| (index, searched.original_position(index)))
            .filter(|&(_, position)| position <= hi);
        // Nothing searched from the midpoint to the upper bound: go left.
        let Some((index, position)) = probe else {
            if mid == 0 {
                break;
            }
            hi = mid - 1;
            continue;
        };
        let c = cmp_for_approximate(searched.get(index), needle, searched.date_system)
            .expect("SearchedVector contains only entries comparable with the lookup value");
        if c == 0 {
            // Walk the run of equal entries (skipped entries are passed over)
            // to its last entry, or its first for a descending search.
            let mut end = index;
            if descending {
                while end > 0 && equal(end - 1) {
                    end -= 1;
                }
            } else {
                while end + 1 < searched.len() && equal(end + 1) {
                    end += 1;
                }
            }
            return Some(searched.original_position(end));
        }
        if (c < 0) != descending {
            nearest = Some(position);
            lo = position + 1;
        } else if mid == 0 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    nearest
}

/// The rows and columns a reference spans as written. A whole column or row
/// (`A:A`, `1:1`) reaches the sheet edge, although the range view resolved
/// from it stops at the last used cell. `None` for references whose extent is
/// only known once resolved (names, tables, 3D references).
pub(crate) fn reference_extent(reference: &ReferenceType) -> Option<(usize, usize)> {
    const MAX_ROWS: u32 = 1_048_576;
    const MAX_COLS: u32 = 16_384;
    let span = |start: Option<u32>, end: Option<u32>, max: u32| {
        let (start, end) = (start.unwrap_or(1), end.unwrap_or(max));
        (start.max(end) - start.min(end) + 1) as usize
    };
    match reference {
        ReferenceType::Cell { .. } => Some((1, 1)),
        ReferenceType::Range {
            start_row,
            start_col,
            end_row,
            end_col,
            ..
        } => Some((
            span(*start_row, *end_row, MAX_ROWS),
            span(*start_col, *end_col, MAX_COLS),
        )),
        ReferenceType::External(ext) => match &ext.kind {
            ExternalRefKind::Cell { .. } => Some((1, 1)),
            ExternalRefKind::Range {
                start_row,
                start_col,
                end_row,
                end_col,
                ..
            } => Some((
                span(*start_row, *end_row, MAX_ROWS),
                span(*start_col, *end_col, MAX_COLS),
            )),
        },
        _ => None,
    }
}

/// Detect ascending sort (strict or equal allowed) for slice according to cmp_for_lookup.
pub fn is_sorted_ascending(values: &[LiteralValue], date_system: DateSystem) -> bool {
    values
        .windows(2)
        .all(|w| cmp_for_lookup(&w[0], &w[1], date_system).is_some_and(|c| c <= 0))
}

/// Detect descending sort (strict or equal allowed).
pub fn is_sorted_descending(values: &[LiteralValue], date_system: DateSystem) -> bool {
    values
        .windows(2)
        .all(|w| cmp_for_lookup(&w[0], &w[1], date_system).is_some_and(|c| c >= 0))
}

/// Approximate mode selection (ascending):
/// match_mode 1 -> largest <= needle
/// match_mode -1 -> smallest >= needle (Excel MATCH uses -1 for descending; we adapt for XLOOKUP semantics)
pub fn approximate_select_ascending(
    values: &[LiteralValue],
    needle: &LiteralValue,
    mode: i32,
    date_system: DateSystem,
) -> Option<usize> {
    if values.is_empty() {
        return None;
    }
    let needle_num = value_to_f64_lenient(needle, date_system);
    match mode {
        -1 => {
            // exact or next smaller (our XLOOKUP -1 semantics) -> largest <= needle
            let mut best: Option<usize> = None;
            for (i, v) in values.iter().enumerate() {
                if cmp_for_lookup(v, needle, date_system)
                    .map(|c| c == 0)
                    .unwrap_or(false)
                {
                    return Some(i);
                }
                if let (Some(nn), Some(vv)) = (needle_num, value_to_f64_lenient(v, date_system))
                    && vv <= nn
                    && best.is_none_or(|b| {
                        value_to_f64_lenient(&values[b], date_system).unwrap_or(f64::NEG_INFINITY)
                            < vv
                    })
                {
                    best = Some(i);
                }
            }
            best
        }
        1 => {
            // exact or next larger -> smallest >= needle
            let mut best: Option<usize> = None;
            for (i, v) in values.iter().enumerate() {
                if cmp_for_lookup(v, needle, date_system)
                    .map(|c| c == 0)
                    .unwrap_or(false)
                {
                    return Some(i);
                }
                if let (Some(nn), Some(vv)) = (needle_num, value_to_f64_lenient(v, date_system))
                    && vv >= nn
                    && best.is_none_or(|b| {
                        value_to_f64_lenient(&values[b], date_system).unwrap_or(f64::INFINITY) > vv
                    })
                {
                    best = Some(i);
                }
            }
            best
        }
        _ => None,
    }
}

/// Validate ascending sort for approximate selection; return #N/A if unsorted.
pub fn guard_sorted_ascending(
    values: &[LiteralValue],
    date_system: DateSystem,
) -> Result<(), ExcelError> {
    if !is_sorted_ascending(values, date_system) {
        return Err(ExcelError::new(ExcelErrorKind::Na));
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum WildcardToken {
    AnySeq,
    AnyChar,
    Lit(Box<[char]>),
}

#[derive(Clone, Debug)]
struct CompiledWildcardPattern {
    tokens: Vec<WildcardToken>,
}

impl CompiledWildcardPattern {
    fn from_folded(pattern: &str) -> Self {
        let mut tokens: Vec<WildcardToken> = Vec::new();
        let mut lit = String::new();
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '~' => {
                    if let Some(next) = chars.next() {
                        lit.push(next);
                    } else {
                        lit.push('~');
                    }
                }
                '*' => {
                    if !lit.is_empty() {
                        tokens.push(WildcardToken::Lit(
                            lit.chars().collect::<Vec<_>>().into_boxed_slice(),
                        ));
                        lit.clear();
                    }
                    tokens.push(WildcardToken::AnySeq);
                }
                '?' => {
                    if !lit.is_empty() {
                        tokens.push(WildcardToken::Lit(
                            lit.chars().collect::<Vec<_>>().into_boxed_slice(),
                        ));
                        lit.clear();
                    }
                    tokens.push(WildcardToken::AnyChar);
                }
                _ => lit.push(ch),
            }
        }
        if !lit.is_empty() {
            tokens.push(WildcardToken::Lit(
                lit.chars().collect::<Vec<_>>().into_boxed_slice(),
            ));
        }

        let mut compact: Vec<WildcardToken> = Vec::new();
        for t in tokens {
            match t {
                WildcardToken::AnySeq => {
                    if !matches!(compact.last(), Some(WildcardToken::AnySeq)) {
                        compact.push(t);
                    }
                }
                _ => compact.push(t),
            }
        }

        Self { tokens: compact }
    }

    fn matches_folded(&self, text: &str) -> bool {
        let text_chars: Vec<char> = text.chars().collect();
        self.matches_folded_chars(&text_chars)
    }

    fn matches_folded_chars(&self, text: &[char]) -> bool {
        let mut ti = 0usize;
        let mut si = 0usize;
        let mut star_retry: Option<(usize, usize)> = None;
        loop {
            if ti == self.tokens.len() {
                if si == text.len() {
                    return true;
                }
            } else {
                match &self.tokens[ti] {
                    WildcardToken::AnySeq => {
                        ti += 1;
                        star_retry = Some((ti, si));
                        continue;
                    }
                    WildcardToken::AnyChar => {
                        if si < text.len() {
                            ti += 1;
                            si += 1;
                            continue;
                        }
                    }
                    WildcardToken::Lit(lit) => {
                        let ll = lit.len();
                        if si + ll <= text.len() && &text[si..si + ll] == lit.as_ref() {
                            ti += 1;
                            si += ll;
                            continue;
                        }
                    }
                }
            }
            if let Some((resume_ti, consumed_to)) = star_retry.as_mut()
                && *consumed_to < text.len()
            {
                *consumed_to += 1;
                ti = *resume_ti;
                si = *consumed_to;
                continue;
            }
            return false;
        }
    }
}

/// Excel-style wildcard pattern matcher with escape (~) supporting *, ? and literal escaping of ~ * ?
pub fn wildcard_pattern_match(pattern: &str, text: &str) -> bool {
    let pattern_folded = pattern.to_lowercase();
    let text_folded = text.to_lowercase();
    let compiled = CompiledWildcardPattern::from_folded(&pattern_folded);
    compiled.matches_folded(&text_folded)
}

/// Find index of exact (or wildcard) match in values; returns first match (Excel semantics).
pub fn find_exact_index(
    values: &[LiteralValue],
    needle: &LiteralValue,
    wildcard: bool,
    date_system: DateSystem,
) -> Option<usize> {
    let matcher = PreparedLookupMatcher::new(needle, wildcard, date_system);
    for (i, v) in values.iter().enumerate() {
        if matcher.matches(v) {
            return Some(i);
        }
    }
    None
}

/// Find index of exact (or wildcard) match in a 1D RangeView; returns first match (Excel semantics).
/// Supports both single-column (vertical) and single-row (horizontal) views.
pub fn find_exact_index_in_view(
    view: &RangeView<'_>,
    needle: &LiteralValue,
    wildcard: bool,
    date_system: DateSystem,
) -> Result<Option<usize>, ExcelError> {
    let (rows, cols) = view.dims();
    let vertical = if cols == 1 {
        true
    } else if rows == 1 {
        false
    } else {
        // Not a 1D range
        return Ok(None);
    };

    match needle {
        LiteralValue::Number(n) => find_exact_number_in_view(view, *n, vertical),
        LiteralValue::Int(i) => find_exact_number_in_view(view, *i as f64, vertical),
        LiteralValue::Text(s) => find_exact_text_in_view(view, s, wildcard, vertical),
        LiteralValue::Boolean(b) => find_exact_boolean_in_view(view, *b, vertical),
        LiteralValue::Empty => find_exact_number_in_view(view, 0.0, vertical),
        LiteralValue::Error(e) => Err(e.clone()),
        // A temporal needle searches the numeric lane by serial: the arrow
        // store keeps dates and times as serials under a temporal type tag,
        // so an exact lookup for a date must not fall through to "no match".
        LiteralValue::Date(_) | LiteralValue::DateTime(_) | LiteralValue::Time(_) => {
            match needle.as_serial_number_for(date_system) {
                Some(serial) => find_exact_number_in_view(view, serial, vertical),
                None => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

fn find_exact_number_in_view(
    view: &RangeView<'_>,
    n: f64,
    vertical: bool,
) -> Result<Option<usize>, ExcelError> {
    if vertical {
        for res in view.numbers_slices() {
            let (row_start, _row_len, cols) = res?;
            if !cols.is_empty() {
                let arr = &cols[0];
                for i in 0..arr.len() {
                    if !arr.is_null(i) && (arr.value(i) - n).abs() < 1e-12 {
                        return Ok(Some(row_start + i));
                    }
                }
            }
        }
    } else {
        // Horizontal: check columns in the first row segment
        for res in view.numbers_slices() {
            let (_row_start, _row_len, cols) = res?;
            for (c, arr) in cols.iter().enumerate() {
                if !arr.is_null(0) && (arr.value(0) - n).abs() < 1e-12 {
                    return Ok(Some(c));
                }
            }
        }
    }

    // Excel exact-match semantics: blank cells are NOT equal to numeric
    // zero. MATCH(0, {blank,1,2}, 0) returns #N/A, not a position. (#319)

    Ok(None)
}

fn find_exact_text_in_view(
    view: &RangeView<'_>,
    s: &str,
    wildcard: bool,
    vertical: bool,
) -> Result<Option<usize>, ExcelError> {
    let needle_folded = s.to_lowercase();
    let compiled_wildcard = (wildcard && (s.contains('*') || s.contains('?') || s.contains('~')))
        .then(|| CompiledWildcardPattern::from_folded(&needle_folded));

    // The lowered-text lane can carry text renderings of non-text cells
    // (some load paths materialize them), so a lane hit is only a match
    // when the underlying cell value really is text: Excel exact lookups
    // never match a text needle against a number/boolean/date cell.
    if vertical {
        for res in view.lowered_text_slices() {
            let (row_start, _row_len, cols) = res?;
            if !cols.is_empty() {
                let arr = &cols[0];
                for i in 0..arr.len() {
                    if !arr.is_null(i) {
                        let val = arr.value(i);
                        let hit = if let Some(pattern) = &compiled_wildcard {
                            pattern.matches_folded(val)
                        } else {
                            val == needle_folded
                        };
                        if hit && matches!(view.get_cell(row_start + i, 0), LiteralValue::Text(_)) {
                            return Ok(Some(row_start + i));
                        }
                    }
                }
            }
        }
    } else {
        for res in view.lowered_text_slices() {
            let (_row_start, _row_len, cols) = res?;
            for (c, arr) in cols.iter().enumerate() {
                if !arr.is_null(0) {
                    let val = arr.value(0);
                    let hit = if let Some(pattern) = &compiled_wildcard {
                        pattern.matches_folded(val)
                    } else {
                        val == needle_folded
                    };
                    if hit && matches!(view.get_cell(0, c), LiteralValue::Text(_)) {
                        return Ok(Some(c));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn find_exact_boolean_in_view(
    view: &RangeView<'_>,
    b: bool,
    vertical: bool,
) -> Result<Option<usize>, ExcelError> {
    if vertical {
        for res in view.booleans_slices() {
            let (row_start, _row_len, cols) = res?;
            if !cols.is_empty() {
                let arr = &cols[0];
                for i in 0..arr.len() {
                    if !arr.is_null(i) && arr.value(i) == b {
                        return Ok(Some(row_start + i));
                    }
                }
            }
        }
    } else {
        for res in view.booleans_slices() {
            let (_row_start, _row_len, cols) = res?;
            for (c, arr) in cols.iter().enumerate() {
                if !arr.is_null(0) && arr.value(0) == b {
                    return Ok(Some(c));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, EvalConfig};
    use crate::test_workbook::TestWorkbook;
    use crate::traits::EvaluationContext;
    use formualizer_parse::parser::ReferenceType;
    use std::hint::black_box;
    use std::time::Instant;

    fn arrow_eval_config() -> EvalConfig {
        EvalConfig {
            arrow_storage_enabled: true,
            delta_overlay_enabled: true,
            write_formula_overlay_enabled: true,
            ..Default::default()
        }
    }

    fn build_vertical_text_engine(
        values: &[LiteralValue],
        chunk_rows: usize,
    ) -> Engine<TestWorkbook> {
        let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
        let mut ab = engine.begin_bulk_ingest_arrow();
        ab.add_sheet("Sheet1", 1, chunk_rows);
        for value in values {
            ab.append_row("Sheet1", std::slice::from_ref(value))
                .unwrap();
        }
        ab.finish().unwrap();
        engine
    }

    fn build_horizontal_text_engine(
        values: &[LiteralValue],
        chunk_rows: usize,
    ) -> Engine<TestWorkbook> {
        let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
        let mut ab = engine.begin_bulk_ingest_arrow();
        ab.add_sheet("Sheet1", values.len(), chunk_rows);
        ab.append_row("Sheet1", values).unwrap();
        ab.finish().unwrap();
        engine
    }

    fn raw_baseline_find_exact_text_in_view(
        view: &RangeView<'_>,
        s: &str,
        wildcard: bool,
        vertical: bool,
    ) -> Result<Option<usize>, ExcelError> {
        let needle_folded = s.to_lowercase();
        let compiled_wildcard = (wildcard
            && (s.contains('*') || s.contains('?') || s.contains('~')))
        .then(|| CompiledWildcardPattern::from_folded(&needle_folded));

        if vertical {
            for res in view.text_slices() {
                let (row_start, _row_len, cols) = res?;
                if !cols.is_empty() {
                    let arr = cols[0]
                        .as_any()
                        .downcast_ref::<arrow_array::StringArray>()
                        .unwrap();
                    for i in 0..arr.len() {
                        if !arr.is_null(i) {
                            let val = arr.value(i);
                            if let Some(pattern) = &compiled_wildcard {
                                let val_folded = val.to_lowercase();
                                if pattern.matches_folded(&val_folded) {
                                    return Ok(Some(row_start + i));
                                }
                            } else if val.to_lowercase() == needle_folded {
                                return Ok(Some(row_start + i));
                            }
                        }
                    }
                }
            }
        } else {
            for res in view.text_slices() {
                let (_row_start, _row_len, cols) = res?;
                for (c, arr_ref) in cols.iter().enumerate() {
                    let arr = arr_ref
                        .as_any()
                        .downcast_ref::<arrow_array::StringArray>()
                        .unwrap();
                    if !arr.is_null(0) {
                        let val = arr.value(0);
                        if let Some(pattern) = &compiled_wildcard {
                            let val_folded = val.to_lowercase();
                            if pattern.matches_folded(&val_folded) {
                                return Ok(Some(c));
                            }
                        } else if val.to_lowercase() == needle_folded {
                            return Ok(Some(c));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    #[test]
    fn find_exact_index_in_view_matches_unicode_exact_and_wildcard_across_chunks_and_overlays() {
        let values = vec![LiteralValue::Empty; 8];
        let mut engine = build_vertical_text_engine(&values, 3);
        engine
            .set_cell_value("Sheet1", 2, 1, LiteralValue::Text("ИВАН".into()))
            .unwrap();
        engine
            .set_cell_value("Sheet1", 7, 1, LiteralValue::Text("Иванов".into()))
            .unwrap();

        let range = ReferenceType::range(
            Some("Sheet1".to_string()),
            Some(1),
            Some(1),
            Some(8),
            Some(1),
        );
        let view = engine.resolve_range_view(&range, "Sheet1").unwrap();

        let exact = LiteralValue::Text("иван".into());
        let wildcard = LiteralValue::Text("ив?н*".into());

        assert_eq!(
            find_exact_index_in_view(&view, &exact, false, DateSystem::Excel1900).unwrap(),
            Some(1)
        );
        assert_eq!(
            find_exact_index_in_view(&view, &wildcard, true, DateSystem::Excel1900).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn find_exact_index_in_view_matches_unicode_exact_and_wildcard_horizontally() {
        let values = vec![
            LiteralValue::Text("Петр".into()),
            LiteralValue::Text("ИВАН".into()),
            LiteralValue::Text("Иванов".into()),
            LiteralValue::Text("Анна".into()),
        ];
        let engine = build_horizontal_text_engine(&values, 4);
        let range = ReferenceType::range(
            Some("Sheet1".to_string()),
            Some(1),
            Some(1),
            Some(1),
            Some(4),
        );
        let view = engine.resolve_range_view(&range, "Sheet1").unwrap();

        let exact = LiteralValue::Text("иван".into());
        let wildcard = LiteralValue::Text("ив?н*".into());

        assert_eq!(
            find_exact_index_in_view(&view, &exact, false, DateSystem::Excel1900).unwrap(),
            Some(1)
        );
        assert_eq!(
            find_exact_index_in_view(&view, &wildcard, true, DateSystem::Excel1900).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn find_exact_number_in_view_does_not_match_blank_as_zero() {
        // Regression test for #319: MATCH(0, {blank, 1, 2}, 0) must return
        // None (which surfaces as #N/A), not match the blank cell.
        let values = vec![
            LiteralValue::Empty,
            LiteralValue::Number(1.0),
            LiteralValue::Number(2.0),
        ];
        let engine = build_vertical_text_engine(&values, 8);
        let range = ReferenceType::range(
            Some("Sheet1".to_string()),
            Some(1),
            Some(1),
            Some(3),
            Some(1),
        );
        let view = engine.resolve_range_view(&range, "Sheet1").unwrap();

        // Searching for 0 must NOT match the blank cell at index 0.
        assert_eq!(
            find_exact_index_in_view(
                &view,
                &LiteralValue::Number(0.0),
                false,
                DateSystem::Excel1900
            )
            .unwrap(),
            None
        );
        assert_eq!(
            find_exact_index_in_view(&view, &LiteralValue::Int(0), false, DateSystem::Excel1900)
                .unwrap(),
            None
        );

        // Ratified S6: a blank needle coerces to zero, not a blank candidate.
        assert_eq!(
            find_exact_index_in_view(&view, &LiteralValue::Empty, false, DateSystem::Excel1900)
                .unwrap(),
            None
        );

        // Searching for 1 still works normally.
        assert_eq!(
            find_exact_index_in_view(
                &view,
                &LiteralValue::Number(1.0),
                false,
                DateSystem::Excel1900
            )
            .unwrap(),
            Some(1)
        );
    }

    #[test]
    fn zero_needles_only_find_real_zero_in_both_orientations() {
        let values = vec![
            LiteralValue::Empty,
            LiteralValue::Text(String::new()),
            LiteralValue::Text("0".into()),
            LiteralValue::Boolean(false),
            LiteralValue::Number(-0.0),
            LiteralValue::Number(0.0),
            LiteralValue::Empty,
            LiteralValue::Boolean(false),
            LiteralValue::Text("0".into()),
        ];
        let no_numeric_zero = vec![
            LiteralValue::Empty,
            LiteralValue::Text(String::new()),
            LiteralValue::Text("0".into()),
            LiteralValue::Boolean(false),
            LiteralValue::Text("0".into()),
            LiteralValue::Boolean(false),
        ];
        for vertical in [true, false] {
            let make_rows = |values: &[LiteralValue]| {
                if vertical {
                    values.iter().cloned().map(|value| vec![value]).collect()
                } else {
                    vec![values.to_vec()]
                }
            };
            let view = RangeView::from_owned_rows(make_rows(&values), DateSystem::Excel1900);
            let missing_view =
                RangeView::from_owned_rows(make_rows(&no_numeric_zero), DateSystem::Excel1900);
            for needle in [
                LiteralValue::Number(0.0),
                LiteralValue::Number(-0.0),
                LiteralValue::Empty,
            ] {
                assert_eq!(
                    find_exact_index_in_view(&view, &needle, false, DateSystem::Excel1900).unwrap(),
                    Some(4)
                );
                assert_eq!(
                    find_exact_index_in_view(&missing_view, &needle, false, DateSystem::Excel1900)
                        .unwrap(),
                    None
                );
            }
        }
    }

    #[test]
    fn materialized_zero_needles_only_match_numeric_zero() {
        let values = vec![
            LiteralValue::Empty,
            LiteralValue::Text(String::new()),
            LiteralValue::Text("0".into()),
            LiteralValue::Boolean(false),
            LiteralValue::Number(-0.0),
            LiteralValue::Number(0.0),
            LiteralValue::Boolean(false),
            LiteralValue::Text("0".into()),
        ];
        let no_numeric_zero = vec![
            LiteralValue::Empty,
            LiteralValue::Text(String::new()),
            LiteralValue::Text("0".into()),
            LiteralValue::Boolean(false),
            LiteralValue::Text("0".into()),
            LiteralValue::Boolean(false),
        ];
        for needle in [
            LiteralValue::Number(0.0),
            LiteralValue::Number(-0.0),
            LiteralValue::Empty,
        ] {
            assert_eq!(
                find_exact_index(&values, &needle, false, DateSystem::Excel1900),
                Some(4)
            );
            assert_eq!(
                find_exact_index(&no_numeric_zero, &needle, false, DateSystem::Excel1900),
                None
            );
        }
    }

    #[test]
    fn wildcard_pattern_match_treats_unicode_scalar_as_single_char() {
        assert!(wildcard_pattern_match("?", "😀"));
        assert!(wildcard_pattern_match("??", "😀x"));
        assert!(!wildcard_pattern_match("?", "😀x"));
    }

    #[test]
    fn wildcard_pattern_match_retries_stars_without_branching() {
        // oracle: lo-verified
        let cases = [
            ("*", "", true),
            ("*", "bravo", true),
            ("br*", "bravo", true),
            ("*avo", "bravo", true),
            ("a**b***d", "abcbd", true),
            ("a*?d", "abcbd", true),
            ("*b*d", "abcbd", true),
            ("*b*d", "abcbx", false),
            ("~*", "*", true),
            ("~?", "?", true),
            ("~~", "~", true),
            ("a~**~?", "a*middle?", true),
            ("ив?н*", "ИВАНОВИЧ", true),
        ];

        for (pattern, text, expected) in cases {
            assert_eq!(
                wildcard_pattern_match(pattern, text),
                expected,
                "pattern={pattern:?}, text={text:?}"
            );
        }
    }

    #[test]
    fn wildcard_pattern_match_pathological_shape_completes_fast() {
        let text = "a".repeat(250_000);
        let start = Instant::now();
        assert!(!wildcard_pattern_match("*a*a*a*a*a*b", &text));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "wildcard retry became pathologically slow: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn find_exact_index_matches_unicode_exact_and_wildcard_in_materialized_vectors() {
        let values = vec![
            LiteralValue::Text("Петр".into()),
            LiteralValue::Text("ИВАН".into()),
            LiteralValue::Text("Иванов".into()),
            LiteralValue::Text("Анна".into()),
        ];
        let exact = LiteralValue::Text("иван".into());
        let wildcard = LiteralValue::Text("ив?н*".into());

        assert_eq!(
            find_exact_index(&values, &exact, false, DateSystem::Excel1900),
            Some(1)
        );
        assert_eq!(
            find_exact_index(&values, &wildcard, true, DateSystem::Excel1900),
            Some(1)
        );
    }

    #[test]
    #[ignore = "benchmark smoke test"]
    fn benchmark_text_lookup_vector_path_vs_raw_baseline() {
        let total = 50_000usize;
        let mut values = Vec::with_capacity(total);
        for i in 0..total {
            if i + 1 == total {
                values.push(LiteralValue::Text("Иванов".into()));
            } else {
                values.push(LiteralValue::Text(format!("строка-{i}")));
            }
        }

        let exact_needle = LiteralValue::Text("иванов".into());
        let wildcard_needle = LiteralValue::Text("ив?н*".into());
        let iters = 30;

        let start = Instant::now();
        for _ in 0..iters {
            let mut out = None;
            for (i, value) in values.iter().enumerate() {
                if equals_maybe_wildcard(
                    &exact_needle,
                    black_box(value),
                    false,
                    DateSystem::Excel1900,
                ) {
                    out = Some(i);
                    break;
                }
            }
            black_box(out);
        }
        let raw_exact = start.elapsed();

        let start = Instant::now();
        for _ in 0..iters {
            black_box(find_exact_index(
                black_box(&values),
                &exact_needle,
                false,
                DateSystem::Excel1900,
            ));
        }
        let opt_exact = start.elapsed();

        let start = Instant::now();
        for _ in 0..iters {
            let mut out = None;
            for (i, value) in values.iter().enumerate() {
                if equals_maybe_wildcard(
                    &wildcard_needle,
                    black_box(value),
                    true,
                    DateSystem::Excel1900,
                ) {
                    out = Some(i);
                    break;
                }
            }
            black_box(out);
        }
        let raw_wildcard = start.elapsed();

        let start = Instant::now();
        for _ in 0..iters {
            black_box(find_exact_index(
                black_box(&values),
                &wildcard_needle,
                true,
                DateSystem::Excel1900,
            ));
        }
        let opt_wildcard = start.elapsed();

        println!(
            "vector exact raw={:?} opt={:?} speedup={:.2}x",
            raw_exact,
            opt_exact,
            raw_exact.as_secs_f64() / opt_exact.as_secs_f64()
        );
        println!(
            "vector wildcard raw={:?} opt={:?} speedup={:.2}x",
            raw_wildcard,
            opt_wildcard,
            raw_wildcard.as_secs_f64() / opt_wildcard.as_secs_f64()
        );
    }

    #[test]
    #[ignore = "benchmark smoke test"]
    fn benchmark_text_lookup_view_path_vs_raw_baseline() {
        let total_rows = 50_000u32;
        let chunk_rows = 512usize;
        let mut values = Vec::with_capacity(total_rows as usize);
        for i in 0..total_rows {
            if i + 1 == total_rows {
                values.push(LiteralValue::Text("Иванов".into()));
            } else {
                values.push(LiteralValue::Text(format!("строка-{i}")));
            }
        }

        let engine = build_vertical_text_engine(&values, chunk_rows);
        let range = ReferenceType::range(
            Some("Sheet1".to_string()),
            Some(1),
            Some(1),
            Some(total_rows),
            Some(1),
        );
        let view = engine.resolve_range_view(&range, "Sheet1").unwrap();

        let exact_needle = LiteralValue::Text("иванов".into());
        let wildcard_needle = LiteralValue::Text("ив?н*".into());
        let iters = 20;

        let start = Instant::now();
        for _ in 0..iters {
            black_box(raw_baseline_find_exact_text_in_view(&view, "иванов", false, true).unwrap());
        }
        let raw_exact = start.elapsed();

        let start = Instant::now();
        for _ in 0..iters {
            black_box(
                find_exact_index_in_view(
                    &view,
                    black_box(&exact_needle),
                    false,
                    DateSystem::Excel1900,
                )
                .unwrap(),
            );
        }
        let opt_exact = start.elapsed();

        let start = Instant::now();
        for _ in 0..iters {
            black_box(raw_baseline_find_exact_text_in_view(&view, "ив?н*", true, true).unwrap());
        }
        let raw_wildcard = start.elapsed();

        let start = Instant::now();
        for _ in 0..iters {
            black_box(
                find_exact_index_in_view(
                    &view,
                    black_box(&wildcard_needle),
                    true,
                    DateSystem::Excel1900,
                )
                .unwrap(),
            );
        }
        let opt_wildcard = start.elapsed();

        println!(
            "lookup exact raw={:?} opt={:?} speedup={:.2}x",
            raw_exact,
            opt_exact,
            raw_exact.as_secs_f64() / opt_exact.as_secs_f64()
        );
        println!(
            "lookup wildcard raw={:?} opt={:?} speedup={:.2}x",
            raw_wildcard,
            opt_wildcard,
            raw_wildcard.as_secs_f64() / opt_wildcard.as_secs_f64()
        );
    }

    #[test]
    fn reference_extent_counts_whole_columns_and_rows_to_the_sheet_edge() {
        for (text, expected) in [
            ("A:A", (1_048_576, 1)),
            ("$C:$Z", (1_048_576, 24)),
            ("2:3", (2, 16_384)),
            ("A$4:A$1048576", (1_048_573, 1)),
            ("B2:D5", (4, 3)),
            ("B2", (1, 1)),
            ("[1]Sort!$C:$Z", (1_048_576, 24)),
        ] {
            let reference = ReferenceType::from_string(text).unwrap();
            assert_eq!(reference_extent(&reference), Some(expected), "{text}");
        }
    }
}
