//! The paired phone as a key: walk-away lock, lock-screen unlock, administrator approval.
//!
//! Nothing here is on unless the user switched it on for one paired device. The
//! password always keeps working, and nothing in this module ever sees it.
//!
//! * Presence is only ever a reason to lock. `Walkaway` has one output, "lock now",
//!   and no code path leads from "the phone is here" to an unlock or an approval.
//! * Unlock and approve are signatures, not proximity: the phone signs a fresh
//!   challenge with a key it enrolled for this purpose (ECDSA P-256, held in the
//!   phone's hardware keystore behind its own biometric). The challenge binds the
//!   purpose, both pinned TLS certificates, the requesting user, the exact text
//!   shown to the user, a random nonce and an expiry.
//! * A challenge is answered once. Any answer to it, right or wrong, uses it up.
//!
//! The wire format is specified in `desos/phonekey/README.md`; the byte layout of the
//! signed message is `Challenge::message`, pinned by `tests/phonekey_vectors.json`.
use crate::identity::{hex, unhex};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use openssl::{ec::EcKey, ecdsa::EcdsaSig, pkey::{PKey, Public}, sha::sha256};
use serde_json::{Map, Value, json};
use std::{collections::{BTreeMap, VecDeque}, fs::{self, OpenOptions}, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};

/// Both the identity capability and the only packet type. Peers that do not list it
/// in their encrypted identity are never sent one, and ignore it if they were.
pub const CAPABILITY: &str = "des.phonekey.v1";
pub const VERSION: u64 = 1;
const DOMAIN: &[u8; 16] = b"DES-PHONEKEY-v1\0";
/// DER header of a P-256 SubjectPublicKeyInfo, up to and including the 0x04 that
/// starts an uncompressed point. Nothing else is accepted as a phone key.
const P256_SPKI: [u8; 27] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
];
pub const SPKI_LEN: usize = 91;

/// Seconds a request stays answerable unless the caller asks for less.
pub const REQUEST_TTL: u64 = 30;
pub const ENROL_TTL: u64 = 60;
pub const MAX_TTL: u64 = 120;
pub const MAX_ACTION: usize = 512;
pub const MAX_USER: usize = 64;
/// Presence heartbeat, and the bounds of the walk-away grace period.
pub const PRESENCE_INTERVAL_MS: u64 = 10_000;
pub const WALKAWAY_MIN: u32 = 30;
pub const WALKAWAY_MAX: u32 = 3600;
pub const WALKAWAY_DEFAULT: u32 = 120;

const MIN_INTERVAL_MS: u64 = 2_000;
const PER_MINUTE: usize = 6;
const PER_HOUR: usize = 30;
const FAILURES_BEFORE_LOCKOUT: u32 = 3;
const LOCKOUT_MS: u64 = 300_000;
const BAD_SIGNATURES_BEFORE_SUSPEND: u32 = 5;
const MAX_PENDING: usize = 4;
const RESULT_LIFETIME_MS: u64 = 150_000;
const RECENT_IDS: usize = 1024;
const AUDIT_LIMIT: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose { Enrol, Unlock, Approve }
impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self { Purpose::Enrol => "enrol", Purpose::Unlock => "unlock", Purpose::Approve => "approve" }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text { "enrol" => Some(Purpose::Enrol), "unlock" => Some(Purpose::Unlock), "approve" => Some(Purpose::Approve), _ => None }
    }
}

/// Why a request was not made, or an answer was not believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Malformed, UnknownRequest, WrongDevice, WrongUser, Expired, BadSignature, NoKey,
    Busy, RateLimited, LockedOut, Disabled, Timeout, Cancelled, Revoked,
}
impl Reject {
    pub fn code(self) -> &'static str {
        match self {
            Reject::Malformed => "malformed", Reject::UnknownRequest => "unknown_request", Reject::WrongDevice => "wrong_device",
            Reject::WrongUser => "wrong_user", Reject::Expired => "expired", Reject::BadSignature => "bad_signature",
            Reject::NoKey => "no_key", Reject::Busy => "busy", Reject::RateLimited => "rate_limited", Reject::LockedOut => "locked_out",
            Reject::Disabled => "disabled", Reject::Timeout => "timeout", Reject::Cancelled => "cancelled", Reject::Revoked => "revoked",
        }
    }
}
impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.code()) }
}
impl std::error::Error for Reject {}

