//! Bounded, read-only embedded XMP inspection. Never decodes pixels or registers files.
use photo_core::metadata::{EmbeddedMetadata, EmbeddedProperty};
use std::{fs::OpenOptions, io::Read, path::Path};

pub(super) const JPEG_XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const JPEG_EXTENDED_HEADER: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";
const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_XMP: usize = 1024 * 1024;
const MAX_FIELDS: usize = 256;

pub(super) fn inspect(path: &Path) -> EmbeddedMetadata {
    let mut result = EmbeddedMetadata::default();
    let read = (|| {
        let before = photo_core::rename::capture(path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut input = options.open(path)?;
        if photo_core::rename::capture(path)? != before {
            return Err(std::io::Error::other("Source changed before metadata read"));
        }
        let mut bytes = Vec::new();
        input
            .by_ref()
            .take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)?;
        if photo_core::rename::capture(path)? != before {
            return Err(std::io::Error::other("Source changed during metadata read"));
        }
        Ok::<_, std::io::Error>(bytes)
    })();
    match read {
        Ok(bytes) if bytes.len() <= MAX_FILE => result = inspect_bytes(&bytes),
        Ok(_) => result.warnings.push(
            "Embedded metadata inspection limited to files ≤64 MiB; file was not inspected".into(),
        ),
        Err(error) => result
            .warnings
            .push(format!("Cannot read embedded metadata: {error}")),
    }
    result
}

fn packet(bytes: &[u8], out: &mut EmbeddedMetadata) {
    if out.raw_xmp.len() >= 16 {
        out.warnings
            .push("XMP packet count exceeds inspection limit".into());
        return;
    }
    let used: usize = out.raw_xmp.iter().map(String::len).sum();
    if bytes.len() > MAX_XMP.saturating_sub(used) {
        out.warnings
            .push("XMP exceeds the 1 MiB total inspection limit".into());
        return;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            out.warnings.push("XMP packet is not valid UTF-8".into());
            return;
        }
    };
    out.raw_xmp.push(text.to_owned());
    match parse_packet(bytes) {
        Ok(properties) => {
            let remaining = MAX_FIELDS.saturating_sub(out.properties.len());
            if properties.len() >= remaining {
                out.warnings.push("XMP property display limited to 256 fields; inspect raw packet for complete content".into());
            }
            if properties.iter().any(|p| p.value.chars().count() >= 4096) {
                out.warnings
                    .push("Long XMP values are limited to 4096 characters".into());
            }
            out.properties
                .extend(properties.into_iter().take(remaining));
        }
        Err(error) => out.warnings.push(format!("Cannot parse XMP: {error}")),
    }
}

pub(super) fn inspect_bytes(bytes: &[u8]) -> EmbeddedMetadata {
    let mut result = EmbeddedMetadata::default();
    if bytes.len() > MAX_FILE {
        result
            .warnings
            .push("Embedded metadata exceeds 64 MiB file limit".into());
        return result;
    }
    scan_xmp(bytes, &mut result);
    for mut other in [
        super::generation_metadata::inspect_bytes(bytes),
        super::photo_metadata::inspect_bytes(bytes),
    ] {
        result.properties.append(&mut other.properties);
        result.raw_xmp.append(&mut other.raw_xmp);
        result.raw_payloads.append(&mut other.raw_payloads);
        result.warnings.append(&mut other.warnings);
        result.c2pa_present |= other.c2pa_present;
    }
    if result.properties.len() > MAX_FIELDS {
        result
            .warnings
            .push("Embedded properties limited to 256 fields".into());
    }
    result.properties.truncate(MAX_FIELDS);
    for property in &mut result.properties {
        property.value = property.value.chars().take(4096).collect();
        property.name = property.name.chars().take(4096).collect();
        property.namespace = property.namespace.chars().take(4096).collect();
    }
    bound_raw_payloads(&mut result);
    for warning in &mut result.warnings {
        *warning = warning.chars().take(4096).collect();
    }
    result.warnings.truncate(32);
    result
}

fn bound_raw_payloads(result: &mut EmbeddedMetadata) {
    let mut remaining = MAX_XMP;
    let mut omitted = 0;
    let mut keep = |text: &str| {
        if text.len() <= remaining {
            remaining -= text.len();
            true
        } else {
            omitted += 1;
            false
        }
    };
    result.raw_xmp.retain(|text| keep(text));
    result.raw_payloads.retain(|payload| keep(&payload.text));
    if omitted > 0 {
        result.warnings.insert(0, format!("Omitted {omitted} whole embedded payload(s): 1 MiB combined raw limit. Retained packets are complete."));
    }
}

