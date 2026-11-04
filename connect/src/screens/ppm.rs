use std::io::{self, BufRead};

pub const MAX_DIMENSION: usize = 640;
pub const MAX_FRAME_BYTES: usize = MAX_DIMENSION * MAX_DIMENSION * 3 + 64;

pub fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    // ffmpeg emits a three-line P6 header. Read bytewise with a hard cap:
    // read_until alone could allocate without bound on corrupt decoder output.
    let mut header = Vec::with_capacity(64);
    while header.iter().filter(|&&b| b == b'\n').count() < 3 {
        let mut byte = [0];
        if reader.read(&mut byte)? == 0 {
            if header.is_empty() { return Ok(None); }
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Partial PPM header"));
        }
        header.push(byte[0]);
        if header.len() > 64 { return Err(io::Error::new(io::ErrorKind::InvalidData, "PPM header too large")); }
    }
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "Invalid or oversized PPM frame");
    let text = std::str::from_utf8(&header).map_err(|_| invalid())?;
    let tokens: Vec<_> = text.split_ascii_whitespace().collect();
    if tokens.len() != 4 || tokens[0] != "P6" || tokens[3] != "255" { return Err(invalid()); }
    let width: usize = tokens[1].parse().map_err(|_| invalid())?;
    let height: usize = tokens[2].parse().map_err(|_| invalid())?;
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION { return Err(invalid()); }
    let offset = header.len();
    header.resize(offset + width * height * 3, 0);
    reader.read_exact(&mut header[offset..])?;
    debug_assert!(header.len() <= MAX_FRAME_BYTES);
    Ok(Some(header))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    #[test]
    fn reads_binary_pixels_and_back_to_back_frames() {
        let fixture = b"P6\n1 1\n255\n\n\0\xffP6\n1 1\n255\nabc";
        let mut input = BufReader::with_capacity(1, Cursor::new(fixture));
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"P6\n1 1\n255\n\n\0\xff");
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"P6\n1 1\n255\nabc");
        assert!(read_frame(&mut input).unwrap().is_none());
    }
    #[test]
    fn rejects_partial_invalid_and_unbounded_frames() {
        for fixture in [&b"P6\n1 1\n255\nab"[..], b"P6\n999999999 1\n255\n", b"P6\n0 1\n255\n", b"P6\n641 1\n255\n", b"P6\n1 1\n65535\n", b"P3\n1 1\n255\n", &vec![b'X'; 1000]] {
            assert!(read_frame(&mut Cursor::new(fixture)).is_err(), "{fixture:?}");
        }
    }
}
