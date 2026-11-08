# Public release validation

Afiocira 3.0.0 targets tag `v3.0.0`. See the [3.0 verification record](release-3.0.md), [release notes](releases/v3.0.0.md) and [publication status](https://github.com/LilyAvelis/Afiocira/releases/tag/v3.0.0). The [2.5 record](release-2.5.md), [published v2.5.0](https://github.com/LilyAvelis/Afiocira/releases/tag/v2.5.0) and [2.0 record](release-2.0.md) remain historical evidence for their own versions.

The last reported local run passed 296 Rust unit/native tests and 3 doctests, formatting and strict Clippy, supplemented by isolated native synthetic scenarios for metadata, temporary history and live-preview editing. Ctrl+L received manual user validation. These results do not certify all physical shortcuts or image formats. Historical package results do not certify the 3.0 candidate.

The [Windows pipeline](ci-cd.md) checks main and PR commits. `workflow_dispatch` with `package=true` creates an unpublished verified candidate. A matching stable version tag runs check → package → publish. Require green results for the exact release commit and record actual package/publication evidence after it exists.

Installer/portable preparation includes Yazi, ES, Nerd Font and component notices. Optional video setup downloads checksummed libmpv/FFmpeg directly from upstream on the user's machine; those binaries are not bundled.

Outstanding: independent testing on a second unprepared Windows 11 x64 machine, including installation, missing VC++ runtime, video download failure/retry and uninstall. Remote CI results are available in [GitHub Actions](https://github.com/LilyAvelis/Afiocira/actions). The installer is unsigned.
