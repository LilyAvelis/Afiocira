"""Isolated native contracts for metadata, session history and adjustments.

Run after building target/release/photo-gui.exe. Requires Pillow for pixel checks.
Artifacts and the JSON report remain under target/edit-panels-<id>.
"""
import json
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import time
import uuid
from PIL import Image, ImageChops, ImageStat
from native_fixture import stop_fixture

project = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument('--bin-dir', default='target/debug')
args = parser.parse_args()
bins = project / args.bin_dir
root = project / 'target' / ('edit-panels-' + uuid.uuid4().hex[:8])
media = root / 'media'
media.mkdir(parents=True)
png = media / 'fixture.png'
# Nonuniform, nonsaturated pixels make exposure and rotation observable.
image = Image.new('RGB', (48, 32))
image.putdata([(30 + x * 2, 25 + y * 3, 45 + (x + y) % 90)
               for y in range(32) for x in range(48)])
image.save(png)
original = png.read_bytes()
data = root / 'data'
env = dict(os.environ, AFIO_API_NAME='afio-edit-' + uuid.uuid4().hex,
           AFIO_DATA_HOME=str(data), AFIO_SETTINGS_HOME=str(root / 'settings'),
           APPDATA=str(root / 'appdata'), LOCALAPPDATA=str(root / 'localappdata'),
           TEMP=str(root / 'temp'), TMP=str(root / 'temp'))
(root / 'temp').mkdir()
# Reproduce preferences written by the old physical-modifier recorder. These
# inactive bindings must not block saving a different command's valid shortcut.
(root / 'settings').mkdir()
(root / 'settings' / 'settings.json').write_text(json.dumps(dict(version=1,
    shortcuts=dict(space=dict(rename='Alt AltRight')))), encoding='utf8')
pipe = '\\\\.\\pipe\\' + env['AFIO_API_NAME']


def rpc(command='status', allow_error=False, **args):
    with open(pipe, 'r+b', buffering=0) as stream:
        stream.write(json.dumps(dict(command=command, **args)).encode() + b'\n')
        payload = b''
        while not payload.endswith(b'\n'):
            part = stream.read(65536)
            if not part:
                raise RuntimeError('Native API disconnected')
            payload += part
    result = json.loads(payload)
    if not allow_error:
        assert not result['error'], result
    return result


