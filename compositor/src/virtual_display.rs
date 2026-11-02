//! Connector-independent outputs mapped into the existing desktop Space.
use smithay::{output::{Output, Mode, PhysicalProperties, Subpixel}, reexports::wayland_server::backend::GlobalId, utils::{Logical, Rectangle}};
use crate::DesComp;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config { pub width: i32, pub height: i32, pub fps: u32 }
impl Config {
    pub fn new(width:i32,height:i32,fps:u32)->Option<Self> {
        ((320..=3840).contains(&width)&&(240..=2160).contains(&height)&&(1..=60).contains(&fps)).then_some(Self{width,height,fps})
    }
    fn geometry(self,physical:&[Rectangle<i32,Logical>])->Option<Rectangle<i32,Logical>> {
        let x=physical.iter().map(|r|r.loc.x.checked_add(r.size.w)).collect::<Option<Vec<_>>>()?.into_iter().max()?;
        x.checked_add(self.width)?;
        Some(Rectangle::new((x,physical.first()?.loc.y).into(),(self.width,self.height).into()))
    }
}
pub struct VirtualOutput { pub output:Output, pub global:GlobalId, pub config:Config }
impl DesComp {
    pub fn virtual_geometry(&mut self,physical:&mut Vec<Rectangle<i32,Logical>>) {
        self.physical_screens=physical.clone();
        if let Some(v)=&self.virtual_output {
            if let Some(rect)=v.config.geometry(physical) {
                v.output.change_current_state(None,None,None,Some(rect.loc));
                self.space.map_output(&v.output,rect.loc);
                physical.push(rect);
            }
        }
    }
    pub fn stop_virtual_output(&mut self) {
        if let Some(v)=self.virtual_output.take() {
            self.space.unmap_output(&v.output);
            self.display_handle.remove_global::<DesComp>(v.global);
            self.set_screens(self.physical_screens.clone());
            if let (Some(pointer),Some(rect))=(self.seat.get_pointer(),self.screens.first().copied()) {
                let pos=pointer.current_location();
                if !self.screens.iter().any(|r|r.to_f64().contains(pos)) {self.pointer_moved(rect.loc.to_f64()-pos,(0.0,0.0).into(),0,0);}
            }
        }
    }
    pub fn start_virtual_output(&mut self,config:Config)->Result<(),String> {
        if self.locked {return Err("locked".into());}
        if config.geometry(&self.physical_screens).is_none() {return Err("no physical display".into());}
        if self.virtual_output.as_ref().is_some_and(|v|v.config==config) {return Ok(());}
        self.stop_virtual_output();
        let output=Output::new("DES-VIRTUAL-1".into(),PhysicalProperties{size:(0,0).into(),subpixel:Subpixel::Unknown,make:"DEs-UI".into(),model:"Virtual display".into()});
        let mode=Mode{size:(config.width,config.height).into(),refresh:config.fps as i32*1000};
        output.change_current_state(Some(mode),None,None,None);output.set_preferred(mode);
        let global=output.create_global::<DesComp>(&self.display_handle);
        self.virtual_output=Some(VirtualOutput{output,global,config});
        self.set_screens(self.physical_screens.clone());Ok(())
    }
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn bounded_configuration() {assert!(Config::new(3840,2160,60).is_some());for(w,h,f)in[(0,600,30),(800,0,30),(800,600,0),(3841,600,30),(800,600,61)]{assert!(Config::new(w,h,f).is_none());}}
 #[test] fn rightmost_geometry() {let c=Config::new(800,600,30).unwrap();let p=vec![Rectangle::new((-1920,0).into(),(1920,1080).into()),Rectangle::new((0,-200).into(),(1280,1024).into())];assert_eq!(c.geometry(&p).unwrap(),Rectangle::new((1280,0).into(),(800,600).into()));assert!(c.geometry(&[]).is_none());}
}

