"""Verify dynamic album capacity using generated fixtures and the native API only."""
import json
import os
from pathlib import Path
import struct
import subprocess
import time
import uuid
import zlib

project = Path(__file__).resolve().parent.parent
root = project / "target" / ("album-capacity-" + uuid.uuid4().hex[:8])
media = root / "media"
media.mkdir(parents=True)

def png(color):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 48, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress((b"\0" + bytes(color) * 64) * 48)) + chunk(b"IEND", b""))

for album in range(14):
    folder = media / f"{album:02}-album"
    folder.mkdir()
    for frame in range(60 if album < 3 else 25):
        (folder / f"frame-{frame:02}.png").write_bytes(png((40 + album * 10, 50 + frame * 3, 110)))
deep = media / "00-album" / "deeper"
deep.mkdir()
(deep / "never-requested.png").write_bytes(png((120, 80, 60)))
env = dict(os.environ, AFIO_API_NAME="afio-capacity-" + uuid.uuid4().hex,
           AFIO_SETTINGS_HOME=str(root / "config"))
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]

def rpc(command="status", **args):
    with open(pipe, "r+b", buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            part = stream.read(65536)
            if not part:
                raise RuntimeError("GUI disconnected")
            data += part
    result = json.loads(data)
    assert not result["error"], result["error"]
    return result

def wait(predicate, timeout=15):
    end = time.monotonic() + timeout
    latest = None
    while time.monotonic() < end:
        try:
            latest = rpc()
            if predicate(latest):
                return latest
        except OSError:
            pass
        time.sleep(.04)
    raise AssertionError(f"Timed out: {latest}")

def album_state(result, name="00-album"):
    return next(a for a in result["nested"]["albums"] if Path(a["folder"]).name == name)

def stable():
    previous = None
    since = time.monotonic()
    end = since + 15
    while time.monotonic() < end:
        state = rpc()
        counts = [(a["folder"], a["loaded"]) for a in state["nested"]["albums"]]
        if state["nested"]["loading"] or counts != previous:
            since = time.monotonic()
        elif time.monotonic() - since > .5:
            return state
        previous = counts
        time.sleep(.04)
    raise AssertionError("Album loading did not settle")

process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)], env=env,
                           creationflags=subprocess.CREATE_NO_WINDOW)
try:
    wait(lambda r: r["snapshot"] is not None)
    rpc("set_navigator_visible", visible=False)
    rpc("resize_viewport", width=680, height=520)
    rpc("set_gallery_scale", scale=2.5)
    rpc("open", path=str(media))
    wait(lambda r: r["nested"]["albums"] and album_state(r)["loaded"] >= 5)
    initial = stable()
    assert album_state(initial)["loaded"] == 5, initial["nested"]
    for _ in range(3):
        rpc("album_move", horizontal=1, vertical=0)
    cursor = rpc()["snapshot"]["cursor_path"]
    rpc("set_marked", paths=[cursor])
    before = rpc()
    positions = dict(before["nested"]["album_positions"])
    active = before["nested"]["active_album"]
    rpc("resize_viewport", width=1100, height=520)
    rpc("set_gallery_scale", scale=.4)
    wait(lambda r: album_state(r)["loaded"] > 5)
    medium = stable()
    rpc("resize_viewport", width=1800, height=520)
    wait(lambda r: album_state(r)["loaded"] > album_state(medium)["loaded"])
    wide = stable()
    assert album_state(wide)["loaded"] > 12, wide["nested"]
    assert album_state(wide)["loaded"] < 60, wide["nested"]
    assert wide["snapshot"]["cursor_path"] == cursor
    assert wide["snapshot"]["marked_paths"] == [cursor]
    assert wide["nested"]["active_album"] == active
    assert dict(wide["nested"]["album_positions"])[active] == positions[active]
    assert album_state(wide, "13-album")["loaded"] == 0
    assert all(a["loaded"] == 0 for a in wide["nested"]["albums"] if a["depth"] > 1)
    ready = wait(lambda r: sum(Path(path).parent.name == "00-album" for path, _ in r["render"]["thumbnail_textures"]) > 12)
    cached = sum(Path(path).parent.name == "00-album" for path, _ in ready["render"]["thumbnail_textures"])
    rpc("focus_pane", pane="navigator")
    restored = rpc("focus_pane", pane="gallery")
    assert restored["navigator"]["focus"] == "gallery"
    assert restored["nested"]["overview"], restored["nested"]
    assert restored["snapshot"]["cursor_path"] == cursor, restored["snapshot"]
    assert restored["snapshot"]["marked_paths"] == [cursor], restored["snapshot"]
    assert dict(restored["nested"]["album_positions"])[active] == positions[active]
    result = dict(initial_loaded=album_state(initial)["loaded"], medium_loaded=album_state(medium)["loaded"],
                  wide_loaded=album_state(wide)["loaded"], first_album_thumbnails=cached,
                  cursor_and_marks_retained=True, album_position_retained=True,
                  invisible_neighbor_unloaded=True, deeper_unloaded=True, bounded_continuation=True,
                  album_focus_roundtrip=True)
    (root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf8")
    print(json.dumps(dict(artifact=str(root / "result.json"), **result)))
finally:
    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                   creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
    process.wait(timeout=10)
