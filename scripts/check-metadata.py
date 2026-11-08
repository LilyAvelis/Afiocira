"""Exercise sparse metadata through real GUI pipe + CLI on synthetic files only.

Usage: python scripts/check-metadata.py [--bin-dir target/debug]
All processes/config/database/media belong to a unique retained target fixture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time
import uuid

from native_fixture import stop_fixture

project = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument("--bin-dir", default="target/debug")
args = parser.parse_args()
bins = (project / args.bin_dir).resolve()
root = project / "target" / ("metadata-" + uuid.uuid4().hex[:10])
media = root / "media"
media.mkdir(parents=True)
source = media / "кадр.png"
sample = (project / "assets/test-images/1.png").read_bytes()
for index in range(101):
    (source if index == 0 else media / f"untouched-{index}.png").write_bytes(sample)
original = hashlib.sha256(source.read_bytes()).hexdigest()
env = dict(os.environ, AFIO_API_NAME="afio-metadata-" + uuid.uuid4().hex,
           AFIO_DATA_HOME=str(root / "data"), AFIO_SETTINGS_HOME=str(root / "settings"),
           APPDATA=str(root / "appdata"), LOCALAPPDATA=str(root / "localappdata"))
pipe = "\\\\.\\pipe\\" + env["AFIO_API_NAME"]
db_path = root / "data/registry/registry.sqlite3"
process = None


def rpc(command="status", allow_error=False, **args):
    with open(pipe, "r+b", buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
        data = b""
        while not data.endswith(b"\n"):
            part = stream.read(65536)
            if not part:
                raise RuntimeError("Fixture disconnected")
            data += part
    result = json.loads(data)
    if not allow_error:
        assert not result["error"], result["error"]
    return result


def launch():
    global process
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    process = subprocess.Popen([str(bins / "photo-gui.exe"), str(media)], env=env,
                               startupinfo=startup, creationflags=subprocess.CREATE_NO_WINDOW)
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        assert process.poll() is None, "Fixture exited before API startup"
        try:
            state = rpc()
            if state["snapshot"]:
                rpc("open", path=str(media))
                return
        except OSError:
            pass
        time.sleep(.05)
    raise AssertionError("Fixture API startup timed out")


def receipt(started):
    job = started["metadata"]
    deadline = time.monotonic() + 10
    while job["pending"]:
        assert time.monotonic() < deadline, job
        time.sleep(.02)
        reply = rpc("metadata_status", request_id=job["request_id"])
        assert reply["session_id"] == started["session_id"]
        job = reply["metadata"]
    return job


def metadata(op, path=source, **kwargs):
    return receipt(rpc("metadata", request=dict(op=op, path=str(path), **kwargs)))


def save(text, revision=None):
    return metadata("save", expected_revision=revision,
                    metadata=dict(tags=["портрет", "test"], comment=text))


def cli(*args, payload=None):
    result = subprocess.run([str(bins / "photo-cli.exe"), *args], input=payload,
                            text=True, encoding="utf8", capture_output=True, timeout=10,
                            env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    assert result.returncode == 0, (result.stdout, result.stderr)
    return json.loads(result.stdout)


try:
    launch()
    # Repeated gallery/Space operations and reads must not initialize storage.
    for _ in range(5):
        rpc("open", path=str(source))
        rpc("back")
        rpc("select", index=0)
        rpc("open_selected")
        rpc("next")
        rpc("back")
        rpc("refresh")
        assert metadata("inspect")["snapshot"]["image_id"] is None
    assert not (root / "data").exists()
    first = save("persistent note")
    assert not first["error"] and first["snapshot"]["revision"] == 1, first
    image_id = first["snapshot"]["image_id"]
    assert len(image_id) == 36
    assert save("must not overwrite")["error"]["code"] == "conflict"
    assert save("second note", 1)["snapshot"]["revision"] == 2
    assert save("stale", 1)["error"]["code"] == "conflict"
    # Canonicalized paths returned by Gallery resolve the same metadata.
    gallery_path = rpc("open", path=str(source))["snapshot"]["selected_path"]
    assert metadata("inspect", Path(gallery_path))["snapshot"]["image_id"] == image_id
    for path in sorted(media.glob("untouched-*.png"))[:10]:
        assert metadata("inspect", path)["snapshot"]["image_id"] is None
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT count(*) FROM images").fetchone()[0] == 1
        assert db.execute("SELECT count(*) FROM file_bindings").fetchone()[0] == 1
        assert db.execute("PRAGMA user_version").fetchone()[0] == 1
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert not db.execute("PRAGMA foreign_key_check").fetchall()
        # While another writer owns SQLite, status/navigation stay responsive.
        db.execute("BEGIN IMMEDIATE")
        pending = rpc("metadata", request=dict(op="save", path=str(source),
                      expected_revision=2, metadata=dict(comment="locked")))
        start = time.monotonic()
        rpc("status")
        assert time.monotonic() - start < 2, "SQLite blocked GUI status"
        locked = receipt(pending)
        assert locked["error"]["code"] == "storage", locked
        db.rollback()
    read = receipt(cli("ctl", "metadata", str(source)))
    assert read["snapshot"]["metadata"]["comment"] == "second note"
    # CLI JSON save uses exactly the same dispatcher/worker/revision checks.
    started = cli("ctl-json", payload=json.dumps(dict(command="metadata", request=dict(
        op="save", path=str(source), expected_revision=2, metadata=dict(comment="CLI saved")))) + "\n")
    assert receipt(started)["snapshot"]["revision"] == 3
    stop_fixture(process)
    process = None
    launch()
    restored = metadata("inspect")["snapshot"]
    assert restored["image_id"] == image_id and restored["revision"] == 3
    assert restored["metadata"]["comment"] == "CLI saved"
    assert hashlib.sha256(source.read_bytes()).hexdigest() == original
    # Keep the old file alive so a replacement definitely has another File ID.
    source.rename(media / "retained-original.png")
    source.write_bytes(sample)
    assert not metadata("inspect")["snapshot"]["binding_current"]
    assert save("wrong file", 3)["error"]["code"] == "binding_changed"
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT count(*) FROM images").fetchone()[0] == 1
    print(json.dumps(dict(result="passed", fixture=str(root), images=101,
                         registered=1, checks=["sparse browse/read", "canonical path", "save/restart",
                         "optimistic revisions", "source bytes unchanged", "foreign keys/integrity",
                         "bounded SQLite busy", "live GUI responsiveness", "CLI inspect/save",
                         "replacement refused"])))
finally:
    if process is not None:
        stop_fixture(process)
