# Lazy nested browsing, layout, cache and Space processing

## Album browsing (updated after user clarification)

Nested media are now album rows, one named folder per row, with1–3 visible centered thumbnails depending on width. The current folder's own media form a row too when subfolders exist. A folder without subfolders retains the ordinary grid. Captions use filenames; the folder is already in the row header. Each album remembers its independent position.

Left/Right select images in that album; Right at the end of known contents continues ONLY its directory iterator and advances after results arrive. Up/Down changes albums and restores that album's previous cursor. Space opens the selected image; Enter enters that album as an ordinary local grid with the same file under its cursor. Back from Space returns to the current gallery. Page counters show loaded contents with + until exhaustion; there is no fake global total.

The current folder's direct children are counted/named shallowly in a background worker. Only visible first-level rows receive a small initial page while Gallery is active. Offscreen rows stay unopened. A deeper folder row appears only after examining its parent; it is never automatically entered merely because it is visible. Explicit selection/click/Right continues it. No recursive initial scan, folder cards, or second file manager.

Each selected album page reads at most512 entries and5 media; the existing legacy batch API remains available for compatibility. The worker coalesces duplicate requests for the same in-flight album. Session changes cancel/discard stale results. Limits remain depth8,512 discovered folders and2000 descendant media. Symlinks/junctions are skipped. Known media are grouped by album with path-based cache remapping; appending a later album page does not require decoding survivor textures again.

Tab retains the Yazi/Gallery switch. Marks, selected file, album positions and loaded contents survive normal Space/Back and panel switches. Watching is limited to the root and already displayed directories. New direct folders update the shallow count. New files in an already exhausted child are not guaranteed to enter an old iterator: opening that directory through Yazi provides a fresh local listing. Slow filesystem reads occur off UI, with cancellation checked between entries.

API: load_album {folder} accepts only a known descriptor; album_move {horizontal:-1|0|1,vertical:-1|0|1} drives the same gallery navigation. status.nested includes albums[{folder,depth,loaded,exhausted,children,error}], active_album and album_positions. Root media row is derived from Gallery membership, not duplicated in nested.albums. Legacy load_nested/deeper remain for older clients; UI no longer uses a global “more examples” button.

## Layout

Thumbnail hitboxes have fixed sizes, while actual aspect-fitted images are centered horizontally and vertically within them. Square/portrait files no longer hug an edge. Space computes its fitted rectangle from the intersection of logical layout space and the actual clip rectangle; wrapping toolbar controls and truncating the filename prevent narrow panels from widening the image canvas beyond the window. Explicit user zoom remains zoom; Fit is centered and fully visible. Geometry tests cover portrait/square/16:9 at multiple widths and an oversized logical panel clipped to300px.

## Cache

A successful F2 updates the gallery path identity before membership refresh; remapping keeps the same TextureHandle IDs through reordering. Duplicate/paste/rename/delete refresh membership without throwing away survivor textures. Real content modifications invalidate affected entries; rename/create/remove notifications alone do not discard survivor pixels. Processing invalidates its output only. Manual F5 still deliberately reloads pixels. Cache stays bounded (160 thumbnails,12 full previews/128MiB preview budget); there is no eager decode of every file or unbounded forever-cache. Pending decode generations are retired when indices move; unchanged loaded textures survive.

API render.preview_textures / thumbnail_textures expose path and textureID for regression checks. They describe CPU/GPU resource identity, not actual monitor presentation time.

## Space commands from two light Sol modules

Actions → “Размер и вес…” creates an atomic unique copy alongside the source. Width/height are optional bounds with aspect preservation by default. Target weight is a maximum byte count, not an exact size: JPEG quality search at requested dimensions; lossless PNG/WebP reject an impossible target. Actual dimensions and bytes appear after completion. ICC is retained, orientation applied; other EXIF/XMP removed from the new copy with disclosure in the dialog. Animated input, CMYK JPEG, unsafe precision/alpha loss and excessive memory are rejected.32MP nominal pixel cap and conservative256MiB working estimate may reject smaller large images too.

Actions → “Конвертировать формат…” supports still8-bit PNG/JPEG/WebP. It verifies encoded dimensions/pixels (lossless formats), prepares sibling staging, binds source identity/content and uses no-clobber checked publication. Default replaces the original after verified output is ready; Ctrl+D first if a separate original is wanted. Existing target paths are never overwritten. Failure restores original when possible and reports retained recovery paths. A crash may leave the private staging/backup file; this is recoverable publication, not an atomic transaction across two different names. JPEG alpha and animated/high-depth/CMYK inputs are rejected. Metadata/ICC stripping requires an explicit checkbox; removing ICC can change appearance because there is no color management. Source/output64MiB caps, predecode memory estimates and finite decoder limits bound work.

