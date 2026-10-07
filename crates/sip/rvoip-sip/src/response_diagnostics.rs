//! Observation only: never changes SIP state, response delivery or negotiation.
//! Raw headers, SDP and arbitrary provider text never enter tracing.
use crate::api::incoming::IncomingResponse;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const REASON_WORDS: &str = "trying ringing call is being forwarded queued session progress ok accepted multiple choices moved permanently temporarily use proxy alternative service bad request unauthorized payment required forbidden not found method allowed acceptable authentication timeout gone entity too large uri long unsupported media type extension interval brief temporarily unavailable transaction does exist loop detected hops address incomplete ambiguous busy here pending undecipherable server internal error implemented gateway unavailable version global failure decline declined anywhere unwanted rejected anonymous caller identity id invalid unverified verified number destination origin originating terminating blocked block blocking spam suspected fraud fraudulent policy restricted restriction permission permissions denied prohibited limit exceeded rate capacity congestion network normal clearing user no response answer subscriber absent unallocated unspecified cause bearer capability facility incompatible invalid format disconnected disconnected session authorization outbound inbound disabled enabled account balance insufficient funds trunk routing route redirection requested security verification failed unable to complete the at this time resource exhausted maximum concurrent calls voice profile country international premium toll free tollfree dnis ani stir shaken attestation prohibited by carrier provider reason text screening authentication unsuccessful successful misconfiguration";

fn reason_text(input: &str) -> (String, bool) {
    // Unknown words are visibly redacted, not silently discarded. Do not try
    // to recognize every possible secret format in provider-controlled text.
    let mut redacted = input.len() > 512;
    let bounded: String = input.chars().take(512).collect();
    let mut words = Vec::new();
    for word in bounded.split_whitespace().take(48) {
        let clean = word
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .to_ascii_lowercase();
        if clean.chars().all(|c| c.is_ascii_alphabetic())
            && REASON_WORDS
                .split_whitespace()
                .any(|allowed| allowed == clean)
        {
            words.push(clean);
        } else {
            redacted = true;
            if words.last().is_none_or(|last| last != "[redacted]") {
                words.push("[redacted]".into());
            }
        }
    }
    (words.join(" "), redacted)
}

fn fingerprint(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn header_value(name: &str, displayed: &str) -> String {
    displayed
        .split_once(':')
        .filter(|(prefix, _)| prefix.eq_ignore_ascii_case(name))
        .map_or(displayed, |(_, value)| value)
        .trim()
        .to_owned()
}

fn reason_header(value: &str) -> Value {
    let mut parts = value.split(';');
    let protocol = match parts
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "Q.850" => "Q.850",
        "SIP" => "SIP",
        _ => "other",
    };
    let mut cause = None;
    let mut text = None;
    let mut analytics_block = serde_json::Map::new();
    for part in parts.take(12) {
        if let Some((name, value)) = part.trim().split_once('=') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("cause") {
                cause = value.parse::<u16>().ok().filter(|v| match protocol {
                    "Q.850" => *v <= 127,
                    "SIP" => (100..=699).contains(v),
                    _ => false,
                });
            } else if name.eq_ignore_ascii_case("text") {
                let (context, redacted) = reason_text(value.trim_matches('"'));
                text = Some(json!({"context":context,"redacted":redacted}));
            } else if let Some((field, value)) = analytics_block_parameter(name, value) {
                analytics_block.insert(field.into(), Value::String(value));
            }
        }
    }
    let analytics_block = if protocol != "SIP"
        || cause != Some(603)
        || analytics_block.get("version").and_then(Value::as_str) != Some("analytics1")
    {
        Value::Null
    } else if analytics_block.is_empty() {
        Value::Null
    } else {
        Value::Object(analytics_block)
    };
    json!({"protocol":protocol,"cause":cause,"text":text,"analytics_block":analytics_block})
}

