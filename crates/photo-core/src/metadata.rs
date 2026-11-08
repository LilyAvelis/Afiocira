//! Explicit, path-targeted metadata commands. Browsing never registers an image.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    pub tags: Vec<String>,
    pub comment: String,
    pub description: String,
    pub caption: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetadataRequest {
    Inspect {
        path: PathBuf,
    },
    /// Full document replacement. None permits creation only; existing records
    /// require the revision returned by inspect/save (optimistic concurrency).
    Save {
        path: PathBuf,
        expected_revision: Option<u64>,
        metadata: Metadata,
    },
}

impl MetadataRequest {
    pub fn path(&self) -> &std::path::Path {
        match self {
            Self::Inspect { path } | Self::Save { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSnapshot {
    #[serde(default)]
    pub embedded: EmbeddedMetadata,
    #[serde(default)]
    pub available_tags: Vec<String>,
    pub path: PathBuf,
    pub image_id: Option<String>,
    pub revision: Option<u64>,
    /// False also covers unavailable files. Saved text remains readable; saves
    /// are refused until the original binding has been explicitly reconciled.
    pub binding_current: bool,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataErrorCode {
    InvalidInput,
    Conflict,
    BindingChanged,
    Schema,
    Storage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataError {
    pub code: MetadataErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataJob {
    pub request_id: u64,
    pub path: PathBuf,
    pub pending: bool,
    pub snapshot: Option<MetadataSnapshot>,
    pub error: Option<MetadataError>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetadataPanelState {
    pub embedded: Option<EmbeddedMetadata>,
    pub open: bool,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub pending: bool,
    pub metadata: Option<Metadata>,
    pub revision: Option<u64>,
    pub error: Option<String>,
}

/// Read-only information from the explicitly requested source. Never stored in
/// the sparse registry or merged into the user's notes without an explicit edit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedMetadata {
    pub properties: Vec<EmbeddedProperty>,
    pub raw_xmp: Vec<String>,
    pub raw_payloads: Vec<EmbeddedPayload>,
    pub warnings: Vec<String>,
    pub c2pa_present: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedProperty {
    /// Namespace URI or stable format identifier, independent of display order.
    pub namespace: String,
    pub name: String,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedPayload {
    pub kind: String,
    pub label: String,
    pub text: String,
}

/// Logical metadata fields; storage column names are deliberately not exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Tags,
    Multiline,
}

#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub kind: FieldKind,
    read: fn(&Metadata) -> String,
    write: fn(&mut Metadata, &str),
}
impl FieldSpec {
    pub fn text(&self, metadata: &Metadata) -> String {
        (self.read)(metadata)
    }
    pub fn set_text(&self, metadata: &mut Metadata, text: &str) {
        (self.write)(metadata, text);
    }
}

pub const FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "tags",
        label: "Теги",
        description: "Выберите теги или создайте новые",
        kind: FieldKind::Tags,
        read: |m| m.tags.join("\n"),
        write: |m, s| {
            m.tags = s
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        },
    },
    FieldSpec {
        key: "comment",
        label: "Комментарий",
        description: "Личные заметки об изображении",
        kind: FieldKind::Multiline,
        read: |m| m.comment.clone(),
        write: |m, s| m.comment = s.to_owned(),
    },
    FieldSpec {
        key: "description",
        label: "Описание",
        description: "Подробное описание изображения",
        kind: FieldKind::Multiline,
        read: |m| m.description.clone(),
        write: |m, s| m.description = s.to_owned(),
    },
];

#[cfg(test)]
mod catalogue_tests {
    use super::*;
    #[test]
    fn catalogue_edits_logical_fields_without_losing_other_values() {
        let mut metadata = Metadata::default();
        for field in FIELDS {
            field.set_text(&mut metadata, "Русский текст\nвторая строка");
        }
        for field in FIELDS {
            assert_eq!(field.text(&metadata), "Русский текст\nвторая строка");
        }
        assert_eq!(metadata.tags.len(), 2);
    }
}
