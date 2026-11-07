"""API smoke test against an explicitly isolated GUI; never drives desktop input.

Usage: python scripts/check-actions.py --cli PATH --pipe afio-actions-test
       --folder target/actions-fixture
Folder must contain synthetic 01-photo.png and 02-video.mp4, never personal media.
Creates copies inside that folder and leaves them for inspection.
"""
import argparse
import hashlib
import json
import os
import subprocess
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--cli", required=True)
parser.add_argument("--pipe", required=True)
parser.add_argument("--folder", required=True)
args = parser.parse_args()
env = {**os.environ, "AFIO_API_NAME": args.pipe}
folder = Path(args.folder).resolve()
photo, video = folder / "01-photo.png", folder / "02-video.mp4"


def ctl(*arguments):
    result = json.loads(subprocess.check_output([args.cli, "ctl", *map(str, arguments)], env=env))
    assert result["error"] is None, result
    return result


def wait(predicate):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        result = ctl("status")
        if predicate(result):
            return result
        time.sleep(0.05)
    raise AssertionError(result)


def finish(result):
    job_id = result["operation"]["id"]
    return wait(lambda r: r["operation"]["id"] == job_id and not r["operation"]["running"])


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


ctl("open", folder)
gallery = ctl("actions")
assert gallery["palette"]["mode"] == "gallery" and gallery["palette"]["open"]
assert {a["id"] for a in gallery["commands"]} >= {"copy", "duplicate", "open", "refresh"}
assert "back" not in {a["id"] for a in gallery["commands"]}
assert [a["id"] for a in ctl("search-actions", "duplicate")["commands"]] == ["duplicate"]
ctl("close-actions")
ctl("select-path", photo)
ctl("mark", photo, video)
batch = finish(ctl("duplicate"))
assert len(batch["operation"]["files"]) == 2
for item in batch["operation"]["files"]:
    assert item["error"] is None, item
    assert digest(item["source"]) == digest(item["destination"])
created = {f["destination"] for f in batch["operation"]["files"]}
assert set(batch["snapshot"]["marked_paths"]) == created

ctl("select-path", photo)
ctl("open-selected")
space = ctl("actions")
assert space["palette"]["mode"] == "space"
assert {a["id"] for a in space["commands"]} >= {"back", "fit", "copy", "duplicate"}
assert "clear_marks" not in {a["id"] for a in space["commands"]}
ctl("close-actions")
single = finish(ctl("duplicate"))
assert len(single["operation"]["files"]) == 1
assert single["snapshot"]["selected_path"] == single["operation"]["files"][0]["destination"]
assert single["snapshot"]["view"] == "Photo"

# Hold a writer on one source to deterministically exercise a partial failure.
import ctypes
from ctypes import wintypes
kernel = ctypes.WinDLL("kernel32", use_last_error=True)
kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                              wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p]
kernel.CreateFileW.restype = wintypes.HANDLE
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
ctl("back")
ctl("mark", photo, video)
handle = kernel.CreateFileW(str(video), 0x40000000, 7, None, 3, 0, None)
assert handle != ctypes.c_void_p(-1).value
try:
    partial = finish(ctl("duplicate"))
finally:
    kernel.CloseHandle(handle)
assert sum(item["error"] is not None for item in partial["operation"]["files"]) == 1
assert any(path.endswith("02-video.mp4") for path in partial["snapshot"]["marked_paths"])

ctl("select-path", video)
ctl("open-selected")
wait(lambda r: r["video"]["duration"] > 0)
commands = ctl("actions")["commands"]
assert "play_pause" in {a["id"] for a in commands}
assert "fit" not in {a["id"] for a in commands}
ctl("close-actions")
ctl("action", "play_pause")
wait(lambda r: r["video"]["paused"])
ctl("back")
ctl("mark")
print("PASS: contextual palettes, search, batch/Space duplicates, byte identity, selection transfer, partial failure, video actions")
