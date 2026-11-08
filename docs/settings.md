# Settings — Afiocira 2.5

This document describes 2.5.0 source being prepared for release. See the
[release record](release-2.5.md) for publication and validation status. Ctrl+, opens **Afiocira Settings**, a separate native window in the
same process. Repeating it focuses that window; Escape or Close returns focus
to the gallery. The gallery, command/dialog draft and live API session remain open.
Navigation and search select the registered Afio and Yazi modules; search filters
modules rather than individual rows. Setting rows align controls in a shared
column and place them below the title when the pane is narrow.

## Afio

Shortcuts uses the same action catalogue as the Gallery, album and Space Actions
menus. Rebindable actions can have a key or remain unbound; a conflicting binding
requires explicit replacement. Reset returns an action to its default. Tab,
Ctrl+K, Ctrl+,, Ctrl+Shift+K and gallery selection arrows remain fixed foundations.
New catalogue actions appear in Settings and the relevant Actions menu together.
Gallery, Albums and Space shortcut groups start collapsed; shortcut search expands
matching groups. Yazi keybinding layers also have collapsible categories and search.
In the Action Panel, select a rebindable command and press Ctrl+Shift+, to record
its shortcut in place. The assignment uses the same context, persistence and
conflict checks as Settings; Escape cancels recording. The editor shows the
current binding. Ctrl+Backspace removes it without removing the command;
Delete remains bindable. Modifier-only events wait for a main key, so Ctrl+H
records the chord rather than a physical left/right Ctrl key. Conflict messages
identify the chord and its existing command. Use arrows or Tab/Shift+Tab to
select conflict buttons and Enter to confirm. During initial recording, Tab
first moves to buttons; otherwise arrow keys can themselves be recorded.

Rotate left/right default to Q/E; horizontal and vertical flips start unbound.
Ctrl+L copies the current folder path in Gallery or the current file path in
Space. Ctrl+Shift+L copies the cursor file path in Gallery without opening it.
These Actions are rebindable; Yazi retains its own Ctrl+L behavior.
Settings → Afio → «Уведомления о копировании» controls the short path/raw-text
copy confirmation. It defaults on, saves independently of other preferences,
and can be changed through `set_copy_notifications {enabled}` in the local API.
These actions overwrite the same file without changing its name or format,
through a checked staging/rollback transaction. Selection and view stay in place.
PNG and static WebP transforms preserve pixels; JPEG is re-encoded at quality 95.
ICC data is preserved; JPEG EXIF orientation and dimensions are normalized.
Unsupported metadata or formats are rejected before replacement.
Space rotates and mirrors the ready texture over 230 ms while saving in the
background. Early saves do not truncate the motion; late saves hold its final
pose. Errors reverse the visual transition. Moving to another image or reopening
the collection retires the animation. API clients can observe
`render.transform_animating` and wait for it before another transform.

Video-only Space actions `copy_video_frame` and `save_video_frame` start unbound.
They capture the current decoded frame without seeking or changing playback.
Copy places full-size pixels on the clipboard (32 MP limit); Save creates a unique
PNG beside the video and focuses it if navigation has not changed during capture.
The source video remains unchanged; capture errors and timeouts are reported.

Gallery scale is continuous through Ctrl+wheel or a manual value from 0.4 to 2.5.
Reset selects automatic sizing. Resizing reuses ready thumbnail textures.
The Afio checkbox **Подстраивать карточки под окно** selects automatic sizing
for both Gallery and album rows. Turning it off restores the last manual size;
manual cards keep that size as the window changes and reflow into more columns.
The slider is disabled in automatic mode; Ctrl+wheel switches to manual mode.
The mode and remembered manual size persist independently.
Album columns follow the available viewport width and scale without a fixed
column cap. Visible rows fetch further bounded pages until their current window
is filled; off-screen albums and deeper directories are not scanned for this.

## Yazi

The adapter targets the bundled **Yazi 26.5.6**. It builds private session TOML
from the user's configuration, honoring `YAZI_CONFIG_HOME`, and adds Afio-owned
overrides. Original TOML, comments, plugins and standalone Yazi configuration are
preserved. The UI reports the private session directory and loaded font.

- **Appearance:** immediate persistent terminal text size (10–24 pt, default 12),
  separate dark/light flavors, inherited or minimal icons, and a theme-card grid
  with cached preview thumbnails and an Apply control on each card.
  The GitHub flavor installer accepts `owner/repository:subdirectory` (for example
  `yazi-rs/flavors:catppuccin-mocha.yazi`). It requires Git and Internet access,
  fetches data without checkout or scripts, records the resolved revision, keeps
  license attribution and validates data/package structure before installation.
  Native compatibility is checked by the candidate before applying. Installed
  flavors can be applied from their cards.
  A separate Dark/Light mode control activates the configured variant for Yazi
  alone. Switching mode uses the verified local Yazi replacement because this
  Yazi version reads the terminal background mode at startup; Afio stays dark.
- **General:** selected visibility, sort, file-label and cursor-scroll settings.
  Rows show effective values and provenance; Reset removes the Afio override and
  reveals the inherited value.
- **Quicklinks:** named `g`, then key folder bindings with a native folder picker
  and test action. Paths use safe `cd --raw` arguments. Entries can be disabled,
  soft-deleted and restored; conflicts require explicit replacement.
- **Keybindings:** native editing for `mgr`, `input`, `confirm`, `tasks`, `spot`,
  `pick`, `cmp` and `help`, including effective inherited defaults and their source.
  Defaults are copied from the pinned Yazi version; see
  [preset attribution](../assets/yazi-defaults/README.md).
  Edit expands beside the selected row. Recording accepts actual key presses,
  modifier chords and sequences up to eight keys. Escape restores the previous
  sequence; leaving the page or closing Settings stops recording.