/// A login name as the signed message carries it.
pub fn valid_user(user: &str) -> bool {
    !user.is_empty() && user.len() <= MAX_USER && !user.starts_with('-')
        && user.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
/// The text the user is shown and the phone signs: one line, nothing invisible, and
/// nothing that could reorder what is displayed.
pub fn valid_action(action: &str) -> bool {
    !action.is_empty() && action.len() <= MAX_ACTION && action.trim() == action
        && !action.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
}

/// A phone's enrolled signing key: ECDSA over P-256 and nothing else.
#[derive(Clone)]
pub struct PublicKey { spki: Vec<u8>, key: EcKey<Public> }
impl PublicKey {
    pub fn from_spki(spki: &[u8]) -> Result<Self, Reject> {
        if spki.len() != SPKI_LEN || spki[..P256_SPKI.len()] != P256_SPKI { return Err(Reject::Malformed); }
        let key = PKey::public_key_from_der(spki).and_then(|key| key.ec_key()).map_err(|_| Reject::Malformed)?;
        key.check_key().map_err(|_| Reject::Malformed)?;
        Ok(Self { spki: spki.to_vec(), key })
    }
    pub fn from_base64(text: &str) -> Result<Self, Reject> {
        if text.len() > 2 * SPKI_LEN { return Err(Reject::Malformed); }
        Self::from_spki(&B64.decode(text).map_err(|_| Reject::Malformed)?)
    }
    pub fn base64(&self) -> String { B64.encode(&self.spki) }
    /// SHA-256 of the SubjectPublicKeyInfo: how both sides name the key.
    pub fn id(&self) -> [u8; 32] { sha256(&self.spki) }
    /// A strict DER ECDSA signature over SHA-256 of `message`.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        if !(8..=72).contains(&signature.len()) { return false; }
        let Ok(parsed) = EcdsaSig::from_der(signature) else { return false };
        if parsed.to_der().ok().as_deref() != Some(signature) { return false; }
        parsed.verify(&sha256(message), &self.key).unwrap_or(false)
    }
}
impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "PublicKey({})", hex(&self.id()[..8])) }
}
/// The key's name as a person compares it: four groups from its SHA-256.
pub fn fingerprint(key_id: &[u8; 32]) -> String {
    hex(&key_id[..8]).to_uppercase().as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

/// The pairing a signature belongs to. Each side fills this in from its own TLS
/// session and pin, never from a packet, so a signature made for another desktop
/// or over another pairing does not verify here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding { pub desktop_id: String, pub desktop_cert: [u8; 32], pub phone_id: String, pub phone_cert: [u8; 32] }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub purpose: Purpose,
    pub request_id: [u8; 16],
    pub nonce: [u8; 32],
    pub user: String,
    pub action: String,
    /// Unix seconds on the desktop's clock.
    pub issued_at: u64,
    pub expires_at: u64,
}
impl Challenge {
    /// A fresh challenge with a random identifier and nonce.
    pub fn new(purpose: Purpose, user: &str, action: &str, wall: u64, ttl: u64) -> Result<Self, Reject> {
        let mut request_id = [0; 16];
        let mut nonce = [0; 32];
        openssl::rand::rand_bytes(&mut request_id).map_err(|_| Reject::Malformed)?;
        openssl::rand::rand_bytes(&mut nonce).map_err(|_| Reject::Malformed)?;
        let challenge = Self { purpose, request_id, nonce, user: user.into(), action: action.into(), issued_at: wall, expires_at: wall.saturating_add(ttl) };
        challenge.validate()?;
        Ok(challenge)
    }
    pub fn validate(&self) -> Result<(), Reject> {
        let ttl = self.expires_at.checked_sub(self.issued_at).ok_or(Reject::Malformed)?;
        if !valid_user(&self.user) || !valid_action(&self.action) || ttl == 0 || ttl > MAX_TTL || self.expires_at > i64::MAX as u64 { return Err(Reject::Malformed); }
        Ok(())
    }
    pub fn ttl(&self) -> u64 { self.expires_at - self.issued_at }
    /// The exact bytes the phone signs. Every variable field carries a 32-bit
    /// big-endian length, so no two different requests share an encoding.
    pub fn message(&self, binding: &Binding, key_id: &[u8; 32]) -> Vec<u8> {
        let mut out = DOMAIN.to_vec();
        let fields: [&[u8]; 10] = [
            self.purpose.as_str().as_bytes(), binding.desktop_id.as_bytes(), &binding.desktop_cert, binding.phone_id.as_bytes(),
            &binding.phone_cert, key_id, &self.request_id, &self.nonce, self.user.as_bytes(), self.action.as_bytes(),
        ];
        for field in fields {
            out.extend((field.len() as u32).to_be_bytes());
            out.extend(field);
        }
        out.extend(self.issued_at.to_be_bytes());
        out.extend(self.expires_at.to_be_bytes());
        out
    }
    /// The request as the phone receives it. `op` is `enrol` or `request`.
    pub fn body(&self, key_id: Option<&[u8; 32]>) -> Value {
        let mut body = json!({
            "v": VERSION, "op": if self.purpose == Purpose::Enrol { "enrol" } else { "request" }, "purpose": self.purpose.as_str(),
            "request_id": hex(&self.request_id), "nonce": B64.encode(self.nonce), "user": self.user, "action": self.action,
            "issued_at": self.issued_at, "expires_at": self.expires_at,
        });
        if let Some(id) = key_id { body["key_id"] = json!(hex(id)); }
        body
    }
    /// A challenge the caller made itself, because the caller is the one who will
    /// verify the answer. Returns `None` when the caller supplied none of it.
    pub fn from_fields(purpose: Purpose, user: &str, action: &str, fields: &BTreeMap<String, Value>) -> Result<Option<Self>, Reject> {
        let names = ["request_id", "nonce", "issued_at", "expires_at"];
        if names.iter().all(|name| !fields.contains_key(*name)) { return Ok(None); }
        let text = |name: &str| fields.get(name).and_then(Value::as_str).ok_or(Reject::Malformed);
        let number = |name: &str| fields.get(name).and_then(Value::as_u64).ok_or(Reject::Malformed);
        let request_id = parse_id(text("request_id")?)?;
        let nonce: [u8; 32] = decode(text("nonce")?, 32)?.try_into().map_err(|_| Reject::Malformed)?;
        let challenge = Self { purpose, request_id, nonce, user: user.into(), action: action.into(), issued_at: number("issued_at")?, expires_at: number("expires_at")? };
        challenge.validate()?;
        Ok(Some(challenge))
    }
}
pub fn parse_id(text: &str) -> Result<[u8; 16], Reject> {
    if text.len() != 32 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) { return Err(Reject::Malformed); }
    unhex(text).ok().and_then(|bytes| bytes.try_into().ok()).ok_or(Reject::Malformed)
}
fn decode(text: &str, most: usize) -> Result<Vec<u8>, Reject> {
    if text.len() > most.div_ceil(3) * 4 { return Err(Reject::Malformed); }
    B64.decode(text).map_err(|_| Reject::Malformed)
}

#[derive(Clone, Debug)]
pub enum Decision {
    /// A valid signature over this challenge. For an enrolment, the key it enrolled.
    Approved { signature: Vec<u8>, key: PublicKey },
    /// The person said no on the phone.
    Denied,
    Failed(Reject),
}
#[derive(Clone, Debug)]
pub struct Outcome {
    pub challenge: Challenge,
    pub device: String,
    pub decision: Decision,
    /// Too many forged answers: the caller must switch unlock and approve off.
    pub suspend: bool,
}
impl Outcome {
    pub fn approved(&self) -> bool { matches!(self.decision, Decision::Approved { .. }) }
    pub fn word(&self) -> &'static str {
        match &self.decision { Decision::Approved { .. } => "approved", Decision::Denied => "denied", Decision::Failed(why) => why.code() }
    }
}

struct Pending { challenge: Challenge, deadline: u64 }
#[derive(Default)]
struct Limit { issued: VecDeque<u64>, failures: u32, forged: u32, locked_until: u64 }

