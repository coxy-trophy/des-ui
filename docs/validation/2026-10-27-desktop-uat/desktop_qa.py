import os,time,json,subprocess,shutil,pathlib,urllib.parse,struct
from evdev import UInput,AbsInfo,ecodes as e
ROOT=pathlib.Path('/home/coxy/Locker/desui/des-ui/docs/validation/2026-10-27-desktop-uat')
class Desktop:
 def __enter__(self):
  self.keyboard=UInput({e.EV_KEY:list(range(1,249)),e.EV_REL:[e.REL_X,e.REL_Y,e.REL_WHEEL]},name='DEs-UI validation keyboard')
  self.mouse=UInput({e.EV_ABS:[(e.ABS_X,AbsInfo(0,0,65535,0,0,0)),(e.ABS_Y,AbsInfo(0,0,65535,0,0,0))],e.EV_KEY:[e.BTN_LEFT,e.BTN_RIGHT,e.BTN_MIDDLE]},name='DEs-UI validation pointer',input_props=[e.INPUT_PROP_POINTER]);time.sleep(.7);return self
 def __exit__(self,*args):self.keyboard.close();self.mouse.close()
 def ctl(self,*args):
  result=subprocess.run(['des-ui-ctl',*map(str,args)],capture_output=True,text=True,timeout=20)
  with (ROOT/'actions.jsonl').open('a') as f:f.write(json.dumps({'time':time.time(),'command':list(args),'returncode':result.returncode,'answer':result.stdout.strip(),'error':result.stderr.strip()})+'\n')
  if result.returncode:raise RuntimeError(result.stdout+result.stderr)
  return result.stdout.strip()
 def state(self):return json.loads(self.ctl('state')[3:])
 def key(self,*keys,delay=.22):
  keys=[getattr(e,'KEY_'+k.upper()) for k in keys]
  for key in keys:self.keyboard.write(e.EV_KEY,key,1)
  self.keyboard.syn();time.sleep(.035)
  for key in reversed(keys):self.keyboard.write(e.EV_KEY,key,0)
  self.keyboard.syn();time.sleep(delay)
 def text(self,text):
  for char in text:
   if char.isalpha():self.key(*(['LEFTSHIFT'] if char.isupper() else []),char.upper(),delay=.025)
   elif char.isdigit():self.key(char,delay=.025)
   elif char==' ':self.key('SPACE',delay=.025)
   elif char in '/.-_':self.key(*(['LEFTSHIFT'] if char=='_' else []),{'/':'SLASH','.':'DOT','-':'MINUS','_':'MINUS'}[char],delay=.025)
   else:raise ValueError(char)
 def move(self,x,y,width=1920,height=1080):
  self.mouse.write(e.EV_ABS,e.ABS_X,round(x/width*65535));self.mouse.write(e.EV_ABS,e.ABS_Y,round(y/height*65535));self.mouse.syn();time.sleep(.18)
 def click(self,x,y,button='LEFT',**kwargs):
  self.move(x,y,**kwargs);b=getattr(e,'BTN_'+button);self.mouse.write(e.EV_KEY,b,1);self.mouse.syn();time.sleep(.06);self.mouse.write(e.EV_KEY,b,0);self.mouse.syn();time.sleep(.3)
 def drag(self,x1,y1,x2,y2):
  self.move(x1,y1);self.mouse.write(e.EV_KEY,e.BTN_LEFT,1);self.mouse.syn();time.sleep(.1)
  for i in range(1,11):self.move(x1+(x2-x1)*i/10,y1+(y2-y1)*i/10)
  self.mouse.write(e.EV_KEY,e.BTN_LEFT,0);self.mouse.syn();time.sleep(.3)
 def wheel(self,amount):self.keyboard.write(e.EV_REL,e.REL_WHEEL,amount);self.keyboard.syn();time.sleep(.3)
 def shot(self,name,mode='screen'):
  time.sleep(.35); answer=self.ctl('screenshot-file',mode);src=pathlib.Path(urllib.parse.unquote(urllib.parse.urlparse(json.loads(answer[3:])['uri']).path));dest=ROOT/(name+'.png');shutil.copy2(src,dest);os.chmod(dest,0o600);print('captured',name,struct.unpack('>II',dest.read_bytes()[16:24]),flush=True);return dest
 def record(self,label,details):
  with (ROOT/'results.jsonl').open('a') as f:f.write(json.dumps({'case':label,**details})+'\n')

# Every user input is logged in addition to the native CLI responses.
def logged(method):
 def run(self,*args,**kwargs):
  with (ROOT/'interaction-log.jsonl').open('a') as f:f.write(json.dumps({'time':time.time(),'action':method.__name__,'args':args,'kwargs':kwargs})+'\n')
  return method(self,*args,**kwargs)
 return run
for name in ['key','text','move','click','drag','wheel','shot']:
 setattr(Desktop,name,logged(getattr(Desktop,name)))
