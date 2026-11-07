# Import, F2 and opening dropped media

## Ctrl+V / Paste

Paste is available in both Actions contexts, including an empty Gallery. It
captures the destination folder, then a bounded worker reads the clipboard.
Unicode CF_HDROP files take priority; PNG, DIBV5, DIB and finally native bitmap
formats cover copied images. Images are normalized to PNG with available alpha
and orientation preserved. New names start at `Pasted image.png`, with unique
suffixes on conflicts. Existing files and clipboard contents are not changed.

Pasting file lists **copies** each regular file into the captured folder, with
per-file success/error reporting. It never performs a cut/move. Folders,
reparse sources and URL downloads are outside this implementation. Changing
folder during a job does not retarget it or steal the user's new selection.

The image budget is 32 million pixels and 256 MiB of encoded/decoder data.
Raw DIB snapshots and native bitmap dimensions are checked before allocating
the copied pixel buffer. Native clipboard delayed rendering can still block
the worker; it does not run on the UI thread. Other clipboard owners can
temporarily prevent access; the operation returns a retryable error.

egui-winit normally turns Ctrl+V into a **text** paste and swallows the press
for non-text clipboard contents. `native_shortcuts.rs` observes the application's
GUI-thread message queue to capture the press before that happens. It neither
injects/modifies messages nor observes other applications. Path/name text
fields retain normal text paste. If registration fails, Actions remains
available and key-release handling provides a limited keyboard fallback.

## F2 / Rename

With exactly one target selected, F2 or the Rename action opens a compact
dialogue with the stem selected. Enter commits, Escape cancels before submission.
The draft captures identity/version at opening, so an externally replaced or
edited source is rejected at commit. A collision leaves both files intact and
the dialogue displays the error.

On Windows, a rename handle locks writes/deletes, validates volume/file identity,
size and last-write time, and renames that same handle without replacement.
Metadata capture can coexist with video readers. During a committed active-video
rename the player releases the old path; short sharing-violation retries allow
it to close. On completion, the new file retains Space focus and playback seeks
back to the prior position/pause state. Failure resumes the original instead.
Gallery marks follow a renamed target. Navigation during a job is tracked
separately from watcher refreshes so completion does not steal a new selection.

Metadata deliberately restored to the captured values can evade content-version
detection; this is not content hashing. The non-Windows path-based fallback
does not offer Windows' same-handle source guarantee. Batch rename, global undo,
and persistent crash recovery are not implemented. Image zoom can reset on a
rename refresh.

## Drop opens

Dragging local media into Afio shows **Открыть в Afio**. One folder opens its
gallery; one file opens in Space. Several files from one folder open that
gallery and mark the dropped files. Different-parent groups and unsupported
files return an error. Drop never imports/moves files; Ctrl+V is the separate
copy/import gesture. The same controller is accessible through the API.

## API and verification

```powershell
photo-cli ctl paste
photo-cli ctl import 'C:\source\one.png' 'C:\source\two.mp4'
photo-cli ctl begin-rename
photo-cli ctl rename 'new name.png'
photo-cli ctl cancel-rename
photo-cli ctl drop-open 'C:\photos\one.png'
photo-cli ctl status
```

JSON commands: `run_action {id:"paste"}`, `begin_rename`, `rename {new_name}`,
`cancel_rename`, `import_files {paths}`, `open_dropped {paths}`. A direct rename
without a draft captures the current single target first. `rename` in responses
describes the open draft; file operations return the shared asynchronous job
and per-file results. `native_paste_shortcut` reports native observer installation.

47 workspace tests passed on Windows with isolated Yazi configuration; strict
Clippy passed. Native tests cover rename identity changes, occupied readers,
destination races, import collisions and temporary cleanup. Five pure clipboard
tests cover PNG/bitmap normalization, DIBV5 alpha/top-down orientation, malformed
headers, budgets and Unicode file lists without touching the user's clipboard.

`scripts/check-import-rename.py` passed against a running isolated GUI: multi-file
import, conflict suffixes, byte identity, rename drafts/collisions/stale source,
active-video rename with position/pause restoration, drop controller semantics
and successful native paste observer installation. Actual browser clipboard
paste, the physical Ctrl+V/F2 keys and drag gesture remain user-side checks.
The earlier Actions and Ctrl-selection scripts provide regression checks.

Three light GPT-6.1 Sol agents independently supplied clipboard input/native key
observation, safe import, and handle-bound rename modules. Integration, UI/API
wiring, watcher reconciliation and shared-worker checks were performed together.
