//! Bounded private IPC and protocol8 TLS device sessions. No capture starts on boot.
use crate::{cast::{self,Cast},phonekey::{self,Purpose,Reject},identity::{self,Store,Result},packet::{Identity,Packet,MAX_FRAME},pairing::{Pairing,Session,Attempt,State}};
use openssl::{ssl::{Ssl,SslContextBuilder,SslMethod,SslStream,SslVerifyMode,SslVersion},x509::X509};
use serde::{Serialize,Deserialize};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashMap,VecDeque},fs,io::{Read,Write},net::{TcpListener,TcpStream,UdpSocket,SocketAddr,IpAddr},os::unix::{net::{UnixListener,UnixStream},fs::{PermissionsExt,MetadataExt}},path::{Path,PathBuf},sync::{Arc,Mutex,mpsc::{self,SyncSender},atomic::{AtomicUsize,AtomicU64,AtomicBool,Ordering}},thread,time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
#[derive(Clone,Serialize,Deserialize,Debug)]
pub struct Device {
 pub id:String,pub name:String,pub kind:String,pub connected:bool,pub paired:bool,pub pair_state:String,
 pub attempt:Option<u64>,pub verification_key:Option<String>,pub battery:Option<u8>,pub charging:bool,
 pub last_ping:Option<i64>,pub error:Option<String>,
 #[serde(default)] pub files_enabled:bool,#[serde(default)] pub files_reconnect_needed:bool,
 #[serde(default)] pub media_enabled:bool,#[serde(default)] pub volume_enabled:bool,#[serde(default)] pub media_reconnect_needed:bool,#[serde(default)] pub volume_reconnect_needed:bool,
 #[serde(default)] pub wireless_cast:bool,
 #[serde(skip)] epoch:u64,
}
impl Device {fn offline(id:String,name:String,kind:String,files_enabled:bool,media_enabled:bool,volume_enabled:bool)->Self {Self{id,name,kind,connected:false,paired:true,pair_state:"offline".into(),attempt:None,verification_key:None,battery:None,charging:false,last_ping:None,error:None,files_enabled,files_reconnect_needed:false,media_enabled,volume_enabled,media_reconnect_needed:false,volume_reconnect_needed:false,wireless_cast:false,epoch:0}}}
#[derive(Clone,Deserialize,Serialize,Debug)]
pub struct Request {pub op:String,#[serde(default)] pub device_id:Option<String>,#[serde(default)] pub attempt:Option<u64>,#[serde(flatten)]pub args:BTreeMap<String,Value>}
struct Replacement {stream:SslStream<TcpStream>,identity:Identity,ip:AddressPermit,files_advertised:bool,media_advertised:bool,volume_advertised:bool}
#[derive(Clone)]
struct Route {epoch:u64,digest:[u8;32],commands:SyncSender<Job>,replacement:SyncSender<Replacement>}
struct Job {op:String,attempt:Option<u64>,args:BTreeMap<String,Value>,reply:SyncSender<std::result::Result<(),String>>,client:UnixStream,expires_at:Instant}
impl Job {fn content_guard(&self)->Result<()> {
 if Instant::now()>=self.expires_at{return Err("queued file request expired before content start".into());}
 if !control_alive(&self.client){return Err("file request was cancelled before content start".into());}Ok(())
}}
struct Shared {
 store:Mutex<Store>,ips:Mutex<BTreeMap<IpAddr,usize>>,devices:Mutex<BTreeMap<String,Device>>,routes:Mutex<BTreeMap<String,Route>>,
 screens:Mutex<crate::screens::ScreenBridge>,screen_peer:Mutex<Option<String>>,cast:Mutex<Option<Cast>>,transfers:Mutex<Option<crate::transfers::TransferManager>>,loopback:bool,stop:AtomicBool,
 /// Never held together with `store`: take one, let it go, then take the other.
 phonekey:Mutex<phonekey::Manager>,
 next:AtomicU64,next_approval:AtomicU64,active:AtomicUsize,unpaired:AtomicUsize,ipc_active:AtomicUsize,id:String,name:String,port:u16,
}
impl Shared {
 fn snapshot(&self)->Value {
  let devices=self.devices.lock().unwrap().values().cloned().collect::<Vec<_>>();let transfers=self.transfers.lock().unwrap().as_mut().map(|manager|manager.snapshots()).unwrap_or_default();let mut screens=self.screens.lock().unwrap();
  let mut screen_status=serde_json::to_value(screens.status()).unwrap();
  screen_status["device_id"]=json!(self.screen_peer.lock().unwrap().clone());
  if let Some(cast)=self.cast.lock().unwrap().as_ref(){screen_status=cast.snapshot();}
  json!({"id":self.id,"name":self.name,"tcp_port":self.port,"running":!self.stop.load(Ordering::Acquire),"devices":devices,"transfers":transfers,"screens":screen_status,"screen_capabilities":screens.capabilities()})
 }
 fn paired(&self,id:&str)->bool {self.devices.lock().unwrap().get(id).is_some_and(|d|d.connected&&d.paired)&&self.store.lock().unwrap().peers.contains_key(id)}
 fn cast_peer_authorized(&self,id:&str,epoch:u64)->bool {
  if self.stop.load(Ordering::Acquire){return false;}
  let digest=self.routes.lock().unwrap().get(id).filter(|route|route.epoch==epoch).map(|route|route.digest);
  let Some(digest)=digest else{return false;};
  let connected=self.devices.lock().unwrap().get(id).is_some_and(|d|d.epoch==epoch&&d.connected&&d.paired);
  connected&&self.store.lock().unwrap().pin(id).is_ok_and(|known|known==Some(digest))
 }
 fn clear_cast(&self, check:impl FnOnce(&Cast)->bool) {
  let mut slot=self.cast.lock().unwrap();
  if slot.as_ref().is_some_and(check){*slot=None;}
 }
 fn identity(&self)->Packet {self.identity_with_policies(false,false,false)}
 fn identity_with_policies(&self,files:bool,media:bool,volume:bool)->Packet {let mut identity=packet("kdeconnect.identity",json!({"deviceId":self.id,"deviceName":self.name,"deviceType":"desktop","protocolVersion":8,"tcpPort":self.port,
 "incomingCapabilities":["kdeconnect.ping","kdeconnect.battery"],"outgoingCapabilities":["kdeconnect.ping","kdeconnect.battery.request"]}));
  if files {for key in ["incomingCapabilities","outgoingCapabilities"] {identity.body.get_mut(key).unwrap().as_array_mut().unwrap().push(json!("kdeconnect.share.request"));}}
  for(enabled,request,reply)in[(media,"kdeconnect.mpris.request","kdeconnect.mpris"),(volume,"kdeconnect.systemvolume.request","kdeconnect.systemvolume")] {
   if enabled{identity.body.get_mut("incomingCapabilities").unwrap().as_array_mut().unwrap().push(json!(request));identity.body.get_mut("outgoingCapabilities").unwrap().as_array_mut().unwrap().push(json!(reply));}
  }identity
 }
 fn secure_identity(&self,id:&str,digest:[u8;32])->Result<(Packet,(bool,bool,bool))> {
  let store=self.store.lock().unwrap();let trusted=store.pin(id)?==Some(digest);let peer=store.peers.get(id);
  let files=trusted&&peer.is_some_and(|p|p.files_enabled);let media=trusted&&peer.is_some_and(|p|p.media_enabled);let volume=trusted&&peer.is_some_and(|p|p.volume_enabled);
  let mut identity=self.identity_with_policies(files,media,volume);
  for key in ["incomingCapabilities","outgoingCapabilities"] {
   identity.body[key].as_array_mut().unwrap().push(json!(cast::CAPABILITY));
   // Says only that this desktop speaks the phone-key packets. Nothing is asked of
   // a phone until the user enrols it, and a phone that does not list it is never sent one.
   identity.body[key].as_array_mut().unwrap().push(json!(phonekey::CAPABILITY));
  }
  Ok((identity,(files,media,volume)))
 }
 fn files_allowed(&self,id:&str)->bool {self.paired(id)&&self.store.lock().unwrap().peers.get(id).is_some_and(|peer|peer.files_enabled)}
 fn ensure_transfers(&self)->Result<()> {
  if self.stop.load(Ordering::Acquire){return Err("DES Connect is stopping".into());}
  if self.transfers.lock().unwrap().is_some(){return Ok(());}
  // Directory preparation may block in the OS; never hold the manager mutex.
  let prepared=crate::transfers::TransferManager::new(crate::transfers::default_download_directory()?)?;
  let mut manager=self.transfers.lock().unwrap();
  if self.stop.load(Ordering::Acquire){return Err("DES Connect is stopping".into());}
  if manager.is_none(){*manager=Some(prepared);}Ok(())
 }
 fn with_transfers<T>(&self,operation:impl FnOnce(&mut crate::transfers::TransferManager)->std::result::Result<T,String>)->Result<T> {
  self.ensure_transfers()?;let mut manager=self.transfers.lock().unwrap();
  if self.stop.load(Ordering::Acquire){return Err("DES Connect is stopping".into());}
  operation(manager.as_mut().ok_or("file manager is stopping")?).map_err(Into::into)
 }
 fn cancel_peer_files(&self,id:&str){if let Some(manager)=self.transfers.lock().unwrap().as_mut(){manager.cancel_peer(id);}}
 fn cancel_all_files(&self){if let Some(manager)=self.transfers.lock().unwrap().as_mut(){for transfer in manager.snapshots(){let _=manager.cancel(&transfer.id);}}}
 fn update(&self,view:Device) {let mut devices=self.devices.lock().unwrap();if devices.get(&view.id).is_none_or(|d|d.epoch<=view.epoch){if devices.len()<72||devices.contains_key(&view.id){devices.insert(view.id.clone(),view);}}}
}
pub fn packet(kind:&str,body:Value)->Packet {Packet{id:wall()*1000,kind:kind.into(),body:body.as_object().cloned().unwrap_or_default(),payload_size:None,payload_transfer_info:None,extra:Default::default()}}
fn wall()->i64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().min(i64::MAX as u64) as i64}
fn local(ip:IpAddr)->bool {match ip {IpAddr::V4(ip)=>ip.is_private()||ip.is_loopback()||ip.is_link_local(),IpAddr::V6(ip)=>ip.is_loopback()||ip.is_unique_local()||ip.is_unicast_link_local()}}
pub fn default_socket()->PathBuf {std::env::var_os("DES_CONNECT_SOCKET").map(PathBuf::from).unwrap_or_else(||std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(||PathBuf::from(format!("/run/user/{}",rustix::process::getuid().as_raw()))).join("des-connect/control.sock"))}
pub fn default_state()->PathBuf {std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")).join("des-ui/connect")}
fn read_line(stream:&mut impl Read,max:usize)->Result<Vec<u8>> {
 let mut bytes=Vec::new();let mut b=[0;1];loop {stream.read_exact(&mut b)?;bytes.push(b[0]);if bytes.len()>max{return Err("frame too large".into());}if b[0]==b'\n'{return Ok(bytes);}}
}
fn decode(bytes:&[u8])->Result<Packet> {Ok(Packet::read(&mut std::io::Cursor::new(bytes))?)}
fn write_packet(stream:&mut SslStream<TcpStream>,packet:Packet)->Result<()> {stream.write_all(&packet.encode()?)?;stream.flush()?;Ok(())}
fn tls_context(shared:&Shared,id:&str)->Result<openssl::ssl::SslContext> {
 let store=shared.store.lock().unwrap();let pin=store.pin(id)?;let expected=id.to_owned();
 let mut ctx=SslContextBuilder::new(SslMethod::tls())?;
 ctx.set_min_proto_version(Some(SslVersion::TLS1_2))?;ctx.set_max_proto_version(Some(SslVersion::TLS1_2))?;
 ctx.set_certificate(&store.certificate)?;ctx.set_private_key(&store.key)?;ctx.check_private_key()?;
 // Self-signed identities are possession-checked by TLS, but unpaired identities
 // get no feature access. Existing pins are enforced inside AND after handshake.
 ctx.set_verify_callback(SslVerifyMode::PEER|SslVerifyMode::FAIL_IF_NO_PEER_CERT,move|_,verify|{
  if verify.error_depth()!=0{return true;}
  verify.current_cert().is_some_and(|cert| identity::peer_digest(&cert.to_owned(),&expected).is_ok_and(|digest|pin.is_none_or(|known|known==digest)))
 });Ok(ctx.build())
}
fn handshake(shared:&Shared,mut tcp:TcpStream,claim:Option<Packet>)->Result<(SslStream<TcpStream>,Identity,X509,(bool,bool,bool))> {
 tcp.set_read_timeout(Some(Duration::from_secs(5)))?;tcp.set_write_timeout(Some(Duration::from_secs(3)))?;tcp.set_nodelay(true)?;
 let _deadline=HandshakeDeadline::new(&tcp)?;
 let client_mode=claim.is_none();
 let plain=if let Some(claim)=claim {tcp.write_all(&shared.identity().encode()?)?;claim}else{decode(&read_line(&mut tcp,MAX_FRAME)?)?};
 let before=Identity::from_packet(&plain)?;
 if before.device_id==shared.id{return Err("self connection".into());}
 if plain.body.get("targetDeviceId").and_then(Value::as_str).is_some_and(|id|id!=shared.id){return Err("wrong target identity".into());}
 let context=tls_context(shared,&before.device_id)?;let ssl=Ssl::new(&context)?;let mut stream=SslStream::new(ssl,tcp)?;
 if client_mode {stream.connect()?;}else{stream.accept()?;}
 let cert=stream.ssl().peer_certificate().ok_or("TLS peer sent no certificate")?;
 let digest=identity::peer_digest(&cert,&before.device_id)?;
 if shared.store.lock().unwrap().pin(&before.device_id)?.is_some_and(|pin|pin!=digest){return Err("TLS pin changed".into());}
 let (secure_identity,files_advertised)=shared.secure_identity(&before.device_id,digest)?;write_packet(&mut stream,secure_identity)?;
 let secure=decode(&read_line(&mut stream,MAX_FRAME)?)?;let after=Identity::from_packet(&secure)?;
 if before.device_id!=after.device_id||before.protocol_version!=after.protocol_version{return Err("identity changed during TLS handshake".into());}
 stream.get_ref().set_read_timeout(Some(Duration::from_millis(200)))?;
 Ok((stream,after,cert,files_advertised))
}
struct Actor {shared:Arc<Shared>,view:Device,stream:SslStream<TcpStream>,cert:X509,pair:Pairing,session:Session,started:Instant,timestamp:Option<i64>,write_failed:bool,public_attempt:Option<(Attempt,u64)>,unpaired_permit:Option<UnpairedPermit>,ip:AddressPermit,incoming_capabilities:Vec<String>,outgoing_capabilities:Vec<String>,files_advertised:bool,media_advertised:bool,volume_advertised:bool,reconnect_requested:bool,media_dispatched:bool,cast_id:Option<String>,last_cast_lease:Instant,presence_seq:u64,presence_answered:u64,last_presence:Instant}
impl Actor {
 fn send(&mut self,packet:Packet)->Result<()> {let result=write_packet(&mut self.stream,packet);if result.is_err(){self.write_failed=true;}result}
 fn request_battery(&mut self)->Result<()> {
  // Current Android peers publish battery updates but no longer accept this
  // legacy poll. Only send it when the encrypted identity explicitly supports it.
  if self.incoming_capabilities.iter().any(|kind|kind=="kdeconnect.battery.request") {
   self.send(packet("kdeconnect.battery.request",json!({"request":true})))?;
  }Ok(())
 }
 fn now(&self)->u64 {self.started.elapsed().as_secs()}
 fn publish(&mut self) {
  let policy=self.shared.store.lock().unwrap().peers.get(&self.view.id).map(|peer|(peer.files_enabled,peer.media_enabled,peer.volume_enabled)).unwrap_or_default();
  let authorized=self.pair.authorized();self.view.files_enabled=authorized&&policy.0;self.view.media_enabled=authorized&&policy.1;self.view.volume_enabled=authorized&&policy.2;
  self.view.files_reconnect_needed=authorized&&self.view.files_enabled!=self.files_advertised;self.view.media_reconnect_needed=authorized&&self.view.media_enabled!=self.media_advertised;self.view.volume_reconnect_needed=authorized&&self.view.volume_enabled!=self.volume_advertised;
  self.view.wireless_cast=self.cast_capable();
  self.view.paired=self.pair.authorized();self.view.pair_state=match self.pair.state(){State::Incoming=>"incoming",State::Outgoing=>"outgoing",State::Accepting=>"accepting",State::Paired=>"paired",State::Disconnected=>"offline",State::Unpaired=>"unpaired"}.into();
  let attempt=self.pair.pending();if self.public_attempt.map(|p|p.0)!=attempt {self.public_attempt=attempt.map(|a|(a,self.shared.next_approval.fetch_add(1,Ordering::Relaxed)));}
  self.view.attempt=self.public_attempt.map(|a|a.1);
  self.view.verification_key=if self.view.attempt.is_some(){self.timestamp.and_then(|stamp|identity::verification_key(&self.shared.store.lock().unwrap().certificate,&self.cert,stamp).ok())}else{None};
  self.shared.update(self.view.clone());
 }
 fn persist(&mut self)->Result<()> {self.shared.store.lock().unwrap().remember(&self.view.id,&self.view.name,&self.view.kind,&self.cert)?;Ok(())}
 fn file_guard(&self)->Result<()> {
  if self.shared.stop.load(Ordering::Acquire){return Err("DES Connect is stopping".into());}
  if !self.pair.authorized()||!self.shared.files_allowed(&self.view.id){return Err("file sharing requires a paired connected device with file permission enabled".into());}
  if !self.files_advertised{return Err("reconnect this device to apply file permission before sharing".into());}
  if !session_unlocked(){return Err("file sharing requires an unlocked DES UI session".into());}Ok(())
 }
 fn payload_peer(&self)->Result<crate::transfers::PeerTls> {
  let store=self.shared.store.lock().unwrap();let pin=store.pin(&self.view.id)?.ok_or("device is no longer paired")?;
  crate::transfers::PeerTls::new(store.key.clone(),store.certificate.clone(),self.view.id.clone(),self.cert.clone(),pin,self.stream.get_ref().peer_addr()?.ip()).map_err(Into::into)
 }
 fn cast_capable(&self)->bool {
  matches!(self.view.kind.as_str(),"phone"|"tablet")&&
   [&self.incoming_capabilities,&self.outgoing_capabilities].iter().all(|caps|caps.iter().any(|cap|cap==cast::CAPABILITY))
 }
 fn cast_authorized(&self)->bool {
  if self.shared.stop.load(Ordering::Acquire)||!self.pair.authorized()||!self.cast_capable(){return false;}
  let pin=self.pair.pinned();
  pin.is_some()&&self.shared.store.lock().unwrap().pin(&self.view.id).is_ok_and(|known|known==pin)
 }
 fn cast_job_guard(&self,job:&Job)->Result<()> {
  if Instant::now()>=job.expires_at||!control_alive(&job.client){return Err("cast command expired or was cancelled".into());}
  if !self.cast_authorized(){return Err("cast requires a paired phone that supports wireless casting".into());}
  if !session_unlocked(){
   self.shared.clear_cast(|cast|cast.owned_by(&self.view.id,self.view.epoch));
   return Err("casting requires an unlocked DES UI session".into());
  }
  Ok(())
 }
 fn cast_start(&mut self,job:&Job)->Result<()> {
  self.cast_job_guard(job)?;
  self.cast_tick()?;
  if self.cast_id.is_some(){return Err("stop the current screen session before casting".into());}
  let id=cast::new_id()?;
  // Always take the USB screen lock first when both transports are involved.
  {
   let mut screens=self.shared.screens.lock().unwrap();
   let mut slot=self.shared.cast.lock().unwrap();
   if slot.is_some()||screens.status().running{return Err("stop the current screen session before casting".into());}
   *slot=Some(Cast::new(self.view.id.clone(),self.view.epoch,id.clone(),Instant::now()));
  }
  let result=self.cast_job_guard(job).and_then(|()|{
   if !self.shared.cast.lock().unwrap().as_ref().is_some_and(|cast|cast.id==id){return Err("cast request was stopped".into());}
   self.send(packet(cast::CAPABILITY,json!({"op":"start","id":id})))
  });
  if result.is_err(){self.shared.clear_cast(|cast|cast.id==id);}
  else {self.cast_id=Some(id);self.last_cast_lease=Instant::now();}
  result
 }
 fn cast_input(&mut self,job:&Job)->Result<()> {
  self.cast_job_guard(job)?;
  let id=job.args.get("serial").and_then(Value::as_str).filter(|id|cast::valid_id(id)).ok_or("missing cast session")?;
  let body={
   let mut slot=self.shared.cast.lock().unwrap();
   let cast=slot.as_mut().filter(|cast|cast.id==id&&cast.owned_by(&self.view.id,self.view.epoch)).ok_or("cast session has ended")?;
   cast.input(&job.args,Instant::now())?
  };
  // Re-check the live client, lock, trust and current frame after preparing
  // input. Do not block Stop or frame revocation on a TLS write.
  self.cast_job_guard(job)?;
  let geometry=body["geometry"].as_u64().ok_or("missing cast geometry")?;
  let valid=self.shared.cast.lock().unwrap().as_ref().is_some_and(|cast|
   cast.id==id&&cast.owned_by(&self.view.id,self.view.epoch)&&cast.input_allowed(geometry,Instant::now()));
  if !valid{return Err("cast session or preview geometry changed".into());}
  if Instant::now()>=job.expires_at||!control_alive(&job.client){return Err("cast command expired or was cancelled".into());}
  self.send(packet(cast::CAPABILITY,body))
 }
 fn cast_tick(&mut self)->Result<()> {
  let Some(id)=self.cast_id.clone() else{return Ok(());};
  let due=self.last_cast_lease.elapsed()>=Duration::from_secs(1);
  if due&&(!self.cast_authorized()||!session_unlocked()){
   self.shared.clear_cast(|cast|cast.id==id);
  }
  let active={
   let mut slot=self.shared.cast.lock().unwrap();
   if slot.as_ref().is_some_and(|cast|cast.id==id&&cast.expired(Instant::now())){*slot=None;}
   slot.as_ref().is_some_and(|cast|cast.id==id)
  };
  if !active {
   self.cast_id=None;
   self.send(packet(cast::CAPABILITY,json!({"op":"stop","id":id})))?;
  }else if due{
   self.last_cast_lease=Instant::now();
   self.send(packet(cast::CAPABILITY,json!({"op":"lease","id":id})))?;
  }
  Ok(())
 }
 fn cast_received(&mut self,np:&Packet) {
  let Some(id)=self.cast_id.as_deref() else{return;};
  if np.payload_size.is_some()||!self.cast_authorized(){return;}
  let mut slot=self.shared.cast.lock().unwrap();
  if let Some(cast)=slot.as_mut().filter(|cast|cast.id==id&&cast.owned_by(&self.view.id,self.view.epoch)){
   if matches!(cast.receive(&np.body,Instant::now()),cast::Update::Stop){*slot=None;self.cast_id=None;}
  }
 }
 fn cast_clear_link(&mut self) {
  self.shared.clear_cast(|cast|cast.owned_by(&self.view.id,self.view.epoch));
  self.cast_id=None;
 }
 fn phonekey_capable(&self)->bool {
  [&self.incoming_capabilities,&self.outgoing_capabilities].iter().all(|caps|caps.iter().any(|cap|cap==phonekey::CAPABILITY))
 }
 /// Ask the walk-away phone whether it is still there. Its answers can only ever
 /// postpone a lock; see `phonekey::Walkaway`.
 fn phonekey_tick(&mut self)->Result<()> {
  if self.last_presence.elapsed()<bounded_duration("DES_CONNECT_PRESENCE_MS",phonekey::PRESENCE_INTERVAL_MS){return Ok(());}
  self.last_presence=Instant::now();
  if !self.pair.authorized()||!self.phonekey_capable(){return Ok(());}
  if !self.shared.store.lock().unwrap().peers.get(&self.view.id).is_some_and(|peer|peer.walkaway){return Ok(());}
  self.presence_seq+=1;
  self.send(packet(phonekey::CAPABILITY,json!({"v":phonekey::VERSION,"op":"presence","seq":self.presence_seq})))
 }
 fn phonekey_received(&mut self,np:&Packet) {
  if np.payload_size.is_some()||!self.pair.authorized()||!self.phonekey_capable(){return;}
  if np.body.get("v").and_then(Value::as_u64)!=Some(phonekey::VERSION){return;}
  match np.body.get("op").and_then(Value::as_str).unwrap_or("") {
   "present"=>{
    // Only the answer to the newest question counts, and only once.
    if np.body.get("seq").and_then(Value::as_u64)!=Some(self.presence_seq)||self.presence_seq==self.presence_answered{return;}
    self.presence_answered=self.presence_seq;
    if self.shared.store.lock().unwrap().peers.get(&self.view.id).is_some_and(|peer|peer.walkaway){self.shared.phonekey.lock().unwrap().walkaway.seen(mono_ms());}
   }
   "response"|"enrolled"=>{
    // The binding and the key are read from the trust store now, not from when the
    // request was made: unpairing or revoking refuses an answer already in flight.
    let Ok(binding)=phonekey_binding(&self.shared,&self.view.id)else{return;};
    if self.pair.pinned()!=Some(binding.phone_cert){return;}
    let key=phonekey_key(&self.shared,&self.view.id);
    let _=self.shared.phonekey.lock().unwrap().resolve(mono_ms(),&self.view.id,&np.body,key.as_ref(),&binding);
   }
   "revoke"=>{
    // The phone deleted its key. Forget ours; it can only be enrolled again from here.
    let had=self.shared.store.lock().unwrap().update_peer(&self.view.id,|peer|{peer.phonekey=None;Ok(())}).is_ok();
    if had{self.shared.phonekey.lock().unwrap().revoke_device(mono_ms(),&self.view.id);}
   }
   _=>{},
  }
 }
 fn pending(&self,id:Option<u64>)->Result<Attempt>{self.public_attempt.filter(|a|Some(a.1)==id&&self.pair.pending()==Some(a.0)).map(|a|a.0).ok_or_else(||"pairing request changed; refresh before accepting".into())}
 fn command(&mut self,job:&Job)->Result<()> {
  match job.op.as_str() {
   "cast_phone"=>self.cast_start(job)?,
   "cast_input"=>return self.cast_input(job),
   "phonekey_send"=>{
    if !self.pair.authorized(){return Err("pair this device before using it as a key".into());}
    if !self.phonekey_capable(){return Err("this device's DES Connect does not support phone-key requests".into());}
    let body=job.args.get("body").filter(|body|body.is_object()).cloned().ok_or("missing phone-key body")?;
    return self.send(packet(phonekey::CAPABILITY,body));
   }
   "set_files"|"set_media"|"set_volume"=>{
    if !self.pair.authorized()||!session_unlocked(){return Err("permission changes require a paired device and unlocked DES UI session".into());}
    let enabled=job.args.get("enabled").and_then(Value::as_bool).ok_or("enabled must be a boolean")?;
    {let mut store=self.shared.store.lock().unwrap();match job.op.as_str(){"set_files"=>store.set_files_enabled(&self.view.id,enabled)?,"set_media"=>store.set_media_enabled(&self.view.id,enabled)?,_=>store.set_volume_enabled(&self.view.id,enabled)?}}
    if job.op=="set_files"&&!enabled{self.shared.cancel_peer_files(&self.view.id);}
   }
   "reconnect"=>{
    if !self.pair.authorized()||!session_unlocked(){return Err("reconnect requires a paired device and unlocked DES UI session".into());}
    self.shared.cancel_peer_files(&self.view.id);self.reconnect_requested=true;
   }
   "send_file"=>{
    job.content_guard()?;self.file_guard()?;
    if !self.incoming_capabilities.iter().any(|cap|cap=="kdeconnect.share.request"){return Err("peer does not advertise receiving shared files".into());}
    let path=job.args.get("path").and_then(Value::as_str).ok_or("select a local regular file path")?;
    let prepared=crate::transfers::PreparedSource::open(Path::new(path))?;
    job.content_guard()?;let peer=self.payload_peer()?;self.shared.ensure_transfers()?;job.content_guard()?;self.file_guard()?;
    let ticket=self.shared.with_transfers(|manager|{job.content_guard().map_err(|e|e.to_string())?;manager.send_prepared(peer,prepared)})?;
    if let Err(error)=job.content_guard().and_then(|_|self.file_guard()).and_then(|_|job.content_guard()){let _=self.shared.with_transfers(|manager|manager.cancel(&ticket.id));return Err(error);}
    if let Err(error)=self.send(ticket.packet){let _=self.shared.with_transfers(|manager|manager.cancel(&ticket.id));return Err(error);}
   }
   "accept_transfer"=>{
    job.content_guard()?;self.file_guard()?;let id=job.args.get("transfer_id").and_then(Value::as_str).ok_or("transfer_id required")?;
    self.shared.with_transfers(|manager|{
     let transfer=manager.snapshots().into_iter().find(|view|view.id==id).ok_or("unknown transfer")?;
     if transfer.peer_id!=self.view.id||transfer.direction!="receive"{return Err("transfer does not belong to this device".into());}
     job.content_guard().map_err(|e|e.to_string())?;manager.accept(id).map(|_|())
    })?;
   }
   "pair"=>{
    if self.pair.state()!=State::Unpaired{return Err("device is already paired or pairing".into());}
    let timestamp=wall();self.send(packet("kdeconnect.pair",json!({"pair":true,"timestamp":timestamp})))?;
    self.pair.request(self.session,self.now())?;self.timestamp=Some(timestamp);
   }
   "accept"=>{let attempt=self.pending(job.attempt)?;self.pair.accept(self.session,attempt,self.now())?;
    if let Err(e)=self.send(packet("kdeconnect.pair",json!({"pair":true}))){self.pair.send_failed(self.session,attempt);return Err(e);}
    self.pair.accept_sent(self.session,attempt,self.now())?;
    if let Err(e)=self.persist(){self.pair.unpair(self.session)?;let _=self.send(packet("kdeconnect.pair",json!({"pair":false})));return Err(e);}
    let _=self.request_battery();
   }
   "reject"=>{let attempt=self.pending(job.attempt)?;self.pair.cancel(self.session,attempt)?;self.send(packet("kdeconnect.pair",json!({"pair":false})))?;}
   "unpair"=>{self.cast_clear_link();self.shared.cancel_peer_files(&self.view.id);self.shared.store.lock().unwrap().forget(&self.view.id)?;self.shared.phonekey.lock().unwrap().revoke_device(mono_ms(),&self.view.id);self.pair.unpair(self.session)?;self.send(packet("kdeconnect.pair",json!({"pair":false})))?;}
   "ping"=>{if !self.pair.authorized(){return Err("pair device before sending commands".into());}
    self.send(packet("kdeconnect.ping",json!({"message":"DES Connect"})))?;
   }
   _=>return Err("unsupported action".into()),
  }self.view.error=None;self.publish();Ok(())
 }
 fn received(&mut self,np:Packet)->Result<()> {
  if np.kind==cast::CAPABILITY {self.cast_received(&np);return Ok(());}
  if np.kind==phonekey::CAPABILITY {self.phonekey_received(&np);return Ok(());}
  if np.payload_size.is_some(){
   if !self.pair.authorized(){return Ok(());}
   if matches!(np.kind.as_str(),crate::media::MPRIS_REQUEST|crate::media::SYSTEM_VOLUME_REQUEST){self.view.error=Some("Media payloads are unsupported".into());self.publish();return Ok(());}
   if np.kind!="kdeconnect.share.request"{return Err("unsupported payload packet".into());}
   let result=(||->Result<()>{self.file_guard()?;
    if !self.outgoing_capabilities.iter().any(|cap|cap=="kdeconnect.share.request"){return Err("peer did not advertise sending files".into());}
    let peer=self.payload_peer()?;self.shared.with_transfers(|manager|manager.offer_receive(peer,&np).map(|_|()))?;Ok(())
   })();self.view.error=result.err().map(|error|format!("File offer refused: {error}"));self.publish();return Ok(());
  }
  if np.kind=="kdeconnect.pair" {
   let wants=np.body.get("pair").and_then(Value::as_bool).ok_or("invalid pairing packet")?;
   if !wants {self.cast_clear_link();self.shared.cancel_peer_files(&self.view.id);self.shared.store.lock().unwrap().forget(&self.view.id)?;self.shared.phonekey.lock().unwrap().revoke_device(mono_ms(),&self.view.id);self.pair.unpair(self.session)?;self.timestamp=None;}
   else if self.pair.state()==State::Outgoing {
    // A simultaneous v8 request carries a timestamp; only a bare response is consent.
    if np.body.contains_key("timestamp"){return Ok(());}
    let attempt=self.pair.pending().ok_or("no pairing attempt")?;self.pair.peer_accepts(self.session,attempt,self.now())?;
    if let Err(e)=self.persist(){self.pair.unpair(self.session)?;return Err(e);}
    self.request_battery()?;
   } else if self.pair.state()==State::Unpaired {
    let stamp=np.body.get("timestamp").and_then(Value::as_i64).ok_or("v8 pairing request missing timestamp")?;
    self.pair.incoming(self.session,stamp,wall(),self.now())?;self.timestamp=Some(stamp);
   }
   self.publish();return Ok(());
  }
  if matches!(np.kind.as_str(),crate::media::MPRIS_REQUEST|crate::media::SYSTEM_VOLUME_REQUEST){return self.media_request(np);}
  if !self.pair.authorized(){return Ok(());}
  match np.kind.as_str(){
   "kdeconnect.share.request"=>{self.view.error=Some("URL/text sharing is not enabled; only file offers supported".into());}
   "kdeconnect.ping"=>{self.view.last_ping=Some(wall());}
   "kdeconnect.battery"=>{if let Some(n)=np.body.get("currentCharge").and_then(Value::as_u64).filter(|n|*n<=100){self.view.battery=Some(n as u8);self.view.charging=np.body.get("isCharging").and_then(Value::as_bool).unwrap_or(false);}}
   _=>{},
  }self.publish();Ok(())
 }
 fn media_request(&mut self,np:Packet)->Result<()> {
  self.media_dispatched=true;
  let kind=np.kind.as_str();let response=if kind==crate::media::MPRIS_REQUEST{"kdeconnect.mpris"}else{"kdeconnect.systemvolume"};
  let advertised=if kind==crate::media::MPRIS_REQUEST{self.media_advertised}else{self.volume_advertised};
  let pin=self.pair.pinned();let shared=self.shared.clone();let id=self.view.id.clone();
  let capable=self.outgoing_capabilities.iter().any(|cap|cap==kind)&&self.incoming_capabilities.iter().any(|cap|cap==response);
  let guard=||media_request_guard(&shared,&id,pin,kind,advertised,capable);
  if let Err(error)=guard(){self.view.error=Some(error.to_string().chars().take(256).collect());self.publish();return Ok(());}
  let mut adapter=crate::media::MediaAdapter::default();
  let replies=adapter.handle_guarded(kind,&Value::Object(np.body.clone()),||guard().map_err(|e|crate::media::MediaError(e.to_string())));
  let replies=match replies{Ok(replies)if replies.len()<=4=>replies,_=>{self.view.error=Some("Media request failed or is unsupported; no successful reply".into());self.publish();return Ok(());}};
  for reply in replies{if let Err(error)=guard(){self.view.error=Some(error.to_string().chars().take(256).collect());self.publish();return Ok(());}self.send(packet(reply.kind,reply.body))?;}
  self.view.error=None;self.publish();Ok(())
 }
 fn replace(&mut self,replacement:Replacement)->Result<()> {
  self.cast_clear_link();
  // peer_worker only supplies replacements whose authenticated certificate
  // digest equals this route's pin. Routine same-link discovery refreshes must
  // not interrupt independent payload sockets or pending local consent.
  let preserve=preserve_files_on_refresh(self.ip.ip,replacement.ip.ip,self.file_guard().is_ok(),replacement.files_advertised,
   (&self.incoming_capabilities,&self.outgoing_capabilities),(&replacement.identity.incoming_capabilities,&replacement.identity.outgoing_capabilities));
  if !preserve{self.shared.cancel_peer_files(&self.view.id);}
  self.media_advertised=replacement.media_advertised;self.volume_advertised=replacement.volume_advertised;
  self.files_advertised=replacement.files_advertised;self.incoming_capabilities=replacement.identity.incoming_capabilities;self.outgoing_capabilities=replacement.identity.outgoing_capabilities;
  self.stream=replacement.stream;self.ip=replacement.ip;self.view.name=replacement.identity.name;self.view.kind=replacement.identity.device_type;self.write_failed=false;
  if self.pair.authorized(){self.request_battery()?;}self.publish();Ok(())
 }
 fn run(&mut self,rx:mpsc::Receiver<Job>,replacements:mpsc::Receiver<Replacement>)->Result<()> {
  if self.pair.authorized(){self.request_battery()?;}
  let mut pending=Vec::new();let mut frame_start=None;let mut bytes=[0;4096];let mut carry=VecDeque::new();
  loop {
   if self.shared.stop.load(Ordering::Acquire){return Ok(());}
   if let Ok(replacement)=replacements.try_recv(){self.replace(replacement)?;pending.clear();carry.clear();frame_start=None;}
   self.cast_tick()?;
   self.phonekey_tick()?;
   if self.pair.authorized(){self.unpaired_permit=None;}else{
    let limit=bounded_duration("DES_CONNECT_UNPAIRED_MS",90_000);let elapsed=self.started.elapsed();
    if elapsed>limit&&(self.pair.state()==State::Unpaired||elapsed>limit+Duration::from_secs(30)){return Err("unpaired session expired; discover again".into());}
    if self.unpaired_permit.is_none(){self.unpaired_permit=Some(UnpairedPermit::new(self.shared.clone())?);}
   }
   let state=self.pair.state();self.pair.expire(self.session,self.now());if state!=self.pair.state(){self.publish();}
   for _ in 0..8 {let Ok(job)=rx.try_recv()else{break};let result=self.command(&job).map_err(|e|e.to_string());if let Err(e)=&result{if job.op!="cast_input"{self.view.error=Some(e.clone());self.publish();}}let _=job.reply.try_send(result);if self.reconnect_requested{return Ok(()); }if self.write_failed{if let Ok(replacement)=replacements.recv_timeout(Duration::from_millis(300)){self.replace(replacement)?;pending.clear();carry.clear();frame_start=None;}else{return Err("TLS write failed".into());}}}
   self.media_dispatched=false;
   // Yield to local commands after each potentially blocking media request.
   // Keep at most one existing 4KiB read of raw bytes; do not drop coalesced
   // player/volume requests or read more network data while that carry remains.
   let read=if carry.is_empty(){self.stream.read(&mut bytes)}else{let n=carry.len().min(bytes.len());for slot in &mut bytes[..n]{*slot=carry.pop_front().unwrap();}Ok(n)};
   match read{
    Ok(0)=>{if let Ok(replacement)=replacements.recv_timeout(Duration::from_millis(300)){self.replace(replacement)?;pending.clear();carry.clear();frame_start=None;continue;}return Err("device disconnected".into());},
    Ok(n)=>{
     for (offset,b) in bytes[..n].iter().enumerate() {
      if frame_start.is_none(){frame_start=Some(Instant::now());}
      pending.push(*b);if pending.len()>MAX_FRAME{return Err("packet frame too large".into());}
      if *b==b'\n'{let np=decode(&pending)?;pending.clear();frame_start=None;self.received(np)?;if self.media_dispatched{carry.extend(bytes[offset+1..n].iter().copied());break;}}
     }
    }
    Err(e)if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut)=>{},
    Err(e)=>{if let Ok(replacement)=replacements.recv_timeout(Duration::from_millis(300)){self.replace(replacement)?;pending.clear();carry.clear();frame_start=None;continue;}return Err(e.into());},
   }
   if frame_start.is_some_and(|at:Instant|at.elapsed()>Duration::from_secs(5)){return Err("partial packet timed out".into());}
  }
 }
}
fn media_request_guard(shared:&Shared,id:&str,pin:Option<[u8;32]>,kind:&str,advertised:bool,capable:bool)->Result<()> {
 if shared.stop.load(Ordering::Acquire)||!shared.paired(id){return Err("media request refused: device must be paired and connected".into());}
 let enabled={let store=shared.store.lock().unwrap();if pin.is_none()||store.pin(id)?!=pin{return Err("media request refused: paired certificate changed".into());}store.peers.get(id).is_some_and(|peer|if kind==crate::media::MPRIS_REQUEST{peer.media_enabled}else{peer.volume_enabled})};
 if !enabled{return Err("media request refused: this permission is disabled".into());}
 if !advertised{return Err("media request refused: reconnect to apply permission".into());}
 if !capable{return Err("media request refused: peer lacks secure request/reply capabilities".into());}
 if !session_unlocked(){return Err("media request refused: session is locked or unavailable".into());}Ok(())
}

