# Editing UX correction — 8 November 2026

This supersedes the first UI checkpoint's modal editor, click-only history
preview and newline tag field. Source branch remains `codex/sqlite-metadata`.

## Behavior

- Light/color controls are in a right side panel. The central image stays
  visible and updates from live, non-destructive previews. One background worker
  caches one immutable, oriented base with maximum side 1280; one active request
  and one replaceable pending request bound work. Path, source version, request
  serial and session epoch reject stale results. Each adjustment starts from the
  same base rather than accumulating edits on prior previews.
- Original comparison, reset, cancel and fixed Apply controls remain available.
  Cancel restores the ordinary source display without a file write. Apply uses
  the exact latest completed parameter set and keeps its texture visible while
  the full-size checked save runs. Changes never register metadata or add history
  until an actual eligible file operation succeeds.
- Arbitrary API curve points remain intact until the user edits the curve
  controls. Slider rounding runs on edits rather than repaint, preventing a
  rendered form from silently changing parameters. Preview and commit use the
  same LUT; preview-resolution sampling and JPEG recompression remain expected
  differences.
- History has one thumbnail card per version; visible rows load automatically,
  one at a time. At most 24 textures, maximum side 256, are retained. Failures,
  close/reopen and source changes clear pending state; changed snapshots
  invalidate their thumbnails. There is no eager decode of the full stack.
- Metadata shows only tags, comment and description. Tags are colored removable
  chips with Enter-to-create and an existing-tag multiselect, not a text area.
  Existing-tag suggestions are read-only and bounded to 64 entries. Caption and
  prompt are hidden without deleting their previously stored contents.

## Validation

Formatting and strict workspace/all-targets Clippy pass. The full workspace
suite passes **236 tests and 3 doctests**, including preview latest-wins/cancel,
alpha/reset/source preservation, metadata loading before async completion, tag
normalization/removal/hidden-field preservation, sparse suggestions and thumbnail
cache/error/pending bounds.

The native edit-panels script checks the actual central rendered image becomes
brighter at +1 EV, parameters coalesce through several revisions, source bytes
and history remain unchanged before Apply, Cancel/reopen restores the original,
and conversion/history/metadata identity still work. It additionally applies an
arbitrary three-point curve through the same Apply command used by the button
and compares decoded pixels against that exact curve. Source hash, requested
options and root frames are retained with the synthetic fixture.

Root metadata/history/editor frames at 680/1100/1600 logical widths were
inspected. Live previews, chips and both history version thumbnails were visible.
The static-CRT release build and both native scripts passed on the final source.
Release fixtures: `target/edit-panels-abf8e113` and
`target/metadata-80b0a9e745`. The permanent shortcut was updated to working version
`30e33d97a986`; the installed executable's SHA-256 matches the tested release.
Existing user windows were left open.

No cross-session history, new property-schema editor, tag-based Gallery query,
or expanded color-profile support is claimed by this correction. Preview is
bounded-resolution CPU rendering; the checked commit uses the supported full
source resolution. Personal images are not used by automated tests.
