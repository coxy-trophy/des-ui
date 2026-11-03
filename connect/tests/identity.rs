use des_connect::identity::{Store,verification_key,peer_digest};
use std::{fs,os::unix::fs::PermissionsExt};
#[test] fn identity_is_private_stable_and_bound_to_its_certificate() {
 let dir=tempfile::tempdir().unwrap();let p=dir.path().join("state");
 let a=Store::open(&p).unwrap();let id=a.id().to_owned();let cert=a.certificate.to_der().unwrap();
 assert_eq!(peer_digest(&a.certificate,&id).unwrap().len(),32);
 assert!(peer_digest(&a.certificate,"00000000000000000000000000000000").is_err());
 assert_eq!(fs::metadata(p.join("identity.json")).unwrap().permissions().mode()&0o777,0o600);
 assert_eq!(fs::metadata(&p).unwrap().permissions().mode()&0o777,0o700);
 let b=Store::open(&p).unwrap();assert_eq!(b.id(),id);assert_eq!(b.certificate.to_der().unwrap(),cert);
}
#[test] fn verification_code_is_symmetric_and_timestamp_bound() {
 let a=tempfile::tempdir().unwrap();let b=tempfile::tempdir().unwrap();
 let a=Store::open(&a.path().join("a")).unwrap();let b=Store::open(&b.path().join("b")).unwrap();
 let x=verification_key(&a.certificate,&b.certificate,123).unwrap();
 assert_eq!(x.len(),8);assert_eq!(x,verification_key(&b.certificate,&a.certificate,123).unwrap());
 assert_ne!(x,verification_key(&a.certificate,&b.certificate,124).unwrap());
}
#[test] fn pins_persist_and_forget_without_identity_rotation() {
 let d=tempfile::tempdir().unwrap();let peer=tempfile::tempdir().unwrap();
 let peer=Store::open(&peer.path().join("p")).unwrap();let mut s=Store::open(&d.path().join("s")).unwrap();
 s.remember(peer.id(),"Tablet","tablet",&peer.certificate).unwrap();
 let mut s=Store::open(&d.path().join("s")).unwrap();assert_eq!(s.pin(peer.id()).unwrap(),Some(peer_digest(&peer.certificate,peer.id()).unwrap()));
 s.forget(peer.id()).unwrap();assert!(Store::open(&d.path().join("s")).unwrap().pin(peer.id()).unwrap().is_none());
}
#[test] fn symlink_and_world_readable_identity_are_refused() {
 let d=tempfile::tempdir().unwrap();let p=d.path().join("s");Store::open(&p).unwrap();
 let file=p.join("identity.json");fs::set_permissions(&file,fs::Permissions::from_mode(0o644)).unwrap();assert!(Store::open(&p).is_err());
 fs::set_permissions(&file,fs::Permissions::from_mode(0o600)).unwrap();fs::rename(&file,p.join("real.json")).unwrap();
 std::os::unix::fs::symlink(p.join("real.json"),&file).unwrap();assert!(Store::open(&p).is_err());
}
