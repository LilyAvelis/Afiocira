# 3.0 working development: metadata, session history, light and color

This stage extends the SQLite foundation on `codex/sqlite-metadata`; it is not a
published 3.0 release. Existing install/launcher files are unchanged.

## Surfaces and commands

- **Ctrl+G / Metadata**: scrollable right panel for the current still image in
  Gallery or Space. Tags, comment and description come from a
  shared field catalogue (`photo-core::metadata::FIELDS`). Field kinds describe
  the renderer and adapters; extending this catalogue does not require another
  hardcoded panel section. Tags render as removable colored chips with a
  searchable existing-tag multiselect and Enter-to-create. Suggestions are read
  from at most 64 saved dictionary entries, without registering other images.
  Caption/prompt are hidden; existing stored values survive ordinary panel saves.
  User-created arbitrary field schemas are not included.
  The separate read-only file section now displays supported XMP, EXIF/IPTC,
  color/header information and PNG generation data, including full ComfyUI
  workflow JSON. It reads only the requested file, does not register it, and
  never silently imports embedded values into Afio notes. Raw XML/workflow can
  be copied; per-field visibility/reordering and embedded-field editing remain
  future work. See [reader limits and compatibility](reviews/2026-11-08-embedded-metadata.md).
  Embedded groups start collapsed. Workflow JSON has a compact summary and a
  separate complete-source copy button; long previews require explicit expansion.
  Escape cancels the metadata UI draft and closes the panel from Gallery or
  Yazi focus. It never submits a new save; an already admitted save request may
  finish in the background. Late receipts cannot reopen a canceled panel.
- **Ctrl+H / History**: separate right panel, Space still images only. It lists
  session versions as thumbnail cards. Visible cards load automatically one at a
  time; the texture cache is bounded to 24 small previews. Scrolling does not
  decode the entire history. Hover, click and unmodified arrow keys select a
  read-only version on the central canvas. The selected preview and the marked
  current stack node are independent. One full canvas preview (maximum side
  1600) is loaded with latest-selection guards; thumbnail preload never selects
  a version. Undo/redo are separate actions and remain the only stack movement.
- **Ctrl+Z / Ctrl+Shift+Z**: undo/redo the current image's version stack. They
  restore the captured bytes, source format and path; destination conflicts and
  changed external files are refused.
- **Ctrl+S**: retain the current stack point and its past, explicitly discard its
  forward continuation. Starting another history-producing edit while redo
  exists is refused until this explicit commit; automatic branch/fork UI is not
  part of this first stack implementation.
- **Actions → Редактирование → Свет и цвет… / Детали… / Виньетка…**:
  three independently bindable tools with their own controls. The dialog has a
  right side panel with a fixed Apply/Cancel/Reset footer and scrolling controls.
  Parameter changes preview on the main image in real time. The preview worker
  caches one immutable downscaled base (maximum side 1280), coalesces pending
  parameters and rejects stale path/version/revision results. Preview never
  writes the image, creates a history step or registers metadata. Cancel returns
  to the source; Apply performs the checked full-size edit. A comparison toggle
  temporarily shows the original. Navigation requires Apply or Cancel first.
  Repeating the active tool's resolved shortcut or pressing Enter applies and
  closes it. A neutral form closes without writing or adding history. If the
  latest preview is pending, application waits for that exact revision. Numeric
  text entry keeps ordinary shortcut keys; Enter commits its last value before
  submitting. Escape cancels. Switching tools requires finishing the current one.

Metadata and History buttons sit beside Actions in the existing bottom footer.
They use the shared action catalogue and shortcut customization. Metadata is
available for the current Gallery cursor, independently of multi-selection;
History is unavailable in Gallery/video contexts. Narrow windows temporarily
collapse Yazi while a side panel is open, preserving its running session and
visibility preference. Closing the side panel or widening the window restores
it; the Yazi button can return to it after a clean panel is closed.

Metadata drafts remain bound to their original file. Save is explicit, Discard
restores the loaded document, and a dirty or pending draft cannot be silently
closed or replaced. Text editors retain their local keyboard undo/copy behavior.
Repeating Ctrl+G saves a changed metadata draft and closes only after its matching
successful receipt. A clean panel closes without registering the image. Enter
does the same outside text input; multiline Enter and tag creation stay local.
Conversion/resize retain Enter submission and also accept their assigned command
shortcut again outside text entry. Ctrl+H toggles the read-only History panel.
Actions and Settings retain existing global keys.

## Sparse indexing and storage

Browsing, showing Metadata, and opening an empty History create no image record
and no snapshot directory. Only an explicit metadata save registers an image in
SQLite. History captures are limited to the image being edited and do not require
registration in the permanent metadata database.

Snapshots are temporary immutable files under
`AFIO_DATA_HOME/history-sessions/<unique-session>` (default data home is
`%LOCALAPPDATA%/Afiocira`). This should be a local SSD directory. They are not
held as full-resolution RAM generations and are not loaded at next startup.
The store has a 1 GiB snapshot quota, at most 256 tracked stacks and 4096 stored
versions. Exhaustion refuses capture before the edit where possible; failure
after publication preserves a recovery snapshot and reports the actual outcome.
Normal shutdown drains the file worker and removes its owned session snapshots.
A crash/forced kill can retain orphan recovery files; automatic orphan cleanup
and durable cross-session recovery are not implemented.