fn scan_xmp(bytes: &[u8], out: &mut EmbeddedMetadata) {
    let parsed = if bytes.starts_with(b"\xff\xd8") {
        jpeg(bytes, out)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes, out)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp(bytes, out)
    } else {
        out.warnings
            .push("Embedded metadata inspection supports JPEG, PNG and WebP".into());
        Ok(())
    };
    if let Err(error) = parsed {
        out.warnings.push(error);
    }
}

fn jpeg(bytes: &[u8], out: &mut EmbeddedMetadata) -> Result<(), String> {
    let mut offset = 2;
    while offset < bytes.len() {
        if out.warnings.len() >= 31 {
            return Err("JPEG inspection warning limit reached".into());
        }
        if bytes[offset] != 0xff {
            return Err("Invalid JPEG marker".into());
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset).ok_or("Truncated JPEG marker")?;
        offset += 1;
        if marker == 0xda || marker == 0xd9 {
            return Ok(());
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = bytes
            .get(offset..offset + 2)
            .ok_or("Truncated JPEG segment")?;
        let length = u16::from_be_bytes([length[0], length[1]]) as usize;
        if length < 2 {
            return Err("Invalid JPEG segment length".into());
        }
        let segment = bytes
            .get(offset + 2..offset + length)
            .ok_or("Truncated JPEG segment payload")?;
        if marker == 0xe1 {
            if let Some(xml) = segment.strip_prefix(JPEG_XMP_HEADER) {
                packet(xml, out);
            } else if segment.starts_with(JPEG_EXTENDED_HEADER) {
                out.warnings.push("Extended JPEG XMP is present; reconstruction is unsupported, standard packet may be incomplete".into());
            }
        }
        if marker == 0xeb {
            let c2pa = segment.windows(4).any(|s| s == b"c2pa");
            let jumbf = c2pa || segment.windows(4).any(|s| s == b"jumb");
            out.c2pa_present |= c2pa;
            out.warnings.push(if jumbf { "JPEG APP11 contains potential JUMBF/C2PA provenance; signatures have not been verified" } else { "JPEG APP11 present; possible provenance container, not verified or decoded" }.into());
        }
        offset += length;
    }
    Err("JPEG ended before scan/end marker".into())
}

fn png(bytes: &[u8], out: &mut EmbeddedMetadata) -> Result<(), String> {
    let mut offset = 8;
    while offset < bytes.len() {
        if out.warnings.len() >= 31 {
            return Err("PNG inspection warning limit reached".into());
        }
        let header = bytes
            .get(offset..offset + 8)
            .ok_or("Truncated PNG chunk header")?;
        let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .ok_or("PNG chunk length overflow")?;
        let chunk = bytes
            .get(offset + 8..end.checked_sub(4).ok_or("PNG chunk length")?)
            .ok_or("Truncated PNG chunk")?;
        if end > bytes.len() {
            return Err("Truncated PNG CRC".into());
        }
        if &header[4..8] == b"iTXt" {
            png_text(chunk, out)?;
        }
        if &header[4..8] == b"caBX" {
            out.c2pa_present |= chunk.windows(4).any(|s| s == b"c2pa");
            out.warnings.push(
                "PNG JUMBF provenance chunk present; signatures have not been verified".into(),
            );
        }
        offset = end;
        if &header[4..8] == b"IEND" {
            return Ok(());
        }
    }
    Err("PNG ended before IEND".into())
}

fn png_text(chunk: &[u8], out: &mut EmbeddedMetadata) -> Result<(), String> {
    let keyword_end = chunk
        .iter()
        .position(|&b| b == 0)
        .ok_or("Malformed PNG iTXt keyword")?;
    if &chunk[..keyword_end] != b"XML:com.adobe.xmp" {
        return Ok(());
    }
    let rest = chunk.get(keyword_end + 1..).ok_or("Malformed PNG iTXt")?;
    let (&compressed, rest) = rest.split_first().ok_or("Missing iTXt compression flag")?;
    let (&method, mut text) = rest
        .split_first()
        .ok_or("Missing iTXt compression method")?;
    if compressed > 1 || method != 0 {
        return Err("Unsupported PNG XMP compression".into());
    }
    for _ in 0..2 {
        let end = text
            .iter()
            .position(|&b| b == 0)
            .ok_or("Malformed PNG iTXt language/translation")?;
        text = &text[end + 1..];
    }
    if compressed == 0 {
        packet(text, out);
    } else {
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(text)
            .take((MAX_XMP + 1) as u64)
            .read_to_end(&mut decoded)
            .map_err(|e| format!("Invalid compressed PNG XMP: {e}"))?;
        packet(&decoded, out);
    }
    Ok(())
}

fn webp(bytes: &[u8], out: &mut EmbeddedMetadata) -> Result<(), String> {
    let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let end = size
        .checked_add(8)
        .filter(|&n| n <= bytes.len() && n >= 12)
        .ok_or("Truncated/invalid WebP RIFF size")?;
    let mut offset = 12;
    while offset < end {
        if out.warnings.len() >= 31 {
            return Err("WebP inspection warning limit reached".into());
        }
        let header = bytes
            .get(offset..offset + 8)
            .filter(|_| offset + 8 <= end)
            .ok_or("Truncated WebP chunk header")?;
        let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let next = offset
            .checked_add(8)
            .and_then(|n| n.checked_add(length))
            .ok_or("WebP chunk overflow")?;
        let chunk = bytes
            .get(offset + 8..next)
            .filter(|_| next <= end)
            .ok_or("Truncated WebP chunk")?;
        if &header[..4] == b"XMP " {
            packet(chunk, out);
        }
        if &header[..4] == b"C2PA" {
            out.c2pa_present = true;
            out.warnings
                .push("WebP provenance chunk present; signatures have not been verified".into());
        }
        offset = next
            .checked_add(length % 2)
            .filter(|&n| n <= end)
            .ok_or("Missing WebP chunk padding")?;
    }
    Ok(())
}

/// Parse generic RDF attributes and leaf values using expanded namespace paths.
/// DTDs/entities are forbidden; namespace URI and container item position survive.
pub(super) fn parse_packet(bytes: &[u8]) -> Result<Vec<EmbeddedProperty>, String> {
    if bytes.len() > MAX_XMP {
        return Err("Packet exceeds 1 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err("DTD and entity declarations are forbidden".into());
    }
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 8192,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        if fields.len() == MAX_FIELDS {
            break;
        }
        let ancestors: Vec<_> = node.ancestors().filter(|n| n.is_element()).collect();
        if ancestors.len() > 64 {
            return Err("XML nesting exceeds 64 elements".into());
        }
        let mut path = String::new();
        for part in ancestors.iter().rev() {
            let tag = part.tag_name();
            let ns = tag.namespace().unwrap_or("");
            if ns.len() > 4096 || tag.name().len() > 512 {
                return Err("XML name/namespace exceeds inspection limit".into());
            }
            let index = part
                .prev_siblings()
                .filter(|n| n.is_element() && n.tag_name() == tag)
                .count();
            path.push_str(&format!("/{{{ns}}}{}[{index}]", tag.name()));
            if path.len() > 4096 {
                return Err("XML property path exceeds inspection limit".into());
            }
        }
        for attr in node.attributes() {
            if attr.namespace().unwrap_or("").len() > 4096 || attr.name().len() > 512 {
                return Err("XML attribute name/namespace exceeds inspection limit".into());
            }
            if fields.len() == MAX_FIELDS {
                break;
            }
            fields.push(EmbeddedProperty {
                namespace: attr.namespace().unwrap_or("").to_owned(),
                name: format!("{path}/@{}", attr.name()),
                value: attr.value().chars().take(4096).collect(),
            });
        }
        if fields.len() < MAX_FIELDS && !node.children().any(|n| n.is_element()) {
            let value = node.text().unwrap_or("").trim();
            if !value.is_empty() {
                fields.push(EmbeddedProperty {
                    namespace: node.tag_name().namespace().unwrap_or("").to_owned(),
                    name: path,
                    value: value.chars().take(4096).collect(),
                });
            }
        }
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_raw_budget_keeps_whole_json_and_xml_packets() {
        let xml = format!("<x>{}</x>", "a".repeat(MAX_XMP - 32));
        let workflow = r#"{"nodes":[{"type":"Example","text":"whole workflow"}]}"#.to_owned();
        let mut result = EmbeddedMetadata {
            raw_xmp: vec![xml.clone()],
            raw_payloads: vec![
                photo_core::metadata::EmbeddedPayload {
                    kind: "workflow".into(),
                    label: "Workflow".into(),
                    text: workflow,
                },
                photo_core::metadata::EmbeddedPayload {
                    kind: "workflow".into(),
                    label: "Small workflow".into(),
                    text: "{}".into(),
                },
            ],
            ..Default::default()
        };
        bound_raw_payloads(&mut result);
        assert_eq!(result.raw_xmp, vec![xml]);
        assert_eq!(result.raw_payloads.len(), 1);
        assert_eq!(result.raw_payloads[0].text, "{}");
        assert!(result.warnings[0].contains("Omitted 1 whole"));
        assert!(roxmltree::Document::parse(&result.raw_xmp[0]).is_ok());
        assert!(serde_json::from_str::<serde_json::Value>(&result.raw_payloads[0].text).is_ok());
    }
    const XML: &[u8] = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:a="urn:a" xmlns:b="urn:b"><rdf:RDF><rdf:Description a:same="one" b:same="two"><dc:subject><rdf:Bag><rdf:li>forest</rdf:li><rdf:li>portrait</rdf:li></rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    fn jpeg_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xff, marker];
        bytes.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&[0; 4]);
        bytes
    }
    fn own(bytes: &[u8]) -> EmbeddedMetadata {
        let mut result = EmbeddedMetadata::default();
        scan_xmp(bytes, &mut result);
        result
    }
    #[test]
    fn rdf_containers_and_namespace_attributes_remain_distinct() {
        let properties = parse_packet(XML).unwrap();
        assert!(
            properties
                .iter()
                .any(|p| p.namespace == "urn:a" && p.value == "one")
        );
        assert!(
            properties
                .iter()
                .any(|p| p.namespace == "urn:b" && p.value == "two")
        );
        let first = properties.iter().find(|p| p.value == "forest").unwrap();
        let second = properties.iter().find(|p| p.value == "portrait").unwrap();
        assert_ne!(first.name, second.name);
        assert!(first.name.contains("Bag[1]") && second.name.ends_with("li[2]"));
    }
    #[test]
    fn jpeg_standard_extended_and_unverified_jumbf_are_reported() {
        let mut bytes = b"\xff\xd8".to_vec();
        bytes.extend(jpeg_segment(0xe1, &[JPEG_XMP_HEADER, XML].concat()));
        bytes.extend(jpeg_segment(0xe1, JPEG_EXTENDED_HEADER));
        bytes.extend(jpeg_segment(0xeb, b"JP\0\x01jumbc2pa"));
        bytes.extend_from_slice(b"\xff\xd9");
        let result = own(&bytes);
        assert_eq!(result.raw_xmp.len(), 1);
        assert!(result.c2pa_present);
        assert!(result.warnings.iter().any(|w| w.contains("Extended")));
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("not been verified"))
        );
    }
    #[test]
    fn generic_jumbf_containers_do_not_claim_c2pa() {
        let mut jpeg = b"\xff\xd8".to_vec();
        jpeg.extend(jpeg_segment(0xeb, b"JP\0\x01jumbgeneric-box"));
        jpeg.extend_from_slice(b"\xff\xd9");
        let result = own(&jpeg);
        assert!(!result.c2pa_present);
        assert!(result.warnings.iter().any(|w| w.contains("JUMBF")));
        for (payload, expected) in [
            (b"jumbgeneric-box".as_slice(), false),
            (b"jumbc2pa".as_slice(), true),
        ] {
            let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
            png.extend(png_chunk(b"caBX", payload));
            png.extend(png_chunk(b"IEND", &[]));
            let result = own(&png);
            assert_eq!(result.c2pa_present, expected);
            assert!(result.warnings.iter().any(|w| w.contains("JUMBF")));
        }
    }
    #[test]
    fn png_uncompressed_and_compressed_xmp() {
        use std::io::Write;
        for compressed in [false, true] {
            let mut payload = b"XML:com.adobe.xmp\0".to_vec();
            payload.extend_from_slice(&[u8::from(compressed), 0, 0, 0]);
            if compressed {
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(XML).unwrap();
                payload.extend(encoder.finish().unwrap());
            } else {
                payload.extend_from_slice(XML);
            }
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            bytes.extend(png_chunk(b"iTXt", &payload));
            bytes.extend(png_chunk(b"IEND", &[]));
            let result = own(&bytes);
            assert_eq!(result.raw_xmp.len(), 1);
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }
    }
    #[test]
    fn webp_xmp_and_truncated_chunks() {
        let mut body = b"WEBPXMP ".to_vec();
        body.extend_from_slice(&(XML.len() as u32).to_le_bytes());
        body.extend_from_slice(XML);
        if XML.len() % 2 == 1 {
            body.push(0);
        }
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bytes.extend(body);
        assert_eq!(own(&bytes).raw_xmp.len(), 1);
        bytes.pop();
        assert!(!own(&bytes).warnings.is_empty());
        assert!(!own(b"\xff\xd8\xff\xe1\x00\x20").warnings.is_empty());
    }
    #[test]
    fn rejects_dtd_corrupt_large_deep_and_node_heavy_xml() {
        assert!(parse_packet(b"<!DOCTYPE x><x/>").is_err());
        assert!(parse_packet(b"<broken>").is_err());
        assert!(parse_packet(&vec![b'x'; MAX_XMP + 1]).is_err());
        let deep = format!("{}{}", "<n>".repeat(65), "</n>".repeat(65));
        assert!(parse_packet(deep.as_bytes()).is_err());
        let many = format!("<root>{}</root>", "<n/>".repeat(9000));
        assert!(parse_packet(many.as_bytes()).is_err());
        let long = format!("<x>{}</x>", "a".repeat(5000));
        assert_eq!(parse_packet(long.as_bytes()).unwrap()[0].value.len(), 4096);
    }
}
