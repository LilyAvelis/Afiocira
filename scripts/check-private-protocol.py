"""Native regressions for asynchronous focus and settings without ya/DDS helpers.

Only synthetic media/config and Popen-owned process trees are changed. No global
DDS listener, clipboard, existing GUI, or installed binary is touched.
"""
import concurrent.futures
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

from native_fixture import stop_fixture, descendant_processes

PROJECT = Path(__file__).resolve().parents[1]
ROOT = PROJECT / "target" / ("private-protocol-" + uuid.uuid4().hex[:8])
ROOT.mkdir()
BINARY = Path(os.environ.get("AFIO_TEST_GUI", PROJECT / "target/release/photo-gui.exe"))
MEDIA = ROOT / "media"
MEDIA.mkdir()
for name in ["a.png", "b.png"]:
    (MEDIA / name).write_bytes((PROJECT / "assets/test-images/1.png").read_bytes())
BASE = ROOT / "base"
BASE.mkdir()
(BASE / "yazi.toml").write_text('[mgr]\nsort_by="alphabetical"\nshow_hidden=false\nlinemode="none"\n', encoding="utf8")

# An ordinary synthetic user plugin changes runtime preferences only once, in
# the initial process. Replacement Yazi must restore them through our protocol.
plugin = BASE / "plugins/fixture-runtime.yazi"
plugin.mkdir(parents=True)
(plugin / "main.lua").write_text(r'''
local pref = ya.sync(function()
  local p = cx.active.pref
  return { sort_by=p.sort_by, sort_reverse=p.sort_reverse, show_hidden=p.show_hidden, linemode=p.linemode }
end)
return { entry=function()
  ya.emit("sort", { "natural", reverse=true, dir_first=false })
  ya.emit("hidden", { "show" })
  ya.emit("linemode", { "size" })
  ya.sleep(0.1)
  fs.write(Url(os.getenv("AFIO_TEST_PREF_PROOF")), ya.json_encode(pref()))
  ya.sleep(tonumber(os.getenv("AFIO_TEST_USER_TASK_SECONDS")) or 0)
end }
''', encoding="utf8")
(BASE / "init.lua").write_text(r'''
local once = os.getenv("AFIO_TEST_PREF_ONCE")
local existing = io.open(once, "rb")
if existing then existing:close() else
  local file = io.open(once, "wb"); file:write("initial"); file:close()
  ya.async(function() ya.sleep(0.4); ya.emit("plugin", { "fixture-runtime" }) end)
end
''', encoding="utf8")


class Session:
    def __init__(self, binary=BINARY, mode="none", user_task_seconds=0):
        self.root = ROOT / uuid.uuid4().hex[:8]
        self.root.mkdir()
        self.env = dict(os.environ, AFIO_API_NAME="afio-private-" + uuid.uuid4().hex,
                        AFIO_SETTINGS_HOME=str(self.root / "settings"), YAZI_CONFIG_HOME=str(BASE),
                        AFIO_TEST_PREF_ONCE=str(self.root / "once"),
                        AFIO_TEST_PREF_PROOF=str(self.root / "preferences.json"),
                        AFIO_TEST_YA_MARKER=str(self.root / "ya-called"), AFIO_TEST_YA_MODE=mode)
        self.env["AFIO_TEST_USER_TASK_SECONDS"] = str(user_task_seconds)
        self.pipe = "\\\\.\\pipe\\" + self.env["AFIO_API_NAME"]
        self.process = subprocess.Popen([str(binary), str(MEDIA)], env=self.env,
                                        creationflags=subprocess.CREATE_NO_WINDOW)
        self.wait(lambda r: r.get("snapshot") is not None)
        self.config = Path(self.rpc("configure_yazi", request={"op": "inspect"})["settings"]["yazi"]["config_directory"])
        until = time.monotonic() + 8
        while not (self.root / "preferences.json").exists():
            assert time.monotonic() < until, "runtime preference fixture did not run"
            time.sleep(.02)
        self.preferences = json.loads((self.root / "preferences.json").read_text())
        assert self.preferences == dict(sort_by="natural", sort_reverse=True, show_hidden=True, linemode="size"), self.preferences
        # The proof is written by a user-plugin task just before it returns.
        # Allow that fixture task and initial synthetic-file loads to settle.
        time.sleep(.5)

    def rpc(self, command="status", allow_error=False, **args):
        # Windows may briefly report a busy named-pipe instance as EINVAL while
        # the listener hands the preceding connection to its worker.
        until = time.monotonic() + 1
        while True:
            try:
                stream = open(self.pipe, "r+b", buffering=0)
                break
            except OSError as error:
                if error.errno not in [2, 22] or time.monotonic() >= until:
                    raise
                time.sleep(.01)
        with stream:
            stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
            data = b""
            while not data.endswith(b"\n"):
                part = stream.read(65536)
                if not part:
                    raise RuntimeError("Owned GUI disconnected")
                data += part
        result = json.loads(data)
        if not allow_error:
            assert not result["error"], result["error"]
        return result

    def wait(self, predicate, timeout=15):
        until = time.monotonic() + timeout
        last = None
        while time.monotonic() < until:
            try:
                last = self.rpc()
                if predicate(last):
                    return last
            except OSError:
                pass
            time.sleep(.03)
        raise AssertionError(last)

    def close(self):
        stop_fixture(self.process)


