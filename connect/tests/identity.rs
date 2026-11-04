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

#[test] fn file_permission_defaults_off_persists_and_remember_preserves_it() {
 let d=tempfile::tempdir().unwrap();let peer=tempfile::tempdir().unwrap();
 let peer=Store::open(&peer.path().join("p")).unwrap();let path=d.path().join("s");let mut store=Store::open(&path).unwrap();
 store.remember(peer.id(),"Phone","phone",&peer.certificate).unwrap();
 assert!(!store.peers[peer.id()].files_enabled);
 let pin=store.pin(peer.id()).unwrap();store.set_files_enabled(peer.id(),true).unwrap();
 store.remember(peer.id(),"Renamed phone","phone",&peer.certificate).unwrap();
 assert!(store.peers[peer.id()].files_enabled);assert_eq!(store.pin(peer.id()).unwrap(),pin);
 assert!(Store::open(&path).unwrap().peers[peer.id()].files_enabled);
 store.set_files_enabled(peer.id(),false).unwrap();assert!(!Store::open(&path).unwrap().peers[peer.id()].files_enabled);
 assert!(store.set_files_enabled("unknown",true).is_err());
 let path=path.join("peers.json");let mut json:serde_json::Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();json[peer.id()].as_object_mut().unwrap().remove("files_enabled");fs::write(&path,serde_json::to_vec(&json).unwrap()).unwrap();
 assert!(!Store::open(path.parent().unwrap()).unwrap().peers[peer.id()].files_enabled);
}

#[test] fn media_and_volume_permissions_are_independent_default_off_and_preserved(){
 let temp=tempfile::tempdir().unwrap();let remote=Store::open(&temp.path().join("phone")).unwrap();let path=temp.path().join("store");let mut store=Store::open(&path).unwrap();store.remember(remote.id(),"Phone","phone",&remote.certificate).unwrap();
 assert!(!store.peers[remote.id()].media_enabled);assert!(!store.peers[remote.id()].volume_enabled);
 let pin=store.pin(remote.id()).unwrap();store.set_files_enabled(remote.id(),true).unwrap();store.set_media_enabled(remote.id(),true).unwrap();assert!(!store.peers[remote.id()].volume_enabled);store.set_volume_enabled(remote.id(),true).unwrap();
 store.remember(remote.id(),"Updated phone","phone",&remote.certificate).unwrap();let loaded=Store::open(&path).unwrap();let peer=&loaded.peers[remote.id()];assert!(peer.files_enabled&&peer.media_enabled&&peer.volume_enabled);assert_eq!(loaded.pin(remote.id()).unwrap(),pin);
 store.set_media_enabled(remote.id(),false).unwrap();assert!(store.peers[remote.id()].files_enabled&&store.peers[remote.id()].volume_enabled);
 assert!(store.set_media_enabled("unknown",true).is_err());assert!(store.set_volume_enabled("unknown",true).is_err());
 let file=path.join("peers.json");let mut data:serde_json::Value=serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();data[remote.id()].as_object_mut().unwrap().remove("media_enabled");data[remote.id()].as_object_mut().unwrap().remove("volume_enabled");fs::write(file,serde_json::to_vec(&data).unwrap()).unwrap();let legacy=Store::open(&path).unwrap();assert!(!legacy.peers[remote.id()].media_enabled&&!legacy.peers[remote.id()].volume_enabled);assert!(legacy.peers[remote.id()].files_enabled);
}