fn preserve_files_on_refresh(old_ip:IpAddr,new_ip:IpAddr,current_authorized:bool,new_advertised:bool,
 old_caps:(&[String],&[String]),new_caps:(&[String],&[String]))->bool {
 let share=|caps:(&[String],&[String])|(caps.0.iter().any(|cap|cap=="kdeconnect.share.request"),caps.1.iter().any(|cap|cap=="kdeconnect.share.request"));
 let old=share(old_caps);let new=share(new_caps);
 current_authorized&&new_advertised&&old_ip==new_ip&&old==new&&(new.0||new.1)
}

fn peer_worker(shared:Arc<Shared>,tcp:TcpStream,claim:Option<Packet>){
 let result=(||->Result<()>{let ip=AddressPermit::new(shared.clone(),tcp.peer_addr()?.ip())?;let (mut stream,mut id,cert,(mut files_advertised,mut media_advertised,mut volume_advertised))=handshake(&shared,tcp,claim)?;
  let digest=identity::peer_digest(&cert,&id.device_id)?;let pin=shared.store.lock().unwrap().pin(&id.device_id)?;
  let mut ip=Some(ip);
  let existing=shared.routes.lock().unwrap().get(&id.device_id).cloned();
  if let Some(route)=existing {
   if route.digest!=digest{return Err("active device certificate changed".into());}
   match route.replacement.try_send(Replacement{stream,identity:id,ip:ip.take().unwrap(),files_advertised,media_advertised,volume_advertised}){
    Ok(())=>return Ok(()),
    Err(mpsc::TrySendError::Full(_))=>return Err("device reconnect already pending".into()),
    Err(mpsc::TrySendError::Disconnected(replacement))=>{stream=replacement.stream;id=replacement.identity;ip=Some(replacement.ip);files_advertised=replacement.files_advertised;media_advertised=replacement.media_advertised;volume_advertised=replacement.volume_advertised;let mut routes=shared.routes.lock().unwrap();if routes.get(&id.device_id).is_some_and(|r|r.epoch==route.epoch){routes.remove(&id.device_id);}}
   }
  }
  let mut pair=Pairing::from_pin(pin);let session=pair.connected(digest)?;
  let epoch=shared.next.fetch_add(1,Ordering::Relaxed);let (tx,rx)=mpsc::sync_channel(8);let (replacement_tx,replacement_rx)=mpsc::sync_channel(1);
  {let mut routes=shared.routes.lock().unwrap();if routes.contains_key(&id.device_id){return Err("simultaneous device connection".into());}routes.insert(id.device_id.clone(),Route{epoch,digest,commands:tx,replacement:replacement_tx});}
  let view=Device{id:id.device_id.clone(),name:id.name,kind:id.device_type,connected:true,paired:pair.authorized(),pair_state:"unpaired".into(),attempt:None,verification_key:None,battery:None,charging:false,last_ping:None,error:None,files_enabled:false,files_reconnect_needed:false,media_enabled:false,volume_enabled:false,media_reconnect_needed:false,volume_reconnect_needed:false,wireless_cast:false,epoch};
  let mut actor=Actor{shared:shared.clone(),view,stream,cert,pair,session,started:Instant::now(),timestamp:None,write_failed:false,public_attempt:None,unpaired_permit:None,ip:ip.unwrap(),incoming_capabilities:id.incoming_capabilities,outgoing_capabilities:id.outgoing_capabilities,files_advertised,media_advertised,volume_advertised,reconnect_requested:false,media_dispatched:false,cast_id:None,last_cast_lease:Instant::now(),presence_seq:0,presence_answered:0,last_presence:Instant::now()};actor.publish();
  let result=actor.run(rx,replacement_rx);actor.cast_clear_link();let _=actor.stream.get_ref().shutdown(std::net::Shutdown::Both);shared.cancel_peer_files(&id.device_id);actor.pair.disconnected(session);
  let mut routes=shared.routes.lock().unwrap();if routes.get(&id.device_id).is_some_and(|r|r.epoch==epoch){routes.remove(&id.device_id);actor.view.connected=false;actor.view.wireless_cast=false;actor.view.paired=shared.store.lock().unwrap().peers.contains_key(&id.device_id);actor.view.pair_state="offline".into();actor.view.files_reconnect_needed=false;actor.view.media_reconnect_needed=false;actor.view.volume_reconnect_needed=false;actor.view.attempt=None;actor.view.verification_key=None;actor.view.error=result.as_ref().err().map(ToString::to_string);if actor.view.paired{shared.update(actor.view.clone());}else{shared.devices.lock().unwrap().remove(&id.device_id);}}
  result
 })();if let Err(e)=result{eprintln!("DES Connect session: {e}");}shared.active.fetch_sub(1,Ordering::Release);
}
fn start_peer(shared:&Arc<Shared>,tcp:TcpStream,claim:Option<Packet>){
 if shared.active.fetch_update(Ordering::AcqRel,Ordering::Relaxed,|n|(n<8).then_some(n+1)).is_ok(){let shared=shared.clone();thread::spawn(move||peer_worker(shared,tcp,claim));}
}
fn handle_ipc(shared:&Arc<Shared>,request:Request,udp:&UdpSocket,client:&UnixStream)->Value {
 if request.op.starts_with("phonekey_") {return phonekey_ipc(shared,&request,client);}
 if request.op=="usb_devices" {
  let result=if session_unlocked() {crate::screens::usb_devices()} else {Err("Unlock DES UI to choose a USB device".into())};
  let mut state=shared.snapshot();
  return match result {
   Ok(devices)=>{state["usb_devices"]=json!(devices);json!({"ok":true,"state":state})},
   Err(error)=>json!({"ok":false,"error":error,"state":state}),
  };
 }
 let result=(||->Result<()>{
  match request.op.as_str(){
   "state"=>{},
   "stop"=>{},
   "stop_screen"=>{
    let mut screens=shared.screens.lock().unwrap();
    shared.clear_cast(|_|true);
    screens.stop().map_err(|e|->Box<dyn std::error::Error+Send+Sync>{e.into()})?;
    *shared.screen_peer.lock().unwrap()=None;
   },
   "mirror_phone"|"extend"=>{
    let id=request.device_id.as_deref().ok_or("select a paired device first")?;
    if !shared.paired(id){return Err("device must be paired and connected before screen sharing".into());}
    if !session_unlocked(){return Err("screen sharing requires an unlocked DES UI session with available lock status".into());}
    let serial=request.args.get("serial").and_then(Value::as_str).ok_or("select the authorized USB device serial")?;
    let mut screens=shared.screens.lock().unwrap();
    if shared.cast.lock().unwrap().is_some(){return Err("stop the wireless cast before starting USB sharing".into());}
    let result=if request.op=="mirror_phone" {screens.start_phone_guarded(serial,request.args.get("control").and_then(Value::as_bool).unwrap_or(false),||screen_guard(shared,id,client))}else{
     let arg=|key:&str,default:u32|->Result<u32>{Ok(request.args.get(key).map(|v|v.as_u64().and_then(|v|u32::try_from(v).ok()).ok_or("invalid screen dimension/rate")).transpose()?.unwrap_or(default))};
     screens.start_extended_guarded(serial,arg("width",960)?,arg("height",600)?,arg("fps",15)?,||screen_guard(shared,id,client))
    };
    result.map_err(|e|->Box<dyn std::error::Error+Send+Sync>{e.into()})?;*shared.screen_peer.lock().unwrap()=Some(id.into());
   }
   "cancel_transfer"=>{
    let id=request.args.get("transfer_id").and_then(Value::as_str).ok_or("transfer_id required")?;
    let mut transfers=shared.transfers.lock().unwrap();let manager=transfers.as_mut().ok_or("unknown transfer")?;
    if let Some(peer)=request.device_id.as_deref(){if !manager.snapshots().iter().any(|view|view.id==id&&view.peer_id==peer){return Err("transfer does not belong to this device".into());}}
    manager.cancel(id)?;
   }
   "set_files"|"set_media"|"set_volume" if !shared.routes.lock().unwrap().contains_key(request.device_id.as_deref().unwrap_or(""))=>{
    if !session_unlocked(){return Err("permission changes require an unlocked DES UI session".into());}
    let id=request.device_id.as_deref().ok_or("device_id required")?;
    let enabled=request.args.get("enabled").and_then(Value::as_bool).ok_or("enabled must be a boolean")?;
    {let mut store=shared.store.lock().unwrap();match request.op.as_str(){"set_files"=>store.set_files_enabled(id,enabled)?,"set_media"=>store.set_media_enabled(id,enabled)?,_=>store.set_volume_enabled(id,enabled)?}}
    if request.op=="set_files"&&!enabled{shared.cancel_peer_files(id);}
    if let Some(view)=shared.devices.lock().unwrap().get_mut(id){match request.op.as_str(){"set_files"=>{view.files_enabled=enabled;view.files_reconnect_needed=false;},"set_media"=>{view.media_enabled=enabled;view.media_reconnect_needed=false;},_=>{view.volume_enabled=enabled;view.volume_reconnect_needed=false;}}}
   }
   "discover"=>{udp.send_to(&shared.identity().encode()?,"255.255.255.255:1716")?;}
   "unpair" if !shared.routes.lock().unwrap().contains_key(request.device_id.as_deref().unwrap_or(""))=>{let id=request.device_id.as_deref().ok_or("device_id required")?;shared.cancel_peer_files(id);shared.store.lock().unwrap().forget(id)?;shared.phonekey.lock().unwrap().revoke_device(mono_ms(),id);shared.devices.lock().unwrap().remove(id);}
   "pair"|"accept"|"reject"|"unpair"|"ping"|"set_files"|"set_media"|"set_volume"|"reconnect"|"send_file"|"accept_transfer"|"cast_phone"|"cast_input"=>{
    let id=request.device_id.as_deref().ok_or("device_id required")?;
    let route=shared.routes.lock().unwrap().get(id).cloned().ok_or("device is not connected")?;let tx=route.commands;
    let reconnect=request.op=="reconnect";
    let (reply,rx)=mpsc::sync_channel(1);tx.try_send(Job{op:request.op,attempt:request.attempt,args:request.args.clone(),reply,client:client.try_clone()?,expires_at:Instant::now()+Duration::from_secs(4)}).map_err(|_|"device command queue is full")?;
    rx.recv_timeout(Duration::from_secs(4)).map_err(|_|"device action timed out")?.map_err(|e|->Box<dyn std::error::Error+Send+Sync>{e.into()})?;
    if reconnect {
     let deadline=Instant::now()+Duration::from_secs(1);
     while shared.routes.lock().unwrap().get(id).is_some_and(|current|current.epoch==route.epoch){
      if Instant::now()>=deadline{return Err("reconnect close has not completed; refresh device status".into());}thread::sleep(Duration::from_millis(10));
     }
     if !shared.loopback{udp.send_to(&shared.identity().encode()?,"255.255.255.255:1716")?;}
    }
   }
   _=>return Err("unsupported operation".into()),
  }Ok(())
 })();match result {Ok(())=>json!({"ok":true,"state":shared.snapshot()}),Err(e)=>json!({"ok":false,"error":e.to_string(),"state":shared.snapshot()})}
}
struct SocketGuard {path:PathBuf,ino:u64}
impl Drop for SocketGuard {fn drop(&mut self){if fs::symlink_metadata(&self.path).is_ok_and(|m|m.ino()==self.ino){let _=fs::remove_file(&self.path);}}}
pub fn run(socket:&Path,state:&Path,name:&str)->Result<()> {run_options(socket,state,name,false)}
pub fn run_options(socket:&Path,state:&Path,name:&str,loopback:bool)->Result<()> {
 if name.trim().is_empty()||name.chars().count()>256||name.chars().any(char::is_control){return Err("invalid device name".into());}
 let store=Store::open(state)?;identity::private_dir(socket.parent().ok_or("socket parent missing")?)?;
 if socket.exists(){match connect_control(socket){Ok(_)=>return Err("DES Connect already running".into()),Err(e) if matches!(e.kind(),std::io::ErrorKind::ConnectionRefused|std::io::ErrorKind::NotFound)=>{},Err(e)=>return Err(format!("existing control socket is busy or unavailable: {e}").into())}let m=fs::symlink_metadata(socket)?;if m.uid()!=rustix::process::getuid().as_raw()||!std::os::unix::fs::FileTypeExt::is_socket(&m.file_type()){return Err("refusing to replace foreign socket path".into());}fs::remove_file(socket)?;}
 let ipc=UnixListener::bind(socket)?;fs::set_permissions(socket,fs::Permissions::from_mode(0o600))?;let _guard=SocketGuard{path:socket.into(),ino:fs::symlink_metadata(socket)?.ino()};ipc.set_nonblocking(true)?;
 let tcp=if loopback{TcpListener::bind(("127.0.0.1",0))?}else{(1716..=1764).find_map(|port|TcpListener::bind(("0.0.0.0",port)).ok()).ok_or("no available KDE Connect TCP port")?};tcp.set_nonblocking(true)?;let port=tcp.local_addr()?.port();
 let udp=UdpSocket::bind(if loopback{("127.0.0.1",0)}else{("0.0.0.0",1716)})?;udp.set_broadcast(true)?;udp.set_nonblocking(true)?;
 let audit=state.join("phonekey-audit.log");
 let devices=store.peers.iter().map(|(id,p)|(id.clone(),Device::offline(id.clone(),p.name.clone(),p.kind.clone(),p.files_enabled,p.media_enabled,p.volume_enabled))).collect();
 let mut random=[0;8];openssl::rand::rand_bytes(&mut random)?;let approval_seed=u64::from_be_bytes(random)&(u64::MAX>>1);
 let shared=Arc::new(Shared{id:store.id().into(),store:Mutex::new(store),ips:Mutex::new(Default::default()),devices:Mutex::new(devices),routes:Mutex::new(Default::default()),screens:Mutex::new(crate::screens::ScreenBridge::new()),screen_peer:Mutex::new(None),cast:Mutex::new(None),transfers:Mutex::new(None),loopback,stop:AtomicBool::new(false),phonekey:Mutex::new(phonekey::Manager::new(Some(audit))),next:AtomicU64::new(1),next_approval:AtomicU64::new(approval_seed),unpaired:AtomicUsize::new(0),active:AtomicUsize::new(0),ipc_active:AtomicUsize::new(0),name:name.into(),port});
 eprintln!("DES Connect listening on private control socket and LAN TCP {port}");
 let mut screen_check=Instant::now();let mut walkaway_check=Instant::now();let mut broadcast=Instant::now()-Duration::from_secs(31);let mut rate=HashMap::<IpAddr,Instant>::new();let mut data=[0;MAX_FRAME+1];
 while !shared.stop.load(Ordering::Acquire) {
  if !loopback&&broadcast.elapsed()>Duration::from_secs(30){let _=udp.send_to(&shared.identity().encode()?,"255.255.255.255:1716");broadcast=Instant::now();}
  for _ in 0..8 {let Ok((stream,addr))=tcp.accept()else{break};if local(addr.ip()){
   let now=Instant::now();rate.retain(|_,t|t.elapsed()<Duration::from_secs(30));
   if rate.get(&addr.ip()).is_some_and(|at|at.elapsed()<Duration::from_millis(250))||rate.len()>=64&&!rate.contains_key(&addr.ip()){continue;}
   rate.insert(addr.ip(),now);start_peer(&shared,stream,None);
  }}
  for _ in 0..8 {let Ok((n,addr))=udp.recv_from(&mut data)else{break};if n>MAX_FRAME||!local(addr.ip()){continue;}
   if rate.get(&addr.ip()).is_some_and(|t|t.elapsed()<Duration::from_secs(3)){continue;}rate.retain(|_,t|t.elapsed()<Duration::from_secs(30));if rate.len()>=64{continue;}
   let Ok(claim)=decode(&data[..n])else{continue};let Ok(id)=Identity::from_packet(&claim)else{continue};if id.device_id==shared.id||shared.routes.lock().unwrap().contains_key(&id.device_id){continue;}
   let Some(port)=id.tcp_port.filter(|p|(1716..=1764).contains(p))else{continue};rate.insert(addr.ip(),Instant::now());
   if shared.active.fetch_update(Ordering::AcqRel,Ordering::Relaxed,|n|(n<8).then_some(n+1)).is_ok() {let shared=shared.clone();let remote=SocketAddr::new(addr.ip(),port);thread::spawn(move||{match TcpStream::connect_timeout(&remote,Duration::from_secs(3)){Ok(tcp)=>peer_worker(shared,tcp,Some(claim)),Err(_)=>{shared.active.fetch_sub(1,Ordering::Release);}}});}
  }
  for _ in 0..8 {let Ok((mut stream,_))=ipc.accept()else{break};
   let peer=rustix::net::sockopt::socket_peercred(&stream)?;if peer.uid!=rustix::process::getuid(){continue;}
   if shared.ipc_active.fetch_update(Ordering::AcqRel,Ordering::Relaxed,|n|(n<8).then_some(n+1)).is_err(){continue;}
   let shared=shared.clone();let udp=udp.try_clone()?;thread::spawn(move||{
    let _=stream.set_read_timeout(Some(Duration::from_secs(2)));let _=stream.set_write_timeout(Some(Duration::from_secs(2)));
    if let Ok(line)=read_line(&mut stream,4096){
     match serde_json::from_slice::<Request>(&line) {
      Ok(request) if request.op=="screen_frame"=>{let bytes=screen_frame(&shared,&request,&stream);let _=stream.write_all(&bytes);},
      request=>{let mut stopping=false;let result=request.map(|r|{stopping=r.op=="stop";handle_ipc(&shared,r,&udp,&stream)}).unwrap_or_else(|_|json!({"ok":false,"error":"invalid request"}));if let Ok(mut bytes)=serde_json::to_vec(&result){bytes.push(b'\n');let _=stream.write_all(&bytes);}if stopping{shared.stop.store(true,Ordering::Release);}}
     }
    }
    shared.ipc_active.fetch_sub(1,Ordering::Release);
   });
  }
  if screen_check.elapsed()>Duration::from_millis(500) {screen_check=Instant::now();revocation_tick(&shared);}
  if walkaway_check.elapsed()>bounded_duration("DES_CONNECT_WALKAWAY_CHECK_MS",2000) {walkaway_check=Instant::now();phonekey_tick(&shared);}

  thread::sleep(Duration::from_millis(100));
 }
 shared.clear_cast(|_|true);let _=shared.screens.lock().unwrap().stop();shared.cancel_all_files();let transfers=shared.transfers.lock().unwrap().take();drop(transfers);Ok(())
}
fn revocation_tick(shared:&Shared) {
 // Screen revocation must not wait behind file source/destination preparation.
 let wireless=shared.cast.lock().unwrap().as_ref().map(|cast|(cast.id.clone(),cast.peer.clone(),cast.epoch,cast.expired(Instant::now())));
 if let Some((id,peer,epoch,expired))=wireless {
  if expired||!shared.cast_peer_authorized(&peer,epoch)||!session_unlocked(){shared.clear_cast(|cast|cast.id==id);}
 }
 let running=shared.screens.lock().unwrap().status().running;
 if running {let id=shared.screen_peer.lock().unwrap().clone();if id.as_deref().is_none_or(|id|!shared.paired(id))||!session_unlocked(){let _=shared.screens.lock().unwrap().stop();*shared.screen_peer.lock().unwrap()=None;}}
 // All normal manager operations now do filesystem preparation outside this
 // mutex. A contending short bookkeeping operation is retried on the next tick.
 let peers:Vec<_>={
  let Ok(mut transfers)=shared.transfers.try_lock()else{return};
  let Some(manager)=transfers.as_mut()else{return};
  manager.snapshots().into_iter().filter(|view|matches!(view.state.as_str(),"offered"|"connecting"|"waiting"|"transferring")).map(|view|view.peer_id).collect()
 };
 if !peers.is_empty(){
  let unlocked=session_unlocked();let revoked:Vec<_>=peers.into_iter().filter(|peer|!unlocked||!shared.files_allowed(peer)).collect();
  if let Ok(mut transfers)=shared.transfers.try_lock(){if let Some(manager)=transfers.as_mut(){for peer in revoked{manager.cancel_peer(&peer);}}}
 }
}

