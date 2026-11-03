//! Independently implemented packet framing for KDE Connect interoperability.
//! The frame limit includes its newline. Discovery identity is never proof of trust.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, BufRead, Read};

pub const MAX_FRAME: usize = 64 * 1024;
fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Packet {
    #[serde(default)]
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub body: Map<String, Value>,
    #[serde(rename = "payloadSize", skip_serializing_if = "Option::is_none")]
    pub payload_size: Option<i64>,
    #[serde(rename = "payloadTransferInfo", skip_serializing_if = "Option::is_none")]
    pub payload_transfer_info: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Packet {
    fn validate(&self) -> io::Result<()> {
        if !token(&self.kind) { return Err(invalid("invalid packet type")); }
        if self.payload_size.is_some_and(|n| n < -1)
            || self.payload_size.is_some() != self.payload_transfer_info.is_some() {
            return Err(invalid("inconsistent payload metadata"));
        }
        Ok(())
    }
    /// Every framing/validation error is connection-fatal. The caller must close
    /// the stream; a bounded read may leave an arbitrary suffix, not a new frame.
    pub fn read(reader: &mut impl BufRead) -> io::Result<Self> {
        let mut bytes = Vec::new();
        (&mut *reader).take((MAX_FRAME + 1) as u64).read_until(b'\n', &mut bytes)?;
        if bytes.len() > MAX_FRAME { return Err(invalid("packet exceeds 64KiB")); }
        if bytes.last() != Some(&b'\n') { return Err(invalid("incomplete packet")); }
        let packet: Self = serde_json::from_slice(&bytes).map_err(invalid)?;
        packet.validate()?;
        Ok(packet)
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        // Limit while serializing as well: callers must not allocate an unbounded frame.
        struct Bounded(Vec<u8>);
        impl io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if bytes.len() > (MAX_FRAME - 1).saturating_sub(self.0.len()) {
                    return Err(invalid("packet exceeds 64KiB"));
                }
                self.0.extend_from_slice(bytes); Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut out = Bounded(Vec::new());
        serde_json::to_writer(&mut out, self).map_err(invalid)?;
        out.0.push(b'\n');
        Ok(out.0)
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub device_id: String,
    #[serde(rename = "deviceName")]
    pub name: String,
    pub device_type: String,
    pub protocol_version: u32,
    pub tcp_port: Option<u16>,
    pub incoming_capabilities: Vec<String>,
    pub outgoing_capabilities: Vec<String>,
}
impl Identity {
    pub fn from_packet(packet: &Packet) -> io::Result<Self> {
        if packet.kind != "kdeconnect.identity" || packet.payload_size.is_some() {
            return Err(invalid("not an identity packet"));
        }
        let identity: Self = serde_json::from_value(Value::Object(packet.body.clone())).map_err(invalid)?;
        if !(32..=38).contains(&identity.device_id.len())
            || !identity.device_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || identity.name.trim().is_empty() || identity.name.chars().count() > 256
            || identity.name.chars().any(char::is_control)
            || !["phone", "tablet", "desktop", "laptop", "tv"].contains(&identity.device_type.as_str())
            || identity.protocol_version != 8 || identity.tcp_port == Some(0)
            || [&identity.incoming_capabilities, &identity.outgoing_capabilities].iter()
                .any(|caps| caps.len() > 256 || caps.iter().any(|cap| !token(cap))) {
            return Err(invalid("invalid or unsupported identity"));
        }
        Ok(identity)
    }
}
