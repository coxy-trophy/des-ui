//! Disposable daemon/control/payload TLS fixtures. No real device or user file.
use des_connect::{identity::{self, Store}, packet::Packet, service::{self, Request}, transfers::{PeerTls, TransferManager}};
use openssl::ssl::{Ssl, SslContextBuilder, SslMethod, SslStream, SslVerifyMode, SslVersion};
use serde_json::{json, Value};
use std::{fs, io::{Read, Write}, net::{Ipv4Addr, TcpStream}, os::unix::{fs::PermissionsExt, net::UnixListener}, path::{Path, PathBuf}, process::{Child, Command, Stdio}, sync::{Arc, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
struct Daemon(Child);
impl Drop for Daemon { fn drop(&mut self) { let _=self.0.kill();let _=self.0.wait(); } }
struct Shell { path:PathBuf, locked:Arc<AtomicBool>, stop:Arc<AtomicBool>, handle:Option<thread::JoinHandle<()>> }
impl Shell {
 fn new(path:PathBuf)->Self {
  let listener=UnixListener::bind(&path).unwrap();listener.set_nonblocking(true).unwrap();
  let locked=Arc::new(AtomicBool::new(false));let stop=Arc::new(AtomicBool::new(false));let lock=locked.clone();let done=stop.clone();
  let handle=thread::spawn(move||while !done.load(Ordering::Acquire){
   if let Ok((mut stream,_))=listener.accept(){let _=stream.set_read_timeout(Some(Duration::from_millis(100)));let mut request=[0;6];if stream.read_exact(&mut request).is_ok(){assert_eq!(&request,b"state\n");let body=format!("ok {{\"locked\":{}}}\n",lock.load(Ordering::Acquire));let _=stream.write_all(body.as_bytes());}}
   else{thread::sleep(Duration::from_millis(5));}
  });Self{path,locked,stop,handle:Some(handle)}
 }
}
impl Drop for Shell { fn drop(&mut self){self.stop.store(true,Ordering::Release);if let Some(handle)=self.handle.take(){handle.join().unwrap();}} }
struct Fixture { _daemon:Daemon,shell:Shell,temp:tempfile::TempDir,socket:PathBuf,state:PathBuf,home:PathBuf,port:u16,phone:Store,desktop:Store }
impl Fixture {
 fn new()->Self {Self::with_music(None)}
 fn with_music(music:Option<&Path>)->Self {
  let temp=tempfile::tempdir().unwrap();fs::set_permissions(temp.path(),fs::Permissions::from_mode(0o700)).unwrap();
  let state=temp.path().join("state");let home=temp.path().join("home");fs::create_dir(&home).unwrap();fs::set_permissions(&home,fs::Permissions::from_mode(0o700)).unwrap();
  let phone=Store::open(&temp.path().join("phone")).unwrap();let mut desktop=Store::open(&state).unwrap();desktop.remember(phone.id(),"Fixture phone","phone",&phone.certificate).unwrap();
  let socket=temp.path().join("runtime/control.sock");let shell=Shell::new(temp.path().join("shell.sock"));
  let mut command=Command::new(env!("CARGO_BIN_EXE_des-connectd"));command.arg("--loopback").env("HOME",&home).env("DES_UI_CONTROL_SOCKET",&shell.path).env("DBUS_SESSION_BUS_ADDRESS","unix:path=/nonexistent/des-connect-fixture-bus").args(["--socket",socket.to_str().unwrap(),"--state-dir",state.to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null());
  if let Some(path)=music{command.env("DES_MUSIC_SOCKET",path);}
  let daemon=Daemon(command.spawn().unwrap());
  let ready=wait(&socket,|state|state["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
  Self{_daemon:daemon,shell,temp,socket,state,home,port,phone,desktop}
 }
 fn call(&self,op:&str,args:Value)->Value {service::request(&self.socket,&Request{op:op.into(),device_id:Some(self.phone.id().into()),attempt:None,args:serde_json::from_value(args).unwrap()}).unwrap()}
 fn connect(&self)->(SslStream<TcpStream>,Packet){connect(self.port,&self.phone,&self.desktop,true)}
 fn enable_and_reconnect(&self,stream:SslStream<TcpStream>)->SslStream<TcpStream> {
  let changed=self.call("set_files",json!({"enabled":true}));assert_eq!(changed["ok"],true,"{changed}");assert_eq!(device(&changed,self.phone.id())["files_reconnect_needed"],true);
  assert_eq!(self.call("reconnect",json!({}))["ok"],true);drop(stream);
  wait(&self.socket,|state|device(state,self.phone.id())["connected"]==false);thread::sleep(Duration::from_millis(300));
  let (mut stream,identity)=self.connect();assert!(has_share(&identity));assert_eq!(read(&mut stream).kind,"kdeconnect.battery.request");
  wait(&self.socket,|state|device(state,self.phone.id())["connected"]==true&&device(state,self.phone.id())["files_reconnect_needed"]==false);stream
 }
 fn payload_peer(&self)->PeerTls {PeerTls::new(self.phone.key.clone(),self.phone.certificate.clone(),self.desktop.id().into(),self.desktop.certificate.clone(),identity::peer_digest(&self.desktop.certificate,self.desktop.id()).unwrap(),Ipv4Addr::LOCALHOST.into()).unwrap()}
 fn phone_files(&self,name:&str)->TransferManager {TransferManager::new(self.temp.path().join(name)).unwrap()}
 fn source(&self,name:&str)->PathBuf {let path=self.temp.path().join(name);fs::write(&path,b"disposable fixture bytes").unwrap();path}
}
fn device<'a>(state:&'a Value,id:&str)->&'a Value {state["state"]["devices"].as_array().unwrap().iter().find(|view|view["id"]==id).unwrap()}
fn wait(socket:&Path,predicate:impl Fn(&Value)->bool)->Value {let end=Instant::now()+Duration::from_secs(10);loop{if let Ok(state)=service::request(socket,&Request{op:"state".into(),device_id:None,attempt:None,args:Default::default()}){if predicate(&state){return state;}assert!(Instant::now()<end,"state deadline: {state}");}else{assert!(Instant::now()<end);}thread::sleep(Duration::from_millis(20));}}
fn read(stream:&mut impl Read)->Packet {let mut bytes=Vec::new();loop{let mut b=[0];stream.read_exact(&mut b).unwrap();bytes.push(b[0]);assert!(bytes.len()<65536);if b[0]==b'\n'{return Packet::read(&mut std::io::Cursor::new(bytes)).unwrap();}}}
fn send(stream:&mut impl Write,packet:Packet){stream.write_all(&packet.encode().unwrap()).unwrap();}
fn has_share(identity:&Packet)->bool { ["incomingCapabilities","outgoingCapabilities"].iter().all(|key|identity.body[*key].as_array().unwrap().iter().any(|cap|cap=="kdeconnect.share.request")) }
fn connect(port:u16,phone:&Store,desktop:&Store,files:bool)->(SslStream<TcpStream>,Packet){connect_with_caps(port,phone,desktop,files,true,true)}
fn connect_with_caps(port:u16,phone:&Store,desktop:&Store,files:bool,media:bool,volume:bool)->(SslStream<TcpStream>,Packet){
 let mut incoming=vec!["kdeconnect.ping","kdeconnect.battery.request"];let mut outgoing=vec!["kdeconnect.ping","kdeconnect.battery"];if files{incoming.push("kdeconnect.share.request");outgoing.push("kdeconnect.share.request");}
 if media{incoming.push("kdeconnect.mpris");outgoing.push("kdeconnect.mpris.request");}if volume{incoming.push("kdeconnect.systemvolume");outgoing.push("kdeconnect.systemvolume.request");}
 let identity=service::packet("kdeconnect.identity",json!({"deviceId":phone.id(),"deviceName":"Fixture phone","deviceType":"phone","protocolVersion":8,"incomingCapabilities":incoming,"outgoingCapabilities":outgoing}));
 let mut tcp=TcpStream::connect((Ipv4Addr::LOCALHOST,port)).unwrap();tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();send(&mut tcp,identity.clone());
 let mut context=SslContextBuilder::new(SslMethod::tls()).unwrap();context.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_max_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_certificate(&phone.certificate).unwrap();context.set_private_key(&phone.key).unwrap();let pin=identity::peer_digest(&desktop.certificate,desktop.id()).unwrap();let id=desktop.id().to_owned();context.set_verify_callback(SslVerifyMode::PEER|SslVerifyMode::FAIL_IF_NO_PEER_CERT,move|_,verify|verify.current_cert().is_some_and(|cert|identity::peer_digest(&cert.to_owned(),&id).is_ok_and(|digest|digest==pin)));
 let mut stream=SslStream::new(Ssl::new(&context.build()).unwrap(),tcp).unwrap();stream.accept().unwrap();send(&mut stream,identity);let remote=read(&mut stream);assert_eq!(remote.kind,"kdeconnect.identity");(stream,remote)
}
fn transfer<'a>(state:&'a Value,id:&str)->Option<&'a Value>{state["state"]["transfers"].as_array()?.iter().find(|view|view["id"]==id)}

#[test] fn file_policy_requires_explicit_reconnect_and_encrypted_peer_capability_then_sends_real_payload(){
 let fixture=Fixture::new();let(mut stream,identity)=fixture.connect();assert!(!has_share(&identity));assert_eq!(read(&mut stream).kind,"kdeconnect.battery.request");
 let source=fixture.source("send.txt");assert_eq!(fixture.call("send_file",json!({"path":source}))["ok"],false);
 let change=fixture.call("set_files",json!({"enabled":true}));assert_eq!(change["ok"],true);assert_eq!(device(&change,fixture.phone.id())["files_reconnect_needed"],true);
 assert_eq!(fixture.call("send_file",json!({"path":source}))["ok"],false);
 stream=fixture.enable_and_reconnect(stream);
 send(&mut stream,service::packet("kdeconnect.share.request",json!({"url":"https://fixture.invalid/private-content"})));
 let refusal=wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("URL/text sharing is not enabled")));
 assert!(!serde_json::to_string(&refusal).unwrap().contains("fixture.invalid"));assert_eq!(refusal["state"]["transfers"].as_array().unwrap().len(),0);
 let sent=fixture.call("send_file",json!({"path":source}));assert_eq!(sent["ok"],true,"{sent}");let header=read(&mut stream);assert_eq!(header.kind,"kdeconnect.share.request");
 let mut receiver=fixture.phone_files("phone-received");let offer=receiver.offer_receive(fixture.payload_peer(),&header).unwrap();receiver.accept(&offer.id).unwrap();
 let end=Instant::now()+Duration::from_secs(5);loop{let status=receiver.snapshots().into_iter().find(|view|view.id==offer.id).unwrap();if status.state=="complete"{assert_eq!(fs::read(status.path.unwrap()).unwrap(),b"disposable fixture bytes");break;}assert!(!matches!(status.state.as_str(),"failed"|"cancelled"),"{status:?}");assert!(Instant::now()<end);thread::sleep(Duration::from_millis(10));}
 let pin=Store::open(&fixture.state).unwrap().pin(fixture.phone.id()).unwrap();assert!(Store::open(&fixture.state).unwrap().peers[fixture.phone.id()].files_enabled);
 assert_eq!(fixture.call("reconnect",json!({}))["ok"],true);drop(stream);wait(&fixture.socket,|state|device(state,fixture.phone.id())["connected"]==false);thread::sleep(Duration::from_millis(300));
 let(mut no_files,_)=connect(fixture.port,&fixture.phone,&fixture.desktop,false);read(&mut no_files);
 let refused=fixture.call("send_file",json!({"path":source}));assert_eq!(refused["ok"],false);assert!(refused["error"].as_str().unwrap().contains("advertise"));assert_eq!(Store::open(&fixture.state).unwrap().pin(fixture.phone.id()).unwrap(),pin);
}

