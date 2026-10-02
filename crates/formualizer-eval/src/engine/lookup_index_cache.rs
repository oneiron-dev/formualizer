use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use formualizer_common::{ExcelError, LiteralValue, SheetId};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::builtins::lookup::lookup_utils::cmp_for_lookup;
use crate::engine::{DateSystem, range_view::RangeView};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct LookupIndexKey {
    pub(crate) sheet_id: SheetId,
    pub(crate) start_row: u32,
    pub(crate) start_col: u32,
    pub(crate) end_row: u32,
    pub(crate) end_col: u32,
    pub(crate) axis: LookupAxis,
    pub(crate) snapshot_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum LookupAxis {
    ColumnInView(usize),
    RowInView(usize),
}

#[derive(Debug, Eq, PartialEq)]
pub enum LookupHashKey {
    Number(u64),
    Text(Box<str>),
    Boolean(bool),
    Empty,
}

impl Hash for LookupHashKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Number(bits) => {
                0u8.hash(state);
                bits.hash(state);
            }
            Self::Text(text) => {
                1u8.hash(state);
                text.hash(state);
            }
            Self::Boolean(value) => {
                2u8.hash(state);
                value.hash(state);
            }
            Self::Empty => {
                3u8.hash(state);
            }
        }
    }
}

impl LookupHashKey {
    fn from_needle(value: &LiteralValue, date_system: DateSystem) -> Option<Self> {
        // Blank needles retain numeric-zero coercion; blank candidates do not.
        if matches!(value, LiteralValue::Empty) {
            Some(Self::Number(0.0f64.to_bits()))
        } else {
            Self::from_literal(value, date_system)
        }
    }

    pub(crate) fn from_literal(value: &LiteralValue, date_system: DateSystem) -> Option<Self> {
        match value {
            LiteralValue::Number(n) => Some(Self::Number(normalize_f64_bits(*n))),
            LiteralValue::Int(i) => Some(Self::Number(normalize_f64_bits(*i as f64))),
            LiteralValue::Text(s) => Some(Self::Text(
                crate::locale::fold_text_case(s).into_boxed_str(),
            )),
            LiteralValue::Boolean(b) => Some(Self::Boolean(*b)),
            LiteralValue::Empty => None,
            // Temporal values are numbers in Excel: key them by their serial so
            // an exact lookup finds them whether the needle or the cell (or
            // both) carry a temporal type rather than a plain numeric.
            LiteralValue::Date(_) | LiteralValue::DateTime(_) | LiteralValue::Time(_) => value
                .as_serial_number_for(date_system)
                .map(|serial| Self::Number(normalize_f64_bits(serial))),
            LiteralValue::Error(_)
            | LiteralValue::Array(_)
            | LiteralValue::Duration(_)
            | LiteralValue::Pending => None,
        }
    }
}

