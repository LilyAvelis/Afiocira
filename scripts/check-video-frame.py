"""Isolated API verification; never changes the real clipboard."""
import json, os, shutil, subprocess, time, uuid
from pathlib import Path
from PIL import Image
project = Path(__file__).resolve().parent.parent
root = project / "target" / ("video-frame-" + uuid.uuid4().hex[:8])
media = root / "media"
media.mkdir(parents=True)
video = media / "synthetic.mp4"
subprocess.run([shutil.which("ffmpeg"), "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=red:s=64x48:r=10:d=2", "-f", "lavfi", "-i", "color=c=blue:s=64x48:r=10:d=2", "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0", "-c:v", "mpeg4", str(video)], check=True, creationflags=subprocess.CREATE_NO_WINDOW)
original = video.read_bytes()
env = dict(os.environ, AFIO_API_NAME="afio-frame-" + uuid.uuid4().hex, AFIO_SETTINGS_HOME=str(root / "settings"), YAZI_CONFIG_HOME=str(root / "yazi"))
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]
def rpc(command="status", allow_error=False, **args):
    with open(pipe, "r+b", buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data=b""
        while not data.endswith(b"\n"):
            part=stream.read(65536)
            if not part: raise RuntimeError("GUI disconnected")
            data+=part
    result=json.loads(data)
    if not allow_error: assert not result["error"], result["error"]
    return result
def wait(predicate, timeout=20):
    end=time.monotonic()+timeout
    while time.monotonic()<end:
        try:
            result=rpc()
            if predicate(result): return result
        except OSError: pass
        time.sleep(.05)
    raise AssertionError("Timed out")
def commands(): return {a["id"]:a for a in rpc("list_commands")["commands"]}
process=subprocess.Popen([str(project / "target/release/photo-gui.exe"),str(media)],env=env,creationflags=subprocess.CREATE_NO_WINDOW)
try:
    wait(lambda r:r["snapshot"] is not None)
    rpc("open",path=str(media))
    assert not {"copy_video_frame","save_video_frame"}.intersection(commands())
    rpc("open",path=str(video))
    wait(lambda r:r.get("video") and r["video"]["duration"]>3)
    rpc("video_pause",paused=True)
    rpc("video_seek",seconds=3)
    wait(lambda r:r["video"]["paused"] and r["video"]["position"]>2.8)
    assert {"copy_video_frame","save_video_frame"}.issubset(commands())
    settings=rpc("show_settings")["settings"]["shortcuts"]
    entries=[e for e in settings if e["action"] in ["copy_video_frame","save_video_frame"]]
    assert len(entries)==2 and all(e["context"]=="space" and e["binding"] is None and e["default_binding"] is None and e["rebindable"] for e in entries)
    rpc("close_settings")
    rpc("run_action",id="save_video_frame")
    result=wait(lambda r:r.get("operation") and not r["operation"]["running"])
    file=result["operation"]["files"][0]
    assert file["error"] is None,file
    output=Path(file["destination"])
    assert os.path.samefile(output.parent, video.parent) and output.suffix==".png" and output!=video
    assert Path(result["snapshot"]["selected_path"])==output and result["snapshot"]["view"]=="Photo"
    with Image.open(output) as im:
        assert im.size==(64,48)
        r,g,b=im.convert("RGB").getpixel((32,24))
        assert b>200 and r<30 and g<30,(r,g,b)
    assert video.read_bytes()==original
    assert not {"copy_video_frame","save_video_frame"}.intersection(commands())
    assert rpc("run_action",id="save_video_frame",allow_error=True)["error"]
    # Repeat exact same timestamp: checked publication must preserve the first PNG.
    first=output.read_bytes()
    rpc("open",path=str(video)); wait(lambda r:r.get("video") and r["video"]["duration"]>3)
    rpc("video_pause",paused=True); rpc("video_seek",seconds=3)
    wait(lambda r:r["video"]["paused"] and r["video"]["position"]>2.8)
    rpc("run_action",id="save_video_frame")
    result=wait(lambda r:r.get("operation") and not r["operation"]["running"] and r["operation"]["id"]>1)
    second=Path(result["operation"]["files"][0]["destination"])
    assert second!=output and output.read_bytes()==first and video.read_bytes()==original
    print(json.dumps(dict(result="passed",fixture=str(root),outputs=[str(output),str(second)],checks=["nonzero decoded blue frame","64x48 PNG","focus resulting image","source intact","unique no overwrite","Space-only actions","unbound Settings catalogue"])))
finally:
    process.terminate()
    process.wait(timeout=10)
