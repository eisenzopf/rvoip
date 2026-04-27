//! RFC 5626 SIP Outbound + RFC 5627 GRUU Contact parameter helpers.
//!
//! These two RFCs layer registration identity onto the standard Contact
//! header through a handful of new parameters:
//!
//! - **RFC 5626 §4.1 `+sip.instance`** — a UA-stable URN (typically a UUID)
//!   that survives process restarts and NAT rebinding. Quoted value with
//!   angle brackets: `+sip.instance="<urn:uuid:...>"`.
//! - **RFC 5626 §4.2 `reg-id`** — a positive integer identifying which of a
//!   UA's outbound flows is bound to a given registration. First flow is
//!   typically `reg-id=1`.
//! - **RFC 5627 §5.3.1 `pub-gruu`** — a public GRUU URI the registrar
//!   assigned to this binding. Quoted value: `pub-gruu="<sip:...>"`.
//! - **RFC 5627 §5.3.2 `temp-gruu`** — a temporary (privacy-preserving)
//!   GRUU for the same binding.
//!
//! The helpers below keep the quoting / key-casing conventions in one
//! place so callers (session-core, client code) don't re-derive them and
//! can unit-test against a single authoritative representation.
//!
//! Transport-layer keep-alive, flow-token management, and registration
//! state-machine work — the rest of RFC 5626 — live in `dialog-core`.
//! This module is the pure-serialisation layer only.

use crate::types::address::Address;
use crate::types::param::{GenericValue, Param};

/// Outbound registration parameters carried on a Contact header per
/// RFC 5626 §4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundContactParams {
    /// UA-stable instance URN (typically `urn:uuid:<uuid>`). Stored on the
    /// wire with surrounding angle brackets.
    pub instance_urn: String,
    /// Registration identifier for this outbound flow. First flow should
    /// typically use `1`.
    pub reg_id: u32,
}

/// GRUU URIs assigned by the registrar per RFC 5627 §5.3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GruuContactParams {
    /// Public GRUU URI (visible to anyone the UA calls).
    pub pub_gruu: Option<String>,
    /// Temporary (privacy-preserving) GRUU URI.
    pub temp_gruu: Option<String>,
}

/// Case-insensitive match against `Param::Other` entries without the `+`
/// prefix sensitivity.
fn param_matches(param: &Param, name: &str) -> bool {
    if let Param::Other(k, _) = param {
        k.eq_ignore_ascii_case(name)
    } else {
        false
    }
}

fn set_or_replace_quoted(address: &mut Address, name: &str, value: String) {
    address.params.retain(|p| !param_matches(p, name));
    address.params.push(Param::Other(
        name.to_string(),
        Some(GenericValue::Quoted(value)),
    ));
}

fn set_or_replace_token(address: &mut Address, name: &str, value: String) {
    address.params.retain(|p| !param_matches(p, name));
    address.params.push(Param::Other(
        name.to_string(),
        Some(GenericValue::Token(value)),
    ));
}

fn read_string_param(address: &Address, name: &str) -> Option<String> {
    address.params.iter().find_map(|p| match p {
        Param::Other(k, Some(v)) if k.eq_ignore_ascii_case(name) => match v {
            GenericValue::Token(s) | GenericValue::Quoted(s) => Some(s.clone()),
            GenericValue::Host(_) => None,
        },
        _ => None,
    })
}

/// Write the RFC 5626 outbound Contact parameters onto the address. If
/// either parameter was previously present, the existing entry is
/// replaced so repeated calls are idempotent.
pub fn set_outbound_contact_params(address: &mut Address, params: &OutboundContactParams) {
    // `+sip.instance` value is a URN inside angle brackets, as a quoted
    // string (RFC 5626 §4.1 "URN" production).
    set_or_replace_quoted(
        address,
        "+sip.instance",
        format!("<{}>", params.instance_urn),
    );
    set_or_replace_token(address, "reg-id", params.reg_id.to_string());
}

