"""Verify native Yazi mode, persistence, draft safety, and private config ownership."""
import json
import os
from pathlib import Path
import subprocess
import time
import uuid
from PIL import Image
from native_fixture import stop_fixture

PROJECT = Path(__file__).resolve().parents[1]
ROOT = PROJECT / "target" / ("yazi-mode-" + uuid.uuid4().hex[:8])
BASE = ROOT / "base"
MEDIA = ROOT / "media"
for directory in [BASE, MEDIA]:
    directory.mkdir(parents=True)
(MEDIA / "a.png").write_bytes((PROJECT / "assets/test-images/1.png").read_bytes())
(MEDIA / "b.png").write_bytes((PROJECT / "assets/test-images/2.png").read_bytes())
(BASE / "theme.toml").write_text('[flavor]\ndark="fixture-dark"\nlight="fixture-light"\n', encoding="utf8")
for name, color, border in [("fixture-dark", "#123456", "D"), ("fixture-light", "#654321", "L")]:
    directory = BASE / "flavors" / (name + ".yazi")
    directory.mkdir(parents=True)
    (directory / "flavor.toml").write_text(f'[mgr]\ncwd={{fg="{color}"}}\nborder_symbol="{border}"\n', encoding="utf8")
    (directory / "tmtheme.xml").write_text('<?xml version="1.0"?><plist version="1.0"><dict><key>name</key><string>Fixture</string><key>settings</key><array/></dict></plist>', encoding="utf8")
    for license_name in ["LICENSE", "LICENSE-tmtheme"]:
        (directory / license_name).write_text("MIT License\nSynthetic test fixture", encoding="utf8")
originals = {str(path.relative_to(BASE)): path.read_bytes() for path in BASE.rglob("*") if path.is_file()}
env = dict(os.environ, AFIO_API_NAME="afio-mode-" + uuid.uuid4().hex,
           AFIO_SETTINGS_HOME=str(ROOT / "settings"), YAZI_CONFIG_HOME=str(BASE))
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]
binary = Path(os.environ.get("AFIO_TEST_GUI", PROJECT / "target/release/photo-gui.exe"))


def rpc(command="status", allow_error=False, **args):
    until = time.monotonic() + 2
    while True:
        try:
            stream = open(pipe, "r+b", buffering=0)
            break
        except OSError as error:
            if error.errno not in [2, 22] or time.monotonic() > until:
                raise
            time.sleep(.01)
    with stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            data += stream.read(65536)
    result = json.loads(data)
    if not allow_error:
        assert not result["error"], result["error"]
    return result


def wait(predicate, timeout=18):
    until = time.monotonic() + timeout
    last = None
    while time.monotonic() < until:
        last = rpc()
        if predicate(last):
            return last
        time.sleep(.03)
    raise AssertionError(last)


def native_proof():
    metadata = rpc("navigator_snapshot")["navigator"]["runtime"]["metadata"]
    return dict(light=metadata["light"], theme=metadata["theme"])


def capture(name):
    path = ROOT / (name + ".png")
    until = time.monotonic() + 6
    while time.monotonic() < until:
        path.unlink(missing_ok=True)
        rpc("capture_frame", path=str(path), viewport="root")
        while time.monotonic() < until and not path.is_file():
            rpc()
            time.sleep(.03)
        if path.is_file():
            with Image.open(path) as image:
                # Resize may clear the native grid for a frame before Yazi
                # redraws. Require actual terminal ink, not a blank pane.
                width, height = image.size
                area = image.crop((int(width*.02), int(height*.16), int(width*.28), int(height*.3))).convert("L")
                low, high = area.getextrema()
                if high - low > 32:
                    return str(path)
        time.sleep(.05)
    raise AssertionError("No root capture: " + str(path))


def apply_mode(mode):
    before = rpc()
    rpc("configure_yazi", request=dict(op="apply_theme_mode", mode=mode))
    done = wait(lambda r: not r["settings"]["yazi"]["busy"])
    assert not done["settings"]["yazi"]["error"], done["settings"]["yazi"]
    assert done["settings"]["yazi"]["profile"]["theme_mode"] == mode
    assert done["snapshot"] == before["snapshot"], "Mode switch changed Gallery context"
    assert done["session_id"] == before["session_id"], "Mode switch restarted Afio"
    proof = native_proof()
    assert proof["light"] == (mode == "light"), proof
    assert proof["theme"][3] == ("L" if mode == "light" else "D"), proof
    return proof


process = subprocess.Popen([str(binary), str(MEDIA)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
results = dict(root=str(ROOT))
try:
    wait(lambda r: r.get("snapshot") is not None)
    wait(lambda r: not r["navigator"]["pending"])
    dark = native_proof()
    assert dark["light"] is False and dark["theme"][3] == "D", dark
    rpc("focus_pane", pane="gallery")
    rpc("set_marked", paths=[str(MEDIA / "a.png"), str(MEDIA / "b.png")])
    wait(lambda r: not r["navigator"]["pending"])
    rpc("resize_viewport", width=1100, height=720, viewport="root")
    results["captures"] = [capture("01-dark")]
    light = apply_mode("light")
    results["captures"].append(capture("02-light"))
    returned_dark = apply_mode("dark")
    results["captures"].append(capture("03-returned-dark"))
    assert dark == returned_dark and dark != light
    # Refuse an unrelated General draft without changing the real active mode.
    rpc("configure_yazi", request=dict(op="set_general", key="scrolloff", value="8"))
    refused = rpc("configure_yazi", allow_error=True, request=dict(op="apply_theme_mode", mode="light"))
    assert refused["error"], refused
    rpc("configure_yazi", request=dict(op="discard"))
    apply_mode("light")
    stop_fixture(process)
    process = subprocess.Popen([str(binary), str(MEDIA)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    restarted = wait(lambda r: r.get("snapshot") is not None)
    persisted = native_proof()
    assert restarted["settings"]["yazi"]["profile"]["theme_mode"] == "light"
    assert persisted == light, persisted
    assert originals == {str(path.relative_to(BASE)): path.read_bytes() for path in BASE.rglob("*") if path.is_file()}
    results.update(dark=dark, light=light, returned_dark=returned_dark,
                   persisted_light=persisted, source_config_untouched=True,
                   context_preserved=True, unrelated_draft_refused=True)
finally:
    stop_fixture(process)
    (ROOT / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf8")
    print(json.dumps(results, ensure_ascii=False, indent=2))
