# SQLite metadata foundation — verification

Development branch: `codex/sqlite-metadata`, based on `131fa2c`. This record
describes the first 3.0 backend stage, not a public release or installed upgrade.
Contract and reproduction: [Metadata Registry](../metadata-registry.md).

## Implemented

Separate `photo-registry` crate with bundled SQLite 3.54.1, atomic versioned
migrations, sparse registration, image/file bindings, normalized tags and four
annotation fields. API/CLI requests go through the existing live dispatcher and
a lazy bounded metadata worker. Optimistic revisions protect whole-document
replacement. Reads do not register or initialize storage; gallery browsing has
no Registry call path. Source bytes are never written by metadata commands.

## Passed on 8 November 2026

- Formatting and strict workspace/all-targets Clippy.
- Full locked workspace suite: **207 tests and 3 doctests** (16 new Registry
  tests, including the migration unit test; existing 191 tests still pass).
- Offline locked `photo-registry` build.
- Locked Windows release workspace build with static CRT, source-path remapping
  and the pinned Zig 0.15.2 directory selected for the build process.
- `cargo about generate --workspace --locked --fail`; Rust notices refreshed.
- `scripts/check-metadata.py` on both debug and static-CRT release executables.
  Final release fixture: `target/metadata-f3060e2551`, retained for inspection.

Native evidence: 101 synthetic PNGs, exactly one image and one binding registered;
repeated Gallery/Space/refresh/inspect creates no data directory before a save;
metadata survives GUI restart; normal and Gallery-canonical Windows paths find
the same image; stale and unconditional-overwrite attempts are refused; CLI
inspect and JSON save share the live worker; database integrity/foreign keys and
source SHA-256 pass. With a separate process holding a SQLite writer lock,
ordinary status remains responsive and the save returns a bounded storage error.
A replacement source cannot overwrite the original metadata binding.

Rust tests additionally cover 151-file sparse registration, CRLF/LF migration
history, future/foreign/modified schemas, transaction rollback on DDL failure and
injected document-write failure, independent connection races, unavailable files,
hardlinks/external renames, exact UTF-16 storage and ambiguous Win32 suffix refusal.

## Independent review

A GPT-6.1 Sol subagent at low effort reviewed the source without editing it. It
identified inconsistent ordinary/extended Windows path keys and checkout-EOL
sensitivity in migration identity. Both were corrected and regression-tested.
The follow-up found no further mainstream backend/API blocker and highlighted
ambiguous verbatim trailing-dot/space names; those forms are now rejected rather
than exposing an unrelated binding.

## Boundaries

No Metadata UI, virtual collections, version history, dataset export, automatic
reconnect or F2/transform binding integration. A changed physical fingerprint
leaves saved metadata readable with `binding_current=false` and refuses saves.
Filename case is preserved for case-sensitive Windows directories. A volume/File
ID plus size/mtime is an observable fingerprint, not a cryptographic content
identity. Synthetic checks do not certify every filesystem, cloud library,
power-loss condition or personal-library latency.

No personal files, installed-app configuration or permanent launcher were
updated. Only isolated test processes were stopped. No release was published
and no 2.5 CI workflow changes were made.
