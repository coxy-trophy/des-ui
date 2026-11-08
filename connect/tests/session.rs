//! A real TLS transcript against the daemon, using disposable identities only.
use des_connect::{identity::Store,packet::Packet,service::{Request,packet,request}};
use openssl::ssl::{Ssl,SslContextBuilder,SslMethod,SslStream,SslVerifyMode,SslVersion};
use serde_json::{json,Value};
use std::{io::{Read,Write},net::TcpStream,path::Path,process::{Child,Command,Stdio},time::{Duration,Instant}};
struct Daemon(Child);
impl Drop for Daemon {fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
fn call(socket:&Path,op:&str,id:Option<&str>)->Value {request(socket,&Request{op:op.into(),device_id:id.map(Into::into),attempt:None,args:Default::default()}).unwrap()}
fn wait(socket:&Path,test:impl Fn(&Value)->bool)->Value {
 let end=Instant::now()+Duration::from_secs(10);loop {if let Ok(state)=request(socket,&Request{op:"state".into(),device_id:None,attempt:None,args:Default::default()}){if test(&state){return state;}}assert!(Instant::now()<end,"daemon state deadline");std::thread::sleep(Duration::from_millis(50));}
}
fn dev<'a>(state:&'a Value,id:&str)->Option<&'a Value>{state["state"]["devices"].as_array()?.iter().find(|d|d["id"]==id)}
fn fake_identity(store:&Store)->Packet{packet("kdeconnect.identity",json!({"deviceId":store.id(),"deviceName":"Fixture phone","deviceType":"phone","protocolVersion":8,"incomingCapabilities":["kdeconnect.ping","kdeconnect.battery.request"],"outgoingCapabilities":["kdeconnect.ping","kdeconnect.battery"]}))}
fn read(stream:&mut impl Read)->Packet {let mut bytes=Vec::new();loop {let mut b=[0];stream.read_exact(&mut b).unwrap();bytes.push(b[0]);if b[0]==b'\n'{break;}assert!(bytes.len()<65536);}Packet::read(&mut std::io::Cursor::new(bytes)).unwrap()}
fn send(stream:&mut impl Write,packet:Packet){stream.write_all(&packet.encode().unwrap()).unwrap();}
fn connect(port:u16,peer:&Store)->SslStream<TcpStream>{connect_with_cast(port,peer,false)}
fn connect_with_cast(port:u16,peer:&Store,cast:bool)->SslStream<TcpStream>{
 let mut identity=fake_identity(peer);
 if cast {for field in ["incomingCapabilities","outgoingCapabilities"] {identity.body[field].as_array_mut().unwrap().push(json!("des.connect.cast.v1"));}}
 let mut tcp=TcpStream::connect(("127.0.0.1",port)).unwrap();tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();send(&mut tcp,identity.clone());
 let mut context=SslContextBuilder::new(SslMethod::tls()).unwrap();context.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_max_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_certificate(&peer.certificate).unwrap();context.set_private_key(&peer.key).unwrap();context.set_verify_callback(SslVerifyMode::PEER|SslVerifyMode::FAIL_IF_NO_PEER_CERT,|_,_|true);
 let mut stream=SslStream::new(Ssl::new(&context.build()).unwrap(),tcp).unwrap();stream.accept().unwrap();send(&mut stream,identity);let identity=read(&mut stream);assert_eq!(identity.kind,"kdeconnect.identity");stream
}
#[test] fn tls_pairing_gates_features_persists_reconnects_and_unpairs() {
 let temp=tempfile::tempdir().unwrap();let state=temp.path().join("state");let socket=temp.path().join("runtime/control.sock");
 let child=Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").args(["--socket",socket.to_str().unwrap(),"--state-dir",state.to_str().unwrap(),"--name","DES Connect test"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
 let _daemon=Daemon(child);let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
 let peer=Store::open(&temp.path().join("phone")).unwrap();let id=peer.id();let desktop=Store::open(&state).unwrap();
 std::thread::sleep(Duration::from_millis(300));let mut stream=connect(port,&peer);wait(&socket,|s|dev(s,id).is_some_and(|d|d["connected"]==true));
 assert_eq!(call(&socket,"ping",Some(id))["ok"],false);
 send(&mut stream,packet("kdeconnect.battery",json!({"currentCharge":99,"isCharging":true})));
 std::thread::sleep(Duration::from_millis(300));assert!(dev(&call(&socket,"state",None),id).unwrap()["battery"].is_null());
 let sent=call(&socket,"pair",Some(id));assert_eq!(sent["ok"],true);let pair=read(&mut stream);assert_eq!(pair.kind,"kdeconnect.pair");let stamp=pair.body["timestamp"].as_i64().unwrap();
 assert_eq!(dev(&sent,id).unwrap()["verification_key"],des_connect::identity::verification_key(&desktop.certificate,&peer.certificate,stamp).unwrap());
 // A simultaneous request is not consent. An acknowledgement has no timestamp.
 send(&mut stream,packet("kdeconnect.pair",json!({"pair":true,"timestamp":stamp})));
 stream.get_ref().set_read_timeout(Some(Duration::from_millis(700))).unwrap();
 let mut byte=[0];let early=stream.read(&mut byte).unwrap_err();
 assert!(matches!(early.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut));
 assert_eq!(dev(&call(&socket,"state",None),id).unwrap()["pair_state"],"outgoing");
 assert!(Store::open(&state).unwrap().pin(id).unwrap().is_none());
 stream.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
 send(&mut stream,packet("kdeconnect.pair",json!({"pair":true})));wait(&socket,|s|dev(s,id).is_some_and(|d|d["paired"]==true));assert_eq!(read(&mut stream).kind,"kdeconnect.battery.request");
 send(&mut stream,packet("kdeconnect.battery",json!({"currentCharge":82,"isCharging":true})));send(&mut stream,packet("kdeconnect.ping",json!({"message":"fixture"})));
 wait(&socket,|s|dev(s,id).is_some_and(|d|d["battery"]==82&&!d["last_ping"].is_null()));
 assert_eq!(call(&socket,"ping",Some(id))["ok"],true);assert_eq!(read(&mut stream).kind,"kdeconnect.ping");
 assert!(Store::open(&state).unwrap().pin(id).unwrap().is_some());drop(stream);
 wait(&socket,|s|dev(s,id).is_some_and(|d|d["connected"]==false));
 std::thread::sleep(Duration::from_millis(300));let mut stream=connect(port,&peer);wait(&socket,|s|dev(s,id).is_some_and(|d|d["paired"]==true&&d["connected"]==true));assert_eq!(read(&mut stream).kind,"kdeconnect.battery.request");
 assert_eq!(call(&socket,"unpair",Some(id))["ok"],true);let unpair=read(&mut stream);assert_eq!(unpair.body["pair"],false);
 assert!(Store::open(&state).unwrap().pin(id).unwrap().is_none());assert_eq!(call(&socket,"ping",Some(id))["ok"],false);
}

