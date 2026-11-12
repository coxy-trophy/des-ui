//! Phone-key against a real daemon over real TLS, with a disposable identity, a
//! stand-in phone that signs, and a stand-in shell that records what it is asked.
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use des_connect::{identity::{Store, hex}, packet::Packet, phonekey::{self, Binding, Challenge, PublicKey, Purpose}, service::{Request, packet, request, request_within}};
use openssl::{ec::{EcGroup, EcKey}, hash::MessageDigest, nid::Nid, pkey::{PKey, Private}, sign::Signer, ssl::{Ssl, SslContextBuilder, SslMethod, SslStream, SslVerifyMode, SslVersion}};
use serde_json::{Map, Value, json};
use std::{io::{BufRead, BufReader, Read, Write}, net::TcpStream, os::unix::net::UnixListener, path::{Path, PathBuf}, process::{Child, Command, Stdio},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};

struct Daemon(Child);
impl Drop for Daemon { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

/// The shell's control socket: says whether the session is locked, and records
/// every line it is sent. A real shell locks when asked; so does this one.
#[derive(Clone)]
struct Shell { locked: Arc<AtomicBool>, lines: Arc<Mutex<Vec<String>>> }
impl Shell {
    fn start(path: &Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let shell = Shell { locked: Arc::new(AtomicBool::new(false)), lines: Arc::default() };
        let seen = shell.clone();
        thread::spawn(move || for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() { continue; }
            let line = line.trim_end().to_owned();
            let answer = match line.as_str() {
                "state" => format!("ok {{\"locked\":{}}}\n", seen.locked.load(Ordering::SeqCst)),
                "lock" => { seen.locked.store(true, Ordering::SeqCst); "ok\n".to_owned() }
                _ => "error unknown\n".to_owned(),
            };
            seen.lines.lock().unwrap().push(line);
            let _ = stream.write_all(answer.as_bytes());
        });
        shell
    }
    fn count(&self, word: &str) -> usize { self.lines.lock().unwrap().iter().filter(|line| *line == word).count() }
    /// Everything the daemon ever said to the shell is one of the two words it may say.
    fn only_asked_state_or_lock(&self) -> bool { self.lines.lock().unwrap().iter().all(|line| line == "state" || line == "lock") }
}