pub fn request(socket:&Path,request:&Request)->Result<Value> {request_within(socket,request,Duration::from_secs(5))}
/// A request whose answer waits on a person, such as a phone-key approval.
pub fn request_within(socket:&Path,request:&Request,patience:Duration)->Result<Value> {
 let mut stream=connect_control(socket)?;stream.set_read_timeout(Some(patience))?;stream.set_write_timeout(Some(Duration::from_secs(2)))?;
 let mut bytes=serde_json::to_vec(request)?;bytes.push(b'\n');stream.write_all(&bytes)?;
 Ok(serde_json::from_slice(&read_line(&mut stream,1024*1024)?)?)
}

/// One line to the shell's private control socket, and its one-line answer.
fn shell_line(command:&str)->Result<String> {
 let path=std::env::var_os("DES_UI_CONTROL_SOCKET").map(PathBuf::from).unwrap_or_else(||std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(||std::env::temp_dir().join(format!("des-ui-{}",rustix::process::getuid().as_raw()))).join("des-ui-control.sock"));
 use rustix::net::{socket_with,connect,AddressFamily,SocketType,SocketFlags,SocketAddrUnix};
 let fd=socket_with(AddressFamily::UNIX,SocketType::STREAM,SocketFlags::CLOEXEC|SocketFlags::NONBLOCK,None)?;connect(&fd,&SocketAddrUnix::new(&path)?)?;
 let mut stream=UnixStream::from(fd);stream.set_nonblocking(false)?;
 if rustix::net::sockopt::socket_peercred(&stream)?.uid!=rustix::process::getuid(){return Err("wrong shell owner".into());}
 stream.set_read_timeout(Some(Duration::from_millis(300)))?;stream.set_write_timeout(Some(Duration::from_millis(300)))?;stream.write_all(format!("{command}\n").as_bytes())?;
 Ok(String::from_utf8(read_line(&mut stream,256*1024)?)?)
}
/// Whether the session is locked, or `None` when there is no shell to say.
pub fn session_locked()->Option<bool> {
 let text=shell_line("state").ok()?;let state:Value=serde_json::from_str(text.strip_prefix("ok ")?).ok()?;
 state.get("locked").and_then(Value::as_bool)
}
/// Fail closed when the compositor/shell cannot establish lock state.
pub fn session_unlocked()->bool {session_locked()==Some(false)}
/// The same request `des-ui-ctl lock` makes. There is no counterpart that unlocks.
fn request_lock()->Result<()> {
 if shell_line("lock")?.trim_end()=="ok"{Ok(())}else{Err("the shell refused to lock".into())}
}
fn mono_ms()->u64 {static START:std::sync::OnceLock<Instant>=std::sync::OnceLock::new();START.get_or_init(Instant::now).elapsed().as_millis() as u64}
/// This session's login name: the only user a phone-key request may name.
fn login_name()->Result<String> {
 let uid=rustix::process::getuid().as_raw();
 let listed=fs::read_to_string("/etc/passwd").ok().and_then(|text|text.lines().find_map(|line|{
  let mut fields=line.split(':');let name=fields.next()?;fields.next()?;
  (fields.next()?.parse::<u32>().ok()?==uid).then(||name.to_owned())
 }));
 listed.or_else(||std::env::var("USER").ok()).filter(|name|phonekey::valid_user(name)).ok_or_else(||"cannot tell this session's login name".into())
}
fn phonekey_binding(shared:&Shared,id:&str)->Result<phonekey::Binding> {
 let store=shared.store.lock().unwrap();
 Ok(phonekey::Binding{desktop_id:shared.id.clone(),desktop_cert:store.certificate_digest()?,phone_id:id.into(),phone_cert:store.pin(id)?.ok_or("device is not paired")?})
}
fn phonekey_key(shared:&Shared,id:&str)->Option<phonekey::PublicKey> {
 shared.store.lock().unwrap().peers.get(id)?.phonekey.as_ref().and_then(|record|phonekey::PublicKey::from_base64(&record.public_key).ok())
}
fn walkaway_grace(peer:&identity::PeerRecord)->u64 {
 let seconds=if peer.walkaway_seconds==0{phonekey::WALKAWAY_DEFAULT}else{peer.walkaway_seconds}.clamp(phonekey::WALKAWAY_MIN,phonekey::WALKAWAY_MAX) as u64;
 // The override can only shorten the wait, which only locks sooner.
 bounded_duration("DES_CONNECT_WALKAWAY_MS",seconds*1000).as_millis() as u64
}
/// Lock the session when the walk-away phone has been gone for its grace period.
/// This is the whole of what presence can do.
fn phonekey_tick(shared:&Shared) {
 let target=shared.store.lock().unwrap().peers.values().find(|peer|peer.walkaway).map(|peer|(peer.id.clone(),walkaway_grace(peer)));
 let now=mono_ms();shared.phonekey.lock().unwrap().expire(now);
 let Some((id,grace))=target else{shared.phonekey.lock().unwrap().walkaway.forget();return;};
 let unlocked=session_locked()==Some(false);
 let lock=shared.phonekey.lock().unwrap().walkaway.tick(now,grace,unlocked);
 if lock {
  let result=request_lock();
  shared.phonekey.lock().unwrap().audit("walkaway_lock",json!({"device":id,"grace_seconds":grace/1000,"locked":result.is_ok()}));
  if let Err(error)=result{eprintln!("DES Connect walk-away lock: {error}");}
 }
}
fn phonekey_status(shared:&Shared)->Value {
 let peers:Vec<_>=shared.store.lock().unwrap().peers.values().cloned().collect();
 let connected:Vec<String>=shared.devices.lock().unwrap().values().filter(|device|device.connected&&device.paired).map(|device|device.id.clone()).collect();
 let now=mono_ms();let manager=shared.phonekey.lock().unwrap();
 let devices:Vec<Value>=peers.iter().map(|peer|{
  let key=peer.phonekey.as_ref().and_then(|record|phonekey::PublicKey::from_base64(&record.public_key).ok());
  let grace=walkaway_grace(peer);
  json!({"id":peer.id,"name":peer.name,"connected":connected.contains(&peer.id),
   "enrolled":key.is_some(),"key_id":key.as_ref().map(|key|identity::hex(&key.id())),"fingerprint":key.as_ref().map(|key|phonekey::fingerprint(&key.id())),"public_key":key.as_ref().map(|key|key.base64()),
   "unlock":key.is_some()&&peer.phonekey.as_ref().is_some_and(|record|record.unlock),"approve":key.is_some()&&peer.phonekey.as_ref().is_some_and(|record|record.approve),
   "walkaway":peer.walkaway,"walkaway_seconds":grace/1000,
   "present":peer.walkaway.then(||manager.walkaway.present(now,grace)),"last_seen_seconds":if peer.walkaway{manager.walkaway.last_seen().map(|at|now.saturating_sub(at)/1000)}else{None},
   "armed":peer.walkaway&&manager.walkaway.armed(),"locked_out":manager.locked_out(&peer.id,now)})
 }).collect();
 json!({"version":phonekey::VERSION,"capability":phonekey::CAPABILITY,"devices":devices})
}
/// Hand one phone-key packet to the device's own session thread.
fn phonekey_send(shared:&Shared,id:&str,body:Value,client:&UnixStream)->Result<()> {
 let route=shared.routes.lock().unwrap().get(id).cloned().ok_or("the phone is not connected")?;
 let (reply,rx)=mpsc::sync_channel(1);
 route.commands.try_send(Job{op:"phonekey_send".into(),attempt:None,args:BTreeMap::from([("body".to_owned(),body)]),reply,client:client.try_clone()?,expires_at:Instant::now()+Duration::from_secs(4)}).map_err(|_|"device command queue is full")?;
 rx.recv_timeout(Duration::from_secs(4)).map_err(|_|"device action timed out")?.map_err(Into::into)
}
/// Send a challenge and wait for the one decision it can have. The wait ends at
/// the deadline, or sooner if whoever asked goes away; either way the phone is told.
fn phonekey_exchange(shared:&Shared,id:&str,challenge:phonekey::Challenge,key_id:Option<[u8;32]>,client:&UnixStream)->Result<phonekey::Outcome> {
 let request_id=challenge.request_id;let patience=Duration::from_secs(challenge.ttl())+Duration::from_millis(250);let body=challenge.body(key_id.as_ref());
 shared.phonekey.lock().unwrap().issue(mono_ms(),id,challenge).map_err(|why|format!("phone-key request refused: {why}"))?;
 if let Err(error)=phonekey_send(shared,id,body,client){shared.phonekey.lock().unwrap().abandon(mono_ms(),&request_id,Reject::Cancelled);return Err(error);}
 let deadline=Instant::now()+patience;
 let outcome=loop {
  if let Some(outcome)=shared.phonekey.lock().unwrap().take(&request_id){break outcome;}
  let ended=if shared.stop.load(Ordering::Acquire)||!control_alive(client){Some(Reject::Cancelled)}else if Instant::now()>=deadline{Some(Reject::Timeout)}else{None};
  if let Some(why)=ended {
   let outcome=shared.phonekey.lock().unwrap().abandon(mono_ms(),&request_id,why);
   let _=phonekey_send(shared,id,json!({"v":phonekey::VERSION,"op":"cancel","request_id":identity::hex(&request_id),"reason":why.code()}),client);
   break outcome.ok_or("phone-key request was lost")?;
  }
  thread::sleep(Duration::from_millis(50));
 };
 if outcome.suspend {
  // Repeated forged answers: both signature capabilities go off until the user
  // turns them on again from an unlocked session.
  let _=shared.store.lock().unwrap().update_peer(id,|peer|{if let Some(key)=peer.phonekey.as_mut(){key.unlock=false;key.approve=false;}Ok(())});
  shared.phonekey.lock().unwrap().audit("suspended",json!({"device":id}));
 }
 Ok(outcome)
}
fn phonekey_ipc(shared:&Arc<Shared>,request:&Request,client:&UnixStream)->Value {
 let result=phonekey_op(shared,request,client);let status=phonekey_status(shared);
 match result {
  Ok(mut reply)=>{reply["ok"]=json!(true);reply["phonekey"]=status;reply},
  Err(error)=>json!({"ok":false,"error":error.to_string(),"phonekey":status}),
 }
}
fn phonekey_op(shared:&Arc<Shared>,request:&Request,client:&UnixStream)->Result<Value> {
 let text=|name:&str|request.args.get(name).and_then(Value::as_str);
 let device=||request.device_id.as_deref().ok_or("device_id required");
 match request.op.as_str() {
  "phonekey_status"=>Ok(json!({})),
  "phonekey_set"=>{
   if !session_unlocked(){return Err("phone-key settings can only be changed in an unlocked DES UI session".into());}
   let id=device()?;let capability=text("capability").unwrap_or("");
   let enabled=request.args.get("enabled").and_then(Value::as_bool).ok_or("enabled must be a boolean")?;
   let seconds=request.args.get("seconds").map(|value|value.as_u64().filter(|n|(phonekey::WALKAWAY_MIN as u64..=phonekey::WALKAWAY_MAX as u64).contains(n)).ok_or("seconds must be between 30 and 3600")).transpose()?;
   {
    let mut store=shared.store.lock().unwrap();
    if capability=="walkaway"&&enabled&&store.peers.values().any(|peer|peer.walkaway&&peer.id!=id){return Err("walk-away lock is already on for another device; turn that off first".into());}
    store.update_peer(id,|peer|{
     match capability {
      "walkaway"=>{peer.walkaway=enabled;if let Some(seconds)=seconds{peer.walkaway_seconds=seconds as u32;}else if peer.walkaway_seconds==0{peer.walkaway_seconds=phonekey::WALKAWAY_DEFAULT;}},
      "unlock"|"approve"=>{
       let key=peer.phonekey.as_mut().ok_or("enrol this phone's key first")?;
       if capability=="unlock"{key.unlock=enabled;}else{key.approve=enabled;}
      },
      _=>return Err("capability must be walkaway, unlock or approve".into()),
     }Ok(())
    })?;
   }
   let mut manager=shared.phonekey.lock().unwrap();if capability=="walkaway"{manager.walkaway.forget();}
   manager.audit("set",json!({"device":id,"capability":capability,"enabled":enabled}));Ok(json!({}))
  }
  "phonekey_revoke"=>{
   // Always allowed, locked or not: it can only take a capability away.
   let id=device()?;shared.store.lock().unwrap().update_peer(id,|peer|{peer.phonekey=None;Ok(())})?;
   shared.phonekey.lock().unwrap().revoke_device(mono_ms(),id);
   let _=phonekey_send(shared,id,json!({"v":phonekey::VERSION,"op":"revoke","reason":"desktop"}),client);Ok(json!({}))
  }
  "phonekey_enrol"=>{
   if !session_unlocked(){return Err("a phone can only be enrolled from an unlocked DES UI session".into());}
   let id=device()?;if !shared.paired(id){return Err("the phone must be paired and connected".into());}
   let user=login_name()?;
   let name:String=shared.name.chars().filter(|c|phonekey::valid_action(&c.to_string())||*c==' ').take(64).collect();
   let action=format!("Enrol this phone as a key for {user} on {}",name.trim());
   let challenge=phonekey::Challenge::new(Purpose::Enrol,&user,action.trim(),wall() as u64,phonekey::ENROL_TTL)?;
   let outcome=phonekey_exchange(shared,id,challenge,None,client)?;
   let phonekey::Decision::Approved{key,..}=&outcome.decision else{return Err(format!("enrolment was not completed: {}",outcome.word()).into());};
   // A new key starts with unlock and approve off, whatever the old one had.
   shared.store.lock().unwrap().update_peer(id,|peer|{peer.phonekey=Some(identity::PhoneKeyRecord{public_key:key.base64(),enrolled_at:wall(),unlock:false,approve:false});Ok(())})?;
   shared.phonekey.lock().unwrap().audit("enrolled",json!({"device":id,"key_id":identity::hex(&key.id())}));
   Ok(json!({"decision":"approved","device_id":id,"key_id":identity::hex(&key.id()),"fingerprint":phonekey::fingerprint(&key.id())}))
  }
  "phonekey_request"=>{
   let purpose=match text("purpose"){Some("unlock")=>Purpose::Unlock,Some("approve")=>Purpose::Approve,_=>return Err("purpose must be unlock or approve".into())};
   let wants=|peer:&identity::PeerRecord|peer.phonekey.as_ref().is_some_and(|key|if purpose==Purpose::Unlock{key.unlock}else{key.approve});
   let enabled:Vec<String>=shared.store.lock().unwrap().peers.values().filter(|peer|wants(peer)).map(|peer|peer.id.clone()).collect();
   let id=match request.device_id.as_deref() {
    Some(id)=>{if !enabled.iter().any(|known|known==id){return Err(format!("disabled: {} by phone is off for that device",purpose.as_str()).into());}id.to_owned()},
    None=>{
     let mut here:Vec<&String>=enabled.iter().filter(|id|shared.paired(id)).collect();
     match (enabled.len(),here.len()) {(0,_)=>return Err(format!("disabled: {} by phone is off",purpose.as_str()).into()),(_,0)=>return Err("the phone is not connected".into()),(_,1)=>here.remove(0).clone(),_=>return Err("more than one phone could answer; name one".into())}
    }
   };
   if !shared.paired(&id){return Err("the phone is not connected".into());}
   let key=phonekey_key(shared,&id).ok_or("no_key: this phone has no enrolled key")?;
   let me=login_name()?;let user=text("user").unwrap_or(&me);
   if user!=me {
    shared.phonekey.lock().unwrap().audit("refused",json!({"device":id,"purpose":purpose.as_str(),"user":user,"reason":Reject::WrongUser.code()}));
    return Err("wrong_user: a request can only name this session's own user".into());
   }
   // An approval is for someone at an unlocked desktop; an unlock is only for a locked one.
   match (purpose,session_locked()) {
    (Purpose::Approve,Some(false))|(Purpose::Unlock,Some(true))=>{},
    (Purpose::Approve,_)=>return Err("approvals need an unlocked DES UI session".into()),
    _=>return Err("unlock requests need a locked DES UI session".into()),
   }
   let action=text("action").ok_or("action text required")?;let now=wall() as u64;
   let ttl=request.args.get("ttl").map(|value|value.as_u64().filter(|n|(5..=phonekey::MAX_TTL).contains(n)).ok_or("ttl must be between 5 and 120 seconds")).transpose()?.unwrap_or(phonekey::REQUEST_TTL);
   let challenge=match phonekey::Challenge::from_fields(purpose,user,action,&request.args)? {
    Some(own)=>{if own.expires_at<=now||own.issued_at>now+60{return Err("expired: the caller's challenge is not current".into());}own},
    None=>phonekey::Challenge::new(purpose,user,action,now,ttl)?,
   };
   let binding=phonekey_binding(shared,&id)?;let sent=challenge.clone();
   let outcome=phonekey_exchange(shared,&id,challenge,Some(key.id()),client)?;
   // Everything a verifier needs to check the signature for itself.
   let mut reply=sent.body(Some(&key.id()));reply.as_object_mut().unwrap().remove("op");
   reply["decision"]=json!(outcome.word());reply["device_id"]=json!(id);reply["public_key"]=json!(key.base64());
   reply["desktop_id"]=json!(binding.desktop_id);reply["desktop_cert"]=json!(identity::hex(&binding.desktop_cert));reply["phone_cert"]=json!(identity::hex(&binding.phone_cert));
   if let phonekey::Decision::Approved{signature,..}=&outcome.decision {use base64::Engine;reply["signature"]=json!(base64::engine::general_purpose::STANDARD.encode(signature));}
   Ok(reply)
  }
  _=>Err("unsupported operation".into()),
 }
}

