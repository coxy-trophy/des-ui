//! Bounded, resumable writes for the desktop's nonblocking sockets.
use std::io::{self, Write};

pub struct WriteBuffer {
    bytes: Vec<u8>,
    limit: usize,
    offset: usize,
}

impl WriteBuffer {
    pub fn new(limit: usize) -> Self {
        Self { bytes: Vec::new(), limit, offset: 0 }
    }

    pub fn enqueue(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.bytes.len() - self.offset + bytes.len() > self.limit {
            return Err(io::Error::new(io::ErrorKind::OutOfMemory, "desktop write queue is full"));
        }
        if self.offset != 0 {
            self.bytes.drain(..self.offset);
            self.offset = 0;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
        self.offset = 0;
    }

    /// Returns whether all queued bytes have been written.
    pub fn flush(&mut self, writer: &mut impl Write) -> io::Result<bool> {
        let mut written = 0;
        while self.offset < self.bytes.len() && written < 64 * 1024 {
            let end = self.bytes.len().min(self.offset + 64 * 1024 - written);
            match writer.write(&self.bytes[self.offset..end]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => { self.offset += n; written += n; }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        if self.offset == self.bytes.len() {
            self.clear();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SlowWriter { available: usize, bytes: Vec<u8> }
    impl Write for SlowWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.available == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let n = bytes.len().min(self.available);
            self.available -= n;
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn partial_writes_resume_without_losing_or_repeating_bytes() {
        let mut pending = WriteBuffer::new(128);
        pending.enqueue(b"first\n").unwrap();
        pending.enqueue(b"second\n").unwrap();
        let mut writer = SlowWriter { available: 3, bytes: Vec::new() };
        assert!(!pending.flush(&mut writer).unwrap());
        pending.enqueue(b"third\n").unwrap();
        writer.available = 100;
        assert!(pending.flush(&mut writer).unwrap());
        assert_eq!(writer.bytes, b"first\nsecond\nthird\n");
    }

    #[test]
    fn queue_overflow_keeps_existing_bytes_and_rejects_new_message() {
        let mut pending = WriteBuffer::new(6);
        pending.enqueue(b"first\n").unwrap();
        assert!(pending.enqueue(b"extra\n").is_err());
        let mut bytes = Vec::new();
        assert!(pending.flush(&mut bytes).unwrap());
        assert_eq!(bytes, b"first\n");
    }

    #[test]
    fn writing_has_a_per_poll_budget() {
        let mut pending = WriteBuffer::new(1024 * 1024);
        pending.enqueue(&vec![b'x'; 256 * 1024]).unwrap();
        let mut bytes = Vec::new();
        assert!(!pending.flush(&mut bytes).unwrap());
        assert!(bytes.len() <= 64 * 1024);
        while !pending.flush(&mut bytes).unwrap() {}
        assert_eq!(bytes.len(), 256 * 1024);
    }
}