use std::{io::{Read,Write},os::unix::{net::{UnixListener,UnixStream},fs::{PermissionsExt,MetadataExt}},path::PathBuf,time::{Duration,Instant}};
struct Client { stream:UnixStream,input:Vec<u8>,data:Vec<u8>,sent:usize,since:Instant }
pub struct Server { listener:UnixListener,path:PathBuf,clients:Vec<Client>,last:Instant,last_frame:Instant }
impl Drop for Server {fn drop(&mut self){let _=std::fs::remove_file(&self.path);}}
impl Server {
 pub fn bind(path:PathBuf)->std::io::Result<Self>{
  let parent=path.parent().ok_or_else(||std::io::Error::other("socket needs a private directory"))?;
  let meta=std::fs::symlink_metadata(parent)?;
  if !meta.is_dir() || meta.mode()&0o077!=0 {return Err(std::io::Error::other("socket directory must be mode 0700"));}
  let listener=UnixListener::bind(&path)?;
  std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600))?;
  listener.set_nonblocking(true)?;
  Ok(Self{listener,path,clients:vec![],last:Instant::now(),last_frame:Instant::now()})
 }
}
impl DesComp {
 pub fn virtual_tick(&mut self){
  let Some(mut server)=self.virtual_server.take() else{return};
  for _ in 0..4 {let Ok((stream,_))=server.listener.accept() else{break};let uid=std::fs::metadata(&server.path).map(|m|m.uid()).ok();let peer=smithay::reexports::rustix::net::sockopt::socket_peercred(&stream).ok().map(|c|c.uid.as_raw());if uid.is_none() || peer!=uid {continue;}if server.clients.len()>=2{continue;}if stream.set_nonblocking(true).is_err(){continue;}server.clients.push(Client{stream,input:vec![],data:vec![],sent:0,since:Instant::now()});}
  let mut clients=std::mem::take(&mut server.clients);
  clients.retain_mut(|c|{
   if c.since.elapsed()>Duration::from_secs(2){return false;}
   if self.locked&&c.data.starts_with(b"RGBA"){return false;}
   if c.data.is_empty(){
    let mut b=[0;128];match c.stream.read(&mut b){Ok(0)=>return false,Ok(n)=>c.input.extend_from_slice(&b[..n]),Err(e)if e.kind()==std::io::ErrorKind::WouldBlock=>return true,Err(_)=>return false}
    if c.input.len()>128{return false;}if !c.input.contains(&b'\n'){return true;}
    let text=String::from_utf8_lossy(&c.input);let a:Vec<_>=text.split_whitespace().collect();
    c.data=match a.as_slice(){
     ["start",w,h,f]=>{let config=w.parse().ok().zip(h.parse().ok()).zip(f.parse().ok()).and_then(|((w,h),f)|Config::new(w,h,f));match config{Some(config)=>match self.start_virtual_output(config){Ok(())=>{server.last=Instant::now();b"OK\n".to_vec()},Err(e)=>format!("ERR {e}\n").into_bytes()},None=>b"ERR configuration\n".to_vec()}},
     ["move"] if !self.locked=>{self.move_active();b"OK\n".to_vec()},
     ["stop"]=>{self.stop_virtual_output();b"OK\n".to_vec()},
     ["frame"]=>{
      server.last=Instant::now();
      if self.locked{b"LOCKED\n".to_vec()}else if let Some(v)=&self.virtual_output{
       if server.last_frame.elapsed()<Duration::from_secs_f64(1.0/v.config.fps as f64){b"WAIT\n".to_vec()}else{server.last_frame=Instant::now();match self.grab(self.physical_screens.len(),true){Ok(f)=>{let mut data=format!("RGBA {} {}\n",f.width,f.height).into_bytes();data.extend(f.rgba);data},Err(e)=>format!("ERR {e}\n").into_bytes()}}
      }else{b"STOPPED\n".to_vec()}
     },_=>b"ERR command\n".to_vec()
    };
   }
   match c.stream.write(&c.data[c.sent..]){Ok(0)=>false,Ok(n)=>{c.sent+=n;c.sent<c.data.len()},Err(e)=>e.kind()==std::io::ErrorKind::WouldBlock}
  });
  server.clients=clients;
  if server.last.elapsed()>Duration::from_secs(5){self.stop_virtual_output();}
  self.virtual_server=Some(server);
 }
}

#[cfg(test)] mod transport_tests {
 use super::*;
 #[test] fn private_socket_is_removed_on_drop_and_never_replaces_existing_socket() {
  let dir=std::env::temp_dir().join(format!("des-virtual-test-{}",std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();std::fs::set_permissions(&dir,std::fs::Permissions::from_mode(0o700)).unwrap();
  let path=dir.join("display.sock");let server=Server::bind(path.clone()).unwrap();
  assert_eq!(std::fs::metadata(&path).unwrap().mode()&0o777,0o600);
  assert!(Server::bind(path.clone()).is_err());drop(server);assert!(!path.exists());std::fs::remove_dir(dir).unwrap();
 }
 #[test] fn shared_directories_are_refused() {assert!(Server::bind(std::env::temp_dir().join("des-virtual-insecure.sock")).is_err());}
}
