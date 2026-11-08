"""Native API verification for shortcuts, scale, transforms and persistence."""
import json
import os
from pathlib import Path
import subprocess
import shutil
import time
import uuid

project = Path(__file__).resolve().parent.parent
root = project / "target" / ("afio-settings-" + uuid.uuid4().hex[:8])
media = root / "media"
media.mkdir(parents=True)
source = media / "source.png"
source.write_bytes((project / "assets/test-images/1.png").read_bytes())
original = source.read_bytes()
video = root / "video" / "synthetic.mp4"
video.parent.mkdir()
# A tiny generated clip keeps video context verification independent of user media.
subprocess.run([shutil.which("ffmpeg") or "ffmpeg", "-hide_banner", "-loglevel", "error",
                "-f", "lavfi", "-i", "color=c=black:s=16x16:r=1", "-frames:v", "1",
                "-c:v", "mpeg4", str(video)], check=True, creationflags=subprocess.CREATE_NO_WINDOW)
transforms = {"rotate_left", "rotate_right", "flip_horizontal", "flip_vertical"}
env = dict(os.environ, AFIO_API_NAME="afio-prefs-" + uuid.uuid4().hex, AFIO_SETTINGS_HOME=str(root / "config"))
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


def wait(predicate, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            result = rpc()
            if predicate(result):
                return result
        except OSError:
            pass
        time.sleep(.05)
    raise AssertionError("Timed out waiting for native state")


def launch():
    process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    wait(lambda r: r["snapshot"] is not None)
    rpc("open", path=str(media))
    return process


def command_map():
    return {action["id"]: action for action in rpc("list_commands")["commands"]}


def settings_from_palette(view, session):
    opened = rpc("show_commands")
    assert opened["palette"]["open"]
    assert opened["snapshot"]["view"] == view
    assert command_map()["settings"]["shortcut"] == "Ctrl ,"
    settings = rpc("run_action", id="settings")
    assert settings["settings"]["open"]
    assert not settings["palette"]["open"]
    assert settings["session_id"] == session
    assert settings["snapshot"]["view"] == view
    # ShowSettings includes the detailed fixed-shortcut catalogue.
    detailed = rpc("show_settings")
    entries = [entry for entry in detailed["settings"]["shortcuts"] if entry["action"] == "settings"]
    assert {entry["context"] for entry in entries} == {"gallery", "albums", "space"}
    assert all(not entry["rebindable"] and entry["binding"] == "Ctrl ," for entry in entries)
    rpc("close_settings")


process = launch()
try:
    wait(lambda r: r["render"]["thumbnail_ready"])
    before = rpc()
    assert before["settings"]["gallery_auto_size"] is True
    assert not transforms.intersection(command_map())
    settings_from_palette("Grid", before["session_id"])
    for context in ["gallery", "albums", "space"]:
        for change in [dict(binding="Ctrl J"), dict(binding=None), dict(reset=True), dict(binding="Ctrl J", replace=True)]:
            assert rpc("set_shortcut", action="settings", context=context, allow_error=True, **change)["error"]
    texture_ids = before["render"]["thumbnail_textures"]
    rpc("set_gallery_scale", scale=.65)
    time.sleep(.2)
    scaled = rpc()
    assert scaled["settings"]["gallery_scale"] == .65
    assert scaled["settings"]["gallery_auto_size"] is False
    assert scaled["render"]["thumbnail_textures"] == texture_ids
    automatic = rpc("set_gallery_auto_size", enabled=True)
    assert automatic["settings"]["gallery_auto_size"] is True
    assert automatic["settings"]["gallery_scale"] == .65
    rpc("resize_viewport", width=1100, height=720)
    time.sleep(.2)
    assert rpc()["render"]["thumbnail_textures"] == texture_ids
    manual = rpc("set_gallery_auto_size", enabled=False)
    assert manual["settings"]["gallery_auto_size"] is False
    assert manual["settings"]["gallery_scale"] == .65
    rpc("open_selected")
    assert transforms.issubset(command_map())
    settings_from_palette("Photo", before["session_id"])
    selected = rpc("select_settings", section="afio", page="Shortcuts")
    assert selected["settings"]["section"] == "afio"
    assert selected["settings"]["page"] == "Shortcuts"
    rpc("close_settings")
    rpc("open", path=str(video))
    assert rpc()["snapshot"]["view"] == "Photo"
    assert not transforms.intersection(command_map())
    assert rpc("run_action", id="rotate_left", allow_error=True)["error"]
    rpc("open", path=str(source))
    assert transforms.issubset(command_map())
    rpc("set_shortcut", action="rotate_left", context="space", binding="Ctrl J")
    actions = rpc("list_commands")["commands"]
    assert next(a for a in actions if a["id"] == "rotate_left")["shortcut"] == "Ctrl J"
    rpc("set_shortcut", action="rotate_left", context="space", binding=None)
    assert next(a for a in rpc("list_commands")["commands"] if a["id"] == "rotate_left")["shortcut"] == ""
    assert rpc("set_shortcut", action="rotate_left", context="space", binding="Ctrl K", allow_error=True)["error"]
    assert rpc("set_shortcut", action="rotate_left", context="space", binding="E", allow_error=True)["error"]
    rpc("set_shortcut", action="rotate_left", context="space", binding="E", replace=True)
    assert next(a for a in rpc("list_commands")["commands"] if a["id"] == "rotate_right")["shortcut"] == ""
    rpc("set_shortcut", action="rotate_left", context="space", binding="Ctrl J")
    from PIL import Image
    initial_pixels = Image.open(source).convert("RGBA").tobytes()
    initial_files = {p.name for p in media.iterdir()}
    for action in ["rotate_right"] * 4 + ["flip_horizontal"] * 2 + ["flip_vertical"] * 2:
        rpc("open", path=str(source))
        started = rpc("run_action", id=action)
        operation = started["operation"]["id"]
        result = wait(lambda r: r["operation"] and r["operation"]["id"] == operation and not r["operation"]["running"] and not r["render"].get("transform_animating", False))
        file = result["operation"]["files"][0]
        assert not file["error"], file
        assert Path(file["destination"]).is_file()
        assert result["snapshot"]["selected_path"] == file["destination"]
        assert result["snapshot"]["view"] == "Photo"
        assert os.path.samefile(file["destination"], source)
        assert {p.name for p in media.iterdir()} == initial_files
    assert Image.open(source).convert("RGBA").tobytes() == initial_pixels
    rpc("set_gallery_auto_size", enabled=True)
finally:
    process.terminate()
    process.wait(timeout=10)

process = launch()
try:
    rpc("open_selected")
    assert rpc()["settings"]["gallery_scale"] == .65
    assert rpc()["settings"]["gallery_auto_size"] is True
    restored = rpc("set_gallery_auto_size", enabled=False)
    assert restored["settings"]["gallery_scale"] == .65
    assert restored["settings"]["gallery_auto_size"] is False
    assert next(a for a in rpc("list_commands")["commands"] if a["id"] == "rotate_left")["shortcut"] == "Ctrl J"
finally:
    process.terminate()
    process.wait(timeout=10)

result = dict(shortcuts_optional_and_contextual=True, conflicts_require_explicit_replacement=True,
              settings_palette_gallery_and_space=True, settings_palette_closes_same_session=True,
              settings_binding_fixed_in_all_contexts=True, transform_palette_scope_verified=True,
              fixed_shortcuts_protected=True, scale_keeps_texture_ids=True, four_transforms_verified=True,
              same_path_cumulative_transforms=True, state_persisted_after_restart=True,
              auto_size_toggle_remembers_manual_size=True, auto_size_persisted_after_restart=True)
(root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf8")
print(json.dumps(result))
