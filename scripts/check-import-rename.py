"""Exercise import/F2/drop controllers through an isolated GUI API, no desktop input.
Fixture: --folder contains source/photo.png, source/video.mp4, and gallery/.
Use synthetic media only. Creates/renames copies in gallery for inspection.
"""
import argparse
import hashlib
import json
import os
import subprocess
import time
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument("--cli",required=True)
p.add_argument("--pipe",required=True)
p.add_argument("--folder",required=True)
a=p.parse_args()
root=Path(a.folder).resolve()
folder=root/"gallery"
env={**os.environ,"AFIO_API_NAME":a.pipe}

def call(*args,allow_error=False):
    process=subprocess.run([a.cli,"ctl",*map(str,args)],env=env,stdout=subprocess.PIPE)
    result=json.loads(process.stdout)
    assert allow_error or result["error"] is None,result
    return result

def wait(predicate):
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        state=call("status")
        if predicate(state):return state
        time.sleep(.05)
    raise AssertionError(state)

def finish(state):
    ident=state["operation"]["id"]
    return wait(lambda r:r["operation"]["id"]==ident and not r["operation"]["running"])

def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()

assert call("open",folder)["native_paste_shortcut"],"Native Ctrl+V observer failed to install"
assert "paste" in {c["id"] for c in call("commands")["commands"]}
sources=[root/"source/photo.png",root/"source/video.mp4"]
for _ in range(2):
    imported=finish(call("import",*sources))
    assert len(imported["operation"]["files"])==2
    for file in imported["operation"]["files"]:
        assert file["error"] is None,file
        assert digest(file["source"])==digest(file["destination"])
assert len(list(folder.iterdir()))==4

call("mark")
photo=folder/"photo.png"
call("select-path",photo)
draft=call("begin-rename")["rename"]
assert draft["new_name"]=="photo.png" and not draft["busy"]
renamed=finish(call("rename","renamed-photo.png"))
item=renamed["operation"]["files"][0]
assert item["error"] is None,item
photo=Path(item["destination"])
assert not (folder/"photo.png").exists()
assert digest(photo)==digest(sources[0])
assert renamed["snapshot"]["cursor_path"]==str(photo)

call("begin-rename")
collision=finish(call("rename","video.mp4"))
assert collision["operation"]["files"][0]["error"]
assert collision["rename"]["error"]
assert digest(folder/"video.mp4")==digest(sources[1])
call("cancel-rename")

call("begin-rename")
original=photo.read_bytes()
photo.write_bytes(b"externally changed content")
changed=finish(call("rename","must-not-exist.png"))
assert changed["operation"]["files"][0]["error"]
assert not (folder/"must-not-exist.png").exists()
assert photo.read_bytes()==b"externally changed content"
photo.write_bytes(original)
call("cancel-rename")

call("mark")
video=folder/"video.mp4"
call("select-path",video)
call("open-selected")
wait(lambda r:r["video"]["duration"]>0)
call("video-pause")
wait(lambda r:r["video"]["paused"])
call("video-seek",8)
wait(lambda r:abs(r["video"]["position"]-8)<.1)
call("begin-rename")
renamed=finish(call("rename","renamed-video.mp4"))
item=renamed["operation"]["files"][0]
assert item["error"] is None,item
video=Path(item["destination"])
assert digest(video)==digest(sources[1])
assert renamed["snapshot"]["view"]=="Photo",renamed
wait(lambda r:r["video"]["path"]==str(video) and r["video"]["paused"] and abs(r["video"]["position"]-8)<.2)
before={f.name for f in folder.iterdir()}
multiple=call("drop-open",photo,video)
assert multiple["snapshot"]["view"]=="Grid"
assert len(multiple["snapshot"]["marked_paths"])==2
single=call("drop-open",photo)
assert single["snapshot"]["view"]=="Photo"
call("drop-open",folder)
assert before=={f.name for f in folder.iterdir()}
assert digest(sources[0])==digest(photo)
print("PASS: unique import, exact bytes, rename draft/collision/stale source, active-video rename+position, drop controllers, native paste observer installation")