struct Phone { store: Store, stream: SslStream<TcpStream>, key: PKey<Private>, public: PublicKey, binding: Binding }
impl Phone {
    fn connect(port: u16, directory: &Path, desktop: &Store, capable: bool) -> Self {
        let store = Store::open(directory).unwrap();
        let mut identity = packet("kdeconnect.identity", json!({"deviceId": store.id(), "deviceName": "Fixture phone", "deviceType": "phone", "protocolVersion": 8,
            "incomingCapabilities": ["kdeconnect.ping", "kdeconnect.battery.request"], "outgoingCapabilities": ["kdeconnect.ping", "kdeconnect.battery"]}));
        if capable { for field in ["incomingCapabilities", "outgoingCapabilities"] { identity.body[field].as_array_mut().unwrap().push(json!(phonekey::CAPABILITY)); } }
        let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        tcp.write_all(&identity.encode().unwrap()).unwrap();
        let mut context = SslContextBuilder::new(SslMethod::tls()).unwrap();
        context.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap();
        context.set_max_proto_version(Some(SslVersion::TLS1_2)).unwrap();
        context.set_certificate(&store.certificate).unwrap();
        context.set_private_key(&store.key).unwrap();
        context.set_verify_callback(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT, |_, _| true);
        let mut stream = SslStream::new(Ssl::new(&context.build()).unwrap(), tcp).unwrap();
        stream.accept().unwrap();
        stream.write_all(&identity.encode().unwrap()).unwrap();
        let key = PKey::from_ec_key(EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap()).unwrap();
        let public = PublicKey::from_spki(&key.public_key_to_der().unwrap()).unwrap();
        // The phone's own view of the pairing: the certificates of its TLS session.
        let binding = Binding { desktop_id: desktop.id().into(), desktop_cert: desktop.certificate_digest().unwrap(), phone_id: store.id().into(), phone_cert: store.certificate_digest().unwrap() };
        let mut phone = Phone { store, stream, key, public, binding };
        let theirs = phone.read().unwrap();
        assert_eq!(theirs.kind, "kdeconnect.identity");
        for field in ["incomingCapabilities", "outgoingCapabilities"] {
            assert!(theirs.body[field].as_array().unwrap().contains(&json!(phonekey::CAPABILITY)), "the desktop says it speaks phone-key");
        }
        phone
    }
    fn id(&self) -> String { self.store.id().to_owned() }
    fn read(&mut self) -> std::io::Result<Packet> {
        let mut bytes = Vec::new();
        loop {
            let mut b = [0];
            self.stream.read_exact(&mut b)?;
            bytes.push(b[0]);
            if b[0] == b'\n' { break; }
        }
        Packet::read(&mut std::io::Cursor::new(bytes))
    }
    fn send(&mut self, kind: &str, body: Value) { self.stream.write_all(&packet(kind, body).encode().unwrap()).unwrap(); }
    /// The next phone-key packet with this `op`, skipping everything else.
    fn expect(&mut self, op: &str) -> Map<String, Value> {
        loop {
            let packet = self.read().unwrap_or_else(|error| panic!("waiting for {op}: {error}"));
            if packet.kind == phonekey::CAPABILITY && packet.body["op"] == op { return packet.body; }
        }
    }
    /// No phone-key packet arrives for this long.
    fn hears_nothing(&mut self, patience: Duration) -> bool {
        self.stream.get_ref().set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let end = Instant::now() + patience;
        let mut quiet = true;
        while Instant::now() < end { if self.read().is_ok_and(|packet| packet.kind == phonekey::CAPABILITY) { quiet = false; } }
        self.stream.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        quiet
    }
    fn pair(&mut self, socket: &Path) {
        let id = self.id();
        wait(socket, |state| device(state, &id).is_some());
        assert_eq!(call(socket, "pair", Some(&id), json!({}))["ok"], true);
        loop { if self.read().unwrap().kind == "kdeconnect.pair" { break; } }
        self.send("kdeconnect.pair", json!({"pair": true}));
        wait(socket, |state| device(state, &id).is_some_and(|d| d["paired"] == true));
    }
    /// What the phone signs: built from the fields it was sent and would show, and
    /// from its own view of the pairing. Nothing here trusts the desktop's bytes.
    fn challenge(body: &Map<String, Value>) -> Challenge {
        Challenge {
            purpose: Purpose::parse(body["purpose"].as_str().unwrap()).unwrap(),
            request_id: phonekey::parse_id(body["request_id"].as_str().unwrap()).unwrap(),
            nonce: B64.decode(body["nonce"].as_str().unwrap()).unwrap().try_into().unwrap(),
            user: body["user"].as_str().unwrap().into(), action: body["action"].as_str().unwrap().into(),
            issued_at: body["issued_at"].as_u64().unwrap(), expires_at: body["expires_at"].as_u64().unwrap(),
        }
    }
    fn sign(&self, message: &[u8]) -> String {
        B64.encode(Signer::new(MessageDigest::sha256(), &self.key).unwrap().sign_oneshot_to_vec(message).unwrap())
    }
    fn approve(&mut self, body: &Map<String, Value>) -> Value {
        assert_eq!(body["v"], 1);
        let challenge = Self::challenge(body);
        let signature = self.sign(&challenge.message(&self.binding, &self.public.id()));
        let mut answer = json!({"v": 1, "op": "response", "request_id": body["request_id"], "decision": "approved", "signature": signature});
        if challenge.purpose == Purpose::Enrol { answer["op"] = json!("enrolled"); answer["public_key"] = json!(self.public.base64()); }
        self.send(phonekey::CAPABILITY, answer.clone());
        answer
    }
}

fn call(socket: &Path, op: &str, id: Option<&str>, args: Value) -> Value {
    request_within(socket, &Request { op: op.into(), device_id: id.map(Into::into), attempt: None, args: serde_json::from_value(args).unwrap() }, Duration::from_secs(40)).unwrap()
}
/// A request that waits on the phone, made from another thread as a client would.
fn ask(socket: &Path, op: &str, id: Option<String>, args: Value) -> thread::JoinHandle<Value> {
    let socket = socket.to_owned();
    let op = op.to_owned();
    thread::spawn(move || call(&socket, &op, id.as_deref(), args))
}
fn wait(socket: &Path, test: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(state) = request(socket, &Request { op: "state".into(), device_id: None, attempt: None, args: Default::default() }) { if test(&state) { return state; } }
        assert!(Instant::now() < end, "daemon state deadline");
        thread::sleep(Duration::from_millis(50));
    }
}
fn device<'a>(state: &'a Value, id: &str) -> Option<&'a Value> { state["state"]["devices"].as_array()?.iter().find(|d| d["id"] == id) }
fn key_of<'a>(status: &'a Value, id: &str) -> &'a Value { status["phonekey"]["devices"].as_array().unwrap().iter().find(|d| d["id"] == id).expect("device in phone-key status") }

