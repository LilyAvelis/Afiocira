"""Native API smoke/regression check using only synthetic images and directories."""
import json,time,shutil,uuid,os,struct,hashlib
from pathlib import Path
pipe='\\\\.\\pipe\\'+os.environ.get('AFIO_API_NAME','afio-nested-preview')
def rpc(command,**kwargs):
 with open(pipe,'r+b',buffering=0) as p:
  p.write(json.dumps(dict(command=command,**kwargs),ensure_ascii=False).encode()+b'\n'); data=b''
  while not data.endswith(b'\n'):
   chunk=p.read(65536)
   if not chunk:raise RuntimeError('Disconnected API')
   data+=chunk
 r=json.loads(data);assert not r['error'],r['error'];return r
def wait(predicate,timeout=8):
 start=time.monotonic()
 while True:
  r=rpc('status')
  if predicate(r):return r
  assert time.monotonic()-start<timeout,r
  time.sleep(.04)
def job():return wait(lambda r:r['operation'] and not r['operation']['running'])
root=(Path('target')/('nested-check-'+uuid.uuid4().hex[:8])).resolve();root.mkdir()
source=Path('target/import-fixture/source/photo.png').read_bytes()
for folder,n in [('',8),('a',20),('b',6),('empty/deep',1)]:
 (root/folder).mkdir(parents=True,exist_ok=True)
 for i in range(n):(root/folder/f'{i:02}.png').write_bytes(source)
rpc('open',path=str(root));r=wait(lambda r:r['nested']['subfolders'] is not None and not r['nested']['loading'])
assert r['nested']['subfolders']==3 and r['snapshot']['count']>=8,r
rpc('load_nested',deeper=False);r=wait(lambda r:not r['nested']['loading']);assert r['nested']['loaded']<=26 and r['nested']['depth']==1,r
while r['nested']['can_load_more']:
 rpc('load_nested',deeper=False);r=wait(lambda r:not r['nested']['loading'])
assert r['nested']['loaded']==26 and r['nested']['visited']==3 and r['nested']['can_go_deeper'],r
rpc('load_nested',deeper=True);r=wait(lambda r:not r['nested']['loading']);assert r['nested']['depth']==2 and r['nested']['loaded']==27,r
r=rpc('list_order');assert any('deep' in p for p in r['snapshot']['ordered_paths']),r
rpc('load_nested',deeper=True);r=wait(lambda r:not r['nested']['loading']);assert r['nested']['loaded']==27,r
# A newly created direct directory updates the count, without reading its contents.
(root/'new child').mkdir();r=wait(lambda r:r['nested']['subfolders']==4 and not r['nested']['loading'])
assert r['nested']['loaded']==27,r
(root/'new child'/'new.png').write_bytes(source)
rpc('load_nested',deeper=False);r=wait(lambda r:not r['nested']['loading']);assert r['nested']['loaded']==28,r
# Warm previews and capture texture identity. Rename must preserve every existing texture.
rpc('select',index=0);rpc('open_selected');r=wait(lambda r:r['render']['preview_ready']);before=dict(r['render']['preview_textures']);old=r['snapshot']['selected_path']
rpc('begin_rename');rpc('rename',new_name='renamed-z.png');r=job();assert all(not f['error'] for f in r['operation']['files']),r
new=r['operation']['files'][0]['destination'];time.sleep(.7);r=rpc('status');after=dict(r['render']['preview_textures']);assert after[new]==before[old],(before,after)
for path,texture in before.items():
 if path!=old:assert after.get(path)==texture,(before,after)
# A real content edit invalidates this one preview; neighbours retain their textures.
Path(new).write_bytes(Path(new).read_bytes()+b'\n')
r=wait(lambda r:dict(r['render']['preview_textures']).get(new) not in (None,after[new]))
for path,texture in after.items():
 if path!=new:assert dict(r['render']['preview_textures']).get(path)==texture,r
# Nested cursor/marks survive Tab into Yazi and back.
nested=str(Path(r['snapshot']['folder'])/'a'/'00.png');rpc('back');rpc('select_path',path=nested);rpc('set_marked',paths=[nested]);rpc('focus_pane',pane='navigator');time.sleep(.25);rpc('focus_pane',pane='gallery');r=rpc('status');assert Path(r['snapshot']['folder']).name==root.name and nested in r['snapshot']['marked_paths'],r
rpc('open_selected');r=rpc('list_commands');assert {'resize_image','convert_image'}<=set(c['id'] for c in r['commands'])
original=Path(nested).read_bytes();rpc('run_action',id='resize_image');rpc('submit_image',options=dict(format='png',width=20));r=job();assert all(not f['error'] for f in r['operation']['files']),r
copy=Path(r['operation']['files'][0]['destination']);assert copy!=Path(nested) and Path(nested).read_bytes()==original
assert struct.unpack('>I',copy.read_bytes()[16:20])[0]==20
rpc('run_action',id='convert_image');rpc('submit_image',options=dict(format='webp'));r=job();assert all(not f['error'] for f in r['operation']['files']),r
converted=Path(r['operation']['files'][0]['destination']);assert converted.exists() and not copy.exists() and Path(nested).exists()
# No image command for video (API action rejected separately by applicability).
print(json.dumps(dict(root=str(root),nested_loaded=27,rename_same_texture=True,resize=str(copy),convert=str(converted)),ensure_ascii=False))
print('PASS lazy levels, bounded batches, cache identity, Tab marks, resize copy, conversion replacement')
