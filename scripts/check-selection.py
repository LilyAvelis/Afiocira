"""Verify Ctrl-style toggles and live Yazi round-trip using an isolated GUI API.
Usage: python scripts/check-selection.py --cli PATH --pipe afio-selection-test
       --folder target/selection-fixture
The fixture contains four synthetic images, 01-photo.png .. 04-photo.png.
No desktop input or system clipboard access is performed.
"""
import argparse
import json
import os
import subprocess
import time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("--cli", required=True)
p.add_argument("--pipe", required=True)
p.add_argument("--folder", required=True)
args = p.parse_args()
environment = {**os.environ, "AFIO_API_NAME": args.pipe}


def call(*arguments, allow_error=False):
    process = subprocess.run([args.cli, "ctl", *map(str, arguments)], env=environment, stdout=subprocess.PIPE)
    result = json.loads(process.stdout)
    assert allow_error or result["error"] is None, result
    return result


def marked(result):
    return {Path(path).name for path in result["snapshot"]["marked_paths"]}


call("open", Path(args.folder).resolve())
call("mark")
# Allow Yazi's independent process to initialize its DDS subscriber.
deadline = time.monotonic() + 5
while True:
    result = call("focus-gallery", allow_error=True)
    if result["error"] is None:
        break
    assert time.monotonic() < deadline, result
    time.sleep(0.1)

for index in [0, 2, 3]:
    result = call("toggle-selection", index)
assert marked(result) == {"01-photo.png", "03-photo.png", "04-photo.png"}
assert result["snapshot"]["view"] == "Grid"
result = call("toggle-selection", 2)
assert marked(result) == {"01-photo.png", "04-photo.png"}
assert result["snapshot"]["grid_anchor"] == 2
time.sleep(0.3)
# Import Yazi's actual snapshot: losing a mark here would expose a failed push.
result = call("focus-gallery")
assert marked(result) == {"01-photo.png", "04-photo.png"}, result

# Restore Ctrl anchor after focus-gallery imports Yazi's cursor.
call("toggle-selection", 2)
result = call("extend-selection", 1)
assert marked(result) == {"01-photo.png", "02-photo.png", "03-photo.png", "04-photo.png"}
result = call("extend-selection", 3)
assert marked(result) == {"01-photo.png", "03-photo.png", "04-photo.png"}
time.sleep(0.3)
result = call("focus-gallery")
assert marked(result) == {"01-photo.png", "03-photo.png", "04-photo.png"}
call("mark")
time.sleep(0.3)
assert not marked(call("focus-gallery"))
call("open-selected")
assert call("toggle-selection", 0, allow_error=True)["error"] is not None
call("back")
print("PASS: accumulating toggles, single removal, no photo opening, Shift range/reversal, automatic Yazi round-trip and clear")
