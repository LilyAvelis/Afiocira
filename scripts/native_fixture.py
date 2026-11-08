"""Clean up only a Popen-created synthetic GUI and its owned child processes."""
import subprocess
import json


def descendant_processes(process):
    """Inventory only descendants of the synthetic process, including wrappers."""
    query = ('Get-CimInstance Win32_Process | Select-Object '
             'ProcessId,ParentProcessId,ExecutablePath,CommandLine | ConvertTo-Json')
    rows = json.loads(subprocess.check_output(
        ["powershell", "-NoProfile", "-Command", query], text=True, encoding="utf8"))
    if isinstance(rows, dict):
        rows = [rows]
    owned = {process.pid}
    descendants = []
    while True:
        children = [row for row in rows if row["ParentProcessId"] in owned
                    and row["ProcessId"] not in owned]
        if not children:
            return descendants
        descendants.extend(children)
        owned.update(row["ProcessId"] for row in children)


def stop_fixture(process):
    if process.poll() is None:
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       capture_output=True, timeout=10, check=False,
                       creationflags=subprocess.CREATE_NO_WINDOW)
        process.wait(timeout=10)
