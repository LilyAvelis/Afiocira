# Changelog

## 2.5.0 — 2026-11-08

Full release notes and Windows downloads: [v2.5.0](https://github.com/LilyAvelis/Afiocira/releases/tag/v2.5.0).

- Add an explicit automatic card sizing checkbox for Gallery and albums; retain the manual size across mode switches and restarts.
- Add a separate native Settings window, shared UI/API dispatcher and persistent host configuration with backup.
- Add Afio shortcut remapping with explicit conflict replacement and fixed foundational keys; align responsive setting rows and group shortcut/keybinding categories with search.
- Assign shortcuts directly to the selected Action Panel command with Ctrl+Shift+, using the shared Settings resolver.
- Add continuous gallery scaling and rotate/flip actions that overwrite the same image with checked replacement, preserving its name and format.
- Add default-unbound video-only Space actions to copy current decoded-frame pixels or save a unique PNG beside the video.
- Add Yazi dark/light flavors, icon choices, cached theme-card previews, in-card appearance application and a revision-recording, data-only GitHub flavor installer.
- Add selected General overrides with provenance/reset, safe folder Quicklinks with soft deletion/restore, and native keymap editing against pinned Yazi 26.5.6 defaults.
- Validate candidate Yazi configuration before applying it; hot-reload appearance and restore current folder, hover, marks and preferences when replacing the embedded pane.
- Keep user Yazi TOML separate from Afio overrides; keep Apply/Discard and status visible while scrolling and add isolated native API verification scripts.
- Remove unbounded UI-thread waits for Yazi helpers and terminal input through bounded background work; test stalled helpers/writers.
- Share UI/terminal font fallback, correct wide-glyph placement and terminal clipping, and contain album-header overflow.
- Fill album thumbnail windows dynamically from viewport width and scale, without fixed column limits.
- Fix theme-card application with read-only installed flavors while preserving source attributes and isolation between Afio instances.
- Restore Tab handoff and cursor/mark synchronization through paired session mailboxes, independent of shared Windows Yazi DDS failures.
- Give private requests ordered IDs, observed-state acknowledgements and cancellation; make focus handoffs nonblocking and preserve unrelated native selections across albums.
- Fence captured deletion targets by collection identity, merge concurrent settings edits with semantic validation, and bound IPC clients, frames, queues and deadlines.
- Refresh same-path replacements without discarding unaffected textures; keep the previous image visible and reuse verified saved pixels after transforms.
- Remove repeated exact buffer growth from checked image reads while retaining source/version/readback and rollback checks.
- Share Settings geometry, typography and spacing through UI Foundation; verify responsive localized rows and input at multiple DPI values.
- Animate Space rotations and mirrors using the ready GPU texture while the checked save runs; handle early completion, failure rollback and navigation.
- Add an explicit Yazi-only Dark/Light mode control, preserving the configured variant and Afio's dark appearance.
- Edit Yazi keybindings inline and record key sequences/chords; add middle-button scrolling to Settings and Actions.
- Restore Shift+Tab file details across Windows ConPTY's native and negotiated terminal key representations.

See [Settings](docs/settings.md) for current application guards and the Yazi history limitation. This release does not add history/SQLite, ComfyUI, curves or batch editing.

## 2.0.0 — 2026-11-07

Afiocira grows from a photo-navigation gallery into a mixed-media workspace.

- Embed libmpv video playback and FFmpeg thumbnails in the mixed gallery.
- Add context Actions, vector command icons and keyboard-operated resize/format forms.
- Add multi-select, file/pixel clipboard copy-paste, duplicate, single-file rename, drag-in open and Recycle Bin deletion.
- Add centered Everything search through the official ES IPC helper.
- Browse nested directories as independent lazy album carousels.
- Correct album Tab handoff and Enter-to-local-grid behavior.
- Keep decoded survivor textures through rename, reorder and deletion; fix narrow-window and thumbnail centering.
- Preserve exact terminal colors instead of brightening bold Powerline separators.
- Split application concerns and commands into modules; provide a permanent working-version launcher.
- Ship Windows x64 installer/portable package, pinned runtimes, third-party notices and SHA256 checksums.

See README for format support and remaining limitations. This release follows the original local1.0 workflow; v2.0.0 is the published GitHub release tag.