fn normalize_f64_bits(n: f64) -> u64 {
    if n.is_nan() {
        return f64::NAN.to_bits();
    }
    let rounded = n.round();
    if (n - rounded).abs() < 1e-12 {
        // Exact comparisons identify signed zero; the index must do so too.
        if rounded == 0.0 {
            0.0f64.to_bits()
        } else {
            rounded.to_bits()
        }
    } else {
        n.to_bits()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DuplicateIndices {
    pub(crate) first: usize,
    pub(crate) last: usize,
    pub(crate) all: SmallVec<[usize; 1]>,
}

/// An exact-match index over one lookup axis. It answers exactly as the scan
/// in `lookup_utils::find_exact_index_in_view` does, so a lookup gives the
/// same result whether or not an index was built for its range (which depends
/// on how many lookups over that range ran before it).
pub struct LookupIndex {
    pub(crate) len: usize,
    date_system: DateSystem,
    pub(crate) bytes: usize,
    /// Candidate positions by key: case-folded text, a boolean, or a number
    /// (temporal values by their serial; numbers within 1e-12 of an integer
    /// filed under the integer).
    pub(crate) entries: FxHashMap<LookupHashKey, DuplicateIndices>,
    pub(crate) cell_values: Box<[LiteralValue]>,
}

#[cfg(test)]
thread_local! {
    static BUILD_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_build_attempts() -> usize {
    BUILD_ATTEMPTS.with(|c| c.replace(0))
}

impl LookupIndex {
    pub(crate) fn build(
        view: &RangeView<'_>,
        axis: LookupAxis,
        date_system: DateSystem,
    ) -> Result<BuildOutcome, ExcelError> {
        #[cfg(test)]
        BUILD_ATTEMPTS.with(|c| c.set(c.get() + 1));
        let (rows, cols) = view.dims();
        let len = match axis {
            LookupAxis::ColumnInView(col) => {
                if col >= cols {
                    return Ok(BuildOutcome::Degenerate);
                }
                rows
            }
            LookupAxis::RowInView(row) => {
                if row >= rows {
                    return Ok(BuildOutcome::Degenerate);
                }
                cols
            }
        };
        if len == 0 {
            return Ok(BuildOutcome::Degenerate);
        }

        let mut entries: FxHashMap<LookupHashKey, DuplicateIndices> = FxHashMap::default();
        let mut cell_values = Vec::with_capacity(len);
        let mut error_count = 0usize;

        for idx in 0..len {
            let value = match axis {
                LookupAxis::ColumnInView(col) => view.get_cell(idx, col),
                LookupAxis::RowInView(row) => view.get_cell(row, idx),
            };
            if matches!(value, LiteralValue::Error(_)) {
                error_count += 1;
            }
            if let Some(key) = LookupHashKey::from_literal(&value, date_system) {
                let dups = entries.entry(key).or_insert_with(|| DuplicateIndices {
                    first: idx,
                    last: idx,
                    all: SmallVec::new(),
                });
                if dups.all.is_empty() {
                    dups.first = idx;
                }
                dups.last = idx;
                dups.all.push(idx);
            }
            cell_values.push(value);
        }

        if error_count > 0 {
            return Ok(BuildOutcome::ErrorInLookupAxis);
        }

        let bytes = retained_bytes(&cell_values, &entries);
        Ok(BuildOutcome::Built(Self {
            len,
            date_system,
            bytes,
            entries,
            cell_values: cell_values.into_boxed_slice(),
        }))
    }

    pub(crate) fn find_first_exact(&self, needle: &LiteralValue) -> Option<usize> {
        self.find_exact(needle, false)
    }

    pub(crate) fn find_last_exact(&self, needle: &LiteralValue) -> Option<usize> {
        self.find_exact(needle, true)
    }

    fn find_exact(&self, needle: &LiteralValue, last: bool) -> Option<usize> {
        // A blank needle searches for zero.
        let hash_key = LookupHashKey::from_needle(needle, self.date_system)?;
        let dups = self.entries.get(&hash_key)?;
        // Text matches text case-insensitively and a boolean the same boolean:
        // exactly the candidates stored under the needle's key, with no
        // numeric re-check (which read "NaN" or "inf" text as a number that
        // never equals itself). A numeric key also holds the near-integers
        // filed under it, so each candidate is confirmed with the lookup's
        // exact comparison.
        let numeric = matches!(hash_key, LookupHashKey::Number(_));
        let matches = |idx: usize| {
            !numeric || cmp_for_lookup(needle, &self.cell_values[idx], self.date_system) == Some(0)
        };
        if last {
            dups.all.iter().rev().copied().find(|&idx| matches(idx))
        } else {
            dups.all.iter().copied().find(|&idx| matches(idx))
        }
    }
}

fn retained_bytes(
    values: &[LiteralValue],
    entries: &FxHashMap<LookupHashKey, DuplicateIndices>,
) -> usize {
    // HashMap capacity excludes control bytes and vacant buckets. Round up to
    // the backing power-of-two bucket count; allocator bookkeeping is estimated.
    let buckets = if entries.capacity() == 0 {
        0
    } else {
        entries.capacity().saturating_add(1).next_power_of_two()
    };
    let mut bytes = values
        .len()
        .saturating_mul(std::mem::size_of::<LiteralValue>())
        .saturating_add(
            buckets.saturating_mul(std::mem::size_of::<(LookupHashKey, DuplicateIndices)>() + 1),
        )
        .saturating_add(256);
    for value in values {
        bytes = bytes.saturating_add(literal_payload_bytes(value));
    }
    for (key, indices) in entries {
        if let LookupHashKey::Text(text) = key {
            bytes = bytes.saturating_add(text.len());
        }
        if indices.all.spilled() {
            bytes = bytes.saturating_add(
                indices
                    .all
                    .capacity()
                    .saturating_mul(std::mem::size_of::<usize>()),
            );
        }
    }
    bytes
}

fn literal_payload_bytes(value: &LiteralValue) -> usize {
    match value {
        LiteralValue::Text(text) => text.capacity(),
        LiteralValue::Array(rows) => rows.iter().fold(
            rows.capacity()
                .saturating_mul(std::mem::size_of::<Vec<LiteralValue>>()),
            |bytes, row| {
                row.iter().fold(
                    bytes.saturating_add(
                        row.capacity()
                            .saturating_mul(std::mem::size_of::<LiteralValue>()),
                    ),
                    |bytes, value| bytes.saturating_add(literal_payload_bytes(value)),
                )
            },
        ),
        _ => 0,
    }
}

pub(crate) fn estimate_bytes(len: usize, entries: usize) -> usize {
    len.saturating_mul(std::mem::size_of::<LiteralValue>().saturating_add(8))
        .saturating_add(entries.saturating_mul(96))
        .saturating_add(256)
}

pub(crate) enum BuildOutcome {
    Built(LookupIndex),
    ErrorInLookupAxis,
    Degenerate,
}

const LOOKUP_INDEX_BUILD_THRESHOLD: u32 = 3;
const CAP_REJECTED: u32 = u32::MAX;
const CALL_COUNT_PRUNE_LIMIT: usize = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LookupIndexCacheReport {
    pub(crate) builds: usize,
    pub(crate) hits: usize,
    pub(crate) misses: usize,
    pub(crate) skipped_volatile: usize,
    pub(crate) skipped_error: usize,
    pub(crate) skipped_tiny: usize,
    pub(crate) skipped_cap: usize,
    pub(crate) skipped_below_threshold: usize,
    pub(crate) bytes_in_cache: usize,
    pub(crate) entries_count: usize,
}

pub struct LookupIndexCache {
    inner: RwLock<FxHashMap<LookupIndexKey, Arc<LookupIndex>>>,
    call_counts: RwLock<FxHashMap<LookupIndexKey, u32>>,
    volatile_keys: RwLock<FxHashMap<LookupIndexKey, ()>>,
    build_threshold: u32,
    bytes_in_use: AtomicUsize,
    max_bytes: usize,
    builds: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
    skipped_volatile: AtomicUsize,
    skipped_error: AtomicUsize,
    skipped_tiny: AtomicUsize,
    skipped_cap: AtomicUsize,
    skipped_below_threshold: AtomicUsize,
}

fn volatile_key(mut key: LookupIndexKey) -> LookupIndexKey {
    key.snapshot_id = 0;
    key
}

impl LookupIndexCache {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            inner: RwLock::new(FxHashMap::default()),
            call_counts: RwLock::new(FxHashMap::default()),
            volatile_keys: RwLock::new(FxHashMap::default()),
            build_threshold: LOOKUP_INDEX_BUILD_THRESHOLD,
            bytes_in_use: AtomicUsize::new(0),
            max_bytes,
            builds: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            skipped_volatile: AtomicUsize::new(0),
            skipped_error: AtomicUsize::new(0),
            skipped_tiny: AtomicUsize::new(0),
            skipped_cap: AtomicUsize::new(0),
            skipped_below_threshold: AtomicUsize::new(0),
        }
    }

    // Called only at an exclusive Engine mutation boundary, after all evaluation
    // workers have joined. No old-generation builder can race this reclamation.
    pub(crate) fn clear(&mut self) {
        self.inner
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.call_counts
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.volatile_keys
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        self.bytes_in_use.store(0, Ordering::Relaxed);
    }

    pub(crate) fn get(&self, key: &LookupIndexKey) -> Option<Arc<LookupIndex>> {
        let found = self
            .inner
            .read()
            .ok()
            .and_then(|guard| guard.get(key).cloned());
        if found.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        found
    }

    pub(crate) fn should_build(&self, key: LookupIndexKey) -> bool {
        let Ok(mut guard) = self.call_counts.write() else {
            self.skipped_below_threshold.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if guard.len() > CALL_COUNT_PRUNE_LIMIT {
            guard.retain(|existing_key, _| existing_key.snapshot_id == key.snapshot_id);
        }
        let count = guard.entry(key).or_insert(0);
        if *count == CAP_REJECTED {
            self.skipped_cap.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        *count = count.saturating_add(1).min(CAP_REJECTED - 1);
        if *count <= self.build_threshold {
            self.skipped_below_threshold.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    pub(crate) fn would_exceed_cap(&self, bytes: usize) -> bool {
        self.bytes_in_use
            .load(Ordering::Relaxed)
            .saturating_add(bytes)
            > self.max_bytes
    }

    pub(crate) fn is_known_volatile(&self, key: &LookupIndexKey) -> bool {
        let volatile_key = volatile_key(*key);
        self.volatile_keys
            .read()
            .map(|guard| guard.contains_key(&volatile_key))
            .unwrap_or(false)
    }

    pub(crate) fn note_volatile_key(&self, key: LookupIndexKey) {
        if let Ok(mut guard) = self.volatile_keys.write() {
            if guard.len() > CALL_COUNT_PRUNE_LIMIT {
                guard.clear();
            }
            guard.insert(volatile_key(key), ());
        }
    }

    pub(crate) fn insert_if_room(
        &self,
        key: LookupIndexKey,
        index: LookupIndex,
    ) -> Option<Arc<LookupIndex>> {
        let bytes = index.bytes;
        if let Ok(mut guard) = self.inner.write() {
            if let Some(existing) = guard.get(&key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(existing.clone());
            }
            // Serialize duplicate detection, admission and accounting. The earlier
            // preflight estimate is only a hint and cannot reserve concurrent space.
            let current = self.bytes_in_use.load(Ordering::Relaxed);
            if bytes > self.max_bytes.saturating_sub(current) {
                self.skipped_cap.fetch_add(1, Ordering::Relaxed);
                // Actual payloads (especially text) can exceed the preflight
                // estimate. Do not rebuild and discard them on every later call.
                if let Ok(mut counts) = self.call_counts.write() {
                    counts.insert(key, CAP_REJECTED);
                }
                return None;
            }
            let index = Arc::new(index);
            guard.insert(key, index.clone());
            self.bytes_in_use.fetch_add(bytes, Ordering::Relaxed);
            self.builds.fetch_add(1, Ordering::Relaxed);
            Some(index)
        } else {
            None
        }
    }

    pub(crate) fn note_skipped_volatile(&self) {
        self.skipped_volatile.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_skipped_error(&self) {
        self.skipped_error.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_skipped_tiny(&self) {
        self.skipped_tiny.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_skipped_cap(&self) {
        self.skipped_cap.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn reset_counters(&self) {
        self.builds.store(0, Ordering::Relaxed);
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.skipped_volatile.store(0, Ordering::Relaxed);
        self.skipped_error.store(0, Ordering::Relaxed);
        self.skipped_tiny.store(0, Ordering::Relaxed);
        self.skipped_cap.store(0, Ordering::Relaxed);
        self.skipped_below_threshold.store(0, Ordering::Relaxed);
    }

    pub(crate) fn report(&self) -> LookupIndexCacheReport {
        let guard = self.inner.read().ok();
        let entries_count = guard.as_ref().map_or(0, |guard| guard.len());
        LookupIndexCacheReport {
            builds: self.builds.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            skipped_volatile: self.skipped_volatile.load(Ordering::Relaxed),
            skipped_error: self.skipped_error.load(Ordering::Relaxed),
            skipped_tiny: self.skipped_tiny.load(Ordering::Relaxed),
            skipped_cap: self.skipped_cap.load(Ordering::Relaxed),
            skipped_below_threshold: self.skipped_below_threshold.load(Ordering::Relaxed),
            bytes_in_cache: self.bytes_in_use.load(Ordering::Relaxed),
            entries_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn key(col: u32) -> LookupIndexKey {
        LookupIndexKey {
            sheet_id: 0,
            start_row: 0,
            start_col: col,
            end_row: 128,
            end_col: col,
            axis: LookupAxis::ColumnInView(0),
            snapshot_id: 1,
        }
    }
    fn index(bytes: usize) -> LookupIndex {
        LookupIndex {
            len: 0,
            date_system: DateSystem::Excel1900,
            bytes,
            entries: FxHashMap::default(),
            cell_values: Box::new([]),
        }
    }

    #[test]
    fn exact_index_selects_signed_zero_and_temporal_zero_duplicates() {
        assert_eq!(
            LookupHashKey::from_literal(&LiteralValue::Empty, DateSystem::Excel1900),
            None
        );
        assert_eq!(
            LookupHashKey::from_needle(&LiteralValue::Empty, DateSystem::Excel1900),
            Some(LookupHashKey::Number(0.0f64.to_bits()))
        );
        let midnight = LiteralValue::Time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let values = vec![
            LiteralValue::Empty,
            LiteralValue::Boolean(false),
            LiteralValue::Text("0".into()),
            LiteralValue::Number(-0.0),
            midnight.clone(),
            LiteralValue::Number(0.0),
            LiteralValue::Boolean(false),
            LiteralValue::Text("0".into()),
        ];
        let view = RangeView::from_owned_rows(
            values.into_iter().map(|value| vec![value]).collect(),
            DateSystem::Excel1900,
        );
        let BuildOutcome::Built(index) =
            LookupIndex::build(&view, LookupAxis::ColumnInView(0), DateSystem::Excel1900).unwrap()
        else {
            panic!("expected a lookup index");
        };

        assert_eq!(index.find_first_exact(&LiteralValue::Number(0.0)), Some(3));
        assert_eq!(index.find_last_exact(&LiteralValue::Number(-0.0)), Some(5));
        assert_eq!(index.find_first_exact(&midnight), Some(3));
        assert_eq!(index.find_last_exact(&midnight), Some(5));
        assert_eq!(index.find_first_exact(&LiteralValue::Empty), Some(3));
        assert_eq!(index.find_last_exact(&LiteralValue::Empty), Some(5));

        let temporal_only_view = RangeView::from_owned_rows(
            vec![
                vec![LiteralValue::Empty],
                vec![LiteralValue::Boolean(false)],
                vec![LiteralValue::Text("0".into())],
                vec![midnight],
            ],
            DateSystem::Excel1900,
        );
        let BuildOutcome::Built(temporal_only) = LookupIndex::build(
            &temporal_only_view,
            LookupAxis::ColumnInView(0),
            DateSystem::Excel1900,
        )
        .unwrap() else {
            panic!("expected a temporal lookup index");
        };
        assert_eq!(
            temporal_only.find_first_exact(&LiteralValue::Number(0.0)),
            Some(3)
        );
    }

    #[test]
    fn exact_index_answers_as_the_scan() {
        use crate::builtins::lookup::lookup_utils::find_exact_index_in_view;
        // The index is only built once a range has been searched a few times,
        // and parallel evaluation decides which lookups come first, so any
        // answer it gives that the scan would not makes results vary by run.
        let midnight = LiteralValue::Time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let column = vec![
            LiteralValue::Text("Nan".into()),
            LiteralValue::Number(0.30000000000000004),
            LiteralValue::Number(0.3),
            LiteralValue::Text("inf".into()),
            LiteralValue::Number(2.0000000000015),
            LiteralValue::Number(2.0000000000005),
            LiteralValue::Number(3.3),
            LiteralValue::Boolean(true),
            LiteralValue::Text("INFINITY".into()),
            LiteralValue::Number(f64::INFINITY),
            LiteralValue::Number(-0.0),
            LiteralValue::Empty,
            midnight.clone(),
            LiteralValue::Number(1e15),
            LiteralValue::Number(1e15 + 0.125),
            LiteralValue::Text("3.3".into()),
            LiteralValue::Number(0.3),
            LiteralValue::Text("NAN".into()),
            LiteralValue::Int(7),
            LiteralValue::Number(7.0000000000001),
        ];
        let view = RangeView::from_owned_rows(
            column.iter().cloned().map(|value| vec![value]).collect(),
            DateSystem::Excel1900,
        );
        let BuildOutcome::Built(index) =
            LookupIndex::build(&view, LookupAxis::ColumnInView(0), DateSystem::Excel1900).unwrap()
        else {
            panic!("expected a lookup index");
        };
        let needles = [
            LiteralValue::Text("nan".into()),
            LiteralValue::Text("NaN".into()),
            LiteralValue::Text("Inf".into()),
            LiteralValue::Text("infinity".into()),
            LiteralValue::Text("3.3".into()),
            LiteralValue::Number(0.1 + 0.2),
            LiteralValue::Number(0.3),
            LiteralValue::Number(1.1 * 3.0),
            LiteralValue::Number(2.000000000001),
            LiteralValue::Number(2.0),
            LiteralValue::Number(0.0),
            LiteralValue::Number(1e-13),
            LiteralValue::Number(1e15),
            LiteralValue::Number(1e15 + 0.125),
            LiteralValue::Number(7.0),
            LiteralValue::Int(7),
            LiteralValue::Number(f64::INFINITY),
            LiteralValue::Number(f64::NAN),
            LiteralValue::Number(1.0),
            LiteralValue::Boolean(true),
            LiteralValue::Boolean(false),
            LiteralValue::Empty,
            midnight,
        ];
        // The last match is the scan's first match in the reversed column.
        let reversed = RangeView::from_owned_rows(
            column
                .iter()
                .rev()
                .cloned()
                .map(|value| vec![value])
                .collect(),
            DateSystem::Excel1900,
        );
        for needle in &needles {
            let first =
                find_exact_index_in_view(&view, needle, false, DateSystem::Excel1900).unwrap();
            let last = find_exact_index_in_view(&reversed, needle, false, DateSystem::Excel1900)
                .unwrap()
                .map(|i| column.len() - 1 - i);
            assert_eq!(index.find_first_exact(needle), first, "first {needle:?}");
            assert_eq!(index.find_last_exact(needle), last, "last {needle:?}");
        }
        // Text that Rust would read as NaN or infinity is text.
        assert_eq!(
            index.find_first_exact(&LiteralValue::Text("nan".into())),
            Some(0)
        );
        assert_eq!(
            index.find_last_exact(&LiteralValue::Text("nan".into())),
            Some(17)
        );
        assert_eq!(
            index.find_first_exact(&LiteralValue::Text("Inf".into())),
            Some(3)
        );
        // A number finds only the identical number: no tolerance, and a
        // near-integer is not the integer it is filed under.
        assert_eq!(index.find_first_exact(&LiteralValue::Number(0.3)), Some(2));
        assert_eq!(index.find_last_exact(&LiteralValue::Number(0.3)), Some(16));
        assert_eq!(
            index.find_first_exact(&LiteralValue::Number(0.1 + 0.2)),
            Some(1)
        );
        assert_eq!(
            index.find_first_exact(&LiteralValue::Number(1.1 * 3.0)),
            None
        );
        assert_eq!(
            index.find_first_exact(&LiteralValue::Number(2.000000000001)),
            None
        );
        assert_eq!(index.find_first_exact(&LiteralValue::Number(7.0)), Some(18));
        assert_eq!(index.find_last_exact(&LiteralValue::Number(7.0)), Some(18));
    }

    #[test]
    fn exact_index_tells_close_numbers_apart() {
        // 0.2500000000005 and 0.25 are distinct within Excel's 15 significant
        // digits: a warm index finds the 0.25 as the scan does, not the
        // earlier near value.
        let mut column = vec![
            LiteralValue::Number(0.2500000000005),
            LiteralValue::Number(0.25),
        ];
        column.extend((3..=100).map(|n| LiteralValue::Number(n as f64)));
        let view = RangeView::from_owned_rows(
            column.into_iter().map(|value| vec![value]).collect(),
            DateSystem::Excel1900,
        );
        let BuildOutcome::Built(index) =
            LookupIndex::build(&view, LookupAxis::ColumnInView(0), DateSystem::Excel1900).unwrap()
        else {
            panic!("expected a lookup index");
        };
        assert_eq!(index.find_first_exact(&LiteralValue::Number(0.25)), Some(1));
        assert_eq!(index.find_last_exact(&LiteralValue::Number(0.25)), Some(1));
        assert_eq!(
            index.find_first_exact(&LiteralValue::Number(0.2500000000005)),
            Some(0)
        );
        assert_eq!(
            index.find_first_exact(&LiteralValue::Number(0.2500000000001)),
            None
        );
        assert_eq!(index.find_first_exact(&LiteralValue::Number(3.0)), Some(2));
        assert_eq!(
            index.find_first_exact(&LiteralValue::Number(3.0000000000001)),
            None
        );
    }

    #[test]
    fn concurrent_admission_and_duplicate_races_obey_cap() {
        for duplicate in [false, true] {
            let mut cache = LookupIndexCache::new(if duplicate { 1024 } else { 4096 });
            let barrier = std::sync::Barrier::new(16);
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..16)
                    .map(|i| {
                        let cache = &cache;
                        let barrier = &barrier;
                        scope.spawn(move || {
                            barrier.wait();
                            cache.insert_if_room(key(if duplicate { 0 } else { i }), index(1024))
                        })
                    })
                    .collect();
                let admitted: Vec<_> = handles
                    .into_iter()
                    .filter_map(|h| h.join().unwrap())
                    .collect();
                assert_eq!(admitted.len(), if duplicate { 16 } else { 4 });
                if duplicate {
                    assert!(admitted.iter().all(|item| Arc::ptr_eq(item, &admitted[0])));
                }
            });
            let report = cache.report();
            assert_eq!(report.bytes_in_cache, cache.max_bytes);
            assert_eq!(report.builds, if duplicate { 1 } else { 4 });
            assert_eq!(report.entries_count, report.builds);
            assert_eq!(report.skipped_cap, if duplicate { 0 } else { 12 });
            cache.clear();
            assert_eq!(cache.report().bytes_in_cache, 0);
            assert_eq!(cache.report().entries_count, 0);
            assert!(cache.insert_if_room(key(0), index(1024)).is_some());
        }
    }

    #[test]
    fn retained_text_and_duplicate_heap_payloads_are_charged() {
        let mut entries = FxHashMap::default();
        let mut text = String::with_capacity(4096);
        text.push_str("LONG KEY");
        let values = [LiteralValue::Text(text)];
        let empty_bytes = retained_bytes(&[], &entries);
        assert_eq!(
            retained_bytes(&values, &entries) - empty_bytes,
            std::mem::size_of::<LiteralValue>() + 4096
        );
        let mut dups = DuplicateIndices::default();
        dups.all.extend(0..100);
        let heap_bytes = dups.all.capacity() * std::mem::size_of::<usize>();
        entries.insert(LookupHashKey::Text("long key".into()), dups);
        let charged = retained_bytes(&values, &entries);
        assert!(charged >= empty_bytes + 4096 + 8 + heap_bytes);
        entries
            .get_mut(&LookupHashKey::Text("long key".into()))
            .unwrap()
            .all = SmallVec::new();
        assert_eq!(charged - retained_bytes(&values, &entries), heap_bytes);
        let cache = LookupIndexCache::new(charged - 1);
        assert!(cache.insert_if_room(key(0), index(charged)).is_none());
    }
}
