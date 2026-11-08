# Architecture checkpoint after Terra review

Video is now embedded through libmpv; see [video checkpoint](video.md) for its
rendering, runtime, asynchronous commands, validation and current limits.
The historical `Photo`, `photos` and `View::Photo` names now also cover movies.
The image-only decode limits below still describe still-image previews.


## Ownership and flow

| Owner | Responsibility |
| --- | --- |
| `photo-core::Gallery` | Ordered folder snapshot, cursor, marked paths, range selection, photo view, zoom math. No GUI dependency. |
| `photo-core::api` | Versioned serializable command, response, session snapshot, error code. |
| `photo-gui::Workshop` | One live session and dispatcher. GUI keys/buttons, Yazi folder reports, and local API requests call `execute(Command)`. |
| `photo-gui::decode` | Separate bounded thumbnail/preview queues. Work from an older folder generation is dropped. |
| `photo-gui::yazi` | Native Yazi in ConPTY with a private config copy; reports hovered folders with monotonic event serials and exchanges state/commands through per-session file mailboxes and a private plugin. Core pane handoff does not require shared DDS. |
| `photo-gui::ipc` | Windows local named pipe. Requests are JSON lines; responses are JSON lines. |
| `photo-cli` | `ctl` and `ctl-json` control the live GUI. `browse` and `open` are independent offline sessions for core debugging. |

`Workshop` is still both application service and GUI host. If commands grow, separate the service from rendering before adding editing or ComfyUI tasks. Only one process can own the current pipe name; later instances show an API warning and remain usable as viewers.

## Live command contract

Protocol version is `3`; the pipe name and Rust types are in `photo-core/src/api.rs`. Responses carry `session_id`, gallery snapshot, navigator state, and `error` with a code and message. The snapshot exposes `cursor_path` and `marked_paths`; `list_order` additionally returns `ordered_paths`. Indices are zero-based and valid only for the current ordered folder snapshot. Clients should query `status` again after a folder change and use paths for durable references. Paths in JSON are Windows paths; no shell evaluation occurs.

| GUI action | Command | CLI |
| --- | --- | --- |
| Open folder or file | `open` | `ctl open PATH` |
| Select thumbnail | `select` | `ctl select INDEX` |
| Select exact photo | `select_path` | `ctl select-path PATH` |
| Extend marked range | `extend_selection` | `ctl extend-selection INDEX` |
| Replace gallery marks | `set_marked` | `ctl mark [PATH ...]` |
| Clear marks with Escape in the grid | `set_marked` with no paths | `ctl mark` |
| Apply navigator order | `set_order` | `ctl order PATH ...` |
| Read current order | `list_order` | `ctl list-order` |
| Enter or Space on selection | `open_selected` | `ctl open-selected` |
| Next / previous | `next` / `previous` | `ctl next` / `ctl previous` |
| Escape or Space in photo | `back` | `ctl back` |
| Zoom / pan / Fit | `zoom_at` / `pan_by` / `reset_zoom` | `ctl zoom` / `ctl pan` / `ctl reset-zoom` |
| Show/hide Yazi | `set_navigator_visible` | `ctl show-navigator` / `ctl hide-navigator` |
| Tab between panes and synchronize cursor/marks | `focus_pane` | `ctl focus-gallery` / `ctl focus-navigator` |
| Yazi hover | `preview_folder` | `ctl-json` |
| Refresh button or F5 | `refresh` | `ctl refresh` |

For example, `photo-cli ctl status` reports the active frame, and `photo-cli ctl open-selected` changes that same session. `ctl-json` accepts a single line such as `{"command":"status"}` from stdin. The GUI handles API requests on its UI thread, so commands are serialized with input and rendering. API transport is local to the machine; it is not an HTTP service or a remote automation endpoint. The transport does not yet provide authentication beyond Windows pipe access control.

## File and Yazi boundary

Yazi remains a full file manager. Its rename, move and delete actions are external to Photo Workshop. Its sort keys report Yazi's actual ordered file paths through a private plugin; the gallery applies them through `set_order` and preserves selection and decoded textures by path. The private keymap keeps the user's bindings and augments sort actions; global Yazi config is untouched. Mouse clicks reach Yazi as SGR reports, and a GUI double click enters the hovered folder. The app uses nonrecursive filesystem notifications, a 350 ms debounce, and `Gallery::refresh` to reconcile the current folder. Selection is restored by path when possible. If the open file disappears, the view returns to the grid. If files are inserted or removed, image caches are invalidated; modifying one file invalidates that image. The watcher may miss events or be unavailable, so `ctl refresh` forces a rescan.

There is no app file-write command or undo journal yet. Do not claim that Yazi actions are undoable through this app. Before adding move/rename/delete, define conflict handling, source identity/version checks, cross-volume behavior and interruption recovery, then test them in temporary folders. A future AI job must bind to immutable source path/version, not the current grid index.

## Decode and image limits

The visible grid submits thumbnails to a 24-entry queue. The current photo is prioritized in a separate 24-entry preview queue. The result channel holds at most four images; the preview texture cache has a count limit of 12 and estimated RGBA budget of 128 MiB. Generation checks discard queued and completed work from an older folder, but a decode already in progress cannot be interrupted. The decoder uses a 256 MiB allocation limit, which is best-effort for some image formats. It can still allocate memory for a full original before shrinking it to 320 or 2560 pixels. A very large image or cloud placeholder can therefore take time and memory. This has not been measured across the complete iCloud collection.

EXIF orientation is applied. The app does not implement ICC transforms, GIF playback, HEIC/HEIF/AVIF decoding or full-resolution tiling. The zoom factor is relative to the fitted 2560-pixel preview; it is not source-pixel 1:1. Keep this distinction in UI and documentation until the rendering path changes.

## Verification and next boundary

`cargo test --workspace` covers core selection/zoom and external removal, decode queue bounds/priority, Yazi config and a local ConPTY smoke test. A hidden GUI process was controlled through `photo-cli ctl` to confirm live session state for selection, opening, zoom, pan, back and refresh. These tests do not verify visual appearance or performance of 8000 cloud photos. The user checks the interface visually.

Before daily use on a large photo library: measure cold/warm opening and first preview, p95 navigation latency, peak process/GPU memory, and iCloud network activity; move folder enumeration off the UI thread if it misses the latency target. Before editing: add durable file identity/version rules, a tested write/undo contract, a full-resolution rendering strategy, and color handling. Recheck Yazi configuration portability and the installer on a clean machine. ComfyUI integration comes after the standalone viewer and write boundary are reliable.
