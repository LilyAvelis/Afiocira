# Afiocira 2.0

Created by **Lilia Avelis and GPT-6.1 Sol**.

A Windows photo and video gallery with a real embedded Yazi file manager, fast Everything search and keyboard-first actions.

## Download and run

Download the **2.0.0 Windows x64 installer** or portable ZIP from [Releases](https://github.com/LilyAvelis/Afiocira/releases/tag/v2.0.0). Windows11 x64 with an OpenGL-capable driver is the tested platform. The installer is unsigned. It installs for the current user and adds Open With entries without changing your defaults.

The package carries Yazi26.5.6, the Everything ES helper, a Nerd Font and notices. The installer downloads the pinned libmpv and FFmpeg distributions directly from upstream for video support (Internet required). Portable users run **Enable-Video.cmd** once before starting Afio to enable video playback and thumbnails. Everything itself must be installed and running to enable indexed search; the gallery works without it. For the portable ZIP, extract the whole folder, install the included Microsoft VC++ runtime if needed, and launch **Afiocira.exe**. Keep its adjacent runtime files/folders together.

## What's in 2.0

- Mixed photo/video gallery and Space viewer: libmpv playback, audio, pause, seeking and volume; FFmpeg video thumbnails.
- Raycast-inspired Actions menus for Gallery and Space, with vector icons and keyboard-operated forms.
- Ctrl+click independent selection, Shift range selection and selection synchronization with Yazi.
- Ctrl+C copies one or many files, with pixel clipboard data for a single supported image; Ctrl+V imports files or image pixels into the current folder.
- Ctrl+D duplicates; F2 renames one file; Delete uses the Windows Recycle Bin, with Enter confirmation for groups.
- Drag a folder or media files into Afio to open them.
- Ctrl+Shift+K searches Everything and opens files/folders directly in Afio/Yazi.
- Named album rows for nested folders, independent horizontal cursors and bounded lazy loading.
- Space actions resize/compress to a new copy or convert PNG/JPEG/WebP with checked replacement.
- Cache-preserving rename, deletion and reordering; centered thumbnails and narrow-window Fit.

## Keyboard

| Context | Keys | Result |
| --- | --- | --- |
| Everywhere | Ctrl+K | Contextual Actions |
| Everywhere | Ctrl+Shift+K | Everything search |
| Gallery / Yazi | Tab | Switch pane and synchronize context |
| Ordinary Gallery | Arrows; Shift+arrows | Cursor; range selection |
| Album overview | Left/Right | Browse and continue this album |
| Album overview | Up/Down | Change album; restore its cursor |
| Album overview | Enter | Enter the album's local grid at the same file |
| Gallery | Space | Open current image/video |
| Space | Space / Esc | Return to Gallery |
| Space | Left/Right; P | Adjacent file; video pause/play |
| File actions | Ctrl+C/V/D, F2, Delete | Copy/paste/duplicate, rename, recycle |
| Forms | Tab / Shift+Tab; Space; Enter; Esc | Cycle controls; toggle; apply; cancel |

In an album overview, Tab highlights that **album folder** in its parent in Yazi. In an ordinary local grid or Space, Tab reveals the current **file**. Own Yazi settings/themes stay intact: Afio creates a private session configuration.

## Boundaries

Image previews support PNG/JPEG/WebP/GIF/BMP/TIFF with EXIF orientation. HEIC/HEIF/AVIF are recognized in listings but not decoded yet. No ICC-managed display, animated-image playback, full-resolution1:1 mode, batch F2, shortcut customization, general undo or drag-out is claimed.

Nested browsing counts the immediate level only; pages are bounded and deeper directories require exploration. Image conversion currently accepts static8-bit PNG/JPEG/WebP; unsupported animation/alpha/precision cases are rejected. Converting replaces the source after verification; use Ctrl+D first to keep a separate original. Metadata removal requires explicit permission. Resize creates a new unique copy. Recovery files may remain after an interrupted conversion.

The local API controls the first GUI owning the default Windows pipe; set AFIO_API_NAME for an isolated instance. It is not a network service or a multi-session discovery protocol.

## Build

Install Rust/MSVC build tools and Zig0.15.2, then:

```powershell
cargo build --locked --release --workspace
./scripts/setup-video.ps1 -OutputDirectory target/release
./scripts/setup-everything.ps1 -OutputDirectory target/release
./target/release/photo-gui.exe 'C:\Photos'
./target/release/photo-cli.exe ctl status
```

Release packaging: [docs/release-2.0.md](docs/release-2.0.md). Runtime dependencies are pinned and checksummed; see their notices in the package. Afio's code is MIT licensed; external components retain their own licenses.

For development, `scripts/update-working.ps1` updates the permanent Afiocira desktop shortcut via a launcher and immutable working-version directories. It does not overwrite an already open GUI.

## Technical notes

[Architecture](docs/architecture.md) · [Yazi](docs/yazi-navigation.md) · [Video](docs/video.md) · [Actions](docs/actions.md) · [Clipboard/rename/drop](docs/import-and-rename.md) · [Delete](docs/delete.md) · [Latency](docs/delete-performance.md) · [Everything](docs/everything.md) · [Albums and image actions](docs/nested-and-image-actions.md) · [Terminal colors](docs/terminal-colors.md).

All GUI, keyboard, CLI and native API commands share one dispatcher. The Windows release was checked with129 unit/native tests,3 doctests, strict Clippy and isolated native API scenarios. Those checks do not certify every codec, filesystem or machine.

## Project

Afiocira's source code is [MIT licensed](LICENSE). Third-party components keep their own licenses; [Afiocira's Rust notices](licenses/THIRD_PARTY_RUST.html) and [bundled Yazi dependency notices](licenses/THIRD_PARTY_YAZI.html) are available in `licenses/`.

Lilia conceived the product, directed its interaction design and tested it in daily use. GPT-6.1 Sol designed and wrote the Rust viewer, its GUI, local API and Yazi integration in collaboration with her. Other AI models contributed independent reviews and release preparation. This is a human–AI collaboration, with the implementation contribution named directly.

The project grew from Lilia's [photo workshop in the Golden Garden](https://app.notion.com/p/41149fda329681e0b7fcfc89dcba8db8). The Garden holds the product story and next ideas; this repository holds the code and its current technical behavior.
