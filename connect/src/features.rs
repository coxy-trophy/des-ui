//! Independently implemented, bounded peer feature packets. No OS or network actions.
//!
//! Caller must authenticate the current paired transport before decoding, and require
//! explicit local consent before sending any request. Capability is not consent.
use crate::packet::Packet;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;

pub const MAX_TEXT: usize = 16 * 1024;
pub const MAX_ID: usize = 512;
pub const MAX_MESSAGES: usize = 128;
pub const MAX_CONTACTS: usize = 512;
pub const MAX_ATTACHMENTS: usize = 16;
pub const MAX_CACHE_EVENTS: usize = 128;
pub const MAX_CACHE_BYTES: usize = 256 * 1024;
const MAX_CARD: usize = 48 * 1024;
const MAX_ADDRESSES: usize = 32;

pub const INCOMING_TYPES: &[&str] = &[
    "kdeconnect.share.request", "kdeconnect.clipboard", "kdeconnect.clipboard.connect",
    "kdeconnect.notification", "kdeconnect.telephony", "kdeconnect.sms.messages",
    "kdeconnect.sms.attachment_file", "kdeconnect.contacts.response_uids_timestamps",
    "kdeconnect.contacts.response_vcards",
];
pub const OUTGOING_TYPES: &[&str] = &[
    "kdeconnect.share.request", "kdeconnect.clipboard", "kdeconnect.findmyphone.request",
    "kdeconnect.notification.request", "kdeconnect.notification.reply", "kdeconnect.notification.action",
    "kdeconnect.telephony.request_mute", "kdeconnect.sms.request", "kdeconnect.sms.request_conversations",
    "kdeconnect.sms.request_conversation", "kdeconnect.sms.request_attachment",
    "kdeconnect.contacts.request_all_uids_timestamps", "kdeconnect.contacts.request_vcards_by_uid",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureError {
    Unsupported,
    Disabled,
    Malformed(&'static str),
    Limit(&'static str),
    PayloadHandoff(&'static str),
}
impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("Unsupported feature packet"),
            Self::Disabled => f.write_str("This feature is disabled"),
            Self::Malformed(field) => write!(f, "Invalid feature field: {field}"),
            Self::Limit(field) => write!(f, "Feature limit exceeded: {field}"),
            Self::PayloadHandoff(kind) => write!(f, "A payload transfer is required for {kind}"),
        }
    }
}
impl std::error::Error for FeatureError {}
type Result<T> = std::result::Result<T, FeatureError>;

/// Explicit runtime opt-ins. Default never subscribes to or retains private data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureFlags {
    pub urls: bool,
    pub clipboard: bool,
    pub notifications: bool,
    pub calls: bool,
    pub sms: bool,
    pub contacts: bool,
}