struct Fixture { _temp: tempfile::TempDir, _daemon: Daemon, socket: PathBuf, state: PathBuf, phone_dir: PathBuf, port: u16, shell: Shell, desktop: Store }
fn fixture(env: &[(&str, &str)]) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let (state, socket, shell_path) = (temp.path().join("state"), temp.path().join("runtime/control.sock"), temp.path().join("shell.sock"));
    let shell = Shell::start(&shell_path);
    let mut command = Command::new(env!("CARGO_BIN_EXE_des-connectd"));
    command.arg("--loopback").args(["--socket", socket.to_str().unwrap(), "--state-dir", state.to_str().unwrap(), "--name", "Fixture desktop"])
        .env("DES_UI_CONTROL_SOCKET", &shell_path).env("USER", "fixture").stdout(Stdio::null()).stderr(Stdio::null());
    for (name, value) in env { command.env(name, value); }
    let daemon = Daemon(command.spawn().unwrap());
    let ready = wait(&socket, |state| state["ok"] == true);
    let port = ready["state"]["tcp_port"].as_u64().unwrap() as u16;
    thread::sleep(Duration::from_millis(300));
    let desktop = Store::open(&state).unwrap();
    Fixture { phone_dir: temp.path().join("phone"), _temp: temp, _daemon: daemon, socket, state, port, shell, desktop }
}
/// The name the daemon resolves for this process's user, as a request must give it.
fn me() -> String {
    let uid = rustix::process::getuid().as_raw();
    std::fs::read_to_string("/etc/passwd").unwrap().lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.get(2)?.parse::<u32>().ok()? == uid).then(|| fields[0].to_owned())
    }).unwrap_or_else(|| "fixture".into())
}
fn spacing() { thread::sleep(Duration::from_millis(2100)); }

