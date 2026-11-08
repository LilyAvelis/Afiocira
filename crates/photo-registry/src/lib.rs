//! Sparse metadata storage. Only `Save` may create a database or image record.
//! There is deliberately no folder enumeration, watcher, or gallery callback.
use photo_core::metadata::{
    Metadata, MetadataError, MetadataErrorCode, MetadataRequest, MetadataSnapshot,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;

mod migrations;

pub type Result<T> = std::result::Result<T, MetadataError>;

fn error(code: MetadataErrorCode, message: impl Into<String>) -> MetadataError {
    MetadataError {
        code,
        message: message.into(),
    }
}
fn storage(e: impl std::fmt::Display) -> MetadataError {
    error(MetadataErrorCode::Storage, e.to_string())
}

/// No directory creation, connection or filesystem access in the constructor.
pub struct Registry {
    path: PathBuf,
}

pub fn default_data_home() -> Result<PathBuf> {
    std::env::var_os("AFIO_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Afiocira")))
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            error(
                MetadataErrorCode::InvalidInput,
                "Set an absolute AFIO_DATA_HOME or LOCALAPPDATA",
            )
        })
}

impl Registry {
    /// Follow a verified application-owned publication, without discovering or
    /// registering files. A missing database/binding is a no-op.
    pub fn rebind_known(
        &self,
        source: &Path,
        before: &photo_core::rename::SourceVersion,
        destination: &Path,
        after: &photo_core::rename::SourceVersion,
    ) -> Result<bool> {
        if !self.path.try_exists().map_err(storage)? {
            return Ok(false);
        }
        let mut conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(storage)?;
        conn.busy_timeout(Duration::from_millis(250))
            .map_err(storage)?;
        migrations::validate(&conn)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(storage)?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(storage)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let Some(row) = record(&tx, source)? else {
            return Ok(false);
        };
        if row.fingerprint != before.fingerprint()
            || photo_core::rename::capture(destination).ok().as_ref() != Some(after)
        {
            return Err(error(
                MetadataErrorCode::BindingChanged,
                "Controlled operation binding no longer matches",
            ));
        }
        let fingerprint = after.fingerprint();
        tx.execute(
            "UPDATE file_bindings SET path=?2,identity=?3,fingerprint=?4 WHERE image_id=?1",
            params![
                row.id,
                path_key(destination),
                &fingerprint[..16],
                fingerprint
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(true)
    }
    pub fn new(data_home: impl AsRef<Path>) -> Self {
        Self {
            path: data_home.as_ref().join("registry/registry.sqlite3"),
        }
    }

    pub fn execute(&self, request: MetadataRequest) -> Result<MetadataSnapshot> {
        let path = request.path();
        if !path.is_absolute()
            || !photo_core::is_media_path(path)
            || photo_core::is_video_path(path)
            || ambiguous_windows_path(path)
        {
            return Err(error(
                MetadataErrorCode::InvalidInput,
                "An absolute image file path is required",
            ));
        }
        match request {
            MetadataRequest::Inspect { path } => self.inspect(&path),
            MetadataRequest::Save {
                path,
                expected_revision,
                metadata,
            } => self.save(&path, expected_revision, normalize(metadata)?),
        }
    }

    fn connect(&self, write: bool) -> Result<Option<Connection>> {
        if !write && !self.path.try_exists().map_err(storage)? {
            return Ok(None);
        }
        if write {
            std::fs::create_dir_all(self.path.parent().expect("registry parent"))
                .map_err(storage)?;
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let mut conn = Connection::open_with_flags(&self.path, flags).map_err(storage)?;
        conn.busy_timeout(Duration::from_millis(250))
            .map_err(storage)?;
        if write {
            // Migration validation precedes persistent PRAGMA changes, so a
            // foreign/future database is never silently rewritten.
            migrations::migrate(&mut conn, &self.path)?;
            conn.pragma_update(None, "journal_mode", "WAL")
                .map_err(storage)?;
            conn.pragma_update(None, "synchronous", "FULL")
                .map_err(storage)?;
            conn.pragma_update(None, "foreign_keys", "ON")
                .map_err(storage)?;
        } else {
            migrations::validate(&conn)?;
        }
        conn.pragma_update(None, "cache_size", -2048)
            .map_err(storage)?;
        Ok(Some(conn))
    }

    fn inspect(&self, path: &Path) -> Result<MetadataSnapshot> {
        let Some(mut conn) = self.connect(false)? else {
            return Ok(empty(path));
        };
        let tx = conn.transaction().map_err(storage)?;
        let Some(record) = record(&tx, path)? else {
            let mut result = empty(path);
            result.available_tags = available_tags(&tx)?;
            return Ok(result);
        };
        snapshot(&tx, path, &record)
    }

    fn save(
        &self,
        path: &Path,
        expected: Option<u64>,
        metadata: Metadata,
    ) -> Result<MetadataSnapshot> {
        // Validate source before creating even an empty registry directory.
        let before = photo_core::rename::capture(path)
            .map_err(|e| error(MetadataErrorCode::InvalidInput, e.to_string()))?;
        // Empty first saves are read-only no-ops, including missing registry.
        if metadata == Metadata::default() && expected.is_none() {
            let current = self.inspect(path)?;
            if current.image_id.is_none() {
                return Ok(current);
            }
        }
        let mut conn = self.connect(true)?.expect("write connection");
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let existing = record(&tx, path)?;
        let (id, revision) = if let Some(row) = existing {
            if row.fingerprint != before.fingerprint() {
                return Err(error(
                    MetadataErrorCode::BindingChanged,
                    "File binding changed; explicit reconciliation is required",
                ));
            }
            if expected != Some(row.revision as u64) {
                return Err(error(
                    MetadataErrorCode::Conflict,
                    "Metadata revision changed; inspect before saving",
                ));
            }
            let revision = row
                .revision
                .checked_add(1)
                .ok_or_else(|| error(MetadataErrorCode::Conflict, "Metadata revision exhausted"))?;
            tx.execute(
                "UPDATE images SET revision=?2, updated_at=unixepoch() WHERE id=?1",
                params![row.id, revision],
            )
            .map_err(storage)?;
            (row.id, revision)
        } else {
            if expected.is_some() {
                return Err(error(
                    MetadataErrorCode::Conflict,
                    "Image has no metadata at this path",
                ));
            }
            let fingerprint = before.fingerprint();
            let alias: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_bindings WHERE identity=?1)",
                    [&fingerprint[..16]],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if alias {
                return Err(error(
                    MetadataErrorCode::BindingChanged,
                    "File identity is already registered at another path; explicit reconciliation is required",
                ));
            }
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute("INSERT INTO images(id,revision) VALUES(?1,1)", [&id])
                .map_err(storage)?;
            tx.execute(
                "INSERT INTO file_bindings(path,image_id,identity,fingerprint) VALUES(?1,?2,?3,?4)",
                params![path_key(path), id, &fingerprint[..16], fingerprint],
            )
            .map_err(storage)?;
            (id, 1)
        };
        tx.execute("DELETE FROM image_tags WHERE image_id=?1", [&id])
            .map_err(storage)?;
        for tag in &metadata.tags {
            tx.execute(
                "INSERT INTO tags(name) VALUES(?1) ON CONFLICT(name) DO NOTHING",
                [tag],
            )
            .map_err(storage)?;
            tx.execute(
                "INSERT INTO image_tags(image_id,tag_id) SELECT ?1,id FROM tags WHERE name=?2",
                params![id, tag],
            )
            .map_err(storage)?;
        }
        tx.execute("DELETE FROM annotations WHERE image_id=?1", [&id])
            .map_err(storage)?;
        for (kind, text) in texts(&metadata) {
            if !text.is_empty() {
                tx.execute(
                    "INSERT INTO annotations(image_id,kind,text) VALUES(?1,?2,?3)",
                    params![id, kind, text],
                )
                .map_err(storage)?;
            }
        }
        // No source bytes are written. Observe ordinary external edits during
        // the save too; all metadata changes roll back on a stale source.
        if photo_core::rename::capture(path).ok().as_ref() != Some(&before) {
            return Err(error(
                MetadataErrorCode::BindingChanged,
                "Source changed while saving metadata",
            ));
        }
        let available_tags = available_tags(&tx)?;
        tx.commit().map_err(storage)?;
        Ok(MetadataSnapshot {
            embedded: Default::default(),
            available_tags,
            path: path.to_owned(),
            image_id: Some(id),
            revision: Some(revision as u64),
            binding_current: true,
            metadata,
        })
    }
}

fn ambiguous_windows_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // Win32 trims these suffixes while verbatim paths do not. Refuse both
        // forms rather than accidentally showing another file's metadata.
        path.components().any(|component| match component {
            std::path::Component::Normal(name) => {
                matches!(name.encode_wide().last(), Some(32 | 46))
            }
            _ => false,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn texts(m: &Metadata) -> [(&str, &str); 4] {
    [
        ("comment", &m.comment),
        ("description", &m.description),
        ("caption", &m.caption),
        ("prompt", &m.prompt),
    ]
}
/// Pure tag validation/normalization shared with the editor; performs no I/O.
pub fn normalize_tag(text: &str) -> Result<String> {
    let tag: String = text.trim().nfc().collect();
    if tag.is_empty() || tag.chars().count() > 128 || tag.chars().any(char::is_control) {
        return Err(error(
            MetadataErrorCode::InvalidInput,
            "Tags require 1–128 non-control characters",
        ));
    }
    Ok(tag)
}
fn normalize(mut m: Metadata) -> Result<Metadata> {
    if m.tags.len() > 128
        || texts(&m)
            .iter()
            .any(|(_, text)| text.len() > 65536 || text.contains('\0'))
    {
        return Err(error(
            MetadataErrorCode::InvalidInput,
            "At most 128 tags and 65536 bytes per text field; NUL is not allowed",
        ));
    }
    for tag in &mut m.tags {
        *tag = normalize_tag(tag)?;
    }
    m.tags.sort();
    m.tags.dedup();
    Ok(m)
}
fn available_tags(conn: &Connection) -> Result<Vec<String>> {
    let mut statement = conn
        .prepare("SELECT name FROM tags ORDER BY name LIMIT 64")
        .map_err(storage)?;
    statement
        .query_map([], |row| row.get(0))
        .map_err(storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(storage)
}

fn empty(path: &Path) -> MetadataSnapshot {
    MetadataSnapshot {
        embedded: Default::default(),
        available_tags: Vec::new(),
        path: path.to_owned(),
        image_id: None,
        revision: None,
        binding_current: false,
        metadata: Metadata::default(),
    }
}

struct Record {
    id: String,
    revision: i64,
    fingerprint: Vec<u8>,
}
fn record(conn: &Connection, path: &Path) -> Result<Option<Record>> {
    conn.query_row("SELECT i.id,i.revision,b.fingerprint FROM file_bindings b JOIN images i ON i.id=b.image_id WHERE b.path=?1",
        [path_key(path)], |row| Ok(Record { id: row.get(0)?, revision: row.get(1)?, fingerprint: row.get(2)? }))
        .optional().map_err(storage)
}
fn snapshot(conn: &Connection, path: &Path, row: &Record) -> Result<MetadataSnapshot> {
    let mut metadata = Metadata::default();
    let mut stmt = conn.prepare("SELECT t.name FROM tags t JOIN image_tags it ON t.id=it.tag_id WHERE it.image_id=?1 ORDER BY t.name").map_err(storage)?;
    metadata.tags = stmt
        .query_map([&row.id], |r| r.get(0))
        .map_err(storage)?
        .collect::<std::result::Result<_, _>>()
        .map_err(storage)?;
    let mut stmt = conn
        .prepare("SELECT kind,text FROM annotations WHERE image_id=?1")
        .map_err(storage)?;
    for annotation in stmt
        .query_map([&row.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(storage)?
    {
        let (kind, text) = annotation.map_err(storage)?;
        match kind.as_str() {
            "comment" => metadata.comment = text,
            "description" => metadata.description = text,
            "caption" => metadata.caption = text,
            "prompt" => metadata.prompt = text,
            _ => return Err(error(MetadataErrorCode::Schema, "Unknown annotation kind")),
        }
    }
    Ok(MetadataSnapshot {
        embedded: Default::default(),
        available_tags: available_tags(conn)?,
        path: path.to_owned(),
        image_id: Some(row.id.clone()),
        revision: Some(row.revision as u64),
        metadata,
        binding_current: photo_core::rename::capture(path)
            .is_ok_and(|v| v.fingerprint() == row.fingerprint),
    })
}

// Normalize lexical drive/UNC aliases without touching the source disk. Preserve
// UTF-16 filename units and case (Windows can have case-sensitive directories).
// Hardlinks, symlinks and actual renames are not inferred from path spellings.
fn path_key(path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::path::{Component, Prefix};
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => match prefix.kind() {
                    Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                        normalized.push(format!("{}:", drive.to_ascii_uppercase() as char));
                    }
                    Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                        let mut unc = std::ffi::OsString::from("\\\\");
                        unc.push(server);
                        unc.push("\\");
                        unc.push(share);
                        normalized.push(unc);
                    }
                    _ => normalized.push(component.as_os_str()),
                },
                Component::RootDir => normalized.push("\\"),
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::Normal(name) => normalized.push(name),
            }
        }
        normalized
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().as_encoded_bytes().to_vec()
    }
}
