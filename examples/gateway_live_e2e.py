#!/usr/bin/env python3
"""Exercise the public ACP gateway with a real configured backend and verify its artifact.
Use a dedicated empty --root. This consumes provider quota, and is distinct from an IDE UI test.
"""
import argparse,json,os,queue,signal,subprocess,threading,time
from pathlib import Path

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary',type=Path,default=Path(__file__).resolve().parents[1]/'target/debug/orochi')
p.add_argument('--config',type=Path,required=True)
p.add_argument('--data-dir',type=Path,required=True)
p.add_argument('--root',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
a=p.parse_args()
root=a.root.resolve();root.mkdir(parents=True,exist_ok=True)
assert not any(root.iterdir()), 'Use a new empty workspace'
base=[str(a.binary.resolve()),'--config',str(a.config.resolve()),'--data-dir',str(a.data_dir.resolve()),'-C',str(root)]
child=subprocess.Popen(base+['serve'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
messages=queue.Queue()
report={'status':'failed','client':'ACP v1 test client, not an IDE','updates':0,'permission_requests':0}
def read():
 try:
  for line in child.stdout:messages.put(json.loads(line))
 except Exception as e:messages.put(e)
 messages.put(EOFError())
threading.Thread(target=read,daemon=True).start()
threading.Thread(target=lambda:list(child.stderr),daemon=True).start()
def send(v):child.stdin.write(json.dumps(dict(jsonrpc='2.0',**v))+'\n');child.stdin.flush()
def rpc(n,method,params):
 send(dict(id=n,method=method,params=params))
 deadline=time.monotonic()+240
 while time.monotonic()<deadline:
  v=messages.get(timeout=max(.1,deadline-time.monotonic()))
  if isinstance(v,Exception):raise RuntimeError(type(v).__name__)
  if v.get('method')=='session/update':report['updates']+=1
  elif v.get('method')=='session/request_permission':
   report['permission_requests']+=1
   choice=next((o for o in v['params']['options'] if o['kind']=='allow_once'),None)
   assert choice, 'No one-time permission option'
   send(dict(id=v['id'],result={'outcome':{'outcome':'selected','optionId':choice['optionId']}}))
  elif v.get('id')==n:
   assert 'error' not in v, v.get('error')
   return v['result']
 raise TimeoutError()
try:
 init=rpc(1,'initialize',{'protocolVersion':1,'clientCapabilities':{},'clientInfo':{'name':'orochi-live-e2e','version':'1'}})
 assert init['protocolVersion']==1
 session=rpc(2,'session/new',{'cwd':str(root),'mcpServers':[]})['sessionId'];report['session_id']=session
 r=rpc(3,'session/prompt',{'sessionId':session,'prompt':[{'type':'text','text':'Create result.txt containing exactly OROCHI_IDE_OK followed by a newline. Modify no other files. Do not spawn subagents.'}]})
 report['stop_reason']=r['stopReason']
 assert r['stopReason']=='end_turn'
 assert (root/'result.txt').read_text()=='OROCHI_IDE_OK\n'
 runs=subprocess.run(base+['runs','--limit','5'],capture_output=True,text=True,timeout=15)
 report['runs']=json.loads(runs.stdout)
 assert any(r['outcome']=='success' for r in report['runs'])
 assert report['updates']>0
 report['status']='passed'
except Exception as e:report['error']=str(e)[:1000]
finally:
 try:os.killpg(child.pid,signal.SIGKILL)
 except ProcessLookupError:pass
 child.wait()
 a.output.parent.mkdir(parents=True,exist_ok=True)
 a.output.write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps({k:v for k,v in report.items() if k!='runs'},indent=2))
raise SystemExit(0 if report['status']=='passed' else 1)
