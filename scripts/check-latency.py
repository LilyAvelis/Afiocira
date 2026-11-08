"""Compare same synthetic rotation workloads; never touch the user's GUI/media."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid
from native_fixture import stop_fixture

parser = argparse.ArgumentParser()
parser.add_argument('--baseline', type=Path)
parser.add_argument('--textured', action='store_true')
parser.add_argument('--rotation-only', action='store_true', help='Compare the identical initial source bytes in both builds')
args = parser.parse_args()
project = Path(__file__).resolve().parent.parent
root = project / 'target' / ('latency-' + uuid.uuid4().hex[:8])
root.mkdir()
results = {}
executables = [('current', project / 'target/release/photo-gui.exe')]
if args.baseline:
    executables.insert(0, ('baseline', args.baseline.resolve()))
for label, executable in executables:
    fixture = root / label
    media = fixture / 'media'
    media.mkdir(parents=True)
    base = fixture / 'base'
    base.mkdir()
    (base / 'yazi.toml').write_text('[mgr]\nshow_hidden=false\n', encoding='utf8')
    name = 'afio-latency-' + uuid.uuid4().hex
    env = dict(os.environ, AFIO_API_NAME=name, AFIO_SETTINGS_HOME=str(fixture / 'settings'), YAZI_CONFIG_HOME=str(base))
    process = subprocess.Popen([str(executable), str(media)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        deadline = time.monotonic() + 15
        while True:
            try:
                with open('\\\\.\\pipe\\' + name, 'r+b', buffering=0) as pipe:
                    pipe.write(b'{"command":"status"}\n')
                    response = b''
                    while not response.endswith(b'\n'):
                        response += pipe.read(65536)
                    if json.loads(response).get('snapshot'):
                        break
            except OSError:
                pass
            assert time.monotonic() < deadline, 'Native fixture did not start'
            time.sleep(.05)
        command = [sys.executable, str(project / 'scripts/check-cache-continuity.py'), '--pipe', name,
                   '--media', str(media), '--out', str(fixture / 'result'), '--width', '3840', '--height', '2160']
        if label == 'baseline':
            command += ['--rotation-only', '--allow-blank']
        elif args.rotation_only:
            command += ['--rotation-only']
        if args.textured:
            command += ['--textured']
        completed = subprocess.run(command, capture_output=True, text=True, timeout=90)
        (fixture / 'run.log').write_text(completed.stdout + completed.stderr, encoding='utf8')
        assert completed.returncode == 0, (fixture / 'run.log').read_text(encoding='utf8')
        result = json.loads((fixture / 'result/results.json').read_text(encoding='utf8'))
        results[label] = {key: result[key] for key in ['replacement_ms', 'rotation_ready_ms', 'operation', 'limitation']}
        results[label]['blank_samples'] = sum(not s['ready'] and not s['fallback'] for s in result['samples'])
        results[label]['samples'] = len(result['samples'])
    finally:
        stop_fixture(process)
(root / 'summary.json').write_text(json.dumps(results, indent=2), encoding='utf8')
print(json.dumps(dict(artifact=str(root / 'summary.json'), **results), ensure_ascii=False))
