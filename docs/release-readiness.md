# Public release validation

Afiocira 2.5.0 source, installer, portable ZIP and checksums are associated with [v2.5.0](https://github.com/LilyAvelis/Afiocira/releases/tag/v2.5.0). The [2.5 release record](release-2.5.md) describes the full implemented scope, reproduction steps and evidence. Preserve the [2.0 record](release-2.0.md) as historical evidence; public history retains the original 1.0 commit and does not merge private development history.

The final full local run passed 191 unit/native tests and 3 doctests, including automatic sizing; formatting, strict Clippy and the static-CRT build passed. The staged portable package passed its manifest hashes, a three-image CLI listing, isolated native Yazi theme switching/persistence and UTF-8/UTF-16LE workspace/user-home path scans. Silent TESTMODE install/launch/Settings/theme/uninstall checks passed in an isolated directory. Video binaries were absent. Isolated native API checks supplement unit tests. Physical keyboard behavior and user-reported Settings visual feedback are not blanket-certified by these checks.

Installer/portable preparation includes Yazi, ES, Nerd Font and component notices. Optional video setup downloads checksummed libmpv/FFmpeg directly from upstream on the user's machine; those binaries are not bundled.

Outstanding: independent testing on a second unprepared Windows 11 x64 machine, including installation, missing VC++ runtime, video download failure/retry and uninstall. Remote CI results are available in [GitHub Actions](https://github.com/LilyAvelis/Afiocira/actions). The installer is unsigned.