#[test] fn incoming_files_need_local_accept_and_are_cancelled_on_lock_disable_and_disconnect(){
 let fixture=Fixture::new();let(mut stream,_)=fixture.connect();read(&mut stream);stream=fixture.enable_and_reconnect(stream);
 let source=fixture.source("incoming.txt");let mut sender=fixture.phone_files("phone-sent");
 let offer=sender.send(fixture.payload_peer(),&source).unwrap();send(&mut stream,offer.packet);
 let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().is_some_and(|views|views.iter().any(|view|view["state"]=="offered")));let incoming=state["state"]["transfers"][0]["id"].as_str().unwrap().to_owned();
 assert_eq!(fs::read_dir(fixture.home.join("Downloads/DESConnect")).unwrap().count(),0);assert_eq!(sender.snapshots()[0].transferred,0);
 fixture.shell.locked.store(true,Ordering::Release);wait(&fixture.socket,|state|transfer(state,&incoming).is_some_and(|view|view["state"]=="cancelled"));
 assert_eq!(fixture.call("accept_transfer",json!({"transfer_id":incoming}))["ok"],false);assert_eq!(fixture.call("set_files",json!({"enabled":false}))["ok"],false);assert_eq!(fixture.call("cancel_transfer",json!({"transfer_id":incoming}))["ok"],true);sender.cancel(&offer.id).unwrap();
 fixture.shell.locked.store(false,Ordering::Release);
 let next=sender.send(fixture.payload_peer(),&source).unwrap();send(&mut stream,next.packet);let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().unwrap().iter().any(|view|view["state"]=="offered"));let accepted=state["state"]["transfers"].as_array().unwrap().iter().find(|view|view["state"]=="offered").unwrap()["id"].as_str().unwrap().to_owned();
 assert_eq!(fixture.call("accept_transfer",json!({"transfer_id":accepted}))["ok"],true);let finished=wait(&fixture.socket,|state|transfer(state,&accepted).is_some_and(|view|view["state"]=="complete"));assert_eq!(fs::read(transfer(&finished,&accepted).unwrap()["path"].as_str().unwrap()).unwrap(),b"disposable fixture bytes");
 let end=Instant::now()+Duration::from_secs(5);while sender.snapshots().iter().any(|view|view.id==next.id&&view.state!="complete"){assert!(Instant::now()<end);thread::sleep(Duration::from_millis(10));}
 let pending=sender.send(fixture.payload_peer(),&source).unwrap();send(&mut stream,pending.packet);let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().unwrap().iter().any(|view|view["state"]=="offered"));let disabled=state["state"]["transfers"][0]["id"].as_str().unwrap().to_owned();assert_eq!(fixture.call("set_files",json!({"enabled":false}))["ok"],true);wait(&fixture.socket,|state|transfer(state,&disabled).is_some_and(|view|view["state"]=="cancelled"));sender.cancel(&pending.id).unwrap();
 assert_eq!(fixture.call("set_files",json!({"enabled":true}))["ok"],true);
 let pending=sender.send(fixture.payload_peer(),&source).unwrap();send(&mut stream,pending.packet);let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().unwrap().iter().any(|view|view["state"]=="offered"));let disconnected=state["state"]["transfers"][0]["id"].as_str().unwrap().to_owned();drop(stream);wait(&fixture.socket,|state|transfer(state,&disconnected).is_some_and(|view|view["state"]=="cancelled")&&device(state,fixture.phone.id())["connected"]==false);sender.cancel(&pending.id).unwrap();
}

