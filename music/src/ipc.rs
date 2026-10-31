//! Bounded transport for the music service. Slow clients cannot exhaust memory or tasks.

use crate::protocol::Request;
use std::io::{self, ErrorKind};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;
use tokio::time::timeout;

const MAX_CLIENTS: usize = 16;

pub struct Clients(std::sync::Arc<tokio::sync::Semaphore>);

impl Clients {
    pub fn new() -> Self { Self(std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CLIENTS))) }

    pub fn admit(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.0.clone().try_acquire_owned().ok()
    }
}
// Cover uploads carry up to 256 KiB of base64 plus JSON framing.
const MAX_REQUEST_BYTES: usize = 272 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

async fn read_request(reader: &mut (impl AsyncBufRead + Unpin)) -> io::Result<Option<Request>> {
    let mut bytes = Vec::new();
    let size = reader.take((MAX_REQUEST_BYTES + 1) as u64).read_until(b'\n', &mut bytes).await?;
    if size == 0 { return Ok(None); }
    if size > MAX_REQUEST_BYTES || bytes.last() != Some(&b'\n') {
        return Err(ErrorKind::InvalidData.into());
    }
    let line = std::str::from_utf8(&bytes).map_err(|_| io::Error::from(ErrorKind::InvalidData))?;
    Request::parse(line).map(Some).ok_or_else(|| ErrorKind::InvalidData.into())
}

async fn write_reply(writer: &mut (impl AsyncWrite + Unpin), reply: &str, deadline: Duration) -> io::Result<()> {
    timeout(deadline, writer.write_all(reply.as_bytes())).await
        .map_err(|_| io::Error::from(ErrorKind::TimedOut))?
}

struct Limits {
    read: Duration,
    write: Duration,
    operation: Duration,
}

pub async fn serve_client<R, W, H, F>(reader: R, writer: W, initial: String, updates: broadcast::Receiver<String>, handle: H)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    H: FnMut(Request) -> F,
    F: std::future::Future<Output = Option<String>>,
{
    serve_with_limits(reader, writer, initial, updates, handle,
        Limits { read: READ_TIMEOUT, write: WRITE_TIMEOUT, operation: OPERATION_TIMEOUT }).await;
}

async fn serve_with_limits<R, W, H, F>(reader: R, mut writer: W, initial: String, mut updates: broadcast::Receiver<String>, mut handle: H, limits: Limits)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    H: FnMut(Request) -> F,
    F: std::future::Future<Output = Option<String>>,
{
    if write_reply(&mut writer, &initial, limits.write).await.is_err() { return; }
    let mut reader = BufReader::new(reader);
    loop {
        // Keep this future alive across broadcasts: partial input and its deadline
        // must not reset every time the current track changes.
        let request = timeout(limits.read, read_request(&mut reader));
        tokio::pin!(request);
        let request = loop {
            tokio::select! {
                result = &mut request => match result {
                    Ok(Ok(Some(request))) => break request,
                    _ => return,
                },
                update = updates.recv() => match update {
                    Ok(line) => if write_reply(&mut writer, &line, limits.write).await.is_err() { return; },
                    Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        };
        match timeout(limits.operation, handle(request)).await {
            Ok(Some(reply)) => if write_reply(&mut writer, &reply, limits.write).await.is_err() { return; },
            Ok(None) => {},
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, BufReader};

    #[test]
    fn excess_clients_are_rejected_and_disconnect_releases_a_slot() {
        let clients = Clients::new();
        let mut active = Vec::new();
        for _ in 0..16 { active.push(clients.admit().expect("available client slot")); }
        assert!(clients.admit().is_none());
        active.pop();
        assert!(clients.admit().is_some());
    }

    #[tokio::test]
    async fn rejects_a_request_before_allocating_an_unbounded_line() {
        let bytes = vec![b'x'; MAX_REQUEST_BYTES + 1];
        let mut reader = BufReader::new(bytes.as_slice());
        let error = read_request(&mut reader).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn reads_fragmented_requests_without_consuming_the_next_one() {
        let (mut writer, reader) = tokio::io::duplex(64);
        tokio::spawn(async move {
            writer.write_all(b"{\"cmd\":\"sta").await.unwrap();
            tokio::task::yield_now().await;
            writer.write_all(b"te\"}\n{\"cmd\":\"next\"}\n").await.unwrap();
        });
        let mut reader = BufReader::new(reader);
        assert!(matches!(read_request(&mut reader).await.unwrap(), Some(crate::protocol::Request::State)));
        assert!(matches!(read_request(&mut reader).await.unwrap(), Some(crate::protocol::Request::Next)));
        assert!(read_request(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_malformed_and_unterminated_requests() {
        for bytes in [b"not json\n".as_slice(), b"{\"cmd\":\"state\"}"] {
            let mut reader = BufReader::new(bytes);
            assert_eq!(read_request(&mut reader).await.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        }
    }

    #[tokio::test]
    async fn slow_readers_cannot_hold_a_writer_indefinitely() {
        let (_reader, mut writer) = tokio::io::duplex(8);
        let result = write_reply(&mut writer, "a response larger than the pipe", std::time::Duration::from_millis(20)).await;
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }
    #[tokio::test]
    async fn state_updates_do_not_discard_a_partial_request() {
        let (client, server) = tokio::io::duplex(256);
        let (reader, writer) = tokio::io::split(server);
        let (updates, receiver) = broadcast::channel(8);
        let task = tokio::spawn(serve_client(reader, writer, "initial\n".into(), receiver,
            |_| async { Some("handled\n".into()) }));
        let (client_reader, mut client_writer) = tokio::io::split(client);
        let mut client_reader = BufReader::new(client_reader);
        let mut line = String::new();
        client_reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "initial\n");
        client_writer.write_all(b"{\"cmd\":\"sta").await.unwrap();
        updates.send("broadcast\n".into()).unwrap();
        line.clear();
        client_reader.read_line(&mut line).await.unwrap();
        assert_eq!(line, "broadcast\n");
        client_writer.write_all(b"te\"}\n").await.unwrap();
        line.clear();
        timeout(Duration::from_secs(1), client_reader.read_line(&mut line)).await.unwrap().unwrap();
        assert_eq!(line, "handled\n");
        client_writer.shutdown().await.unwrap();
        timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn partial_and_stalled_operations_release_the_connection() {
        for operation in [false, true] {
            let (mut client, server) = tokio::io::duplex(256);
            let (reader, writer) = tokio::io::split(server);
            let (_updates, receiver) = broadcast::channel(8);
            let limits = Limits { read: Duration::from_millis(20), write: Duration::from_millis(20), operation: Duration::from_millis(20) };
            let task = tokio::spawn(serve_with_limits(reader, writer, String::new(), receiver,
                |_| std::future::pending::<Option<String>>(), limits));
            client.write_all(if operation { b"{\"cmd\":\"state\"}\n" } else { b"{" }).await.unwrap();
            timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
        }
    }

}
