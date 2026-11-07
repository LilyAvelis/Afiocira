# Windows installation and build notes

Afiocira currently targets Windows 11 x64 with a working OpenGL driver. The installer is unsigned and may trigger SmartScreen. It installs for the current user, bundles Yazi 26.5.6 and the Microsoft Visual C++ Redistributable, creates shortcuts, and registers **Open with** for decoded image formats. It does not change default file associations. Its uninstaller removes its own shortcuts and registration.

The public installer is pending an independent Windows 11 test, license review of Yazi's bundled dependencies, and a successful GitHub Actions run. The older `scripts/install-windows.ps1` and `scripts/unregister-windows.ps1` are development helpers for previous builds and use the `photo-gui.exe` registration.

## Building

Developers need Rust, Windows MSVC build tools, Zig 0.15.x and Inno Setup 6. Build with `cargo build --locked --release --workspace --target-dir target/remapped`, enabling static CRT and source-path remapping as shown in [the Windows workflow](../.github/workflows/windows-package.yml). Then run `scripts/stage-windows-release.ps1` and compile `scripts/Afiocira.iss` with `ISCC.exe`. The staging script downloads pinned Yazi and Visual C++ packages, verifies SHA-256 and includes synthetic test images. The workflow uploads an installer candidate and its SHA-256; it does not publish a Release. End users do not need these build tools.

Inno Setup's current compiler is for non-commercial use; commercial installer distribution requires revisiting its license or choosing another installer technology.

## Local API

Build `photo-cli` alongside the GUI. `photo-cli list <folder>` lists supported image names. `photo-cli ctl status` and other `ctl` commands control the first running GUI through a local Windows named pipe. `ctl-json` accepts one JSON command on stdin. Only one GUI can own the pipe. Requests are limited to 16 MiB and eight simultaneous clients. The API has no application-level authentication or read timeout. See [architecture](architecture.md) for the command contract.

## Dependency notices

The [Rust dependency report](../licenses/THIRD_PARTY_RUST.html) was generated from Cargo.lock with `cargo about generate --workspace --locked --fail about.hbs --output-file licenses/THIRD_PARTY_RUST.html`. Lucide and egui-lucide notices are in `licenses/`; Yazi's own license is packaged. Yazi's transitive notices still require review before a public binary release.