#[test] fn offline_permission_is_trusted_only_and_unknown_tls_peers_never_receive_share_capability(){
 let fixture=Fixture::new();assert_eq!(fixture.call("set_files",json!({"enabled":true}))["ok"],true);
 let unknown=Store::open(&fixture.temp.path().join("unknown")).unwrap();let(mut stranger,identity)=connect(fixture.port,&unknown,&fixture.desktop,true);assert!(!has_share(&identity));
 send(&mut stranger,service::packet("kdeconnect.ping",json!({})));let rejected=service::request(&fixture.socket,&Request{op:"set_files".into(),device_id:Some(unknown.id().into()),attempt:None,args:serde_json::from_value(json!({"enabled":true})).unwrap()}).unwrap();assert_eq!(rejected["ok"],false);
 drop(stranger);thread::sleep(Duration::from_millis(350));let(mut paired,identity)=fixture.connect();assert!(has_share(&identity));read(&mut paired);
 fixture.shell.locked.store(true,Ordering::Release);assert_eq!(fixture.call("reconnect",json!({}))["ok"],false);assert_eq!(fixture.call("set_files",json!({"enabled":false}))["ok"],false);
}

#[test] fn active_payload_is_closed_and_partial_removed_on_lock_unpair_and_daemon_stop(){
 for action in ["lock","unpair","stop"] {
  let mut fixture=Fixture::new();assert_eq!(fixture.call("set_files",json!({"enabled":true}))["ok"],true);
  let(mut control,identity)=fixture.connect();assert!(has_share(&identity));read(&mut control);
  let listener=(1739..=1764).find_map(|port|std::net::TcpListener::bind((Ipv4Addr::LOCALHOST,port)).ok()).unwrap();let port=listener.local_addr().unwrap().port();listener.set_nonblocking(true).unwrap();
  let local_key=fixture.phone.key.clone();let local_cert=fixture.phone.certificate.clone();let desktop_id=fixture.desktop.id().to_owned();let pin=identity::peer_digest(&fixture.desktop.certificate,fixture.desktop.id()).unwrap();
  let(closed_tx,closed_rx)=std::sync::mpsc::sync_channel(1);
  let server=thread::spawn(move||{
   let end=Instant::now()+Duration::from_secs(5);let tcp=loop{if let Ok((tcp,_))=listener.accept(){break tcp;}assert!(Instant::now()<end);thread::sleep(Duration::from_millis(5));};tcp.set_read_timeout(Some(Duration::from_secs(4))).unwrap();tcp.set_write_timeout(Some(Duration::from_secs(4))).unwrap();
   let mut context=SslContextBuilder::new(SslMethod::tls()).unwrap();context.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_max_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_certificate(&local_cert).unwrap();context.set_private_key(&local_key).unwrap();context.set_verify_callback(SslVerifyMode::PEER|SslVerifyMode::FAIL_IF_NO_PEER_CERT,move|_,verify|verify.current_cert().is_some_and(|cert|identity::peer_digest(&cert.to_owned(),&desktop_id).is_ok_and(|digest|digest==pin)));
   let mut stream=SslStream::new(Ssl::new(&context.build()).unwrap(),tcp).unwrap();stream.accept().unwrap();stream.write_all(b"begin").unwrap();let mut byte=[0];let closed=match stream.read(&mut byte){Ok(0)=>true,Err(error)=>!matches!(error.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock),_=>false};closed_tx.send(closed).unwrap();
  });
  let mut packet=service::packet("kdeconnect.share.request",json!({"filename":"held.txt"}));packet.payload_size=Some(1000);packet.payload_transfer_info=Some(json!({"port":port}).as_object().unwrap().clone());send(&mut control,packet);
  let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().unwrap().iter().any(|view|view["state"]=="offered"));let id=state["state"]["transfers"][0]["id"].as_str().unwrap().to_owned();assert_eq!(fixture.call("accept_transfer",json!({"transfer_id":id}))["ok"],true);
  wait(&fixture.socket,|state|transfer(state,&id).is_some_and(|view|view["transferred"]==5));
  match action {
   "lock"=>{fixture.shell.locked.store(true,Ordering::Release);wait(&fixture.socket,|state|transfer(state,&id).is_some_and(|view|view["state"]=="cancelled"));}
   "unpair"=>{assert_eq!(fixture.call("unpair",json!({}))["ok"],true);wait(&fixture.socket,|state|transfer(state,&id).is_some_and(|view|view["state"]=="cancelled"));assert!(Store::open(&fixture.state).unwrap().pin(fixture.phone.id()).unwrap().is_none());}
   "stop"=>{assert_eq!(fixture.call("stop",json!({}))["ok"],true);let end=Instant::now()+Duration::from_secs(5);while fixture._daemon.0.try_wait().unwrap().is_none(){assert!(Instant::now()<end);thread::sleep(Duration::from_millis(10));}}
   _=>unreachable!(),
  }
  assert!(closed_rx.recv_timeout(Duration::from_secs(2)).unwrap(),"payload socket stayed open after {action}");server.join().unwrap();
  assert_eq!(fs::read_dir(fixture.home.join("Downloads/DESConnect")).unwrap().count(),0,"partial remained after {action}");
 }
}

