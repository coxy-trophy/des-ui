//! The DEs-UI desktop portal backend. `xdg-desktop-portal` starts it over D-Bus when
//! an application asks for a screenshot or a colour, and it asks the shell in turn,
//! over the shell's control socket, so that the desktop's own chooser does the work
//! and the picture goes where screenshots always go.
//!
//! Only the screenshot portal is served here: `org.freedesktop.impl.portal.Screenshot`
//! (`Screenshot` and `PickColor`). Everything else keeps the GTK backend, as
//! `des-ui-portals.conf` says.

mod protocol;

use async_io::{Async, Timer};
use futures_lite::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, future, io::BufReader};
use protocol::{Answer, Outcome};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, interface};

const NAME: &str = "org.freedesktop.impl.portal.desktop.des-ui";
const PATH: &str = "/org/freedesktop/portal/desktop";
/// How long the shell may take to accept a connection and start answering.
const CONNECT: Duration = Duration::from_secs(5);
const MAX_ANSWER: usize = 16 * 1024;

/// The request object the portal may close while the user is still choosing.
struct Request {
    closed: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

struct Screenshot;

#[interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }

    async fn screenshot(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
    ) -> Outcome {
        let line = protocol::screenshot_request(&options);
        serve_request(server, handle, &line, protocol::screenshot_outcome).await
    }

    async fn pick_color(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        _app_id: String,
        _parent_window: String,
        _options: HashMap<String, OwnedValue>,
    ) -> Outcome {
        serve_request(server, handle, protocol::pick_color_request(), protocol::color_outcome).await
    }
}

/// Exports the request object for as long as the shell is asked, then turns the
/// shell's answer into the portal's. A request closed meanwhile is cancelled,
/// whatever the shell went on to say.
async fn serve_request(server: &ObjectServer, handle: OwnedObjectPath, line: &str, outcome: fn(Answer) -> Outcome) -> Outcome {
    let closed = Arc::new(AtomicBool::new(false));
    let exported = server.at(&handle, Request { closed: closed.clone() }).await.unwrap_or(false);
    if !exported { return (protocol::FAILED, HashMap::new()); }
    let answer = match future::or(ask_shell(line), wait_closed(&closed)).await {
        Ok(answer) => answer,
        Err(why) => Answer::Error(why),
    };
    if exported {
        let _ = server.remove::<Request, _>(&handle).await;
    }
    if closed.load(Ordering::SeqCst) {
        return (protocol::CANCELLED, HashMap::new());
    }
    outcome(answer)
}

async fn wait_closed(closed: &AtomicBool) -> Result<Answer, String> {
    while !closed.load(Ordering::SeqCst) {
        Timer::after(Duration::from_millis(10)).await;
    }
    Ok(Answer::Error("cancelled".into()))
}

/// One line to the shell's control socket, one line back, within the time a person
/// may take to choose.
async fn ask_shell(line: &str) -> Result<Answer, String> {
    let path = des_common::control::socket_path();
    let connect = async {
        Async::<UnixStream>::connect(path.clone()).await.map_err(|error| format!("the shell is not listening at {}: {error}", path.display()))
    };
    let stream = future::or(connect, async {
        Timer::after(CONNECT).await;
        Err("the shell did not accept the connection".to_string())
    })
    .await?;
    let exchange = async {
        let mut writer = &stream;
        writer.write_all(format!("{line}\n").as_bytes()).await.map_err(|error| format!("could not ask the shell: {error}"))?;
        read_answer(&stream).await
    };
    future::or(exchange, async {
        Timer::after(protocol::INTERACTION).await;
        Err("the shell did not answer in time".to_string())
    })
    .await
}

/// Cap bytes before allocation; an unterminated answer is not a complete reply.
async fn read_answer(reader: impl futures_lite::AsyncRead + Unpin) -> Result<Answer, String> {
    let mut bytes = Vec::new();
    BufReader::new(reader.take((MAX_ANSWER + 1) as u64)).read_until(b'\n', &mut bytes).await
        .map_err(|error| format!("the shell gave no answer: {error}"))?;
    if bytes.len() > MAX_ANSWER { return Err("the shell's answer was too long".into()); }
    if bytes.last() != Some(&b'\n') { return Err("the shell closed before a complete answer".into()); }
    let line = std::str::from_utf8(&bytes).map_err(|_| "the shell's answer was not UTF-8".to_string())?;
    Ok(protocol::parse_answer(line))
}

fn main() {
    if let Err(error) = async_io::block_on(serve()) {
        eprintln!("des-portal: {error}");
        std::process::exit(1);
    }
}

async fn serve() -> zbus::Result<()> {
    let connection = Connection::session().await?;
    connection.object_server().at(PATH, Screenshot).await?;
    connection.request_name(NAME).await?;
    // Nothing more to do here: the object server answers on the connection's own task.
    future::pending::<()>().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_interrupts_a_shell_that_never_answers() {
        async_io::block_on(async {
            let closed = Arc::new(AtomicBool::new(false));
            let began = std::time::Instant::now();
            let close = async {
                Timer::after(Duration::from_millis(15)).await;
                Request { closed: closed.clone() }.close();
                future::pending::<Result<Answer, String>>().await
            };
            assert!(matches!(future::or(close, wait_closed(&closed)).await.unwrap(), Answer::Error(ref why) if why == "cancelled"));
            assert!(began.elapsed() < Duration::from_secs(1));
        });
    }

    #[test]
    fn shell_answers_are_bounded_and_require_a_complete_line() {
        async_io::block_on(async {
            assert!(read_answer(futures_lite::io::Cursor::new(vec![b'x'; MAX_ANSWER + 1])).await.is_err());
            assert!(read_answer(futures_lite::io::Cursor::new(b"ok".to_vec())).await.is_err());
            assert!(matches!(read_answer(futures_lite::io::Cursor::new(b"error cancelled\n".to_vec())).await.unwrap(), Answer::Error(ref why) if why == "cancelled"));
        });
    }

    /// The interface declarations hold together: zbus can describe them.
    #[test]
    fn the_interfaces_are_well_formed() {
        let screenshot = <Screenshot as zbus::object_server::Interface>::name();
        assert_eq!(screenshot.as_str(), "org.freedesktop.impl.portal.Screenshot");
        let request = <Request as zbus::object_server::Interface>::name();
        assert_eq!(request.as_str(), "org.freedesktop.impl.portal.Request");
        let introspection = <Screenshot as zbus::object_server::Interface>::introspect_to_writer;
        let _ = introspection;
    }
}