class PausedOwnYazi:
    def __init__(self, session):
        rows = descendant_processes(session.process)
        child = next(row for row in rows if row["ExecutablePath"] and Path(row["ExecutablePath"]).name.lower() == "yazi.exe")
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.native = ctypes.WinDLL("ntdll")
        self.kernel.OpenProcess.restype = ctypes.c_void_p
        self.kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        self.kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        self.native.NtSuspendProcess.argtypes = [ctypes.c_void_p]
        self.native.NtResumeProcess.argtypes = [ctypes.c_void_p]
        self.handle = self.kernel.OpenProcess(0x0800, False, child["ProcessId"])
        assert self.handle and self.native.NtSuspendProcess(self.handle) == 0

    def close(self):
        if self.handle:
            assert self.native.NtResumeProcess(self.handle) == 0
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def process_cpu_seconds(pid):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel.GetProcessTimes.argtypes = [ctypes.c_void_p] + [ctypes.POINTER(ctypes.c_ulonglong)] * 4
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    handle = kernel.OpenProcess(0x1000, False, pid)
    assert handle
    values = [ctypes.c_ulonglong() for _ in range(4)]
    try:
        assert kernel.GetProcessTimes(handle, *[ctypes.byref(value) for value in values])
        return (values[2].value + values[3].value) / 10_000_000
    finally:
        kernel.CloseHandle(handle)


def asynchronous_focus():
    session = Session()
    paused = None
    try:
        session.rpc("focus_pane", pane="navigator")
        session.wait(lambda r: not r["navigator"]["pending"])
        paused = PausedOwnYazi(session)
        idle_cpu_before = process_cpu_seconds(session.process.pid)
        idle_wall_before = time.perf_counter()
        time.sleep(.3)
        idle_cpu_seconds = process_cpu_seconds(session.process.pid) - idle_cpu_before
        idle_wall_seconds = time.perf_counter() - idle_wall_before
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as workers:
            pending = workers.submit(session.rpc, "focus_pane", pane="gallery", allow_error=True)
            session.wait(lambda r: r["navigator"]["pending"], timeout=1)
            cpu_before = process_cpu_seconds(session.process.pid)
            wall_before = time.perf_counter()
            latencies = []
            for _ in range(8):
                start = time.perf_counter()
                result = session.rpc()
                latencies.append(time.perf_counter() - start)
                assert result["navigator"]["focus"] == "navigator"
                time.sleep(.025)
            cpu_during_pending = process_cpu_seconds(session.process.pid) - cpu_before
            wall_during_pending = time.perf_counter() - wall_before
            assert max(latencies) < .5, latencies
            expired = pending.result(timeout=5)
        assert expired["error"] and "timed out" in expired["error"]["message"], expired
        # A context-sensitive action also waits for a fresh snapshot. It cannot
        # open a rename dialog or act against the old Gallery after timeout.
        blocked = session.rpc("run_action", id="rename", allow_error=True)
        assert blocked["error"] and not session.rpc()["rename"], blocked
        canceled = []
        last_id = session.rpc()["navigator"]["request_id"] or 0
        cases = [
            ("rename", dict(new_name="renamed.png"), "cancel_rename"),
            ("begin_rename", {}, "cancel_rename"),
            ("run_action", dict(id="rename"), "cancel_rename"),
            ("run_action", dict(id="delete"), "cancel_delete"),
            ("show_commands", {}, "close_commands"),
        ]
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as workers:
            for command, args, cancel in cases:
                original = workers.submit(session.rpc, command, allow_error=True, **args)
                try:
                    pending_state = session.wait(lambda r: (r["navigator"]["request_id"] or 0) > last_id, timeout=1)
                except AssertionError:
                    if original.done():
                        raise AssertionError(dict(command=command, reply=original.result())) from None
                    raise
                last_id = pending_state["navigator"]["request_id"]
                # An unrelated cancel and status must leave the requested
                # continuation pending; the matching cancel retires it.
                session.rpc("cancel_delete" if cancel == "cancel_rename" else "cancel_rename")
                assert not original.done(), "unrelated cancellation retired a continuation"
                session.rpc(cancel)
                result = original.result(timeout=1)
                assert result["error"] and "canceled" in result["error"]["message"], result
                canceled.append(command + (":" + args["id"] if "id" in args else ""))
        paused.close()
        paused = None
        session.wait(lambda r: not r["navigator"]["pending"])
        after_cancel = session.rpc()
        assert not after_cancel["rename"] and not after_cancel["delete_confirmation"]
        assert not after_cancel["palette"]["open"]
        assert not (MEDIA / "renamed.png").exists()
        assert (MEDIA / "a.png").is_file() and (MEDIA / "b.png").is_file()
        session.rpc("open", path=str(MEDIA / "b.png"))
        session.rpc("focus_pane", pane="navigator")
        fresh = session.rpc("focus_pane", pane="gallery")
        assert Path(fresh["snapshot"]["cursor_path"]).name == "b.png", fresh
        return dict(status_latency_max=max(latencies), timeout=expired["error"], stale_action_rejected=True,
                    pending_cpu_seconds=cpu_during_pending, pending_wall_seconds=wall_during_pending,
                    idle_cpu_seconds=idle_cpu_seconds, idle_wall_seconds=idle_wall_seconds,
                    canceled_continuations=canceled,
                    fresh_cursor=fresh["snapshot"]["cursor_path"])
    finally:
        if paused:
            paused.close()
        session.close()