// The daemon's UI approval token must survive neither reconnect nor certificate replacement.
#[test] fn approval_tokens_are_unique_across_unpaired_reconnects_and_daemon_restart() {
 let temp=tempfile::tempdir().unwrap();let state=temp.path().join("state");let socket=temp.path().join("runtime/control.sock");
 let spawn=||Daemon(Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").args(["--socket",socket.to_str().unwrap(),"--state-dir",state.to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
 let daemon=spawn();let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
 let peer=Store::open(&temp.path().join("phone")).unwrap();let id=peer.id();
 let mut first=connect(port,&peer);wait(&socket,|s|dev(s,id).is_some());
 let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
 send(&mut first,packet("kdeconnect.pair",json!({"pair":true,"timestamp":stamp})));
 let view=wait(&socket,|s|dev(s,id).is_some_and(|d|d["pair_state"]=="incoming"));let old=dev(&view,id).unwrap()["attempt"].as_u64().unwrap();
 drop(first);wait(&socket,|s|dev(s,id).is_none_or(|d|d["connected"]==false));std::thread::sleep(Duration::from_millis(300));
 let mut second=connect(port,&peer);send(&mut second,packet("kdeconnect.pair",json!({"pair":true,"timestamp":stamp})));
 let view=wait(&socket,|s|dev(s,id).is_some_and(|d|d["pair_state"]=="incoming"));let new=dev(&view,id).unwrap()["attempt"].as_u64().unwrap();assert_ne!(old,new);
 let rejected=request(&socket,&Request{op:"accept".into(),device_id:Some(id.into()),attempt:Some(old),args:Default::default()}).unwrap();assert_eq!(rejected["ok"],false);assert!(!dev(&rejected,id).unwrap()["paired"].as_bool().unwrap());
 drop(second);drop(daemon);let _daemon=spawn();let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
 let mut third=connect(port,&peer);send(&mut third,packet("kdeconnect.pair",json!({"pair":true,"timestamp":stamp})));
 let view=wait(&socket,|s|dev(s,id).is_some_and(|d|d["pair_state"]=="incoming"));assert_ne!(dev(&view,id).unwrap()["attempt"].as_u64(),Some(old));
 assert_eq!(request(&socket,&Request{op:"accept".into(),device_id:Some(id.into()),attempt:Some(old),args:Default::default()}).unwrap()["ok"],false);
}

