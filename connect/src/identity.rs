//! Private durable identities and certificate pins. Independently implemented v8 compatibility.
use openssl::{asn1::Asn1Time,bn::BigNum,hash::MessageDigest,nid::Nid,pkey::{PKey,Private},rsa::Rsa,x509::{X509,X509NameBuilder}};
use serde::{Deserialize,Serialize};
use std::{collections::BTreeMap,fs::{self,File,OpenOptions},io::{Read,Write},os::unix::fs::{MetadataExt,OpenOptionsExt,PermissionsExt},path::{Path,PathBuf}};
pub type Result<T> = std::result::Result<T,Box<dyn std::error::Error+Send+Sync>>;
pub fn hex(bytes:&[u8])->String {bytes.iter().map(|b|format!("{b:02x}")).collect()}
pub fn unhex(value:&str)->Result<Vec<u8>> {
 if value.len()%2!=0 || !value.is_ascii(){return Err("invalid hex".into());}
 (0..value.len()).step_by(2).map(|i|u8::from_str_radix(&value[i..i+2],16).map_err(Into::into)).collect()
}
pub fn valid_id(id:&str)->bool {(32..=38).contains(&id.len())&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b))}
pub fn private_dir(path:&Path)->Result<()> {
 if !path.exists(){fs::create_dir_all(path)?;}
 let m=fs::symlink_metadata(path)?;
 if !m.is_dir()||m.file_type().is_symlink()||m.uid()!=rustix::process::getuid().as_raw(){return Err("private directory has wrong owner/type".into());}
 fs::set_permissions(path,fs::Permissions::from_mode(0o700))?;Ok(())
}
fn read_private(path:&Path)->Result<Vec<u8>> {
 let m=fs::symlink_metadata(path)?;
 if !m.is_file()||m.file_type().is_symlink()||m.uid()!=rustix::process::getuid().as_raw()||m.mode()&0o077!=0||m.len()>256*1024 {return Err("private file has unsafe owner/type/permissions/size".into());}
 let mut bytes=Vec::new();File::open(path)?.take(256*1024+1).read_to_end(&mut bytes)?;
 if bytes.len()>256*1024{return Err("private file too large".into());}Ok(bytes)
}
fn save_private<T:Serialize>(path:&Path,value:&T)->Result<()> {
 if path.exists(){read_private(path)?;}
 let mut random=[0;8];openssl::rand::rand_bytes(&mut random)?;
 let temporary=path.with_extension(format!("tmp-{}",hex(&random)));
 let result=(||->Result<()>{
  let mut f=OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
  f.write_all(&serde_json::to_vec(value)?)?;f.sync_all()?;fs::rename(&temporary,path)?;
  if let Some(parent)=path.parent(){File::open(parent)?.sync_all()?;}Ok(())
 })();if result.is_err(){let _=fs::remove_file(&temporary);}result
}
#[derive(Serialize,Deserialize)]
struct LocalIdentity {id:String,key_pem:String,cert_pem:String}
#[derive(Clone,Serialize,Deserialize)]
pub struct PeerRecord {pub id:String,pub name:String,pub kind:String,pub certificate:String,#[serde(default)] pub files_enabled:bool,#[serde(default)] pub media_enabled:bool,#[serde(default)] pub volume_enabled:bool}
pub struct Store {directory:PathBuf,id:String,pub key:PKey<Private>,pub certificate:X509,pub peers:BTreeMap<String,PeerRecord>}
impl Store {
 pub fn open(directory:&Path)->Result<Self> {
  private_dir(directory)?;let path=directory.join("identity.json");
  let local:LocalIdentity=if path.exists(){serde_json::from_slice(&read_private(&path)?)?}else{
   let mut random=[0;16];openssl::rand::rand_bytes(&mut random)?;let id=hex(&random);
   let key=PKey::from_rsa(Rsa::generate(2048)?)?;
   let mut name=X509NameBuilder::new()?;name.append_entry_by_nid(Nid::COMMONNAME,&id)?;name.append_entry_by_nid(Nid::ORGANIZATIONNAME,"DES UI")?;let name=name.build();
   let mut cert=X509::builder()?;cert.set_version(2)?;
   let serial=BigNum::from_slice(&random)?.to_asn1_integer()?;cert.set_serial_number(&serial)?;
   cert.set_subject_name(&name)?;cert.set_issuer_name(&name)?;cert.set_pubkey(&key)?;
   let start=Asn1Time::from_unix(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64-86400)?;let end=Asn1Time::days_from_now(3650)?;
   cert.set_not_before(&start)?;cert.set_not_after(&end)?;cert.sign(&key,MessageDigest::sha256())?;let cert=cert.build();
   let local=LocalIdentity{id,key_pem:String::from_utf8(key.private_key_to_pem_pkcs8()?)?,cert_pem:String::from_utf8(cert.to_pem()?)?};save_private(&path,&local)?;local
  };
  if !valid_id(&local.id){return Err("invalid stored device ID".into());}
  let key=PKey::private_key_from_pem(local.key_pem.as_bytes())?;let certificate=X509::from_pem(local.cert_pem.as_bytes())?;
  if !certificate.public_key()?.public_eq(&key){return Err("stored identity key does not match certificate".into());}
  peer_digest(&certificate,&local.id)?;
  let pins=directory.join("peers.json");let peers:BTreeMap<String,PeerRecord>=if pins.exists(){serde_json::from_slice(&read_private(&pins)?)?}else{BTreeMap::new()};
  if peers.len()>64{return Err("too many stored peers".into());}
  for (id,peer) in &peers{if id!=&peer.id{return Err("inconsistent peer record".into());}peer_digest(&X509::from_der(&unhex(&peer.certificate)?)?,id)?;}
  Ok(Self{directory:directory.to_owned(),id:local.id,key,certificate,peers})
 }
 pub fn id(&self)->&str {&self.id}
 pub fn pin(&self,id:&str)->Result<Option<[u8;32]>> {
  self.peers.get(id).map(|p|peer_digest(&X509::from_der(&unhex(&p.certificate)?)?,id)).transpose()
 }
 pub fn remember(&mut self,id:&str,name:&str,kind:&str,cert:&X509)->Result<()> {
  let pin=peer_digest(cert,id)?;
  if self.pin(id)?.is_some_and(|old|old!=pin){return Err("certificate changed; unpair explicitly first".into());}
  if self.peers.len()>=64&&!self.peers.contains_key(id){return Err("paired-device limit reached".into());}
  let record=PeerRecord{id:id.into(),name:name.chars().take(256).collect(),kind:kind.into(),certificate:hex(&cert.to_der()?),files_enabled:self.peers.get(id).is_some_and(|peer|peer.files_enabled),media_enabled:self.peers.get(id).is_some_and(|peer|peer.media_enabled),volume_enabled:self.peers.get(id).is_some_and(|peer|peer.volume_enabled)};
  let mut peers=self.peers.clone();peers.insert(id.into(),record);save_private(&self.directory.join("peers.json"),&peers)?;self.peers=peers;Ok(())
 }
 pub fn set_files_enabled(&mut self,id:&str,enabled:bool)->Result<()> {
  let mut peers=self.peers.clone();let peer=peers.get_mut(id).ok_or("file permission requires a trusted paired peer")?;
  peer.files_enabled=enabled;save_private(&self.directory.join("peers.json"),&peers)?;self.peers=peers;Ok(())
 }
 pub fn set_media_enabled(&mut self,id:&str,enabled:bool)->Result<()> {
  let mut peers=self.peers.clone();peers.get_mut(id).ok_or("media permission requires a trusted paired peer")?.media_enabled=enabled;
  save_private(&self.directory.join("peers.json"),&peers)?;self.peers=peers;Ok(())
 }
 pub fn set_volume_enabled(&mut self,id:&str,enabled:bool)->Result<()> {
  let mut peers=self.peers.clone();peers.get_mut(id).ok_or("volume permission requires a trusted paired peer")?.volume_enabled=enabled;
  save_private(&self.directory.join("peers.json"),&peers)?;self.peers=peers;Ok(())
 }
 pub fn forget(&mut self,id:&str)->Result<()> {
  let mut peers=self.peers.clone();peers.remove(id);save_private(&self.directory.join("peers.json"),&peers)?;self.peers=peers;Ok(())
 }
}
pub fn peer_digest(cert:&X509,id:&str)->Result<[u8;32]> {
 if !valid_id(id){return Err("invalid peer ID".into());}
 let names:Vec<_>=cert.subject_name().entries_by_nid(Nid::COMMONNAME).collect();
 if names.len()!=1||names[0].data().to_string()?!=id{return Err("TLS certificate identity mismatch".into());}
 let public=cert.public_key()?;
 if public.bits()<2048&&public.id()==openssl::pkey::Id::RSA{return Err("weak RSA identity".into());}
 if !cert.verify(&public)?{return Err("invalid self-signed peer certificate".into());}
 let now=Asn1Time::days_from_now(0)?;
 if cert.not_before()>now||cert.not_after()<now{return Err("peer certificate is not currently valid".into());}
 Ok(cert.digest(MessageDigest::sha256())?.as_ref().try_into()?)
}
pub fn verification_key(a:&X509,b:&X509,timestamp:i64)->Result<String> {
 let a=a.public_key()?.public_key_to_der()?;let b=b.public_key()?.public_key_to_der()?;
 let mut bytes=if a>=b{[a,b].concat()}else{[b,a].concat()};bytes.extend(timestamp.to_string().as_bytes());
 Ok(hex(&openssl::sha::sha256(&bytes)[..4]).to_uppercase())
}
