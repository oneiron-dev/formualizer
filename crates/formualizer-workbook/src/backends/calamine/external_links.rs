//! Saved values of linked workbooks (`xl/externalLinks/externalLinkN.xml`).
//!
//! Stored formulas name a linked workbook by its 1-based position in the
//! workbook's `<externalReferences>` list (`[1]Sheet1!A1`). Each position's
//! part keeps the linked sheet names and the values Excel last read from them.

use super::CalamineAdapter;
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_eval::engine::external_book::ExternalBook;
use quick_xml::Reader as XmlReader;
use quick_xml::events::{BytesStart, Event};
use std::collections::BTreeMap;
use std::io::{BufReader, Read, Seek};
use zip::ZipArchive;

fn read_member<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn local_attr<R>(xml: &XmlReader<R>, start: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    start
        .attributes()
        .filter_map(Result::ok)
        .find(|attr| attr.key.local_name().as_ref() == local)
        .and_then(|attr| {
            attr.decode_and_unescape_value(xml.decoder())
                .ok()
                .map(|v| v.into_owned())
        })
}

/// Resolve a workbook relationship target to a package member name.
fn member_name(target: &str) -> String {
    let path = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Book tokens (`[1]`, `[2]`, ...) with the saved values of each linked
/// workbook. DDE and OLE links carry no workbook values and are skipped.
pub(super) fn scan_external_books<R: Read + Seek>(reader: R) -> Vec<(String, ExternalBook)> {
    let Ok(mut archive) = ZipArchive::new(reader) else {
        return Vec::new();
    };
    let Some(workbook) = read_member(&mut archive, "xl/workbook.xml") else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    let mut xml = XmlReader::from_reader(workbook.as_slice());
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(ref e) | Event::Empty(ref e))
                if e.local_name().as_ref() == b"externalReference" =>
            {
                ids.push(local_attr(&xml, e, b"id"));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let mut targets = BTreeMap::new();
    if let Some(rels) = read_member(&mut archive, "xl/_rels/workbook.xml.rels") {
        let mut xml = XmlReader::from_reader(rels.as_slice());
        loop {
            buf.clear();
            match xml.read_event_into(&mut buf) {
                Ok(Event::Start(ref e) | Event::Empty(ref e))
                    if e.local_name().as_ref() == b"Relationship" =>
                {
                    if let (Some(id), Some(target)) =
                        (local_attr(&xml, e, b"Id"), local_attr(&xml, e, b"Target"))
                    {
                        targets.insert(id, target);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }
    let mut books = Vec::new();
    for (position, id) in ids.iter().enumerate() {
        let Some(target) = id.as_ref().and_then(|id| targets.get(id)) else {
            continue;
        };
        let Some(part) = read_member(&mut archive, &member_name(target)) else {
            continue;
        };
        if let Some(book) = parse_external_book(&part) {
            books.push((format!("[{}]", position + 1), book));
        }
    }
    books
}

fn cell_value(kind: Option<&str>, text: &str) -> LiteralValue {
    match kind {
        Some("str" | "s" | "inlineStr") => LiteralValue::Text(text.to_string()),
        Some("b") => LiteralValue::Boolean(text.trim() == "1" || text.trim() == "true"),
        Some("e") => LiteralValue::Error(ExcelError::from_error_string(text.trim())),
        _ => text
            .trim()
            .parse::<f64>()
            .map(LiteralValue::Number)
            .unwrap_or_else(|_| LiteralValue::Text(text.to_string())),
    }
}

/// Parse one `externalLink` part; `None` unless it links a workbook.
fn parse_external_book(part: &[u8]) -> Option<ExternalBook> {
    let mut xml = XmlReader::from_reader(BufReader::new(part));
    let mut buf = Vec::new();
    let mut names = Vec::new();
    let mut saved: BTreeMap<usize, Vec<(u32, u32, LiteralValue)>> = BTreeMap::new();
    let mut in_book = false;
    let mut sheet: Option<usize> = None;
    let mut cell: Option<(u32, u32, Option<String>)> = None;
    let mut value: Option<String> = None;
    let mut in_value = false;
    loop {
        buf.clear();
        let event = xml.read_event_into(&mut buf).ok()?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"externalBook" => in_book = true,
                    b"sheetName" if in_book => names.push(local_attr(&xml, e, b"val")?),
                    b"sheetData" if in_book && !empty => {
                        sheet = local_attr(&xml, e, b"sheetId").and_then(|id| id.parse().ok());
                    }
                    b"cell" if sheet.is_some() => {
                        let position = local_attr(&xml, e, b"r").and_then(|r| {
                            formualizer_common::coord::parse_a1_1based(&r)
                                .ok()
                                .map(|(row, col, _, _)| (row, col))
                        });
                        if let Some((row, col)) = position {
                            if empty {
                                // A saved blank.
                            } else {
                                cell = Some((row, col, local_attr(&xml, e, b"t")));
                                value = None;
                            }
                        }
                    }
                    b"v" if cell.is_some() && !empty => {
                        in_value = true;
                        value = Some(String::new());
                    }
                    _ => {}
                }
            }
            Event::Text(t) if in_value => {
                let text = t.xml10_content().ok()?;
                value.get_or_insert_with(String::new).push_str(&text);
            }
            Event::GeneralRef(entity) if in_value => {
                CalamineAdapter::append_xml_entity(&entity, value.get_or_insert_with(String::new))
                    .ok()?;
            }
            Event::End(ref e) => match e.local_name().as_ref() {
                b"v" => in_value = false,
                b"cell" => {
                    if let (Some((row, col, kind)), Some(text), Some(index)) =
                        (cell.take(), value.take(), sheet)
                    {
                        saved.entry(index).or_default().push((
                            row,
                            col,
                            cell_value(kind.as_deref(), &text),
                        ));
                    }
                }
                b"sheetData" => sheet = None,
                b"externalBook" => in_book = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if names.is_empty() {
        return None;
    }
    let mut book = ExternalBook::new();
    for (index, name) in names.into_iter().enumerate() {
        let sheet = book.add_sheet(name);
        for (row, col, value) in saved.remove(&index).unwrap_or_default() {
            sheet.set(row, col, value);
        }
    }
    Some(book)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_saved_values_by_sheet_position() {
        let part = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><externalBook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"><sheetNames><sheetName val="Rates"/><sheetName val="Q&amp;A"/></sheetNames><sheetDataSet><sheetData sheetId="0" refreshError="1"/><sheetData sheetId="1" refreshError="1"><row r="2"><cell r="B2"><v>3507</v></cell><cell r="C2" t="str"><v>a &lt;b&gt;</v></cell><cell r="D2" t="b"><v>1</v></cell><cell r="E2" t="e"><v>#DIV/0!</v></cell><cell r="F2"/></row></sheetData></sheetDataSet></externalBook></externalLink>"#;
        let book = parse_external_book(part).expect("workbook link");
        assert_eq!(book.sheets().len(), 2);
        assert_eq!(book.sheet("rates").unwrap().extent(), (0, 0));
        let qa = book.sheet("Q&A").unwrap();
        assert_eq!(qa.get(2, 2), LiteralValue::Number(3507.0));
        assert_eq!(qa.get(2, 3), LiteralValue::Text("a <b>".into()));
        assert_eq!(qa.get(2, 4), LiteralValue::Boolean(true));
        assert!(
            matches!(qa.get(2, 5), LiteralValue::Error(e) if e.kind == formualizer_common::ExcelErrorKind::Div)
        );
        assert_eq!(qa.get(2, 6), LiteralValue::Empty);
        assert_eq!(qa.extent(), (2, 5));
    }

    #[test]
    fn dde_links_carry_no_book() {
        let part = br#"<externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><ddeLink ddeService="Excel" ddeTopic="x"/></externalLink>"#;
        assert!(parse_external_book(part).is_none());
    }

    #[test]
    fn relationship_targets_resolve_under_xl() {
        assert_eq!(
            member_name("externalLinks/externalLink1.xml"),
            "xl/externalLinks/externalLink1.xml"
        );
        assert_eq!(
            member_name("/xl/externalLinks/externalLink2.xml"),
            "xl/externalLinks/externalLink2.xml"
        );
    }
}
