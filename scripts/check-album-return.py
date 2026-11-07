"""Album folder handoff, explicit Enter and ordinary file handoff regression."""
import json,time,os,uuid
from pathlib import Path
pipe='\\\\.\\pipe\\'+os.environ.get('AFIO_API_NAME','afio-album-return')
def rpc(command,**args):
 with open(pipe,'r+b',buffering=0) as p:
  p.write(json.dumps(dict(command=command,**args),ensure_ascii=False).encode()+b'\n');data=b''
  while not data.endswith(b'\n'):
   part=p.read(65536)
   if not part:raise RuntimeError('API disconnected')
   data+=part
 r=json.loads(data);assert not r['error'],r['error'];return r
def wait(predicate,timeout=8,command='status'):
 start=time.monotonic()
 while True:
  r=rpc(command)
  if predicate(r):return r
  assert time.monotonic()-start<timeout,r
  time.sleep(.03)
def norm(path):return os.path.normcase(os.path.normpath(str(path).replace('\\\\?\\','')))
def same(a,b):return a is not None and norm(a)==norm(b)
def goto(folder):
 for _ in range(20):
  r=rpc('status');current=r['nested']['active_album']
  if same(current,folder):return r
  rows=[r['snapshot']['folder']]+[x['folder'] for x in r['nested']['albums']]
  current_index=next((i for i,p in enumerate(rows) if same(p,current)),0)
  target_index=next(i for i,p in enumerate(rows) if same(p,folder))
  rpc('album_move',horizontal=0,vertical=1 if target_index>current_index else -1)
  wait(lambda r:not r['nested']['loading'])
 raise AssertionError('Could not select album')
def handoff(folder):
 rpc('focus_pane',pane='navigator')
 r=wait(lambda r:same(r['navigator']['folder'],folder.parent) and same(r['navigator']['hovered'],folder),command='navigator_snapshot')
 assert r['navigator']['focus']=='navigator' and r['nested']['overview'],r
 rpc('focus_pane',pane='gallery');r=rpc('status')
 assert same(r['snapshot']['folder'],root) and r['nested']['overview'] and same(r['nested']['active_album'],folder),r
 return r
root=(Path('target')/('album-return-'+uuid.uuid4().hex[:8])).resolve();root.mkdir();source=Path('target/import-fixture/source/photo.png').read_bytes()
(root/'root.png').write_bytes(source)
for name,count in [('a',12),('b',6),('a/deeper',1),('empty',0)]:
 (root/name).mkdir(parents=True,exist_ok=True)
 for i in range(count):(root/name/f'{i:02}.png').write_bytes(source)
(root/'b'/'video.mp4').write_bytes(Path('target/import-fixture/source/video.mp4').read_bytes())
rpc('open',path=str(root));wait(lambda r:r['nested']['overview'] and not r['nested']['loading'])
goto(root/'a');wait(lambda r:not r['nested']['loading'])
for _ in range(2):rpc('album_move',horizontal=1,vertical=0)
r=wait(lambda r:r['render']['thumbnail_ready']);photo=r['snapshot']['cursor_path'];textures=dict(r['render']['thumbnail_textures']);photo_texture=textures[photo]
rpc('set_marked',paths=[photo])
for _ in range(4):
 r=handoff(root/'a');assert same(r['snapshot']['cursor_path'],photo) and photo in r['snapshot']['marked_paths'],r
# An empty album must hand off to its directory, never the stale file cursor.
goto(root/'empty');handoff(root/'empty');goto(root/'b')
for _ in range(6):
 rpc('album_move',horizontal=1,vertical=0);wait(lambda r:not r['nested']['loading'])
assert rpc('status')['snapshot']['cursor_path'].endswith('video.mp4')
handoff(root/'b');goto(root/'a');r=rpc('status');assert same(r['snapshot']['cursor_path'],photo),r
commands=rpc('list_commands')['commands'];assert any(c['id']=='enter_album' and c['shortcut']=='Enter' for c in commands);assert next(c for c in commands if c['id']=='open')['shortcut']=='Space'
rpc('enter_album');r=wait(lambda r:r['nested']['subfolders'] is not None and not r['nested']['loading'])
assert same(r['snapshot']['folder'],root/'a') and r['snapshot']['view']=='Grid' and same(r['snapshot']['cursor_path'],photo),r
assert r['snapshot']['count']==12 and not r['nested']['overview'],r
assert dict(r['render']['thumbnail_textures']).get(photo)==photo_texture,r['render']
for _ in range(3):
 rpc('focus_pane',pane='navigator');wait(lambda r:same(r['navigator']['folder'],root/'a') and same(r['navigator']['hovered'],photo),command='navigator_snapshot')
 rpc('focus_pane',pane='gallery');r=rpc('status');assert not r['nested']['overview'] and same(r['snapshot']['cursor_path'],photo),r
rpc('open_selected');r=rpc('status');assert r['snapshot']['view']=='Photo' and same(r['snapshot']['selected_path'],photo),r
rpc('back');r=rpc('status');assert not r['nested']['overview'] and r['snapshot']['view']=='Grid'
result=dict(root=str(root),overview_roundtrips=4,empty_folder_handoff=True,ordinary_roundtrips=3,video_folder_handoff=True,entered_cursor=photo,same_thumbnail_texture=True,space_opens_file=True)
Path('target/album-return-result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf8');print(json.dumps(result,ensure_ascii=False))