**Conversion and light/color adjustments produce versions. Rotations,
reflections and F2 do not add versions.** An excluded operation on an already
tracked file refreshes only the current snapshot and binding so existing undo
does not become invalid; it does not create a new stack for an untracked image.
Resize-to-copy remains a separate new-file operation, without history creation.
Persistent HDD storage, copy-on-write shared branches, Ctrl+D history forks and
dataset export remain future stages.

## File and metadata safety

History capture streams checked source bytes into uniquely created files and
records a BLAKE3 digest. Explicit preview and restoration verify snapshot
integrity; corrupt snapshots cannot replace the working image.
Undo/redo stages the selected snapshot in the same media directory and verifies
it before touching the live file. Publication uses the existing no-clobber rename
rules and an original backup, with rollback and explicit recovery paths on
failure. Windows backup validation holds a deny-write/delete handle through
publication and deletes that exact handle, avoiding deletion of a replacement
file. Cross-directory transitions are rejected. Ctrl+S verifies the current live
bytes before truncating the future. External replacements do not acquire the
original's metadata/history automatically.

Known metadata follows verified Afio-owned conversion, adjustment, rotation,
reflection, rename and undo/redo receipts without creating new Registry entries.
The path/identity update is transactional and preserves UUID/text revision; a
missing binding/database remains a no-op. Filesystem publication and SQLite are
not a single atomic transaction: failure to rebind after a successful file write
is reported as a warning, and later saves refuse a stale binding. A durable
cross-session operation journal is still required for stronger recovery.

## Editing engine and limits

The existing pinned `image` engine decodes/encodes static 8-bit PNG/JPEG/WebP.
The adjustment layer validates finite ranges and uses linear-light exposure and
relative RGB white balance, encoded-sRGB contrast/tonal weights and a piecewise
linear curve whose output may rise or fall (input coordinates stay ordered).
Alpha is retained. JPEG is explicitly re-encoded at
quality 95; applying an unchanged form is refused. Checked source/version and
pixel/dimension verification precede publication.

This is a conservative sRGB editor, not a color-managed RAW pipeline. ICC-bearing
images, custom PNG color encoding, unsupported metadata, animation and high
precision are refused without changing the source. Existing metadata guards
preserve supported PNG chunks and JPEG EXIF, normalize orientation and detach
obsolete embedded thumbnails. File size, dimensions and an estimated 1 GiB
working-memory budget bound processing; the memory bound can reject large photos
below the absolute pixel limit. No masks, local brushes or live GPU preview is
claimed.

The three tools expose fifteen controls: Light and Color includes exposure,
brilliance, contrast, shadows, highlights, brightness, black point, saturation,
vibrance, temperature, tint and the tone curve. Details includes sharpness,
definition and noise reduction. Vignette contains its own single control.
All tools share processing and UI primitives, with preview, Apply,
Cancel, Reset and the existing history transaction. Opening or previewing an
edit never registers an image or writes its source.

Brightness and black point join the channel LUT; saturation, vibrance and
brilliance use an RGB luminance stage. Detail controls use bounded, alpha-aware
spatial filters; vignette uses normalized image coordinates. Preview never
upscales a small image; images above 1280 pixels are reduced and spatial radii
scale with that reduction. Full-resolution save uses the same stages with scale
one. Fine detail at reduced preview scale remains approximate. These are our
deterministic sRGB algorithms, not Apple's proprietary processing algorithms.

## API and verification

New commands share the dispatcher: `show_metadata`, `close_metadata`,
`submit_metadata {metadata}`, `discard_metadata`, `show_history`, `close_history`,
`preview_history {index}`, `undo_history`, `redo_history`, `commit_history`,
`show_adjustments`, `preview_adjustments {options}`, `submit_adjustments {options}`,
`apply_adjustments` (wait for and commit the exact latest preview, or close a
neutral form), `cancel_adjustments`. The stable `adjust_image` action remains
Light and Color, preserving existing bindings; `adjust_details` and
`adjust_vignette` are new `run_action` IDs. `show_adjustments` and its options
remain compatible for programmatic combined-engine use.
They are available through `photo-cli ctl-json`. Metadata jobs remain async via
receipts; panels expose `metadata_panel`, `history`, `adjustments_open` and
`adjustments_preview` state. The latter includes requested/ready revisions and
pending/error state plus active tool/finishing state; History exposes loaded
thumbnail indices, explicit preview selection and full-preview readiness.
File/history work exposes the existing `operation`; admission is not completion.
Poll `status` until the captured operation finishes and inspect file errors and
warnings. Session/request/path guards prevent late results from changing another
image's panel. Native API does not substitute for physical keyboard acceptance.

```powershell
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p photo-gui -p photo-cli
python scripts/check-edit-panels.py
python scripts/check-metadata.py
```

`check-edit-panels.py --bin-dir target/distribution/release` tests the static-CRT
release. It retains synthetic images, a result JSON and metadata/history/editor
root captures at 680/1100/1600 logical widths under `target/edit-panels-*`.
All file operations use synthetic fixtures and isolated app-data/API names.
