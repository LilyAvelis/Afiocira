# Metadata Registry — 3.0 backend foundation

The Registry now backs a catalogue-driven **Ctrl+G Metadata** side panel in
Gallery and Space. Session history and editing are described in
[session editing](session-editing.md). There is no virtual Gallery, dataset
export or reconciliation UI yet. The application and
published 2.5 release keep their existing version numbers; `photo-registry` is
the new 3.0 component. No existing settings are migrated into SQLite.

## Sparse indexing is the contract

Opening Afio, listing/opening a folder, selecting an image, viewing Space,
refreshing, and ordinary `status` never contact the Registry. There is no
filesystem scan, background registration, image hash or thumbnail generation
in this crate. The metadata worker starts only on an explicit metadata request.

`inspect` performs an indexed lookup of **one supplied path**. With no database
it returns an unregistered snapshot without creating a directory/database. With
an existing database and no matching binding it returns immediately without
opening/statting the source image. Inspecting an existing binding reads the
saved text and checks only that file's observable identity/version; it does not
decode it. Missing/offline sources keep their saved metadata.

Only an explicit **nonempty `save`** registers a new image UUID. Empty initial
saves are no-ops. Saving an existing record requires its current revision;
clearing its fields keeps its registered identity. No operation changes source
bytes or EXIF/XMP. Tags and text persist independently of future pixel history.

## Storage and schema

Default: `%LOCALAPPDATA%/Afiocira/registry/registry.sqlite3`. For tests or isolated
instances set an absolute `AFIO_DATA_HOME`; its `registry/` subdirectory is used.
Use a local data directory, outside install-version folders and cloud/network
sync roots. The library does not certify arbitrary redirected/network storage.

`rusqlite = 0.41.1`, bundled SQLite 3.54.1 (`libsqlite3-sys` 0.39.1 in Cargo.lock),
backup API, no dynamic extension loading. Users need no SQLite installation or
server. Writes enable foreign keys, WAL, synchronous FULL, a 250 ms busy timeout
and a 2 MiB page-cache target per connection. Each explicit request opens its
own connection on the worker; browsing holds no database connection.

Schema v1 has `images`, `file_bindings`, `tags`, `image_tags`, `annotations`, and
`schema_migrations`. Foreign keys/uniqueness protect relationships. Metadata
replacements and revision increments share an IMMEDIATE transaction; reads use
one snapshot transaction. Migration DDL and `user_version` changes are atomic,
and the application ID and migration SQL are checked before access. SQL history
normalizes CRLF/LF. Foreign, altered or future schemas are refused. Future
upgrades of initialized databases use SQLite's backup API before migration;
v1's initial empty database needs no backup. Failed writes roll back both the
document and any new registration. Normal SQL transactions cannot make future
file edits and SQLite commits jointly atomic; pixel history needs a separate
recovery journal before implementation.

Paths use reversible UTF-16 BLOB keys on Windows, with lexical normalization of
drive-letter case, normal/extended drive and UNC prefixes, separators and dot
components. Filename case stays exact to support case-sensitive directories.
Supply the same filename casing returned by Gallery; arbitrary casing changes,
symlink aliases, real renames and hardlinks are not automatically reconciled.
Path components ending in a dot/space are rejected because Win32 and verbatim
paths interpret them differently.
The live JSON API retains its existing Unicode-string path limitation; the
storage library itself preserves unpaired UTF-16 filename units.

Physical bindings store unsigned volume/File ID, size and modification time as
binary data. These detect ordinary changes, not edits with deliberately restored
attributes. They are not content hashes or eternal image identities. A known
identity at another path is refused rather than silently creating another
logical image. After an external edit/replacement/rename, `inspect` can return
saved text with `binding_current=false`; saves are refused. Verified Afio-owned
F2, rotation/reflection, conversion, adjustment and history restoration now update
an existing binding without registering new images. Explicit reconciliation is
still a later layer; a failed binding update is reported separately from a
successful file publication.

