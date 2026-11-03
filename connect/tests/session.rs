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
fn connect(port:u16,peer:&Store)->SslStream<TcpStream>{
 let mut tcp=TcpStream::connect(("127.0.0.1",port)).unwrap();tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();send(&mut tcp,fake_identity(peer));
 let mut context=SslContextBuilder::new(SslMethod::tls()).unwrap();context.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_max_proto_version(Some(SslVersion::TLS1_2)).unwrap();context.set_certificate(&peer.certificate).unwrap();context.set_private_key(&peer.key).unwrap();context.set_verify_callback(SslVerifyMode::PEER|SslVerifyMode::FAIL_IF_NO_PEER_CERT,|_,_|true);
 let mut stream=SslStream::new(Ssl::new(&context.build()).unwrap(),tcp).unwrap();stream.accept().unwrap();send(&mut stream,fake_identity(peer));let identity=read(&mut stream);assert_eq!(identity.kind,"kdeconnect.identity");stream
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
