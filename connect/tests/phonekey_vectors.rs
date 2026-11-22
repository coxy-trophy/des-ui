//! The signed message, pinned against vectors made by a second implementation.
//!
//! `phonekey_vectors.json` was produced by the DesOS administrator helper (Python,
//! with the OpenSSL command signing). The same file ships as
//! `desos/phonekey/testvectors.json` and is what the phone app is built against, so
//! all three must agree on every byte.
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use des_connect::{identity::{hex, unhex}, phonekey::{self, Binding, Challenge, PublicKey, Purpose}};
use serde_json::Value;

fn vectors() -> Value { serde_json::from_str(include_str!("phonekey_vectors.json")).unwrap() }
fn digest(value: &Value) -> [u8; 32] { unhex(value.as_str().unwrap()).unwrap().try_into().unwrap() }
fn challenge(vector: &Value) -> Challenge {
    Challenge {
        purpose: Purpose::parse(vector["purpose"].as_str().unwrap()).unwrap(),
        request_id: phonekey::parse_id(vector["request_id"].as_str().unwrap()).unwrap(),
        nonce: B64.decode(vector["nonce"].as_str().unwrap()).unwrap().try_into().unwrap(),
        user: vector["user"].as_str().unwrap().into(), action: vector["action"].as_str().unwrap().into(),
        issued_at: vector["issued_at"].as_u64().unwrap(), expires_at: vector["expires_at"].as_u64().unwrap(),
    }
}

#[test] fn the_signed_bytes_match_the_other_implementation_and_its_signatures_verify() {
    let file = vectors();
    let key = PublicKey::from_base64(file["public_key"].as_str().unwrap()).unwrap();
    assert_eq!(hex(&key.id()), file["key_id"].as_str().unwrap());
    assert_eq!(phonekey::fingerprint(&key.id()), file["fingerprint"].as_str().unwrap());
    let b = &file["binding"];
    let binding = Binding { desktop_id: b["desktop_id"].as_str().unwrap().into(), desktop_cert: digest(&b["desktop_cert"]), phone_id: b["phone_id"].as_str().unwrap().into(), phone_cert: digest(&b["phone_cert"]) };
    let all = file["vectors"].as_array().unwrap();
    assert!(all.len() >= 4);
    let mut purposes = Vec::new();
    for vector in all {
        let challenge = challenge(vector);
        challenge.validate().unwrap();
        purposes.push(challenge.purpose);
        assert_eq!(hex(&challenge.message(&binding, &key.id())), vector["message"].as_str().unwrap(), "{}", vector["action"]);
        let signature = B64.decode(vector["signature"].as_str().unwrap()).unwrap();
        assert_eq!(phonekey::verify_answer(&challenge, &binding, &key, &signature, challenge.issued_at), Ok(()));
        // The request as it goes on the wire carries exactly these values.
        let body = challenge.body(Some(&key.id()));
        for field in ["purpose", "request_id", "nonce", "user", "action", "issued_at", "expires_at"] { assert_eq!(body[field], vector[field], "{field}"); }
        assert_eq!(body["key_id"], file["key_id"]);
    }
    for purpose in [Purpose::Enrol, Purpose::Unlock, Purpose::Approve] { assert!(purposes.contains(&purpose)); }
    // One request's signature is no use for another.
    let first = B64.decode(all[0]["signature"].as_str().unwrap()).unwrap();
    assert!(phonekey::verify_answer(&challenge(&all[1]), &binding, &key, &first, 0).is_err());
}