/// Check live authorization on both sides of acquiring a frame. A revocation
/// during decode/snapshotting must never release the cached pixels.
fn guarded_preview(read: impl FnOnce() -> std::result::Result<Option<Vec<u8>>, String>, mut allowed: impl FnMut() -> bool) -> Vec<u8> {
 if !allowed() { return b"LOCKED\n".to_vec(); }
 let frame=read();
 if !allowed() { return b"LOCKED\n".to_vec(); }
 match frame { Ok(Some(bytes))=>bytes, Ok(None)=>b"WAIT\n".to_vec(), Err(_)=>b"ERROR\n".to_vec() }
}

fn screen_session_matches(status:&crate::screens::ScreenStatus, owner:Option<&str>, expected_owner:&str, serial:&str)->bool {
 status.running && status.mode=="phone" && status.serial.as_deref()==Some(serial) && owner==Some(expected_owner)
}

fn wireless_frame(shared:&Shared,id:&str,serial:&str,epoch:u64,client:&UnixStream)->Vec<u8> {
 let trusted=||shared.cast_peer_authorized(id,epoch)&&session_unlocked();
 let revoke=||{shared.clear_cast(|cast|cast.matches(id,serial));b"LOCKED\n".to_vec()};
 if !control_alive(client){return b"STOPPED\n".to_vec();}
 if !trusted(){return revoke();}
 let frame={
  let mut slot=shared.cast.lock().unwrap();
  let Some(cast)=slot.as_mut().filter(|cast|cast.matches(id,serial)) else{return b"STOPPED\n".to_vec();};
  let now=Instant::now();
  if !cast.renew(now){*slot=None;return b"STOPPED\n".to_vec();}
  cast.frame(now)
 };
 if !trusted(){return revoke();}
 if !control_alive(client){return b"STOPPED\n".to_vec();}
 let generation=frame.as_ref().map(|(generation,_)|*generation);
 let permitted=shared.cast.lock().unwrap().as_ref().is_some_and(|cast|cast.matches(id,serial)&&cast.can_release(generation,Instant::now()));
 if !permitted{return b"WAIT\n".to_vec();}
 frame.map(|(_,bytes)|bytes).unwrap_or_else(||b"WAIT\n".to_vec())
}

