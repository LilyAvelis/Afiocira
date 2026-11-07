import json,time
from pathlib import Path
import os
pipe="\\\\.\\pipe\\"+os.environ.get("AFIO_API_NAME","afio-everything-preview")
def rpc(command):
 with open(pipe,'r+b',buffering=0) as p:
  p.write(json.dumps(command,ensure_ascii=False).encode()+b'\n'); data=b''
  while not data.endswith(b'\n'): data+=p.read(65536)
 r=json.loads(data);assert not r['error'],r['error'];return r
def search(query):
 start=time.monotonic();rpc(dict(command='search_files',query=query))
 while True:
  r=rpc(dict(command='status'))['search']
  if not r['loading']:break
  assert time.monotonic()-start<8;time.sleep(.025)
 assert not r['error'],r['error'];return r,round((time.monotonic()-start)*1000,1)
root=Path('target/everything-fixture-path.txt').read_text(encoding='utf8')
rows=[]
for name,view in [('фото тест.png','Photo'),('видео тест.mp4','Photo'),('папка тест','Grid'),('заметка тест.txt','Grid')]:
 r,ms=search('path:"'+root+'" "'+name+'"');assert len(r['results'])==1,r
 t=time.monotonic();opened=rpc(dict(command='open_search_result',index=0));accepted=(time.monotonic()-t)*1000
 while opened['search']['opening']:
  assert time.monotonic()-t<8
  time.sleep(.015);opened=rpc(dict(command='status'))
 assert not opened['search']['error'],opened['search']['error']
 assert opened['snapshot']['view']==view,opened
 if name.endswith('.txt'):assert opened['navigator']['focus']=='navigator',opened['navigator']
 rows.append(dict(name=name,search_ms=ms,accept_ms=round(accepted,1),open_ms=opened["search"]["open_ms"],view=view))
r,ms=search('afio_query_no_such_file_27a330986');assert r['total']==0 and not r['results']
r,ms=search('photo-workshop');assert len(r['results'])<=100 and r['total']>=len(r['results'])
for q in ['photo-workshop','*.png','afio_query_no_such_file_27a330986']:rpc(dict(command='search_files',query=q))
time.sleep(.4);r=rpc(dict(command='status'))['search'];assert not r['results'] and r['query'].endswith('27a330986')
rpc(dict(command='search_files',query='photo-workshop'));rpc(dict(command='close_search'));time.sleep(.3);assert not rpc(dict(command='status'))['search']['open']
rpc(dict(command='show_search'));time.sleep(.4);assert rpc(dict(command='status'))['search']['results']
r,ms=search('path:"'+root+'" "фото тест.png"');rpc(dict(command='open_search_result',index=0));time.sleep(.2);commands=rpc(dict(command='list_commands'))['commands'];assert commands[-1]['id']=='delete';assert any(c['id']=='search' for c in commands)
Path('target/everything-live-check.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2),encoding='utf8');print(json.dumps(rows,ensure_ascii=False));print('PASS: file/video/folder/nonmedia, Unicode, empty, bounded, stale/reopen, Delete last')
