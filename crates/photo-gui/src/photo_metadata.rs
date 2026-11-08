//! Bounded, read-only container metadata inspection; never decodes image pixels.
//! References: IPTC IIM 4.2; ICC.1:2022; CIPA Exif (via kamadak-exif).
use photo_core::metadata::{EmbeddedMetadata, EmbeddedProperty};
const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_METADATA: usize = 1024 * 1024;
const MAX_FIELDS: usize = 256;
const MAX_VALUE: usize = 4096;
fn prop(out: &mut EmbeddedMetadata, ns: &str, name: &str, value: impl AsRef<str>) {
    if out.properties.len() < MAX_FIELDS {
        out.properties.push(EmbeddedProperty {
            namespace: ns.into(),
            name: name.into(),
            value: value.as_ref().chars().take(MAX_VALUE).collect(),
        });
    } else {
        warn(
            out,
            "Metadata property count exceeds the 256-field inspection limit",
        );
    }
}
fn warn(out: &mut EmbeddedMetadata, text: &str) {
    if out.warnings.len() < 16 && !out.warnings.iter().any(|s| s == text) {
        out.warnings.push(text.into());
    }
}
fn be32(b: &[u8]) -> Option<usize> {
    Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?) as usize)
}
// Preflight IFD graph before the library: duplicate child pointers can otherwise
// multiply a small input into a very large number of parsed fields.
fn bounded_ifds(data: &[u8]) -> bool {
    let little = data.starts_with(b"II");
    let u16_at = |p: usize| -> Option<u16> {
        let b: [u8; 2] = data.get(p..p.checked_add(2)?)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |p: usize| -> Option<usize> {
        let b: [u8; 4] = data.get(p..p.checked_add(4)?)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    };
    let Some(first) = u32_at(4) else {
        return false;
    };
    let mut pending = vec![first];
    let mut visited = std::collections::HashSet::new();
    let mut fields = 0usize;
    while let Some(offset) = pending.pop() {
        if offset == 0 {
            continue;
        }
        if !visited.insert(offset) || visited.len() > 32 {
            return false;
        }
        let Some(count) = u16_at(offset).map(usize::from) else {
            return false;
        };
        fields = fields.saturating_add(count);
        if fields > MAX_FIELDS {
            return false;
        }
        let Some(start) = offset.checked_add(2) else {
            return false;
        };
        let Some(end) = start.checked_add(count * 12) else {
            return false;
        };
        if data.get(start..end).is_none() {
            return false;
        }
        for p in (start..end).step_by(12) {
            if matches!(u16_at(p), Some(0x8769 | 0x8825 | 0xa005)) {
                if u16_at(p + 2) != Some(4) || u32_at(p + 4) != Some(1) {
                    return false;
                }
                let Some(child) = u32_at(p + 8) else {
                    return false;
                };
                pending.push(child);
            }
        }
        let Some(next) = u32_at(end) else {
            return false;
        };
        pending.push(next);
    }
    true
}
fn exif_data(data: &[u8], out: &mut EmbeddedMetadata) {
    if data.len() > MAX_METADATA {
        warn(out, "EXIF metadata exceeds the 1 MiB inspection limit");
        return;
    }
    if !bounded_ifds(data) {
        warn(
            out,
            "EXIF IFD graph is malformed or exceeds inspection limits",
        );
        return;
    }
    match exif::Reader::new().read_raw(data.to_vec()) {
        Ok(exif) => {
            for field in exif.fields().take(MAX_FIELDS) {
                prop(
                    out,
                    "EXIF",
                    &format!("{} ({})", field.tag, field.ifd_num),
                    field.display_value().with_unit(&exif).to_string(),
                );
            }
        }
        Err(_) => warn(out, "EXIF metadata is malformed or unsupported"),
    }
}
fn iptc(data: &[u8], out: &mut EmbeddedMetadata) {
    let mut p = 0;
    let mut utf8 = false;
    while p + 5 <= data.len() {
        if data[p] != 0x1c {
            warn(out, "Malformed IPTC-IIM dataset");
            break;
        }
        let record = data[p + 1];
        let dataset = data[p + 2];
        let length = u16::from_be_bytes([data[p + 3], data[p + 4]]);
        p += 5;
        let n = if length & 0x8000 != 0 {
            let k = (length & 0x7fff) as usize;
            if k == 0 || k > 4 || p + k > data.len() {
                warn(out, "Malformed IPTC-IIM length");
                break;
            }
            let mut n = 0usize;
            for b in &data[p..p + k] {
                n = (n << 8) | *b as usize;
            }
            p += k;
            n
        } else {
            length as usize
        };
        let Some(value) = data.get(p..p.saturating_add(n)) else {
            warn(out, "Truncated IPTC-IIM dataset");
            break;
        };
        p += n;
        if record == 1 && dataset == 90 {
            utf8 = value == b"\x1b%G";
        }
        let name = match (record, dataset) {
            (2, 25) => "Keywords",
            (2, 80) => "By-line",
            (2, 116) => "Copyright Notice",
            (2, 120) => "Caption/Abstract",
            (2, 5) => "Object Name",
            _ => continue,
        };
        // Without a declared character set preserve readable UTF-8 and flag ambiguity.
        if !utf8 && !value.is_ascii() {
            warn(
                out,
                "IPTC text has no supported declared character set; displayed as lossy UTF-8",
            );
        }
        prop(out, "IPTC-IIM", name, String::from_utf8_lossy(value));
    }
}
fn photoshop(data: &[u8], out: &mut EmbeddedMetadata) {
    let Some(mut b) = data.strip_prefix(b"Photoshop 3.0\0") else {
        return;
    };
    while !b.is_empty() {
        if b.len() < 7 || &b[..4] != b"8BIM" {
            warn(out, "Malformed Photoshop image resource");
            return;
        }
        let id = u16::from_be_bytes([b[4], b[5]]);
        let name = 1 + b[6] as usize;
        let offset = 6 + name + (name % 2);
        let Some(n) = b.get(offset..).and_then(be32) else {
            warn(out, "Truncated Photoshop image resource");
            return;
        };
        let start = offset + 4;
        let Some(payload) = b.get(start..start.saturating_add(n)) else {
            warn(out, "Truncated Photoshop image resource");
            return;
        };
        if id == 0x0404 {
            iptc(payload, out);
        }
        let end = start + n + n % 2;
        let Some(rest) = b.get(end..) else {
            return;
        };
        b = rest;
    }
}
fn icc(data: &[u8], out: &mut EmbeddedMetadata) {
    if data.len() < 132
        || data.get(36..40) != Some(b"acsp")
        || be32(data).is_none_or(|n| n > data.len() || n < 132)
    {
        warn(out, "Malformed or unsupported ICC profile");
        return;
    }
    for (name, range) in [
        ("Device class", 12..16),
        ("Color space", 16..20),
        ("Connection space", 20..24),
        ("Creator", 80..84),
    ] {
        prop(out, "ICC", name, String::from_utf8_lossy(&data[range]));
    }
    prop(
        out,
        "ICC",
        "Version",
        format!("{}.{}.{}", data[8], data[9] >> 4, data[9] & 15),
    );
    let count = be32(&data[128..]).unwrap_or(0).min(256);
    for i in 0..count {
        let Some(entry) = data.get(132 + i * 12..144 + i * 12) else {
            break;
        };
        if &entry[..4] != b"desc" {
            continue;
        }
        let offset = be32(&entry[4..]).unwrap_or(usize::MAX);
        let n = be32(&entry[8..]).unwrap_or(0);
        let Some(tag) = data.get(offset..offset.saturating_add(n)) else {
            continue;
        };
        if tag.starts_with(b"desc") && tag.len() >= 12 {
            let len = be32(&tag[8..]).unwrap_or(0);
            if let Some(text) = tag.get(12..12usize.saturating_add(len)) {
                prop(
                    out,
                    "ICC",
                    "Description",
                    String::from_utf8_lossy(text).trim_end_matches('\0'),
                );
            }
        } else if tag.starts_with(b"mluc") && tag.len() >= 28 {
            let len = be32(&tag[20..]).unwrap_or(0);
            let offset = be32(&tag[24..]).unwrap_or(usize::MAX);
            if let Some(text) = tag.get(offset..offset.saturating_add(len)) {
                let units: Vec<_> = text
                    .chunks_exact(2)
                    .take(MAX_VALUE)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect();
                prop(out, "ICC", "Description", String::from_utf16_lossy(&units));
            }
        }
    }
}
pub(super) fn inspect_bytes(data: &[u8]) -> EmbeddedMetadata {
    let mut out = EmbeddedMetadata::default();
    if data.len() > MAX_FILE {
        warn(&mut out, "File exceeds the 64 MiB inspection limit");
        return out;
    }
    if let Ok(format) = image::guess_format(data) {
        prop(&mut out, "Image", "Format", format!("{format:?}"));
        if let Ok((w, h)) =
            image::ImageReader::with_format(std::io::Cursor::new(data), format).into_dimensions()
        {
            prop(&mut out, "Image", "Dimensions", format!("{w} × {h}"));
        }
    }
    let mut budget = 0usize;
    let mut consume = |bytes: &[u8], kind: u8, out: &mut EmbeddedMetadata| {
        budget = budget.saturating_add(bytes.len());
        if budget > MAX_METADATA {
            warn(out, "Embedded metadata exceeds the 1 MiB inspection limit");
            return;
        }
        match kind {
            1 => exif_data(bytes, out),
            2 => photoshop(bytes, out),
            3 => icc(bytes, out),
            _ => {}
        }
    };
    if data.starts_with(b"\xff\xd8") {
        let mut p = 2;
        let mut parts: Vec<(u8, u8, Vec<u8>)> = Vec::new();
        let mut icc_size = 0usize;
        while p < data.len() {
            if data[p] != 0xff {
                break;
            }
            while p < data.len() && data[p] == 0xff {
                p += 1;
            }
            let Some(&marker) = data.get(p) else {
                break;
            };
            p += 1;
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let Some(len) = data.get(p..p + 2) else {
                warn(&mut out, "Truncated JPEG segment");
                break;
            };
            let len = u16::from_be_bytes([len[0], len[1]]) as usize;
            if len < 2 {
                warn(&mut out, "Malformed JPEG segment length");
                break;
            }
            let Some(b) = data.get(p + 2..p.saturating_add(len)) else {
                warn(&mut out, "Truncated JPEG segment");
                break;
            };
            p += len;
            if marker == 0xe1
                && let Some(tiff) = b.strip_prefix(b"Exif\0\0")
            {
                consume(tiff, 1, &mut out);
            }
            if marker == 0xed {
                consume(b, 2, &mut out);
            }
            if marker == 0xe2 && b.starts_with(b"ICC_PROFILE\0") && b.len() >= 14 {
                icc_size = icc_size.saturating_add(b.len() - 14);
                if icc_size <= MAX_METADATA && parts.len() < 255 {
                    parts.push((b[12], b[13], b[14..].to_vec()));
                } else {
                    warn(&mut out, "ICC profile exceeds inspection limits");
                }
            }
        }
        if !parts.is_empty() && icc_size <= MAX_METADATA {
            parts.sort_by_key(|p| p.0);
            let count = parts[0].1 as usize;
            if count == parts.len()
                && parts
                    .iter()
                    .enumerate()
                    .all(|(i, p)| p.0 as usize == i + 1 && p.1 as usize == count)
            {
                let profile: Vec<u8> = parts.into_iter().flat_map(|p| p.2).collect();
                consume(&profile, 3, &mut out);
            } else {
                warn(&mut out, "Incomplete or duplicate ICC profile segments");
            }
        }
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        if let Some(h) = data.get(24..26) {
            prop(&mut out, "Image", "PNG bit depth", h[0].to_string());
            prop(
                &mut out,
                "Image",
                "PNG color type",
                match h[1] {
                    0 => "Grayscale",
                    2 => "RGB",
                    3 => "Indexed",
                    4 => "Grayscale with alpha",
                    6 => "RGBA",
                    _ => "Unknown",
                },
            );
        }
        let mut p = 8;
        while let Some(header) = data.get(p..p + 8) {
            let n = be32(header).unwrap_or(0);
            let Some(b) = data.get(p + 8..(p + 8).saturating_add(n)) else {
                warn(&mut out, "Truncated PNG chunk");
                break;
            };
            if &header[4..8] == b"eXIf" {
                consume(b, 1, &mut out);
            }
            if &header[4..8] == b"iCCP"
                && let Some(end) = b.iter().position(|b| *b == 0)
            {
                prop(
                    &mut out,
                    "ICC",
                    "PNG profile name",
                    String::from_utf8_lossy(&b[..end.min(79)]),
                );
                warn(
                    &mut out,
                    "Compressed PNG ICC profile identity is not inspected",
                );
            }
            p = match p.checked_add(n).and_then(|v| v.checked_add(12)) {
                Some(p) => p,
                None => break,
            };
            if &header[4..8] == b"IEND" {
                break;
            }
        }
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        let end = data
            .get(4..8)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize + 8)
            .unwrap_or(0)
            .min(data.len());
        let mut p = 12;
        while let Some(h) = data.get(p..p + 8).filter(|_| p + 8 <= end) {
            let n = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
            let Some(b) = data
                .get(p + 8..(p + 8).saturating_add(n))
                .filter(|_| p + 8 + n <= end)
            else {
                warn(&mut out, "Truncated WebP chunk");
                break;
            };
            if &h[..4] == b"EXIF" {
                consume(b.strip_prefix(b"Exif\0\0").unwrap_or(b), 1, &mut out);
            }
            if &h[..4] == b"ICCP" {
                consume(b, 3, &mut out);
            }
            p = p.saturating_add(8 + n + n % 2);
        }
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        consume(data, 1, &mut out);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn jpeg_segment(marker: u8, b: &[u8]) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, marker];
        v.extend_from_slice(&((b.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(b);
        v.extend_from_slice(&[0xff, 0xd9]);
        v
    }
    #[test]
    fn synthetic_exif_orientation() {
        let mut b =
            b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
        let o = inspect_bytes(&jpeg_segment(0xe1, &b));
        assert!(o.properties.iter().any(|p| p.name.contains("Orientation")));
        b.truncate(10);
        assert!(!inspect_bytes(&jpeg_segment(0xe1, &b)).warnings.is_empty());
    }
    #[test]
    fn synthetic_iptc() {
        let mut b = b"Photoshop 3.0\0".to_vec();
        b.extend_from_slice(b"8BIM\x04\x04\0\0\0\0\0\x0a\x1c\x02\x78\0\x05hello");
        let o = inspect_bytes(&jpeg_segment(0xed, &b));
        assert!(
            o.properties
                .iter()
                .any(|p| p.name == "Caption/Abstract" && p.value == "hello")
        );
    }
    #[test]
    fn icc_identity_and_truncation() {
        let mut profile = vec![0u8; 132];
        profile[..4].copy_from_slice(&132u32.to_be_bytes());
        profile[8] = 4;
        profile[9] = 0x40;
        profile[12..16].copy_from_slice(b"mntr");
        profile[16..20].copy_from_slice(b"RGB ");
        profile[20..24].copy_from_slice(b"XYZ ");
        profile[36..40].copy_from_slice(b"acsp");
        let mut out = EmbeddedMetadata::default();
        icc(&profile, &mut out);
        assert!(
            out.properties
                .iter()
                .any(|p| p.name == "Color space" && p.value == "RGB ")
        );
        profile.truncate(100);
        icc(&profile, &mut out);
        assert!(!out.warnings.is_empty());
    }
    #[test]
    fn rejects_repeated_and_oversized_ifd_graphs() {
        let mut bytes = b"II*\0\x08\0\0\0\x02\0".to_vec();
        for _ in 0..2 {
            bytes.extend_from_slice(&[0x69, 0x87, 4, 0, 1, 0, 0, 0, 38, 0, 0, 0]);
        }
        bytes.extend_from_slice(&[0; 10]);
        assert!(!bounded_ifds(&bytes));
        let mut huge = b"II*\0\x08\0\0\0".to_vec();
        huge.extend_from_slice(&257u16.to_le_bytes());
        huge.extend(vec![0; 257 * 12 + 4]);
        assert!(!bounded_ifds(&huge));
    }
    #[test]
    fn malformed_and_limits() {
        for b in [
            &[][..],
            b"\xff\xd8\xff\xe1\0\x01",
            b"RIFF\xff\xff\xff\xffWEBPEXIF\xff\xff\xff\xff",
        ] {
            let _ = inspect_bytes(b);
        }
        let o = inspect_bytes(&vec![0; MAX_FILE + 1]);
        assert_eq!(o.warnings.len(), 1);
        let mut o = EmbeddedMetadata::default();
        for _ in 0..300 {
            prop(&mut o, "a", "b", "x".repeat(5000));
        }
        assert_eq!(o.properties.len(), 256);
        assert_eq!(o.properties[0].value.len(), 4096);
    }
}