fn screen_frame(shared:&Shared,request:&Request,client:&UnixStream)->Vec<u8> {
 let Some(id)=request.device_id.as_deref() else {return b"ERROR\n".to_vec();};
 let Some(serial)=request.args.get("serial").and_then(Value::as_str) else {return b"ERROR\n".to_vec();};
 let wireless=shared.cast.lock().unwrap().as_ref().filter(|cast|cast.matches(id,serial)).map(|cast|cast.epoch);
 if let Some(epoch)=wireless{return wireless_frame(shared,id,serial,epoch,client);}
 let bound=|screens:&mut crate::screens::ScreenBridge| {
  let status=screens.status();
  screen_session_matches(&status,shared.screen_peer.lock().unwrap().as_deref(),id,serial)
 };
 if cast::valid_id(serial)&&!bound(&mut shared.screens.lock().unwrap()){
  return b"STOPPED\n".to_vec();
 }
 guarded_preview(|| {
  let mut screens=shared.screens.lock().unwrap();
  if !bound(&mut screens) {return Err("screen session changed".into());}
  screens.preview_frame()
 }, || screen_guard(shared,id,client).is_ok() && bound(&mut shared.screens.lock().unwrap()))
}

fn screen_guard(shared:&Shared,id:&str,client:&UnixStream)->std::result::Result<(),String>{
 if !control_alive(client){return Err("screen request was cancelled before capture started".into());}
 if !shared.paired(id){return Err("device disconnected or unpaired during screen preparation".into());}
 if !session_unlocked(){return Err("session locked or lock status unavailable during screen preparation".into());}Ok(())
}
fn bounded_duration(name:&str,maximum:u64)->Duration {Duration::from_millis(std::env::var(name).ok().and_then(|v|v.parse::<u64>().ok()).unwrap_or(maximum).clamp(250,maximum))}
struct HandshakeDeadline(SyncSender<()>);
impl HandshakeDeadline {
 fn new(tcp:&TcpStream)->Result<Self>{let clone=tcp.try_clone()?;let (tx,rx)=mpsc::sync_channel(1);let timeout=bounded_duration("DES_CONNECT_HANDSHAKE_MS",8000);thread::spawn(move||{if rx.recv_timeout(timeout).is_err(){let _=clone.shutdown(std::net::Shutdown::Both);}});Ok(Self(tx))}
}
impl Drop for HandshakeDeadline {fn drop(&mut self){let _=self.0.try_send(());}}
struct UnpairedPermit(Arc<Shared>);
impl UnpairedPermit {fn new(shared:Arc<Shared>)->Result<Self>{shared.unpaired.fetch_update(Ordering::AcqRel,Ordering::Relaxed,|n|(n<6).then_some(n+1)).map_err(|_|"unpaired device limit reached; paired capacity reserved")?;Ok(Self(shared))}}
impl Drop for UnpairedPermit {fn drop(&mut self){self.0.unpaired.fetch_sub(1,Ordering::Release);}}

