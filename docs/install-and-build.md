# Windows installation and build notes

Afiocira currently targets Windows 11 x64 with a working OpenGL driver. The installer is unsigned and may trigger SmartScreen. It installs for the current user, bundles Yazi 26.5.6 and the Microsoft Visual C++ Redistributable, creates shortcuts, and registers **Open with** for decoded image formats. It does not change default file associations. Its uninstaller removes its own shortcuts and registration.

The 2.5.0 installer and portable ZIP are available from [v2.5.0](https://github.com/LilyAvelis/Afiocira/releases/tag/v2.5.0), following the public 2.0 release. See [the release record](release-2.5.md) for validation details and [Actions](https://github.com/LilyAvelis/Afiocira/actions) for remote CI results. Independent installation on another unprepared Windows 11 machine remains unverified. The older `scripts/install-windows.ps1` and `scripts/unregister-windows.ps1` are development helpers for previous builds and use the `photo-gui.exe` registration.

Video components are fetched directly from upstream by the installer. Portable users run Enable-Video.cmd once before starting Afio; this step requires Internet access.

## Building

Developers need PowerShell 7 and Windows MSVC build tools. The project pins Rust in rust-toolchain.toml. Run `scripts/setup-ci-tools.ps1 -Installer` for the pinned Rust/Zig tools and a compatible Inno Setup 6 compiler, then `scripts/build-release.ps1` for a verified static-CRT, source-path-remapped candidate. Dependency downloads are pinned and SHA256-checked. Ordinary CI commits run checks; stable version tags build and publish verified packages after those checks pass. Manual package runs produce candidates without publication. See [CI/CD](ci-cd.md) for caches, retries and the release procedure. End users do not need build tools.

Inno Setup's current compiler is for non-commercial use; commercial installer distribution requires revisiting its license or choosing another installer technology.

## Local API

Build `photo-cli` alongside the GUI. `photo-cli list <folder>` lists supported image names. `photo-cli ctl status` and other `ctl` commands control the first running GUI through a local Windows named pipe. `ctl-json` accepts one JSON command on stdin. Only one GUI can own the pipe. Requests are limited to 16 MiB and eight simultaneous clients. The API has no application-level authentication. Request reads and response writes have 3-second I/O deadlines, GUI replies have a 15-second deadline, and the bounded dispatch queue admits at most eight pending clients. See [architecture](architecture.md) for the command contract.

## Dependency notices

The [Afiocira Rust dependency report](../licenses/THIRD_PARTY_RUST.html) was generated from Cargo.lock with `cargo about generate --workspace --locked --fail about.hbs --output-file licenses/THIRD_PARTY_RUST.html`. The [Yazi dependency report](../licenses/THIRD_PARTY_YAZI.html) was generated from the tagged Yazi 26.5.6 Cargo.lock using `about-yazi.hbs`; Yazi's own license is packaged. Lucide and egui-lucide notices are in `licenses/`.