def wait_for(predicate, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        state = rpc()
        if predicate(state):
            return state
        assert time.monotonic() < deadline, state
        time.sleep(.05)


def idle():
    state = wait_for(lambda s: not (s.get('operation') or {}).get('running', False))
    for result in (state.get('operation') or {}).get('files', []):
        assert not result.get('error'), result
    return state


def metadata_ready():
    return wait_for(lambda s: s.get('metadata_panel') and
                    not s['metadata_panel']['pending'])


def inspect(path):
    job = rpc('metadata', request=dict(op='inspect', path=str(path)))['metadata']
    while job['pending']:
        time.sleep(.02)
        job = rpc('metadata_status', request_id=job['request_id'])['metadata']
    assert not job['error'], job
    return job['snapshot']


def history():
    rpc('show_history')
    idle()
    state = wait_for(lambda s: s.get('history') and
                    s['history'].get('snapshot') is not None)
    thumbnails_idle(len(state['history']['snapshot']['versions']))
    return rpc()['history']['snapshot']


def thumbnails_idle(count=2):
    # Only visible rows are decoded. Longer histories need not fit every card
    # into the viewport (and must not trigger an eager full-stack scan).
    visible_count = min(count, 3)
    return wait_for(lambda s: len((s.get('history') or {}).get('thumbnail_indices', [])) >= visible_count
                    and not (s.get('operation') or {}).get('running', False))


def preview_ready():
    state = wait_for(lambda s: s.get('adjustments_preview') and
                    not s['adjustments_preview']['pending'])
    preview = state['adjustments_preview']
    assert not preview['error'], preview
    assert preview['ready_revision'] == preview['requested_revision'], preview
    assert png.read_bytes() == original, 'Preview wrote source bytes'
    view = (state.get('history') or {}).get('snapshot')
    assert view is None or len(view['versions']) == 1, view
    return preview


def capture_frame(name):
    path = root / f'{name}.png'
    rpc('capture_frame', path=str(path))
    deadline = time.monotonic() + 10
    while True:
        try:
            with Image.open(path) as frame:
                frame.load()
            return path
        except (OSError, ValueError):
            assert time.monotonic() < deadline, path
            time.sleep(.05)


def central_preview_sample(path):
    with Image.open(path) as frame:
        # Left canvas centre, away from the right editor and toolbar/footer.
        return frame.convert('RGB').crop((int(frame.width * .18), int(frame.height * .35),
                                          int(frame.width * .44), int(frame.height * .65)))


def pixels(path):
    with Image.open(path) as value:
        return value.convert('RGBA').size, value.convert('RGBA').tobytes()


def capture_sizes(surface):
    """Capture native root frames; geometry/appearance require image review."""
    captures = []
    for label, width in [('narrow', 680), ('normal', 1100), ('wide', 1600)]:
        rpc('resize_viewport', width=width, height=800)
        time.sleep(.2)
        path = root / f'{surface}-{label}.png'
        rpc('capture_frame', path=str(path))
        deadline = time.monotonic() + 10
        while True:
            try:
                with Image.open(path) as frame:
                    frame.load()
                    assert frame.width > 0 and frame.height > 0
                break
            except (OSError, ValueError):
                assert time.monotonic() < deadline, path
                time.sleep(.05)
        captures.append(str(path))
    rpc('resize_viewport', width=1100, height=800)
    return captures


native_log = (root / 'native-gui.log').open('wb')
process = subprocess.Popen([str(bins / 'photo-gui.exe'), str(media)],
                           env=env, stdout=native_log, stderr=subprocess.STDOUT,
                           creationflags=subprocess.CREATE_NO_WINDOW)
report = {}
try:
    deadline = time.monotonic() + 30
    while True:
        try:
            rpc()
            break
        except OSError:
            assert time.monotonic() < deadline
            time.sleep(.05)
    rpc('select', index=0)
    rpc('show_metadata')
    state = metadata_ready()
    assert state['metadata_panel']['metadata'] is not None, state
    rpc('run_action', id='metadata')
    assert not rpc()['metadata_panel']['open']
    assert not data.exists(), 'Opening metadata must not register or initialize storage'
    report['metadata_inspect_lazy'] = True
    rpc('show_metadata')
    metadata_ready()
    document = dict(tags=['fixture', 'русский тег'], comment='Проверка метаданных',
                    description='', caption='', prompt='')
    rpc('submit_metadata', metadata=document)
    state = metadata_ready()
    assert not state['metadata_panel']['dirty'], state
    saved = inspect(png)
    assert saved['metadata'] == document and saved['image_id'], saved
    image_id = saved['image_id']
    report['metadata_root_frames'] = capture_sizes('metadata')
    rpc('close_metadata')
    assert rpc('show_history', allow_error=True)['error'], 'Gallery history must be refused'
    report['gallery_history_refused'] = True
    rpc('open_selected')
    before_dirs = {str(p) for p in root.rglob('*') if p.is_dir() and p.name.startswith('photo-history-')}
    empty = history()
    assert not empty['versions'], empty
    after_dirs = {str(p) for p in root.rglob('*') if p.is_dir() and p.name.startswith('photo-history-')}
    assert before_dirs == after_dirs, 'Empty history initialized session directory'
    rpc('close_history')
    report['empty_history_lazy'] = True
    rpc('run_action', id='convert_image')
    rpc('submit_image', options=dict(format='webp', quality=90))
    idle()
    webp = png.with_suffix('.webp')
    assert webp.exists() and not png.exists()
    converted = webp.read_bytes()
    view = history()
    assert len(view['versions']) == 2 and view['cursor'] == 1, view
    report['history_root_frames'] = capture_sizes('history')
    thumbnails_idle()
    rpc('preview_history', index=0)
    idle()
    wait_for(lambda s: s.get('history', {}).get('preview_ready'))
    thumbnails_idle()
    rpc('undo_history')
    idle()
    thumbnails_idle()
    assert png.read_bytes() == original and not webp.exists()
    thumbnails_idle()
    rpc('redo_history')
    idle()
    thumbnails_idle()
    assert webp.read_bytes() == converted and not png.exists()
    assert inspect(webp)['image_id'] == image_id
    report['conversion_undo_redo_exact'] = True
    thumbnails_idle()
    rpc('undo_history')
    idle()
    thumbnails_idle()
    rpc('commit_history')
    view = history()
    assert len(view['versions']) == 1 and not view['can_redo'], view
    report['commit_truncates_forward_branch'] = True
    rpc('close_history')
    before_pixels = pixels(png)
    report['preview_source_sha256'] = hashlib.sha256(original).hexdigest()
    rpc('set_navigator_visible', visible=False)
    rpc('show_adjustments')
    baseline_revision = preview_ready()['ready_revision']
    report['adjustments_root_frames'] = capture_sizes('adjustments')
    time.sleep(.2)
    baseline_frame = capture_frame('adjustments-baseline')
    rpc('preview_adjustments', options=dict(exposure=1.0))
    first_revision = preview_ready()['ready_revision']
    assert first_revision > baseline_revision
    exposure_frame = capture_frame('adjustments-exposure-1ev')
    base_sample = central_preview_sample(baseline_frame)
    exposed_sample = central_preview_sample(exposure_frame)
    difference = ImageStat.Stat(ImageChops.difference(base_sample, exposed_sample)).mean
    brightness_before = sum(ImageStat.Stat(base_sample).mean) / 3
    brightness_after = sum(ImageStat.Stat(exposed_sample).mean) / 3
    assert max(difference) > 5 and brightness_after > brightness_before + 5, (difference, brightness_before, brightness_after)
    rpc('preview_adjustments', options=dict(exposure=2.0))
    second_revision = preview_ready()['ready_revision']
    assert second_revision > first_revision
    rpc('preview_adjustments', options=dict(exposure=1.0))
    third_revision = preview_ready()['ready_revision']
    assert third_revision > second_revision
    rpc('cancel_adjustments')
    assert not rpc()['adjustments_open'] and png.read_bytes() == original
    rpc('show_adjustments')
    preview_ready()
    reopened_frame = capture_frame('adjustments-reopened-original')
    reopened_sample = central_preview_sample(reopened_frame)
    assert max(ImageStat.Stat(ImageChops.difference(base_sample, reopened_sample)).mean) < 1
    assert png.read_bytes() == original
    report['preview_revisions'] = [baseline_revision, first_revision, second_revision, third_revision]
    report['preview_central_brightness'] = [brightness_before, brightness_after]
    report['preview_no_writes_or_history_cancel_restores_original'] = True
    assert rpc('show_history', allow_error=True)['error'], 'History must not open under an edit dialog'
    assert rpc('show_metadata', allow_error=True)['error'], 'Metadata must not open under an edit dialog'
    rpc('preview_adjustments', options=dict(exposure=1.0))
    preview_ready()
    rpc('apply_adjustments')
    assert not rpc()['adjustments_open']
    idle()
    assert pixels(png) != before_pixels, 'Exposure did not change decoded pixels'
    view = history()
    assert len(view['versions']) == 2, view
    assert rpc()['history']['preview_index'] is None, 'Thumbnail preload selected a snapshot'
    working_bytes = png.read_bytes()
    rpc('preview_history', index=0)
    state = wait_for(lambda s: s.get('history', {}).get('preview_ready'))
    assert state['history']['preview_index'] == 0
    assert state['history']['snapshot']['cursor'] == 1 and png.read_bytes() == working_bytes
    history_frame = capture_frame('history-central-preview')
    history_brightness = sum(ImageStat.Stat(central_preview_sample(history_frame)).mean) / 3
    assert history_brightness < brightness_after - 5, (history_brightness, brightness_after)
    report['history_canvas_preview_without_moving_stack'] = True
    rpc('close_history')
    rpc('show_adjustments')
    rpc('cancel_adjustments')
    assert not rpc()['adjustments_open']
    report['adjustments_recorded_and_pixels_changed'] = True
    rpc('run_action', id='rotate_right')
    idle()
    view = history()
    assert len(view['versions']) == 2, view
    assert inspect(png)['image_id'] == image_id
    report['rotation_excluded_and_metadata_identity_preserved'] = True
    rpc('close_history')
    curve_source = png.read_bytes()
    curve_size, curve_before = pixels(png)
    rpc('show_adjustments')
    rpc('preview_adjustments', options=dict(curve=[dict(x=0., y=0.), dict(x=.1, y=.4), dict(x=1., y=1.)]))
    wait_for(lambda s: s.get('adjustments_preview') and not s['adjustments_preview']['pending'] and not s['adjustments_preview']['error'])
    assert png.read_bytes() == curve_source
    (root / 'curve-preview.json').write_text(json.dumps(rpc()['adjustments_preview'], ensure_ascii=False, indent=2),encoding='utf8')
    rpc('apply_adjustments')
    idle()
    curve_after_size, curve_after = pixels(png)
    assert curve_size == curve_after_size
    for index, value in enumerate(curve_before):
        if index % 4 == 3:
            assert curve_after[index] == value
        else:
            v = value / 255.
            expected = int((v * 4 if v <= .1 else .4 + (v - .1) / .9 * .6) * 255 + .5)
            assert abs(curve_after[index] - expected) <= 1, (index, value, curve_after[index], expected)
    report['apply_uses_exact_ready_curve'] = True
    falling_source = png.read_bytes()
    rpc('show_adjustments')
    rpc('preview_adjustments', options=dict(curve=[dict(x=0., y=0.), dict(x=.25, y=.56), dict(x=.5, y=.63), dict(x=.75, y=.61), dict(x=1., y=1.)]))
    wait_for(lambda s: s.get('adjustments_preview') and not s['adjustments_preview']['pending'] and not s['adjustments_preview']['error'])
    assert png.read_bytes() == falling_source
    rpc('apply_adjustments')
    idle()
    assert png.read_bytes() != falling_source
    report['screenshot_falling_curve_saves'] = True
    controls_source = png.read_bytes()
    rpc('show_adjustments')
    rpc('preview_adjustments', options=dict(brightness=.2, black_point=.15, brilliance=.25,
        saturation=.3, vibrance=.2, sharpness=.4, definition=.3, noise_reduction=.2, vignette=.25))
    wait_for(lambda s: s.get('adjustments_preview') and not s['adjustments_preview']['pending'] and not s['adjustments_preview']['error'])
    assert png.read_bytes() == controls_source
    report['expanded_controls_root_frames'] = capture_sizes('expanded-controls')
    rpc('apply_adjustments')
    idle()
    assert png.read_bytes() != controls_source
    view = history()
    assert len(view['versions']) == 5, view
    thumbnails_idle()
    rpc('undo_history')
    idle()
    assert png.read_bytes() == controls_source
    report['new_controls_preview_save_and_exact_undo'] = True
    thumbnails_idle(5)
    rpc('commit_history')
    idle()
    rpc('close_history')
    commands = rpc('list_commands')['commands']
    tool_options = {
        'adjust_image': dict(exposure=.37),
        'adjust_details': dict(sharpness=.4, definition=.25, noise_reduction=.2),
        'adjust_vignette': dict(vignette=.6),
    }
    report['separate_tool_frames'] = {}
    for tool, options in tool_options.items():
        assert any(c['id'] == tool and c['group'] == 'Редактирование' for c in commands)
        rpc('set_shortcut', action=tool, context='space', binding='G', replace=True)
        unchanged = png.read_bytes()
        rpc('run_action', id=tool)
        assert rpc()['adjustments_preview']['tool'] == tool
        rpc('run_action', id=tool)
        assert not rpc()['adjustments_open'] and png.read_bytes() == unchanged
        rpc('run_action', id=tool)
        rpc('preview_adjustments', options=options)
        wait_for(lambda s: s.get('adjustments_preview') and not s['adjustments_preview']['pending'] and not s['adjustments_preview']['error'])
        report['separate_tool_frames'][tool] = capture_sizes(tool)
        # Queue a newer version and toggle without waiting for its preview.
        latest_options = {key: value * .8 for key, value in options.items()}
        rpc('preview_adjustments', options=latest_options)
        rpc('run_action', id=tool)
        wait_for(lambda s: not s['adjustments_open'])
        idle()
        assert png.read_bytes() != unchanged
        view = history()
        assert len(view['versions']) == 5 and view['cursor'] == 4, view
        thumbnails_idle(5)
        rpc('undo_history')
        idle()
        assert png.read_bytes() == unchanged
        thumbnails_idle(5)
        rpc('commit_history')
        idle()
        rpc('close_history')
    report['tool_toggle_commits_latest_or_closes_unchanged'] = True
    for binding in ['Ctrl J', 'J', None]:
        rpc('set_shortcut', action='adjust_details', context='space', binding=binding)
        commands = rpc('list_commands')['commands']
        assert next(c for c in commands if c['id'] == 'adjust_details')['shortcut'] == (binding or '')
    prefs = json.loads((root / 'settings' / 'settings.json').read_text(encoding='utf8'))
    assert prefs['shortcuts']['space']['rename'] == 'Alt AltRight'
    report['legacy_modifier_does_not_block_unrelated_assignment_or_clear'] = True
    (root / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf8')
    print(root)
finally:
    stop_fixture(process)
    native_log.close()
