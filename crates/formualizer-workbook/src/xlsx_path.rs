//! Shared package helpers: relationship resolution, part and attribute
//! reads. No filesystem access.
use quick_xml::Reader as XmlReader;
use quick_xml::events::BytesStart;
use std::io::{self, Read, Seek};
use zip::ZipArchive;

pub(crate) fn resolve(source: &str, target: &str) -> io::Result<String> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    if target.contains(['\\', '\0', '?', '#', ':']) {
        return Err(invalid("invalid internal relationship target"));
    }
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        source
            .rsplit_once('/')
            .map(|(p, _)| p.split('/').collect())
            .unwrap_or_default()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| invalid("relationship escapes archive root"))?;
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(invalid("empty relationship target"));
    }
    Ok(parts.join("/"))
}

/// The bytes of package member `name`, if it exists and reads.
pub(crate) fn read_member<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The unescaped value of the attribute with local name `local`.
pub(crate) fn local_attr<R>(
    xml: &XmlReader<R>,
    start: &BytesStart<'_>,
    local: &[u8],
) -> Option<String> {
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