#[test] fn enrolment_then_signed_approval_replay_forgery_lock_state_and_unpair() {
    let f = fixture(&[]);
    let mut phone = Phone::connect(f.port, &f.phone_dir, &f.desktop, true);
    let id = phone.id();
    phone.pair(&f.socket);

    // Nothing is on after pairing: no key, no capability, nothing sent to the phone.
    let before = call(&f.socket, "phonekey_status", None, json!({}));
    let mine = key_of(&before, &id);
    assert_eq!((mine["enrolled"].clone(), mine["unlock"].clone(), mine["approve"].clone(), mine["walkaway"].clone()), (json!(false), json!(false), json!(false), json!(false)));
    let refused = call(&f.socket, "phonekey_request", None, json!({"purpose": "approve", "action": "sudo true"}));
    assert_eq!(refused["ok"], false);
    assert!(refused["error"].as_str().unwrap().starts_with("disabled"));
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "approve", "enabled": true}))["ok"], false, "no capability without an enrolled key");
    assert!(phone.hears_nothing(Duration::from_millis(600)));

    // Enrolment: the phone proves it holds the new key, over this pairing.
    let enrolling = ask(&f.socket, "phonekey_enrol", Some(id.clone()), json!({}));
    let enrol = phone.expect("enrol");
    assert_eq!(enrol["purpose"], "enrol");
    assert!(enrol["action"].as_str().unwrap().contains("Fixture desktop"));
    phone.approve(&enrol);
    let enrolled = enrolling.join().unwrap();
    assert_eq!(enrolled["ok"], true, "{enrolled}");
    assert_eq!(enrolled["key_id"], hex(&phone.public.id()));
    assert_eq!(enrolled["fingerprint"], phonekey::fingerprint(&phone.public.id()));
    let after = call(&f.socket, "phonekey_status", None, json!({}));
    assert_eq!(key_of(&after, &id)["enrolled"], true);
    assert_eq!(key_of(&after, &id)["approve"], false, "an enrolled key approves nothing until switched on");
    assert_eq!(call(&f.socket, "phonekey_request", None, json!({"purpose": "approve", "action": "sudo true"}))["ok"], false);
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "approve", "enabled": true}))["ok"], true);

    // A signed approval, checked again here from the bytes alone.
    spacing();
    let asking = ask(&f.socket, "phonekey_request", None, json!({"purpose": "approve", "action": "sudo apt upgrade"}));
    let asked = phone.expect("request");
    assert_eq!((asked["purpose"].as_str(), asked["action"].as_str(), asked["user"].as_str()), (Some("approve"), Some("sudo apt upgrade"), Some(me().as_str())));
    assert_eq!(asked["key_id"], hex(&phone.public.id()));
    let first_answer = phone.approve(&asked);
    let answer = asking.join().unwrap();
    assert_eq!((answer["ok"].clone(), answer["decision"].clone()), (json!(true), json!("approved")), "{answer}");
    let signature = B64.decode(answer["signature"].as_str().unwrap()).unwrap();
    let challenge = Phone::challenge(answer.as_object().unwrap());
    let key = PublicKey::from_base64(answer["public_key"].as_str().unwrap()).unwrap();
    assert_eq!(phonekey::verify_answer(&challenge, &phone.binding, &key, &signature, challenge.issued_at), Ok(()));
    assert_eq!(answer["desktop_cert"], hex(&phone.binding.desktop_cert));
    assert_eq!(answer["phone_cert"], hex(&phone.binding.phone_cert));

    // The same answer again approves nothing; neither does it answer a new request.
    phone.send(phonekey::CAPABILITY, first_answer.clone());
    spacing();
    let asking = ask(&f.socket, "phonekey_request", None, json!({"purpose": "approve", "action": "sudo apt upgrade"}));
    let asked = phone.expect("request");
    assert_ne!(asked["nonce"], answer["nonce"]);
    phone.send(phonekey::CAPABILITY, first_answer.clone());
    let mut forged = first_answer.clone();
    forged["request_id"] = asked["request_id"].clone();
    phone.send(phonekey::CAPABILITY, forged);
    let answer = asking.join().unwrap();
    assert_eq!(answer["decision"], "bad_signature", "{answer}");
    assert!(answer.get("signature").is_none());

    // A request may only name this session's own user.
    let other = call(&f.socket, "phonekey_request", None, json!({"purpose": "approve", "action": "sudo true", "user": "mallory"}));
    assert!(other["error"].as_str().unwrap().starts_with("wrong_user"), "{other}");

    // A caller's own challenge comes back signed as given, for the caller to verify.
    spacing();
    let own = Challenge::new(Purpose::Approve, &me(), "pkexec systemctl restart ssh", wall(), 20).unwrap();
    let mut fields = own.body(None);
    fields.as_object_mut().unwrap().remove("op");
    let asking = ask(&f.socket, "phonekey_request", Some(id.clone()), fields);
    let asked = phone.expect("request");
    assert_eq!(Phone::challenge(&asked), own);
    phone.approve(&asked);
    let answer = asking.join().unwrap();
    assert_eq!(answer["decision"], "approved", "{answer}");
    assert_eq!(phonekey::verify_answer(&own, &phone.binding, &phone.public, &B64.decode(answer["signature"].as_str().unwrap()).unwrap(), own.issued_at), Ok(()));

    // Locked: no approvals. An unlock request is only made of a locked session, and
    // its decision is reported; the shell is never told to unlock.
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "unlock", "enabled": true}))["ok"], true);
    let early = call(&f.socket, "phonekey_request", None, json!({"purpose": "unlock", "action": "Unlock this DES UI session"}));
    assert!(early["error"].as_str().unwrap().contains("locked"), "{early}");
    f.shell.locked.store(true, Ordering::SeqCst);
    let locked = call(&f.socket, "phonekey_request", None, json!({"purpose": "approve", "action": "sudo true"}));
    assert!(locked["error"].as_str().unwrap().contains("unlocked"), "{locked}");
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "approve", "enabled": false}))["ok"], false, "settings do not change while locked");
    spacing();
    let asking = ask(&f.socket, "phonekey_request", None, json!({"purpose": "unlock", "action": "Unlock this DES UI session"}));
    let asked = phone.expect("request");
    assert_eq!(asked["purpose"], "unlock");
    phone.approve(&asked);
    assert_eq!(asking.join().unwrap()["decision"], "approved");
    assert!(f.shell.locked.load(Ordering::SeqCst), "a verified unlock decision does not itself unlock");
    assert!(f.shell.only_asked_state_or_lock());
    assert_eq!(f.shell.count("lock"), 0);

    // Unpairing ends it at once, and takes the key with it.
    assert_eq!(call(&f.socket, "unpair", Some(&id), json!({}))["ok"], true);
    let gone = call(&f.socket, "phonekey_request", Some(&id), json!({"purpose": "unlock", "action": "Unlock this DES UI session"}));
    assert_eq!(gone["ok"], false);
    assert!(gone["phonekey"]["devices"].as_array().unwrap().is_empty());
    assert!(Store::open(&f.state).unwrap().peers.is_empty());

    let audit = std::fs::read_to_string(f.state.join("phonekey-audit.log")).unwrap();
    let events: Vec<Value> = audit.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let count = |event: &str, field: &str, value: &str| events.iter().filter(|e| e["event"] == event && e[field] == value).count();
    assert_eq!(count("request", "purpose", "approve"), 3);
    assert_eq!(count("decision", "decision", "approved"), 4, "the enrolment, two approvals and the unlock");
    assert_eq!(count("decision", "decision", "bad_signature"), 1);
    assert_eq!(count("ignored", "reason", "unknown_request"), 2, "both replays were seen and dropped");
    assert_eq!(count("refused", "reason", "wrong_user"), 1);
    assert_eq!(events.iter().filter(|e| e["event"] == "enrolled" || e["event"] == "revoked").count(), 2);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(f.state.join("phonekey-audit.log")).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test] fn a_phone_that_walks_away_locks_once_and_coming_back_unlocks_nothing() {
    let f = fixture(&[("DES_CONNECT_PRESENCE_MS", "250"), ("DES_CONNECT_WALKAWAY_MS", "1500"), ("DES_CONNECT_WALKAWAY_CHECK_MS", "250")]);
    let mut phone = Phone::connect(f.port, &f.phone_dir, &f.desktop, true);
    let id = phone.id();
    phone.pair(&f.socket);
    assert!(phone.hears_nothing(Duration::from_millis(800)), "no presence questions until walk-away is on");
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "walkaway", "enabled": true, "seconds": 5}))["ok"], false, "the grace period has a floor");
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "walkaway", "enabled": true, "seconds": 30}))["ok"], true);

    // Present: answered heartbeats arm it and nothing locks.
    let end = Instant::now() + Duration::from_millis(2500);
    let mut last = 0;
    while Instant::now() < end {
        let question = phone.expect("presence");
        let seq = question["seq"].as_u64().unwrap();
        assert!(seq > last);
        last = seq;
        phone.send(phonekey::CAPABILITY, json!({"v": 1, "op": "present", "seq": seq}));
    }
    let status = call(&f.socket, "phonekey_status", None, json!({}));
    assert_eq!((key_of(&status, &id)["present"].clone(), key_of(&status, &id)["armed"].clone()), (json!(true), json!(true)), "{status}");
    assert_eq!(f.shell.count("lock"), 0);

    // An answer to an old question, or a repeated one, does not keep the session open.
    let end = Instant::now() + Duration::from_secs(6);
    while f.shell.count("lock") == 0 {
        assert!(Instant::now() < end, "walk-away never locked");
        phone.send(phonekey::CAPABILITY, json!({"v": 1, "op": "present", "seq": last}));
        thread::sleep(Duration::from_millis(100));
    }
    assert!(f.shell.locked.load(Ordering::SeqCst));

    // Back again, answering properly: still locked, never asked to unlock, locked once.
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        let question = phone.expect("presence");
        phone.send(phonekey::CAPABILITY, json!({"v": 1, "op": "present", "seq": question["seq"]}));
    }
    let status = call(&f.socket, "phonekey_status", None, json!({}));
    assert_eq!(key_of(&status, &id)["present"], true);
    assert!(f.shell.locked.load(Ordering::SeqCst), "presence never unlocks");
    assert_eq!(f.shell.count("lock"), 1);
    assert!(f.shell.only_asked_state_or_lock());
    let audit = std::fs::read_to_string(f.state.join("phonekey-audit.log")).unwrap();
    assert_eq!(audit.lines().filter(|line| line.contains("\"walkaway_lock\"")).count(), 1);
}

