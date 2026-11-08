//! Read-only, bounded extraction of generation text. Payloads are untrusted data.
//! Formats follow ComfyUI nodes.py SaveImage and A1111 modules/images.py.
use flate2::read::ZlibDecoder;
use photo_core::metadata::{EmbeddedMetadata, EmbeddedPayload, EmbeddedProperty};
use serde_json::Value;
use std::io::Read;

const MAX_SCAN: usize = 64 * 1024 * 1024;
const MAX_TEXT: usize = 1024 * 1024;
const MAX_PROPERTIES: usize = 256;
const MAX_VALUE: usize = 4096;

pub(super) fn inspect_bytes(data: &[u8]) -> EmbeddedMetadata {
    let mut out = EmbeddedMetadata::default();
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return out;
    }
    let bytes = &data[..data.len().min(MAX_SCAN)];
    let mut offset = 8usize;
    let mut remaining = MAX_TEXT;
    let mut chunks = 0;
    while offset + 12 <= bytes.len() {
        chunks += 1;
        if chunks > 65536 {
            warn(&mut out, "PNG chunk inspection limit reached");
            break;
        }
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let Some(end) = offset.checked_add(12).and_then(|n| n.checked_add(size)) else {
            break;
        };
        if end > bytes.len() {
            warn(
                &mut out,
                "PNG chunk is truncated or exceeds the inspection limit",
            );
            break;
        }
        let kind = &bytes[offset + 4..offset + 8];
        let body = &bytes[offset + 8..end - 4];
        if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt") {
            if remaining == 0 {
                warn(&mut out, "Generation metadata text limit reached (1 MiB)");
                break;
            }
            match text_chunk(kind, body, remaining) {
                Ok((key, value)) => {
                    remaining = remaining.saturating_sub(key.len() + value.len());
                    record(&mut out, &key, value);
                }
                Err(message) => {
                    warn(&mut out, message);
                    if message.contains("text limit") {
                        break;
                    }
                }
            }
        }
        offset = end;
        if kind == b"IEND" {
            break;
        }
    }
    out
}

fn warn(out: &mut EmbeddedMetadata, message: &str) {
    if out.warnings.len() < 16 && !out.warnings.iter().any(|s| s == message) {
        out.warnings.push(message.into());
    }
}

fn split_zero(bytes: &[u8]) -> Result<(&[u8], &[u8]), &'static str> {
    let n = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or("Malformed PNG text chunk")?;
    Ok((&bytes[..n], &bytes[n + 1..]))
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn text_chunk(kind: &[u8], body: &[u8], budget: usize) -> Result<(String, String), &'static str> {
    let (key, mut text) = split_zero(body)?;
    if key.is_empty() || key.len() > 79 {
        return Err("Invalid PNG text keyword");
    }
    let key = latin1(key);
    let mut compressed = false;
    let utf8 = kind == b"iTXt";
    if kind == b"zTXt" {
        if text.first() != Some(&0) {
            return Err("Unsupported PNG text compression method");
        }
        text = &text[1..];
        compressed = true;
    } else if utf8 {
        if text.len() < 2 || text[0] > 1 || text[1] != 0 {
            return Err("Invalid PNG international text compression header");
        }
        compressed = text[0] == 1;
        let (_, rest) = split_zero(&text[2..])?; // language
        let (_, rest) = split_zero(rest)?; // translated keyword
        text = rest;
    }
    let limit = budget.saturating_sub(key.len());
    let owned;
    if compressed {
        let mut decoded = Vec::new();
        ZlibDecoder::new(text)
            .take(limit as u64 + 1)
            .read_to_end(&mut decoded)
            .map_err(|_| "Malformed compressed PNG text")?;
        if decoded.len() > limit {
            return Err("Generation metadata text limit reached (1 MiB)");
        }
        owned = decoded;
        text = &owned;
    }
    if text.len() > limit {
        return Err("Generation metadata text limit reached (1 MiB)");
    }
    let value = if utf8 {
        std::str::from_utf8(text)
            .map_err(|_| "Invalid UTF-8 in PNG international text")?
            .to_owned()
    } else {
        latin1(text)
    };
    if value.len() > limit {
        return Err("Generation metadata text limit reached (1 MiB)");
    }
    Ok((key, value))
}

