//! Gap plan §5.2 v1 punch list — coordinator signature-verify gate.
//!
//! Spins up a `UctpCoordinator` via `start_full_with_sig9421` so the
//! gate is wired. Drives four cases through `dispatch_inner`:
//!
//! 1. **Signed envelope verifies**: the auth handshake completes when
//!    `auth.hello` / `auth.response` carry valid signatures.
//! 2. **Tampered signature → `401-1 invalid-signature`**: mutating
//!    the payload after signing must fail verification.
//! 3. **Unsigned envelope of a required type → `401-1 signature-required`**:
//!    when the policy requires signatures on `auth.hello` but the
//!    envelope omits one.
//! 4. **Unsigned envelope of a non-required type passes**: the gate
//!    must not over-reach — types outside the policy's required set
//!    should not be rejected for missing signatures.

mod common;

use std::sync::Arc;

use chrono::Utc;
use ring::signature::{Ed25519KeyPair, KeyPair};
use rvoip_auth_core::bearer_stub;
use rvoip_auth_core::sig9421::{
    jcs_canonicalize, EnvelopeSignature, Sig9421Verifier, StaticKeyResolver,
};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, control};
use rvoip_uctp::state::{
    default_v0_descriptor, rejecting_handler, Sig9421Policy, UctpCoordinator, UctpCoordinatorCaps,
    ENVELOPE_CHANNEL_CAP,
};
use rvoip_uctp::types::MessageType;
use tokio::sync::mpsc;
use uuid::Uuid;

use base64::Engine;

fn signing_keypair() -> (Ed25519KeyPair, Vec<u8>) {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let pub_bytes = kp.public_key().as_ref().to_vec();
    (kp, pub_bytes)
}

/// Sign `env` with `kp`, attach the inline `signature` field. Mutates
/// `env` in place. Uses the same JCS canonicalization the verifier
/// applies, so a fresh verify must succeed.
fn sign_envelope(env: &mut UctpEnvelope, keyid: &str, kp: &Ed25519KeyPair) {
    env.signature = None;
    let value = serde_json::to_value(&env).expect("serialize envelope");
    let canonical = jcs_canonicalize(&value);
    let sig = kp.sign(canonical.as_bytes());
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.as_ref());
    env.signature = Some(EnvelopeSignature {
        keyid: keyid.into(),
        alg: "EdDSA".into(),
        sig: sig_b64,
    });
}

fn fresh_hello() -> UctpEnvelope {
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::AuthHello,
        id: format!("env_{}", Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: None,
        connid: None,
        in_reply_to: None,
        payload: serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_sig_test".into(),
                kind: "desktop".into(),
                platform: "test".into(),
                sdk_version: "sig9421-gate/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })
        .unwrap(),
        signature: None,
    }
}

fn keepalive_env() -> UctpEnvelope {
    // ConnectionQuality is an authenticated envelope; we use it only
    // to assert that a *non-required* type passes the gate unsigned.
    // The handler will subsequently fail to find the conn — but that
    // happens after the gate, which is what we're testing.
    UctpEnvelope {
        v: 1,
        msg_type: MessageType::ConnectionQuality,
        id: format!("env_{}", Uuid::new_v4().simple()),
        ts: Utc::now(),
        cid: None,
        sid: Some("sess_x".into()),
        connid: Some("conn_y".into()),
        in_reply_to: None,
        payload: serde_json::json!({"streams": []}),
        signature: None,
    }
}

fn build_coordinator(
    verifier: Arc<Sig9421Verifier>,
    policy: Sig9421Policy,
) -> (
    Arc<UctpCoordinator>,
    mpsc::Sender<UctpEnvelope>,
    mpsc::Receiver<UctpEnvelope>,
) {
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coord = UctpCoordinator::start_full_with_sig9421(
        "quic",
        in_rx,
        out_tx,
        events_tx,
        bearer_stub(),
        verifier,
        policy,
        Arc::new(default_v0_descriptor()),
        rejecting_handler(),
        UctpCoordinatorCaps::default(),
    );
    (coord, in_tx, out_rx)
}

