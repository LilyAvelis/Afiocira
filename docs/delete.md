# Delete / Windows Recycle Bin

Delete and the final red Actions row use the same command dispatcher. Space
always targets its current image or video, regardless of Gallery marks. One
Gallery target is also immediate. Two or more Gallery targets open a Russian
confirmation with the captured count: Enter confirms and Escape cancels. The
opening frame cannot confirm, and repeated Delete presses are discarded.
Name/path text fields keep their text editing shortcuts.

The request captures ordered paths and source identity, size and last-write
metadata. Later changes to marks or cursor never retarget confirmation. Leaving
the folder requires cancellation and a fresh request. Source capture failures
and changes become per-file OperationState errors, permitting partial results.

A bounded background worker invokes Windows IFileOperation separately per file
with FOFX_RECYCLEONDELETE, FOFX_EARLYFAILURE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION,
FOF_NOERRORUI, FOF_SILENT and FOF_NO_CONNECTED_ELEMENTS. A progress sink checks
source metadata again before deletion and rejects non-recycle transfer flags.
Success requires a successful PostDeleteItem callback, a non-null Recycle Bin
item receipt and no aborted operations. Positive success statuses (including
COPYENGINE_S_DONT_PROCESS_CHILDREN) are accepted with that receipt, not mistaken
for errors. Canonical Windows path prefixes are normalized losslessly for Shell.
There is no permanent-delete fallback. Unsupported locations or a failed Recycle
Bin operation produce errors. The active video is stopped before work begins.

Without intervening navigation, Space opens the next surviving original file,
or the preceding survivor at the end. Gallery keeps surviving marks, including
failed targets, and chooses the nearest surviving cursor. Navigation during the
job prevents completion from stealing the user's new place.

The Shell API operates on paths. A hostile external replacement between the
final metadata check and Shell processing remains a race; deliberately restored
metadata can evade version detection. Recycling provides Windows recovery,
not an application undo journal.

API: RunAction { id: "delete" }, ConfirmDelete, CancelDelete. Status includes
optional delete_confirmation { paths, count }. Running/completed OperationState
uses action "delete"; each FileResult has its source, null destination, and
optional error. CLI aliases: ctl delete, ctl confirm-delete, ctl cancel-delete.

Reference: [Microsoft IFileOperation flags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags)
and [PreDeleteItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperationprogresssink-predeleteitem).
Unit tests use synthetic files and mock recycling; they do not modify Recycle Bin.

The full workspace suite passed 54 tests with isolated Yazi configuration and
strict Clippy. `scripts/check-recycle-live.py` also passed against the real Shell:
Space/single/group scope, cancel/confirmed captured targets, stale-source partial
failure, active-video deletion, next-file selection and last-row ordering. Six
synthetic Recycle Bin receipts were verified and all test items restored with
their expected contents. No user's existing media or clipboard was touched.

Delete refresh now preserves unaffected cached thumbnails/previews by path.
See [before/after latency measurements](delete-performance.md).
