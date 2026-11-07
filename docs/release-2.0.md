# Afiocira2.0 release packaging

The owner approved version2.0 as a complete release of the implemented scope. Published from the existing public LilyAvelis/Afiocira repository, preserving its 1.0 history. The private development history is not merged.

## Reproduce

1. Use Rust1.95/MSVC and Zig0.15.2. Run cargo fmt --all -- --check, strict Clippy, and cargo test --locked --workspace. Native Yazi tests need Yazi/ya on PATH and the Nerd Font installed or beside the test executable under fonts/.
2. Build with static CRT and remapped paths: CARGO_ENCODED_RUSTFLAGS contains -C,target-feature=+crt-static,--remap-path-prefix=YOUR_CHECKOUT=AFIOCIRA_SOURCE separated by ASCII31. Run cargo build --locked --release --workspace --target-dir target/distribution.
3. Run scripts/stage-windows-release.ps1. It verifies the locked Yazi/VC++/font downloads, and the ES setup archives. libmpv and FFmpeg binaries are not redistributed in these assets: enable-video.ps1 downloads the pinned upstream distributions on the end user machine and verifies checksums before installing them. Dependencies are from their upstream distributions. All fonts, runtime notices, Rust license report and source provenance ship in the package.
4. Compile scripts/Afiocira.iss with Inno Setup6.7.3. Zip the complete target/package-2.0.0 directory. Generate SHA256SUMS.txt for installer and portable ZIP. PACKAGE-MANIFEST.json records each staged file hash and the source commit.
5. Publish the selected source commit as v2.0.0 and attach installer, portable ZIP and checksums. GitHub also provides the tagged source snapshot.

The manual/PR Windows workflow builds and uploads an installer candidate; it does not publish automatically. Its first remote run is not claimed complete by this local release record.

## Installation

The per-user installer places immutable release files under Programs/Afiocira/releases/2.0.0 and publishes a launcher pointer only after copying them. The existing working-version launcher remains compatible. Open With entries list supported photo/video extensions; defaults are unchanged. VC++ Runtime is installed only if absent. The compiler output is unsigned.

Portable users extract the whole folder and run Afiocira.exe (or AfioLauncher.exe). Yazi/ya and fonts are discovered next to the executable; no global Yazi/font installation is required. Everything itself is optional and not bundled; only its ES helper is included. The installer offers video setup enabled by default; portable users run Enable-Video.cmd once with an Internet connection, before starting Afio. Failed downloads can be retried using that helper. libmpv and FFmpeg keep their own licenses.

## Local validation

129 unit/native tests and3 doctests, strict Clippy and formatting checks passed on the development machine. The static-CRT release was staged and every manifest hash verified. Afiocira/photo-cli/AfioLauncher were scanned for the development profile path in UTF8/UTF16 and passed that narrow check.

The package was launched with empty APPDATA/LOCALAPPDATA, PATH restricted to Windows System32, and a nonexistent test WINDIR font directory to exercise the bundled font. Native snapshots confirmed the Yazi process came from package/yazi;3 packaged PNGs opened; libmpv played and paused the synthetic H264/AAC video. The full album-folder/file handoff scenario passed against that packaged GUI.

The installer was compiled and silently installed into an isolated workspace directory with TESTMODE=1: this gives a separate uninstall AppId and skips real shortcuts/Open With entries. Its launcher opened the installed3-image fixture and the installed CLI listed all3 files. This validates package layout/launcher, not an independent second Windows machine or actual missing-runtime installation. Test-mode uninstall was checked separately. Normal user configuration, default associations and Notion were not changed by release preparation.

Independent second-machine testing and a confirmed remote CI run remain outstanding. Local checks are not universal certification. The public assets contain no libmpv or FFmpeg binary; those components are fetched directly from upstream by the optional video setup.

Public-package recheck: the Windows PowerShell video helper downloaded FFmpeg from the publisher's official GitHub release, verified both archive and executable checksums, and extracted the pinned libmpv runtime. Public assets exclude those runtime binaries. Formatting, strict Clippy, 129 tests and 3 doctests passed again on the public source snapshot.