#[test] fn a_phone_without_the_capability_is_never_sent_a_phone_key_packet() {
    let f = fixture(&[("DES_CONNECT_PRESENCE_MS", "250"), ("DES_CONNECT_WALKAWAY_MS", "1000"), ("DES_CONNECT_WALKAWAY_CHECK_MS", "250")]);
    let mut phone = Phone::connect(f.port, &f.phone_dir, &f.desktop, false);
    let id = phone.id();
    phone.pair(&f.socket);
    assert_eq!(call(&f.socket, "phonekey_set", Some(&id), json!({"capability": "walkaway", "enabled": true}))["ok"], true);
    let enrol = call(&f.socket, "phonekey_enrol", Some(&id), json!({}));
    assert_eq!(enrol["ok"], false);
    assert!(enrol["error"].as_str().unwrap().contains("does not support"), "{enrol}");
    // An old phone that somehow sends our packets is not believed either.
    phone.send(phonekey::CAPABILITY, json!({"v": 1, "op": "present", "seq": 1}));
    assert!(phone.hears_nothing(Duration::from_millis(2500)));
    let status = call(&f.socket, "phonekey_status", None, json!({}));
    assert_eq!((key_of(&status, &id)["present"].clone(), key_of(&status, &id)["armed"].clone()), (json!(false), json!(false)));
    assert_eq!(f.shell.count("lock"), 0, "a phone that was never here locks nothing");
}

fn wall() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
