"""Exercise Yazi settings against a real ConPTY, using private config/media only."""
import json
import os
from pathlib import Path
import subprocess
import time
import uuid
from native_fixture import stop_fixture

project = Path(__file__).resolve().parent.parent
root = project / "target" / ("yazi-settings-" + uuid.uuid4().hex[:8])
media = root / "media"
base = root / "base"
destination = root / "g l $ Фото"
for folder in [media, base, destination]:
    folder.mkdir(parents=True)
for i in range(3):
    (media / f"{i}.png").write_bytes((project / "assets/test-images/1.png").read_bytes())
(destination / "image.png").write_bytes((project / "assets/test-images/2.png").read_bytes())
(base / "yazi.toml").write_text('# user comment\n[mgr]\nsort_by="natural"\nshow_hidden=false\n[custom]\nkeep="yes"\n', encoding="utf8")
(base / "keymap.toml").write_text('[mgr]\nprepend_keymap=[{on=["g","l"],run="cd /",desc="existing gl"}]\n# keep me\n', encoding="utf8")
(base / "theme.toml").write_text('# individual override\n[mgr]\ncwd={fg="cyan"}\n', encoding="utf8")
originals = {p.name: p.read_bytes() for p in base.iterdir()}
for name, theme in [("valid", '[mgr]\nborder_symbol="!"\n'), ("invalid", '[mgr]\ncwd=42\n')]:
    flavor = base / "flavors" / (name + ".yazi")
    flavor.mkdir(parents=True)
    (flavor / "flavor.toml").write_text(theme, encoding="utf8")
    (flavor / "tmtheme.xml").write_text('<?xml version="1.0"?><plist version="1.0"><dict><key>name</key><string>Test</string><key>settings</key><array/></dict></plist>', encoding="utf8")
    (flavor / "LICENSE").write_text("MIT License\nSynthetic fixture", encoding="utf8")
    (flavor / "LICENSE-tmtheme").write_text("MIT License\nSynthetic fixture", encoding="utf8")

env = dict(os.environ, AFIO_API_NAME="afio-yazi-settings-" + uuid.uuid4().hex,
           AFIO_SETTINGS_HOME=str(root / "settings"), YAZI_CONFIG_HOME=str(base))
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]


def rpc(command="status", allow_error=False, **args):
    with open(pipe, "r+b", buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            part = stream.read(65536)
            if not part:
                raise RuntimeError("GUI disconnected")
            data += part
    result = json.loads(data)
    if not allow_error:
        assert not result["error"], result["error"]
    return result


def wait(predicate, timeout=18, command="status"):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = rpc(command)
            if predicate(last):
                return last
        except OSError:
            pass
        time.sleep(.08)
    raise AssertionError(last)


def cfg(op, **args):
    return rpc("configure_yazi", request=dict(op=op, **args))


def apply():
    cfg("apply")
    result = wait(lambda r: not r["settings"]["yazi"]["busy"])
    assert not result["settings"]["yazi"]["error"], result["settings"]["yazi"]["error"]
    return result


def norm(path):
    return os.path.normcase(os.path.normpath(str(path).replace("\\\\?\\", "")))


process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)],
                           env=env, creationflags=subprocess.CREATE_NO_WINDOW)