fn property(out: &mut EmbeddedMetadata, namespace: &str, name: &str, value: &str) {
    if out.properties.len() >= MAX_PROPERTIES {
        warn(out, "Generation metadata property limit reached (256)");
        return;
    }
    let mut chars = value.chars();
    let mut value: String = chars.by_ref().take(MAX_VALUE).collect();
    if chars.next().is_some() {
        value.pop();
        value.push('…');
    }
    out.properties.push(EmbeddedProperty {
        namespace: namespace.into(),
        name: name.chars().take(512).collect(),
        value,
    });
}

fn record(out: &mut EmbeddedMetadata, key: &str, text: String) {
    if key.eq_ignore_ascii_case("XML:com.adobe.xmp") {
        return;
    }
    let generation_key = ["prompt", "workflow", "parameters"]
        .iter()
        .any(|name| key.eq_ignore_ascii_case(name));
    if let Ok(json) = serde_json::from_str::<Value>(&text) {
        let comfy = record_comfy(out, key, &json, 0);
        let namespace = if comfy {
            "comfyui"
        } else if generation_key {
            "generation"
        } else {
            "png_text"
        };
        if !comfy {
            if generation_key {
                walk(out, namespace, key, &json, 0);
            } else {
                let summary = match &json {
                    Value::Object(map) => format!("JSON object · {} keys", map.len()),
                    Value::Array(values) => format!("JSON array · {} items", values.len()),
                    _ => json.to_string(),
                };
                property(out, namespace, key, &summary);
            }
        }
        out.raw_payloads.push(EmbeddedPayload {
            kind: namespace.into(),
            label: format!("PNG {key} (JSON)"),
            text,
        });
        return;
    }
    if generation_key && text.trim_start().starts_with(['{', '[']) {
        warn(out, "Malformed generation JSON; original text retained");
    }
    if key.eq_ignore_ascii_case("parameters") {
        parameters(out, &text);
    } else {
        let namespace =
            if key.eq_ignore_ascii_case("prompt") || key.eq_ignore_ascii_case("workflow") {
                "generation"
            } else {
                "png_text"
            };
        property(out, namespace, key, &text);
    }
    // Complete long text remains copyable even under an unfamiliar keyword.
    if generation_key || text.chars().count() > 512 {
        out.raw_payloads.push(EmbeddedPayload {
            kind: if generation_key {
                "generation"
            } else {
                "png_text"
            }
            .into(),
            label: format!("PNG {key}"),
            text,
        });
    }
}