#[test] fn same_authenticated_ip_and_file_capabilities_preserve_pending_offer_across_socket_refresh(){
 let fixture=Fixture::new();assert_eq!(fixture.call("set_files",json!({"enabled":true}))["ok"],true);
 let(mut first,_)=fixture.connect();read(&mut first);
 let mut sender=fixture.phone_files("phone-sent");let source=fixture.source("refresh.txt");let ticket=sender.send(fixture.payload_peer(),&source).unwrap();send(&mut first,ticket.packet);
 let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().unwrap().iter().any(|view|view["state"]=="offered"));let id=state["state"]["transfers"][0]["id"].as_str().unwrap().to_owned();
 thread::sleep(Duration::from_millis(300));let(mut replacement,identity)=fixture.connect();assert!(has_share(&identity));assert_eq!(read(&mut replacement).kind,"kdeconnect.battery.request");drop(first);
 let state=fixture.call("state",json!({}));assert_eq!(transfer(&state,&id).unwrap()["state"],"offered","same authenticated refresh cancelled offer: {state}");
 assert_eq!(fixture.call("accept_transfer",json!({"transfer_id":id}))["ok"],true);
 let complete=wait(&fixture.socket,|state|transfer(state,&id).is_some_and(|view|view["state"]=="complete"));assert_eq!(fs::read(transfer(&complete,&id).unwrap()["path"].as_str().unwrap()).unwrap(),b"disposable fixture bytes");
}