#[test] fn absolute_handshake_deadline_releases_slow_private_connections() {
 let temp=tempfile::tempdir().unwrap();let socket=temp.path().join("runtime/control.sock");
 let _daemon=Daemon(Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").env("DES_CONNECT_HANDSHAKE_MS","1500").args(["--socket",socket.to_str().unwrap(),"--state-dir",temp.path().join("state").to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
 let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
 let mut streams=Vec::new();for _ in 0..8 {let mut tcp=TcpStream::connect(("127.0.0.1",port)).unwrap();let _=tcp.write_all(b"{");streams.push(tcp);std::thread::sleep(Duration::from_millis(260));}
 // Bytes keep arriving more frequently than the old per-read timeout.
 for _ in 0..5 {for tcp in &mut streams {let _=tcp.write_all(b" ");}std::thread::sleep(Duration::from_millis(400));}
 let peer=Store::open(&temp.path().join("phone")).unwrap();let _stream=connect(port,&peer);
 wait(&socket,|s|dev(s,peer.id()).is_some_and(|d|d["connected"]==true));
}

#[test] fn idle_unpaired_tls_session_expires_without_keepalive_extending_it() {
 let temp=tempfile::tempdir().unwrap();let socket=temp.path().join("runtime/control.sock");
 let _daemon=Daemon(Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").env("DES_CONNECT_UNPAIRED_MS","1500").args(["--socket",socket.to_str().unwrap(),"--state-dir",temp.path().join("state").to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
 let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
 let peer=Store::open(&temp.path().join("phone")).unwrap();let mut stream=connect(port,&peer);
 wait(&socket,|s|dev(s,peer.id()).is_some_and(|d|d["connected"]==true));
 for _ in 0..3 {let _=stream.write_all(&packet("kdeconnect.ping",json!({})).encode().unwrap());std::thread::sleep(Duration::from_millis(300));}
 wait(&socket,|s|dev(s,peer.id()).is_none());
 let mut byte=[0];assert!(matches!(stream.read(&mut byte),Ok(0)|Err(_)));
}

#[test] fn locked_session_refuses_capture_and_stop_removes_private_socket() {
 use std::os::unix::net::UnixListener;
 let temp=tempfile::tempdir().unwrap();let socket=temp.path().join("runtime/control.sock");let shell_path=temp.path().join("shell.sock");let listener=UnixListener::bind(&shell_path).unwrap();
 let shell=std::thread::spawn(move||{let(mut s,_)=listener.accept().unwrap();let mut b=[0;6];s.read_exact(&mut b).unwrap();assert_eq!(&b,b"state\n");s.write_all(b"ok {\"locked\":true}\n").unwrap();});
 let mut daemon=Daemon(Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").env("DES_UI_CONTROL_SOCKET",&shell_path).args(["--socket",socket.to_str().unwrap(),"--state-dir",temp.path().join("state").to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
 let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;let peer=Store::open(&temp.path().join("phone")).unwrap();let mut stream=connect(port,&peer);wait(&socket,|s|dev(s,peer.id()).is_some());
 assert_eq!(call(&socket,"pair",Some(peer.id()))["ok"],true);read(&mut stream);send(&mut stream,packet("kdeconnect.pair",json!({"pair":true})));wait(&socket,|s|dev(s,peer.id()).is_some_and(|d|d["paired"]==true));read(&mut stream);
 let result=request(&socket,&Request{op:"mirror_phone".into(),device_id:Some(peer.id().into()),attempt:None,args:serde_json::from_value(json!({"serial":"fixture","control":false})).unwrap()}).unwrap();
 assert_eq!(result["ok"],false);assert!(result["error"].as_str().unwrap().contains("unlocked"));assert_eq!(result["state"]["screens"]["running"],false);shell.join().unwrap();
 assert_eq!(call(&socket,"stop",None)["ok"],true);
 let end=Instant::now()+Duration::from_secs(5);while daemon.0.try_wait().unwrap().is_none(){assert!(Instant::now()<end);std::thread::sleep(Duration::from_millis(30));}assert!(!socket.exists());
}

