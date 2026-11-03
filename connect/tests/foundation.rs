use des_connect::{packet::{Packet, Identity, MAX_FRAME}, pairing::{Pairing, State}};
use serde_json::json;
use std::io::{BufReader, Cursor};
fn identity() -> Packet {
    serde_json::from_value(json!({"id":1,"type":"kdeconnect.identity","body":{
        "deviceId":"0123456789abcdef0123456789abcdef", "deviceName":"Test phone",
        "deviceType":"phone", "protocolVersion":8, "tcpPort":1716,
        "incomingCapabilities":["kdeconnect.ping"], "outgoingCapabilities":["kdeconnect.battery"]
    }})).unwrap()
}
#[test] fn frames_roundtrip_and_leave_the_next_packet_unread() {
    let p=identity(); let bytes=p.encode().unwrap();
    let mut reader=BufReader::new(Cursor::new([bytes.clone(),bytes].concat()));
    assert_eq!(Packet::read(&mut reader).unwrap(),p);
    assert_eq!(Packet::read(&mut reader).unwrap(),p);
    assert!(Packet::read(&mut reader).is_err());
}
#[test] fn malformed_partial_and_oversized_frames_fail() {
    for bytes in [b"{}\n".to_vec(),b"{\n".to_vec(),identity().encode().unwrap().into_iter().filter(|b|*b!=b'\n').collect(),vec![b'x';MAX_FRAME+1]] {
        assert!(Packet::read(&mut BufReader::new(Cursor::new(bytes))).is_err());
    }
    let mut p=identity();p.body.insert("large".into(),json!("x".repeat(MAX_FRAME)));
    assert!(p.encode().is_err());
}
#[test] fn identity_is_validated_without_conferring_trust() {
    let p=identity();let id=Identity::from_packet(&p).unwrap();assert_eq!(id.name,"Test phone");assert_eq!(id.protocol_version,8);
    for (field,bad) in [("deviceId",json!("../../bad")),("deviceName",json!("\n")),("protocolVersion",json!(7)),("tcpPort",json!(65536)),("incomingCapabilities",json!(["x".repeat(129)]))] {
        let mut p=p.clone();p.body.insert(field.into(),bad);assert!(Identity::from_packet(&p).is_err(),"{field}");
    }
}
#[test] fn packets_reject_wrong_shape_and_invalid_payload_metadata() {
    for value in [json!({"type":"","body":{}}),json!({"type":"a","body":[]}),json!({"type":"kdeconnect.share.request","body":{},"payloadSize":5}),json!({"type":"a","body":{},"payloadSize":-2,"payloadTransferInfo":{}})] {
        let bytes=[serde_json::to_vec(&value).unwrap(),vec![b'\n']].concat();assert!(Packet::read(&mut BufReader::new(Cursor::new(bytes))).is_err());
    }
}
#[test] fn unknown_fields_survive_packet_roundtrip() {
    let mut p=identity();p.extra.insert("future".into(),json!({"value":1}));
    assert_eq!(Packet::read(&mut BufReader::new(Cursor::new(p.encode().unwrap()))).unwrap(),p);
}
#[test] fn pairing_requires_explicit_acceptance_and_successful_send() {
    let mut p=Pairing::default();assert!(!p.authorized());
    let s=p.connected([1;32]).unwrap();let a=p.incoming(s,100,100,0).unwrap();
    assert_eq!(p.state(),State::Incoming);assert!(!p.authorized());
    p.accept(s,a,1).unwrap();assert_eq!(p.state(),State::Accepting);assert!(!p.authorized());
    p.accept_sent(s,a,2).unwrap();assert!(p.authorized());assert_eq!(p.pinned(),Some([1;32]));
    p.disconnected(s);assert!(!p.authorized());p.connected([1;32]).unwrap();assert!(p.authorized());
}
#[test] fn disconnect_expires_pending_decisions_and_key_mismatch_is_denied() {
    let mut p=Pairing::default();let old=p.connected([1;32]).unwrap();let a=p.incoming(old,100,100,0).unwrap();
    p.disconnected(old);let new=p.connected([2;32]).unwrap();assert!(p.accept(old,a,1).is_err());assert!(!p.authorized());
    let b=p.incoming(new,100,100,1).unwrap();p.accept(new,b,2).unwrap();p.accept_sent(new,b,3).unwrap();p.disconnected(new);
    assert!(p.connected([3;32]).is_err());assert!(!p.authorized());assert_eq!(p.pinned(),Some([2;32]));
}
#[test] fn clock_skew_timeout_failed_send_and_remote_rejection_never_pair() {
    let mut p=Pairing::default();let s=p.connected([1;32]).unwrap();
    assert!(p.incoming(s,100,2000,0).is_err());let a=p.incoming(s,100,100,0).unwrap();assert!(p.accept(s,a,25).is_err());
    let b=p.incoming(s,101,101,26).unwrap();p.accept(s,b,27).unwrap();p.send_failed(s,b);assert!(!p.authorized());assert_eq!(p.pinned(),None);
    let c=p.request(s,30).unwrap();p.peer_accepts(s,c,60).unwrap_err();assert!(!p.authorized());
    let d=p.request(s,61).unwrap();p.peer_accepts(s,d,62).unwrap();assert!(p.authorized());p.unpair(s).unwrap();assert!(!p.authorized());assert_eq!(p.pinned(),None);
}
#[test] fn duplicate_request_does_not_extend_deadline_and_stale_disconnect_is_ignored() {
    let mut p=Pairing::default();let old=p.connected([1;32]).unwrap();let a=p.incoming(old,100,100,0).unwrap();assert!(p.incoming(old,101,101,24).is_err());assert!(p.accept(old,a,25).is_err());
    p.disconnected(old);let new=p.connected([1;32]).unwrap();p.disconnected(old);assert_eq!(p.state(),State::Unpaired);p.incoming(new,100,100,26).unwrap();
}
#[test] fn cancelled_attempt_cannot_approve_a_new_request_on_the_same_connection() {
    let mut p=Pairing::default();let s=p.connected([1;32]).unwrap();
    let a=p.incoming(s,100,100,0).unwrap();p.cancel(s,a).unwrap();
    let b=p.incoming(s,101,101,1).unwrap();
    assert!(p.accept(s,a,2).is_err(),"old approval must not approve the new request");
    assert!(p.cancel(s,a).is_err(),"old rejection must not cancel the new request");
    p.accept(s,b,2).unwrap();assert!(!p.authorized());
}
#[test] fn old_send_callbacks_must_not_authorize_or_cancel_a_new_attempt() {
    let mut p=Pairing::default();let s=p.connected([1;32]).unwrap();
    let a=p.incoming(s,100,100,0).unwrap();p.accept(s,a,1).unwrap();p.cancel(s,a).unwrap();
    let b=p.incoming(s,101,101,2).unwrap();p.send_failed(s,a);
    assert_eq!(p.state(),State::Incoming);p.accept(s,b,3).unwrap();
    assert!(p.accept_sent(s,a,4).is_err());assert!(!p.authorized());
    p.send_failed(s,a);assert_eq!(p.state(),State::Accepting);
    p.accept_sent(s,b,4).unwrap();assert!(p.authorized());
}
#[test] fn frame_limit_accepts_exact_boundary_and_rejects_one_more_byte() {
    let prefix=b"{\"type\":\"kdeconnect.ping\",\"body\":{\"text\":\"";
    let suffix=b"\"}}\n";
    let frame=[prefix.to_vec(),vec![b'x';MAX_FRAME-prefix.len()-suffix.len()],suffix.to_vec()].concat();
    assert_eq!(frame.len(),MAX_FRAME);assert!(Packet::read(&mut BufReader::new(Cursor::new(&frame))).is_ok());
    let mut larger=frame;larger.insert(prefix.len(),b'x');assert!(Packet::read(&mut BufReader::new(Cursor::new(larger))).is_err());
}
#[test] fn identity_limits_are_enforced_at_the_boundary() {
    let mut p=identity();p.body.insert("deviceName".into(),json!("a".repeat(256)));assert!(Identity::from_packet(&p).is_ok());
    p.body.insert("deviceName".into(),json!("a".repeat(257)));assert!(Identity::from_packet(&p).is_err());
    let mut p=identity();p.body.insert("incomingCapabilities".into(),json!(vec!["kdeconnect.ping";256]));assert!(Identity::from_packet(&p).is_ok());
    p.body.insert("incomingCapabilities".into(),json!(vec!["kdeconnect.ping";257]));assert!(Identity::from_packet(&p).is_err());
}
