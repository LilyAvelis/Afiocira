use photo_core::metadata::{Metadata, MetadataErrorCode, MetadataRequest};
use photo_registry::Registry;
use rusqlite::Connection;
use std::{fs, path::PathBuf};

struct Fixture {
    dir: tempfile::TempDir,
    home: PathBuf,
    image: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("data");
        let image = root.join("кадр 🪷.png");
        // Tiny synthetic fixture; registry never decodes or modifies pixels.
        fs::write(&image, b"synthetic image bytes").unwrap();
        Self { dir, home, image }
    }
    fn registry(&self) -> Registry {
        Registry::new(&self.home)
    }
    fn db(&self) -> Connection {
        Connection::open(self.home.join("registry/registry.sqlite3")).unwrap()
    }
    fn save(
        &self,
        revision: Option<u64>,
        metadata: Metadata,
    ) -> photo_registry::Result<photo_core::metadata::MetadataSnapshot> {
        self.registry().execute(MetadataRequest::Save {
            path: self.image.clone(),
            expected_revision: revision,
            metadata,
        })
    }
    fn inspect(&self) -> photo_core::metadata::MetadataSnapshot {
        self.registry()
            .execute(MetadataRequest::Inspect {
                path: self.image.clone(),
            })
            .unwrap()
    }
}
fn note(text: &str) -> Metadata {
    Metadata {
        comment: text.into(),
        ..Default::default()
    }
}
fn count(conn: &Connection, table: &str) -> i64 {
    assert!(
        [
            "images",
            "file_bindings",
            "annotations",
            "tags",
            "image_tags"
        ]
        .contains(&table)
    );
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn browsing_and_inspection_never_create_registry_even_for_missing_sources() {
    let f = Fixture::new();
    let registry = f.registry();
    for _ in 0..50 {
        photo_core::Gallery::open(f.dir.path()).unwrap();
        photo_core::Gallery::open_file(&f.image).unwrap();
        assert!(f.inspect().image_id.is_none());
        assert!(
            registry
                .execute(MetadataRequest::Inspect {
                    path: f.dir.path().join("offline.png")
                })
                .unwrap()
                .image_id
                .is_none()
        );
    }
    assert!(!f.home.exists());
    assert!(
        f.save(None, Metadata::default())
            .unwrap()
            .image_id
            .is_none()
    );
    assert!(!f.home.exists());
}

#[test]
fn first_save_registers_exactly_one_image_and_survives_restart() {
    let f = Fixture::new();
    for i in 0..150 {
        fs::write(f.dir.path().join(format!("untouched-{i}.jpg")), b"other").unwrap();
    }
    let original = fs::read(&f.image).unwrap();
    let metadata = Metadata {
        tags: vec!["  Портрет ".into(), "e\u{301}".into(), "é".into()],
        comment: "Заметка 🪷".into(),
        description: "Описание".into(),
        caption: "caption".into(),
        prompt: "prompt".into(),
    };
    let saved = f.save(None, metadata).unwrap();
    assert_eq!(saved.revision, Some(1));
    assert_eq!(saved.metadata.tags, ["é", "Портрет"]);
    assert!(saved.binding_current);
    assert_eq!(saved, f.inspect());
    let db = f.db();
    assert_eq!(count(&db, "images"), 1);
    assert_eq!(count(&db, "file_bindings"), 1);
    assert_eq!(count(&db, "annotations"), 4);
    for i in 0..150 {
        assert!(
            f.registry()
                .execute(MetadataRequest::Inspect {
                    path: f.dir.path().join(format!("untouched-{i}.jpg"))
                })
                .unwrap()
                .image_id
                .is_none()
        );
    }
    assert_eq!(count(&db, "images"), 1);
    assert_eq!(fs::read(&f.image).unwrap(), original);
    let version: String = db
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, "3.54.1");
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(
        db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}

#[test]
fn stale_revisions_and_missing_expected_revision_cannot_overwrite() {
    let f = Fixture::new();
    let first = f.save(None, note("first")).unwrap();
    assert_eq!(
        f.save(None, note("clobber")).unwrap_err().code,
        MetadataErrorCode::Conflict
    );
    let second = f.save(first.revision, note("second")).unwrap();
    assert_eq!(second.image_id, first.image_id);
    assert_eq!(second.revision, Some(2));
    assert_eq!(
        f.save(first.revision, note("stale")).unwrap_err().code,
        MetadataErrorCode::Conflict
    );
    assert_eq!(f.inspect().metadata.comment, "second");
    assert_eq!(count(&f.db(), "images"), 1);
}

#[test]
fn clearing_metadata_is_explicit_and_preserves_logical_id() {
    let f = Fixture::new();
    let first = f.save(None, note("first")).unwrap();
    let cleared = f.save(first.revision, Metadata::default()).unwrap();
    assert_eq!(first.image_id, cleared.image_id);
    assert_eq!(cleared.metadata, Metadata::default());
    assert_eq!(count(&f.db(), "annotations"), 0);
}

#[test]
fn missing_or_replaced_source_keeps_text_but_refuses_save() {
    let f = Fixture::new();
    let first = f.save(None, note("keep me")).unwrap();
    fs::rename(&f.image, f.dir.path().join("original.png")).unwrap();
    let unavailable = f.inspect();
    assert!(!unavailable.binding_current);
    assert_eq!(unavailable.metadata.comment, "keep me");
    assert!(f.save(first.revision, note("missing")).is_err());
    fs::write(&f.image, b"replacement file").unwrap();
    assert_eq!(
        f.save(first.revision, note("wrong image"))
            .unwrap_err()
            .code,
        MetadataErrorCode::BindingChanged
    );
    assert!(!f.inspect().binding_current);
    assert_eq!(count(&f.db(), "images"), 1);
}

#[test]
fn external_rename_or_hardlink_does_not_create_a_second_identity() {
    let f = Fixture::new();
    f.save(None, note("first")).unwrap();
    let alias = f.dir.path().join("alias.png");
    fs::hard_link(&f.image, &alias).unwrap();
    let e = f
        .registry()
        .execute(MetadataRequest::Save {
            path: alias,
            expected_revision: None,
            metadata: note("alias"),
        })
        .unwrap_err();
    assert_eq!(e.code, MetadataErrorCode::BindingChanged);
    let moved = f.dir.path().join("moved.png");
    fs::rename(&f.image, &moved).unwrap();
    let e = f
        .registry()
        .execute(MetadataRequest::Save {
            path: moved,
            expected_revision: None,
            metadata: note("moved"),
        })
        .unwrap_err();
    assert_eq!(e.code, MetadataErrorCode::BindingChanged);
    assert_eq!(count(&f.db(), "images"), 1);
}

#[test]
fn validation_fails_before_creating_database() {
    let f = Fixture::new();
    for metadata in [
        Metadata {
            tags: vec!["  ".into()],
            ..Default::default()
        },
        note(&"x".repeat(65537)),
        Metadata {
            tags: vec!["tag".into(); 129],
            ..Default::default()
        },
    ] {
        assert_eq!(
            f.save(None, metadata).unwrap_err().code,
            MetadataErrorCode::InvalidInput
        );
    }
    for path in [
        PathBuf::from("relative.png"),
        f.dir.path().join("video.mp4"),
        f.dir.path().join("missing.jpg"),
    ] {
        assert!(
            f.registry()
                .execute(MetadataRequest::Save {
                    path,
                    expected_revision: None,
                    metadata: note("invalid")
                })
                .is_err()
        );
    }
    assert!(!f.home.exists());
}

#[test]
fn foreign_and_future_databases_are_not_migrated_or_rewritten() {
    for future in [false, true] {
        let f = Fixture::new();
        fs::create_dir_all(f.home.join("registry")).unwrap();
        let db = f.db();
        if future {
            db.execute_batch("PRAGMA application_id=1095125327; PRAGMA user_version=999;")
                .unwrap();
        } else {
            db.execute_batch(
                "CREATE TABLE foreign_data(secret TEXT); INSERT INTO foreign_data VALUES('keep');",
            )
            .unwrap();
        }
        drop(db);
        let path = f.home.join("registry/registry.sqlite3");
        let bytes = fs::read(&path).unwrap();
        assert_eq!(
            f.save(None, note("no")).unwrap_err().code,
            MetadataErrorCode::Schema
        );
        assert!(
            f.registry()
                .execute(MetadataRequest::Inspect {
                    path: f.image.clone()
                })
                .is_err()
        );
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn modified_migration_history_is_refused() {
    let f = Fixture::new();
    f.save(None, note("first")).unwrap();
    f.db()
        .execute("UPDATE schema_migrations SET sql='modified'", [])
        .unwrap();
    assert_eq!(
        f.save(Some(1), note("no")).unwrap_err().code,
        MetadataErrorCode::Schema
    );
}

#[test]
fn migration_history_accepts_equivalent_crlf_builds() {
    let f = Fixture::new();
    f.save(None, note("first")).unwrap();
    f.db()
        .execute(
            "UPDATE schema_migrations SET sql=replace(sql,char(10),char(13)||char(10))",
            [],
        )
        .unwrap();
    assert_eq!(f.inspect().metadata.comment, "first");
    f.save(Some(1), note("second")).unwrap();
}

#[test]
fn failed_metadata_write_rolls_back_document_revision_and_registration() {
    let f = Fixture::new();
    let first = f.save(None, note("original")).unwrap();
    f.db().execute_batch("CREATE TRIGGER reject_annotations BEFORE INSERT ON annotations BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;").unwrap();
    assert!(f.save(Some(1), note("failed update")).is_err());
    assert_eq!(f.inspect(), first);
    let other = f.dir.path().join("unregistered.png");
    fs::write(&other, b"synthetic").unwrap();
    assert!(
        f.registry()
            .execute(MetadataRequest::Save {
                path: other.clone(),
                expected_revision: None,
                metadata: note("failed insert")
            })
            .is_err()
    );
    assert!(
        f.registry()
            .execute(MetadataRequest::Inspect { path: other })
            .unwrap()
            .image_id
            .is_none()
    );
    assert_eq!(count(&f.db(), "images"), 1);
    assert_eq!(count(&f.db(), "file_bindings"), 1);
}

#[cfg(windows)]
#[test]
fn normal_extended_and_lexical_paths_share_one_binding() {
    let f = Fixture::new();
    let canonical = f.image.canonicalize().unwrap();
    let normal = PathBuf::from(canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    let first = f
        .registry()
        .execute(MetadataRequest::Save {
            path: normal.clone(),
            expected_revision: None,
            metadata: note("one image"),
        })
        .unwrap();
    for path in [
        canonical,
        normal
            .parent()
            .unwrap()
            .join("unused/../")
            .join(normal.file_name().unwrap()),
    ] {
        // The lexical inspect is indexed and does not need the nonexistent
        // 'unused' directory to exist. Its physical binding_current can be false.
        let found = f
            .registry()
            .execute(MetadataRequest::Inspect { path })
            .unwrap();
        assert_eq!(found.image_id, first.image_id);
    }
    assert_eq!(count(&f.db(), "images"), 1);
}

#[cfg(windows)]
#[test]
fn ambiguous_win32_suffixes_are_rejected_without_registration() {
    let f = Fixture::new();
    for path in [
        f.dir.path().join("literal. /image.png"),
        f.dir.path().join("literal./image.png"),
    ] {
        assert_eq!(
            f.registry()
                .execute(MetadataRequest::Inspect { path })
                .unwrap_err()
                .code,
            MetadataErrorCode::InvalidInput
        );
    }
    assert!(!f.home.exists());
}

#[test]
fn independent_connections_cannot_both_save_the_same_revision() {
    let f = Fixture::new();
    f.save(None, note("first")).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|text| {
            let (home, path, barrier) = (f.home.clone(), f.image.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                Registry::new(home).execute(MetadataRequest::Save {
                    path,
                    expected_revision: Some(1),
                    metadata: note(text),
                })
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(f.inspect().revision, Some(2));
    assert_eq!(count(&f.db(), "images"), 1);
}

#[cfg(windows)]
#[test]
fn windows_path_key_is_not_lossy() {
    use std::os::windows::ffi::OsStringExt;
    let f = Fixture::new();
    let mut units: Vec<u16> = "unpaired-".encode_utf16().collect();
    units.push(0xd800);
    units.extend(".png".encode_utf16());
    let path = f.dir.path().join(std::ffi::OsString::from_wide(&units));
    fs::write(&path, b"test").unwrap();
    let saved = f
        .registry()
        .execute(MetadataRequest::Save {
            path: path.clone(),
            expected_revision: None,
            metadata: note("exact path"),
        })
        .unwrap();
    let read = f
        .registry()
        .execute(MetadataRequest::Inspect { path })
        .unwrap();
    assert_eq!(saved.image_id, read.image_id);
    assert_eq!(read.metadata.comment, "exact path");
}

#[test]
fn tag_options_are_bounded_sorted_and_do_not_register_inspected_images() {
    let f = Fixture::new();
    assert!(f.inspect().available_tags.is_empty());
    assert!(!f.home.exists());
    let tags: Vec<_> = (0..70).rev().map(|n| format!("tag-{n:02}")).collect();
    let saved = f
        .save(
            None,
            Metadata {
                tags,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(saved.available_tags.len(), 64);
    assert_eq!(saved.available_tags.first().unwrap(), "tag-00");
    assert_eq!(saved.available_tags.last().unwrap(), "tag-63");
    let unknown = f.dir.path().join("unregistered.png");
    fs::write(&unknown, b"not decoded").unwrap();
    let before = count(&f.db(), "images");
    let options = f
        .registry()
        .execute(MetadataRequest::Inspect { path: unknown })
        .unwrap();
    assert!(options.image_id.is_none());
    assert_eq!(options.available_tags, saved.available_tags);
    assert_eq!(count(&f.db(), "images"), before);
    assert_eq!(count(&f.db(), "file_bindings"), 1);
}
