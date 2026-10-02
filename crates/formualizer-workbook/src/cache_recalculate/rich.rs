//! Rich error tags ([MS-XLSX] 2.3.6.1.3). The XLSX cell error codes are the
//! legacy seven; Excel saves an error without such a code (#SPILL!, #CALC!)
//! as a cached #VALUE! whose cell `vm` points at value metadata of type
//! XLRICHVALUE in `xl/metadata.xml`, whose future-metadata block names a rich
//! value of structure `_error` in `xl/richData`. Its integer `errorType` is
//! the real error (8 #SPILL!, 13 #CALC!); a #SPILL!'s `colOffset` and
//! `rwOffset` count the additional columns and rows of the result it could
//! not spill. Without the tag the cached value means #VALUE!.
use super::package::{Archive, read_part};
use super::{IoError, Patch, XlsxRecalculateOptions, apply_patches, unsupported, xml};
use formualizer_common::ExcelErrorKind;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

pub(super) const RICH: &str = "http://schemas.microsoft.com/office/spreadsheetml/2017/richdata";
pub(super) const METADATA: &str = "xl/metadata.xml";
pub(super) const STRUCTURES: &str = "xl/richData/rdrichvaluestructure.xml";
pub(super) const VALUES: &str = "xl/richData/rdrichvalue.xml";
const RICH_RELATIONSHIPS: &str = "http://schemas.microsoft.com/office/2017/06/relationships";
const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
/// The extension that names a value-metadata block's rich value.
const RICH_VALUE_BLOCK: &str = "{3e2802c4-a4d2-4d8b-9148-e3be6c30e623}";
const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// The error a rich value tags a cached error as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RichError {
    pub kind: ExcelErrorKind,
    /// A #SPILL!'s additional columns and rows (`colOffset`, `rwOffset`);
    /// `None` for other errors (and a #SPILL! saved without them).
    pub offsets: Option<(u32, u32)>,
}
impl RichError {
    /// The `errorType` of the kinds the engine produces.
    pub fn error_type(kind: ExcelErrorKind) -> Option<u32> {
        match kind {
            ExcelErrorKind::Spill => Some(8),
            ExcelErrorKind::Calc => Some(13),
            _ => None,
        }
    }
    pub fn from_error_type(code: i64) -> Option<ExcelErrorKind> {
        match code {
            8 => Some(ExcelErrorKind::Spill),
            13 => Some(ExcelErrorKind::Calc),
            _ => None,
        }
    }
    /// The keys of the `_error` structure this writer saves the error with,
    /// in Excel's (alphabetical) order. `subType` is optional and only
    /// selects a help topic ([MS-XLSX] 2.3.6.1.3), so it is not written.
    pub fn keys(kind: ExcelErrorKind) -> &'static [&'static str] {
        match kind {
            ExcelErrorKind::Spill => &["colOffset", "errorType", "rwOffset"],
            _ => &["errorType"],
        }
    }
    fn value(&self, key: &str) -> u32 {
        let (cols, rows) = self.offsets.unwrap_or_default();
        match key.to_ascii_lowercase().as_str() {
            "coloffset" => cols,
            "rwoffset" => rows,
            _ => Self::error_type(self.kind).unwrap_or_default(),
        }
    }
}

/// A saved value-metadata record: the error it tags (`None` for a kind the
/// engine never produces) and whether its rich value holds just the keys
/// this writer writes, so that other cells holding that error may share it.
pub(super) type Record = (Option<RichError>, bool);

