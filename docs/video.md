# Embedded video checkpoint

libmpv handles decoding, audio, aspect ratio and playback. The existing glow
renderer draws into an offscreen OpenGL framebuffer and blits it into Space.
The render call sets `MPV_RENDER_PARAM_FLIP_Y=1` to match the window framebuffer
orientation; without it the direct blit displayed video upside down. This fix
was rebuilt and published behind the permanent shortcut after user feedback;
visual confirmation on the user's clips is pending.
Playback does not copy frames through CPU memory. `hwdec=auto-safe` requests
hardware decoding with fallback; acceleration is not guaranteed for every file.

## Windows x64 build

```powershell
# First configure Zig PATH as described in README.
cargo build --locked --release --workspace
./scripts/setup-video.ps1 -OutputDirectory ./target/release
./target/release/photo-gui.exe 'C:\path\to\media'
```

Setup pins the tested shinchiro Windows x64 DLL and checks archive SHA-256.
`AFIO_MPV_DLL` explicitly overrides it; otherwise only `libmpv-2.dll` beside the
GUI is loaded. Missing DLL shows a video error; photo viewing remains usable.
The2.0 private release includes the pinned DLL, its license/source notices and
FFmpeg. See release-2.0.md for staging and package checks. The legacy development
install-windows helper copies an adjacent DLL.

Movies share gallery order, cursor and marks with photos. Recognized extensions:
MP4, MKV, WebM, MOV, AVI, M4V, WMV, MPEG/MPG, TS, MTS, M2TS. Actual codec support
belongs to libmpv; corrupt/unsupported files report an error. Optional FFmpeg
generates grid thumbnails (`AFIO_FFMPEG`, adjacent ffmpeg.exe, then PATH), using
the bounded decode worker, one FFmpeg thread and an eight-second timeout.
Without FFmpeg the Video tile remains openable. No persistent preview files.

Enter/Space opens; Space/Esc returns to the gallery and stops playback.
Left/Right navigates mixed media. P toggles pause when Gallery owns keyboard
focus. Buttons provide pause, -5/+5 s, absolute position slider and volume.
A newly opened movie starts from the beginning. Switching to a photo, changing
folder or losing the active file stops the previous movie. Photo zoom/Fit does
not apply to video. No editing, subtitles UI, persistent playback history, HDR
accuracy or universal codec support is claimed.

## API

```powershell
./target/release/photo-cli.exe ctl video-pause
./target/release/photo-cli.exe ctl video-play
./target/release/photo-cli.exe ctl video-seek 8
./target/release/photo-cli.exe ctl video-volume 25
./target/release/photo-cli.exe ctl status
```

Additive v3 JSON commands: `video_pause` (`paused`), `video_seek` (`seconds`,
absolute, finite/nonnegative), `video_volume` (`volume`, finite, 0–100).
GUI/CLI use the same dispatcher. Commands are asynchronous: success acknowledges
submission; poll status for observed position/pause/duration/volume or error.
The optional `video` object has null `path` when stopped; the whole object is
null when the engine failed to initialize.

Set `AFIO_API_NAME` identically for GUI and CLI to isolate a test instance from
another running viewer. Default pipe stays unchanged; this is an explicit
override, not automatic multi-window discovery.

## Verification — 2026-11-07

Generated 640×360 H.264/AAC MP4 (moving test pattern and tone) and PNG. Live API
verified advancing playback, stable pause, exact seek to 8 s, volume, movie →
PNG → movie, stop on grid return, and readable error for corrupt MP4. User
confirmed visible playback; grid thumbnails were also observed. Audio output
was not independently measured. Long-video/HDR/4K performance is unmeasured.

21 workspace tests passed with an empty `YAZI_CONFIG_HOME`, including mixed
ordering/marks/navigation/external removal and native ConPTY. The user's config
initially caused an existing native Yazi sort/mouse test to fail; clean-config
run passed. Strict workspace Clippy passed. These checks do not certify all
containers/codecs. The public readiness branch is not merged into this branch.

ABI reference: https://github.com/mpv-player/mpv/tree/master/include/mpv
