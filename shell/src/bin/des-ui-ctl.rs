//! `des-ui-ctl`: asks the running shell for something, in its closed vocabulary, and
//! prints what it says back. Exit 0 when the shell says `ok`, 1 when it says `error`,
//! and 2 when there is no shell to ask.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const USAGE: &str = "\
Usage: des-ui-ctl <request...>

Asks the DEs-UI shell of this login session for one of these:

  ping                                   answers ok
  state                                  the displays and applications, as JSON
  lock                                   lock the screen
  workspace <0-3>                        bring that workspace into view
  toast <text>                           show a notification
  dock                                   open the dock for keyboard navigation
  launcher                               open the launcher
  settings [section]                     open the settings, at a section
  music toggle|next|prev                 control the music player
  panel left|right|bottom                open or collapse a panel
  screenshot [screen|window|region]      take a screenshot
  screenshot-file [screen|window|region] take one and answer with its file, as JSON,
                                         once it is saved; the desktop portal uses it
  pick-color                             the colour of a pixel you then click, as JSON

Prints the shell's answer: ok, ok <json>, or error <why>. Exits 0 on ok, 1 on
error, and 2 when the shell is not running. DES_UI_CONTROL_SOCKET names another
socket to ask.";

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if words.is_empty() || words.iter().any(|w| w == "--help" || w == "-h") {
        println!("{USAGE}");
        std::process::exit(if words.is_empty() { 2 } else { 0 });
    }
    let line = words.join(" ");
    if line.contains('\n') || line.len() > 4000 {
        eprintln!("des-ui-ctl: a request is one line of at most 4000 bytes");
        std::process::exit(1);
    }
    let path = des_common::control::socket_path();
    let stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("des-ui-ctl: the shell is not running ({}: {error})", path.display());
            std::process::exit(2);
        }
    };
    // What waits on a person choosing gets the time a person takes.
    let waits_on_user = matches!(words[0].as_str(), "screenshot-file" | "pick-color");
    let _ = stream.set_read_timeout(Some(Duration::from_secs(if waits_on_user { 130 } else { 15 })));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    if let Err(error) = (&stream).write_all(format!("{line}\n").as_bytes()) {
        eprintln!("des-ui-ctl: could not ask the shell: {error}");
        std::process::exit(2);
    }
    let mut answer = String::new();
    match BufReader::new(stream).read_line(&mut answer) {
        Ok(0) | Err(_) => {
            eprintln!("des-ui-ctl: the shell gave no answer");
            std::process::exit(2);
        }
        Ok(_) => {}
    }
    print!("{answer}");
    std::process::exit(if answer.starts_with("ok") { 0 } else { 1 });
}
