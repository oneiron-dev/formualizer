//! Surgical ZIP32 package edits. ZIP7 supplies compression and CRC generation;
//! original local/central metadata is retained, not normalized by raw_copy_file.
//! Admission has rejected ZIP64, descriptors and extra metadata other than the
//! Office growth-hint padding, which carries no sizes or offsets. Added parts
//! follow the last member, deflated, with the first member's times and no
//! extra metadata, and their directory entries follow the saved ones.
use super::super::{BoundedOutput, Patch, apply_patches};
use super::{
    Archive, BTreeMap, IoError, XlsxRecalculateOptions, checkpoint, u16_at, u32_at, unsupported,
};
use std::io::{Cursor, Write};
use zip::{ZipArchive, ZipWriter};

pub(in crate::cache_recalculate) fn rewrite(
    bytes: &[u8],
    archive: &mut Archive<'_>,
    replacements: &BTreeMap<String, Vec<u8>>,
    additions: &[(String, Vec<u8>)],
    options: &XlsxRecalculateOptions,
) -> Result<Vec<u8>, IoError> {
    let mut at = archive.central_directory_start() as usize;
    let mut headers = Vec::new();
    let mut changes = Vec::new();
    let mut patches = Vec::new();
    for _ in 0..archive.len() {
        checkpoint(&options.cancel)?;
        let length = u16_at(bytes, at + 28)?;
        let name = std::str::from_utf8(&bytes[at + 46..at + 46 + length])
            .map_err(|e| IoError::from_backend("zip-name", e))?;
        let local = u32_at(bytes, at + 42)?;
        headers.push((at, local));
        if let Some(data) = replacements.get(name) {
            let source = archive
                .by_name(name)
                .map_err(|e| IoError::from_backend("zip", e))?;
            let start = source.data_start() as usize;
            let end = start + source.compressed_size() as usize;
            let mut writer = ZipWriter::new(BoundedOutput {
                cursor: Cursor::new(Vec::new()),
                limit: options.limits.max_output_bytes,
            });
            writer
                .start_file("payload", source.options())
                .map_err(|e| IoError::from_backend("zip", e))?;
            for chunk in data.chunks(64 * 1024) {
                checkpoint(&options.cancel)?;
                writer.write_all(chunk)?;
            }
            let encoded = writer
                .finish()
                .map_err(|e| IoError::from_backend("zip", e))?
                .cursor
                .into_inner();
            let mut temporary = ZipArchive::new(Cursor::new(&encoded))
                .map_err(|e| IoError::from_backend("zip", e))?;
            let payload = temporary
                .by_index(0)
                .map_err(|e| IoError::from_backend("zip", e))?;
            let mut fields = Vec::with_capacity(12);
            fields.extend_from_slice(&payload.crc32().to_le_bytes());
            fields.extend_from_slice(
                &u32::try_from(payload.compressed_size())
                    .map_err(|_| unsupported("ZIP32 compressed size overflow", name))?
                    .to_le_bytes(),
            );
            fields.extend_from_slice(
                &u32::try_from(data.len())
                    .map_err(|_| unsupported("ZIP32 expanded size overflow", name))?
                    .to_le_bytes(),
            );
            let body = payload.data_start() as usize;
            let compressed = encoded[body..body + payload.compressed_size() as usize].to_vec();
            changes.push((end, compressed.len() as i128 - (end - start) as i128));
            patches.push(Patch {
                span: local + 14..local + 26,
                replacement: fields.clone(),
            });
            patches.push(Patch {
                span: at + 16..at + 28,
                replacement: fields,
            });
            patches.push(Patch {
                span: start..end,
                replacement: compressed,
            });
        }
        at += 46 + length + u16_at(bytes, at + 30)? + u16_at(bytes, at + 32)?;
    }
    if changes.len() != replacements.len() {
        return Err(unsupported("unmatched package replacement", "XLSX output"));
    }
    changes.sort_by_key(|(end, _)| *end);
    let mut delta = 0;
    for (_, change) in &mut changes {
        delta += *change;
        *change = delta;
    }
    let relocate = |old: usize| -> Result<Vec<u8>, IoError> {
        let index = changes.partition_point(|(end, _)| *end <= old);
        let delta = if index == 0 { 0 } else { changes[index - 1].1 };
        Ok(u32::try_from(old as i128 + delta)
            .map_err(|_| unsupported("ZIP32 relocated offset overflow", "XLSX output"))?
            .to_le_bytes()
            .to_vec())
    };
    for (central, local) in headers {
        patches.push(Patch {
            span: central + 42..central + 46,
            replacement: relocate(local)?,
        });
    }
    let directory = archive.central_directory_start() as usize;
    if additions.is_empty() {
        patches.push(Patch {
            span: at + 16..at + 20,
            replacement: relocate(directory)?,
        });
    } else {
        let start = u32_at(&relocate(directory)?, 0)?;
        let (locals, centrals) = added_members(bytes, directory, start, additions, options)?;
        let entries = u16::try_from(archive.len() + additions.len())
            .ok()
            .filter(|n| usize::from(*n) <= options.limits.max_entries && *n != u16::MAX)
            .ok_or_else(|| unsupported("ZIP entry count limit", "XLSX output"))?;
        let size = u32::try_from(at - directory + centrals.len())
            .map_err(|_| unsupported("ZIP32 directory size overflow", "XLSX output"))?;
        let offset = u32::try_from(start + locals.len())
            .map_err(|_| unsupported("ZIP32 relocated offset overflow", "XLSX output"))?;
        let mut footer = Vec::with_capacity(12);
        footer.extend_from_slice(&entries.to_le_bytes());
        footer.extend_from_slice(&entries.to_le_bytes());
        footer.extend_from_slice(&size.to_le_bytes());
        footer.extend_from_slice(&offset.to_le_bytes());
        patches.push(Patch {
            span: directory..directory,
            replacement: locals,
        });
        patches.push(Patch {
            span: at..at,
            replacement: centrals,
        });
        patches.push(Patch {
            span: at + 8..at + 20,
            replacement: footer,
        });
    }
    checkpoint(&options.cancel)?;
    let result = apply_patches(bytes, patches, options.limits.max_output_bytes)?;
    checkpoint(&options.cancel)?;
    Ok(result)
}