/// Project only the recognized SIP 603 analytics1 profile fields. Carrier
/// redress contacts are intentionally observable; arbitrary Reason parameters
/// are not. URL credentials, query parameters and fragments are removed.
fn analytics_block_parameter(name: &str, value: &str) -> Option<(&'static str, String)> {
    let value = value.trim_matches('"').trim();
    let printable = |limit: usize| {
        value.len() <= limit
            && !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_graphic() && c != '"' && c != '<' && c != '>')
    };
    match name.to_ascii_lowercase().as_str() {
        "v" => value
            .eq_ignore_ascii_case("analytics1")
            .then(|| ("version", "analytics1".to_owned())),
        "location" => (!value.is_empty()
            && value.len() <= 32
            && value.chars().all(|c| c.is_ascii_alphabetic()))
        .then(|| ("location", value.to_ascii_lowercase())),
        "url" => {
            if !printable(256) {
                return None;
            }
            let mut url = url::Url::parse(value).ok()?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return None;
            }
            url.set_username("").ok()?;
            url.set_password(None).ok()?;
            url.set_query(None);
            url.set_fragment(None);
            Some(("redress_url", url.to_string()))
        }
        "tel" => (value.len() <= 32
            && !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | '(' | ')')))
        .then(|| ("redress_tel", value.to_owned())),
        "email" => (printable(128) && value.matches('@').count() == 1)
            .then(|| ("redress_email", value.to_owned())),
        _ => None,
    }
}

pub(crate) fn projection(response: &IncomingResponse, connection: &str) -> Value {
    let (reason, reason_redacted) = reason_text(&response.reason_phrase);
    let mut fields = serde_json::Map::new();
    fields.insert(
        "session_reference".into(),
        json!({"context": fingerprint(&response.call_id.to_string())}),
    );
    fields.insert(
        "reason_redacted".into(),
        json!({"context":reason_redacted.to_string()}),
    );
    fields.insert(
        "response_headers_available".into(),
        json!({"context":response.raw_response().is_some().to_string()}),
    );
    if let Some(raw) = response.raw_response() {
        for header in raw.headers.iter().take(128) {
            let name = header.name().to_string().to_ascii_lowercase();
            if !matches!(
                name.as_str(),
                "call-id" | "reason" | "x-telnyx-leg-id" | "x-telnyx-session-id" | "x-request-id"
            ) {
                continue;
            }
            let displayed = header.to_string();
            if displayed.len() > 1024 {
                continue;
            }
            let value = header_value(&name, &displayed);
            let context = if name == "call-id" {
                fingerprint(&value)
            } else if name == "reason" {
                reason_header(&value).to_string()
            } else {
                // Only canonical opaque UUIDs from explicitly named correlation
                // headers may survive; no bearer/call-control tokens are logged.
                match uuid::Uuid::parse_str(&value) {
                    Ok(id) => id.to_string(),
                    Err(_) => "[redacted: not a UUID]".to_owned(),
                }
            };
            fields.insert(name, json!({"context":context}));
        }
    }
    fields.insert(
        "carrier_reason_present".into(),
        json!({"context":fields.contains_key("reason").to_string()}),
    );
    json!({"schema":1,"event":"sip_response","connection_id":connection,
        "level":if response.status_code >= 400 && !matches!(response.status_code,401|407) {"WARN"} else {"INFO"},
        "detail":{"sip_status":response.status_code,"context":reason},"fields":fields})
}

pub(crate) fn observe(response: &IncomingResponse, connection: &str) {
    tracing::info!(target: "rvoip_sip::response_observation",
        observation = %projection(response, connection), "structured SIP response");
}

pub(crate) fn prepared(session: &str, connection: &str, call_id: &str) {
    // This adapter generated the outbound ID; retain it for carrier support
    // lookup only when it matches the exact random-id shape, never a foreign
    // Call-ID containing a user address or provider-controlled payload.
    let support_id = outbound_support_id(session, call_id);
    let observation = json!({"schema":1,"event":"sip_outbound_prepared",
        "level":"INFO","connection_id":connection,
        "fields":{"session_reference":{"context":fingerprint(session)},
                  "outbound_sip_call_id":{"context":support_id},
                  "call-id":{"context":fingerprint(call_id)}}});
    tracing::info!(target: "rvoip_sip::response_observation",
        observation = %observation, "structured SIP preparation");
}

