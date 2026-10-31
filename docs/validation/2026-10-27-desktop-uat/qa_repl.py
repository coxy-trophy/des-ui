import sys,json,traceback,os
from desktop_qa import Desktop
with Desktop() as desktop:
 print('READY',flush=True)
 for line in sys.stdin:
  try:
   request=json.loads(line)
   if request.get('stop'):break
   outputs=[]
   for call in request['calls']:
    method,*args=call
    if method not in ['click','move','drag','key','text','wheel','shot','state','record','ctl']:raise ValueError(method)
    result=getattr(desktop,method)(*args)
    if result is not None:outputs.append(str(result))
   print(json.dumps({'ok':True,'outputs':outputs}),flush=True)
  except Exception as error:print(json.dumps({'ok':False,'error':str(error)}),flush=True)