/// Absence may lock; presence never unlocks. This type cannot say anything else.
#[derive(Debug, Default)]
pub struct Walkaway { armed: bool, last_seen: Option<u64> }
impl Walkaway {
    /// An authenticated presence answer arrived over the paired TLS session.
    pub fn seen(&mut self, now: u64) { self.last_seen = Some(now); }
    pub fn forget(&mut self) { *self = Self::default(); }
    pub fn present(&self, now: u64, grace_ms: u64) -> bool { self.last_seen.is_some_and(|at| now.saturating_sub(at) < grace_ms) }
    pub fn last_seen(&self) -> Option<u64> { self.last_seen }
    pub fn armed(&self) -> bool { self.armed }
    /// True exactly once when the phone, having been here while the session was
    /// unlocked, has now been gone for the whole grace period.
    pub fn tick(&mut self, now: u64, grace_ms: u64, unlocked: bool) -> bool {
        if !unlocked { self.armed = false; return false; }
        if self.present(now, grace_ms) { self.armed = true; return false; }
        std::mem::take(&mut self.armed)
    }
}

/// Whether the lock screen may take a phone answer instead of the password. The
/// lock screen owns one of these; see the hook described in `connect/PHONEKEY.md`.
#[derive(Debug)]
pub struct UnlockPolicy { password_at: Option<u64>, failures: u32, max_age_ms: u64 }
impl UnlockPolicy {
    pub const MAX_FAILURES: u32 = 3;
    pub fn new(max_age_hours: u64) -> Self { Self { password_at: None, failures: 0, max_age_ms: max_age_hours.clamp(1, 72) * 3_600_000 } }
    /// The password was typed and accepted in this session, at the greeter or here.
    pub fn password_accepted(&mut self, now: u64) { self.password_at = Some(now); self.failures = 0; }
    pub fn phone_failed(&mut self) { self.failures = self.failures.saturating_add(1); }
    pub fn allowed(&self, now: u64) -> Result<(), &'static str> {
        let at = self.password_at.ok_or("the password is needed first after signing in")?;
        if now.saturating_sub(at) >= self.max_age_ms { return Err("the password is needed again after this long"); }
        if self.failures >= Self::MAX_FAILURES { return Err("the password is needed after failed phone unlocks"); }
        Ok(())
    }
}

