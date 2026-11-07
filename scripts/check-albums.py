"""Native album and dialog visual checks on synthetic fixtures only."""
import json,time,os,uuid,zlib,struct
from pathlib import Path
pipe='\\\\.\\pipe\\'+os.environ.get('AFIO_API_NAME','afio-albums-preview')
def rpc(command,**kwargs):
 with open(pipe,'r+b',buffering=0) as p:
  p.write(json.dumps(dict(command=command,**kwargs),ensure_ascii=False).encode()+b'\n');data=b''
  while not data.endswith(b'\n'):
   part=p.read(65536)
   if not part:raise RuntimeError('API disconnected')
   data+=part
 r=json.loads(data);assert not r['error'],r['error'];return r
def wait(predicate,timeout=8):
 start=time.monotonic()
 while True:
  r=rpc('status')
  if predicate(r):return r
  assert time.monotonic()-start<timeout,r
  time.sleep(.03)
def shot(name):
 path=(Path('target')/name).resolve();previous=path.stat().st_mtime_ns if path.exists() else None
 rpc('capture_frame',path=str(path));start=time.monotonic()
 while not path.exists() or path.stat().st_mtime_ns==previous:
  assert time.monotonic()-start<5;time.sleep(.04)
 return str(path)
def png(w,h,c):
 def chunk(k,v):return struct.pack('>I',len(v))+k+v+struct.pack('>I',zlib.crc32(k+v)&0xffffffff)
 return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+bytes(c)*w)*h))+chunk(b'IEND',b'')
root=(Path('target')/('album-check-'+uuid.uuid4().hex[:8])).resolve();root.mkdir()
(root/'Корень.png').write_bytes(png(400,240,(65,100,150)))
for folder,n in [('01 — Первый альбом',16),('02 — Второй альбом',9),('03 — Пустой/Внутри',4)]:
 (root/folder).mkdir(parents=True,exist_ok=True)
 for i in range(n):
  w,h=[(400,240),(160,240),(240,240)][i%3];(root/folder/f'Кадр {i+1:02}.png').write_bytes(png(w,h,(70+i*7,90+i*4,130+i*4)))
rpc('resize_viewport',width=1100,height=760);rpc('open',path=str(root));wait(lambda r:r['nested']['subfolders']==3 and not r['nested']['loading']);time.sleep(.4)
rpc('album_move',horizontal=0,vertical=1);r=wait(lambda r:not r['nested']['loading']);
a=next(a['folder'] for a in r['nested']['albums'] if 'Первый' in a['folder']);assert r['nested']['active_album']==a,r
wait(lambda r:next(a for a in r['nested']['albums'] if 'Первый' in a['folder'])['loaded']>=5 and not r['nested']['loading']);time.sleep(.3)
before=rpc('status')['nested'];others={a['folder']:a['loaded'] for a in before['albums'] if 'Первый' not in a['folder']}
for _ in range(5):rpc('album_move',horizontal=1,vertical=0)
r=wait(lambda r:not r['nested']['loading'] and dict(r['nested']['album_positions']).get(a)==5)
assert next(x for x in r['nested']['albums'] if x['folder']==a)['loaded']==10,r
assert all(x['loaded']==others[x['folder']] for x in r['nested']['albums'] if x['folder'] in others),r
rpc('album_move',horizontal=0,vertical=1);wait(lambda r:not r['nested']['loading']);rpc('album_move',horizontal=0,vertical=-1);r=rpc('status');assert dict(r['nested']['album_positions'])[a]==5
deep=[x for x in r['nested']['albums'] if x['depth']>1];assert all(x['loaded']==0 for x in deep),r
shots=[shot('albums-gallery.png')]
rpc('open_selected');rpc('run_action',id='resize_image');time.sleep(.2);shots.append(shot('albums-resize.png'));rpc('cancel_image')
rpc('run_action',id='convert_image');time.sleep(.2);shots.append(shot('albums-convert.png'));rpc('cancel_image')
rpc('show_commands');time.sleep(.2);shots.append(shot('albums-actions.png'));rpc('close_commands')
rpc('resize_viewport',width=680,height=520);rpc('run_action',id='resize_image');time.sleep(.2);shots.append(shot('albums-resize-narrow.png'));rpc('cancel_image')
result=dict(root=str(root),active_album=a,independent_cursor=5,neighbours_unchanged=True,screenshots=shots)
Path('target/albums-native-result.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf8');print(json.dumps(result,ensure_ascii=False))
