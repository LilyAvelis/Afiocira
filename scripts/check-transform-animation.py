"""Check four transform animations in an isolated native GUI using synthetic pixels.

Captures are requested through the existing root-frame API. Request/ack timings
bound API latency; they are not exact monitor presentation timestamps.
"""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import time
import uuid

from PIL import Image, ImageChops, ImageDraw

from native_fixture import stop_fixture


COLORS = {
    "red": (240, 32, 48),
    "green": (24, 220, 80),
    "blue": (32, 80, 240),
    "yellow": (240, 210, 32),
}
ACTIONS = [
    ("rotate_left", Image.Transpose.ROTATE_90),
    ("rotate_right", Image.Transpose.ROTATE_270),
    ("flip_horizontal", Image.Transpose.FLIP_LEFT_RIGHT),
    ("flip_vertical", Image.Transpose.FLIP_TOP_BOTTOM),
]


def color_geometry(path):
    """Measure only the synthetic image colors, excluding text and chrome."""
    with Image.open(path) as screenshot:
        image = screenshot.convert("RGB")
    result = {}
    for name, color in COLORS.items():
        difference = ImageChops.difference(image, Image.new("RGB", image.size, color))
        red, green, blue = difference.split()
        maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
        mask = maximum.point(lambda value: 255 if value <= 18 else 0)
        bounds = mask.getbbox()
        assert bounds is not None, f"No {name} image pixels in {path}"
        crop = mask.crop(bounds)
        width = crop.width
        count = sum_x = sum_y = 0
        for index, value in enumerate(crop.getdata()):
            if value:
                count += 1
                sum_x += index % width + bounds[0]
                sum_y += index // width + bounds[1]
        assert count >= 100, (path.name, name, count)
        result[name] = {"pixels": count, "centroid": [sum_x / count, sum_y / count],
                        "bounds": list(bounds)}
    return result


def direction(geometry):
    red = geometry["red"]["centroid"]
    green = geometry["green"]["centroid"]
    return math.degrees(math.atan2(green[1] - red[1], green[0] - red[0]))


def angle_delta(before, after):
    return (after - before + 180) % 360 - 180


def colored_span(geometry, axis):
    return (max(color["bounds"][axis + 2] for color in geometry.values())
            - min(color["bounds"][axis] for color in geometry.values()))


def normalized_centroid(geometry, color):
    lower = [min(item["bounds"][axis] for item in geometry.values()) for axis in (0, 1)]
    return [(geometry[color]["centroid"][axis] - lower[axis]) / colored_span(geometry, axis)
            for axis in (0, 1)]