#[test] fn authenticated_socket_refresh_that_revokes_file_capability_cancels_pending_offer(){
 let fixture=Fixture::new();assert_eq!(fixture.call("set_files",json!({"enabled":true}))["ok"],true);
 let(mut first,_)=fixture.connect();read(&mut first);
 let mut sender=fixture.phone_files("phone-sent");let source=fixture.source("revoked.txt");let ticket=sender.send(fixture.payload_peer(),&source).unwrap();send(&mut first,ticket.packet);
 let state=wait(&fixture.socket,|state|state["state"]["transfers"].as_array().unwrap().iter().any(|view|view["state"]=="offered"));let id=state["state"]["transfers"][0]["id"].as_str().unwrap().to_owned();
 thread::sleep(Duration::from_millis(300));let(mut replacement,_)=connect(fixture.port,&fixture.phone,&fixture.desktop,false);read(&mut replacement);drop(first);
 wait(&fixture.socket,|state|transfer(state,&id).is_some_and(|view|view["state"]=="cancelled"));
}

struct Music {path:PathBuf,_directory:tempfile::TempDir,calls:Arc<std::sync::atomic::AtomicUsize>,commands:Arc<std::sync::Mutex<Vec<Value>>>,revoke:Arc<std::sync::Mutex<Option<Arc<AtomicBool>>>>,stop:Arc<AtomicBool>,handle:Option<thread::JoinHandle<()>>}
impl Music {
 fn new()->Self{
  let dir=tempfile::tempdir().unwrap();fs::set_permissions(dir.path(),fs::Permissions::from_mode(0o700)).unwrap();let path=dir.path().join("music.sock");let listener=UnixListener::bind(&path).unwrap();fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();listener.set_nonblocking(true).unwrap();
  let calls=Arc::new(std::sync::atomic::AtomicUsize::new(0));let commands=Arc::new(std::sync::Mutex::new(Vec::new()));let revoke:Arc<std::sync::Mutex<Option<Arc<AtomicBool>>>>=Arc::new(std::sync::Mutex::new(None));let stop=Arc::new(AtomicBool::new(false));let(count,received,locked,done)=(calls.clone(),commands.clone(),revoke.clone(),stop.clone());
  let handle=thread::spawn(move||{
   while !done.load(Ordering::Acquire){let(mut stream,_)=match listener.accept(){Ok(pair)=>pair,Err(_)=>{thread::sleep(Duration::from_millis(5));continue}};count.fetch_add(1,Ordering::Release);
    let mut state=json!({"type":"state","track":{"name":"Private fixture song","artists":"Fixture artist","album":"Fixture album","duration_ms":5000,"cover":null},"playing":true,"position_ms":2000,"volume":40});
    if let Some(lock)=locked.lock().unwrap().take(){thread::sleep(Duration::from_millis(50));lock.store(true,Ordering::Release);}
    if writeln!(stream,"{state}").is_err(){continue;}stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();let mut reader=std::io::BufReader::new(stream.try_clone().unwrap());
    loop{let mut line=String::new();if !matches!(std::io::BufRead::read_line(&mut reader,&mut line),Ok(n)if n>0){break;}let command:Value=serde_json::from_str(&line).unwrap();received.lock().unwrap().push(command.clone());match command["cmd"].as_str(){Some("toggle")=>state["playing"]=json!(!state["playing"].as_bool().unwrap()),Some("volume")=>state["volume"]=command["percent"].clone(),Some("state")=>{if writeln!(stream,"{state}").is_err(){break;}},Some("next")|Some("prev")=>{},other=>panic!("unexpected fake command: {other:?}")}}
   }
  });Self{path,_directory:dir,calls,commands,revoke,stop,handle:Some(handle)}
 }
}
impl Drop for Music {fn drop(&mut self){self.stop.store(true,Ordering::Release);if let Some(handle)=self.handle.take(){handle.join().unwrap();}}}
fn has(identity:&Packet,key:&str,capability:&str)->bool{identity.body[key].as_array().unwrap().iter().any(|value|value==capability)}
fn reconnect_policy(fixture:&Fixture,stream:SslStream<TcpStream>)->(SslStream<TcpStream>,Packet){assert_eq!(fixture.call("reconnect",json!({}))["ok"],true);drop(stream);wait(&fixture.socket,|state|device(state,fixture.phone.id())["connected"]==false);thread::sleep(Duration::from_millis(300));let(mut stream,identity)=fixture.connect();read(&mut stream);(stream,identity)}