/// Read the RFC 5626 outbound Contact parameters from an address, if both
/// are present. Returns `None` when either parameter is missing —
/// RFC 5626 §4 treats them as a pair; a Contact with only one is either
/// malformed or pre-RFC-5626 legacy.
pub fn read_outbound_contact_params(address: &Address) -> Option<OutboundContactParams> {
    let instance_raw = read_string_param(address, "+sip.instance")?;
    // Strip the surrounding angle brackets. Tolerant: if they're missing
    // (non-conforming peer), take the raw string as-is.
    let instance_urn = instance_raw
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .map(|s| s.to_string())
        .unwrap_or(instance_raw);

    let reg_id_str = read_string_param(address, "reg-id")?;
    let reg_id = reg_id_str.parse::<u32>().ok()?;

    Some(OutboundContactParams {
        instance_urn,
        reg_id,
    })
}

/// Write the RFC 5627 GRUU Contact parameters. Each `None` field is
/// written-through as "remove"; existing entries for the named parameter
/// are always cleared first, so `set_gruu_contact_params(addr, Default::default())`
/// removes both GRUU entries from an address.
pub fn set_gruu_contact_params(address: &mut Address, params: &GruuContactParams) {
    address
        .params
        .retain(|p| !param_matches(p, "pub-gruu") && !param_matches(p, "temp-gruu"));
    if let Some(ref pub_g) = params.pub_gruu {
        address.params.push(Param::Other(
            "pub-gruu".to_string(),
            Some(GenericValue::Quoted(pub_g.clone())),
        ));
    }
    if let Some(ref temp_g) = params.temp_gruu {
        address.params.push(Param::Other(
            "temp-gruu".to_string(),
            Some(GenericValue::Quoted(temp_g.clone())),
        ));
    }
}

/// Read the RFC 5627 GRUU Contact parameters. Either or both may be
/// `None` — these are independent (a registrar may assign only pub-gruu).
pub fn read_gruu_contact_params(address: &Address) -> GruuContactParams {
    GruuContactParams {
        pub_gruu: read_string_param(address, "pub-gruu"),
        temp_gruu: read_string_param(address, "temp-gruu"),
    }
}