/// The value-metadata records of a package and the ones the writer adds.
#[derive(Debug, Default)]
pub(super) struct RichTags {
    records: Vec<Record>,
    added: Vec<RichError>,
    shared: HashMap<RichError, usize>,
}
impl RichTags {
    pub fn new(records: Vec<Record>) -> Self {
        let mut shared = HashMap::new();
        for (i, record) in records.iter().enumerate().rev() {
            if let (Some(error), true) = record {
                shared.insert(*error, i + 1);
            }
        }
        Self {
            records,
            added: Vec::new(),
            shared,
        }
    }
    /// The number of saved records (valid `vm` values are 1 to this).
    pub fn len(&self) -> usize {
        self.records.len()
    }
    /// The error record `vm` (1-based) tags.
    pub fn get(&self, vm: usize) -> Option<RichError> {
        let i = vm.checked_sub(1)?;
        match self.records.get(i) {
            Some((error, _)) => *error,
            None => self.added.get(i - self.records.len()).copied(),
        }
    }
    /// The `vm` of a record tagging `error`, adding one if no record may be
    /// shared.
    pub fn tag(&mut self, error: RichError) -> usize {
        if let Some(&vm) = self.shared.get(&error) {
            return vm;
        }
        self.added.push(error);
        let vm = self.records.len() + self.added.len();
        self.shared.insert(error, vm);
        vm
    }
    /// The package edits that save the added records: their rich values,
    /// structures, future-metadata blocks and value metadata, and the content
    /// types and workbook relationships of any part this creates.
    pub fn finish(
        &self,
        archive: &mut Archive<'_>,
        options: &XlsxRecalculateOptions,
    ) -> Result<PackageEdits, IoError> {
        let mut edits = PackageEdits::default();
        if self.added.is_empty() {
            return Ok(edits);
        }
        let limit = options.limits.max_worksheet_bytes;
        let mut created: Vec<(&str, &str, String, &str)> = Vec::new();
        // Structures: one `_error` structure per kind, reusing a saved one
        // with exactly these keys.
        let mut kinds: Vec<ExcelErrorKind> = Vec::new();
        for error in &self.added {
            if !kinds.contains(&error.kind) {
                kinds.push(error.kind);
            }
        }
        let saved = if archive.index_for_name(STRUCTURES).is_some() {
            let data = read_part(archive, STRUCTURES, limit)?;
            let parsed = Container::parse(&data, options, "rvStructures", "s", Some("k"))?;
            Some((data, parsed))
        } else {
            None
        };
        let existing = saved.as_ref().map_or(&[][..], |(_, p)| &p.items);
        let mut structure: HashMap<ExcelErrorKind, (usize, Vec<String>)> = HashMap::new();
        let mut appended = String::new();
        let mut count = existing.len();
        for &kind in &kinds {
            let wanted: HashSet<String> = RichError::keys(kind)
                .iter()
                .map(|k| k.to_ascii_lowercase())
                .collect();
            let found = existing.iter().position(|keys| {
                keys.len() == wanted.len()
                    && keys
                        .iter()
                        .all(|(n, t)| t == "i" && wanted.contains(&n.to_ascii_lowercase()))
            });
            let entry = match found {
                Some(i) => (i, existing[i].iter().map(|(n, _)| n.clone()).collect()),
                None => {
                    let keys = RichError::keys(kind);
                    appended.push_str(&format!(
                        "<{{p}}s t=\"_error\">{}</{{p}}s>",
                        keys.iter()
                            .map(|k| format!("<{{p}}k n=\"{k}\" t=\"i\"/>"))
                            .collect::<String>()
                    ));
                    count += 1;
                    (count - 1, keys.iter().map(|k| (*k).to_owned()).collect())
                }
            };
            structure.insert(kind, entry);
        }
        if !appended.is_empty() {
            match &saved {
                Some((data, parsed)) => {
                    edits.replace(STRUCTURES, parsed.append(data, &appended, count, limit)?);
                }
                None => {
                    edits.create(
                        STRUCTURES,
                        Container::create("rvStructures", RICH, &appended, count),
                    );
                    created.push((
                        STRUCTURES,
                        "application/vnd.ms-excel.rdrichvaluestructure+xml",
                        format!("{RICH_RELATIONSHIPS}/rdRichValueStructure"),
                        "richData/rdrichvaluestructure.xml",
                    ));
                }
            }
        }
        // Rich values, after the saved ones.
        let saved = if archive.index_for_name(VALUES).is_some() {
            let data = read_part(archive, VALUES, limit)?;
            let parsed = Container::parse(&data, options, "rvData", "rv", None)?;
            Some((data, parsed))
        } else {
            None
        };
        let first_value = saved.as_ref().map_or(0, |(_, p)| p.items.len());
        let values: String = self
            .added
            .iter()
            .map(|error| {
                let (s, keys) = &structure[&error.kind];
                format!(
                    "<{{p}}rv s=\"{s}\">{}</{{p}}rv>",
                    keys.iter()
                        .map(|k| format!("<{{p}}v>{}</{{p}}v>", error.value(k)))
                        .collect::<String>()
                )
            })
            .collect();
        let total = first_value + self.added.len();
        match &saved {
            Some((data, parsed)) => {
                edits.replace(VALUES, parsed.append(data, &values, total, limit)?)
            }
            None => {
                edits.create(VALUES, Container::create("rvData", RICH, &values, total));
                created.push((
                    VALUES,
                    "application/vnd.ms-excel.rdrichvalue+xml",
                    format!("{RICH_RELATIONSHIPS}/rdRichValue"),
                    "richData/rdrichvalue.xml",
                ));
            }
        }
        // Value metadata: an XLRICHVALUE future-metadata block per rich
        // value and a record per block.
        let rich_type = "<{p}metadataType name=\"XLRICHVALUE\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\"/>";
        let blocks = |first_value: usize| -> String {
            (0..self.added.len())
                .map(|j| {
                    format!(
                        "<{{p}}bk><{{p}}extLst><{{p}}ext uri=\"{RICH_VALUE_BLOCK}\"><xlrd:rvb xmlns:xlrd=\"{RICH}\" i=\"{}\"/></{{p}}ext></{{p}}extLst></{{p}}bk>",
                        first_value + j
                    )
                })
                .collect()
        };
        let records = |rich_type: usize, first_block: usize| -> String {
            (0..self.added.len())
                .map(|j| {
                    format!(
                        "<{{p}}bk><{{p}}rc t=\"{rich_type}\" v=\"{}\"/></{{p}}bk>",
                        first_block + j
                    )
                })
                .collect()
        };
        let n = self.added.len();
        if archive.index_for_name(METADATA).is_some() {
            let data = read_part(archive, METADATA, limit)?;
            let m = Metadata::parse(&data, options)?;
            let p = prefix(&m.root.qualified);
            let fill = |s: &str| s.replace("{p}", &p);
            let root_close = m
                .root
                .close
                .as_ref()
                .ok_or_else(|| unsupported("empty metadata root", METADATA))?
                .start;
            let mut patches = Vec::new();
            let type_index = match m.type_names.iter().position(|t| t == "XLRICHVALUE") {
                Some(i) => i + 1,
                None => {
                    match &m.types {
                        Some(types) => {
                            types.append(&data, &fill(rich_type), &mut patches)?;
                            types.set_count(m.type_names.len() + 1, &mut patches);
                        }
                        None => patches.push(insert(
                            m.root.open.end,
                            fill(&format!(
                                "<{{p}}metadataTypes count=\"1\">{rich_type}</{{p}}metadataTypes>"
                            )),
                        )),
                    }
                    m.type_names.len() + 1
                }
            };
            let first_block = match m.futures.iter().find(|(name, _)| name == "XLRICHVALUE") {
                Some((_, future)) => {
                    future.append(&data, &fill(&blocks(first_value)), &mut patches)?;
                    future.set_count(future.children + n, &mut patches);
                    future.children
                }
                None => {
                    let at = m
                        .futures
                        .last()
                        .map(|(_, f)| f.end())
                        .or(m.types.as_ref().map(Element::end))
                        .unwrap_or(m.root.open.end);
                    patches.push(insert(
                        at,
                        fill(&format!(
                            "<{{p}}futureMetadata name=\"XLRICHVALUE\" count=\"{n}\">{}</{{p}}futureMetadata>",
                            blocks(first_value)
                        )),
                    ));
                    0
                }
            };
            let records = fill(&records(type_index, first_block));
            match &m.values {
                Some(values) => {
                    if values.children != self.records.len() {
                        return Err(unsupported("value metadata record count", METADATA));
                    }
                    values.append(&data, &records, &mut patches)?;
                    values.set_count(values.children + n, &mut patches);
                }
                None => patches.push(insert(
                    m.extension.unwrap_or(root_close),
                    fill(&format!(
                        "<{{p}}valueMetadata count=\"{n}\">{records}</{{p}}valueMetadata>"
                    )),
                )),
            }
            edits.replace(METADATA, apply_patches(&data, patches, limit)?);
        } else {
            let body = format!(
                "<{{p}}metadataTypes count=\"1\">{rich_type}</{{p}}metadataTypes><{{p}}futureMetadata name=\"XLRICHVALUE\" count=\"{n}\">{}</{{p}}futureMetadata><{{p}}valueMetadata count=\"{n}\">{}</{{p}}valueMetadata>",
                blocks(first_value),
                records(1, 0)
            );
            edits.create(
                METADATA,
                format!(
                    "{DECLARATION}<metadata xmlns=\"{}\" xmlns:xlrd=\"{RICH}\">{}</metadata>",
                    xml::MAIN,
                    body.replace("{p}", "")
                )
                .into_bytes(),
            );
            created.push((
                METADATA,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml",
                format!("{}/sheetMetadata", xml::OFFICE),
                "metadata.xml",
            ));
        }
        if !created.is_empty() {
            declare_parts(archive, options, &created, &mut edits)?;
        }
        Ok(edits)
    }
}