/// Recognize graph shape independently of PNG keyword spelling. Workflow layout
/// and widgets stay in the complete JSON; only executable prompt graphs expand.
fn record_comfy(out: &mut EmbeddedMetadata, path: &str, json: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    let Some(map) = json.as_object() else {
        return false;
    };
    if let Some(nodes) = map.get("nodes").and_then(Value::as_array) {
        property(
            out,
            "comfyui",
            &format!("{path}.node_count"),
            &nodes.len().to_string(),
        );
        let mut types = std::collections::BTreeMap::<&str, usize>::new();
        for node in nodes {
            if let Some(kind) = node.get("type").and_then(Value::as_str) {
                *types.entry(kind).or_default() += 1;
            }
        }
        let mut summary = types
            .iter()
            .take(32)
            .map(|(kind, count)| format!("{kind} × {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        if types.len() > 32 {
            summary.push_str(&format!(" (+{} other types)", types.len() - 32));
        }
        if !summary.is_empty() {
            property(out, "comfyui", &format!("{path}.node_types"), &summary);
        }
        return true;
    }
    if !map.is_empty()
        && map
            .values()
            .any(|node| node.get("class_type").and_then(Value::as_str).is_some())
    {
        walk(out, "comfyui", path, json, 0);
        return true;
    }
    let mut found = false;
    for (key, value) in map {
        if key.eq_ignore_ascii_case("workflow") || key.eq_ignore_ascii_case("prompt") {
            found |= record_comfy(out, &format!("{path}.{key}"), value, depth + 1);
        }
    }
    found
}

fn walk(out: &mut EmbeddedMetadata, namespace: &str, path: &str, json: &Value, depth: usize) {
    if out.properties.len() >= MAX_PROPERTIES {
        warn(out, "Generation metadata property limit reached (256)");
        return;
    }
    if depth > 32 {
        warn(
            out,
            "Generation JSON display depth limit reached; full payload retained",
        );
        return;
    }
    match json {
        Value::Object(map) => {
            for (key, value) in map {
                walk(out, namespace, &format!("{path}.{key}"), value, depth + 1);
                if out.properties.len() >= MAX_PROPERTIES {
                    break;
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                walk(
                    out,
                    namespace,
                    &format!("{path}[{index}]"),
                    value,
                    depth + 1,
                );
                if out.properties.len() >= MAX_PROPERTIES {
                    break;
                }
            }
        }
        Value::String(value) => property(out, namespace, path, value),
        other => property(out, namespace, path, &other.to_string()),
    }
}

fn parameters(out: &mut EmbeddedMetadata, text: &str) {
    // The final settings line is conventionally Steps: ..., Sampler: ..., Seed: ... .
    let settings = text
        .rfind("\nSteps: ")
        .map(|n| n + 1)
        .or_else(|| text.starts_with("Steps: ").then_some(0));
    let prompts = settings.map_or(text, |n| text[..n].trim_end());
    if let Some((positive, negative)) = prompts.split_once("\nNegative prompt: ") {
        property(out, "generation", "Prompt", positive);
        property(out, "generation", "Negative prompt", negative);
    } else if !prompts.is_empty() {
        property(out, "generation", "Prompt", prompts);
    }
    if let Some(start) = settings {
        // Keep the full line as well: extension values can contain commas or colons.
        property(out, "generation", "Settings", &text[start..]);
        for part in text[start..].split(", ") {
            if let Some((name, value)) = part.split_once(": ") {
                property(out, "generation", name, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    fn png(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(&[0; 4]); // CRC is outside this text inspector's remit.
        bytes
    }
    fn zip(bytes: &[u8]) -> Vec<u8> {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(bytes).unwrap();
        z.finish().unwrap()
    }
    #[test]
    fn unfamiliar_comfy_keyword_keeps_complete_graph_and_compact_summary() {
        let workflow = serde_json::json!({"nodes": (0..200).map(|id| serde_json::json!({
            "id": id, "type": "KSampler", "pos": [id, id],
            "widgets_values": ["x".repeat(100), 123], "size": [400, 300]
        })).collect::<Vec<_>>()})
        .to_string();
        for key in ["ComfyUI", "ComfyUI workflow", "Unfamiliar graph key"] {
            let out = inspect_bytes(&png(b"tEXt", format!("{key}\0{workflow}").as_bytes()));
            assert_eq!(out.raw_payloads[0].text, workflow);
            assert_eq!(out.raw_payloads[0].kind, "comfyui");
            assert_eq!(out.properties.len(), 2);
            assert!(
                out.properties
                    .iter()
                    .any(|p| p.name.ends_with("node_count") && p.value == "200")
            );
            assert!(
                out.properties
                    .iter()
                    .any(|p| p.name.ends_with("node_types") && p.value.contains("KSampler × 200"))
            );
            assert!(
                out.properties
                    .iter()
                    .all(|p| !p.name.contains("widgets") && !p.name.contains("pos"))
            );
        }
    }
    #[test]
    fn nested_graphs_unknown_json_and_long_ordinary_text_remain_copyable() {
        let text = r#"{"workflow":{"nodes":[{"type":"Example"}]},"prompt":{"7":{"class_type":"KSampler","inputs":{"seed":42}}}}"#;
        let out = inspect_bytes(&png(b"tEXt", format!("ComfyUI\0{text}").as_bytes()));
        assert!(
            out.properties
                .iter()
                .any(|p| p.name == "ComfyUI.prompt.7.inputs.seed")
        );
        assert!(
            out.properties
                .iter()
                .any(|p| p.name == "ComfyUI.workflow.node_count")
        );
        assert_eq!(out.raw_payloads[0].text, text);
        let out = inspect_bytes(&png(b"tEXt", b"Custom\0{\"unknown\":true}"));
        assert_eq!(out.raw_payloads[0].kind, "png_text");
        assert_eq!(out.raw_payloads[0].text, "{\"unknown\":true}");
        let text = "long note ".repeat(100);
        let out = inspect_bytes(&png(b"tEXt", format!("Author\0{text}").as_bytes()));
        assert_eq!(out.raw_payloads[0].text, text);
        assert_eq!(out.properties[0].namespace, "png_text");
    }
    #[test]
    fn comfy_fields_and_original_payload() {
        let text = r#"{"3":{"class_type":"KSampler","inputs":{"seed":42,"steps":20,"cfg":7,"sampler_name":"euler"}},"6":{"class_type":"CLIPTextEncode","inputs":{"text":"hello"}}}"#;
        let out = inspect_bytes(&png(b"tEXt", format!("prompt\0{text}").as_bytes()));
        assert!(out.properties.iter().any(|p| p.namespace == "comfyui"
            && p.name == "prompt.3.inputs.seed"
            && p.value == "42"));
        assert!(out.properties.iter().any(|p| p.value == "KSampler"));
        assert_eq!(out.raw_payloads[0].text, text);
    }
    #[test]
    fn compressed_workflow() {
        let mut body = b"workflow\0\0".to_vec();
        body.extend(zip(br#"{"nodes":[{"type":"CheckpointLoaderSimple","widgets_values":["model.safetensors"]}]}"#));
        let out = inspect_bytes(&png(b"zTXt", &body));
        assert!(out.properties.iter().any(|p| p.namespace == "comfyui"
            && p.name == "workflow.node_count"
            && p.value == "1"));
    }
    #[test]
    fn international_compressed_text() {
        let mut body = b"prompt\0\x01\0en\0Prompt\0".to_vec();
        body.extend(zip("a café".as_bytes()));
        let out = inspect_bytes(&png(b"iTXt", &body));
        assert_eq!(out.properties[0].value, "a café");
        assert_eq!(out.properties[0].namespace, "generation");
    }
    #[test]
    fn a1111_parameters() {
        let out = inspect_bytes(&png(b"tEXt", b"parameters\0a cat\nNegative prompt: blurry\nSteps: 20, Sampler: Euler, CFG scale: 7, Seed: 42"));
        for (name, value) in [
            ("Prompt", "a cat"),
            ("Negative prompt", "blurry"),
            ("Seed", "42"),
        ] {
            assert!(
                out.properties
                    .iter()
                    .any(|p| p.name == name && p.value == value)
            );
        }
    }
    #[test]
    fn malformed_text_and_json_do_not_panic() {
        assert!(
            !inspect_bytes(&png(b"zTXt", b"prompt\0\0broken"))
                .warnings
                .is_empty()
        );
        let out = inspect_bytes(&png(b"tEXt", b"prompt\0{broken"));
        assert_eq!(out.raw_payloads[0].text, "{broken");
        assert!(!out.warnings.is_empty());
        assert!(
            inspect_bytes(&png(b"iTXt", b"prompt\0\x01"))
                .properties
                .is_empty()
        );
    }
    #[test]
    fn decompression_bomb_is_bounded() {
        let mut body = b"prompt\0\0".to_vec();
        body.extend(zip(&vec![b'x'; MAX_TEXT * 2]));
        let out = inspect_bytes(&png(b"zTXt", &body));
        assert!(out.properties.is_empty());
        assert!(out.raw_payloads.is_empty());
        assert!(!out.warnings.is_empty());
    }
    #[test]
    fn display_limits_preserve_raw_json() {
        let text = serde_json::json!({"nodes": (0..300).map(|i| serde_json::json!({"id": i, "text": "x".repeat(5000)})).collect::<Vec<_>>()}).to_string();
        // This exceeds the total budget and must be rejected before JSON parsing.
        assert!(
            inspect_bytes(&png(b"tEXt", format!("workflow\0{text}").as_bytes()))
                .raw_payloads
                .is_empty()
        );
        let text =
            serde_json::json!({"nodes": (0..300).collect::<Vec<_>>(), "long": "x".repeat(5000)})
                .to_string();
        let out = inspect_bytes(&png(b"tEXt", format!("workflow\0{text}").as_bytes()));
        assert!(out.properties.len() <= 2);
        assert!(
            out.properties
                .iter()
                .all(|p| p.value.chars().count() <= MAX_VALUE + 1)
        );
        assert_eq!(out.raw_payloads[0].text, text);
    }
    #[test]
    fn unrelated_json_prompt_is_not_comfy() {
        let out = inspect_bytes(&png(b"tEXt", br#"prompt\u0000{"text":"hello"}"#));
        assert!(out.properties.is_empty()); // no actual NUL delimiter: malformed chunk
        let out = inspect_bytes(&png(b"tEXt", b"prompt\0{\"text\":\"hello\"}"));
        assert!(out.properties.iter().all(|p| p.namespace == "generation"));
    }
    #[test]
    fn ordinary_png_text_is_not_generation_and_xmp_is_delegated() {
        let out = inspect_bytes(&png(b"tEXt", b"Author\0Photographer"));
        assert_eq!(out.properties[0].namespace, "png_text");
        assert!(out.raw_payloads.is_empty());
        let out = inspect_bytes(&png(b"tEXt", b"XML:com.adobe.xmp\0<x:xmpmeta/>"));
        assert!(out.properties.is_empty());
        assert!(out.raw_payloads.is_empty());
    }
}
