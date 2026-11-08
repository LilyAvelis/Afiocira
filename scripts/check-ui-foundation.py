"""Check existing Settings pages via native API; report the immediate viewport capture limitation."""
import json
import os
from pathlib import Path
import struct
import subprocess
import time
import uuid
import zlib

project = Path(__file__).resolve().parent.parent
root = project / 'target' / ('ui-foundation-' + uuid.uuid4().hex[:8])
media = root / 'media'
config = root / 'settings'
yazi = root / 'source-yazi'
for directory in (media, config, yazi):
    directory.mkdir(parents=True)
def chunk(kind, value):
    return struct.pack('>I', len(value)) + kind + value + struct.pack('>I', zlib.crc32(kind + value) & 0xffffffff)
(media / 'fixture.png').write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress((b'\0' + bytes([70, 100, 140]) * 2) * 2)) + chunk(b'IEND', b''))
(config / 'settings.json').write_text(json.dumps({'version': 1, 'yazi': {
    'general': {'scrolloff': '5'},
    'quicklinks': [{'id': 'long-link', 'name': 'Очень длинное русское название рабочего альбома для проверки переноса', 'key': 'q', 'path': str(media), 'enabled': True, 'deleted': False}],
    'bindings': [{'id': 'long-binding', 'description': 'Очень длинное описание назначения клавиш файлового менеджера', 'layer': 'mgr', 'on': ['g', 'z'], 'run': ['noop'], 'enabled': True}]
}}, ensure_ascii=False), encoding='utf-8')
env = dict(os.environ, AFIO_API_NAME='afio-ui-' + uuid.uuid4().hex, AFIO_SETTINGS_HOME=str(config), YAZI_CONFIG_HOME=str(yazi))
pipe = '\\\\.\\pipe\\' + env['AFIO_API_NAME']
def rpc(command='status', allow_error=False, **args):
    with open(pipe, 'r+b', buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b'\n')
        data = b''
        while not data.endswith(b'\n'):
            part = stream.read(65536)
            if not part:
                raise RuntimeError('API disconnected')
            data += part
    result = json.loads(data)
    if not allow_error:
        assert not result.get('error'), result
    return result
process = subprocess.Popen([str(project / 'target/release/photo-gui.exe'), str(media)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
try:
    deadline = time.monotonic() + 20
    while True:
        try:
            rpc()
            break
        except OSError:
            assert time.monotonic() < deadline
            time.sleep(.05)
    rpc('show_settings')
    states = []
    for label, width in [('narrow', 680), ('normal', 1100), ('wide', 1600)]:
        rpc('resize_viewport', viewport='settings', width=width, height=860)
        for section, page in [('yazi', 'General'), ('yazi', 'Quicklinks'), ('yazi', 'Keybindings'), ('yazi', 'Appearance'), ('afio', 'Shortcuts')]:
            selected = rpc('select_settings', section=section, page=page)
            state = selected['settings']
            assert state['open'] and state['section'] == section, state
            if section == 'yazi':
                assert state['page'] == page, state
            time.sleep(.1)
            capture = rpc('capture_frame', allow_error=True, viewport='settings', path=str(root / f'{label}-{section}-{page.lower()}.png'))
            assert capture['error'] and capture['error']['code'] == 'invalid_state', capture
            states.append({'width': width, 'size': label, 'section': section, 'page': page, 'settings': state, 'capture_error': capture['error']})
    geometry = subprocess.run(['cargo', 'test', '-p', 'photo-gui', 'ui_foundation::tests', '--', '--nocapture'], cwd=project, capture_output=True, text=True, encoding='utf-8', errors='replace')
    (root / 'geometry-tests.log').write_text(geometry.stdout + geometry.stderr, encoding='utf-8')
    report = {'states': states, 'geometry_tests': {'exit_code': geometry.returncode, 'log': 'geometry-tests.log'}, 'screenshots': [], 'capture_limitation': 'Pinned eframe Glow immediate viewport renderer does not handle Screenshot actions; Settings capture returns explicit InvalidState.', 'subjective_visual_acceptance': 'requires human review; no native visual verification claimed'}
    (root / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    assert geometry.returncode == 0, geometry.stdout + geometry.stderr
    print(root)
finally:
    subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
    process.wait(timeout=10)
