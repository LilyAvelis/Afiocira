# 3.0 metadata, history and editing — working verification

Historical first UI checkpoint. The modal/no-live-preview design below is
superseded by [the UX correction](2026-11-08-editing-ux.md).

This extends the [backend checkpoint](2026-11-08-metadata-foundation.md) on
`codex/sqlite-metadata`. It is a development result, not a public release or an
installed-app update. Behavior and limitations: [session editing](../session-editing.md).

## Result

Ctrl+G opens catalogue-driven metadata in Gallery/Space. Ctrl+H opens session
history in Space only. Both have bottom-footer buttons beside Actions and use
the existing action catalogue/shortcut resolver/API. Metadata opening and empty
history inspection remain sparse and create neither registrations nor snapshots.

Format conversion and light/color adjustments capture session versions. Exact
undo/redo restores bytes and format; Ctrl+S validates the current file then
truncates forward history. Rotations/reflections/F2 update an existing current
snapshot without adding a step or creating an untracked image's history. Known
metadata UUIDs follow verified owned publications and history restoration.

The editor provides exposure, relative temperature/tint, contrast, shadows,
highlights and a monotonic master curve. Its sRGB math is precomputed into small
8-bit channel lookup tables before processing pixels. Source/version, supported
metadata and encoded-result checks still fence same-file publication.

## Validation

- Formatting, strict locked workspace/all-targets Clippy, **231 tests and 3
  doctests** passed. Existing native Yazi/settings/IPC tests remain included.
- Windows static-CRT, source-remapped locked release workspace build passed.
  Final native release fixtures: `target/edit-panels-81545062` (including nine
  rendered root frames) and `target/metadata-f23d310a01` (sparse Registry suite).
- `check-edit-panels.py` passed against debug and release: metadata Save/readback,
  Gallery history refusal, lazy empty history, PNG→WebP conversion, byte-exact
  undo/redo with extension restoration, bounded snapshot preview, Ctrl+S forward
  truncation, exposure pixel changes, rotation exclusion, and preserved metadata
  UUID. Native API rejects opening metadata/history under an adjustment modal.
- `check-metadata.py` passed against release: 101 images, exactly one registered;
  restart persistence, revisions, database writer contention and responsive GUI,
  source bytes and replaced-file refusal.
- Root screenshots captured at 680/1100/1600 logical widths for metadata, history
  and adjustments; metadata headless layout tests also cover DPI 1/1.5/2. Frames
  were inspected. Narrow side panels preserve space for the image by temporarily
  collapsing Yazi. Editor Apply/Cancel/Reset stay outside scrolling content.
- Rust dependency notices regenerated with the project's cargo-about command.

These fixtures use generated media and isolated app-data/settings/API names.
No personal photographs, permanent launcher or installed configuration were
changed. Test processes are stopped by their owned fixture PID/descendants.

## Delegation and review

Three GPT-6.1 Sol agents at low effort built the field panel, adjustment layer
and session store. Independent review found and repaired restore staging cleanup,
cross-directory restoration, backup replacement races, inconsistent metadata
and modal guards, and scroll-hidden primary controls. Snapshot BLAKE3 integrity
checks and corrupt-snapshot refusal tests were added. History restore retains a
locked Windows backup through publication and deletes the exact guarded handle.

## Deliberate first-stage limits

No cross-session/HDD history, automatic fork UI, history-aware Ctrl+D, arbitrary
user-created property schemas, color-managed RAW editing or interactive pixel
preview. A new edit after undo requires explicit Ctrl+S first. Temporary store
limits are 1 GiB, 256 stacks and 4096 versions; crashes/forced kills can leave
orphan files. A file publication followed by failed SQLite rebind is diagnosed
as a warning; durable multi-resource crash recovery remains a separate stage.
Physical keyboard acceptance, personal-library performance and broader color
handling are not certified by the synthetic API/geometry checks.