def unavailable_helper(mode, bundle):
    session = Session(bundle / "photo-gui.exe", mode=mode, user_task_seconds=1.3)
    try:
        session.rpc("focus_pane", pane="gallery")
        session.rpc("configure_yazi", request=dict(op="set_general", key="scrolloff", value="5"))
        started = time.perf_counter()
        session.rpc("configure_yazi", request=dict(op="apply"))
        done = session.wait(lambda r: not r["settings"]["yazi"]["busy"])
        state = done["settings"]["yazi"]
        assert not state["error"], state
        assert state["config_directory"] != str(session.config), state
        assert state["effective_general"]["scrolloff"] == "5", state
        assert not (session.root / "ya-called").exists(), "Managed focus/Apply invoked ya.exe"
        return dict(mode=mode, seconds=time.perf_counter()-started, restored_preferences=session.preferences,
                    replacement_verified=True, ya_invoked=False, transient_user_task_finished=True)
    finally:
        session.close()


def persistent_user_task_is_preserved(bundle):
    session = Session(bundle / "photo-gui.exe", mode="fail", user_task_seconds=8)
    try:
        session.rpc("configure_yazi", request=dict(op="set_general", key="scrolloff", value="5"))
        started = time.perf_counter()
        session.rpc("configure_yazi", request=dict(op="apply"))
        done = session.wait(lambda r: not r["settings"]["yazi"]["busy"])
        state = done["settings"]["yazi"]
        assert state["error"] and "tasks did not finish" in state["error"], state
        assert state["config_directory"] == str(session.config), state
        assert not (session.root / "settings/settings.json").exists(), "blocked Apply persisted its draft"
        assert not (session.root / "ya-called").exists()
        return dict(seconds=time.perf_counter()-started, error=state["error"], old_yazi_preserved=True,
                    draft_not_persisted=True)
    finally:
        session.close()


def main():
    bundle = ROOT / "bundle"
    tools = bundle / "yazi"
    tools.mkdir(parents=True)
    shutil.copy2(BINARY, bundle / "photo-gui.exe")
    shutil.copy2(BINARY.with_name("photo-cli.exe"), bundle / "photo-cli.exe")
    yazi = shutil.which("yazi.exe")
    assert yazi, "Yazi must be installed for this synthetic native check"
    shutil.copy2(yazi, tools / "yazi.exe")
    source = ROOT / "unavailable-ya.rs"
    source.write_text('''fn main() {
      std::fs::write(std::env::var_os("AFIO_TEST_YA_MARKER").unwrap(), b"called").unwrap();
      if std::env::var("AFIO_TEST_YA_MODE").unwrap() == "hang" { std::thread::sleep(std::time::Duration::from_secs(20)); }
      std::process::exit(9);
    }''', encoding="utf8")
    subprocess.run(["rustc", "--crate-name", "fixture_ya", str(source), "-o", str(tools / "ya.exe")], check=True)
    results = dict(root=str(ROOT), helpers=[])
    try:
        results["asynchronous_focus"] = asynchronous_focus()
        for mode in ["fail", "hang"]:
            results["helpers"].append(unavailable_helper(mode, bundle))
        results["persistent_user_task"] = persistent_user_task_is_preserved(bundle)
    except Exception as error:
        results["error"] = str(error)
        raise
    finally:
        (ROOT / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf8")
        print(json.dumps(results, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