struct AddressPermit {shared:Arc<Shared>,ip:IpAddr}
impl AddressPermit {fn new(shared:Arc<Shared>,ip:IpAddr)->Result<Self>{let mut ips=shared.ips.lock().unwrap();let count=ips.entry(ip).or_default();if *count>=2{return Err("per-address connection limit reached".into());}*count+=1;drop(ips);Ok(Self{shared,ip})}}
impl Drop for AddressPermit {fn drop(&mut self){let mut ips=self.shared.ips.lock().unwrap();if let Some(n)=ips.get_mut(&self.ip){*n-=1;if *n==0{ips.remove(&self.ip);}}}}

fn control_alive(client:&UnixStream)->bool {
 let mut byte=[0;1];match rustix::net::recv(client,&mut byte,rustix::net::RecvFlags::PEEK|rustix::net::RecvFlags::DONTWAIT){Ok((_,0))=>false,Ok(_)=>true,Err(rustix::io::Errno::AGAIN)=>true,Err(_)=>false}
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn preview_is_bound_to_the_requested_owner_and_usb_session() {
  let status=crate::screens::ScreenStatus {mode:"phone".into(),serial:Some("USB-A".into()),running:true,control:false,message:String::new()};
  assert!(screen_session_matches(&status,Some("peer-a"),"peer-a","USB-A"));
  assert!(!screen_session_matches(&status,Some("peer-b"),"peer-a","USB-A"));
  assert!(!screen_session_matches(&status,Some("peer-a"),"peer-a","USB-B"));
  assert!(!screen_session_matches(&status,None,"peer-a","USB-A"));
  let stopped=crate::screens::ScreenStatus {running:false,..status.clone()};
  assert!(!screen_session_matches(&stopped,Some("peer-a"),"peer-a","USB-A"));
 }
 #[test] fn preview_denies_before_reading_or_after_policy_changes() {
  assert_eq!(guarded_preview(|| panic!("locked capture was read"), || false), b"LOCKED\n");
  let mut checks=0;
  assert_eq!(guarded_preview(|| Ok(Some(b"P6\n1 1\n255\nsecret".to_vec())), || {checks+=1;checks==1}),b"LOCKED\n");
 }
 #[test] fn preview_waits_without_a_frame_and_reports_reader_failure() {
  assert_eq!(guarded_preview(|| Ok(None), || true),b"WAIT\n");
  assert_eq!(guarded_preview(|| Err("decoder stopped".into()), || true),b"ERROR\n");
  let frame=b"P6\n1 1\n255\nabc".to_vec();
  assert_eq!(guarded_preview(|| Ok(Some(frame.clone())), || true),frame);
 }
 #[test] fn transfer_refresh_rejects_address_permission_advertisement_or_share_capability_change(){
  let a:IpAddr="127.0.0.1".parse().unwrap();let b:IpAddr="127.0.0.2".parse().unwrap();
  let share=vec!["kdeconnect.share.request".into()];let core=vec!["kdeconnect.ping".into()];
  let allowed=(&share[..],&share[..]);let revoked=(&core[..],&share[..]);
  assert!(preserve_files_on_refresh(a,a,true,true,allowed,allowed));
  let unrelated=vec!["kdeconnect.share.request".into(),"kdeconnect.mpris.request".into()];assert!(preserve_files_on_refresh(a,a,true,true,allowed,(&unrelated,&share)));
  assert!(!preserve_files_on_refresh(a,b,true,true,allowed,allowed));
  assert!(!preserve_files_on_refresh(a,a,false,true,allowed,allowed));
  assert!(!preserve_files_on_refresh(a,a,true,false,allowed,allowed));
  assert!(!preserve_files_on_refresh(a,a,true,true,allowed,revoked));
 }
 #[test] fn disconnected_control_client_cannot_start_a_capture(){let (server,client)=UnixStream::pair().unwrap();assert!(control_alive(&server));drop(client);assert!(!control_alive(&server));}
 #[test] fn disconnected_file_command_is_rejected_before_content_start(){
  let(server,client)=UnixStream::pair().unwrap();let(reply,_)=mpsc::sync_channel(1);
  let job=Job{op:"send_file".into(),attempt:None,args:Default::default(),reply,client:server,expires_at:Instant::now()+Duration::from_secs(4)};
  assert!(job.content_guard().is_ok());drop(client);assert!(job.content_guard().unwrap_err().to_string().contains("cancelled"));
 }
 #[test] fn expired_queued_file_command_is_rejected_even_with_live_client(){
  let(server,_client)=UnixStream::pair().unwrap();let(reply,_)=mpsc::sync_channel(1);
  let job=Job{op:"accept_transfer".into(),attempt:None,args:Default::default(),reply,client:server,expires_at:Instant::now()-Duration::from_millis(1)};
  assert!(job.content_guard().unwrap_err().to_string().contains("expired"));
 }
 #[test] fn file_manager_contention_cannot_delay_locked_screen_revocation(){
  use std::process::{Command,Stdio};
  if let Some(base)=std::env::var_os("DES_CONNECT_WATCHDOG_FIXTURE"){
   let base=PathBuf::from(base);let peer=Store::open(&base.join("peer")).unwrap();let mut store=Store::open(&base.join("local")).unwrap();store.remember(peer.id(),"Fixture","phone",&peer.certificate).unwrap();
   let mut device=Device::offline(peer.id().into(),"Fixture".into(),"phone".into(),false,false,false);device.connected=true;
   let listener=UnixListener::bind(base.join("shell.sock")).unwrap();
   let shell=thread::spawn(move||{let(mut stream,_)=listener.accept().unwrap();let mut bytes=[0;6];stream.read_exact(&mut bytes).unwrap();stream.write_all(b"ok {\"locked\":true}\n").unwrap();});
   let mut screens=crate::screens::ScreenBridge::new();screens.start_phone_guarded("TEST",false,||Ok(())).unwrap();
   let shared=Arc::new(Shared{id:store.id().into(),store:Mutex::new(store),ips:Mutex::new(Default::default()),devices:Mutex::new(BTreeMap::from([(peer.id().into(),device)])),routes:Mutex::new(Default::default()),screens:Mutex::new(screens),screen_peer:Mutex::new(Some(peer.id().into())),cast:Mutex::new(None),transfers:Mutex::new(None),loopback:true,stop:AtomicBool::new(false),phonekey:Mutex::new(phonekey::Manager::new(None)),next:AtomicU64::new(1),next_approval:AtomicU64::new(1),unpaired:AtomicUsize::new(0),active:AtomicUsize::new(0),ipc_active:AtomicUsize::new(0),name:"Fixture".into(),port:0});
   let stalled=shared.transfers.lock().unwrap();let(tx,rx)=mpsc::sync_channel(1);let worker=shared.clone();
   let watchdog=thread::spawn(move||{revocation_tick(&worker);tx.send(()).unwrap();});
   let prompt=rx.recv_timeout(Duration::from_secs(1)).is_ok();drop(stalled);watchdog.join().unwrap();shell.join().unwrap();
   assert!(prompt,"file-manager contention blocked the locked-screen watchdog");assert!(!shared.screens.lock().unwrap().status().running);return;
  }
  let temp=tempfile::tempdir().unwrap();let adb=temp.path().join("adb");let viewer=temp.path().join("viewer");let ffmpeg=temp.path().join("ffmpeg");
  fs::write(temp.path().join("scrcpy-server"),b"fixture server").unwrap();
  fs::write(&adb,b"#!/bin/sh\ncase \"$4\" in CLASSPATH=*) exec /bin/sleep 20;; esac\nprintf 'usb:fixture\\n'\n").unwrap();fs::write(&viewer,b"#!/bin/sh\nexec /bin/sleep 20\n").unwrap();fs::write(&ffmpeg,b"#!/bin/sh\nexec /bin/sleep 20\n").unwrap();
  for path in [&adb,&viewer,&ffmpeg]{fs::set_permissions(path,fs::Permissions::from_mode(0o700)).unwrap();}
  let result=Command::new(std::env::current_exe().unwrap()).args(["--exact","service::tests::file_manager_contention_cannot_delay_locked_screen_revocation","--nocapture"]).env("DES_CONNECT_WATCHDOG_FIXTURE",temp.path()).env("DES_CONNECT_SCRCPY",viewer).env("DES_UI_CONTROL_SOCKET",temp.path().join("shell.sock")).env("PATH",format!("{}:/usr/bin:/bin",temp.path().display())).stdin(Stdio::null()).output().unwrap();
  assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stdout));
 }

}

fn connect_control(path:&Path)->std::io::Result<UnixStream>{
 use rustix::net::{socket_with,connect,AddressFamily,SocketType,SocketFlags,SocketAddrUnix};
 let fd=socket_with(AddressFamily::UNIX,SocketType::STREAM,SocketFlags::CLOEXEC|SocketFlags::NONBLOCK,None)?;
 connect(&fd,&SocketAddrUnix::new(path)?)?;let stream=UnixStream::from(fd);stream.set_nonblocking(false)?;Ok(stream)
}
