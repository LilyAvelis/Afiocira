"""Synthetic cache/rotation regression against an already running isolated GUI.
Usage: python scripts/check-cache-continuity.py --pipe NAME --media SYNTHETIC_DIR --out RESULT_DIR
The caller owns GUI lifecycle. This script writes only its named synthetic media/output.
"""
import argparse, json, os, time
from pathlib import Path
from PIL import Image

p = argparse.ArgumentParser()
p.add_argument('--pipe', required=True)
p.add_argument('--media', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--width', type=int, default=640)
p.add_argument('--height', type=int, default=480)
p.add_argument('--rotation-only', action='store_true')
p.add_argument('--allow-blank', action='store_true', help='Measure an older build without asserting continuity')
p.add_argument('--textured', action='store_true', help='Use deterministic high-entropy pixels in half the image')
a = p.parse_args()
a.media.mkdir(parents=True, exist_ok=True)
a.out.mkdir(parents=True, exist_ok=True)
assert not any(a.media.iterdir()), 'Use a new empty synthetic media directory'
def fixture_image(color):
    image = Image.new('RGB', (a.width, a.height), color)
    if a.textured:
        import random
        noise = random.Random(42).randbytes((a.width // 2) * a.height * 3)
        image.paste(Image.frombytes('RGB', (a.width // 2, a.height), noise), (a.width // 2, 0))
    return image

for name, color in [('a.png', (255, 0, 0)), ('b.png', (0, 0, 255))]:
    fixture_image(color).save(a.media/name)

def rpc(command='status', **args):
    with open('\\\\.\\pipe\\'+a.pipe, 'r+b', buffering=0) as pipe:
        pipe.write(json.dumps(dict(command=command, **args)).encode()+b'\n')
        result = b''
        while not result.endswith(b'\n'):
            part = pipe.read(65536)
            if not part: raise RuntimeError('API disconnected')
            result += part
    result = json.loads(result)
    if result.get('error'): raise RuntimeError(result['error'])
    return result

def until(predicate, timeout=15):
    end = time.perf_counter()+timeout
    while time.perf_counter() < end:
        state = rpc()
        if predicate(state): return state
        time.sleep(.005)
    raise AssertionError('Timed out')

def textures(state, key='preview_textures'):
    return {Path(path).name: value for path, value in state['render'].get(key, [])}

def capture(name, rgb):
    path = (a.out/name).resolve()
    rpc('capture_frame', path=str(path), viewport='root')
    end = time.perf_counter()+5
    while not path.exists() and time.perf_counter() < end: time.sleep(.02)
    image = Image.open(path).convert('RGB')
    count = sum(abs(r-rgb[0])+abs(g-rgb[1])+abs(b-rgb[2]) < 20 for r,g,b in image.getdata())
    assert count > 10000, (name, count)
    return count

rpc('open', path=str(a.media.resolve()))
rpc('focus_pane', pane='gallery')
rpc('select_path', path=str((a.media/'a.png').resolve()))
rpc('open_selected')
before = until(lambda s: set(textures(s)) >= {'a.png', 'b.png'})
old = textures(before)
red = capture('before.png', (255,0,0))
replace_ms = None
green = None
after = before
color = (255,0,0)
if not a.rotation_only:
    staged = a.out/'staged.png'
    fixture_image((0,255,0)).save(staged)
    start = time.perf_counter()
    os.replace(staged, a.media/'a.png')
    after = until(lambda s: textures(s).get('a.png') not in (None, old['a.png']))
    replace_ms = (time.perf_counter()-start)*1000
    assert textures(after)['b.png'] == old['b.png'], 'Unrelated preview texture was discarded'
    green = capture('after.png', (0,255,0))
    color = (0,255,0)
start = time.perf_counter()
initial = rpc('run_action', id='rotate_left')
samples = []
while time.perf_counter()-start < 15:
    state = rpc()
    ready = state['render']['preview_ready'] or state['render']['thumbnail_ready']
    fallback = 'a.png' in textures(state, 'fallback_preview_textures')
    samples.append({'ms': (time.perf_counter()-start)*1000, 'ready': ready, 'fallback': fallback})
    if not a.allow_blank:
        assert ready or fallback, 'No current or fallback image available (Loading flash)'
    if state.get('operation') and not state['operation']['running'] and textures(state).get('a.png') not in (None, textures(after)['a.png']): break
    time.sleep(.005)
else: raise AssertionError('Rotation timed out')
assert textures(state)['b.png'] == old['b.png']
rotation_ms = (time.perf_counter()-start)*1000
rotated = Image.open(a.media/'a.png')
assert rotated.size == (a.height,a.width)
capture('rotated.png', color)
result = {'replacement_ms': replace_ms, 'rotation_ready_ms': rotation_ms, 'operation': state.get('operation'), 'samples': samples, 'red_pixels_before': red, 'green_pixels_after': green, 'unrelated_preview_preserved': True, 'limitation': 'API samples establish CPU texture availability, not every presented monitor frame; polling adds latency.'}
(a.out/'results.json').write_text(json.dumps(result, indent=2), encoding='utf8')
print(json.dumps(result, indent=2))
