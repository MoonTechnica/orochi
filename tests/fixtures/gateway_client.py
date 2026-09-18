"""An actual ACP client for exercising Orochi's stdio gateway, including permission and cancellation."""
import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time

mode, binary, config, data, root = sys.argv[1:]
p = subprocess.Popen([binary, '--config', config, '--data-dir', data, '-C', root, 'serve'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
messages = queue.Queue()
errors = []

def read():
    try:
        for line in p.stdout:
            messages.put(json.loads(line))  # Any stray stdout is a protocol failure.
    except Exception as e:
        messages.put(e)
    messages.put(EOFError())

threading.Thread(target=read, daemon=True).start()
threading.Thread(target=lambda: errors.extend(p.stderr.readlines()), daemon=True).start()

def send(value):
    p.stdin.write(json.dumps(dict(jsonrpc='2.0', **value))+'\n')
    p.stdin.flush()

def receive():
    value = messages.get(timeout=15)
    if isinstance(value, Exception):
        raise AssertionError(f'{value!r}: {errors}')
    return value

def rpc(id, method, params):
    send(dict(id=id,method=method,params=params))
    value=receive()
    assert value.get('id')==id,value
    return value

try:
    value=rpc(0,'session/new',{'cwd':root,'mcpServers':[]})
    assert 'error' in value,value
    value=rpc(1,'initialize',{'protocolVersion':1,'clientCapabilities':{},'clientInfo':{'name':'test','version':'1'}})
    assert value['result']['protocolVersion']==1,value
    assert not value['result']['agentCapabilities'].get('loadSession',False)
    value=rpc(2,'session/new',{'cwd':root,'mcpServers':[]})
    session=value['result']['sessionId']
    value=rpc(3,'session/prompt',{'sessionId':'unknown','prompt':[{'type':'text','text':'x'}]})
    assert 'error' in value,value
    send(dict(id=4,method='session/prompt',params={'sessionId':session,'prompt':[{'type':'text','text':'Implement the fixture task'}]}))
    if mode in ('cancel','disconnect'):
        deadline=time.monotonic()+10
        while not Path(root,'child.pid').exists():
            assert time.monotonic()<deadline,'backend never started'
            time.sleep(.02)
        if mode=='disconnect':
            p.stdin.close()
            p.wait(timeout=10)
            print(json.dumps({'ok':True,'mode':mode}))
            sys.exit(0)
        send(dict(method='session/cancel',params={'sessionId':session}))
    permission_seen=False
    streamed=False
    while True:
        value=receive()
        if value.get('method')=='session/request_permission':
            assert value['params']['sessionId']==session
            permission_seen=True
            assert mode in ('allow','deny')
            outcome={'outcome':'selected','optionId':'allow'} if mode=='allow' else {'outcome':'cancelled'}
            send(dict(id=value['id'],result={'outcome':outcome}))
        elif value.get('method')=='session/update':
            assert value['params']['sessionId']==session
            streamed=True
        elif value.get('id')==4:
            reason=value['result']['stopReason']
            assert reason==('cancelled' if mode in ('deny','cancel') else 'end_turn'),value
            break
        else:
            raise AssertionError(value)
    if mode in ('allow','deny'):
        assert permission_seen
    if mode not in ('deny','cancel'):
        assert streamed
        send(dict(id=5,method='session/prompt',params={'sessionId':session,'prompt':[{'type':'text','text':'Continue the same session'}]}))
        while True:
            value=receive()
            if value.get('method')=='session/request_permission':
                send(dict(id=value['id'],result={'outcome':{'outcome':'selected','optionId':'allow'}}))
            elif value.get('id')==5:
                assert value['result']['stopReason']=='end_turn',value
                break
    p.stdin.close()
    p.wait(timeout=10)
    print(json.dumps({'ok':True,'mode':mode}))
finally:
    if p.poll() is None:
        p.kill()
        p.wait()