#[tokio::test]
async fn signed_auth_hello_passes_verify_gate() {
    let (kp, pubkey) = signing_keypair();
    let mut resolver = StaticKeyResolver::new();
    resolver.insert("key:peer-1", pubkey);
    let verifier = Arc::new(Sig9421Verifier::new(Arc::new(resolver)));
    let (_coord, in_tx, mut out_rx) =
        build_coordinator(Arc::clone(&verifier), Sig9421Policy::auth_envelopes_only());

    let mut hello = fresh_hello();
    sign_envelope(&mut hello, "key:peer-1", &kp);
    in_tx.send(hello).await.expect("send hello");

    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("reply within deadline")
        .expect("channel open");
    assert_eq!(
        reply.msg_type,
        MessageType::AuthChallenge,
        "signed hello must produce an auth.challenge, not an error; got {:?}",
        reply.msg_type
    );
}

#[tokio::test]
async fn tampered_signature_rejected_with_401_invalid_signature() {
    let (kp, pubkey) = signing_keypair();
    let mut resolver = StaticKeyResolver::new();
    resolver.insert("key:peer-1", pubkey);
    let verifier = Arc::new(Sig9421Verifier::new(Arc::new(resolver)));
    let (_coord, in_tx, mut out_rx) =
        build_coordinator(verifier, Sig9421Policy::auth_envelopes_only());

    let mut hello = fresh_hello();
    sign_envelope(&mut hello, "key:peer-1", &kp);

    // Tamper after signing: change a payload field. Signature canon
    // covered the original payload, so verification must fail.
    let mut payload = hello.payload.as_object().cloned().unwrap();
    payload.insert(
        "auth_methods".into(),
        serde_json::json!(["malicious-mutation"]),
    );
    hello.payload = serde_json::Value::Object(payload);

    in_tx.send(hello).await.expect("send tampered hello");
    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("reply within deadline")
        .expect("channel open");
    assert_eq!(reply.msg_type, MessageType::Error);
    let err: control::Error = reply.decode_payload().unwrap();
    assert_eq!(err.code, 401);
    assert_eq!(err.category, "auth");
    assert_eq!(err.reason, "invalid-signature");
}

#[tokio::test]
async fn unsigned_required_type_rejected_with_signature_required() {
    let (_kp, pubkey) = signing_keypair();
    let mut resolver = StaticKeyResolver::new();
    resolver.insert("key:peer-1", pubkey);
    let verifier = Arc::new(Sig9421Verifier::new(Arc::new(resolver)));
    let (_coord, in_tx, mut out_rx) =
        build_coordinator(verifier, Sig9421Policy::auth_envelopes_only());

    let hello = fresh_hello(); // signature: None
    in_tx.send(hello).await.expect("send unsigned hello");
    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("reply within deadline")
        .expect("channel open");
    assert_eq!(reply.msg_type, MessageType::Error);
    let err: control::Error = reply.decode_payload().unwrap();
    assert_eq!(err.code, 401);
    assert_eq!(err.reason, "signature-required");
}

#[tokio::test]
async fn unsigned_non_required_type_passes_gate() {
    let (_kp, pubkey) = signing_keypair();
    let mut resolver = StaticKeyResolver::new();
    resolver.insert("key:peer-1", pubkey);
    let verifier = Arc::new(Sig9421Verifier::new(Arc::new(resolver)));
    // Policy requires signatures on auth envelopes ONLY; ConnectionQuality
    // is outside the set, so the gate must let it through (handler will
    // then reject for unrelated auth-state reasons — fine).
    let (_coord, in_tx, mut out_rx) =
        build_coordinator(verifier, Sig9421Policy::auth_envelopes_only());

    let env = keepalive_env();
    let env_id = env.id.clone();
    in_tx.send(env).await.expect("send keepalive");
    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), out_rx.recv())
        .await
        .expect("reply within deadline")
        .expect("channel open");

    // The gate let it through. The handler then rejects for "not
    // authed yet" (401 auth/unauthenticated). What we must NOT see
    // is a `signature-required` reason — that would mean the gate
    // misapplied the policy.
    if reply.msg_type == MessageType::Error {
        let err: control::Error = reply.decode_payload().unwrap();
        assert_ne!(
            err.reason, "signature-required",
            "gate must not reject non-required types for missing signature; got {err:?}"
        );
    }
    // And the in_reply_to must still correlate to our envelope id.
    assert_eq!(reply.in_reply_to.as_deref(), Some(env_id.as_str()));
}
