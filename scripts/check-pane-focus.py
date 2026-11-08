"""Exercise the same focus commands used by Tab, in an isolated native session."""
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

project = Path(__file__).resolve().parent.parent
root = project / "target" / ("pane-focus-" + uuid.uuid4().hex[:8])
media = root / "media"
media.mkdir(parents=True)
(media / "a.png").write_bytes((project / "assets/test-images/1.png").read_bytes())
(media / "b.png").write_bytes((project / "assets/test-images/2.png").read_bytes())
env = dict(os.environ, AFIO_API_NAME="afio-focus-" + uuid.uuid4().hex,
           AFIO_SETTINGS_HOME=str(root / "config"), AFIO_YAZI_TRACE_PTY="1")
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]

def rpc(command="status", **args):
    with open(pipe, "r+b", buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            data += stream.read(65536)
    return json.loads(data)

process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)],
                           env=env, creationflags=subprocess.CREATE_NO_WINDOW)
try:
    deadline = time.monotonic() + 15
    while True:
        try:
            if rpc()["snapshot"]:
                break
        except OSError:
            pass
        assert time.monotonic() < deadline
        time.sleep(.05)
    time.sleep(1)
    for iteration in range(3):
        for pane in ["gallery", "navigator"]:
            if pane == "navigator":
                result = rpc("select_path", path=str(media / "b.png"))
                assert not result["error"], result["error"]
                result = rpc("set_marked", paths=[str(media / "a.png"), str(media / "b.png")])
                assert not result["error"], result["error"]
            result = rpc("focus_pane", pane=pane)
            if result["error"]:
                import shutil
                for session in Path(os.environ["TEMP"]).glob(f"photo-workshop-yazi-{process.pid}-*"):
                    destination = root / session.name
                    destination.mkdir()
                    for artifact in [session / "pty.log", *session.glob("snapshot.*")]:
                        if artifact.exists():
                            shutil.copy2(artifact, destination / artifact.name)
                print(root)
            assert not result["error"], result["error"]
            assert result["navigator"]["focus"] == pane, result["navigator"]
            if pane == "gallery" and iteration:
                assert Path(result["snapshot"]["cursor_path"]).name == "b.png", result["snapshot"]
                assert {Path(p).name for p in result["snapshot"]["marked_paths"]} == {"a.png", "b.png"}, result["snapshot"]
    print(json.dumps(dict(repeated_pane_focus=True, cursor_and_marks_roundtrip=True)))
finally:
    # Terminate only the owned synthetic GUI tree, including any helper in flight.
    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                   creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
    process.wait(timeout=10)
