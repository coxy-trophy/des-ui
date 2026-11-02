#!/usr/bin/env python3
"""USB browser viewer for a compositor started with DES_VIRTUAL_SOCKET.
Only loopback HTTP is used. ADB reverse carries it to an authorized Android device.
"""
import argparse
import functools
import http.server
import secrets
import socket
import subprocess
import threading

PAGE = b'''<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><title>DEs display</title><style>body{margin:0;background:#090c10;color:white;font:18px sans-serif}canvas{width:100vw;height:100vh;object-fit:contain}#state{position:fixed;top:8px;left:8px;background:#090c10}</style><canvas></canvas><span id="state">Connecting</span><script>
const c=document.querySelector('canvas'),s=document.querySelector('#state'),ctx=c.getContext('2d');
async function loop(){try{const r=await fetch(location.pathname+'frame',{cache:'no-store'});if(!r.ok)throw Error('Disconnected');const b=new Uint8Array(await r.arrayBuffer()),n=b.indexOf(10),h=new TextDecoder().decode(b.slice(0,n));if(h.startsWith('RGBA ')){const [_,w,hg]=h.split(' ').map((x,i)=>i?Number(x):x);if(b.length-n-1!==w*hg*4)throw Error('Incomplete frame');c.width=w;c.height=hg;ctx.putImageData(new ImageData(new Uint8ClampedArray(b.buffer,n+1),w,hg),0,0);s.textContent='';}else if(h==='LOCKED'){ctx.clearRect(0,0,c.width,c.height);s.textContent='Session locked';}else if(h==='STOPPED'){ctx.clearRect(0,0,c.width,c.height);s.textContent='Reconnecting';await fetch(location.pathname+'start',{method:'POST'});}else if(h!=='WAIT'){throw Error(h);}}catch(e){ctx.clearRect(0,0,c.width,c.height);s.textContent='Disconnected - reconnecting';}setTimeout(loop,33);}loop();
</script>'''

def request(path, command):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(3)
        sock.connect(path)
        sock.sendall(command.encode()+b'\n')
        chunks=[]
        total=0
        while True:
            b=sock.recv(65536)
            if not b:
                return b''.join(chunks)
            total+=len(b)
            if total>3840*2160*4+128:
                raise ValueError('Frame exceeds limit')
            chunks.append(b)

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):
        pass
    def do_GET(self):
        self.respond(False)
    def do_POST(self):
        self.respond(True)
    def respond(self,post):
        root='/'+self.server.token+'/'
        if not self.path.startswith(root):
            self.send_error(403)
            return
        route=self.path[len(root):]
        try:
            if not post and route=='':
                data=PAGE
            elif not post and route=='frame':
                data=request(self.server.socket_path,'frame')
            elif post and route=='start':
                data=request(self.server.socket_path,self.server.start_command)
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type','text/html' if route=='' else 'application/octet-stream')
            self.send_header('Cache-Control','no-store')
            self.send_header('Content-Length',str(len(data)))
            self.send_header('X-Content-Type-Options','nosniff')
            self.end_headers()
            self.wfile.write(data)
        except (OSError,ValueError):
            self.send_error(503)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--socket',required=True)
    p.add_argument('--serial',required=True)
    p.add_argument('--width',type=int,default=1280)
    p.add_argument('--height',type=int,default=800)
    p.add_argument('--fps',type=int,default=30)
    p.add_argument('--port',type=int,default=8767)
    args=p.parse_args()
    server=http.server.HTTPServer(('127.0.0.1',args.port),Handler)
    server.token=secrets.token_urlsafe(32)
    server.socket_path=args.socket
    server.start_command=f'start {args.width} {args.height} {args.fps}'
    answer=request(args.socket,server.start_command)
    if answer!=b'OK\n':
        raise SystemExit(answer.decode(errors='replace'))
    adb=['adb','-s',args.serial]
    subprocess.run(adb+['reverse',f'tcp:{args.port}',f'tcp:{args.port}'],check=True)
    subprocess.run(adb+['shell','am','start','-a','android.intent.action.VIEW','-d',f'http://127.0.0.1:{args.port}/{server.token}/'],check=True,stdout=subprocess.DEVNULL)
    print('USB display viewer running. Ctrl+C stops it.',flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        try:
            request(args.socket,'stop')
        except OSError:
            pass
        subprocess.run(adb+['reverse','--remove',f'tcp:{args.port}'],check=False)
if __name__=='__main__':
    main()