Tags are trimmed, NFC-normalized, case-sensitive and deduplicated; no ASCII-only
SQLite NOCASE claim is made. Limits: 128 tags, 128 Unicode scalars per normalized
tag, 65536 UTF-8 bytes per text field. Comment, description, caption and prompt
are separate stored strings. The UI shows only comment and description; caption
and prompt are hidden and preserved when editing existing records. Tags use
colored multiselect chips, not a multiline text field. Inspect returns up to 64
read-only existing-tag suggestions in `available_tags`, including for an
unregistered target when a database already exists. Save replaces the whole document; omitted document fields
default to empty, so clients should modify a fresh inspected document. Unknown
fields are rejected. Version-specific annotations/provenance and custom fields
will require future migrations.

## Live API and CLI

This is an additive protocol-v3 extension. Existing commands/responses retain
their behavior. Metadata responses add `metadata`; unrelated responses omit it.
GUI keys, future UI and CLI must use the same dispatcher and Registry service.

Submit through `photo-cli ctl-json` (JSON on stdin), using an absolute image path:

```json
{"command":"metadata","request":{"op":"inspect","path":"C:\\Photos\\frame.png"}}
```

The immediate response acknowledges admission, **not completion**:

```json
{"metadata":{"request_id":1,"path":"C:\\Photos\\frame.png","pending":true,"snapshot":null,"error":null}}
```

Poll the same session until `pending=false`:

```json
{"command":"metadata_status","request_id":1}
```

Convenience commands: `photo-cli ctl metadata PATH` and
`photo-cli ctl metadata-status REQUEST_ID`. They return the receipt as-is;
clients poll rather than treating a pending job as a successful read/write.
Compare the response `session_id` as well as `request_id`: request IDs restart
with the GUI session. One request may be outstanding; the last 16 receipts are
retained in bounded memory. Unknown/expired receipts return an API error.
Polling a receipt never queries SQLite again.

Create metadata (no existing record):

```json
{"command":"metadata","request":{"op":"save","path":"C:\\Photos\\frame.png","expected_revision":null,"metadata":{"tags":["portrait"],"comment":"Keep this pose","description":"Side lighting","caption":"","prompt":""}}}
```

Updates/clears must pass the `revision` from the completed snapshot. `null` means
creation only, never unconditional overwrite. Saved snapshots return `image_id`,
`revision`, `binding_current` and the normalized document. An unregistered read
returns null ID/revision and empty fields; `binding_current=false` does not by
itself mean a registered binding exists.

Completed failures are in `metadata.error`, with codes `invalid_input`,
`conflict`, `binding_changed`, `schema`, or `storage`. Admission errors are in the
ordinary top-level `error`. CLI exits nonzero for either completed error. A
timeout/disconnect is an unknown outcome; do not blindly retry a save. Inspect
the receipt, or inspect the document/revision after reconnecting. Pending jobs
are session-local, not a durable queue. Exiting/killing Afio before completion
does not guarantee that an admitted save completed.

## Verification

```powershell
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p photo-gui -p photo-cli
python scripts/check-metadata.py
```

The Rust fixtures exercise sparse reads/browsing, one registration among 151
files, Unicode/path aliases, restart persistence, stale revisions, independent
writers, unavailable/replaced sources, hardlinks, foreign/future/altered schema,
CRLF history, failed migration rollback and failed metadata-write rollback.

The native script creates 101 synthetic PNGs in a unique `target/metadata-*`
directory, uses isolated data/settings/API names, and retains its fixture for
inspection. It checks one registered image, Gallery/Space reads, live API and CLI
saves, restart, revision conflicts, database integrity, source SHA-256,
responsiveness while another process holds a SQLite write lock, and refusal of
a replacement source. It closes only processes it launched. To test a static-CRT
release build, pass `--bin-dir target/distribution/release` after building with
the flags in the Windows workflow. No visual acceptance or personal library
performance is claimed by these synthetic backend checks.
