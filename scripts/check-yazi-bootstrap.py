"""Check only fixture-owned bootstrap/leaf lifetimes inside a private ConPTY.

Uses Windows ctypes and the system C# compiler, without Cargo or desktop input.
The console mock leaf records its arguments and waits; no user Yazi is started.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import hashlib
import os
from pathlib import Path
import subprocess
import threading
import time
import uuid


class Coord(c.Structure):
    _fields_ = [("X", c.c_short), ("Y", c.c_short)]


class StartupInfo(c.Structure):
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR),
                ("title", w.LPWSTR)] + [(name, w.DWORD) for name in
                ("x", "y", "width", "height", "chars_x", "chars_y", "fill", "flags")] + [
                ("show", w.WORD), ("reserved_size", w.WORD), ("reserved_bytes", c.c_void_p),
                ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]


class StartupInfoEx(c.Structure):
    _fields_ = [("startup", StartupInfo), ("attributes", c.c_void_p)]


class ProcessInfo(c.Structure):
    _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE),
                ("pid", w.DWORD), ("tid", w.DWORD)]


kernel = c.WinDLL("kernel32", use_last_error=True)


def api(name, result, arguments):
    function = getattr(kernel, name)
    function.restype = result
    function.argtypes = arguments
    return function


close_handle = api("CloseHandle", w.BOOL, [w.HANDLE])
create_pipe = api("CreatePipe", w.BOOL, [c.POINTER(w.HANDLE), c.POINTER(w.HANDLE), c.c_void_p, w.DWORD])
create_pty = api("CreatePseudoConsole", c.c_long, [Coord, w.HANDLE, w.HANDLE, w.DWORD, c.POINTER(w.HANDLE)])
close_pty = api("ClosePseudoConsole", None, [w.HANDLE])
read_file = api("ReadFile", w.BOOL, [w.HANDLE, c.c_void_p, w.DWORD, c.POINTER(w.DWORD), c.c_void_p])
initialize_attributes = api("InitializeProcThreadAttributeList", w.BOOL,
                            [c.c_void_p, w.DWORD, w.DWORD, c.POINTER(c.c_size_t)])
update_attribute = api("UpdateProcThreadAttribute", w.BOOL,
                       [c.c_void_p, w.DWORD, c.c_size_t, c.c_void_p, c.c_size_t, c.c_void_p, c.c_void_p])
delete_attributes = api("DeleteProcThreadAttributeList", None, [c.c_void_p])
create_process = api("CreateProcessW", w.BOOL,
                     [w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL, w.DWORD,
                      c.c_void_p, w.LPCWSTR, c.POINTER(StartupInfo), c.POINTER(ProcessInfo)])
wait_process = api("WaitForSingleObject", w.DWORD, [w.HANDLE, w.DWORD])
terminate_process = api("TerminateProcess", w.BOOL, [w.HANDLE, w.UINT])
open_process = api("OpenProcess", w.HANDLE, [w.DWORD, w.BOOL, w.DWORD])
process_image = api("QueryFullProcessImageNameW", w.BOOL,
                    [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)])


def checked(result):
    if not result:
        raise c.WinError(c.get_last_error())
    return result


class PrivateConsole:
    def __init__(self, executable, arguments, cwd):
        self.console = w.HANDLE()
        self.input = w.HANDLE()
        self.output = w.HANDLE()
        self.process = None
        self.pid = None
        self.reader = None
        self.bytes = bytearray()
        input_read, output_write = w.HANDLE(), w.HANDLE()
        attributes = None
        attributes_initialized = False
        try:
            checked(create_pipe(c.byref(input_read), c.byref(self.input), None, 0))
            checked(create_pipe(c.byref(self.output), c.byref(output_write), None, 0))
            result = create_pty(Coord(80, 24), input_read, output_write, 0, c.byref(self.console))
            if result < 0:
                raise OSError(f"CreatePseudoConsole HRESULT {result:#x}")
            self.reader = threading.Thread(target=self.drain, daemon=True)
            self.reader.start()
            size = c.c_size_t()
            initialize_attributes(None, 1, 0, c.byref(size))
            attributes = c.create_string_buffer(size.value)
            checked(initialize_attributes(attributes, 1, 0, c.byref(size)))
            attributes_initialized = True
            checked(update_attribute(attributes, 0, 0x00020016, self.console,
                                     c.sizeof(w.HANDLE), None, None))
            startup = StartupInfoEx()
            startup.startup.cb = c.sizeof(startup)
            startup.startup.flags = 0x00000100  # STARTF_USESTDHANDLES, matching portable-pty.
            startup.startup.stdin = startup.startup.stdout = startup.startup.stderr = w.HANDLE(-1).value
            startup.attributes = c.cast(attributes, c.c_void_p)
            information = ProcessInfo()
            command = c.create_unicode_buffer(subprocess.list2cmdline([str(executable), *arguments]))
            checked(create_process(str(executable), command, None, None, False,
                                   0x00080000 | 0x00000400, None, str(cwd),
                                   c.byref(startup.startup), c.byref(information)))
            self.process, self.pid = information.process, information.pid
            close_handle(information.thread)
        except BaseException:
            self.close()
            raise
        finally:
            if attributes_initialized:
                delete_attributes(attributes)
            if input_read:
                close_handle(input_read)
            if output_write:
                close_handle(output_write)

    def drain(self):
        buffer = c.create_string_buffer(4096)
        count = w.DWORD()
        while read_file(self.output, buffer, len(buffer), c.byref(count), None) and count.value:
            if len(self.bytes) < 65536:
                self.bytes.extend(buffer.raw[:min(count.value, 65536 - len(self.bytes))])

    def kill(self):
        if self.process and wait_process(self.process, 0) == 258:
            checked(terminate_process(self.process, 99))
            assert wait_process(self.process, 5000) == 0, "Owned bootstrap did not exit"

    def close(self):
        self.kill()
        if self.console:
            close_pty(self.console)
            self.console = w.HANDLE()
        if self.reader:
            self.reader.join(timeout=2)
        for handle in (self.input, self.output, self.process):
            if handle:
                close_handle(handle)
        self.input, self.output, self.process = w.HANDLE(), w.HANDLE(), None


def receipt(path):
    deadline = time.perf_counter() + 8
    while time.perf_counter() < deadline:
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            time.sleep(0.01)
    raise AssertionError("Bootstrap did not publish ack0: " + str(path))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path)
    args = parser.parse_args()
    project = Path(__file__).resolve().parent.parent
    executable = (args.cli or project / "target/release/photo-cli.exe").resolve()
    root = project / "target" / ("bootstrap-lifecycle-" + uuid.uuid4().hex[:8])
    root.mkdir()
    source = root / "Leaf.cs"
    source.write_text('''using System; using System.IO; using System.Diagnostics; using System.Threading;
using System.Runtime.InteropServices; using System.Text;
class Leaf {
[StructLayout(LayoutKind.Explicit, Size=20, CharSet=CharSet.Unicode)]
struct InputRecord { [FieldOffset(0)] public ushort Type; [FieldOffset(14)] public char Character; }
[DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int kind);
[DllImport("kernel32.dll", SetLastError=true)] static extern bool GetNumberOfConsoleInputEvents(IntPtr input, out uint count);
[DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
static extern bool ReadConsoleInputW(IntPtr input, [Out] InputRecord[] records, uint length, out uint read);
static void Main(string[] args) {
File.WriteAllLines("leaf.argv", args);
uint count, read; IntPtr input=GetStdHandle(-10);
if (!GetNumberOfConsoleInputEvents(input, out count)) throw new Exception("Input count failed");
var text=new StringBuilder();
if (count > 0) {
var records=new InputRecord[Math.Min(count,4096)];
if (!ReadConsoleInputW(input, records, (uint)records.Length, out read)) throw new Exception("Input read failed");
for (int i=0;i<read;i++) if (records[i].Type==1 && records[i].Character!=0) text.Append(records[i].Character);
}
File.WriteAllText("leaf.input",text.ToString());
File.WriteAllText("leaf.pid", Process.GetCurrentProcess().Id.ToString());
Thread.Sleep(60000);
} }''', encoding="utf-8")
    leaf = root / "fixture-leaf.exe"
    compiler = Path(os.environ["WINDIR"]) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
    subprocess.run([str(compiler), "/nologo", "/target:exe", "/out:" + str(leaf), str(source)],
                   check=True, timeout=30, capture_output=True,
                   creationflags=subprocess.CREATE_NO_WINDOW)
    results = {"artifact": str(root), "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(), "cases": []}
    try:
        for mode in ("light", "dark", "spawn-error"):
            folder = root / (mode + " Юникод")
            folder.mkdir()
            status = folder / "ack.0"
            client = "fixture client with spaces"
            console = PrivateConsole(executable, ["yazi-bootstrap", "--mode", "light" if mode == "spawn-error" else mode,
                                      "--status", str(status), "--yazi",
                                      str(folder / "missing.exe" if mode == "spawn-error" else leaf),
                                      "--cwd", str(folder), "--client-id", client], folder)
            child = None
            try:
                response = receipt(status)
                assert response["id"] == 0, response
                if mode == "spawn-error":
                    assert response["ok"] is False and response.get("error"), response
                    assert wait_process(console.process, 5000) == 0, "Failed helper did not exit"
                    assert not (folder / "leaf.pid").exists()
                    results["cases"].append({"mode": mode, "receipt": response,
                                             "error_receipt_published": True, "helper_exited": True})
                    continue
                assert response["ok"] is True and response["pid"] != console.pid, response
                # Hold a process handle before killing the helper, avoiding PID reuse.
                child = checked(open_process(0x00100000 | 0x1000 | 0x0001, False, response["pid"]))
                image = c.create_unicode_buffer(32768)
                length = w.DWORD(len(image))
                checked(process_image(child, 0, image, c.byref(length)))
                assert Path(image.value).resolve() == leaf.resolve(), image.value
                deadline = time.perf_counter() + 5
                while not (folder / "leaf.pid").exists() and time.perf_counter() < deadline:
                    time.sleep(0.01)
                assert int((folder / "leaf.pid").read_text(encoding="utf-8")) == response["pid"]
                assert (folder / "leaf.argv").read_text(encoding="utf-8").splitlines() == [str(folder), "--client-id", client]
                color = "fafa" if mode == "light" else "1818"
                expected_report = "\x1b]11;rgb:" + "/".join([color] * 3) + "\x1b\\"
                assert (folder / "leaf.input").read_text(encoding="utf-8") == expected_report, "Unexpected primed console input"
                assert response["console"]["background"] == (0xFAFAFA if mode == "light" else 0x181818)
                assert response["console"]["foreground"] == (0x202020 if mode == "light" else 0xE0E0E0)
                assert response["console"]["attributes"] == 7
                assert wait_process(child, 0) == 258, "Mock leaf exited before lifecycle check"
                console.kill()
                assert wait_process(child, 3000) == 0, "Owned leaf survived bootstrap termination"
                results["cases"].append({"mode": mode, "helper_pid": console.pid,
                                         "receipt": response, "arguments_preserved": True,
                                         "primed_osc11_only": True,
                                         "leaf_terminated_after_helper_kill": True})
            finally:
                console.close()
                if child:
                    # This handle was verified to name our mock leaf, never a user process.
                    if wait_process(child, 0) == 258:
                        terminate_process(child, 99)
                        wait_process(child, 5000)
                    close_handle(child)
                (folder / "pty.log").write_bytes(console.bytes)
        results["passed"] = True
    except BaseException as error:
        results["passed"] = False
        results["error"] = repr(error)
        raise
    finally:
        (root / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        print(json.dumps({"artifact": str(root / "results.json"), "passed": results.get("passed")}))


if __name__ == "__main__":
    main()
