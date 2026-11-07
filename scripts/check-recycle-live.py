"""Explicit live Delete smoke test: synthetic GUID media only, no desktop input.

Requires isolated app with AFIO_API_NAME matching --pipe. Creates its own fixture
under target/, verifies Recycle Bin receipts, and restores its own items in finally.
Never touches the user's existing media or clipboard. No permanent deletion.
"""
import argparse
import hashlib
import json
import os
import subprocess
import time
import uuid
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument("--cli",required=True)
p.add_argument("--pipe",required=True)
p.add_argument("--image",required=True)
p.add_argument("--video",required=True)
a=p.parse_args()
prefix="afio-delete-"+uuid.uuid4().hex+"-"
root=(Path("target")/(prefix+"fixture")).resolve()
root.mkdir()
env={**os.environ,"AFIO_API_NAME":a.pipe,"AFIO_DELETE_PREFIX":prefix,"AFIO_DELETE_FOLDER":str(root)}
image=Path(a.image).read_bytes()
video=Path(a.video).read_bytes()
files={i:root/(prefix+f"{i:02}."+("mp4" if i==8 else "png")) for i in range(1,9)}
for i,path in files.items():path.write_bytes(video if i==8 else image)
expected={path.name:hashlib.sha256(path.read_bytes()).hexdigest() for path in files.values()}


def call(*arguments):
    process=subprocess.run([a.cli,"ctl",*map(str,arguments)],env=env,stdout=subprocess.PIPE)
    result=json.loads(process.stdout)
    assert result["error"] is None,result
    return result


def wait(predicate):
    deadline=time.monotonic()+12
    while time.monotonic()<deadline:
        state=call("status")
        if predicate(state):return state
        time.sleep(.06)
    raise AssertionError(state)


def finish(state):
    ident=state["operation"]["id"]
    return wait(lambda r:r["operation"]["id"]==ident and not r["operation"]["running"])


def bin_items(restore=False):
    # Read/restore only this unpredictable test prefix AND the exact fixture folder.
    script=r'''
$ErrorActionPreference='Stop'
$folder=(New-Object -ComObject Shell.Application).Namespace(10)
if(-not $folder){throw 'Recycle Bin namespace unavailable'}
$matches=@()
foreach($item in @($folder.Items())){
  if(-not $item.Name.StartsWith($env:AFIO_DELETE_PREFIX,[StringComparison]::Ordinal)){continue}
  $from=[string]$item.ExtendedProperty('System.Recycle.DeletedFrom')
  if(-not $from){$from=$folder.GetDetailsOf($item,1)}
  if(-not $from.TrimEnd('\').Equals($env:AFIO_DELETE_FOLDER.TrimEnd('\'),[StringComparison]::OrdinalIgnoreCase)){continue}
  $matches+= [pscustomobject]@{name=$item.Name;from=$from}
  if($env:AFIO_DELETE_RESTORE -eq '1'){$item.InvokeVerb('undelete')}
}
ConvertTo-Json -InputObject @($matches) -Compress
'''
    process=subprocess.run(["powershell.exe","-NoProfile","-NonInteractive","-Command",script],env={**env,"AFIO_DELETE_RESTORE":"1" if restore else "0"},stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert process.returncode==0,process.stderr.decode(errors="replace")
    return json.loads(process.stdout.decode("utf-8-sig",errors="replace"))


try:
    call("open",root)
    # Wait for the independent navigator before applying a deterministic order.
    time.sleep(.4)
    call("order",*files.values())
    call("mark",files[1],files[2])
    call("select-path",files[1])
    call("open-selected")
    one=call("delete")
    assert one["delete_confirmation"] is None
    one=finish(one)
    assert one["operation"]["files"][0]["error"] is None,one
    assert not files[1].exists() and files[2].exists()
    assert one["snapshot"]["view"]=="Photo" and one["snapshot"]["selected_path"].endswith(files[2].name),one
    assert len(bin_items())>=1,"Deleted synthetic file did not reach Recycle Bin"
    call("back")
    call("mark")
    call("select-path",files[3])
    one=call("delete")
    assert one["delete_confirmation"] is None
    one=finish(one)
    assert one["operation"]["files"][0]["error"] is None and not files[3].exists(),one
    call("mark",files[2],files[4])
    draft=call("delete")
    assert draft["delete_confirmation"]["count"]==2
    assert files[2].exists() and files[4].exists()
    call("cancel-delete")
    assert files[2].exists() and files[4].exists()
    call("delete")
    call("mark",files[7])
    batch=finish(call("confirm-delete"))
    assert all(f["error"] is None for f in batch["operation"]["files"]),batch
    assert not files[2].exists() and not files[4].exists() and files[7].exists()
    assert len(batch["snapshot"]["marked_paths"])==1 and batch["snapshot"]["marked_paths"][0].endswith(files[7].name),batch
    call("mark",files[5],files[6])
    call("delete")
    files[6].write_bytes(b"changed after confirmation opened")
    expected[files[6].name]=hashlib.sha256(files[6].read_bytes()).hexdigest()
    partial=finish(call("confirm-delete"))
    assert sum(f["error"] is not None for f in partial["operation"]["files"])==1,partial
    assert not files[5].exists() and files[6].exists()
    assert any(path.endswith(files[6].name) for path in partial["snapshot"]["marked_paths"]),partial
    call("mark")
    call("select-path",files[8])
    call("open-selected")
    wait(lambda r:r["video"]["duration"]>0)
    movie=finish(call("delete"))
    assert movie["operation"]["files"][0]["error"] is None and not files[8].exists(),movie
    commands=call("commands")["commands"]
    assert commands[-1]["id"]=="delete",commands
    assert len(bin_items())==6,"Recycle receipts did not match successful deletions"
    print("PASS: single Space scope/successor, single Gallery, group confirmation/cancel/captured targets, stale-source partial error, active video, Delete-last, 6 native Recycle Bin receipts")
finally:
    # Stop the test player's access and restore only the test's own recycled media.
    try:call("back")
    except Exception:pass
    restored=bin_items(restore=True)
    deadline=time.monotonic()+10
    while time.monotonic()<deadline and not all(path.exists() for path in files.values()):time.sleep(.1)
    for path in files.values():
        assert path.exists(),f"Synthetic test file needs restore: {path}"
        assert hashlib.sha256(path.read_bytes()).hexdigest()==expected[path.name]
    print(f"Restored {len(restored)} test items; all 8 synthetic files retained with expected contents.")
