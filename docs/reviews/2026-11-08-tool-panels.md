# Independent editing tools and read-only history navigation

## Behavior

Ctrl+K now has an Editing category with three independently bindable commands:
Light and Color (`adjust_image`, stable existing ID), Details (`adjust_details`)
and Vignette (`adjust_vignette`). Each opens only its own controls using the
shared processing/preview service. Existing Light and Color bindings survive.

Repeating the active command applies and closes its panel. Enter is the same
default submit; opening Enter and key repeats cannot accidentally submit a new
panel. A neutral edit closes without source writes or a history step. If a
changed preview is pending, submission waits for the exact latest requested
revision. Errors retain the panel; Cancel/Escape remains available. Numeric text
entry keeps ordinary keys, while Enter commits the numeric editor before
submission. Footer hints show Enter and the current resolved tool binding.

Conversion/resize retain their existing Enter behavior; repeating their bound
command outside text editing submits the current form through the same worker.
Metadata Ctrl+G now commits a draft, then closes after the matching successful
receipt. Clean metadata closes without registering a file. Failed saves preserve
the draft. Enter outside text fields behaves the same; multiline comments keep
newlines and the tag field retains Enter-to-create. Metadata Enter is routed
before gallery Enter to avoid opening a file as a side effect.

History hover, click and unmodified arrows now select a read-only central canvas
preview. They leave source bytes, path and history cursor untouched. The current
stack node keeps its own marker, independent from the selected preview. Undo/redo
remain the stack-changing actions. The gallery cannot consume history arrows.

Thumbnail preload (256 pixels, 24-texture bound) never selects a version. A
separate full preview (1600 pixels) retains only one texture. Epoch and selection
generation reject stale completions/errors; fast selection while busy keeps the
latest choice. A cached thumbnail can display while the full preview loads.
Processing remains on workers, with no bulk indexing or history scanning.

## Verification

- Full workspace: 254 tests plus 3 doctests passed. Focused shortcut tests passed
  again after the final Enter routing correction. Strict Clippy/format checks
  passed.
- Native static-CRT fixture checks category discoverability, binding changes,
  neutral toggles, new-parameter submission without waiting for preview, one
  history step per Apply, exact-byte undo, central history pixels and unchanged
  history cursor/source during preview. Root captures cover each independent
  tool at 680/1100/1600 logical widths; Details normal and Vignette narrow were
  inspected. Shared geometry tests cover DPI 1/1.5/2.
- Sparse fixture `target/metadata-a0b37e4819`: 101 files, one explicitly
  registered image; integrity, revisions and persistence checks passed.
- Final native fixture: `target/edit-panels-b453d94f/results.json` (all checks
  passed). Working version `a2569294dc62` installed through the permanent
  desktop shortcut; installed executable hash matches the tested release.
  Existing user windows remain open. Synthetic fixtures and private API names isolate all tests from
  the user's images. Physical keyboard/mouse acceptance remains a manual check;
  key ownership and history selection are covered by headless egui tests and
  the native shared-dispatch contracts.
