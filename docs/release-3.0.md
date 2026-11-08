# Afiocira 3.0.0 release record

Public release target: [`v3.0.0`](https://github.com/LilyAvelis/Afiocira/releases/tag/v3.0.0).
Publication results are recorded in GitHub Actions. See [release notes](releases/v3.0.0.md),
[session editing](session-editing.md), [metadata registry](metadata-registry.md)
and [Windows CI/CD](ci-cd.md).

## Accepted scope

Sparse SQLite notes/tags, read-only embedded metadata, temporary SSD session
history and three live-preview photo tools are included. The tools expose
fifteen controls plus a tone curve. Apply performs a checked full-size edit;
preview and Cancel do not write the source. Undo/redo restore exact captured
bytes, format and path. Ctrl+S explicitly truncates the redo continuation.
Rotations and reflections do not create history versions.

Supported XMP, EXIF/IPTC, ICC/header information and PNG generation data include
complete ComfyUI workflow JSON for inspection/copy. Reading does not import
embedded values into Afio notes. Supported JPEG XMP and photometric preservation
was checked using a copy while retaining the original test source. Copy Path,
copy-confirmation preferences, Metadata Escape/focus and shortcut fixes complete
the scope. Ctrl+L received manual user validation; no blanket physical-keyboard
certification is implied.

## Evidence and remaining release checks

The last reported full local run passed 296 Rust unit/native tests and 3 doctests,
formatting and strict Clippy. Isolated native synthetic scenarios cover metadata,
history, editor preview and checked writes. They do not certify every image or
installation environment.

Before publication require a green Windows check on the final commit, a verified
manual candidate (`workflow_dispatch`, `package=true`) and a matching annotated
`v3.0.0` tag. A tag push runs check → package → publish. Verification covers asset
names, SHA256, ZIP contents, manifest version/source SHA and binary source-path
remapping. Record actual package and publication results only after they exist;
see [Actions](https://github.com/LilyAvelis/Afiocira/actions).

Independent installation on a second unprepared Windows 11 x64 computer remains
unverified, including missing VC++ runtime, video download failure/retry and
uninstall. The installer is unsigned. Optional video runtimes are upstream
downloads, not bundled package contents.

## Boundaries

History is session-only temporary storage, not durable recovery. Full color
management, HEIC/RAW decoding, persistent history, Fork, tag search and dataset
export are not included. Ctrl+G UX is reserved for 3.1 and persistent history for
3.2. ComfyUI execution remains future work without an assigned version.

The [2.5 record](release-2.5.md) and [2.0 record](release-2.0.md) remain historical
evidence for their own versions.