/// Interoperability hints from a peer's advertised identity, not permissions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PeerCapabilities {
    pub send_url: bool,
    pub receive_url: bool,
    pub send_clipboard: bool,
    pub receive_clipboard: bool,
    pub find_phone: bool,
    pub notifications: bool,
    pub notification_reply: bool,
    pub call_alerts: bool,
    pub mute_ringer: bool,
    pub sms_read: bool,
    pub sms_send: bool,
    pub contacts: bool,
}
impl PeerCapabilities {
    pub fn from_peer(incoming: &[String], outgoing: &[String]) -> Self {
        let can_send = |kind: &str| incoming.iter().any(|value| value == kind);
        let can_receive = |kind: &str| outgoing.iter().any(|value| value == kind);
        Self {
            send_url: can_send("kdeconnect.share.request"), receive_url: can_receive("kdeconnect.share.request"),
            send_clipboard: can_send("kdeconnect.clipboard"),
            receive_clipboard: can_receive("kdeconnect.clipboard") || can_receive("kdeconnect.clipboard.connect"),
            find_phone: can_send("kdeconnect.findmyphone.request"),
            notifications: can_receive("kdeconnect.notification"),
            notification_reply: can_send("kdeconnect.notification.reply"),
            call_alerts: can_receive("kdeconnect.telephony"), mute_ringer: can_send("kdeconnect.telephony.request_mute"),
            sms_read: can_receive("kdeconnect.sms.messages") && can_send("kdeconnect.sms.request_conversation"),
            sms_send: can_send("kdeconnect.sms.request"),
            contacts: can_receive("kdeconnect.contacts.response_uids_timestamps")
                && can_receive("kdeconnect.contacts.response_vcards")
                && can_send("kdeconnect.contacts.request_all_uids_timestamps")
                && can_send("kdeconnect.contacts.request_vcards_by_uid"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PayloadDescriptor {
    pub size: u64,
    pub port: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NotificationLine { pub sender: String, pub content: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub id: String,
    pub app_name: String,
    pub title: String,
    pub text: String,
    pub ticker: String,
    pub time: Option<i64>,
    pub clearable: bool,
    pub silent: bool,
    pub reply_id: Option<String>,
    pub actions: Vec<String>,
    pub conversation: Vec<NotificationLine>,
    pub icon_hash: Option<String>,
    pub icon: Option<PayloadDescriptor>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum CallKind { Ringing, Talking, MissedCall }
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CallAlert {
    pub kind: CallKind,
    pub cancelled: bool,
    pub phone_number: Option<String>,
    pub contact_name: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SmsAttachment {
    pub part_id: i64,
    pub mime_type: String,
    pub unique_identifier: String,
    /// Opaque bounded base64 data; never decoded or rendered automatically.
    pub encoded_thumbnail: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SmsMessage {
    pub id: i64,
    pub thread_id: i64,
    pub addresses: Vec<String>,
    pub body: String,
    pub date: i64,
    pub message_type: i32,
    pub read: bool,
    pub event: u32,
    pub subscription_id: Option<i32>,
    pub attachments: Vec<SmsAttachment>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContactTimestamp { pub uid: String, pub timestamp: i64 }
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContactCard {
    pub uid: String,
    /// Opaque vCard source; parsing/importing/photos require a separate user action.
    pub vcard: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Event {
    UrlOffer { url: String },
    SharedText { text: String },
    ClipboardOffer { content: Option<String>, timestamp: Option<i64> },
    Notification(Notification),
    NotificationCancelled { id: String },
    Call(CallAlert),
    SmsMessages(Vec<SmsMessage>),
    SmsAttachment { filename: String, payload: PayloadDescriptor },
    ContactIndex(Vec<ContactTimestamp>),
    ContactCards(Vec<ContactCard>),
}
impl Event {
    fn enabled(&self, flags: FeatureFlags) -> bool {
        match self {
            Self::UrlOffer { .. } | Self::SharedText { .. } => flags.urls,
            Self::ClipboardOffer { .. } => flags.clipboard,
            Self::Notification(_) | Self::NotificationCancelled { .. } => flags.notifications,
            Self::Call(_) => flags.calls,
            Self::SmsMessages(_) | Self::SmsAttachment { .. } => flags.sms,
            Self::ContactIndex(_) | Self::ContactCards(_) => flags.contacts,
        }
    }
}

/// A small per-authenticated-peer, process-only event cache. Never writes files.
#[derive(Default)]
pub struct PeerState {
    flags: FeatureFlags,
    events: VecDeque<(Event, usize)>,
    bytes: usize,
}
impl PeerState {
    pub fn set_enabled(&mut self, flags: FeatureFlags) {
        self.flags = flags;
        self.events.retain(|(event, _)| event.enabled(flags));
        self.bytes = self.events.iter().map(|(_, bytes)| bytes).sum();
    }
    pub fn flags(&self) -> FeatureFlags { self.flags }
    pub fn events(&self) -> impl DoubleEndedIterator<Item = &Event> { self.events.iter().map(|(event, _)| event) }
    /// Clear on disconnect/unpair, privacy changes or explicit Forget.
    pub fn clear(&mut self) { self.events.clear(); self.bytes = 0; }
    pub fn receive(&mut self, packet: &Packet) -> Result<Event> {
        let event = decode(packet)?;
        if !event.enabled(self.flags) { return Err(FeatureError::Disabled); }
        // Replacements/cancellations invalidate stale notification replies/actions.
        match &event {
            Event::Notification(next) => self.events.retain(|(old, _)| !matches!(old, Event::Notification(old) if old.id == next.id)),
            Event::NotificationCancelled { id } => self.events.retain(|(old, _)| !matches!(old, Event::Notification(old) if &old.id == id)),
            Event::ClipboardOffer { .. } => self.events.retain(|(old, _)| !matches!(old, Event::ClipboardOffer { .. })),
            _ => {}
        }
        self.bytes = self.events.iter().map(|(_, bytes)| bytes).sum();
        let bytes = serde_json::to_vec(&event).map_err(|_| FeatureError::Malformed("event"))?.len();
        if bytes > MAX_CACHE_BYTES { return Err(FeatureError::Limit("cache event")); }
        while self.events.len() >= MAX_CACHE_EVENTS || self.bytes + bytes > MAX_CACHE_BYTES {
            if let Some((_, old_bytes)) = self.events.pop_front() { self.bytes -= old_bytes; } else { break; }
        }
        self.bytes += bytes;
        self.events.push_back((event.clone(), bytes));
        Ok(event)
    }
}

fn text(value: &str, max: usize, field: &'static str) -> Result<String> {
    if value.len() > max { return Err(FeatureError::Limit(field)); }
    if value.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) { return Err(FeatureError::Malformed(field)); }
    Ok(value.to_owned())
}
fn identifier(value: &str, field: &'static str) -> Result<String> {
    if value.is_empty() || value.chars().any(char::is_control) { return Err(FeatureError::Malformed(field)); }
    text(value, MAX_ID, field)
}
fn required_text(map: &Map<String, Value>, field: &'static str, max: usize) -> Result<String> {
    text(map.get(field).and_then(Value::as_str).ok_or(FeatureError::Malformed(field))?, max, field)
}
fn optional_text(map: &Map<String, Value>, field: &'static str, max: usize) -> Result<Option<String>> {
    match map.get(field) { None | Some(Value::Null) => Ok(None), Some(Value::String(s)) => text(s, max, field).map(Some), _ => Err(FeatureError::Malformed(field)) }
}
fn required_id(map: &Map<String, Value>, field: &'static str) -> Result<String> { identifier(&required_text(map, field, MAX_ID)?, field) }
fn optional_id(map: &Map<String, Value>, field: &'static str) -> Result<Option<String>> {
    optional_text(map, field, MAX_ID)?.map(|s| identifier(&s, field)).transpose()
}
fn integer(value: &Value, field: &'static str) -> Result<i64> {
    value.as_i64().or_else(|| value.as_str().filter(|s| s.len() <= 24).and_then(|s| s.parse().ok())).ok_or(FeatureError::Malformed(field))
}
fn required_int(map: &Map<String, Value>, field: &'static str) -> Result<i64> { integer(map.get(field).ok_or(FeatureError::Malformed(field))?, field) }
fn optional_int(map: &Map<String, Value>, field: &'static str) -> Result<Option<i64>> {
    match map.get(field) { None | Some(Value::Null) => Ok(None), Some(v) => integer(v, field).map(Some) }
}
fn boolean(map: &Map<String, Value>, field: &'static str, default: bool) -> Result<bool> {
    match map.get(field) {
        None | Some(Value::Null) => Ok(default), Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) if s == "true" => Ok(true), Some(Value::String(s)) if s == "false" => Ok(false),
        Some(v) if v.as_i64() == Some(0) => Ok(false), Some(v) if v.as_i64() == Some(1) => Ok(true),
        _ => Err(FeatureError::Malformed(field)),
    }
}
fn array<'a>(map: &'a Map<String, Value>, field: &'static str, max: usize, required: bool) -> Result<&'a [Value]> {
    let values = match map.get(field) {
        None | Some(Value::Null) if !required => return Ok(&[]),
        Some(Value::Array(values)) => values,
        _ => return Err(FeatureError::Malformed(field)),
    };
    if values.len() > max { return Err(FeatureError::Limit(field)); }
    Ok(values)
}
fn object<'a>(value: &'a Value, field: &'static str) -> Result<&'a Map<String, Value>> { value.as_object().ok_or(FeatureError::Malformed(field)) }
fn nonnegative(value: i64, field: &'static str) -> Result<i64> { if value < 0 { Err(FeatureError::Malformed(field)) } else { Ok(value) } }
fn no_payload(packet: &Packet) -> Result<()> {
    if packet.payload_size.is_some() { Err(FeatureError::PayloadHandoff("this feature packet")) } else { Ok(()) }
}
fn payload(packet: &Packet, max: u64) -> Result<PayloadDescriptor> {
    let size = packet.payload_size.filter(|n| *n >= 0).ok_or(FeatureError::Malformed("payloadSize"))? as u64;
    if size > max { return Err(FeatureError::Limit("payloadSize")); }
    let port = packet.payload_transfer_info.as_ref().and_then(|info| info.get("port")).and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0).ok_or(FeatureError::Malformed("payload port"))?;
    Ok(PayloadDescriptor { size, port })
}

/// Conservative HTTP(S) URL validation without browser or shell interpretation.
/// ASCII DNS/IP hosts only; credentials, whitespace, backslashes and bad escapes fail closed.
pub fn validate_url(url: &str) -> Result<()> {
    if url.len() > 8192 { return Err(FeatureError::Limit("url")); }
    if url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') { return Err(FeatureError::Malformed("url")); }
    let (scheme, rest) = url.split_once("://").ok_or(FeatureError::Malformed("url"))?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") { return Err(FeatureError::Malformed("url scheme")); }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || !authority.is_ascii() { return Err(FeatureError::Malformed("url host")); }
    let port = if authority.starts_with('[') {
        let end = authority.find(']').ok_or(FeatureError::Malformed("url host"))?;
        authority[1..end].parse::<std::net::Ipv6Addr>().map_err(|_| FeatureError::Malformed("url host"))?;
        let tail = &authority[end + 1..];
        if tail.is_empty() { None } else { Some(tail.strip_prefix(':').ok_or(FeatureError::Malformed("url port"))?) }
    } else {
        let (host, port) = authority.split_once(':').map_or((authority, None), |(host, port)| (host, Some(port)));
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.len() > 253 || host.split('.').any(|label| label.is_empty() || label.len() > 63
            || label.starts_with('-') || label.ends_with('-') || !label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')) {
            return Err(FeatureError::Malformed("url host"));
        }
        port
    };
    if let Some(port) = port {
        if port.is_empty() || !port.bytes().all(|c| c.is_ascii_digit()) || port.parse::<u16>().ok().filter(|n| *n > 0).is_none() { return Err(FeatureError::Malformed("url port")); }
    }
    let bytes = url.as_bytes();
    for at in 0..bytes.len() {
        if bytes[at] == b'%' && (at + 2 >= bytes.len() || !bytes[at + 1..=at + 2].iter().all(u8::is_ascii_hexdigit)) {
            return Err(FeatureError::Malformed("url escape"));
        }
    }
    Ok(())
}

fn notification(packet: &Packet) -> Result<Event> {
    let b = &packet.body;
    let id = required_id(b, "id")?;
    if boolean(b, "isCancel", false)? { no_payload(packet)?; return Ok(Event::NotificationCancelled { id }); }
    let actions = array(b, "actions", 16, false)?.iter().map(|v| {
        text(v.as_str().ok_or(FeatureError::Malformed("actions"))?, 256, "actions")
    }).collect::<Result<Vec<_>>>()?;
    let conversation = array(b, "conversation", 32, false)?.iter().map(|v| {
        let line = object(v, "conversation")?;
        Ok(NotificationLine { sender: required_text(line, "sender", 256)?, content: required_text(line, "content", MAX_TEXT)? })
    }).collect::<Result<Vec<_>>>()?;
    Ok(Event::Notification(Notification {
        id, app_name: optional_text(b, "appName", 256)?.unwrap_or_default(), title: optional_text(b, "title", 1024)?.unwrap_or_default(),
        text: optional_text(b, "text", MAX_TEXT)?.unwrap_or_default(), ticker: optional_text(b, "ticker", MAX_TEXT)?.unwrap_or_default(),
        time: optional_int(b, "time")?.map(|v| nonnegative(v, "time")).transpose()?, clearable: boolean(b, "isClearable", false)?,
        silent: boolean(b, "silent", false)?, reply_id: optional_id(b, "requestReplyId")?, actions, conversation,
        icon_hash: optional_id(b, "payloadHash")?, icon: if packet.payload_size.is_some() { Some(payload(packet, 1024 * 1024)?) } else { None },
    }))
}
fn sms_message(value: &Value) -> Result<SmsMessage> {
    let b = object(value, "messages")?;
    let addresses = array(b, "addresses", MAX_ADDRESSES, true)?.iter().map(|v| {
        let address = object(v, "addresses")?;
        identifier(&required_text(address, "address", MAX_ID)?, "address")
    }).collect::<Result<Vec<_>>>()?;
    if addresses.is_empty() { return Err(FeatureError::Malformed("addresses")); }
    let attachments = array(b, "attachments", MAX_ATTACHMENTS, false)?.iter().map(|v| {
        let attachment = object(v, "attachments")?;
        Ok(SmsAttachment { part_id: nonnegative(required_int(attachment, "part_id")?, "part_id")?,
            mime_type: required_id(attachment, "mime_type")?, unique_identifier: required_id(attachment, "unique_identifier")?,
            encoded_thumbnail: optional_text(attachment, "encoded_thumbnail", 16 * 1024)?, })
    }).collect::<Result<Vec<_>>>()?;
    let read = if b.contains_key("read") { boolean(b, "read", false)? } else { return Err(FeatureError::Malformed("read")); };
    let event = u32::try_from(required_int(b, "event")?).map_err(|_| FeatureError::Malformed("event"))?;
    Ok(SmsMessage {
        id: nonnegative(required_int(b, "_id")?, "_id")?, thread_id: nonnegative(required_int(b, "thread_id")?, "thread_id")?, addresses,
        body: required_text(b, "body", MAX_TEXT)?, date: nonnegative(required_int(b, "date")?, "date")?,
        message_type: i32::try_from(required_int(b, "type")?).map_err(|_| FeatureError::Malformed("type"))?, read, event,
        subscription_id: optional_int(b, "sub_id")?.map(|n| i32::try_from(n).map_err(|_| FeatureError::Malformed("sub_id"))).transpose()?, attachments,
    })
}
fn contact_uids(b: &Map<String, Value>) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    array(b, "uids", MAX_CONTACTS, true)?.iter().map(|v| {
        let uid = identifier(v.as_str().ok_or(FeatureError::Malformed("uids"))?, "uids")?;
        if uid == "uids" || !seen.insert(uid.clone()) { return Err(FeatureError::Malformed("uids")); }
        Ok(uid)
    }).collect()
}

/// Decode data only. Unknown optional fields are ignored; unknown packets fail safely.
/// Icons/MMS return transfer descriptors, never downloaded bytes. Call only after trust checks.
pub fn decode(packet: &Packet) -> Result<Event> {
    packet.encode().map_err(|_| FeatureError::Malformed("frame"))?;
    let b = &packet.body;
    match packet.kind.as_str() {
        "kdeconnect.share.request" => {
            if b.contains_key("filename") || packet.payload_size.is_some() { return Err(FeatureError::PayloadHandoff("shared file")); }
            if b.contains_key("text") { return Ok(Event::SharedText { text: required_text(b, "text", MAX_TEXT)? }); }
            let url = required_text(b, "url", 8192)?; validate_url(&url)?; Ok(Event::UrlOffer { url })
        }
        "kdeconnect.clipboard" | "kdeconnect.clipboard.connect" => {
            no_payload(packet)?;
            let connect = packet.kind.ends_with(".connect");
            let content = optional_text(b, "content", MAX_TEXT)?;
            if !connect && content.is_none() { return Err(FeatureError::Malformed("content")); }
            let timestamp = if connect { Some(nonnegative(required_int(b, "timestamp")?, "timestamp")?) } else { None };
            Ok(Event::ClipboardOffer { content, timestamp })
        }
        "kdeconnect.notification" => notification(packet),
        "kdeconnect.telephony" => {
            no_payload(packet)?;
            let kind = match required_text(b, "event", 32)?.as_str() {
                "ringing" => CallKind::Ringing, "talking" => CallKind::Talking, "missedCall" => CallKind::MissedCall,
                _ => return Err(FeatureError::Unsupported),
            };
            Ok(Event::Call(CallAlert { kind, cancelled: boolean(b, "isCancel", false)?,
                phone_number: optional_text(b, "phoneNumber", 128)?, contact_name: optional_text(b, "contactName", 256)? }))
        }
        "kdeconnect.sms.messages" => {
            no_payload(packet)?;
            if required_int(b, "version")? != 2 { return Err(FeatureError::Unsupported); }
            Ok(Event::SmsMessages(array(b, "messages", MAX_MESSAGES, true)?.iter().map(sms_message).collect::<Result<_>>()?))
        }
        "kdeconnect.sms.attachment_file" => Ok(Event::SmsAttachment {
            filename: required_id(b, "filename")?, payload: payload(packet, 32 * 1024 * 1024)?,
        }),
        "kdeconnect.contacts.response_uids_timestamps" => {
            no_payload(packet)?;
            let contacts = contact_uids(b)?.into_iter().map(|uid| {
                let timestamp = nonnegative(integer(b.get(&uid).ok_or(FeatureError::Malformed("contact timestamp"))?, "contact timestamp")?, "contact timestamp")?;
                Ok(ContactTimestamp { uid, timestamp })
            }).collect::<Result<_>>()?;
            Ok(Event::ContactIndex(contacts))
        }
        "kdeconnect.contacts.response_vcards" => {
            no_payload(packet)?;
            let cards = contact_uids(b)?.into_iter().map(|uid| {
                let card = b.get(&uid).and_then(Value::as_str).ok_or(FeatureError::Malformed("vcard"))?;
                let card = text(card, MAX_CARD, "vcard")?;
                if card.lines().next() != Some("BEGIN:VCARD") || card.lines().last() != Some("END:VCARD") || card.lines().count() > 2048 {
                    return Err(FeatureError::Malformed("vcard"));
                }
                Ok(ContactCard { uid, vcard: card })
            }).collect::<Result<_>>()?;
            Ok(Event::ContactCards(cards))
        }
        _ => Err(FeatureError::Unsupported),
    }
}

fn packet(kind: &str, body: Value) -> Result<Packet> {
    let packet = Packet { id: 0, kind: kind.into(), body: body.as_object().cloned().ok_or(FeatureError::Malformed("body"))?,
        payload_size: None, payload_transfer_info: None, extra: Map::new() };
    packet.encode().map_err(|_| FeatureError::Limit("packet"))?;
    Ok(packet)
}
pub fn send_url(url: &str) -> Result<Packet> { validate_url(url)?; packet("kdeconnect.share.request", json!({"url":url})) }
/// Manual clipboard send only. No connect handshake/synchronization packet is emitted.
pub fn send_clipboard(content: &str) -> Result<Packet> { let content = text(content, MAX_TEXT, "content")?; packet("kdeconnect.clipboard", json!({"content":content})) }
pub fn find_phone() -> Result<Packet> { packet("kdeconnect.findmyphone.request", json!({})) }
pub fn request_notifications() -> Result<Packet> { packet("kdeconnect.notification.request", json!({"request":true})) }
pub fn dismiss_notification(id: &str) -> Result<Packet> { let id = identifier(id, "id")?; packet("kdeconnect.notification.request", json!({"cancel":id})) }
pub fn reply_notification(reply_id: &str, message: &str) -> Result<Packet> {
    let id = identifier(reply_id, "requestReplyId")?; let message = text(message, MAX_TEXT, "message")?;
    if message.trim().is_empty() { return Err(FeatureError::Malformed("message")); }
    packet("kdeconnect.notification.reply", json!({"requestReplyId":id,"message":message}))
}
pub fn notification_action(id: &str, action: &str) -> Result<Packet> {
    let id = identifier(id, "key")?; let action = identifier(action, "action")?;
    packet("kdeconnect.notification.action", json!({"key":id,"action":action}))
}
pub fn mute_ringer() -> Result<Packet> { packet("kdeconnect.telephony.request_mute", json!({})) }
pub fn request_sms_conversations() -> Result<Packet> { packet("kdeconnect.sms.request_conversations", json!({})) }
pub fn request_sms_thread(thread_id: i64, range_start: Option<i64>, count: u16) -> Result<Packet> {
    nonnegative(thread_id, "threadID")?;
    if count == 0 || usize::from(count) > MAX_MESSAGES { return Err(FeatureError::Limit("numberToRequest")); }
    let mut body = json!({"threadID":thread_id,"numberToRequest":count});
    if let Some(start) = range_start { nonnegative(start, "rangeStartTimestamp")?; body["rangeStartTimestamp"] = json!(start); }
    packet("kdeconnect.sms.request_conversation", body)
}
fn phone_number(number: &str) -> Result<String> {
    let number = number.trim();
    if number.is_empty() || number.len() > 64 || !number.bytes().any(|b| b.is_ascii_digit())
        || number.bytes().enumerate().any(|(i, b)| !(b.is_ascii_digit() || b" -().".contains(&b) || (b == b'+' && i == 0))) {
        return Err(FeatureError::Malformed("recipient"));
    }
    Ok(number.into())
}
/// Text-only SMS v2 request. Android chooses actual transport/SIM and provides no delivery ack.
/// Root must show exact recipients/body/SIM in an explicit send confirmation, never auto-retry.
pub fn send_sms(recipients: &[String], message: &str, subscription_id: Option<i32>) -> Result<Packet> {
    if recipients.is_empty() || recipients.len() > 10 { return Err(FeatureError::Limit("recipients")); }
    let addresses = recipients.iter().map(|number| phone_number(number).map(|address| json!({"address":address}))).collect::<Result<Vec<_>>>()?;
    let message = text(message, MAX_TEXT, "messageBody")?;
    if message.trim().is_empty() { return Err(FeatureError::Malformed("messageBody")); }
    let mut body = json!({"version":2,"addresses":addresses,"messageBody":message,"attachments":[]});
    if let Some(id) = subscription_id { if id < -1 { return Err(FeatureError::Malformed("subID")); } body["subID"] = json!(id); }
    packet("kdeconnect.sms.request", body)
}
pub fn request_sms_attachment(part_id: i64, unique_identifier: &str) -> Result<Packet> {
    nonnegative(part_id, "part_id")?; let id = identifier(unique_identifier, "unique_identifier")?;
    packet("kdeconnect.sms.request_attachment", json!({"part_id":part_id,"unique_identifier":id}))
}
pub fn request_contacts_index() -> Result<Packet> { packet("kdeconnect.contacts.request_all_uids_timestamps", json!({})) }
/// Request a small explicit batch. Android may omit missing contact UIDs.
pub fn request_contact_cards(uids: &[String]) -> Result<Packet> {
    if uids.is_empty() || uids.len() > 8 { return Err(FeatureError::Limit("uids")); }
    let uids = uids.iter().map(|uid| identifier(uid, "uids")).collect::<Result<Vec<_>>>()?;
    let body = json!({"uids":uids}); contact_uids(body.as_object().unwrap())?;
    packet("kdeconnect.contacts.request_vcards_by_uid", body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn incoming(kind: &str, body: Value) -> Packet { packet(kind, body).unwrap() }
    fn sms_body() -> Value {
        json!({"version":2,"messages":[{"_id":12,"thread_id":7,"addresses":[{"address":"BANK"}],"body":"A private message","date":1700000000000i64,"type":1,"read":0,"event":1,"sub_id":-1}]})
    }
    fn enabled() -> FeatureFlags { FeatureFlags { urls:true, clipboard:true, notifications:true, calls:true, sms:true, contacts:true } }

    #[test]
    fn urls_require_http_authority_without_credentials_controls_or_bad_escapes() {
        for url in ["https://example.org/path?q=hello%20world#part", "http://127.0.0.1:8080/", "https://[::1]:444/", "HTTP://device.local/"] {
            assert!(send_url(url).is_ok(), "{url}");
        }
        for url in ["javascript:alert(1)", "file:///tmp/file", "https:///path", "https://user:pass@example.org/", "https://good.test\\@evil.test", "https://example.org/\n", "https://example.org/%x0", "https://example.org:70000/", "https://[invalid]/", "https://-bad.test/", "https://example.org:0/"] {
            assert!(send_url(url).is_err(), "{url}");
        }
        assert!(send_url(&format!("https://example.org/{}", "x".repeat(8192))).is_err());
    }
    #[test]
    fn share_and_clipboard_are_data_offers_and_never_auto_sync() {
        let url = send_url("https://example.org/").unwrap();
        assert_eq!(decode(&url).unwrap(), Event::UrlOffer { url:"https://example.org/".into() });
        let clipboard = send_clipboard("line one\nline two").unwrap();
        assert_eq!(clipboard.kind,"kdeconnect.clipboard");
        assert!(!clipboard.body.contains_key("timestamp"));
        assert_eq!(decode(&clipboard).unwrap(), Event::ClipboardOffer {content:Some("line one\nline two".into()),timestamp:None});
        assert_eq!(decode(&incoming("kdeconnect.clipboard.connect",json!({"timestamp":0}))).unwrap(), Event::ClipboardOffer{content:None,timestamp:Some(0)});
        assert!(decode(&incoming("kdeconnect.clipboard",json!({}))).is_err());
        assert!(decode(&incoming("kdeconnect.clipboard.connect",json!({"timestamp":-1,"content":"text"}))).is_err());
        assert_eq!(find_phone().unwrap().body,Map::new());
        assert_eq!(find_phone().unwrap().kind,"kdeconnect.findmyphone.request");
    }
    #[test]
    fn notification_requests_match_android_reply_dismiss_and_action_keys() {
        assert_eq!(request_notifications().unwrap().body["request"],true);
        assert_eq!(dismiss_notification("0|package|42|null|0").unwrap().body["cancel"],"0|package|42|null|0");
        let reply = reply_notification("opaque-reply","Reviewed reply").unwrap();
        assert_eq!(reply.kind,"kdeconnect.notification.reply");
        assert_eq!(reply.body["requestReplyId"],"opaque-reply");
        assert_eq!(reply.body["message"],"Reviewed reply");
        let action = notification_action("notification-key","Open").unwrap();
        assert_eq!(action.body["key"],"notification-key"); assert_eq!(action.body["action"],"Open");
        assert!(reply_notification("reply"," \n ").is_err());
        assert!(dismiss_notification("bad\nid").is_err());
    }
    #[test]
    fn notification_privacy_redaction_cancel_and_conversation_are_supported() {
        let private = incoming("kdeconnect.notification",json!({"id":"n1","appName":"Messaging","isClearable":true,"future":1}));
        let Event::Notification(note) = decode(&private).unwrap() else { panic!() };
        assert_eq!(note.text,""); assert_eq!(note.reply_id,None);
        let full = incoming("kdeconnect.notification",json!({"id":"n2","time":"1700000000000","requestReplyId":"reply2","actions":["Archive"],"conversation":[{"sender":"Person","content":"Hi"}]}));
        let Event::Notification(note) = decode(&full).unwrap() else { panic!() };
        assert_eq!(note.time,Some(1700000000000)); assert_eq!(note.reply_id.as_deref(),Some("reply2"));
        assert_eq!(note.conversation[0].content,"Hi");
        assert_eq!(decode(&incoming("kdeconnect.notification",json!({"id":"n2","isCancel":true}))).unwrap(),Event::NotificationCancelled{id:"n2".into()});
        assert!(decode(&incoming("kdeconnect.notification",json!({"text":"no ID"}))).is_err());
    }
    #[test]
    fn call_cancellation_accepts_android_string_boolean_without_requiring_number() {
        let Event::Call(call) = decode(&incoming("kdeconnect.telephony",json!({"event":"ringing","isCancel":"true","phoneThumbnail":"ignored image"}))).unwrap() else { panic!() };
        assert_eq!(call.kind,CallKind::Ringing); assert!(call.cancelled); assert!(call.phone_number.is_none());
        assert!(matches!(decode(&incoming("kdeconnect.telephony",json!({"event":"sms"}))),Err(FeatureError::Unsupported)));
        assert!(decode(&incoming("kdeconnect.telephony",json!({"event":"talking","isCancel":"perhaps"}))).is_err());
        assert_eq!(mute_ringer().unwrap().kind,"kdeconnect.telephony.request_mute");
    }
    #[test]
    fn sms_uses_real_helper_numeric_fields_and_allows_incoming_alphanumeric_sender() {
        let Event::SmsMessages(messages) = decode(&incoming("kdeconnect.sms.messages",sms_body())).unwrap() else { panic!() };
        assert_eq!(messages[0].id,12); assert_eq!(messages[0].thread_id,7); assert!(!messages[0].read);
        assert_eq!(messages[0].addresses,["BANK"]); assert_eq!(messages[0].subscription_id,Some(-1));
        let mut body = sms_body(); body["messages"][0]["date"] = json!("1700000000000"); body["messages"][0]["read"] = json!(true); body["messages"][0]["future"] = json!("ignored");
        assert!(decode(&incoming("kdeconnect.sms.messages",body)).is_ok());
    }
    #[test]
    fn sms_fails_missing_required_fields_and_incompatible_versions() {
        for key in ["_id","thread_id","addresses","body","date","type","read","event"] {
            let mut body = sms_body(); body["messages"][0].as_object_mut().unwrap().remove(key);
            assert!(decode(&incoming("kdeconnect.sms.messages",body)).is_err(),"{key}");
        }
        let mut body = sms_body(); body["version"] = json!(1);
        assert_eq!(decode(&incoming("kdeconnect.sms.messages",body)),Err(FeatureError::Unsupported));
    }
    #[test]
    fn sms_send_is_explicit_structured_data_with_actual_camel_case_subscription() {
        let message = "Literal text: $(touch nope); \"quoted\"\nsecond line";
        let request = send_sms(&["+233 (20) 123-4567".into()],message,Some(3)).unwrap();
        assert_eq!(request.kind,"kdeconnect.sms.request"); assert_eq!(request.body["version"],2);
        assert_eq!(request.body["addresses"][0]["address"],"+233 (20) 123-4567");
        assert_eq!(request.body["messageBody"],message); assert_eq!(request.body["subID"],3);
        assert!(!request.body.contains_key("sub_id")); assert!(!request.body.contains_key("phoneNumber"));
        assert_eq!(request.body["attachments"],json!([]));
        for bad in ["+123;command", "123\n456", "$(secret)", "email@example.org", "*123#", "+", "123,+456"] {
            let error = send_sms(&[bad.into()],"private body",None).unwrap_err();
            assert!(!error.to_string().contains(bad)); assert!(!error.to_string().contains("private body"));
        }
        assert!(send_sms(&[],"body",None).is_err());
        assert!(send_sms(&["123".into()]," ",None).is_err());
        assert!(send_sms(&["123".into()],"body",Some(-2)).is_err());
    }
    #[test]
    fn thread_and_attachment_requests_use_exact_android_namespace_and_names() {
        assert!(request_sms_conversations().unwrap().body.is_empty());
        let request = request_sms_thread(7,Some(1700000000000),20).unwrap();
        assert_eq!(request.body["threadID"],7); assert_eq!(request.body["numberToRequest"],20);
        assert_eq!(request.body["rangeStartTimestamp"],1700000000000i64);
        assert!(request_sms_thread(-1,None,20).is_err()); assert!(request_sms_thread(7,None,0).is_err());
        let request = request_sms_attachment(4,"image.jpg").unwrap();
        assert_eq!(request.body["part_id"],4); assert_eq!(request.body["unique_identifier"],"image.jpg");
    }
    #[test]
    fn contacts_follow_actual_inline_vcard_wire_and_reject_missing_duplicate_entries() {
        let index = incoming("kdeconnect.contacts.response_uids_timestamps",json!({"uids":["lookup-key"],"lookup-key":1700000000000i64,"future":true}));
        let Event::ContactIndex(contacts) = decode(&index).unwrap() else { panic!() };
        assert_eq!(contacts[0].uid,"lookup-key");
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Sample Person\r\nEND:VCARD\r\n";
        let cards = incoming("kdeconnect.contacts.response_vcards",json!({"uids":["lookup-key"],"lookup-key":card}));
        assert_eq!(decode(&cards).unwrap(),Event::ContactCards(vec![ContactCard{uid:"lookup-key".into(),vcard:card.into()}]));
        for body in [json!({"uids":["missing"]}),json!({"uids":["a","a"],"a":"BEGIN:VCARD\nEND:VCARD"}),json!({"uids":["uids"]})] {
            assert!(decode(&incoming("kdeconnect.contacts.response_vcards",body)).is_err());
        }
        assert!(request_contacts_index().unwrap().body.is_empty());
        assert_eq!(request_contact_cards(&["lookup-key".into()]).unwrap().kind,"kdeconnect.contacts.request_vcards_by_uid");
        assert!(request_contact_cards(&vec!["key".into();9]).is_err());
    }
    #[test]
    fn transfer_descriptors_are_bounded_and_shared_files_never_become_urls() {
        let mut attachment = incoming("kdeconnect.sms.attachment_file",json!({"filename":"photo.jpg"}));
        attachment.payload_size = Some(100); attachment.payload_transfer_info = Some(json!({"port":1742}).as_object().unwrap().clone());
        assert_eq!(decode(&attachment).unwrap(),Event::SmsAttachment{filename:"photo.jpg".into(),payload:PayloadDescriptor{size:100,port:1742}});
        attachment.payload_size = Some(33*1024*1024); assert!(decode(&attachment).is_err());
        attachment.payload_size = Some(100); attachment.payload_transfer_info.as_mut().unwrap().insert("port".into(),json!(0)); assert!(decode(&attachment).is_err());
        assert!(matches!(decode(&incoming("kdeconnect.share.request",json!({"filename":"file","url":"https://example.org"}))),Err(FeatureError::PayloadHandoff(_))));
    }
    #[test]
    fn oversized_arrays_text_ids_and_frames_are_rejected_without_partial_events() {
        assert!(send_clipboard(&"x".repeat(MAX_TEXT+1)).is_err());
        assert!(dismiss_notification(&"n".repeat(MAX_ID+1)).is_err());
        let mut body = sms_body(); body["messages"][0]["attachments"] = json!(vec![json!({});MAX_ATTACHMENTS+1]);
        assert!(matches!(decode(&incoming("kdeconnect.sms.messages",body)),Err(FeatureError::Limit("attachments"))));
        let mut packet = incoming("kdeconnect.notification",json!({"id":"n"})); packet.body.insert("unknown".into(),json!("x".repeat(70*1024)));
        assert!(decode(&packet).is_err());
    }
    #[test]
    fn default_policy_retains_nothing_and_disabling_private_feature_forgets_its_cache() {
        let mut state = PeerState::default();
        let clip = send_clipboard("private contents").unwrap();
        assert_eq!(state.receive(&clip),Err(FeatureError::Disabled)); assert_eq!(state.events().count(),0);
        state.set_enabled(enabled()); state.receive(&clip).unwrap();
        assert_eq!(state.events().count(),1);
        state.set_enabled(FeatureFlags::default()); assert_eq!(state.events().count(),0); assert_eq!(state.bytes,0);
    }
    #[test]
    fn notification_replacement_and_cancel_invalidate_old_reply_identifiers() {
        let mut state = PeerState::default(); state.set_enabled(enabled());
        for reply in ["old-reply","new-reply"] {
            state.receive(&incoming("kdeconnect.notification",json!({"id":"n","requestReplyId":reply}))).unwrap();
        }
        assert_eq!(state.events().count(),1);
        let Event::Notification(note) = state.events().next().unwrap() else { panic!() }; assert_eq!(note.reply_id.as_deref(),Some("new-reply"));
        state.receive(&incoming("kdeconnect.notification",json!({"id":"n","isCancel":true}))).unwrap();
        assert!(!state.events().any(|event|matches!(event,Event::Notification(_))));
    }
    #[test]
    fn event_cache_evicts_oldest_with_both_byte_and_entry_budgets() {
        let mut state = PeerState::default(); state.set_enabled(enabled());
        for n in 0..200 { state.receive(&incoming("kdeconnect.notification",json!({"id":format!("n{n}"),"text":"x".repeat(8192)}))).unwrap(); }
        assert!(state.bytes<=MAX_CACHE_BYTES); assert!(state.events().count()<=MAX_CACHE_EVENTS);
        assert!(!state.events().any(|event|matches!(event,Event::Notification(note) if note.id=="n0")));
        state.clear(); assert_eq!(state.events().count(),0); assert_eq!(state.bytes,0);
    }
    #[test]
    fn capabilities_respect_direction_and_never_enable_user_policy() {
        let incoming = vec!["kdeconnect.sms.request".into(),"kdeconnect.findmyphone.request".into()];
        let outgoing = vec!["kdeconnect.notification".into()];
        let cap = PeerCapabilities::from_peer(&incoming,&outgoing);
        assert!(cap.sms_send && cap.find_phone && cap.notifications); assert!(!cap.sms_read && !cap.call_alerts);
        assert_eq!(PeerState::default().flags(),FeatureFlags::default());
        let flags: FeatureFlags = serde_json::from_value(json!({"sms":true,"future":1})).unwrap();
        assert!(flags.sms); assert!(!flags.clipboard && !flags.contacts && !flags.notifications);
    }
}
