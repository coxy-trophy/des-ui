use des_connect::{identity::Result,service::{self,Request}};
fn main(){if let Err(e)=run(){eprintln!("des-connect-ctl: {e}");std::process::exit(1);}}
fn run()->Result<()> {
 let mut args=std::env::args().skip(1);let op=args.next().unwrap_or_else(||"state".into());
 if op=="--help" {println!("des-connect-ctl state|discover|pair|accept|reject|unpair|ping [DEVICE_ID] [ATTEMPT]\nOr pass a complete JSON request as the first argument. DES_CONNECT_SOCKET selects the private daemon socket.");return Ok(());}
 let req=if op.starts_with('{'){serde_json::from_str(&op)?}else{Request{op,device_id:args.next(),attempt:args.next().map(|v|v.parse()).transpose()?,args:Default::default()}};
 let result=service::request(&service::default_socket(),&req)?;println!("{}",serde_json::to_string_pretty(&result)?);
 if result.get("ok").and_then(serde_json::Value::as_bool)!=Some(true){return Err("request refused".into());}Ok(())
}