#[test] fn media_and_volume_tls_permissions_are_separate_lazy_and_rechecked_at_music_mutation(){
 let music=Music::new();let fixture=Fixture::with_music(Some(&music.path));let(mut stream,identity)=fixture.connect();read(&mut stream);
 for cap in ["kdeconnect.mpris.request","kdeconnect.systemvolume.request"]{assert!(!has(&identity,"incomingCapabilities",cap));}
 let request=||service::packet("kdeconnect.mpris.request",json!({"player":"DES Music","requestNowPlaying":true}));send(&mut stream,request());wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("disabled")));assert_eq!(music.calls.load(Ordering::Acquire),0);
 let enabled=fixture.call("set_media",json!({"enabled":true}));assert_eq!(enabled["ok"],true);assert_eq!(device(&enabled,fixture.phone.id())["media_reconnect_needed"],true);assert_eq!(device(&enabled,fixture.phone.id())["volume_enabled"],false);
 send(&mut stream,request());wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("reconnect")));assert_eq!(music.calls.load(Ordering::Acquire),0);
 let(mut stream,identity)=reconnect_policy(&fixture,stream);assert!(has(&identity,"incomingCapabilities","kdeconnect.mpris.request"));assert!(has(&identity,"outgoingCapabilities","kdeconnect.mpris"));assert!(!has(&identity,"incomingCapabilities","kdeconnect.systemvolume.request"));
 send(&mut stream,request());let response=read(&mut stream);assert_eq!(response.kind,"kdeconnect.mpris");assert_eq!(response.body["title"],"Private fixture song");assert!(music.commands.lock().unwrap().is_empty());
 send(&mut stream,service::packet("kdeconnect.systemvolume.request",json!({"name":"default-output","volume":101})));wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("disabled")));

 let mut combined=request().encode().unwrap();combined.extend(service::packet("kdeconnect.mpris.request",json!({"player":"DES Music","action":"Next"})).encode().unwrap());stream.write_all(&combined).unwrap();assert_eq!(read(&mut stream).kind,"kdeconnect.mpris");assert_eq!(read(&mut stream).kind,"kdeconnect.mpris");assert!(music.commands.lock().unwrap().iter().any(|command|command["cmd"]=="next"));music.commands.lock().unwrap().clear();
 *music.revoke.lock().unwrap()=Some(fixture.shell.locked.clone());send(&mut stream,service::packet("kdeconnect.mpris.request",json!({"player":"DES Music","action":"Pause"})));
 let refused=wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("failed")||error.contains("locked")));assert!(music.commands.lock().unwrap().is_empty());assert!(!serde_json::to_string(&refused).unwrap().contains("Private fixture song"));assert_eq!(fixture.call("set_media",json!({"enabled":false}))["ok"],false);
 fixture.shell.locked.store(false,Ordering::Release);assert_eq!(fixture.call("set_media",json!({"enabled":true}))["ok"],true);
 *music.revoke.lock().unwrap()=Some(fixture.shell.locked.clone());send(&mut stream,request());let hidden=wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("locked")));assert!(!serde_json::to_string(&hidden).unwrap().contains("Private fixture song"));stream.get_ref().set_read_timeout(Some(Duration::from_millis(80))).unwrap();let mut byte=[0];assert!(matches!(stream.read(&mut byte),Err(error)if matches!(error.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock)));stream.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
 fixture.shell.locked.store(false,Ordering::Release);let changed=fixture.call("set_volume",json!({"enabled":true}));assert_eq!(changed["ok"],true);assert_eq!(device(&changed,fixture.phone.id())["media_enabled"],true);assert_eq!(device(&changed,fixture.phone.id())["volume_reconnect_needed"],true);
 let(mut stream,identity)=reconnect_policy(&fixture,stream);assert!(has(&identity,"incomingCapabilities","kdeconnect.systemvolume.request"));assert!(has(&identity,"outgoingCapabilities","kdeconnect.systemvolume"));
 // Invalid volume is rejected before any real wpctl subprocess can run.
 send(&mut stream,service::packet("kdeconnect.systemvolume.request",json!({"name":"default-output","volume":101})));wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("failed")));
 assert_eq!(fixture.call("set_media",json!({"enabled":false}))["ok"],true);let before=music.calls.load(Ordering::Acquire);send(&mut stream,request());wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("disabled")));assert_eq!(music.calls.load(Ordering::Acquire),before);
 send(&mut stream,service::packet("kdeconnect.ping",json!({})));wait(&fixture.socket,|state|!device(state,fixture.phone.id())["last_ping"].is_null());
}