/// The local entries (from offset `start`) and directory entries of the
/// added parts: deflated, ASCII-named, without extra fields, with the first
/// member's creator version and modification time.
fn added_members(
    bytes: &[u8],
    directory: usize,
    start: usize,
    additions: &[(String, Vec<u8>)],
    options: &XlsxRecalculateOptions,
) -> Result<(Vec<u8>, Vec<u8>), IoError> {
    let made_by = bytes
        .get(directory + 4..directory + 6)
        .ok_or_else(|| unsupported("truncated ZIP metadata", "XLSX output"))?;
    let time = bytes
        .get(directory + 12..directory + 16)
        .ok_or_else(|| unsupported("truncated ZIP metadata", "XLSX output"))?;
    let mut locals = Vec::new();
    let mut centrals = Vec::new();
    for (name, data) in additions {
        checkpoint(&options.cancel)?;
        if !name.is_ascii() {
            return Err(unsupported("non-ASCII added part name", "XLSX output"));
        }
        let mut writer = ZipWriter::new(BoundedOutput {
            cursor: Cursor::new(Vec::new()),
            limit: options.limits.max_output_bytes,
        });
        writer
            .start_file(
                "payload",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .map_err(|e| IoError::from_backend("zip", e))?;
        writer.write_all(data)?;
        let encoded = writer
            .finish()
            .map_err(|e| IoError::from_backend("zip", e))?
            .cursor
            .into_inner();
        let mut temporary =
            ZipArchive::new(Cursor::new(&encoded)).map_err(|e| IoError::from_backend("zip", e))?;
        let payload = temporary
            .by_index(0)
            .map_err(|e| IoError::from_backend("zip", e))?;
        let body = payload.data_start() as usize;
        let compressed = &encoded[body..body + payload.compressed_size() as usize];
        let overflow = || unsupported("ZIP32 size overflow", name.as_str());
        let mut fields = Vec::with_capacity(12);
        fields.extend_from_slice(&payload.crc32().to_le_bytes());
        fields.extend_from_slice(
            &u32::try_from(compressed.len())
                .map_err(|_| overflow())?
                .to_le_bytes(),
        );
        fields.extend_from_slice(
            &u32::try_from(data.len())
                .map_err(|_| overflow())?
                .to_le_bytes(),
        );
        let name_length = u16::try_from(name.len()).map_err(|_| overflow())?;
        let offset = u32::try_from(start + locals.len()).map_err(|_| overflow())?;
        // Version 2.0 (deflate), no flags, deflate.
        let common = [20u8, 0, 0, 0, 8, 0];
        locals.extend_from_slice(b"PK\x03\x04");
        locals.extend_from_slice(&common);
        locals.extend_from_slice(time);
        locals.extend_from_slice(&fields);
        locals.extend_from_slice(&name_length.to_le_bytes());
        locals.extend_from_slice(&[0, 0]);
        locals.extend_from_slice(name.as_bytes());
        locals.extend_from_slice(compressed);
        centrals.extend_from_slice(b"PK\x01\x02");
        centrals.extend_from_slice(made_by);
        centrals.extend_from_slice(&common);
        centrals.extend_from_slice(time);
        centrals.extend_from_slice(&fields);
        centrals.extend_from_slice(&name_length.to_le_bytes());
        // No extra field, comment, disk number or attributes.
        centrals.extend_from_slice(&[0; 12]);
        centrals.extend_from_slice(&offset.to_le_bytes());
        centrals.extend_from_slice(name.as_bytes());
    }
    Ok((locals, centrals))
}
