"""Isolated native embedded metadata checks; originals are read only.

--source optionally inspects a real JPEG and edits only a copied fixture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid
from PIL import Image, PngImagePlugin
from native_fixture import stop_fixture

parser = argparse.ArgumentParser()
parser.add_argument('--bin-dir', default='target/distribution/release')
parser.add_argument('--source')
args = parser.parse_args()
project = Path(__file__).resolve().parent.parent
root = project / 'target' / ('embedded-' + uuid.uuid4().hex[:8])
media = root / 'media'
media.mkdir(parents=True)
workflow = {'nodes': [{'id': i, 'type': 'KSampler', 'widgets_values': [42, 'fixed', 20, 7, 'synthetic-layout-data-' * 30]} for i in range(600)]}
prompt = {'1': {'class_type': 'KSampler', 'inputs': {'seed': 42, 'steps': 20, 'cfg': 7, 'sampler_name': 'euler'}}}
info = PngImagePlugin.PngInfo()
info.add_text('prompt', json.dumps(prompt))
info.add_itxt('workflow', json.dumps(workflow), zip=True)
info.add_text('Author', 'Synthetic author')
info.add_text('parameters', 'A synthetic landscape\nNegative prompt: blur\nSteps: 20, Sampler: Euler, CFG scale: 7, Seed: 42')
png = media / 'comfy.png'
Image.new('RGB', (48, 32), (80, 110, 130)).save(png, pnginfo=info)
jpeg = media / 'xmp.jpg'
exif = Image.Exif()
exif[0x010f] = 'Synthetic camera'
exif[0x0112] = 1
Image.new('RGB', (48, 32), (80, 110, 130)).save(jpeg, exif=exif)
xml = b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" photoshop:Credit="Made with Google AI"/></rdf:RDF></x:xmpmeta>'
body = b'http://ns.adobe.com/xap/1.0/\0' + xml
segment = b'\xff\xe1' + (len(body) + 2).to_bytes(2, 'big') + body
data = jpeg.read_bytes()
jpeg.write_bytes(data[:2] + segment + data[2:])
external = Path(args.source).resolve() if args.source else None
external_before = hashlib.sha256(external.read_bytes()).hexdigest() if external else None
if external:
    external_copy = media / 'external-copy.jpg'
    shutil.copyfile(external, external_copy)
env = dict(os.environ, AFIO_API_NAME='afio-embedded-' + uuid.uuid4().hex,
           AFIO_DATA_HOME=str(root / 'data'), AFIO_SETTINGS_HOME=str(root / 'settings'),
           APPDATA=str(root / 'appdata'), LOCALAPPDATA=str(root / 'localappdata'),
           TEMP=str(root / 'temp'), TMP=str(root / 'temp'))
(root / 'temp').mkdir()
pipe = '\\\\.\\pipe\\' + env['AFIO_API_NAME']

def rpc(command='status', **kwargs):
    with open(pipe, 'r+b', buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **kwargs)).encode() + b'\n')
        payload = b''
        while not payload.endswith(b'\n'):
            part = stream.read(65536)
            if not part:
                raise RuntimeError('Native API disconnected')
            payload += part
    state = json.loads(payload)
    assert not state['error'], state['error']
    return state

def wait(predicate, timeout=30):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            state = rpc()
            if predicate(state):
                return state
        except OSError:
            pass
        time.sleep(.05)
    raise AssertionError('Timed out waiting for native state')

def inspect(path):
    rpc('open', path=str(path))
    rpc('show_metadata')
    state = wait(lambda s: s['metadata_panel']['open'] and not s['metadata_panel']['pending'])
    assert not state['metadata_panel']['error'], state
    embedded = state['metadata_panel']['embedded']
    assert embedded
    return embedded

def capture(name):
    output = root / (name + '.png')
    rpc('capture_frame', path=str(output))
    end = time.monotonic() + 10
    while time.monotonic() < end:
        try:
            with Image.open(output) as frame:
                frame.load()
            return str(output)
        except (OSError, ValueError):
            time.sleep(.05)
    raise AssertionError('Capture not ready')

def edit_copy(path):
    rpc('open', path=str(path))
    rpc('show_adjustments')
    rpc('preview_adjustments', options={'exposure': .4})
    state = wait(lambda s: s.get('adjustments_preview') and not s['adjustments_preview']['pending'])
    assert not state['adjustments_preview']['error'], state['adjustments_preview']
    rpc('apply_adjustments')
    state = wait(lambda s: not (s.get('operation') or {}).get('running', False))
    assert all(not f.get('error') for f in state['operation']['files']), state

report = {}
log = (root / 'native.log').open('wb')
process = subprocess.Popen([str(project / args.bin_dir / 'photo-gui.exe'), str(media)], env=env,
                           stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW)
try:
    wait(lambda s: s.get('snapshot') is not None)
    before = png.read_bytes()
    result = inspect(png)
    assert any(p['namespace'] == 'comfyui' and p['value'] == '42' for p in result['properties'])
    assert any(p['namespace'] == 'png_text' and p['value'] == 'Synthetic author' for p in result['properties'])
    assert any(json.loads(p['text']) == workflow for p in result['raw_payloads'] if p['label'] == 'PNG workflow (JSON)')
    assert len([p for p in result['properties'] if p['namespace'] == 'comfyui']) < 20, 'Workflow was expanded into hundreds of display fields'
    report['root_frames'] = []
    for width in [680, 1100, 1600]:
        rpc('resize_viewport', width=width, height=850)
        time.sleep(.15)
        report['root_frames'].append(capture('metadata-' + str(width)))
    rpc('close_metadata')
    assert png.read_bytes() == before and not (root / 'data').exists()
    rpc('show_metadata')
    rpc('cancel_metadata')
    state = wait(lambda s: not s['metadata_panel']['open'])
    assert state['navigator']['focus'] == 'gallery'
    time.sleep(.15)
    assert not rpc()['metadata_panel']['open'], 'Late metadata receipt reopened the canceled panel'
    assert not (root / 'data').exists()
    assert rpc('set_copy_notifications', enabled=False)['settings']['copy_notifications'] is False
    prefs = json.loads((root / 'settings' / 'settings.json').read_text(encoding='utf8'))
    assert prefs['copy_notifications'] is False
    assert rpc('set_copy_notifications', enabled=True)['settings']['copy_notifications'] is True
    report['cancel_metadata_releases_panel_and_copy_preference_persists'] = True
    result = inspect(jpeg)
    assert any(p['value'] == 'Made with Google AI' for p in result['properties'])
    assert any(p['namespace'] == 'EXIF' for p in result['properties'])
    rpc('close_metadata')
    assert not (root / 'data').exists(), 'Read-only inspection initialized storage'
    edit_copy(jpeg)
    result = inspect(jpeg)
    assert result['raw_xmp'] == [xml.decode()]
    rpc('close_metadata')
    edit_copy(png)
    result = inspect(png)
    assert any(json.loads(p['text']) == workflow for p in result['raw_payloads'] if p['label'] == 'PNG workflow (JSON)')
    rpc('close_metadata')
    report.update(read_only_sparse_inspection=True, comfy_workflow_preserved=True, jpeg_xmp_edit_preserved=True)
    if external:
        result = inspect(external_copy)
        assert result['raw_xmp'], 'Expected XMP in the supplied example'
        xmp_before = result['raw_xmp']
        rpc('close_metadata')
        edit_copy(external_copy)
        result = inspect(external_copy)
        assert result['raw_xmp'] == xmp_before
        assert hashlib.sha256(external.read_bytes()).hexdigest() == external_before
        report['supplied_example_copy_edit_preserves_xmp_original_unchanged'] = True
    (root / 'results.json').write_text(json.dumps(report, indent=2), encoding='utf8')
    print(root)
finally:
    stop_fixture(process)
    log.close()
    if external:
        assert hashlib.sha256(external.read_bytes()).hexdigest() == external_before
