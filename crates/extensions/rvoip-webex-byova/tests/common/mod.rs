#![allow(dead_code)]
//! Shared fixtures: a test-only RS256 key, signed Webex tokens, and adapter
//! configuration. The key in `fixtures/` exists only for these tests.

use jsonwebtoken::{jwk::JwkSet, Algorithm, EncodingKey, Header};
use rvoip_webex_byova::{
    auth::{GRPC_SCHEMA_ID, WEBSOCKET_SCHEMA_ID},
    WebexByovaConfig, WebexJwsConfig, WebexJwsValidator, WebexRegistration, WebexVirtualAgent,
};
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub const ORG: &str = "test-org-0001";
pub const AGENT: &str = "test-agent";
pub const ISSUER: &str = "https://idbroker.test.invalid/idb";
pub const AUDIENCE: &str = "test-audience";
pub const NONCE: &str = "test-registration-nonce";
pub const WS_URL: &str = "https://connector.test.invalid/v1/va";
pub const GRPC_URL: &str = "https://connector.test.invalid:443";
pub const KID: &str = "test-key";

const PRIVATE_KEY: &str = include_str!("../fixtures/rs256-test-private-key-pkcs8.txt");
const MODULUS: &str = "5z4fvam3HcLLzmc0y1fHsF7lZeQBXl3aFyVCy8YgSi9nJSiSs3G_Hv1Prja7mHEMposW1E8DvlB70hIGraH849IshkRCvvo_LDU5cwVPaVB-rzmVYu0FUwCh3iFh3obZLtZVLdhko2JItbZbrE2oEFVir9rIau01qDzKGY2J75O9eufxObQVrNkMqQypg14Lc3kZ8fNNsSesEQmVM5keAzQSZmWGWpMN065iGsgS4QtZJva8zo9IF7hn4ul35oQ6KXGhUtGOzMN-1HK7g70B3hMOHPMcLY2TPnhyvbH_-vaqWA7UcyN_Q-EQOFtY92m6qFCgkbu_LVy2RCyAXzIBHQ";

pub fn jwks() -> JwkSet {
    serde_json::from_value(json!({"keys":[{
        "kty":"RSA","kid":KID,"use":"sig","alg":"RS256","n":MODULUS,"e":"AQAB"
    }]}))
    .expect("test JWKS")
}

pub fn jws_config() -> WebexJwsConfig {
    WebexJwsConfig::new(vec![ISSUER.into()], AUDIENCE, ORG)
        .with_websocket(WebexRegistration::websocket(WS_URL, NONCE))
        .with_grpc(WebexRegistration::grpc(GRPC_URL, NONCE))
}

pub fn validator() -> Arc<WebexJwsValidator> {
    Arc::new(WebexJwsValidator::new(jws_config(), jwks()).expect("validator"))
}

pub fn config() -> WebexByovaConfig {
    WebexByovaConfig::new(validator(), WebexVirtualAgent::new(AGENT, "Test agent"))
}

pub fn claims(grpc: bool) -> Map<String, Value> {
    let now = chrono::Utc::now().timestamp();
    let (url, schema) = if grpc {
        (GRPC_URL, GRPC_SCHEMA_ID)
    } else {
        (WS_URL, WEBSOCKET_SCHEMA_ID)
    };
    json!({
        "iss": ISSUER, "aud": AUDIENCE, "sub": "test-subject", "jti": NONCE,
        "iat": now, "exp": now + 3600,
        "com.cisco.user.orgid": ORG,
        "com.cisco.datasource.url": url,
        "com.cisco.datasource.schema.uuid": schema,
    })
    .as_object()
    .cloned()
    .expect("claims object")
}

pub fn sign(claims: &Map<String, Value>, kid: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.into());
    jsonwebtoken::encode(
        &header,
        claims,
        &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).expect("test key"),
    )
    .expect("sign test token")
}

pub fn token(grpc: bool) -> String {
    sign(&claims(grpc), KID)
}
