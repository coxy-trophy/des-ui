import socket,json,os
from desktop_qa import Desktop,ROOT
path='/tmp/desui-uat-control.sock'
if os.path.exists(path):os.unlink(path)
server=socket.socket(socket.AF_UNIX);server.bind(path);os.chmod(path,0o600);server.listen(1)
with Desktop() as desktop:
 print('READY',flush=True)
 while True:
  connection,_=server.accept()
  with connection:
   request=json.loads(connection.makefile('rb').readline())
   if request.get('stop'):connection.sendall(b'{"ok":true}\n');break
   try:
    outputs=[]
    for method,*args in request['calls']:
     if method not in ['click','move','drag','key','text','wheel','shot','state','record','ctl']:raise ValueError(method)
     result=getattr(desktop,method)(*args)
     if result is not None:outputs.append(str(result))
    connection.sendall((json.dumps({'ok':True,'outputs':outputs})+'\n').encode())
   except Exception as error:connection.sendall((json.dumps({'ok':False,'error':str(error)})+'\n').encode())
server.close();os.unlink(path)
