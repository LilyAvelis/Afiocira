"""Monotonic Delete benchmark on synthetic local JPEG copies only.

No desktop input or clipboard use. Recycled test files are restored in finally.
Readiness means a successor preview texture is ready for drawing, not scan-out.
"""
import argparse
import hashlib
import json
import math
import os
import statistics
import subprocess
import time
import uuid
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument("--pipe",required=True)
p.add_argument("--image",required=True)
p.add_argument("--out",required=True)
p.add_argument("--samples",type=int,default=24)
a=p.parse_args()
prefix="afio-bench-"+uuid.uuid4().hex+"-"
root=(Path("target")/(prefix+"fixture")).resolve()
root.mkdir()
data=Path(a.image).read_bytes()
digest=hashlib.sha256(data).hexdigest()
files=[root/(prefix+f"{i:03}.jpg") for i in range(a.samples+8)]
for file in files:file.write_bytes(data)


def rpc(command):
    with open("\\\\.\\pipe\\"+a.pipe,"r+b",buffering=0) as pipe:
        pipe.write(json.dumps(command).encode()+b"\n")
        line=bytearray()
        while not line.endswith(b"\n"):
            chunk=pipe.read(65536)
            if not chunk:raise RuntimeError("API disconnected")
            line.extend(chunk)
    state=json.loads(line)
    assert state["error"] is None,state
    return state


def wait(predicate):
    deadline=time.monotonic()+20
    while time.monotonic()<deadline:
        state=rpc({"command":"status"})
        if predicate(state):return state
        time.sleep(.002)
    raise AssertionError(state)


def bin_items(restore=False):
    script=r'''
$ErrorActionPreference='Stop'
$folder=(New-Object -ComObject Shell.Application).Namespace(10)
if(-not $folder){throw 'No Recycle Bin namespace'}
$matches=@()
foreach($item in @($folder.Items())){
 if(-not $item.Name.StartsWith($env:AFIO_BENCH_PREFIX,[StringComparison]::Ordinal)){continue}
 $from=[string]$item.ExtendedProperty('System.Recycle.DeletedFrom')
 if(-not $from){$from=$folder.GetDetailsOf($item,1)}
 if(-not $from.TrimEnd([char]92).Equals($env:AFIO_BENCH_FOLDER.TrimEnd([char]92),[StringComparison]::OrdinalIgnoreCase)){continue}
 $matches+= $item.Name
 if($env:AFIO_BENCH_RESTORE -eq '1'){$item.InvokeVerb('undelete')}
}
ConvertTo-Json -InputObject @($matches) -Compress
'''
    process=subprocess.run(["powershell.exe","-NoProfile","-NonInteractive","-Command",script],env={**os.environ,"AFIO_BENCH_PREFIX":prefix,"AFIO_BENCH_FOLDER":str(root),"AFIO_BENCH_RESTORE":"1" if restore else "0"},stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert process.returncode==0,process.stderr.decode(errors="replace")
    return json.loads(process.stdout.decode("utf-8-sig"))


def stats(values):
    values=sorted(values)
    return {"median":round(statistics.median(values),3),"p95":round(values[math.ceil(.95*len(values))-1],3),"min":round(values[0],3),"max":round(values[-1],3)}


rows=[]
groups=[]
try:
    rpc({"command":"open","path":str(root)})
    time.sleep(.5)
    rpc({"command":"set_order","paths":[str(f) for f in files]})
    rpc({"command":"set_marked","paths":[]})
    rpc({"command":"select","index":0})
    rpc({"command":"open_selected"})
    for _ in range(a.samples):
        # This is the normal warm-neighbour case; a cold next image is a different test.
        wait(lambda r:r["render"]["preview_ready"] and r["render"]["next_preview_ready"])
        start=time.perf_counter()
        state=rpc({"command":"run_action","id":"delete"})
        accepted=(time.perf_counter()-start)*1000
        ident=state["operation"]["id"]
        state=wait(lambda r:r["operation"]["id"]==ident and not r["operation"]["running"] and r["operation"]["timing"]["preview_ready_ms"] is not None)
        assert all(f["error"] is None for f in state["operation"]["files"]),state
        row={**state["operation"]["timing"],"api_accept_ms":accepted}
        row["completion_to_preview_ms"]=row["preview_ready_ms"]-row["ui_applied_ms"]
        row["worker_to_ui_gap_ms"]=row["ui_received_ms"]-row["prepare_and_queue_ms"]-row["work_ms"]
        rows.append(row)
    rpc({"command":"back"})
    for size in [3,5]:
        remaining=sorted(f for f in files if f.exists())
        rpc({"command":"set_marked","paths":[str(f) for f in remaining[:size]]})
        draft=rpc({"command":"run_action","id":"delete"})
        assert draft["delete_confirmation"]["count"]==size
        state=rpc({"command":"confirm_delete"})
        ident=state["operation"]["id"]
        state=wait(lambda r:r["operation"]["id"]==ident and not r["operation"]["running"])
        assert all(f["error"] is None for f in state["operation"]["files"]),state
        groups.append({"files":size,**state["operation"]["timing"]})
    receipts=len(bin_items())
    assert receipts==len(files),(receipts,len(files))
    result={"scope":{"samples":a.samples,"folder_files_initial":len(files),"image_bytes":len(data),"warm_next_preview":True,"readiness":"CPU-side full preview texture, not monitor presentation"},"milliseconds":{key:stats([row[key] for row in rows]) for key in rows[0]},"groups":groups,"raw":rows,"recycle_receipts":receipts}
    Path(a.out).write_text(json.dumps(result,indent=2),encoding="utf-8")
    print(json.dumps({"milliseconds":result["milliseconds"],"groups":groups,"recycle_receipts":receipts},indent=2))
finally:
    restored=bin_items(restore=True)
    deadline=time.monotonic()+20
    while time.monotonic()<deadline and not all(f.exists() for f in files):time.sleep(.1)
    assert all(f.exists() and hashlib.sha256(f.read_bytes()).hexdigest()==digest for f in files),"Synthetic restore failed"
    print(f"Restored {len(restored)} recycled synthetic JPEGs; all {len(files)} retained with identical bytes.")
