# Contextual actions and background file commands

## Selective multi-selection

Single Ctrl+click in Gallery adds/removes exactly one file. Existing marks stay
intact across clicks and after Ctrl is released; the file is not opened.
The clicked item becomes the cursor and next Shift-range anchor. Shift+click
continues to select a reversible range while preserving prior independent
marks. With both Ctrl and Shift held, Shift range selection takes precedence.
Plain click still opens. CLI/API expose `ctl toggle-selection INDEX` /
`toggle_selection { index }`, and reject toggling outside Grid.

Mark changes (Ctrl toggles, Shift ranges, explicit mark replacement/clear) are
coalesced for 60 ms and sent to the existing Yazi selection bridge without
moving keyboard focus. Other-folder Yazi marks remain preserved. Failed sends
are retried briefly; Tab retains its existing synchronization fallback. Opening
another folder cancels stale pending marks. The sender still uses the existing
short DDS process call; this is not a new high-frequency streaming transport.

The full Windows workspace suite passed 33 tests. `scripts/check-selection.py`
also passed against an isolated running GUI: cumulative toggles, single removal,
Grid preservation, Shift ranges/reversal, automatic Yazi snapshot round-trip
and clearing marks. No desktop input or user clipboard was used.

F2 is now integrated through `rename_action.rs` and the background worker.
The Windows backend captures file identity/version, then renames that same
locked handle without replacing an existing destination. See [import and F2](import-and-rename.md).

The permanent footer exposes **Actions · Ctrl K**. Its bottom-right popup follows
the supplied Raycast reference: grouped rows, selection highlight, keycaps and
search at the bottom. Gallery and Space query the same Rust registry with
different context. Up/Down selects, Enter invokes, Escape dismisses. Ctrl+K
toggles. Modal input is kept out of Yazi and gallery navigation. Text input in
the path field retains Copy. The global dark surfaces, spacing and rounded
controls use the same visual style; this is not a complete shell replacement.
The footer does not repeat Copy/Ctrl+C. Copy lives in Actions and on the
shortcut; for a group its label includes the file count.

## Implemented commands

- **Ctrl+D / duplicate**: in Gallery, duplicate marks or the cursor file; in
  Space, duplicate only the current file. A successful Space operation focuses
  the new copy. Gallery focuses/marks new copies, retaining failed originals
  in a partial batch. If the user navigated or changed selection during the
  job, completion does not steal their selection.
- **Ctrl+C / copy**: place Windows CF_HDROP file references in the system
  clipboard. For one supported still image, also offer full-resolution,
  EXIF-oriented BGRA pixels with alpha through CF_DIBV5. Videos and batches
  use file references. Pixel preparation is bounded to 32 million pixels and
  a decoder allocation limit of 256 MiB. If pixels are unavailable, files
  still copy and the job reports a warning. Ctrl+V is implemented in the next linked section.
- Open/back, next/previous, refresh, clear marks, image Fit, and video
  play/pause are also discoverable in the applicable palette.

Copy/duplicate can import the current supported media selection from Yazi
through the existing snapshot bridge, then operate on the captured paths.
Other-folder Yazi marks are outside this action's current-folder selection.
There is no general file-manager clipboard, folder duplication, batch rename,
user shortcut configuration, dynamic plugin runtime or undo journal here.

## File safety and responsiveness

One bounded background worker serializes file actions; duplicate actions cannot
be queued repeatedly while busy. Commands capture paths, not transient indices.
Each source is opened read-only with Windows sharing that prevents writes and
replacement during copying. Contents stream into a create-new temporary file
in the same folder. Length/modified time are checked, output is synced and
MoveFileW publishes to an unused `name - Copy (N).ext` without replacement,
including if another process claims the name first. Each batch item has its
own outcome. Ordinary errors clean up temporary files. A process crash can
leave a `.afio-copy-*.part` file; originals are never modified. No crash recovery
or global rollback is claimed.

## API

```powershell
photo-cli ctl commands
photo-cli ctl actions
photo-cli ctl search-actions duplicate
photo-cli ctl close-actions
photo-cli ctl duplicate
photo-cli ctl copy
photo-cli ctl action play_pause
photo-cli ctl status
```

The additive v3 JSON commands are `list_commands`, `show_commands`,
`search_commands { query }`, `close_commands`, and `run_action { id }`.
`commands` returns applicable registry entries; `palette` reports mode/query/
selection. File actions acknowledge submission. Poll `operation.running` and
match its `id` to read completion, per-file results and warnings. One latest
operation is retained. Invalid-context/busy actions are rejected. Changing
view while a job runs does not change the job's captured sources.

## Modules and verification

`photo-core/actions.rs` owns the shared catalogue and availability conditions;
`file_ops.rs` owns duplication. GUI `actions.rs` handles invocation, `palette.rs`
renders actions, `operations.rs` manages jobs, and `clipboard.rs` prepares native
clipboard formats. Existing dispatch, file state, navigator, image cache,
input, gallery and media views moved to separate modules. `main.rs` keeps
construction and window composition (about 580 lines, down from about 1600).

24 workspace tests and strict Clippy passed on Windows. The concurrent duplicate
test verifies unique destinations, original/existing-copy integrity and cleanup.
Clipboard payload tests verify Unicode paths, boundaries, top-down orientation
and alpha. `scripts/check-actions.py` passed against an isolated live GUI with
synthetic PNG/MP4: menu contexts/search, batch and Space duplication, byte
identity, focus/marks, partial failure under a writer lock and video actions.
This smoke test does not change the user's clipboard; the user subsequently confirmed multi-copy into Telegram and the visual design.

Follow-up: the native Windows DragQueryFileW parser successfully extracted all
three Unicode/spaced photo paths from our multi-file CF_HDROP payload. This test
uses its own memory handle and leaves the system clipboard untouched. Telegram
Desktop's current source reads local URLs from paste and prepares a media list;
Qt maps Windows CF_HDROP to those URLs. This supports the chosen format, while
actual behaviour in the user's installed Telegram still needs a paste check.
Browsers and other targets choose which clipboard formats they accept.

Sources: [Telegram paste handler](https://github.com/telegramdesktop/tdesktop/blob/dev/Telegram/SourceFiles/history/history_widget.cpp),
[Qt Windows MIME conversion](https://github.com/qt/qtbase/blob/dev/src/plugins/platforms/windows/qwindowsmimeregistry.cpp).

The native Yazi test exposed an old initial PTY size mismatch; matching
egui_tty's 80×24 and serializing native tests removed that failure. Windows
extended path prefixes are normalized at Yazi/API boundaries to prevent a
restarted navigator falling back to the executable folder. Restarting Yazi
for a GUI/API Open no longer steals Gallery's keyboard focus.