/// Add the RFC 5626 §5.4 `;ob` flag to the URI within an address. The
/// flag signals to the registrar that the UA is using outbound-style
/// registration and wants the flow association preserved. Idempotent.
pub fn mark_uri_as_outbound(address: &mut Address) {
    use crate::types::param::Param as P;
    if !address
        .uri
        .parameters
        .iter()
        .any(|p| matches!(p, P::Other(k, None) if k.eq_ignore_ascii_case("ob")))
    {
        address
            .uri
            .parameters
            .push(P::Other("ob".to_string(), None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Uri;
    use std::str::FromStr;

    fn make_address() -> Address {
        let uri = Uri::from_str("sip:alice@192.168.1.10:5060").unwrap();
        Address::new(uri)
    }

    #[test]
    fn set_outbound_contact_params_writes_both() {
        let mut addr = make_address();
        set_outbound_contact_params(
            &mut addr,
            &OutboundContactParams {
                instance_urn: "urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF".into(),
                reg_id: 1,
            },
        );
        let s = addr.to_string();
        assert!(
            s.contains("+sip.instance=\"<urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF>\""),
            "address string missing +sip.instance: {}",
            s
        );
        assert!(
            s.contains("reg-id=1"),
            "address string missing reg-id: {}",
            s
        );
    }

    #[test]
    fn outbound_contact_params_roundtrip() {
        let mut addr = make_address();
        let expected = OutboundContactParams {
            instance_urn: "urn:uuid:11111111-2222-3333-4444-555566667777".into(),
            reg_id: 2,
        };
        set_outbound_contact_params(&mut addr, &expected);
        let read = read_outbound_contact_params(&addr).unwrap();
        assert_eq!(read, expected);
    }

    #[test]
    fn outbound_contact_params_read_none_when_missing() {
        let addr = make_address();
        assert!(read_outbound_contact_params(&addr).is_none());
    }

    #[test]
    fn set_outbound_contact_params_is_idempotent() {
        let mut addr = make_address();
        let first = OutboundContactParams {
            instance_urn: "urn:uuid:aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            reg_id: 1,
        };
        let second = OutboundContactParams {
            instance_urn: "urn:uuid:bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
            reg_id: 5,
        };
        set_outbound_contact_params(&mut addr, &first);
        set_outbound_contact_params(&mut addr, &second);
        let read = read_outbound_contact_params(&addr).unwrap();
        assert_eq!(read, second);
        // Only one +sip.instance / reg-id entry each — no duplication.
        let instance_count = addr
            .params
            .iter()
            .filter(|p| matches!(p, Param::Other(k, _) if k.eq_ignore_ascii_case("+sip.instance")))
            .count();
        assert_eq!(instance_count, 1);
    }

    #[test]
    fn gruu_params_write_and_read() {
        let mut addr = make_address();
        let params = GruuContactParams {
            pub_gruu: Some("sip:alice+pub@example.com;gr=urn:uuid:foo".into()),
            temp_gruu: Some("sip:alice+temp@example.com;gr=urn:uuid:bar".into()),
        };
        set_gruu_contact_params(&mut addr, &params);
        let read = read_gruu_contact_params(&addr);
        assert_eq!(read, params);
    }

    #[test]
    fn gruu_params_pub_only() {
        // Registrar may assign only pub-gruu.
        let mut addr = make_address();
        let params = GruuContactParams {
            pub_gruu: Some("sip:alice+pub@example.com;gr=xyz".into()),
            temp_gruu: None,
        };
        set_gruu_contact_params(&mut addr, &params);
        let read = read_gruu_contact_params(&addr);
        assert_eq!(
            read.pub_gruu.as_deref(),
            Some("sip:alice+pub@example.com;gr=xyz")
        );
        assert!(read.temp_gruu.is_none());
    }

    #[test]
    fn gruu_clearing_empty_params_removes_existing() {
        let mut addr = make_address();
        set_gruu_contact_params(
            &mut addr,
            &GruuContactParams {
                pub_gruu: Some("sip:x@y".into()),
                temp_gruu: None,
            },
        );
        set_gruu_contact_params(&mut addr, &GruuContactParams::default());
        let read = read_gruu_contact_params(&addr);
        assert!(read.pub_gruu.is_none());
        assert!(read.temp_gruu.is_none());
    }

    #[test]
    fn mark_uri_as_outbound_is_idempotent() {
        let mut addr = make_address();
        mark_uri_as_outbound(&mut addr);
        mark_uri_as_outbound(&mut addr);
        let ob_count = addr
            .uri
            .parameters
            .iter()
            .filter(|p| matches!(p, Param::Other(k, None) if k.eq_ignore_ascii_case("ob")))
            .count();
        assert_eq!(ob_count, 1);
        assert!(addr.uri.to_string().contains(";ob"));
    }

    #[test]
    fn read_outbound_tolerates_missing_angle_brackets() {
        // Non-conforming peer omits angle brackets. We accept the raw URN.
        let mut addr = make_address();
        addr.params.push(Param::Other(
            "+sip.instance".to_string(),
            Some(GenericValue::Quoted(
                "urn:uuid:raw-without-brackets".to_string(),
            )),
        ));
        addr.params.push(Param::Other(
            "reg-id".to_string(),
            Some(GenericValue::Token("1".to_string())),
        ));
        let read = read_outbound_contact_params(&addr).unwrap();
        assert_eq!(read.instance_urn, "urn:uuid:raw-without-brackets");
    }

    #[test]
    fn read_outbound_rejects_non_numeric_reg_id() {
        let mut addr = make_address();
        addr.params.push(Param::Other(
            "+sip.instance".to_string(),
            Some(GenericValue::Quoted("<urn:uuid:x>".to_string())),
        ));
        addr.params.push(Param::Other(
            "reg-id".to_string(),
            Some(GenericValue::Token("not-a-number".to_string())),
        ));
        assert!(read_outbound_contact_params(&addr).is_none());
    }
}
