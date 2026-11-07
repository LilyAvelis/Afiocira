# Afiocira

Created by **Lilia Avelis and GPT-6.1 Sol**.

**A photo gallery with a file manager at its side.** Open a folder, see its pictures together, and move between the gallery and your files without losing your place.

Afiocira began with a simple frustration: viewing a large collection on Windows should not mean opening photos one by one in File Explorer or importing everything into a separate library. Here, any folder can become a gallery. Open a picture to look closer, then return to the same place in the grid.

The left panel is [Yazi](https://yazi-rs.github.io/), a real file manager embedded in the window. Browse folders, sort, select, rename or move files with Yazi; press **Tab** to take the current picture and selection into the gallery. Press Tab again to return to those files. You can also use the gallery on its own.

## Try it on Windows 11

The Windows installer is being tested privately before the first public Release. There is no public download yet. Once the independent installation test is complete, the installer will appear under [Releases](../../releases). It will include Yazi; you will not need Rust, Zig or a separate Yazi setup.

After installing, open Afiocira and choose a folder of images, or use **Open with → Afiocira** on a picture. The installer does not replace your default photo app. Afiocira works with ordinary local folders; iCloud Photos is one possible source, not a requirement.

## A few things to try

| What you want | How |
| --- | --- |
| Open a picture from the grid | Click it, or select it with arrows and press **Space** or **Enter** |
| Return to the grid | **Space** or **Esc** |
| Look closer at a detail | Scroll over it; **Ctrl + scroll** zooms around the center; drag to pan; **Fit** resets |
| Move between Yazi and the gallery | **Tab**; the current file and marked pictures follow you |
| Mark several pictures | **Shift + arrows** or **Shift + click** in the gallery; **Esc** clears gallery marks |
| Change the gallery's order | Sort in Yazi; the gallery follows its order |

Yazi also supports keyboard and mouse navigation and your existing Yazi configuration. Its file operations act on your real files, and Afiocira cannot undo them.

## What this version is

This is an early viewer, not yet an image editor. It reads PNG, JPEG, WebP, GIF, BMP and TIFF, including EXIF orientation. HEIC, HEIF and AVIF previews, animated playback, full color management and true original-pixel zoom are not implemented. Preview images are limited to 2560 pixels on their longest side. Image editing and local AI tools are ideas for later stages, not features of this release.

The app is written in Rust with egui. Its keyboard controls and local CLI/API use the same commands, so an agent can work with the open viewer. See [how it works](docs/architecture.md), [Yazi behavior](docs/yazi-navigation.md), [build and installation notes](docs/install-and-build.md), and the [public release checklist](docs/release-readiness.md) for technical details.

## Project

Afiocira's source code is [MIT licensed](LICENSE). Third-party components keep their own licenses; [Rust dependency notices](licenses/THIRD_PARTY_RUST.html) and other packaged notices are available in `licenses/`.

Lilia conceived the product, directed its interaction design and tested it in daily use. GPT-6.1 Sol designed and wrote the Rust viewer, its GUI, local API and Yazi integration in collaboration with her. Other AI models contributed independent reviews and release preparation. This is a human–AI collaboration, with the implementation contribution named directly.

The project grew from Lilia's [photo workshop in the Golden Garden](https://app.notion.com/p/41149fda329681e0b7fcfc89dcba8db8). The Garden holds the product story and next ideas; this repository holds the code and its current technical behavior.
