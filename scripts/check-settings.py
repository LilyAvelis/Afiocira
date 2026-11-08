"""Isolated native Settings checks; only synthetic assets and a private config/pipe."""
import json
import os
from pathlib import Path
import subprocess
import time
import uuid
from native_fixture import stop_fixture

project = Path(__file__).resolve().parent.parent
root = project / "target" / ("settings-check-" + uuid.uuid4().hex[:8])
root.mkdir(parents=True)
media = root / "media"
media.mkdir()
for name in ["a.png", "b.png"]:
    (media / name).write_bytes((project / "assets/test-images/1.png").read_bytes())
(media / "album").mkdir()
(media / "album/c.png").write_bytes((project / "assets/test-images/2.png").read_bytes())
env = dict(os.environ, AFIO_API_NAME="afio-settings-" + uuid.uuid4().hex,
           AFIO_SETTINGS_HOME=str(root / "config"))
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]


def rpc(command, **args):
    with open(pipe, "r+b", buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            part = stream.read(65536)
            if not part:
                raise RuntimeError("API disconnected")
            data += part
    response = json.loads(data)
    assert not response["error"], response["error"]
    return response


def wait(predicate, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            response = rpc("status")
            if predicate(response):
                return response
        except OSError:
            pass
        time.sleep(.05)
    raise AssertionError("Timed out waiting for native API state")


def launch():
    process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)],
                               env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    wait(lambda r: r["snapshot"] is not None)
    return process


def preserved(response):
    return {key: response[key] for key in ["session_id", "snapshot", "navigator"]}


def roundtrip():
    # Compare stable user context after the preceding mark transfer is acked;
    # transport pending/request IDs are progress, not persistent window state.
    before = wait(lambda r: not r["navigator"].get("pending", False))
    opened = rpc("show_settings")
    assert opened["settings"]["open"]
    time.sleep(.25)  # Let the native secondary viewport render before closing it.
    # Repeated opening must reuse the same window and session.
    repeated = rpc("show_settings")
    assert preserved(repeated) == preserved(before), (preserved(before), preserved(repeated))
    rpc("close_settings")
    after = wait(lambda r: not r["settings"]["open"])
    assert preserved(after) == preserved(before), (before, after)


process = launch()
try:
    rpc("open", path=str(media))
    wait(lambda r: r["nested"]["overview"] and not r["nested"]["loading"])
    time.sleep(.5)
    roundtrip()
    rpc("enter_album")
    wait(lambda r: not r["nested"]["loading"])
    rpc("set_marked", paths=[str(media / "a.png")])
    roundtrip()
    rpc("open_selected")
    roundtrip()
    rpc("back")
    rpc("focus_pane", pane="navigator")
    time.sleep(.5)
    before = rpc("navigator_snapshot")
    roundtrip()
    after = rpc("navigator_snapshot")
    assert before["navigator"] == after["navigator"]
    rpc("set_yazi_text_size", size=18)
    assert rpc("status")["settings"]["yazi_text_size"] == 18
finally:
    stop_fixture(process)

process = launch()
try:
    assert rpc("status")["settings"]["yazi_text_size"] == 18
finally:
    stop_fixture(process)

result = dict(preserved_grid_space_albums_and_marks=True, preserved_yazi_context=True,
              reused_api_session=True, persisted_after_restart=True)
(root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf8")
print(json.dumps(result))
