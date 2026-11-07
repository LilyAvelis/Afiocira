# Everything search

Ctrl+Shift+K, Actions → Поиск Everything, or the top search button opens a centered search modal. Type native Everything syntax, use arrows and Enter (or click), Esc closes. Photos/videos open in Space, folders in Gallery; other files reveal in embedded Yazi without executing them. Typing and navigation are isolated from Gallery/Yazi keyboard input.

The official voidtools ES 1.1.0.38 x64 helper queries the running Everything index through IPC. No custom indexing or recursive scan. Requires full Everything (not Lite), running in the user session. Missing helper/service produces a search error; normal viewing stays usable. Setup: `./scripts/setup-everything.ps1 -OutputDirectory target/release`; archives are SHA256-pinned. Package es.exe plus Everything-ES-LICENSE.txt and Everything-ES-PROVENANCE.txt beside photo-gui.exe. Existing desktop shortcut is updated by scripts/update-working.ps1.

Search uses a single bounded worker,120ms debounce, maximum100 results, Unicode UTF-8 output, three-second deadline and stale reply rejection. Search query is one literal process argument after --, never a shell command. Total count is a second IPC query and may briefly differ as the index changes. ES does not change user search settings. There is no pagination yet; refine a broad query.

Opening a result reads only its immediate directory on a background thread; spinner “Открываем папку…” remains cancellable via Esc. An already active directory read finishes in the background after cancellation. Only one such read is allowed at a time. Gallery installation remains on UI. Existing Yazi navigates via an asynchronous bounded DDS request; no PTY/config restart. Late old-folder reports are filtered. Errors are surfaced in the message bar.

API commands: show_search,close_search,search_files {query},open_search_result {index}. Status.search contains query/loading/results/total/selected/error/opening/open_ms. open_search_result acknowledges acceptance; poll opening=false for completion. Indices address the current completed query only. CLI: ctl search / close-search / search-files QUERY / open-result INDEX.

## Checks and measurements (2026-11-07)

Synthetic Unicode PNG/MP4/folder/TXT results exercised through live named-pipe API; checked views, nonmedia Yazi focus, empty results,100 cap, rapid-query stale replies, close/reopen and Search action with Delete still last. Agent also checked switch-like input (-exit), file/folder attributes, hidden Unicode output, real Yazi folder cd/file reveal and unchanged Yazi process identity. scripts/check-everything.py uses an isolated pipe (AFIO_API_NAME, default afio-everything-preview) and fixture directory listed in target/everything-fixture-path.txt; does not alter clipboard or drive desktop input.

Before correction, open_search_result synchronously restarted Yazi and cost2281–2297ms on a tiny directory. After correction, request acceptance0–16ms and background read+UI installation1.5–7.2ms (four samples); separate native Yazi transition72–141ms. This excludes full image texture readiness/display. Live query completion varied roughly0.3–1.0sec including debounce/processes/UI scheduling; not claimed instantaneous. Large, network, and cloud directories have not been benchmarked. Raw local results target/everything-live-check.json.

## Remaining foundation tasks

The 2.0 pitch still includes lazy nested-folder exploration (direct subfolder count, honest empty-state, bounded depth/batches without a second file manager), batch F2 with name preview/conflict policy, and configurable action shortcuts. Drag-in currently opens media; full drag-out or copy/move gestures require separate semantics. These are independent of future Space image processing. Format conversion must decode/encode bytes, not merely rename an extension.