fn outbound_support_id(session: &str, call_id: &str) -> String {
    let valid = session
        .strip_prefix("session-")
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .is_some_and(|id| session == format!("session-{id}"));
    if valid && call_id == format!("{session}@rvoip-sip") {
        call_id.to_owned()
    } else {
        "[redacted]".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_generated_outbound_ids_are_available_for_support_lookup() {
        let session = "session-3ac5b703-9172-4d1b-aace-b9bb7b725a39";
        let call = format!("{session}@rvoip-sip");
        assert_eq!(outbound_support_id(session, &call), call);
        assert_eq!(
            outbound_support_id("secret", "secret@example.com"),
            "[redacted]"
        );
        assert_eq!(
            outbound_support_id(session, "secret@example.com"),
            "[redacted]"
        );
    }
    #[test]
    fn rejection_text_is_useful_and_secrets_are_not_retained() {
        assert_eq!(reason_text("Decline"), ("decline".into(), false));
        assert_eq!(
            reason_text("Caller ID not verified"),
            ("caller id not verified".into(), false)
        );
        let (text, redacted) = reason_text(
            "Authorization: Bearer TOPSECRET sip:alice@example.com +18058253932 inline:KEY eyJhbGciOiJIUzI1NiJ9",
        );
        assert!(redacted);
        for secret in ["TOPSECRET", "alice", "18058253932", "KEY", "eyJhbGci"] {
            assert!(!text.contains(secret));
        }
        assert!(!text.contains('\n'));
    }
    #[test]
    fn reason_cause_survives_but_arbitrary_parameters_do_not() {
        let value = reason_header("Q.850;cause=21;text=\"Call rejected\";token=SECRET");
        assert_eq!(value["cause"], 21);
        assert_eq!(value["text"]["context"], "call rejected");
        assert!(!value.to_string().contains("SECRET"));
        assert!(reason_header("Q.850;cause=999")["cause"].is_null());
        assert_eq!(
            header_value("reason", "Reason: Q.850;cause=21"),
            "Q.850;cause=21"
        );
        assert_eq!(header_value("reason", "Q.850;cause=21"), "Q.850;cause=21");
    }
    #[test]
    fn sip_603_plus_analytics_parameters_are_kept_and_everything_else_is_not() {
        let value = reason_header(
            "SIP;cause=603;text=\"Network Blocked\";v=analytics1;location=terminating;url=\"https://carrier.example/redress?ref=42\";tel=\"+1-800-555-0100\";email=\"blocks@carrier.example\";token=SECRET",
        );
        assert_eq!(value["cause"], 603);
        assert_eq!(value["text"]["context"], "network blocked");
        assert_eq!(value["analytics_block"]["version"], "analytics1");
        assert_eq!(value["analytics_block"]["location"], "terminating");
        assert_eq!(
            value["analytics_block"]["redress_url"],
            "https://carrier.example/redress"
        );
        assert_eq!(value["analytics_block"]["redress_tel"], "+1-800-555-0100");
        assert_eq!(
            value["analytics_block"]["redress_email"],
            "blocks@carrier.example"
        );
        assert!(!value.to_string().contains("SECRET"));

        let hostile = reason_header(
            "SIP;cause=603;v=analytics2;location=\"nowhere near\";url=javascript:alert(1);tel=call-me;email=\"a@b@c\"",
        );
        assert!(hostile["analytics_block"].is_null(), "{hostile}");
        assert!(reason_header("Q.850;cause=16")["analytics_block"].is_null());
    }
    #[test]
    fn analytics_redress_requires_the_exact_profile_and_redacts_url_credentials() {
        for header in [
            "Q.850;cause=21;v=analytics1;url=https://example.test/redress",
            "SIP;cause=403;v=analytics1;url=https://example.test/redress",
            "SIP;cause=603;url=https://example.test/redress",
        ] {
            assert!(reason_header(header)["analytics_block"].is_null());
        }
        let result = reason_header("SIP;cause=603;v=analytics1;url=\"https://user:password@example.test/redress?token=CANARY#CANARY\"");
        assert_eq!(
            result["analytics_block"]["redress_url"],
            "https://example.test/redress"
        );
        let text = result.to_string();
        for secret in ["user", "password", "token", "CANARY"] {
            assert!(!text.contains(secret));
        }
    }
}