#[test] fn same_certificate_socket_replacement_preserves_pending_pairing_and_uses_new_stream() {
 let temp=tempfile::tempdir().unwrap();let socket=temp.path().join("runtime/control.sock");
 let _daemon=Daemon(Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").args(["--socket",socket.to_str().unwrap(),"--state-dir",temp.path().join("state").to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
 let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;let peer=Store::open(&temp.path().join("phone")).unwrap();
 let mut first=connect(port,&peer);let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();send(&mut first,packet("kdeconnect.pair",json!({"pair":true,"timestamp":stamp})));
 let before=wait(&socket,|s|dev(s,peer.id()).is_some_and(|d|d["pair_state"]=="incoming"));let attempt=dev(&before,peer.id()).unwrap()["attempt"].as_u64().unwrap();
 std::thread::sleep(Duration::from_millis(300));let mut second=connect(port,&peer);drop(first);
 let accepted=request(&socket,&Request{op:"accept".into(),device_id:Some(peer.id().into()),attempt:Some(attempt),args:Default::default()}).unwrap();assert_eq!(accepted["ok"],true,"{accepted}");
 assert_eq!(read(&mut second).body["pair"],true);assert_eq!(read(&mut second).kind,"kdeconnect.battery.request");
 assert_eq!(dev(&accepted,peer.id()).unwrap()["paired"],true);
}

