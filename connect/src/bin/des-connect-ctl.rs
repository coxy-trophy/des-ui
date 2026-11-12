use des_connect::{identity::{self,Result},phonekey::{self,Binding,Challenge,PublicKey,Purpose},service::{self,Request}};
use serde_json::{Value,json};
use std::time::Duration;
const HELP:&str="des-connect-ctl state|discover|pair|accept|reject|unpair|ping [DEVICE_ID] [ATTEMPT]
Or pass a complete JSON request as the first argument. DES_CONNECT_SOCKET selects the private daemon socket.

The paired phone as a key (every part is off until switched on for one device):
  phonekey-status                         what is enrolled and enabled
  phonekey-presence                       whether the walk-away phone is here; exit 0 only if it is
  phonekey-enrol DEVICE_ID                ask the phone to create and enrol its signing key
  phonekey-set DEVICE_ID walkaway|unlock|approve on|off [SECONDS]
  phonekey-revoke DEVICE_ID               forget the phone's key
  phonekey-approve [--device ID] [--ttl SECONDS] ACTION TEXT...
                                          ask the phone to approve exactly this text; exit 0 only on a
                                          verified signature
  phonekey-unlock-check [--device ID]     run an unlock request against a locked session and print the
                                          decision; it never unlocks anything";
fn main(){if let Err(e)=run(){eprintln!("des-connect-ctl: {e}");std::process::exit(1);}}
fn run()->Result<()> {
 let mut args=std::env::args().skip(1);let op=args.next().unwrap_or_else(||"state".into());
 if op=="--help" {println!("{HELP}");return Ok(());}
 if op.starts_with("phonekey-") {return phonekey(&op,args.collect());}
 let req=if op.starts_with('{'){serde_json::from_str(&op)?}else{Request{op,device_id:args.next(),attempt:args.next().map(|v|v.parse()).transpose()?,args:Default::default()}};
 let result=service::request(&service::default_socket(),&req)?;println!("{}",serde_json::to_string_pretty(&result)?);
 if result.get("ok").and_then(serde_json::Value::as_bool)!=Some(true){return Err("request refused".into());}Ok(())
}
fn call(op:&str,device:Option<String>,args:Value,patience:Duration)->Result<Value> {
 let request=Request{op:op.into(),device_id:device,attempt:None,args:serde_json::from_value(args)?};
 let result=service::request_within(&service::default_socket(),&request,patience)?;
 if result.get("ok").and_then(Value::as_bool)!=Some(true) {
  return Err(result.get("error").and_then(Value::as_str).unwrap_or("request refused").to_owned().into());
 }Ok(result)
}
fn show(value:&Value)->Result<()> {println!("{}",serde_json::to_string_pretty(value)?);Ok(())}
fn phonekey(op:&str,mut args:Vec<String>)->Result<()> {
 let quick=Duration::from_secs(5);
 let mut option=|name:&str|->Result<Option<String>> {
  let Some(at)=args.iter().position(|arg|arg==name)else{return Ok(None)};
  if at+1>=args.len(){return Err(format!("{name} needs a value").into());}
  args.remove(at);Ok(Some(args.remove(at)))
 };
 match op {
  "phonekey-status"=>show(&call("phonekey_status",None,json!({}),quick)?["phonekey"]),
  "phonekey-presence"=>{
   let status=call("phonekey_status",None,json!({}),quick)?;
   let devices:Vec<&Value>=status["phonekey"]["devices"].as_array().map(|all|all.iter().filter(|device|device["walkaway"]==true).collect()).unwrap_or_default();
   let report:Vec<Value>=devices.iter().map(|d|json!({"id":d["id"],"name":d["name"],"connected":d["connected"],"present":d["present"],"last_seen_seconds":d["last_seen_seconds"],"armed":d["armed"],"walkaway_seconds":d["walkaway_seconds"]})).collect();
   show(&json!({"walkaway":!report.is_empty(),"devices":report}))?;
   // Presence is reported, never acted on here: it can only ever lead to a lock.
   if devices.iter().any(|device|device["present"]==true){Ok(())}else{Err("the walk-away phone is not present".into())}
  }
  "phonekey-enrol"=>{
   let device=args.first().cloned().ok_or("phonekey-enrol DEVICE_ID")?;
   eprintln!("Confirm on the phone within {} seconds.",phonekey::ENROL_TTL);
   let result=call("phonekey_enrol",Some(device),json!({}),Duration::from_secs(phonekey::ENROL_TTL+10))?;
   show(&json!({"decision":result["decision"],"key_id":result["key_id"],"fingerprint":result["fingerprint"]}))?;
   eprintln!("Check that the phone shows the same fingerprint. Unlock and approve stay off until phonekey-set turns them on.");Ok(())
  }
  "phonekey-set"=>{
   let [device,capability,word,rest@..]=args.as_slice()else{return Err("phonekey-set DEVICE_ID walkaway|unlock|approve on|off [SECONDS]".into())};
   let enabled=match word.as_str(){"on"=>true,"off"=>false,_=>return Err("say on or off".into())};
   let mut body=json!({"capability":capability,"enabled":enabled});
   if let Some(seconds)=rest.first(){body["seconds"]=json!(seconds.parse::<u64>()?);}
   show(&call("phonekey_set",Some(device.clone()),body,quick)?["phonekey"])
  }
  "phonekey-revoke"=>show(&call("phonekey_revoke",Some(args.first().cloned().ok_or("phonekey-revoke DEVICE_ID")?),json!({}),quick)?["phonekey"]),
  "phonekey-approve"|"phonekey-unlock-check"=>{
   let device=option("--device")?;let ttl=option("--ttl")?.map(|text|text.parse::<u64>()).transpose()?.unwrap_or(phonekey::REQUEST_TTL);
   let purpose=if op=="phonekey-approve"{Purpose::Approve}else{Purpose::Unlock};
   let action=if purpose==Purpose::Approve{args.join(" ")}else{"Unlock this DES UI session".to_owned()};
   if action.is_empty(){return Err("phonekey-approve needs the text to approve".into());}
   // The challenge is made here, by the one who will check the answer. The daemon
   // only carries it, so its word alone is never the decision.
   let user=std::env::var("USER").or_else(|_|std::env::var("LOGNAME")).map_err(|_|"USER is not set")?;
   let wall=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
   let challenge=Challenge::new(purpose,&user,&action,wall,ttl).map_err(|why|format!("this text cannot be sent for approval: {why}"))?;
   let mut body=challenge.body(None);body.as_object_mut().unwrap().remove("op");
   let result=call("phonekey_request",device,body,Duration::from_secs(ttl+10))?;
   let decision=result["decision"].as_str().unwrap_or("malformed");
   let verified=decision=="approved"&&verify(&challenge,&result).is_ok();
   show(&json!({"decision":decision,"verified":verified,"purpose":purpose.as_str(),"user":user,"action":action,"device_id":result["device_id"],"request_id":result["request_id"]}))?;
   if verified{Ok(())}else{Err(format!("not approved: {decision}").into())}
  }
  _=>Err(format!("unknown command {op}; see --help").into()),
 }
}
/// Check the signature against our own challenge. The key and certificates are the
/// daemon's report here: this command runs as the same user as the daemon. A
/// verifier that must not trust the daemon keeps its own copy of the key.
fn verify(challenge:&Challenge,result:&Value)->Result<()> {
 use base64::Engine;
 let text=|name:&str|result[name].as_str().ok_or_else(||format!("the answer has no {name}"));
 let digest=|name:&str|->Result<[u8;32]>{identity::unhex(text(name)?)?.try_into().map_err(|_|"bad certificate digest".into())};
 let key=PublicKey::from_base64(text("public_key")?)?;
 let binding=Binding{desktop_id:text("desktop_id")?.into(),desktop_cert:digest("desktop_cert")?,phone_id:text("device_id")?.into(),phone_cert:digest("phone_cert")?};
 let signature=base64::engine::general_purpose::STANDARD.decode(text("signature")?)?;
 let wall=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
 Ok(phonekey::verify_answer(challenge,&binding,&key,&signature,wall)?)
}