/// Every pending request, limit and decision. Time is passed in as milliseconds
/// from one monotonic clock, so nothing here depends on the wall clock being right.
pub struct Manager {
    pending: BTreeMap<String, Pending>,
    results: BTreeMap<[u8; 16], (u64, Outcome)>,
    limits: BTreeMap<String, Limit>,
    /// Identifiers already used, so a caller's own challenge cannot be run twice.
    recent: VecDeque<[u8; 16]>,
    pub walkaway: Walkaway,
    audit: Option<PathBuf>,
}
impl Manager {
    pub fn new(audit: Option<PathBuf>) -> Self {
        Self { pending: BTreeMap::new(), results: BTreeMap::new(), limits: BTreeMap::new(), recent: VecDeque::new(), walkaway: Walkaway::default(), audit }
    }
    /// One line per request and per decision, in an owner-only file.
    pub fn audit(&self, event: &str, mut fields: Value) {
        let Some(path) = &self.audit else { return };
        let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        fields["ts"] = json!(wall);
        fields["event"] = json!(event);
        if fs::metadata(path).is_ok_and(|m| m.len() > AUDIT_LIMIT) { let _ = fs::rename(path, path.with_extension("log.1")); }
        if let Ok(mut file) = OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32).open(path) {
            let _ = writeln!(file, "{fields}");
        }
    }
    fn describe(device: &str, challenge: &Challenge) -> Value {
        json!({"device": device, "purpose": challenge.purpose.as_str(), "request_id": hex(&challenge.request_id), "user": challenge.user, "action": challenge.action})
    }
    pub fn locked_out(&self, device: &str, now: u64) -> bool { self.limits.get(device).is_some_and(|limit| now < limit.locked_until) }
    /// Register a request about to be sent to `device`.
    pub fn issue(&mut self, now: u64, device: &str, challenge: Challenge) -> Result<(), Reject> {
        self.expire(now);
        let refused = (|| {
            challenge.validate()?;
            let limit = self.limits.entry(device.into()).or_default();
            if now < limit.locked_until { return Err(Reject::LockedOut); }
            if self.pending.contains_key(device) || self.pending.len() >= MAX_PENDING { return Err(Reject::Busy); }
            if self.recent.contains(&challenge.request_id) { return Err(Reject::Malformed); }
            while limit.issued.front().is_some_and(|at| now.saturating_sub(*at) >= 3_600_000) { limit.issued.pop_front(); }
            let minute = limit.issued.iter().filter(|at| now.saturating_sub(**at) < 60_000).count();
            if limit.issued.back().is_some_and(|at| now.saturating_sub(*at) < MIN_INTERVAL_MS) || minute >= PER_MINUTE || limit.issued.len() >= PER_HOUR { return Err(Reject::RateLimited); }
            limit.issued.push_back(now);
            Ok(())
        })();
        let mut entry = Self::describe(device, &challenge);
        match refused {
            Err(why) => { entry["reason"] = json!(why.code()); self.audit("refused", entry); Err(why) }
            Ok(()) => {
                let deadline = now.saturating_add(challenge.ttl() * 1000);
                self.audit("request", entry);
                if self.recent.len() >= RECENT_IDS { self.recent.pop_front(); }
                self.recent.push_back(challenge.request_id);
                self.pending.insert(device.into(), Pending { challenge, deadline });
                Ok(())
            }
        }
    }
    fn finish(&mut self, now: u64, device: &str, challenge: Challenge, decision: Decision) {
        let limit = self.limits.entry(device.into()).or_default();
        let mut suspend = false;
        match &decision {
            Decision::Approved { .. } => limit.failures = 0,
            // The requester going away is not the phone's failure; the hourly and
            // per-minute limits still count the prompt it caused.
            Decision::Failed(Reject::Cancelled | Reject::Revoked) => {}
            other => {
                limit.failures += 1;
                if matches!(other, Decision::Failed(Reject::BadSignature | Reject::Malformed)) {
                    limit.forged += 1;
                    if limit.forged >= BAD_SIGNATURES_BEFORE_SUSPEND { limit.forged = 0; suspend = true; }
                }
                if limit.failures >= FAILURES_BEFORE_LOCKOUT { limit.failures = 0; limit.locked_until = now.saturating_add(LOCKOUT_MS); }
            }
        }
        let outcome = Outcome { challenge, device: device.into(), decision, suspend };
        let mut entry = Self::describe(device, &outcome.challenge);
        entry["decision"] = json!(outcome.word());
        if suspend { entry["suspended"] = json!(true); }
        if self.locked_out(device, now) { entry["locked_out"] = json!(true); }
        self.audit("decision", entry);
        self.results.insert(outcome.challenge.request_id, (now, outcome));
    }
    /// An answer arrived from `device` over its paired TLS session. `enrolled` is
    /// the key the trust store holds for that device right now, so revoking or
    /// unpairing takes effect on answers already in flight.
    pub fn resolve(&mut self, now: u64, device: &str, body: &Map<String, Value>, enrolled: Option<&PublicKey>, binding: &Binding) -> Result<(), Reject> {
        self.expire(now);
        let op = body.get("op").and_then(Value::as_str).unwrap_or("");
        if body.get("v").and_then(Value::as_u64) != Some(VERSION) || !matches!(op, "response" | "enrolled") { return Err(Reject::Malformed); }
        let stray = |manager: &Self, why: Reject| { manager.audit("ignored", json!({"device": device, "reason": why.code()})); why };
        let id = body.get("request_id").and_then(Value::as_str).ok_or(Reject::Malformed).and_then(parse_id).map_err(|why| stray(self, why))?;
        // An answer that names no pending request (a replay, a late answer) is
        // dropped; one for another device's request does not use that request up.
        if !self.pending.values().any(|p| p.challenge.request_id == id) { return Err(stray(self, Reject::UnknownRequest)); }
        if self.pending.get(device).is_none_or(|p| p.challenge.request_id != id) { return Err(stray(self, Reject::WrongDevice)); }
        let Pending { challenge, deadline } = self.pending.remove(device).expect("pending request");
        let decision = (|| {
            if now >= deadline { return Err(Reject::Expired); }
            if (challenge.purpose == Purpose::Enrol) != (op == "enrolled") || binding.phone_id != device { return Err(Reject::Malformed); }
            match body.get("decision").and_then(Value::as_str) {
                Some("denied") => return Ok(Decision::Denied),
                Some("approved") => {}
                _ => return Err(Reject::Malformed),
            }
            let key = if challenge.purpose == Purpose::Enrol {
                PublicKey::from_base64(body.get("public_key").and_then(Value::as_str).ok_or(Reject::Malformed)?)?
            } else {
                enrolled.cloned().ok_or(Reject::NoKey)?
            };
            let signature = decode(body.get("signature").and_then(Value::as_str).ok_or(Reject::Malformed)?, 72)?;
            if !key.verify(&challenge.message(binding, &key.id()), &signature) { return Err(Reject::BadSignature); }
            Ok(Decision::Approved { signature, key })
        })().unwrap_or_else(Decision::Failed);
        let failed = if let Decision::Failed(why) = &decision { Some(*why) } else { None };
        self.finish(now, device, challenge, decision);
        failed.map_or(Ok(()), Err)
    }
    /// The decision for a request, once. A second ask finds nothing.
    pub fn take(&mut self, request_id: &[u8; 16]) -> Option<Outcome> { self.results.remove(request_id).map(|(_, outcome)| outcome) }
    /// Stop waiting for a request: it timed out, or whoever asked went away.
    pub fn abandon(&mut self, now: u64, request_id: &[u8; 16], why: Reject) -> Option<Outcome> {
        let device = self.pending.iter().find(|(_, p)| p.challenge.request_id == *request_id).map(|(device, _)| device.clone());
        if let Some(device) = device {
            let pending = self.pending.remove(&device).expect("pending request");
            self.finish(now, &device, pending.challenge, Decision::Failed(why));
        }
        self.take(request_id)
    }
    /// Unpairing or revoking a device ends whatever it was being asked.
    pub fn revoke_device(&mut self, now: u64, device: &str) {
        if let Some(pending) = self.pending.remove(device) { self.finish(now, device, pending.challenge, Decision::Failed(Reject::Revoked)); }
        self.limits.remove(device);
        self.audit("revoked", json!({"device": device}));
    }
    pub fn expire(&mut self, now: u64) {
        let late: Vec<String> = self.pending.iter().filter(|(_, p)| now >= p.deadline).map(|(device, _)| device.clone()).collect();
        for device in late {
            let pending = self.pending.remove(&device).expect("pending request");
            self.finish(now, &device, pending.challenge, Decision::Failed(Reject::Timeout));
        }
        self.results.retain(|_, (at, _)| now.saturating_sub(*at) < RESULT_LIFETIME_MS);
    }
    pub fn pending(&self, device: &str) -> bool { self.pending.contains_key(device) }
}

