use crate::SheetId;
use crate::engine::graph::DependencyGraph;
use crate::engine::named_range::NameScope;
use crate::engine::vertex::{VertexId, VertexKind};
use formualizer_common::{ExcelError, ExcelErrorKind};

#[derive(Debug, Clone)]
pub struct SourceScalarEntry {
    pub name: String,
    pub vertex: VertexId,
    pub version: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SourceTableEntry {
    pub name: String,
    pub vertex: VertexId,
    pub version: Option<u64>,
}

impl DependencyGraph {
    fn allocate_source_vertex(&mut self) -> VertexId {
        // External sources are identified by name and have no position; the default sheet
        // is recorded only as the scope their lookups answer for.
        let scope_sheet_id: SheetId = self.default_sheet_id;
        self.allocate_symbol_vertex(VertexKind::External, scope_sheet_id)
    }

    /// Register the saved values of a linked workbook under its book token
    /// (`[1]` in `[1]Sheet1!A1`). References into it evaluate from these values
    /// and need no dependency edges, since they cannot change during a recalc.
    pub fn set_external_book(
        &mut self,
        token: &str,
        book: crate::engine::external_book::ExternalBook,
    ) {
        self.external_books.insert(
            crate::engine::external_book::book_key(token),
            std::sync::Arc::new(book),
        );
        self.bump_symbol_revision();
    }

    pub fn external_book(
        &self,
        token: &str,
    ) -> Option<&std::sync::Arc<crate::engine::external_book::ExternalBook>> {
        if self.external_books.is_empty() {
            return None;
        }
        self.external_books
            .get(&crate::engine::external_book::book_key(token))
    }

    /// A reference into a linked workbook that is read from saved values
    /// (registered book) or, with nothing saved and no source defined for it,
    /// cannot be read at all (#REF!). Either way it has no dependency.
    pub fn is_linked_book_ref(&self, ext: &formualizer_parse::parser::ExternalReference) -> bool {
        let token = ext.book.token();
        self.external_book(token).is_some()
            || (crate::engine::external_book::is_link_index(token)
                && !self.source_scalars.contains_key(&ext.raw)
                && !self.source_tables.contains_key(&ext.raw))
    }

    pub fn resolve_source_scalar_entry(&self, name: &str) -> Option<&SourceScalarEntry> {
        self.source_scalars.get(name)
    }

    pub fn resolve_source_table_entry(&self, name: &str) -> Option<&SourceTableEntry> {
        self.source_tables.get(name)
    }

    pub fn define_source_scalar(
        &mut self,
        name: &str,
        version: Option<u64>,
    ) -> Result<(), ExcelError> {
        if name.is_empty() {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message("Source name cannot be empty".to_string()));
        }
        if self.source_scalars.contains_key(name) || self.source_tables.contains_key(name) {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Source already defined: {name}")));
        }

        let vertex = self.allocate_source_vertex();
        self.source_vertex_lookup.insert(vertex, name.to_string());
        self.mark_volatile(vertex, version.is_none());

        let entry = SourceScalarEntry {
            name: name.to_string(),
            vertex,
            version,
        };
        self.source_scalars.insert(name.to_string(), entry);
        self.resolve_pending_name_references(NameScope::Workbook, name);
        self.bump_symbol_revision();
        Ok(())
    }

    pub fn define_source_table(
        &mut self,
        name: &str,
        version: Option<u64>,
    ) -> Result<(), ExcelError> {
        if name.is_empty() {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message("Source name cannot be empty".to_string()));
        }
        if self.source_tables.contains_key(name) || self.source_scalars.contains_key(name) {
            return Err(ExcelError::new(ExcelErrorKind::Name)
                .with_message(format!("Source already defined: {name}")));
        }

        let vertex = self.allocate_source_vertex();
        self.source_vertex_lookup.insert(vertex, name.to_string());
        self.mark_volatile(vertex, version.is_none());

        let entry = SourceTableEntry {
            name: name.to_string(),
            vertex,
            version,
        };
        self.source_tables.insert(name.to_string(), entry);
        self.bump_symbol_revision();
        Ok(())
    }

    pub fn set_source_scalar_version(
        &mut self,
        name: &str,
        version: Option<u64>,
    ) -> Result<(), ExcelError> {
        let vertex = {
            let entry = self.source_scalars.get_mut(name).ok_or_else(|| {
                ExcelError::new(ExcelErrorKind::Name)
                    .with_message(format!("Unknown source: {name}"))
            })?;

            if entry.version == version {
                return Ok(());
            }

            entry.version = version;
            entry.vertex
        };

        self.mark_volatile(vertex, version.is_none());
        self.mark_dirty(vertex);
        Ok(())
    }

    pub fn set_source_table_version(
        &mut self,
        name: &str,
        version: Option<u64>,
    ) -> Result<(), ExcelError> {
        let vertex = {
            let entry = self.source_tables.get_mut(name).ok_or_else(|| {
                ExcelError::new(ExcelErrorKind::Name)
                    .with_message(format!("Unknown source: {name}"))
            })?;

            if entry.version == version {
                return Ok(());
            }

            entry.version = version;
            entry.vertex
        };

        self.mark_volatile(vertex, version.is_none());
        self.mark_dirty(vertex);
        Ok(())
    }

    pub fn invalidate_source(&mut self, name: &str) -> Result<(), ExcelError> {
        if let Some(s) = self.source_scalars.get(name) {
            self.mark_dirty(s.vertex);
            return Ok(());
        }
        if let Some(t) = self.source_tables.get(name) {
            self.mark_dirty(t.vertex);
            return Ok(());
        }
        Err(ExcelError::new(ExcelErrorKind::Name).with_message(format!("Unknown source: {name}")))
    }
}
