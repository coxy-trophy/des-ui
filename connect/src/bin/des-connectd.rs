use des_connect::{identity::Result,service};
fn main(){if let Err(e)=run(){eprintln!("des-connectd: {e}");std::process::exit(1);}}
fn run()->Result<()> {
 let mut socket=service::default_socket();let mut state=service::default_state();let mut name="DES UI".to_owned();let mut loopback=false;let mut args=std::env::args().skip(1);
 while let Some(arg)=args.next(){match arg.as_str(){"--loopback"=>loopback=true,"--socket"=>socket=args.next().ok_or("--socket requires a path")?.into(),"--state-dir"=>state=args.next().ok_or("--state-dir requires a path")?.into(),"--name"=>name=args.next().ok_or("--name requires a name")?,"--help"=>{println!("des-connectd [--socket PATH] [--state-dir PATH] [--name NAME]\nOpt-in DES Connect LAN daemon. No screen capture is started by default.");return Ok(());},_=>return Err(format!("unknown option: {arg}").into())}}
 service::run_options(&socket,&state,&name,loopback)
}