Hold the middle mouse button inside Settings or the Action Panel list and move
vertically to scroll quickly. A dead zone avoids drift; release stops scrolling.
Plain Tab still switches panes; Shift+Tab opens/closes native Yazi file details.

On Windows, the paired `photo-cli.exe` initializes only the embedded Yazi's own
ConPTY before launching it. The pinned Yazi reads the selected background through
its startup OSC 11 handshake; a startup response bridges older ConPTY versions
that swallow this query. A private startup receipt records the actual Yazi PID.
The helper and its descendants share a kill-on-close job so replacing a pane
does not leave its Yazi process behind. Standalone terminals are unaffected.

General, Quicklinks, bindings and flavor/icon choices are staged until **Apply to
Yazi**; Discard restores the applied profile. Apply, Discard, progress and errors
remain visible outside the scrolling content. Theme-card Apply uses the dedicated
`apply_appearance` request and refuses pending General, Quicklink or binding
changes; apply or discard those drafts first. Apply validates a candidate before
changing the live pane. General/keymap changes replace only embedded Yazi,
restoring its current folder, hovered file, marks and runtime preferences.
Appearance-only changes use live `app:theme`; confirmation checks actual style
fingerprints and failed reloads roll back the theme configuration.
Read-only installed flavor files are copied into writable private session files;
hot reload copies only the selected flavors. Source files and their attributes
stay intact, including when multiple Afio instances share the same base config.
Profiles are merged against the latest saved document under an OS file lock.
Unrelated edits survive; conflicting edits or merged key/prefix collisions fail
explicitly before changing the live pane. The local draft remains available.

Apply currently requires one Yazi tab, deterministic sorting and no active Yazi
tasks or dialogs. Replacement also requires no pending internal Yazi copy/move
selection; appearance-only reload keeps that state in the original instance.
Native Yazi back/forward history is not restored during pane
replacement. This is a focused adapter, not a general TOML or plugin editor.
It adds no history database, SQLite, ComfyUI, curves or batch editing.

## Persistence

Settings live in `%APPDATA%/Afiocira/settings.json`, schema 1, outside installed
and working-version directories. `AFIO_SETTINGS_HOME` overrides the directory
for isolated tests. Saving preserves unknown JSON fields, syncs a temporary file,
saves `settings.json.previous`, then publishes by rename. Malformed or unsupported
schema files are not overwritten. Errors appear in UI and API. Host JSON stores
the overrides separately from user Yazi TOML.

## Shared API

Keyboard, native UI and API use the same dispatcher. `photo-cli ctl settings`
and `photo-cli ctl close-settings` open and close Settings. JSON examples:

```json
{"command":"show_settings"}
{"command":"select_settings","section":"yazi","page":"General"}
{"command":"set_yazi_text_size","size":18}
{"command":"set_gallery_scale","scale":1.2}
{"command":"set_gallery_scale","scale":null}
{"command":"set_shortcut","action":"rotate_left","context":"space","binding":"Q"}
{"command":"configure_yazi","request":{"op":"set_general","key":"show_hidden","value":"true"}}
{"command":"configure_yazi","request":{"op":"apply"}}
{"command":"configure_yazi","request":{"op":"inspect"}}
{"command":"configure_yazi","request":{"op":"apply_appearance","dark":null,"light":null,"icons":"minimal"}}
{"command":"close_settings"}
{"command":"run_action","id":"copy_video_frame"}
{"command":"run_action","id":"save_video_frame"}
```

Use a null General value to reset an override. `set_shortcut` accepts `reset:true`
or `replace:true`; a null binding leaves a rebindable action unbound. Inspect the
catalogue for current action/context values before changing shortcuts.
`show_settings`, `select_settings` and `configure_yazi` Inspect return full lists;
ordinary responses carry compact settings status. Apply and installation are
asynchronous: poll status until `busy` clears and inspect `error`/`message`.
Contracts are in `crates/photo-core/src/settings.rs` and `api.rs`.

## Verification

`python scripts/check-settings.py`, `python scripts/check-yazi-settings.py` and
`python scripts/check-afio-settings.py` exercise isolated native API sessions.
The Yazi suite includes a real `g`, then `l` Quicklink, hot theme changes and
invalid-configuration rollback; `--network` also checks flavor download.
`check-theme-instances.py` covers read-only flavor files and two isolated GUI
sessions sharing settings; `check-album-capacity.py` checks viewport-driven paging.
`check-video-frame.py` verifies decoded-frame saving on synthetic video.
Unit tests include stalled helper and terminal-writer scenarios. Yazi helper
commands run through a bounded, serialized background queue, and terminal input
writes run off the UI thread. Snapshot, cursor/marks, navigation, order and theme
requests use atomic per-session file mailboxes consumed by the paired Yazi.
Pane switching does not depend on the shared Windows DDS listener. Pending
snapshots retry through the same mailbox; pane teardown attempts bounded
graceful exit in the background.
Shared UI/terminal font fallback, wide-cell placement
and terminal clipping improve glyph rendering without promising every Unicode
character. Unit tests and strict Clippy cover the implementation. Native window appearance
and physical keyboard interaction still require desktop feedback; automated
checks do not certify every user plugin or configuration.
`check-pane-focus.py` covers repeated pane handoff with cursor and mark transfer;
the album-capacity check also preserves album position across that handoff.
