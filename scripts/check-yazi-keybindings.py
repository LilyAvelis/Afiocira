"""Exercise Yazi keybinding sequence/conflict contracts in a private native GUI."""
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

project = Path(__file__).resolve().parent.parent
root = project / 'target' / ('yazi-keybinding-editor-' + uuid.uuid4().hex[:8])
media = root / 'media'
source = root / 'source-yazi'
media.mkdir(parents=True)
source.mkdir()
(media / 'fixture.png').write_bytes((project / 'assets/test-images/1.png').read_bytes())
env = dict(os.environ, AFIO_API_NAME='afio-keys-' + uuid.uuid4().hex, AFIO_SETTINGS_HOME=str(root / 'settings'), YAZI_CONFIG_HOME=str(source))
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
        assert not result['error'], result['error']
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
    rpc('select_settings', section='yazi', page='Keybindings')
    sequence = ['z', '1', '2', '3', '4', '5', '6', '7']
    def save(binding, allow_error=False):
        return rpc('configure_yazi', allow_error=allow_error, request={'op': 'save_binding', 'binding': binding, 'replace': True})
    first = dict(id='sequence-a', layer='mgr', on=sequence, run=['noop'], description='Recorded sequence fixture', enabled=True)
    result = save(first)
    assert next(b for b in result['settings']['yazi']['profile']['bindings'] if b['id'] == first['id'])['on'] == sequence
    conflict = save(dict(first, id='sequence-b'), allow_error=True)
    assert conflict['error'], conflict
    assert len(conflict['settings']['yazi']['profile']['bindings']) == 1
    too_long = save(dict(first, id='sequence-long', on=sequence + ['8']), allow_error=True)
    assert too_long['error'], too_long
    reserved = save(dict(first, id='reserved', on=['<S-Tab>']), allow_error=True)
    assert reserved['error'], reserved
    chord = dict(first, id='chord-a', layer='input', on=['<A-Right>'])
    accepted = save(chord)
    assert len(accepted['settings']['yazi']['profile']['bindings']) == 2
    report = {'eight_key_sequence_staged': True, 'same_layer_conflict_explicit': conflict['error'], 'ninth_key_rejected': too_long['error'], 'reserved_key_rejected': reserved['error'], 'different_layer_chord_staged': True, 'capture_input_verification': 'headless yazi_key_capture::tests; physical keyboard not automated', 'settings_screenshot': 'unsupported immediate viewport renderer; no screenshot claimed'}
    (root / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(root)
finally:
    subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW, timeout=10)
    process.wait(timeout=10)