const CAST:&str="des.connect.cast.v1";
// A real baseline 1x1 JPEG, base64 so the fixture has no encoder dependency.
const CAST_JPEG:&str="/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDy2iiivWPPP//Z";
fn cast_call(socket:&Path,op:&str,id:&str,args:Value)->Value {
 request(socket,&Request{op:op.into(),device_id:Some(id.into()),attempt:None,args:serde_json::from_value(args).unwrap()}).unwrap()
}
fn cast_frame(socket:&Path,id:&str,serial:&str)->Vec<u8> {
 use std::os::unix::net::UnixStream;
 let mut stream=UnixStream::connect(socket).unwrap();stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
 writeln!(stream,"{}",json!({"op":"screen_frame","device_id":id,"serial":serial})).unwrap();
 let mut bytes=Vec::new();stream.read_to_end(&mut bytes).unwrap();bytes
}
fn cast_message(stream:&mut impl Read,op:&str)->Packet {
 loop {let np=read(stream);assert_eq!(np.kind,CAST);if np.body["op"]==op{return np;}assert_eq!(np.body["op"],"lease");}
}
#[test] fn wireless_cast_requires_pairing_consent_and_current_geometry_and_stops_on_lock() {
 use std::{os::unix::net::UnixListener,sync::{Arc,atomic::{AtomicBool,Ordering}}};
 let temp=tempfile::tempdir().unwrap();let socket=temp.path().join("runtime/control.sock");let shell_path=temp.path().join("shell.sock");
 let listener=UnixListener::bind(&shell_path).unwrap();listener.set_nonblocking(true).unwrap();
 let flags=Arc::new(AtomicBool::new(false));let locked=flags.clone();
 let (done_tx,done_rx)=std::sync::mpsc::channel::<()>();
 let lock_server=std::thread::spawn(move||{
  while matches!(done_rx.try_recv(),Err(std::sync::mpsc::TryRecvError::Empty)){
   let Ok((mut s,_))=listener.accept() else{std::thread::sleep(Duration::from_millis(5));continue;};
   let mut b=[0;6];if s.read_exact(&mut b).is_ok()&&&b==b"state\n"{let _=writeln!(s,"ok {{\"locked\":{}}}",locked.load(Ordering::Acquire));}
  }
 });
 let _daemon=Daemon(Command::new(env!("CARGO_BIN_EXE_des-connectd")).arg("--loopback").env("DES_UI_CONTROL_SOCKET",&shell_path).args(["--socket",socket.to_str().unwrap(),"--state-dir",temp.path().join("state").to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
 let ready=wait(&socket,|s|s["ok"]==true);let port=ready["state"]["tcp_port"].as_u64().unwrap() as u16;
 let peer=Store::open(&temp.path().join("phone")).unwrap();let id=peer.id();let mut stream=connect_with_cast(port,&peer,true);
 wait(&socket,|s|dev(s,id).is_some_and(|d|d["connected"]==true));
 assert_eq!(call(&socket,"cast_phone",Some(id))["ok"],false);
 assert_eq!(call(&socket,"pair",Some(id))["ok"],true);assert_eq!(read(&mut stream).kind,"kdeconnect.pair");send(&mut stream,packet("kdeconnect.pair",json!({"pair":true})));
 wait(&socket,|s|dev(s,id).is_some_and(|d|d["paired"]==true));read(&mut stream);
 let started=call(&socket,"cast_phone",Some(id));
 assert_eq!(started["ok"],true,"{started}");
 assert_eq!(dev(&started,id).unwrap()["wireless_cast"],true);
 let serial=started["state"]["screens"]["serial"].as_str().unwrap();
 assert_eq!(started["state"]["screens"]["mode"],"wireless");
 assert_eq!(cast_message(&mut stream,"start").body["id"],serial);
 let usb=cast_call(&socket,"mirror_phone",id,json!({"serial":"unused-test-usb"}));
 assert!(usb["error"].as_str().unwrap().contains("stop the wireless cast"));
 assert_eq!(call(&socket,"cast_phone",Some(id))["ok"],false);
 let input=|geometry,x|cast_call(&socket,"cast_input",id,json!({"serial":serial,"geometry":geometry,"action":"tap","x":x,"y":9200}));
 assert_eq!(cast_frame(&socket,id,serial),b"WAIT\n");
 assert_eq!(input(1,2800)["ok"],false);
 send(&mut stream,packet(CAST,json!({"op":"frame","id":serial,"geometry":1,"jpeg":CAST_JPEG})));
 std::thread::sleep(Duration::from_millis(120));assert_eq!(cast_frame(&socket,id,serial),b"WAIT\n");
 send(&mut stream,packet(CAST,json!({"op":"status","id":serial,"phase":"live","control":false,"geometry":2})));
 send(&mut stream,packet(CAST,json!({"op":"frame","id":serial,"geometry":2,"jpeg":CAST_JPEG})));
 let end=Instant::now()+Duration::from_secs(2);loop {
  let frame=cast_frame(&socket,id,serial);if frame.starts_with(b"JPEG "){assert_eq!(frame.split(|&c|c==b'\n').next().unwrap().rsplit(|&c|c==b' ').next().unwrap(),b"2");break;}
  assert!(Instant::now()<end,"frame not delivered: {frame:?}");
 }
 assert!(!cast_frame(&socket,"wrong-peer",serial).starts_with(b"JPEG "));
 assert_eq!(input(2,2800)["ok"],false,"view-only frames must not enable input");
 send(&mut stream,packet(CAST,json!({"op":"status","id":serial,"phase":"live","control":true,"geometry":3})));
 send(&mut stream,packet(CAST,json!({"op":"frame","id":serial,"geometry":2,"jpeg":CAST_JPEG})));
 send(&mut stream,packet(CAST,json!({"op":"frame","id":serial,"geometry":3,"jpeg":"not a JPEG"})));
 wait(&socket,|s|s["state"]["screens"]["control"]==true);
 assert_eq!(cast_frame(&socket,id,serial),b"WAIT\n","rotation must discard old pixels, including late frames");
 assert_eq!(input(3,2800)["ok"],false,"no input before a current frame");
 std::thread::sleep(Duration::from_millis(120));
 send(&mut stream,packet(CAST,json!({"op":"frame","id":serial,"geometry":3,"jpeg":CAST_JPEG})));
 let end=Instant::now()+Duration::from_secs(2);
 while !cast_frame(&socket,id,serial).starts_with(b"JPEG ") {assert!(Instant::now()<end,"missing current frame");}
 assert_eq!(input(2,2800)["ok"],false);
 assert_eq!(input(3,10001)["ok"],false);
 assert_eq!(input(3,2800)["ok"],true);
 let sent=cast_message(&mut stream,"input");assert_eq!(sent.body["x"],2800);assert_eq!(sent.body["y"],9200);
 flags.store(true,Ordering::Release);
 assert!(!cast_frame(&socket,id,serial).starts_with(b"JPEG "));
 assert_eq!(input(2,2800)["ok"],false);
 assert_eq!(cast_message(&mut stream,"stop").body["id"],serial);
 flags.store(false,Ordering::Release);
 assert!(!cast_frame(&socket,id,serial).starts_with(b"JPEG "));
 let second=call(&socket,"cast_phone",Some(id));assert_eq!(second["ok"],true,"{second}");
 let second_id=second["state"]["screens"]["serial"].as_str().unwrap();
 assert_ne!(serial,second_id);assert_eq!(cast_message(&mut stream,"start").body["id"],second_id);
 // The normal state poll does not keep a cast alive if the preview viewer left.
 wait(&socket,|s|s["state"]["screens"]["running"]==false);
 assert_eq!(cast_message(&mut stream,"stop").body["id"],second_id);
 assert!(!cast_frame(&socket,id,second_id).starts_with(b"JPEG "));
 let third=call(&socket,"cast_phone",Some(id));assert_eq!(third["ok"],true,"{third}");
 let third_id=third["state"]["screens"]["serial"].as_str().unwrap();
 assert_eq!(cast_message(&mut stream,"start").body["id"],third_id);
 drop(stream);wait(&socket,|s|s["state"]["screens"]["running"]==false);
 assert!(!cast_frame(&socket,id,third_id).starts_with(b"JPEG "));
 drop(done_tx);lock_server.join().unwrap();
}
