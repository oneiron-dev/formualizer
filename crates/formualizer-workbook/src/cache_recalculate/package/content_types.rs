use super::{
    Archive, BTreeMap, IoError, Sheet, XlsxRecalculateOptions, part_name, read_part, unsupported,
    xml,
};

/// Content types of the workbook part. Excel opens each of these packages
/// the same way: a template (.xltx), a macro-enabled workbook (.xlsm) or
/// template (.xltm) and an add-in (.xlam) hold the same SpreadsheetML
/// workbook as an .xlsx (ECMA-376 Part 1 §12.3.23, [MS-OFFMACRO2] §2.2.1.4;
/// the add-in type is the one Excel writes for .xlam). A VBA project is a
/// separate part (Default `bin`, `vbaProject` relationship) that is neither
/// parsed nor run here; the writer copies it, its relationship and
/// `[Content_Types].xml` unchanged, so the saved package keeps its kind.
/// Each type is paired with the file extension Excel saves that kind under.
const WORKBOOK_CONTENT_TYPES: [(&str, &str); 5] = [
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        "xlsx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
        "xltx",
    ),
    (
        "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        "xlsm",
    ),
    (
        "application/vnd.ms-excel.template.macroEnabled.main+xml",
        "xltm",
    ),
    (
        "application/vnd.ms-excel.addin.macroEnabled.main+xml",
        "xlam",
    ),
];

/// Checks every part's content type and returns the file extension of the
/// workbook's kind (`xlsx`, `xltx`, `xlsm`, `xltm` or `xlam`).
pub(super) fn validate(
    archive: &mut Archive<'_>,
    sheets: &[Sheet],
    options: &XlsxRecalculateOptions,
) -> Result<&'static str, IoError> {
    const NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
    const PREFIX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.";
    let data = read_part(
        archive,
        "[Content_Types].xml",
        options.limits.max_worksheet_bytes,
    )?;
    let mut defaults = BTreeMap::new();
    let mut overrides = BTreeMap::new();
    xml::walk(&data, options, |path, node| {
        if !matches!(node.kind, xml::Kind::Open { .. }) {
            return Ok(());
        }
        let e = path.last().expect("open XML element");
        if path.len() == 1 && !xml::path_is(path, NS, &["Types"]) {
            return Err(unsupported("content types root/namespace", "XLSX package"));
        }
        if path.len() > 1 {
            if path.len() != 2 || e.ns != NS || !matches!(e.local.as_str(), "Default" | "Override")
            {
                return Err(unsupported(
                    "unknown content-type declaration",
                    "XLSX package",
                ));
            }
            let content = node.required("ContentType")?;
            // Cell metadata is admitted only as the vetted xl/metadata.xml
            // part (dynamic-array flags and rich error tags); see
            // `cell_metadata`.
            let metadata_part = e.local == "Override"
                && node.value("PartName") == Some("/xl/metadata.xml")
                && content == format!("{PREFIX}sheetMetadata+xml");
            if content.is_empty()
                || content.contains("digital-signature")
                || (content.contains("sheetMetadata") && !metadata_part)
            {
                return Err(unsupported("unsupported content type", "XLSX package"));
            }
            if e.local == "Default" {
                let extension = node.required("Extension")?;
                if extension.is_empty()
                    || extension.contains(['.', '/', '\\'])
                    || defaults
                        .insert(extension.to_ascii_lowercase(), content.to_owned())
                        .is_some()
                {
                    return Err(unsupported(
                        "invalid/duplicate default content type",
                        "XLSX package",
                    ));
                }
            } else {
                let name = node
                    .required("PartName")?
                    .strip_prefix('/')
                    .ok_or_else(|| unsupported("relative content-type part", "XLSX package"))?;
                part_name(name)?;
                if overrides
                    .insert(name.to_owned(), content.to_owned())
                    .is_some()
                {
                    return Err(unsupported(
                        "duplicate content-type override",
                        "XLSX package",
                    ));
                }
            }
        }
        Ok(())
    })?;
    let mut extension = None;
    for name in archive.file_names() {
        if name == "[Content_Types].xml" || name.ends_with('/') {
            continue;
        }
        let content = overrides
            .get(name)
            .or_else(|| {
                name.rsplit_once('.')
                    .and_then(|(_, e)| defaults.get(&e.to_ascii_lowercase()))
            })
            .ok_or_else(|| unsupported("part without content type", name))?;
        if name == "xl/workbook.xml" {
            let (_, kind) = WORKBOOK_CONTENT_TYPES
                .iter()
                .find(|(workbook, _)| *workbook == content.as_str())
                .ok_or_else(|| unsupported("part/content-type mismatch", name))?;
            extension = Some(*kind);
            continue;
        }
        let expected = if sheets.iter().any(|s| s.part == name) {
            Some(format!("{PREFIX}worksheet+xml"))
        } else if name == "xl/styles.xml" {
            Some(format!("{PREFIX}styles+xml"))
        } else if name == "xl/sharedStrings.xml" {
            Some(format!("{PREFIX}sharedStrings+xml"))
        } else if name.ends_with(".rels") {
            Some("application/vnd.openxmlformats-package.relationships+xml".into())
        } else {
            None
        };
        if expected
            .as_ref()
            .is_some_and(|expected| content != expected)
        {
            return Err(unsupported("part/content-type mismatch", name));
        }
    }
    extension.ok_or_else(|| unsupported("part without content type", "xl/workbook.xml"))
}