/// Check an answer the daemon carried, against a challenge and a key the caller
/// holds itself. This is the whole decision for a verifier that does not trust the
/// daemon: the lock screen, or the administrator helper.
pub fn verify_answer(challenge: &Challenge, binding: &Binding, key: &PublicKey, signature: &[u8], wall: u64) -> Result<(), Reject> {
    challenge.validate()?;
    if wall >= challenge.expires_at { return Err(Reject::Expired); }
    if !key.verify(&challenge.message(binding, &key.id()), signature) { return Err(Reject::BadSignature); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::{ec::EcGroup, hash::MessageDigest, nid::Nid, pkey::Private, sign::Signer};

    struct Phone { private: PKey<Private>, public: PublicKey }
    impl Phone {
        fn new() -> Self {
            let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
            let private = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
            let public = PublicKey::from_spki(&private.public_key_to_der().unwrap()).unwrap();
            Self { private, public }
        }
        fn sign(&self, message: &[u8]) -> Vec<u8> {
            let mut signer = Signer::new(MessageDigest::sha256(), &self.private).unwrap();
            signer.sign_oneshot_to_vec(message).unwrap()
        }
        fn answer(&self, challenge: &Challenge, binding: &Binding) -> Map<String, Value> {
            let signature = self.sign(&challenge.message(binding, &self.public.id()));
            let mut body = json!({"v": 1, "op": "response", "request_id": hex(&challenge.request_id), "decision": "approved", "signature": B64.encode(signature)});
            if challenge.purpose == Purpose::Enrol { body["op"] = json!("enrolled"); body["public_key"] = json!(self.public.base64()); }
            body.as_object().unwrap().clone()
        }
    }
    fn binding() -> Binding {
        Binding { desktop_id: "d".repeat(32), desktop_cert: [1; 32], phone_id: "p".repeat(32), phone_cert: [2; 32] }
    }
    fn approve(action: &str) -> Challenge { Challenge::new(Purpose::Approve, "coxy", action, 1_800_000_000, REQUEST_TTL).unwrap() }
    fn manager() -> Manager { Manager::new(None) }
    const PHONE: &str = "pppppppppppppppppppppppppppppppp";
    fn word(manager: &mut Manager, challenge: &Challenge) -> &'static str { manager.take(&challenge.request_id).unwrap().word() }

    #[test] fn a_signed_answer_is_approved_and_carries_its_signature() {
        let (phone, mut m, c) = (Phone::new(), manager(), approve("sudo apt upgrade"));
        m.issue(0, PHONE, c.clone()).unwrap();
        m.resolve(5_000, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding()).unwrap();
        let outcome = m.take(&c.request_id).unwrap();
        let Decision::Approved { signature, key } = &outcome.decision else { panic!("not approved") };
        // An outside verifier reaches the same answer from the bytes alone.
        assert_eq!(verify_answer(&c, &binding(), key, signature, c.issued_at + 5), Ok(()));
        assert_eq!(verify_answer(&c, &binding(), key, signature, c.expires_at), Err(Reject::Expired));
        assert!(m.take(&c.request_id).is_none(), "a decision is handed out once");
    }
    #[test] fn the_signature_covers_every_field_of_the_request() {
        let (phone, c, b) = (Phone::new(), approve("sudo apt upgrade"), binding());
        let signature = phone.sign(&c.message(&b, &phone.public.id()));
        let key = &phone.public;
        assert!(key.verify(&c.message(&b, &key.id()), &signature));
        let changed: Vec<(Challenge, Binding)> = vec![
            (Challenge { purpose: Purpose::Unlock, ..c.clone() }, b.clone()),
            (Challenge { user: "mallory".into(), ..c.clone() }, b.clone()),
            (Challenge { action: "sudo apt upgrade -y".into(), ..c.clone() }, b.clone()),
            (Challenge { nonce: [9; 32], ..c.clone() }, b.clone()),
            (Challenge { request_id: [9; 16], ..c.clone() }, b.clone()),
            (Challenge { issued_at: c.issued_at + 1, ..c.clone() }, b.clone()),
            (Challenge { expires_at: c.expires_at + 1, ..c.clone() }, b.clone()),
            (c.clone(), Binding { desktop_id: "e".repeat(32), ..b.clone() }),
            (c.clone(), Binding { desktop_cert: [7; 32], ..b.clone() }),
            (c.clone(), Binding { phone_id: "q".repeat(32), ..b.clone() }),
            (c.clone(), Binding { phone_cert: [7; 32], ..b.clone() }),
        ];
        for (challenge, binding) in changed {
            assert!(!key.verify(&challenge.message(&binding, &key.id()), &signature), "{challenge:?} {binding:?}");
        }
        assert!(!key.verify(&c.message(&b, &[0; 32]), &signature), "the key's own name is signed");
    }
    #[test] fn field_lengths_are_signed_so_text_cannot_move_between_fields() {
        let b = binding();
        let one = Challenge { user: "ab".into(), action: "c".into(), ..approve("x") };
        let two = Challenge { user: "a".into(), action: "bc".into(), ..one.clone() };
        assert_ne!(one.message(&b, &[0; 32]), two.message(&b, &[0; 32]));
    }
    #[test] fn a_nonce_is_used_once_and_a_replay_finds_nothing() {
        let (phone, mut m, c) = (Phone::new(), manager(), approve("sudo true"));
        let answer = phone.answer(&c, &binding());
        m.issue(0, PHONE, c.clone()).unwrap();
        m.resolve(100, PHONE, &answer, Some(&phone.public), &binding()).unwrap();
        assert_eq!(m.resolve(200, PHONE, &answer, Some(&phone.public), &binding()), Err(Reject::UnknownRequest));
        assert_eq!(word(&mut m, &c), "approved");
        // The same answer cannot approve a later request, even for the same text.
        let again = Challenge::new(Purpose::Approve, "coxy", "sudo true", c.issued_at, REQUEST_TTL).unwrap();
        assert_ne!(again.nonce, c.nonce);
        m.issue(10_000, PHONE, again.clone()).unwrap();
        assert_eq!(m.resolve(10_100, PHONE, &answer, Some(&phone.public), &binding()), Err(Reject::UnknownRequest));
        let mut forged = answer.clone();
        forged.insert("request_id".into(), json!(hex(&again.request_id)));
        assert_eq!(m.resolve(10_200, PHONE, &forged, Some(&phone.public), &binding()), Err(Reject::BadSignature));
        assert_eq!(word(&mut m, &again), "bad_signature");
        assert_eq!(m.issue(20_000, PHONE, again), Err(Reject::Malformed), "a request identifier is never reused");
    }
    #[test] fn a_wrong_answer_uses_the_challenge_up() {
        let (phone, mut m, c) = (Phone::new(), manager(), approve("sudo true"));
        let mut bad = phone.answer(&c, &binding());
        bad.insert("signature".into(), json!(B64.encode(phone.sign(b"something else"))));
        m.issue(0, PHONE, c.clone()).unwrap();
        assert_eq!(m.resolve(100, PHONE, &bad, Some(&phone.public), &binding()), Err(Reject::BadSignature));
        assert_eq!(m.resolve(200, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding()), Err(Reject::UnknownRequest));
        assert_eq!(word(&mut m, &c), "bad_signature");
    }
    #[test] fn an_answer_after_the_deadline_is_refused() {
        let (phone, mut m, c) = (Phone::new(), manager(), approve("sudo true"));
        m.issue(0, PHONE, c.clone()).unwrap();
        // Expiry is by the monotonic deadline, whatever the wall clock says.
        let _ = m.resolve(REQUEST_TTL * 1000, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding());
        assert_eq!(word(&mut m, &c), "timeout");
        let c = approve("sudo false");
        m.issue(40_000, PHONE, c.clone()).unwrap();
        m.resolve(40_000 + REQUEST_TTL * 1000 - 1, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding()).unwrap();
    }
    #[test] fn another_device_or_another_key_cannot_answer() {
        let (phone, other, mut m, c) = (Phone::new(), Phone::new(), manager(), approve("sudo true"));
        let elsewhere = "q".repeat(32);
        m.issue(0, PHONE, c.clone()).unwrap();
        // A second paired device answers a request that was not sent to it.
        let theirs = Binding { phone_id: elsewhere.clone(), ..binding() };
        assert_eq!(m.resolve(100, &elsewhere, &other.answer(&c, &theirs), Some(&other.public), &theirs), Err(Reject::WrongDevice));
        assert!(m.pending(PHONE), "the real request is still answerable");
        // The right device, but a key that is not the enrolled one.
        assert_eq!(m.resolve(200, PHONE, &other.answer(&c, &binding()), Some(&phone.public), &binding()), Err(Reject::BadSignature));
        // A signature made for another desktop's pairing.
        let c = approve("sudo true");
        m.issue(10_000, PHONE, c.clone()).unwrap();
        let foreign = Binding { desktop_cert: [8; 32], ..binding() };
        assert_eq!(m.resolve(10_100, PHONE, &phone.answer(&c, &foreign), Some(&phone.public), &binding()), Err(Reject::BadSignature));
    }
    #[test] fn a_signature_for_another_user_or_purpose_is_refused() {
        let (phone, mut m, c) = (Phone::new(), manager(), approve("sudo true"));
        m.issue(0, PHONE, c.clone()).unwrap();
        let mallory = Challenge { user: "mallory".into(), ..c.clone() };
        assert_eq!(m.resolve(100, PHONE, &phone.answer(&mallory, &binding()), Some(&phone.public), &binding()), Err(Reject::BadSignature));
        let c = approve("Unlock DES UI");
        m.issue(10_000, PHONE, c.clone()).unwrap();
        let unlock = Challenge { purpose: Purpose::Unlock, ..c.clone() };
        assert_eq!(m.resolve(10_100, PHONE, &phone.answer(&unlock, &binding()), Some(&phone.public), &binding()), Err(Reject::BadSignature));
    }
    #[test] fn revoking_the_key_refuses_answers_already_in_flight() {
        let (phone, mut m, c) = (Phone::new(), manager(), approve("sudo true"));
        m.issue(0, PHONE, c.clone()).unwrap();
        assert_eq!(m.resolve(100, PHONE, &phone.answer(&c, &binding()), None, &binding()), Err(Reject::NoKey));
        let c = approve("sudo true");
        m.issue(10_000, PHONE, c.clone()).unwrap();
        m.revoke_device(10_100, PHONE);
        assert_eq!(m.resolve(10_200, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding()), Err(Reject::UnknownRequest));
        assert_eq!(word(&mut m, &c), "revoked");
    }
    #[test] fn malformed_answers_are_refused_without_panicking() {
        let (phone, b) = (Phone::new(), binding());
        let good = |c: &Challenge| phone.answer(c, &b);
        let edits: Vec<Box<dyn Fn(&mut Map<String, Value>)>> = vec![
            Box::new(|a| { a.remove("signature"); }),
            Box::new(|a| { a.insert("signature".into(), json!(42)); }),
            Box::new(|a| { a.insert("signature".into(), json!("not base64 !!")); }),
            Box::new(|a| { a.insert("signature".into(), json!("")); }),
            Box::new(|a| { a.insert("signature".into(), json!(B64.encode([0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01]))); }),
            Box::new(|a| { a.insert("signature".into(), json!("A".repeat(4096))); }),
            Box::new(|a| { a.insert("decision".into(), json!("yes")); }),
            Box::new(|a| { a.remove("decision"); }),
            Box::new(|a| { a.insert("op".into(), json!("enrolled")); }),
        ];
        for (n, edit) in edits.iter().enumerate() {
            let (mut m, c) = (manager(), approve("sudo true"));
            let mut answer = good(&c);
            edit(&mut answer);
            m.issue(0, PHONE, c.clone()).unwrap();
            assert!(m.resolve(100, PHONE, &answer, Some(&phone.public), &b).is_err(), "edit {n}");
            assert!(!m.take(&c.request_id).unwrap().approved(), "edit {n}");
        }
        // Answers that cannot be matched to a request leave it alone.
        let strays: Vec<Box<dyn Fn(&mut Map<String, Value>)>> = vec![
            Box::new(|a| { a.insert("v".into(), json!(2)); }),
            Box::new(|a| { a.remove("v"); }),
            Box::new(|a| { a.insert("op".into(), json!("presence")); }),
            Box::new(|a| { a.remove("request_id"); }),
            Box::new(|a| { a.insert("request_id".into(), json!("ZZ")); }),
            Box::new(|a| { let id = a["request_id"].as_str().unwrap().to_uppercase(); a.insert("request_id".into(), json!(id)); }),
        ];
        let (mut m, c) = (manager(), Challenge { request_id: [0xab; 16], ..approve("sudo true") });
        m.issue(0, PHONE, c.clone()).unwrap();
        for (n, edit) in strays.iter().enumerate() {
            let mut answer = good(&c);
            edit(&mut answer);
            assert!(m.resolve(100, PHONE, &answer, Some(&phone.public), &b).is_err(), "stray {n}");
            assert!(m.pending(PHONE), "stray {n}");
        }
        m.resolve(200, PHONE, &good(&c), Some(&phone.public), &b).unwrap();
    }
    #[test] fn trailing_bytes_and_non_canonical_signatures_are_refused() {
        let (phone, c, b) = (Phone::new(), approve("sudo true"), binding());
        let message = c.message(&b, &phone.public.id());
        let signature = phone.sign(&message);
        assert!(phone.public.verify(&message, &signature));
        let mut longer = signature.clone();
        longer.push(0);
        assert!(!phone.public.verify(&message, &longer));
        assert!(!phone.public.verify(&message, &signature[..signature.len() - 1]));
        assert!(!phone.public.verify(&message, &[]));
    }
    #[test] fn only_p256_keys_are_accepted() {
        let phone = Phone::new();
        let spki = phone.private.public_key_to_der().unwrap();
        assert!(PublicKey::from_spki(&spki).is_ok());
        assert!(PublicKey::from_spki(&spki[..90]).is_err());
        let mut off_curve = spki.clone();
        off_curve[90] ^= 1;
        assert!(PublicKey::from_spki(&off_curve).is_err(), "a point off the curve is not a key");
        let p384 = PKey::from_ec_key(EcKey::generate(&EcGroup::from_curve_name(Nid::SECP384R1).unwrap()).unwrap()).unwrap();
        assert!(PublicKey::from_spki(&p384.public_key_to_der().unwrap()).is_err());
        let rsa = PKey::from_rsa(openssl::rsa::Rsa::generate(2048).unwrap()).unwrap();
        assert!(PublicKey::from_spki(&rsa.public_key_to_der().unwrap()).is_err());
        assert!(PublicKey::from_base64("").is_err());
        assert!(PublicKey::from_base64(&"A".repeat(10_000)).is_err());
        assert_eq!(fingerprint(&[0xab; 32]), "ABAB-ABAB-ABAB-ABAB");
    }
    #[test] fn enrolment_proves_possession_of_the_new_key() {
        let (phone, other, mut m) = (Phone::new(), Phone::new(), manager());
        let c = Challenge::new(Purpose::Enrol, "coxy", "Enrol this phone as a key for coxy on DES UI", 1_800_000_000, ENROL_TTL).unwrap();
        m.issue(0, PHONE, c.clone()).unwrap();
        m.resolve(100, PHONE, &phone.answer(&c, &binding()), None, &binding()).unwrap();
        let Decision::Approved { key, .. } = m.take(&c.request_id).unwrap().decision else { panic!("not enrolled") };
        assert_eq!(key.id(), phone.public.id());
        // Offering someone else's public key with one's own signature enrols nothing.
        let c = Challenge::new(Purpose::Enrol, "coxy", "Enrol this phone as a key for coxy on DES UI", 1_800_000_000, ENROL_TTL).unwrap();
        m.issue(10_000, PHONE, c.clone()).unwrap();
        let mut stolen = phone.answer(&c, &binding());
        stolen.insert("public_key".into(), json!(other.public.base64()));
        assert_eq!(m.resolve(10_100, PHONE, &stolen, None, &binding()), Err(Reject::BadSignature));
        // An enrolment answer cannot stand in for an approval.
        let c = approve("sudo true");
        m.issue(20_000, PHONE, c.clone()).unwrap();
        let enrol = Challenge { purpose: Purpose::Enrol, ..c.clone() };
        assert_eq!(m.resolve(20_100, PHONE, &phone.answer(&enrol, &binding()), Some(&phone.public), &binding()), Err(Reject::Malformed));
    }
    #[test] fn requests_are_rate_limited_per_device() {
        let (phone, mut m) = (Phone::new(), manager());
        let ok = |m: &mut Manager, now: u64| {
            let c = approve("sudo true");
            m.issue(now, PHONE, c.clone())?;
            m.resolve(now + 1, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding())
        };
        ok(&mut m, 0).unwrap();
        assert_eq!(ok(&mut m, 1_999), Err(Reject::RateLimited), "two seconds between prompts");
        for n in 1..PER_MINUTE as u64 { ok(&mut m, n * 2_000).unwrap(); }
        assert_eq!(ok(&mut m, 20_000), Err(Reject::RateLimited), "six a minute");
        ok(&mut m, 60_000).unwrap();
        // Thirty an hour, however they are spaced.
        let mut now = 120_000;
        let mut made = PER_MINUTE + 1;
        while made < PER_HOUR { ok(&mut m, now).unwrap(); made += 1; now += 15_000; }
        assert_eq!(ok(&mut m, now), Err(Reject::RateLimited));
        ok(&mut m, 3_600_000).unwrap();
        // Another device has its own allowance.
        m.issue(3_600_001, &"q".repeat(32), approve("sudo true")).unwrap();
    }
    #[test] fn one_request_at_a_time_and_a_bounded_number_overall() {
        let mut m = manager();
        m.issue(0, PHONE, approve("sudo true")).unwrap();
        assert_eq!(m.issue(5_000, PHONE, approve("sudo true")), Err(Reject::Busy));
        for n in 1..MAX_PENDING { m.issue(0, &format!("{n:032}"), approve("sudo true")).unwrap(); }
        assert_eq!(m.issue(0, &"z".repeat(32), approve("sudo true")), Err(Reject::Busy));
    }
    #[test] fn three_failures_lock_the_phone_out_and_the_lockout_ends() {
        let (phone, mut m) = (Phone::new(), manager());
        let mut now = 0;
        for _ in 0..FAILURES_BEFORE_LOCKOUT {
            let c = approve("sudo true");
            m.issue(now, PHONE, c.clone()).unwrap();
            let mut answer = phone.answer(&c, &binding());
            answer.insert("decision".into(), json!("denied"));
            answer.remove("signature");
            m.resolve(now + 10, PHONE, &answer, Some(&phone.public), &binding()).unwrap();
            assert_eq!(word(&mut m, &c), "denied");
            now += 3_000;
        }
        assert!(m.locked_out(PHONE, now));
        assert_eq!(m.issue(now, PHONE, approve("sudo true")), Err(Reject::LockedOut));
        assert_eq!(m.issue(now + LOCKOUT_MS - 3_000, PHONE, approve("sudo true")), Err(Reject::LockedOut));
        let c = approve("sudo true");
        m.issue(now + LOCKOUT_MS, PHONE, c.clone()).unwrap();
        m.resolve(now + LOCKOUT_MS + 1, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding()).unwrap();
    }
    #[test] fn timeouts_count_as_failures_but_a_cancelled_request_does_not() {
        let mut m = manager();
        for n in 0..5u64 {
            let c = approve("sudo true");
            m.issue(n * 3_000, PHONE, c.clone()).unwrap();
            assert_eq!(m.abandon(n * 3_000 + 1, &c.request_id, Reject::Cancelled).unwrap().word(), "cancelled");
        }
        assert!(!m.locked_out(PHONE, 20_000));
        let mut m = manager();
        for n in 0..FAILURES_BEFORE_LOCKOUT as u64 {
            let c = approve("sudo true");
            m.issue(n * 40_000, PHONE, c.clone()).unwrap();
            m.expire(n * 40_000 + REQUEST_TTL * 1000);
            assert_eq!(word(&mut m, &c), "timeout");
        }
        assert!(m.locked_out(PHONE, 100_000));
    }
    #[test] fn repeated_forgeries_ask_for_the_capability_to_be_switched_off() {
        let (phone, mut m) = (Phone::new(), manager());
        let mut now = 0;
        let mut suspended = false;
        for _ in 0..BAD_SIGNATURES_BEFORE_SUSPEND {
            while m.locked_out(PHONE, now) { now += LOCKOUT_MS; }
            let c = approve("sudo true");
            m.issue(now, PHONE, c.clone()).unwrap();
            let mut answer = phone.answer(&c, &binding());
            answer.insert("signature".into(), json!(B64.encode(phone.sign(b"forged"))));
            let _ = m.resolve(now + 1, PHONE, &answer, Some(&phone.public), &binding());
            suspended = m.take(&c.request_id).unwrap().suspend;
            now += 3_000;
        }
        assert!(suspended);
    }
    #[test] fn requests_with_unsafe_text_are_never_made() {
        for user in ["", "-rf", "a b", "a/b", "root\n", &"u".repeat(65)] { assert!(!valid_user(user), "{user:?}"); }
        for user in ["coxy", "a.b-c_d", "u1"] { assert!(valid_user(user)); }
        for action in ["", " padded", "two\nlines", "tab\there", "bell\u{7}", "rtl\u{202e}override", "isolate\u{2066}", &"a".repeat(513)] { assert!(!valid_action(action), "{action:?}"); }
        assert!(valid_action("sudo apt install des-ui (as coxy on tty1)"));
        assert!(valid_action(&"a".repeat(512)));
        assert!(Challenge::new(Purpose::Approve, "coxy", "ok", 10, 0).is_err());
        assert!(Challenge::new(Purpose::Approve, "coxy", "ok", 10, MAX_TTL + 1).is_err());
        assert!(Challenge::new(Purpose::Approve, "coxy", "ok", 10, MAX_TTL).is_ok());
        let mut m = manager();
        let backwards = Challenge { issued_at: 20, expires_at: 10, ..approve("ok") };
        assert_eq!(m.issue(0, PHONE, backwards), Err(Reject::Malformed));
    }
    #[test] fn a_callers_own_challenge_is_taken_whole_or_not_at_all() {
        let c = approve("sudo true");
        let fields: BTreeMap<String, Value> = serde_json::from_value(c.body(None)).unwrap();
        assert_eq!(Challenge::from_fields(Purpose::Approve, "coxy", "sudo true", &fields).unwrap(), Some(c.clone()));
        assert_eq!(Challenge::from_fields(Purpose::Approve, "coxy", "sudo true", &BTreeMap::new()).unwrap(), None);
        for missing in ["request_id", "nonce", "issued_at", "expires_at"] {
            let mut partial = fields.clone();
            partial.remove(missing);
            assert_eq!(Challenge::from_fields(Purpose::Approve, "coxy", "sudo true", &partial), Err(Reject::Malformed), "{missing}");
        }
        let mut short = fields.clone();
        short.insert("nonce".into(), json!(B64.encode([1; 16])));
        assert_eq!(Challenge::from_fields(Purpose::Approve, "coxy", "sudo true", &short), Err(Reject::Malformed));
    }
    #[test] fn absence_locks_once_and_presence_never_unlocks() {
        let grace = 120_000;
        let mut w = Walkaway::default();
        assert!(!w.tick(0, grace, true), "a phone never seen locks nothing");
        w.seen(1_000);
        assert!(!w.tick(2_000, grace, true));
        assert!(w.armed());
        assert!(!w.tick(120_999, grace, true), "still inside the grace period");
        assert!(w.tick(121_000, grace, true), "gone for the whole grace period");
        assert!(!w.tick(122_000, grace, true), "the lock is asked for once");
        // The phone coming back while locked changes nothing: `tick` only ever
        // answers "lock", and a locked session disarms it.
        w.seen(130_000);
        assert!(!w.tick(131_000, grace, false));
        assert!(!w.armed());
        // Unlocked with the password while the phone is away: no instant re-lock.
        assert!(!w.tick(400_000, grace, true));
        w.seen(401_000);
        assert!(!w.tick(402_000, grace, true));
        assert!(w.tick(402_000 + grace, grace, true));
    }
    #[test] fn the_lock_screen_wants_the_password_first_and_again_later() {
        let mut policy = UnlockPolicy::new(12);
        assert!(policy.allowed(0).is_err(), "never before the password was typed in this session");
        policy.password_accepted(1_000);
        assert!(policy.allowed(2_000).is_ok());
        assert!(policy.allowed(1_000 + 12 * 3_600_000).is_err());
        for _ in 0..UnlockPolicy::MAX_FAILURES { assert!(policy.allowed(2_000).is_ok()); policy.phone_failed(); }
        assert!(policy.allowed(2_000).is_err());
        policy.password_accepted(3_000);
        assert!(policy.allowed(4_000).is_ok());
        assert!(UnlockPolicy::new(10_000).allowed(0).is_err());
    }
    #[test] fn every_request_and_decision_is_written_to_the_audit_log() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("phonekey-audit.log");
        let (phone, mut m, c) = (Phone::new(), Manager::new(Some(path.clone())), approve("sudo true"));
        m.issue(0, PHONE, c.clone()).unwrap();
        assert_eq!(m.issue(1, PHONE, approve("sudo true")), Err(Reject::Busy));
        m.resolve(100, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding()).unwrap();
        let _ = m.resolve(200, PHONE, &phone.answer(&c, &binding()), Some(&phone.public), &binding());
        let lines: Vec<Value> = fs::read_to_string(&path).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        let events: Vec<&str> = lines.iter().map(|line| line["event"].as_str().unwrap()).collect();
        assert_eq!(events, ["request", "refused", "decision", "ignored"]);
        assert_eq!(lines[0]["action"], "sudo true");
        assert_eq!(lines[1]["reason"], "busy");
        assert_eq!(lines[2]["decision"], "approved");
        assert_eq!(lines[3]["reason"], "unknown_request");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