def verify_geometry(action, captures):
    frames = {name: color_geometry(Path(capture["path"]))
              for name, capture in captures.items()}
    if action.startswith("rotate"):
        angles = {name: direction(geometry) for name, geometry in frames.items()}
        start = angle_delta(angles["before"], angles["start"])
        middle = angle_delta(angles["before"], angles["middle"])
        final = angle_delta(angles["before"], angles["end"])
        expected = -90 if action == "rotate_left" else 90
        assert abs(final - expected) < 2, (action, angles)
        assert 5 < abs(middle) < 85 and middle * expected > 0, (action, angles)
        assert abs(middle) > abs(start) + 3, (action, angles)
        evidence = {"image_angle_delta_degrees": {"start": start, "middle": middle, "end": final}}
    else:
        axis = 0 if action == "flip_horizontal" else 1
        spans = {name: colored_span(geometry, axis) for name, geometry in frames.items()}
        assert spans["middle"] < min(spans["before"], spans["end"]) * 0.9, (action, spans)
        assert spans["middle"] < spans["start"] * 0.95, (action, spans)
        for name in COLORS:
            before = normalized_centroid(frames["before"], name)
            after = normalized_centroid(frames["end"], name)
            assert abs(before[axis] + after[axis] - 1) < 0.01, (action, name, before, after)
            assert abs(before[1 - axis] - after[1 - axis]) < 0.01, (action, name, before, after)
        evidence = {"image_axis_span_pixels": spans}
    return {**evidence, "color_geometry": frames}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--out", type=Path, help="A new, empty artifact directory")
    args = parser.parse_args()
    project = Path(__file__).resolve().parent.parent
    executable = (args.executable or project / "target/release/photo-gui.exe").resolve()
    root = (args.out or project / "target" / ("transform-animation-" + uuid.uuid4().hex[:8])).resolve()
    root.mkdir(parents=True, exist_ok=True)
    assert not any(root.iterdir()), "Use a new, empty artifact directory"
    media = root / "media"
    media.mkdir()
    base = root / "base"
    base.mkdir()
    (base / "yazi.toml").write_text("[mgr]\nshow_hidden=false\n", encoding="utf-8")
    source = media / "asymmetric.png"
    expected = Image.new("RGB", (960, 640), COLORS["red"])
    drawing = ImageDraw.Draw(expected)
    drawing.rectangle((347, 0, 959, 268), fill=COLORS["green"])
    drawing.rectangle((0, 269, 346, 639), fill=COLORS["blue"])
    drawing.rectangle((347, 269, 959, 639), fill=COLORS["yellow"])
    expected.save(source)
    pipe_name = "afio-animation-" + uuid.uuid4().hex
    environment = dict(os.environ, AFIO_API_NAME=pipe_name,
                       AFIO_SETTINGS_HOME=str(root / "settings"), YAZI_CONFIG_HOME=str(base))
    results = {"artifact": str(root), "executable": str(executable), "actions": [],
               "limitation": "Root screenshot requests and color geometry establish rendered image changes; request/ack timestamps are not exact monitor presentation times. Keyboard routing is outside this API check."}

    def rpc(command="status", **arguments):
        with open("\\\\.\\pipe\\" + pipe_name, "r+b", buffering=0) as pipe:
            pipe.write(json.dumps(dict(command=command, **arguments)).encode() + b"\n")
            response = b""
            while not response.endswith(b"\n"):
                chunk = pipe.read(65536)
                if not chunk:
                    raise RuntimeError("Isolated native API disconnected")
                response += chunk
                assert len(response) < 2 * 1024 * 1024, "Unexpected API response size"
        result = json.loads(response)
        if result.get("error"):
            raise RuntimeError(result["error"])
        return result

    def until(predicate, timeout=15):
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            state = rpc()
            if predicate(state):
                return state
            time.sleep(0.005)
        raise AssertionError("Timed out waiting for isolated GUI")

    def texture(state):
        return next((value for path, value in state["render"].get("preview_textures", [])
                     if Path(path).name == source.name), None)

    def request_capture(folder, name, origin):
        path = folder / (name + ".png")
        requested = time.perf_counter()
        rpc("capture_frame", path=str(path), viewport="root")
        return {"path": str(path), "request_ms": (requested - origin) * 1000,
                "ack_ms": (time.perf_counter() - origin) * 1000}

    def wait_capture(capture):
        deadline = time.perf_counter() + 5
        while time.perf_counter() < deadline:
            try:
                with Image.open(capture["path"]) as image:
                    image.load()
                return
            except (OSError, SyntaxError):
                time.sleep(0.005)
        raise AssertionError("Root screenshot was not saved: " + capture["path"])

    with (root / "native.log").open("w", encoding="utf-8") as log:
        process = subprocess.Popen([str(executable), str(media)], env=environment,
                                   creationflags=subprocess.CREATE_NO_WINDOW,
                                   stdout=log, stderr=log)
        try:
            deadline = time.perf_counter() + 15
            while True:
                try:
                    if rpc().get("snapshot"):
                        break
                except OSError:
                    pass
                assert process.poll() is None, "Isolated GUI exited; see native.log"
                assert time.perf_counter() < deadline, "Isolated API did not start"
                time.sleep(0.05)
            rpc("set_navigator_visible", visible=False)
            rpc("resize_viewport", width=1400, height=900, viewport="root")
            rpc("focus_pane", pane="gallery")
            rpc("select_path", path=str(source))
            rpc("open_selected")
            until(lambda state: texture(state) is not None)
            for action, transpose in ACTIONS:
                folder = root / action
                folder.mkdir()
                before_state = rpc()
                old_texture = texture(before_state)
                before = request_capture(folder, "before", time.perf_counter())
                wait_capture(before)
                issued = time.perf_counter()
                initial = rpc("run_action", id=action)
                # Admission may wait behind a prior native screenshot write.
                # Phase delays start after the action has actually been accepted.
                started = time.perf_counter()
                operation_id = initial["operation"]["id"]
                captures = {"before": before, "start": request_capture(folder, "start", started)}
                remaining = started + 0.095 - time.perf_counter()
                if remaining > 0:
                    time.sleep(remaining)
                captures["middle"] = request_capture(folder, "middle", started)
                remaining = started + 0.360 - time.perf_counter()
                if remaining > 0:
                    time.sleep(remaining)
                state = until(lambda item: item.get("operation", {}).get("id") == operation_id
                              and not item["operation"]["running"]
                              and texture(item) not in (None, old_texture))
                assert all(file["error"] is None for file in state["operation"]["files"]), state["operation"]
                captures["end"] = request_capture(folder, "end", started)
                record = {"action": action, "action_ack_ms": (started - issued) * 1000,
                          "captures": captures, "operation": state["operation"]}
                results["actions"].append(record)
                for capture in captures.values():
                    wait_capture(capture)
                expected = expected.transpose(transpose)
                with Image.open(source) as saved:
                    saved = saved.convert("RGB")
                    assert saved.size == expected.size and saved.tobytes() == expected.tobytes(), action
                assert [path.name for path in media.iterdir()] == [source.name], "Transform changed filenames/count"
                evidence = verify_geometry(action, captures)
                record.update({"saved_pixels_equal": True,
                               "filename_and_count_preserved": True, **evidence})
                (root / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
            results["passed"] = True
        except BaseException as error:
            results["passed"] = False
            results["error"] = repr(error)
            raise
        finally:
            stop_fixture(process)
            (root / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
            print(json.dumps({"artifact": str(root / "results.json"), "passed": results.get("passed"),
                              "actions": len(results["actions"])}))


if __name__ == "__main__":
    main()
