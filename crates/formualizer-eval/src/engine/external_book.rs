//! Cached values of linked (external) workbooks.
//!
//! A workbook that references another workbook (`[1]Sheet1!A1` in its stored
//! formulas) saves the referenced values alongside the link (the xlsx
//! `externalLink` part). When the linked workbook is not open, Excel evaluates
//! those references from the saved values: a cell missing from a saved sheet
//! is blank, and a sheet the link does not name is `#REF!`. A sheet Excel could
//! not read when it last refreshed the link (`refreshError`), or saved no
//! values for at all, is known only by the cells it saved: every other cell of
//! it is `#REF!` (Excel for Windows 16.0.20430).

use formualizer_common::{ExcelError, ExcelErrorKind, LiteralValue};
use std::collections::BTreeMap;

/// Values saved for one linked workbook.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExternalBook {
    sheets: Vec<ExternalSheet>,
}

/// Values saved for one sheet of a linked workbook, keyed by 1-based (row, col).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExternalSheet {
    name: String,
    cells: BTreeMap<(u32, u32), LiteralValue>,
    max_row: u32,
    max_col: u32,
    refresh_error: bool,
}

impl ExternalBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a sheet (in the linked workbook's order) and return it for filling.
    pub fn add_sheet(&mut self, name: impl Into<String>) -> &mut ExternalSheet {
        self.sheets.push(ExternalSheet {
            name: name.into(),
            ..ExternalSheet::default()
        });
        self.sheets.last_mut().expect("sheet just added")
    }

    pub fn sheets(&self) -> &[ExternalSheet] {
        &self.sheets
    }

    /// Sheet names match case-insensitively, as in Excel.
    pub fn sheet(&self, name: &str) -> Option<&ExternalSheet> {
        self.sheets
            .iter()
            .find(|sheet| sheet.name.eq_ignore_ascii_case(name))
            .or_else(|| {
                let folded = name.to_lowercase();
                self.sheets
                    .iter()
                    .find(|sheet| sheet.name.to_lowercase() == folded)
            })
    }
}

impl ExternalSheet {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Mark the sheet as one Excel could not read when it last refreshed the
    /// link (`refreshError`), or saved no values for: a cell it did not save
    /// is `#REF!`, not blank.
    pub fn set_refresh_error(&mut self, refresh_error: bool) {
        self.refresh_error = refresh_error;
    }

    pub fn refresh_error(&self) -> bool {
        self.refresh_error
    }

    /// Record the saved value of a cell (1-based row and column); a saved
    /// blank is `LiteralValue::Empty`.
    pub fn set(&mut self, row: u32, col: u32, value: LiteralValue) {
        if row == 0 || col == 0 {
            return;
        }
        self.max_row = self.max_row.max(row);
        self.max_col = self.max_col.max(col);
        self.cells.insert((row, col), value);
    }

    /// Saved value of a cell; a cell that was not saved is [`Self::unsaved`].
    pub fn get(&self, row: u32, col: u32) -> LiteralValue {
        self.cells
            .get(&(row, col))
            .cloned()
            .unwrap_or_else(|| self.unsaved())
    }

    /// The value of a cell that was not saved: blank, or `#REF!` on a sheet
    /// with a refresh error.
    pub fn unsaved(&self) -> LiteralValue {
        if self.refresh_error {
            LiteralValue::Error(ExcelError::new(ExcelErrorKind::Ref))
        } else {
            LiteralValue::Empty
        }
    }

    /// Last saved (row, col), or (0, 0) when nothing was saved.
    pub fn extent(&self) -> (u32, u32) {
        (self.max_row, self.max_col)
    }

    /// Values of the 1-based inclusive rectangle, row-major.
    pub fn rows(
        &self,
        start_row: u32,
        start_col: u32,
        end_row: u32,
        end_col: u32,
    ) -> Vec<Vec<LiteralValue>> {
        (start_row..=end_row)
            .map(|row| {
                (start_col..=end_col)
                    .map(|col| self.get(row, col))
                    .collect()
            })
            .collect()
    }
}

/// `[1]!Table1`: a table in a linked workbook. Excel reads linked tables only
/// while that workbook is open, so references to them evaluate to #REF!.
pub fn is_linked_table_name(name: &str) -> bool {
    name.starts_with('[') && name.contains("]!")
}

/// `[1]`: the position of a linked workbook in the file's link list. A link
/// with no saved values cannot be read while the workbook is closed (#REF!).
pub fn is_link_index(token: &str) -> bool {
    token
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|inner| !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()))
}

/// Normalized key of a book token such as `[1]`.
pub(crate) fn book_key(token: &str) -> String {
    token.trim().to_lowercase()
}
