"""Readonly flavor + two isolated native sessions; synthetic assets/config only."""
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time
import uuid
from native_fixture import descendant_processes

project = Path(__file__).resolve().parent.parent
root = project / "target" / ("theme-instances-" + uuid.uuid4().hex[:8])
base = root / "base"
media = root / "media"
media.mkdir(parents=True)
base.mkdir()
(media / "test.png").write_bytes((project / "assets/test-images/1.png").read_bytes())
(base / "yazi.toml").write_text('[mgr]\nshow_hidden=false\n', encoding="utf8")
for name, symbol in [("first", "!"), ("second", "#")]:
    folder = base / "flavors" / (name + ".yazi")
    folder.mkdir(parents=True)
    files = {
        "flavor.toml": '[mgr]\nborder_symbol="' + symbol + '"\n',
        "tmtheme.xml": '<?xml version="1.0"?><plist version="1.0"><dict><key>name</key><string>Test</string><key>settings</key><array/></dict></plist>',
        "LICENSE": "MIT License\nSynthetic fixture",
        "LICENSE-tmtheme": "MIT License\nSynthetic fixture",
    }
    for name, contents in files.items():
        path = folder / name
        path.write_text(contents, encoding="utf8")
        path.chmod(stat.S_IREAD)
originals = {p: (p.read_bytes(), p.stat().st_file_attributes) for p in base.rglob('*') if p.is_file()}


class Session:
    def __init__(self):
        self.env = dict(os.environ, AFIO_YAZI_TRACE_PTY="1", AFIO_API_NAME="afio-theme-isolated-" + uuid.uuid4().hex,
                        AFIO_SETTINGS_HOME=str(root / "settings"), YAZI_CONFIG_HOME=str(base))
        self.pipe = "\\\\.\\pipe\\" + self.env["AFIO_API_NAME"]
        self.process = subprocess.Popen([str(project / "target/release/photo-gui.exe"), str(media)],
                                        env=self.env, creationflags=subprocess.CREATE_NO_WINDOW)
        self.wait(lambda r: r["snapshot"] is not None)
        self.config = Path(self.cfg("inspect")["settings"]["yazi"]["config_directory"])
        # Read only this synthetic GUI's children to obtain its actual DDS client ID.
        children = descendant_processes(self.process)
        child = next((c for c in children if c["ExecutablePath"] and Path(c["ExecutablePath"]).name.lower() == "yazi.exe"), None)
        if child is None:
            trace = self.config.parent / "pty.log"
            (root / "startup-pty.log").write_bytes(trace.read_bytes() if trace.exists() else b"No PTY trace")
            self.close()
            raise AssertionError("Yazi child exited during startup; " + str(root / "startup-pty.log"))
        import re
        self.client = re.search(r'--client-id\s+"?(\d+)', child["CommandLine"]).group(1)
        self.ya = str(Path(child["ExecutablePath"]).with_name("ya.exe"))

    def rpc(self, command="status", **args):
        with open(self.pipe, "r+b", buffering=0) as stream:
            stream.write(json.dumps(dict(command=command, **args)).encode() + b"\n")
            data = b""
            while not data.endswith(b"\n"):
                part = stream.read(65536)
                if not part:
                    raise RuntimeError("Native API disconnected")
                data += part
        response = json.loads(data)
        assert not response["error"], response["error"]
        return response

    def wait(self, predicate, timeout=20):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                last = self.rpc()
                if predicate(last):
                    return last
            except OSError:
                pass
            time.sleep(.08)
        raise AssertionError(last)

    def cfg(self, op, **args):
        return self.rpc("configure_yazi", request=dict(op=op, **args))

    def fingerprint(self):
        # Use the same fresh, acknowledged request as GUI handoff; never inject
        # legacy mailbox files or connect directly to the global DDS listener.
        response = self.rpc("navigator_snapshot")
        return response["navigator"]["runtime"]["metadata"]["theme"]

    def apply(self, name):
        self.cfg("apply_appearance", dark=name, light=name, icons="minimal")
        return self.wait(lambda r: not r["settings"]["yazi"]["busy"])["settings"]["yazi"]

    def close(self):
        from native_fixture import stop_fixture
        stop_fixture(self.process)


sessions = []
try:
    sessions.append(Session())
    sessions.append(Session())
    first, second = sessions
    assert first.config != second.config
    before_first, before_second = first.fingerprint(), second.fingerprint()
    second_theme = (second.config / "theme.toml").read_bytes() if (second.config / "theme.toml").exists() else None
    second_profile = second.cfg("inspect")["settings"]["yazi"]["profile"]
    applied = first.apply("first")
    if "--expect-readonly-error" in sys.argv:
        assert applied["error"] and ("os error 5" in applied["error"] or "Access" in applied["error"]), applied
        assert first.fingerprint() == before_first
        result = dict(reproduced_readonly_error=True, error=applied["error"])
    else:
        assert not applied["error"], applied["error"]
        assert Path(applied["config_directory"]) == first.config
        assert first.fingerprint() != before_first
        assert first.fingerprint()[-1] == "!"
        again = first.apply("second")
        assert not again["error"], again["error"]
        assert first.fingerprint()[-1] == "#"
        before_general = first.rpc()
        first.cfg("set_general", key="show_hidden", value="true")
        first.cfg("apply")
        general = first.wait(lambda r: not r["settings"]["yazi"]["busy"])
        assert not general["settings"]["yazi"]["error"], general["settings"]["yazi"]["error"]
        replacement = Path(general["settings"]["yazi"]["config_directory"])
        assert replacement != first.config
        assert __import__("tomllib").loads((replacement / "yazi.toml").read_text(encoding="utf8"))["mgr"]["show_hidden"] is True
        assert general["session_id"] == before_general["session_id"]
        result = dict(readonly_flavor_applied=True, repeated_hot_switch=True, own_runtime_fingerprint_changed=True,
                      general_apply_with_readonly_source=True, general_replacement_same_afio_session=True)
    assert second.fingerprint() == before_second
    assert second.cfg("inspect")["settings"]["yazi"]["profile"] == second_profile
    assert Path(second.cfg("inspect")["settings"]["yazi"]["config_directory"]) == second.config
    assert ((second.config / "theme.toml").read_bytes() if (second.config / "theme.toml").exists() else None) == second_theme
    for path, original in originals.items():
        assert (path.read_bytes(), path.stat().st_file_attributes) == original
    result.update(second_session_unchanged=True, source_contents_and_attributes_preserved=True,
                  private_runtime_directories=True, shared_settings_home=True)
    (root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf8")
    print(json.dumps(result))
finally:
    for session in sessions:
        session.close()
