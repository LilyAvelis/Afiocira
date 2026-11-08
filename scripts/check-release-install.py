"""Install/uninstall only an isolated TESTMODE release and check its launcher."""
import ctypes
from ctypes import wintypes
import json
import msvcrt
import os
from pathlib import Path
import subprocess
import time
import uuid

PROJECT = Path(__file__).resolve().parents[1]
ROOT = PROJECT / 'target' / ('installer-25-' + uuid.uuid4().hex[:8])
ROOT.mkdir(parents=True)
INSTALL = ROOT / 'installed'
SETUP = PROJECT / 'target/release-assets/Afiocira-2.5.0-Setup-Windows-x64.exe'
PIPE_NAME = 'afio-install-' + uuid.uuid4().hex
PIPE = '\\\\.\\pipe\\' + PIPE_NAME
flags = subprocess.CREATE_NO_WINDOW
kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.GetNamedPipeServerProcessId.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.ULONG)]
kernel.GetNamedPipeServerProcessId.restype = wintypes.BOOL
kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel.OpenProcess.restype = wintypes.HANDLE
kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
pid = None


def rpc(command='status', **args):
    global pid
    with open(PIPE, 'r+b', buffering=0) as stream:
        server = wintypes.ULONG()
        assert kernel.GetNamedPipeServerProcessId(msvcrt.get_osfhandle(stream.fileno()), ctypes.byref(server))
        if pid is None:
            handle = kernel.OpenProcess(0x1000, False, server.value)
            assert handle
            try:
                image = ctypes.create_unicode_buffer(32768)
                size = wintypes.DWORD(len(image))
                assert kernel.QueryFullProcessImageNameW(handle, 0, image, ctypes.byref(size))
                assert Path(image.value).resolve() == (INSTALL / 'releases/2.5.0/Afiocira.exe').resolve()
                pid = server.value
            finally:
                kernel.CloseHandle(handle)
        assert pid == server.value
        stream.write(json.dumps(dict(command=command, **args)).encode() + b'\n')
        data = b''
        while not data.endswith(b'\n'):
            chunk = stream.read(65536)
            if not chunk:
                raise RuntimeError('Installed fixture API disconnected')
            data += chunk
    result = json.loads(data)
    assert not result['error'], result['error']
    return result


def wait(predicate, timeout=18):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = rpc()
            if predicate(last):
                return last
        except OSError:
            pass
        time.sleep(.04)
    raise AssertionError(last)


result = {}
try:
    installed = subprocess.run([str(SETUP), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
                                '/TESTMODE=1', '/TASKS=', '/DIR=' + str(INSTALL), '/LOG=' + str(ROOT/'install.log')],
                               creationflags=flags, timeout=90, check=False)
    assert installed.returncode == 0, installed.returncode
    release = INSTALL / 'releases/2.5.0'
    assert (INSTALL/'current-version.txt').read_text().strip() == 'releases\\2.5.0\\Afiocira.exe'
    assert not (release/'libmpv-2.dll').exists() and not (release/'ffmpeg.exe').exists()
    listing = subprocess.check_output([str(release/'photo-cli.exe'), 'list', str(release/'test-images')],
                                      text=True, encoding='utf8', creationflags=flags)
    assert len(listing.splitlines()) == 3
    env = dict(os.environ, AFIO_API_NAME=PIPE_NAME, AFIO_SETTINGS_HOME=str(ROOT/'settings'),
               APPDATA=str(ROOT/'appdata'), LOCALAPPDATA=str(ROOT/'localappdata'),
               YAZI_CONFIG_HOME=str(ROOT/'yazi-base'), PATH=str(Path(os.environ['SystemRoot'])/'System32'),
               WINDIR=str(ROOT/'empty-windows'))
    for key in ['APPDATA', 'LOCALAPPDATA', 'YAZI_CONFIG_HOME', 'WINDIR']:
        Path(env[key]).mkdir(parents=True, exist_ok=True)
    subprocess.run([str(INSTALL/'AfioLauncher.exe'), str(release/'test-images')], env=env,
                   creationflags=flags, timeout=10, check=True)
    status = wait(lambda r: r.get('snapshot') is not None and not r['navigator']['pending'])
    assert status['snapshot']['count'] == 3, status['snapshot']
    assert rpc('navigator_snapshot')['navigator']['runtime'] is not None
    rpc('show_settings')
    rpc('select_settings', section='afio', page='Shortcuts')
    assert rpc('set_gallery_auto_size', enabled=False)['settings']['gallery_auto_size'] is False
    rpc('close_settings')
    rpc('configure_yazi', request={'op':'apply_theme_mode', 'mode':'light'})
    wait(lambda r: not r['settings']['yazi']['busy'])
    snapshot = rpc('navigator_snapshot')
    assert snapshot['navigator']['runtime']['metadata']['light'] is True
    result.update(installer_exit=0, installed_cli_lists_three=True, launcher_resolves_25=True,
                  own_server_pid=pid, settings_api=True, native_light_mode=True,
                  video_download_skipped=True)
finally:
    if pid is not None:
        subprocess.run(['taskkill', '/PID', str(pid), '/T', '/F'], creationflags=flags,
                       capture_output=True, timeout=10, check=False)
    uninstaller = INSTALL/'unins000.exe'
    if uninstaller.exists():
        uninstall = subprocess.run([str(uninstaller), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
                                    '/LOG='+str(ROOT/'uninstall.log')], creationflags=flags, timeout=90, check=False)
        assert uninstall.returncode == 0
        deadline = time.monotonic() + 15
        while (INSTALL/'AfioLauncher.exe').exists() and time.monotonic() < deadline:
            time.sleep(.1)
        assert not (INSTALL/'AfioLauncher.exe').exists(), 'Test installation launcher survived uninstall'
        result['test_uninstall'] = True
    (ROOT/'results.json').write_text(json.dumps(result, indent=2), encoding='utf8')
    print(json.dumps(dict(artifact=str(ROOT/'results.json'), **result)))