Both reuse the existing image crate codecs/resampler, not new engines. Their dialogs capture source/version, run through the existing bounded operation worker and do not retarget when selection changes. Only one still image in Space currently; no batch processing. API run_action resize_image/convert_image opens the dialog; submit_image {options:{format:"png"|"jpg"|"webp",width,height,preserve_aspect,target_bytes,quality,strip_metadata}} commits the captured draft; cancel_image closes. Poll operation.running for completion. CLI ctl action ID / cancel-image; structured arguments via ctl-json.

## Dialog design and keyboard

All17 registered actions have an exhaustive Lucide SVG mapping, including Scaling and Replace for the image commands. The two image dialogs use rounded gray frames, header icons, filename subtitles, aligned fields, segmented formats, compact context, and footer Cancel/Esc + Apply/Enter. Focus has a visible border. Tab/Shift+Tab cycles all current controls within the form; Space activates toggles/segments; Enter applies and Esc cancels when idle. Opening Enter must release before a fresh Enter can apply, and repeats are ignored. Busy forms do not cancel a running file transaction.

Rename cycles field/cancel/save; Delete cycles cancel/delete while retaining the explicitly requested global fresh-Enter confirmation. Everything and Actions keep Tab in their search field, with arrows for the list. Action palette Enter resolves after query editing, so same-frame typing cannot execute stale filtered content. Ctrl+C/V in palette search edit text rather than launch media operations.

## Verification

Current full workspace:128 unit/native tests +3 doctests passed, strict Clippy and locked Release passed. Added targeted album iterator test proves paging A leaves B untouched. Focus-ring/Enter tests cover wrap, reverse, outside focus, release and repeat cases. Native scripts/check-albums.py creates only target/album-check-* synthetic fixtures, navigates5 positions into a loaded album, confirms continuation from5 to10 without changing neighbour counts, then Down/Up restores position5. Deeper rows remain unopened until requested. Same application API captures real gallery, both dialogs, Actions and a680×520pt narrow-window dialog. Screenshots target/albums-*.png were visually inspected. Capture reads only the application's own framebuffer; no desktop input/clipboard used. Raw result target/albums-native-result.json.

API capture_frame {path:absolute PNG} and resize_viewport {width,height} provide repeatable visual checks; resize bounds680–4096 by460–4096 points. These are native app commands, not OS desktop automation.

Previous synthetic scripts/check-nested-images.py covers membership/cache and image processing; core/module tests retain those checks. No claim that all platform focus edge cases, network folders or every image format are covered; the user's visual/keyboard confirmation is still pending.

## Album Tab/Enter handoff correction (2026-11-07)

Overview Grid → Tab highlights the current album directory in its parent in Yazi, including empty/video albums. It does not reveal a member file. A scoped folder-handoff token preserves the overview, independent album cursor and marks on an immediate Tab back. Real terminal navigation clears the Yazi guard; a snapshot of a different directory/file may then replace the old view. The former broad “any loaded descendant belongs to this overview” retention was removed because it trapped navigation in the old album list.

Enter → real local Gallery grid of the chosen album; the cursor is selected by full path, marks within that directory are retained, cached textures are remapped by path. If that album also has children but has its own media, it remains an ordinary grid rather than silently returning to overview mode. An empty container with child folders may naturally show its child albums. Space remains file preview. After entering, Tab again reveals the selected file in Yazi as in the original local-grid workflow. The palette exposes “Войти в альбом” with Enter and a folder icon only in overview; file Open shows Space there. A file opened in Space uses ordinary file handoff too.

API enter_album / run_action enter_album and CLI ctl enter-album share the Enter handler. status.nested.overview identifies the active overview presentation. navigator_snapshot / ctl navigator-snapshot requests the actual Yazi cwd and hovered path (ordinary status does not synchronously query Yazi).

Verification:129 unit/native tests +3 doctests, strict Clippy and locked Release passed. The pure handoff-policy test distinguishes exact folder restoration, actual cd into the album and navigation elsewhere. scripts/check-album-return.py passed against a separate native API session:4 overview roundtrips, empty-folder handoff, video-file-in-album handoff, Enter into a12-image local grid that also has a child folder,3 ordinary file roundtrips, unchanged thumbnail textureID, and Space file preview. Marks and independent album position retained. Synthetic fixtures only; no desktop input or clipboard. Raw result target/album-return-result.json.
