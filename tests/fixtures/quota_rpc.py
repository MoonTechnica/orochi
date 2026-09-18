"""Codex app-server's read-only quota protocol fixture."""
import json
import os
import sys
import time
for line in sys.stdin:
    request=json.loads(line)
    if os.environ.get('MOCK_LOG'):
        with open(os.environ['MOCK_LOG'],'a') as log:
            log.write(json.dumps(request)+'\n')
    method=request['method']
    if method=='initialize':
        print(json.dumps({'id':request['id'],'result':{}}),flush=True)
    elif method=='initialized':
        pass
    elif method=='account/rateLimits/read':
        print(json.dumps({'id':request['id'],'result':{'rateLimitsByLimitId':{'codex':{
            'primary':{'usedPercent':42,'resetsAt':int(time.time())+500},
            'secondary':{'usedPercent':60,'resetsAt':int(time.time())+5000}}}}}),flush=True)
    else:
        raise AssertionError(method)