#[test] fn secure_media_capability_direction_and_offline_policy_do_not_authorize_an_unknown_peer(){
 let music=Music::new();let fixture=Fixture::with_music(Some(&music.path));assert_eq!(fixture.call("set_media",json!({"enabled":true}))["ok"],true);assert_eq!(fixture.call("set_volume",json!({"enabled":true}))["ok"],true);
 let unknown=Store::open(&fixture.temp.path().join("unknown-media")).unwrap();let(mut stranger,identity)=connect_with_caps(fixture.port,&unknown,&fixture.desktop,false,true,true);assert!(!has(&identity,"incomingCapabilities","kdeconnect.mpris.request"));assert!(!has(&identity,"incomingCapabilities","kdeconnect.systemvolume.request"));send(&mut stranger,service::packet("kdeconnect.mpris.request",json!({"player":"DES Music","action":"Pause"})));wait(&fixture.socket,|state|device(state,unknown.id())["error"].as_str().is_some_and(|error|error.contains("paired")));assert_eq!(music.calls.load(Ordering::Acquire),0);drop(stranger);thread::sleep(Duration::from_millis(350));
 let(mut stream,_)=connect_with_caps(fixture.port,&fixture.phone,&fixture.desktop,false,false,false);read(&mut stream);send(&mut stream,service::packet("kdeconnect.mpris.request",json!({"player":"DES Music","action":"Pause"})));wait(&fixture.socket,|state|device(state,fixture.phone.id())["error"].as_str().is_some_and(|error|error.contains("capabilities")));assert_eq!(music.calls.load(Ordering::Acquire),0);assert!(music.commands.lock().unwrap().is_empty());
}