/// Parts the writer replaces and adds.
#[derive(Debug, Default)]
pub(super) struct PackageEdits {
    pub replaced: BTreeMap<String, Vec<u8>>,
    pub added: Vec<(String, Vec<u8>)>,
}
impl PackageEdits {
    fn replace(&mut self, part: &str, bytes: Vec<u8>) {
        self.replaced.insert(part.to_owned(), bytes);
    }
    fn create(&mut self, part: &str, bytes: Vec<u8>) {
        self.added.push((part.to_owned(), bytes));
    }
}

/// The content type and workbook relationship of each created part
/// (`part`, content type, relationship type, target from `xl/workbook.xml`).
fn declare_parts(
    archive: &mut Archive<'_>,
    options: &XlsxRecalculateOptions,
    created: &[(&str, &str, String, &str)],
    edits: &mut PackageEdits,
) -> Result<(), IoError> {
    const TYPES: &str = "[Content_Types].xml";
    const RELS: &str = "xl/_rels/workbook.xml.rels";
    let limit = options.limits.max_worksheet_bytes;
    let data = read_part(archive, TYPES, limit)?;
    let mut root = None;
    let mut overrides = HashSet::new();
    xml::walk(&data, options, |path, node| {
        Element::track(&mut root, path, &node, CONTENT_TYPES, "Types");
        if xml::path_is(path, CONTENT_TYPES, &["Types", "Override"])
            && let Some(name) = node.value("PartName")
        {
            overrides.insert(name.trim_start_matches('/').to_ascii_lowercase());
        }
        Ok(())
    })?;
    let root = root.ok_or_else(|| unsupported("content types root", TYPES))?;
    let p = prefix(&root.qualified);
    let declared: String = created
        .iter()
        .filter(|(part, ..)| !overrides.contains(&part.to_ascii_lowercase()))
        .map(|(part, content, ..)| {
            format!("<{p}Override PartName=\"/{part}\" ContentType=\"{content}\"/>")
        })
        .collect();
    if !declared.is_empty() {
        let mut patches = Vec::new();
        root.append(&data, &declared, &mut patches)?;
        edits.replace(TYPES, apply_patches(&data, patches, limit)?);
    }
    let data = read_part(archive, RELS, limit)?;
    let mut root = None;
    let mut ids = HashSet::new();
    let mut related = HashSet::new();
    xml::walk(&data, options, |path, node| {
        Element::track(&mut root, path, &node, xml::RELS, "Relationships");
        if xml::path_is(path, xml::RELS, &["Relationships", "Relationship"]) {
            if let Some(id) = node.value("Id") {
                ids.insert(id.to_owned());
            }
            if let (Some(kind), Some(target)) = (node.value("Type"), node.value("Target"))
                && node.value("TargetMode") != Some("External")
                && let Ok(target) = crate::xlsx_path::resolve("xl/workbook.xml", target)
            {
                related.insert((kind.to_owned(), target));
            }
        }
        Ok(())
    })?;
    let root = root.ok_or_else(|| unsupported("relationships root", RELS))?;
    let p = prefix(&root.qualified);
    let mut next = 1;
    let mut relationships = String::new();
    for (part, _, kind, target) in created {
        if related.contains(&(kind.clone(), (*part).to_owned())) {
            continue;
        }
        while ids.contains(&format!("rId{next}")) {
            next += 1;
        }
        ids.insert(format!("rId{next}"));
        relationships.push_str(&format!(
            "<{p}Relationship Id=\"rId{next}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    if !relationships.is_empty() {
        let mut patches = Vec::new();
        root.append(&data, &relationships, &mut patches)?;
        edits.replace(RELS, apply_patches(&data, patches, limit)?);
    }
    Ok(())
}

/// The namespace prefix (with its colon) of a qualified element name.
fn prefix(qualified: &str) -> String {
    qualified
        .rsplit_once(':')
        .map(|(p, _)| format!("{p}:"))
        .unwrap_or_default()
}
fn insert(at: usize, text: String) -> Patch {
    Patch {
        span: at..at,
        replacement: text.into_bytes(),
    }
}

/// An element's source spans, for appending children and recounting them.
#[derive(Debug)]
struct Element {
    qualified: String,
    open: Range<usize>,
    count: Option<Range<usize>>,
    /// The end tag; `None` for an empty (`/>`) element.
    close: Option<Range<usize>>,
    /// Its child elements (blocks, types, structures or values).
    children: usize,
}
impl Element {
    fn open(path: &[xml::Element], node: &xml::Node) -> Self {
        Self {
            qualified: path.last().map(|e| e.qualified.clone()).unwrap_or_default(),
            open: node.span.clone(),
            count: node.attribute("", "count").map(|a| a.span.clone()),
            close: None,
            children: 0,
        }
    }
    /// Records the root element `local` of namespace `ns` and its end tag.
    fn track(
        root: &mut Option<Self>,
        path: &[xml::Element],
        node: &xml::Node,
        ns: &str,
        local: &str,
    ) {
        if !xml::path_is(path, ns, &[local]) {
            return;
        }
        match node.kind {
            xml::Kind::Open { .. } => *root = Some(Self::open(path, node)),
            xml::Kind::Close => {
                if let Some(root) = root.as_mut() {
                    root.close = Some(node.span.clone());
                }
            }
            xml::Kind::Text(_) => {}
        }
    }
    fn end(&self) -> usize {
        self.close.as_ref().map_or(self.open.end, |c| c.end)
    }
    /// Appends `children` (with `{p}` standing for the element's prefix).
    fn append(&self, data: &[u8], children: &str, patches: &mut Vec<Patch>) -> Result<(), IoError> {
        let children = children.replace("{p}", &prefix(&self.qualified));
        match &self.close {
            Some(close) => patches.push(insert(close.start, children)),
            None => {
                let end = self.open.end;
                if data.get(end.saturating_sub(2)..end) != Some(b"/>") {
                    return Err(unsupported("unexpected empty element", "XLSX XML"));
                }
                patches.push(Patch {
                    span: end - 2..end,
                    replacement: format!(">{children}</{}>", self.qualified).into_bytes(),
                });
            }
        }
        Ok(())
    }
    fn set_count(&self, count: usize, patches: &mut Vec<Patch>) {
        if let Some(span) = &self.count {
            patches.push(Patch {
                span: span.clone(),
                replacement: format!("count=\"{count}\"").into_bytes(),
            });
        }
    }
}

/// A rich data part: its root and its items (structures with their keys'
/// names and types, or rich values).
struct Container {
    root: Element,
    items: Vec<Vec<(String, String)>>,
}
impl Container {
    fn parse(
        data: &[u8],
        options: &XlsxRecalculateOptions,
        root: &str,
        item: &str,
        key: Option<&str>,
    ) -> Result<Self, IoError> {
        let mut element = None;
        let mut items: Vec<Vec<(String, String)>> = Vec::new();
        xml::walk(data, options, |path, node| {
            Element::track(&mut element, path, &node, RICH, root);
            if matches!(node.kind, xml::Kind::Open { .. }) {
                if xml::path_is(path, RICH, &[root, item]) {
                    items.push(Vec::new());
                } else if let Some(key) = key
                    && xml::path_is(path, RICH, &[root, item, key])
                    && let Some(keys) = items.last_mut()
                {
                    keys.push((
                        node.value("n").unwrap_or_default().to_owned(),
                        node.value("t").unwrap_or_default().to_owned(),
                    ));
                }
            }
            Ok(())
        })?;
        let root = element.ok_or_else(|| unsupported("rich data root", root.to_owned()))?;
        Ok(Self { root, items })
    }
    fn append(
        &self,
        data: &[u8],
        items: &str,
        count: usize,
        limit: usize,
    ) -> Result<Vec<u8>, IoError> {
        let mut patches = Vec::new();
        self.root.append(data, items, &mut patches)?;
        self.root.set_count(count, &mut patches);
        apply_patches(data, patches, limit)
    }
    fn create(root: &str, ns: &str, items: &str, count: usize) -> Vec<u8> {
        format!(
            "{DECLARATION}<{root} xmlns=\"{ns}\" count=\"{count}\">{}</{root}>",
            items.replace("{p}", "")
        )
        .into_bytes()
    }
}

/// The parts of `xl/metadata.xml` the writer extends.
struct Metadata {
    root: Element,
    types: Option<Element>,
    type_names: Vec<String>,
    futures: Vec<(String, Element)>,
    values: Option<Element>,
    /// Where the root's extension list starts.
    extension: Option<usize>,
}
impl Metadata {
    fn parse(data: &[u8], options: &XlsxRecalculateOptions) -> Result<Self, IoError> {
        let mut root = None;
        let mut types = None;
        let mut type_names = Vec::new();
        let mut futures: Vec<(String, Element)> = Vec::new();
        let mut values = None;
        let mut extension = None;
        // The section (child of the root) being read.
        let mut section: Option<String> = None;
        xml::walk(data, options, |path, node| {
            Element::track(&mut root, path, &node, xml::MAIN, "metadata");
            let in_main = path.iter().all(|e| e.ns == xml::MAIN);
            match &node.kind {
                xml::Kind::Open { empty, .. } if path.len() == 2 && in_main => {
                    let local = path[1].local.as_str();
                    let mut element = Element::open(path, &node);
                    if *empty {
                        element.close = None;
                    }
                    match local {
                        "metadataTypes" => types = Some(element),
                        "futureMetadata" => futures
                            .push((node.value("name").unwrap_or_default().to_owned(), element)),
                        "valueMetadata" => values = Some(element),
                        "extLst" => extension = Some(node.span.start),
                        _ => {}
                    }
                    section = (!*empty).then(|| local.to_owned());
                }
                xml::Kind::Open { .. } if path.len() == 3 && in_main => {
                    let current = match section.as_deref() {
                        Some("metadataTypes") if path[2].local == "metadataType" => {
                            type_names.push(node.value("name").unwrap_or_default().to_owned());
                            types.as_mut()
                        }
                        Some("futureMetadata") if path[2].local == "bk" => {
                            futures.last_mut().map(|(_, f)| f)
                        }
                        Some("valueMetadata") if path[2].local == "bk" => values.as_mut(),
                        _ => None,
                    };
                    if let Some(element) = current {
                        element.children += 1;
                    }
                }
                xml::Kind::Close if path.len() == 2 && in_main => {
                    let current = match section.take().as_deref() {
                        Some("metadataTypes") => types.as_mut(),
                        Some("futureMetadata") => futures.last_mut().map(|(_, f)| f),
                        Some("valueMetadata") => values.as_mut(),
                        _ => None,
                    };
                    if let Some(element) = current {
                        element.close = Some(node.span.clone());
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        Ok(Self {
            root: root.ok_or_else(|| unsupported("metadata root", METADATA))?,
            types,
            type_names,
            futures,
            values,
            extension,
        })
    }
}
