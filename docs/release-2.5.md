# Afiocira 2.5.0 release record

This record describes the 2.5.0 source and Windows packages in the public LilyAvelis/Afiocira repository. Release notes, installer, portable ZIP and checksums: [v2.5.0](https://github.com/LilyAvelis/Afiocira/releases/tag/v2.5.0). The [2.0 release record](release-2.0.md) is historical evidence for that release only.

## Changes since 2.0

- **Native Settings:** Ctrl+, opens or focuses a separate native viewport without closing the gallery, draft dialogs or API session. Afio shortcuts share the Actions catalogue, context resolver and conflict checks. Ctrl+Shift+, assigns a shortcut to the selected Action Panel command.
- **Yazi configuration in the GUI:** dark/light flavors, cached theme cards, per-card Apply, inherited/minimal icons and a revision-recording data-only GitHub flavor installer; selected General overrides with provenance/reset; safe folder Quicklinks with disable/delete/restore; inline keybinding editing and actual chord/sequence recording up to eight keys against pinned Yazi 26.5.6 defaults.
- **Yazi appearance and input:** a true Yazi-only Dark/Light control activates the configured variant while Afio stays dark. The ConPTY startup handshake and process receipt support replacement of only the embedded pane. Shift+Tab opens native file details across native and negotiated key representations. Middle-button hold scrolling works in Settings and Actions.
- **Image actions:** rotation and mirroring save to the same filename and format through checked replacement and rollback. Space animates the ready GPU texture for 230 ms while saving; early/late completion, errors and navigation are handled. Static PNG/WebP preserve pixels; JPEG is re-encoded at quality 95, with documented metadata guards.
- **Video frames:** default-unbound Space actions copy current decoded-frame pixels or save a unique PNG beside the video, without seeking or modifying the source video. Clipboard capture has a 32 MP limit; failures and timeouts are reported.
- **Gallery and albums:** Ctrl+wheel continuous scaling, viewport-driven album capacity without fixed column caps, and an explicit automatic-sizing checkbox. Disabling automatic sizing restores the last manual size; mode and manual size persist independently. Ready thumbnail textures are reused.
- **UI Foundation:** shared Settings geometry, spacing and typography, responsive localized rows, font fallback, wide-glyph placement, clipping and album-header containment.
- **Architecture and reliability:** ordered private per-session requests, observed-state acknowledgements and cancellation replace dependency on shared Windows DDS for handoffs. Selection scopes preserve unrelated native marks across albums, and deletion targets are fenced by collection identity. Settings merge against the latest document under an OS lock with semantic conflict checks. Helper work and terminal writes run off the UI thread with bounded queues; IPC limits clients, frames and deadlines. Same-path replacement refreshes affected caches while retaining survivor textures and verified transformed pixels. Checked image reads avoid repeated exact buffer growth. These repairs include the eight Azuros review findings and two follow-up findings addressed during release preparation.

See [Settings](settings.md) for precise application guards, persistence, API examples and test scripts. Yazi replacement requires one tab, deterministic sorting and no active tasks/dialogs or pending internal copy/move selection; native back/forward history is not restored. Appearance-only hot reload retains the running pane. User TOML remains separate from Afio overrides.

## Validation status

The final full local run passed **191 unit/native tests and 3 doctests**, including the automatic-sizing regression test. Formatting, strict Clippy and the static-CRT release build passed. Isolated native API scenarios supplement these checks. The package manifest records the exact source commit and staged file hashes; SHA256SUMS.txt covers the installer and ZIP.

The staged 2.5.0 portable package passed its manifest file hashes and listed all three packaged test images through its CLI. Its GUI passed native Yazi Dark/Light switching, return to Dark, persisted Light after restart, preserved Gallery context and source configuration, and refusal of an unrelated draft. This ran with private APPDATA/LOCALAPPDATA, System32-only PATH and an empty substitute WINDIR to exercise packaged runtime/font discovery. The GUI, CLI and launcher contained neither the actual workspace nor user-home path in UTF-8 or UTF-16LE after source-path remapping. No libmpv or FFmpeg binaries were present in the stage.

The installer passed silent isolated TESTMODE installation and uninstall. The installed launcher resolved its 2.5.0 executable, its CLI listed three images, and its GUI applied Settings and native Light mode through the API. Video setup was disabled for this fixture. TESTMODE uses a separate uninstall identity and skips real shortcuts/Open With registration; this is package-layout validation on the development machine, not independent second-machine certification. Reproduce with `python scripts/check-release-install.py`.

Native API suites cover Settings, Yazi candidate validation/rollback, real Quicklinks, theme application, shared read-only flavors in isolated instances, pane focus/marks, album capacity and synthetic-video frame saving. Automated tests do not blanket-certify physical keyboard interaction, user-reported Settings appearance, every plugin, codec or filesystem. Independent installation on a second unprepared Windows 11 x64 machine remains outstanding. Remote Windows checks are reported in [GitHub Actions](https://github.com/LilyAvelis/Afiocira/actions).

## Packaging and reproduction

1. Run formatting, strict Clippy and locked workspace tests on the release source. Native Yazi tests require the pinned Yazi/ya and usable bundled font.
2. Build the locked release workspace with static CRT and source-path remapping, using the Windows workflow configuration and target/distribution.
3. Stage with scripts/stage-windows-release.ps1. Verify pinned downloads, third-party notices, source provenance and every staged manifest hash.
4. Compile scripts/Afiocira.iss with Inno Setup and create the complete portable ZIP. Generate SHA256SUMS.txt for the installer and ZIP; retain PACKAGE-MANIFEST.json and the exact source commit.
5. Validate the staged and installed launchers in isolated local configuration, record results, then publish the approved public source/tag and assets at v2.5.0. The manual/PR workflow creates a candidate; it does not publish automatically.

The installer is unsigned and installs per user. Open With registration does not change defaults. Portable users must extract the complete folder. Yazi, the ES helper, font, runtime installer and notices accompany the package; Everything itself is optional and installed separately. **libmpv and FFmpeg binaries are not bundled:** optional installer video setup or portable Enable-Video.cmd downloads pinned, checksummed upstream distributions on the user's machine and requires Internet access. Downloads can be retried with the helper.

No history database/SQLite, ComfyUI, curves, batch editing, ICC-managed display, animated-image playback, full-resolution 1:1 mode or general undo is claimed. See the README for format and operation boundaries.