try:
    wait(lambda r: r["snapshot"] is not None)
    rpc("open", path=str(media))
    time.sleep(.5)
    rpc("set_marked", paths=[str(media / "0.png"), str(media / "1.png")])
    rpc("open_selected")
    before = rpc()
    inspected = cfg("inspect")["settings"]["yazi"]
    assert inspected["version"].startswith("Yazi 26.5.6 "), inspected
    link = dict(id="test-gl", name="Photos", key="l", path=str(destination), enabled=True)
    conflict = rpc("configure_yazi", allow_error=True, request=dict(op="save_quicklink", link=link, replace=False))
    assert conflict["error"] and "existing gl" in conflict["error"]["message"]
    binding = dict(id="test-binding", layer="mgr", on=["g", "l"],
                   run=["cd --raw '" + str(destination) + "'"],
                   description="Settings binding override", enabled=True)
    rejected = rpc("configure_yazi", allow_error=True,
                   request=dict(op="save_binding", binding=binding, replace=False))
    assert rejected["error"] and "existing gl" in rejected["error"]["message"]
    cfg("save_binding", binding=binding, replace=True)
    apply()
    effective = cfg("inspect")["settings"]["yazi"]
    assert any(b["on"] == binding["on"] and b["run"] == binding["run"]
               and b["description"] == binding["description"] for b in effective["bindings"])
    cfg("delete_binding", id="test-binding")
    apply()
    reset = cfg("inspect")["settings"]["yazi"]
    assert not reset["profile"]["bindings"]
    assert any(b["on"] == ["g", "l"] and b["description"] == "existing gl"
               for b in reset["bindings"])
    cfg("save_quicklink", link=link, replace=True)
    result = apply()
    assert result["session_id"] == before["session_id"]
    assert result["snapshot"] == before["snapshot"], (before, result)
    live_config = Path(cfg("inspect")["settings"]["yazi"]["config_directory"])
    assert "cd --raw" in (live_config / "keymap.toml").read_text(encoding="utf8")
    cfg("test_quicklink", id="test-gl")
    wait(lambda r: norm(r["navigator"]["folder"]) == norm(destination), command="navigator_snapshot")
    cfg("set_general", key="show_hidden", value="true")
    apply()
    assert cfg("inspect")["settings"]["yazi"]["effective_general"]["show_hidden"] == "true"
    cfg("delete_quicklink", id="test-gl")
    apply()
    assert cfg("inspect")["settings"]["yazi"]["profile"]["quicklinks"][0]["deleted"]
    cfg("restore_quicklink", id="test-gl")
    apply()
    previous = cfg("inspect")["settings"]["yazi"]["config_directory"]
    applied_profile = cfg("inspect")["settings"]["yazi"]["profile"]
    applied_config = Path(previous) / "yazi.toml"
    applied_bytes = applied_config.read_bytes()
    assert __import__("tomllib").loads(applied_bytes.decode("utf8"))["mgr"]["show_hidden"] is True
    cfg("set_general", key="show_hidden", value="false")
    draft = cfg("inspect")["settings"]["yazi"]["profile"]
    blocked = rpc("configure_yazi", allow_error=True,
                  request=dict(op="apply_appearance", dark="valid", light="valid", icons="minimal"))
    assert blocked["error"], blocked
    unchanged = cfg("inspect")["settings"]["yazi"]
    assert unchanged["profile"] == draft, unchanged
    # The Settings effective view previews the draft, so verify the live
    # configuration independently to prove rejection did not apply it.
    assert unchanged["effective_general"]["show_hidden"] == "false"
    assert applied_config.read_bytes() == applied_bytes
    assert unchanged["config_directory"] == previous and not unchanged["busy"]
    cfg("discard")
    assert cfg("inspect")["settings"]["yazi"]["profile"] == applied_profile
    started = time.monotonic()
    cfg("apply_appearance", dark="valid", light="valid", icons="minimal")
    assert time.monotonic() - started < 2.0, "Theme request blocked the native API"
    started = time.monotonic()
    rpc("status")
    assert time.monotonic() - started < 2.0, "Status blocked during theme application"
    hot = wait(lambda r: not r["settings"]["yazi"]["busy"])
    assert not hot["settings"]["yazi"]["error"], hot["settings"]["yazi"]["error"]
    assert cfg("inspect")["settings"]["yazi"]["config_directory"] == previous
    assert "app:theme" not in hot["settings"]["yazi"]["message"]  # completed, not still applying
    cfg("apply_appearance", dark="invalid", light="invalid", icons="minimal")
    failed = wait(lambda r: not r["settings"]["yazi"]["busy"])
    assert failed["settings"]["yazi"]["error"]
    assert failed["session_id"] == before["session_id"]
    assert cfg("inspect")["settings"]["yazi"]["config_directory"] == previous
    cfg("discard")
    if "--network" in __import__("sys").argv:
        cfg("install_flavor", source="yazi-rs/flavors:catppuccin-mocha.yazi")
        downloaded = wait(lambda r: not r["settings"]["yazi"]["busy"], timeout=90)
        assert not downloaded["settings"]["yazi"]["error"], downloaded["settings"]["yazi"]["error"]
        assert any(f["name"] == "catppuccin-mocha" and f["revision"] != "unknown" for f in cfg("inspect")["settings"]["yazi"]["flavors"])
    expected_profile = cfg("inspect")["settings"]["yazi"]["profile"]
    stop_fixture(process)
    process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)],
                               env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    wait(lambda r: r["snapshot"] is not None)
    restarted = cfg("inspect")["settings"]["yazi"]
    assert restarted["profile"] == expected_profile, (expected_profile, restarted["profile"])
    assert restarted["effective_general"]["show_hidden"] == "true"
    cfg("test_quicklink", id="test-gl")
    wait(lambda r: norm(r["navigator"]["folder"]) == norm(destination), command="navigator_snapshot")
    for filename, contents in originals.items():
        assert (base / filename).read_bytes() == contents, filename
    result = dict(real_gl_keys=True, dollar_unicode_space_path=True, conflict_detected=True,
                  replacement_verified=True, space_marks_session_preserved=True,
                  appearance_card_endpoint=True, pending_general_draft_preserved=True, api_responsive_during_theme=True,
                  keybinding_override_reset=True, persisted_profile_after_restart=True, general_applied=True, soft_delete_restore=True, original_config_untouched=True, hot_theme_same_yazi=True, invalid_theme_rolled_back=True, network_install="--network" in __import__("sys").argv)
    (root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf8")
    print(json.dumps(result))
finally:
    stop_fixture(process)
