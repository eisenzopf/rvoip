//! SHAKEN-specific leaf certificate profile checks.
//!
//! After `webpki` has validated the cert chain, the leaf still has
//! to be checked against the SHAKEN profile (ATIS-1000080, building
//! on RFC 8226). Two extensions matter:
//!
//! - **TNAuthList** (`id-pe-TNAuthList`, OID `1.3.6.1.5.5.7.1.26`,
//!   RFC 8226 §9) — declares which telephone numbers / service
//!   provider codes (SPCs) the cert is authorised to assert. SHAKEN
//!   leaf certs MUST carry one with at least one SPC; the PASSporT
//!   `origid` (when present and an SPC-shaped string) must appear.
//!
//! - **JWT Claim Constraints** (`id-pe-JWTClaimConstraints`, OID
//!   `1.3.6.1.5.5.7.1.27`, RFC 8226 §10) — constrains the values of
//!   JWT claims signed under this cert. SHAKEN deployments
//!   typically restrict `attest` to a subset of `["A", "B", "C"]`.
//!   When present, the PASSporT's `attest` must be in the permitted
//!   list; absent extension means no constraint.
//!
//! The decoders here are hand-rolled DER walks — no new dependency.

/// `1.3.6.1.5.5.7.1.26` (string form).
pub const TN_AUTH_LIST_OID: &str = "1.3.6.1.5.5.7.1.26";

/// `1.3.6.1.5.5.7.1.27` (string form).
pub const JWT_CLAIM_CONSTRAINTS_OID: &str = "1.3.6.1.5.5.7.1.27";

/// Parsed contents of a TNAuthList extension.
#[derive(Debug, Clone, Default)]
pub struct TNAuthList {
    /// Service Provider Codes (SHAKEN leaf certs carry at least one).
    pub spcs: Vec<String>,
    /// Individual telephone numbers asserted by the cert.
    pub tns: Vec<String>,
    /// `(start, count)` telephone-number ranges asserted by the cert.
    pub tn_ranges: Vec<(String, u64)>,
}

impl TNAuthList {
    /// True if the list contains no SPC / TN / range entries —
    /// SHAKEN requires at least one SPC.
    pub fn is_empty(&self) -> bool {
        self.spcs.is_empty() && self.tns.is_empty() && self.tn_ranges.is_empty()
    }
}

/// Parsed contents of a JWT Claim Constraints extension. Only the
/// `permittedValues` arm is captured — `mustInclude` is not used by
/// SHAKEN base profile and is parsed-then-skipped.
#[derive(Debug, Clone, Default)]
pub struct JwtClaimConstraints {
    /// `claim name -> permitted UTF-8 string values`.
    pub permitted: Vec<(String, Vec<String>)>,
}

impl JwtClaimConstraints {
    /// True if the constraint set is empty (no permittedValues entries).
    pub fn is_empty(&self) -> bool {
        self.permitted.is_empty()
    }

    /// Look up the permitted-value list for a specific claim name.
    pub fn permitted_for(&self, claim: &str) -> Option<&[String]> {
        self.permitted
            .iter()
            .find(|(name, _)| name == claim)
            .map(|(_, vs)| vs.as_slice())
    }
}

/// Parse a TNAuthList extension's `extnValue` body (the OCTET STRING
/// contents — the outer SEQUENCE of TNEntry).
///
/// ```text
/// TNAuthorizationList ::= SEQUENCE SIZE (1..MAX) OF TNEntry
/// TNEntry ::= CHOICE {
///     spc       [0] IA5String,                       -- 0x80
///     range     [1] SEQUENCE { start, count },       -- 0xA1
///     one       [2] IA5String                        -- 0x82
/// }
/// ```
pub fn parse_tnauth_list(body: &[u8]) -> Result<TNAuthList, String> {
    let (tag, contents, _rest) =
        read_tlv(body).ok_or_else(|| "TNAuthList: outer TLV truncated".to_string())?;
    if tag != 0x30 {
        return Err(format!(
            "TNAuthList: expected outer SEQUENCE (0x30), got {:#x}",
            tag
        ));
    }

    let mut out = TNAuthList::default();
    let mut cur = contents;
    while !cur.is_empty() {
        let (entry_tag, entry_body, rest) =
            read_tlv(cur).ok_or_else(|| "TNAuthList: entry TLV truncated".to_string())?;
        match entry_tag {
            0x80 => {
                let s = std::str::from_utf8(entry_body)
                    .map_err(|_| "TNAuthList: SPC not valid UTF-8".to_string())?;
                out.spcs.push(s.to_string());
            }
            0x82 => {
                let s = std::str::from_utf8(entry_body)
                    .map_err(|_| "TNAuthList: TN not valid UTF-8".to_string())?;
                out.tns.push(s.to_string());
            }
            0xA1 => {
                // SEQUENCE { start IA5String, count INTEGER }
                let (start_tag, start_body, after_start) = read_tlv(entry_body)
                    .ok_or_else(|| "TNAuthList: range start TLV truncated".to_string())?;
                // start is plain IA5String (no implicit tag inside the SEQUENCE)
                if start_tag != 0x16 {
                    return Err(format!(
                        "TNAuthList: range start expected IA5String (0x16), got {:#x}",
                        start_tag
                    ));
                }
                let start = std::str::from_utf8(start_body)
                    .map_err(|_| "TNAuthList: range start not UTF-8".to_string())?
                    .to_string();
                let (count_tag, count_body, _after_count) = read_tlv(after_start)
                    .ok_or_else(|| "TNAuthList: range count TLV truncated".to_string())?;
                if count_tag != 0x02 {
                    return Err(format!(
                        "TNAuthList: range count expected INTEGER (0x02), got {:#x}",
                        count_tag
                    ));
                }
                let count = read_unsigned_integer(count_body)
                    .ok_or_else(|| "TNAuthList: range count out of range".to_string())?;
                out.tn_ranges.push((start, count));
            }
            other => {
                return Err(format!("TNAuthList: unknown TNEntry tag {:#x}", other));
            }
        }
        cur = rest;
    }

    Ok(out)
}

/// Parse a JWT Claim Constraints extension's `extnValue` body.
///
/// ```text
/// JWTClaimConstraints ::= SEQUENCE {
///     mustInclude     [0] SEQUENCE OF JWTClaimName OPTIONAL,    -- 0xA0
///     permittedValues [1] SEQUENCE OF JWTClaimValuesList OPTIONAL  -- 0xA1
/// }
/// JWTClaimValuesList ::= SEQUENCE {
///     claim  IA5String,                                  -- 0x16
///     values SEQUENCE OF UTF8String                      -- 0x30 of 0x0C
/// }
/// ```
pub fn parse_jwt_claim_constraints(body: &[u8]) -> Result<JwtClaimConstraints, String> {
    let (tag, contents, _rest) =
        read_tlv(body).ok_or_else(|| "JWTClaimConstraints: outer TLV truncated".to_string())?;
    if tag != 0x30 {
        return Err(format!(
            "JWTClaimConstraints: expected outer SEQUENCE (0x30), got {:#x}",
            tag
        ));
    }

    let mut out = JwtClaimConstraints::default();
    let mut cur = contents;
    while !cur.is_empty() {
        let (entry_tag, entry_body, rest) =
            read_tlv(cur).ok_or_else(|| "JWTClaimConstraints: entry TLV truncated".to_string())?;
        match entry_tag {
            0xA0 => {
                // mustInclude — RFC 8226 lists claim names; SHAKEN doesn't
                // use this. Parse-and-skip so a leaf carrying both fields
                // still validates.
            }
            0xA1 => {
                // permittedValues — SEQUENCE OF JWTClaimValuesList
                let mut inner = entry_body;
                while !inner.is_empty() {
                    let (vt, vb, vr) = read_tlv(inner).ok_or_else(|| {
                        "JWTClaimConstraints: permittedValues entry TLV truncated".to_string()
                    })?;
                    if vt != 0x30 {
                        return Err(format!(
                            "JWTClaimConstraints: permittedValues entry expected SEQUENCE (0x30), got {:#x}",
                            vt
                        ));
                    }
                    let (name_tag, name_body, after_name) = read_tlv(vb).ok_or_else(|| {
                        "JWTClaimConstraints: claim name TLV truncated".to_string()
                    })?;
                    if name_tag != 0x16 {
                        return Err(format!(
                            "JWTClaimConstraints: claim name expected IA5String (0x16), got {:#x}",
                            name_tag
                        ));
                    }
                    let name = std::str::from_utf8(name_body)
                        .map_err(|_| "JWTClaimConstraints: claim name not UTF-8".to_string())?
                        .to_string();

                    let (values_tag, values_body, _) = read_tlv(after_name).ok_or_else(|| {
                        "JWTClaimConstraints: values SEQUENCE TLV truncated".to_string()
                    })?;
                    if values_tag != 0x30 {
                        return Err(format!(
                            "JWTClaimConstraints: values expected SEQUENCE OF (0x30), got {:#x}",
                            values_tag
                        ));
                    }
                    let mut vbytes = values_body;
                    let mut values = Vec::new();
                    while !vbytes.is_empty() {
                        let (val_tag, val_body, val_rest) = read_tlv(vbytes).ok_or_else(|| {
                            "JWTClaimConstraints: value TLV truncated".to_string()
                        })?;
                        if val_tag != 0x0C {
                            return Err(format!(
                                "JWTClaimConstraints: value expected UTF8String (0x0C), got {:#x}",
                                val_tag
                            ));
                        }
                        let v = std::str::from_utf8(val_body)
                            .map_err(|_| "JWTClaimConstraints: value not UTF-8".to_string())?
                            .to_string();
                        values.push(v);
                        vbytes = val_rest;
                    }
                    out.permitted.push((name, values));
                    inner = vr;
                }
            }
            other => {
                return Err(format!(
                    "JWTClaimConstraints: unknown field tag {:#x}",
                    other
                ));
            }
        }
        cur = rest;
    }
    Ok(out)
}

/// Read one DER TLV from `bytes`. Returns `(tag, contents, rest)`
/// where `contents` is the value body (length already consumed) and
/// `rest` is everything after the TLV. Supports short-form and
/// long-form lengths up to 4 length bytes (≤ 4 GiB cert, far above
/// the 256 KB resolver cap).
fn read_tlv(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    if bytes.len() < 2 {
        return None;
    }
    let tag = bytes[0];
    let first_len = bytes[1];
    let (len, header_len) = if first_len & 0x80 == 0 {
        (first_len as usize, 2)
    } else {
        let n = (first_len & 0x7F) as usize;
        if n == 0 || n > 4 || bytes.len() < 2 + n {
            return None;
        }
        let mut acc: usize = 0;
        for &b in &bytes[2..2 + n] {
            acc = acc.checked_shl(8)?.checked_add(b as usize)?;
        }
        (acc, 2 + n)
    };
    if bytes.len() < header_len + len {
        return None;
    }
    Some((
        tag,
        &bytes[header_len..header_len + len],
        &bytes[header_len + len..],
    ))
}

/// Decode a DER INTEGER value into a `u64`. Returns `None` if the
/// value is negative or overflows.
fn read_unsigned_integer(body: &[u8]) -> Option<u64> {
    if body.is_empty() || body.len() > 9 {
        return None;
    }
    // Skip leading zero used to mark positive values.
    let bytes = if body.len() > 1 && body[0] == 0x00 {
        &body[1..]
    } else {
        body
    };
    if bytes.len() > 8 {
        return None;
    }
    let mut acc: u64 = 0;
    for &b in bytes {
        acc = acc.checked_shl(8)?.checked_add(b as u64)?;
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a DER TLV with a short-form length.
    fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let len = body.len();
        if len < 0x80 {
            out.push(len as u8);
        } else if len < 0x100 {
            out.push(0x81);
            out.push(len as u8);
        } else if len < 0x10000 {
            out.push(0x82);
            out.push((len >> 8) as u8);
            out.push((len & 0xFF) as u8);
        } else {
            panic!("test TLV too large");
        }
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn tnauth_with_single_spc_decodes() {
        // outer SEQUENCE of one SPC entry "1234"
        let spc_entry = tlv(0x80, b"1234");
        let outer = tlv(0x30, &spc_entry);
        let parsed = parse_tnauth_list(&outer).expect("parse");
        assert_eq!(parsed.spcs, vec!["1234".to_string()]);
        assert!(parsed.tns.is_empty());
        assert!(parsed.tn_ranges.is_empty());
    }

    #[test]
    fn tnauth_with_tn_and_range() {
        let tn_entry = tlv(0x82, b"+15551234567");
        let start = tlv(0x16, b"+15558000000");
        let count = tlv(0x02, &[0x03, 0xE8]); // 1000
        let range_body = [&start[..], &count[..]].concat();
        let range_entry = tlv(0xA1, &range_body);
        let outer_body = [&tn_entry[..], &range_entry[..]].concat();
        let outer = tlv(0x30, &outer_body);
        let parsed = parse_tnauth_list(&outer).expect("parse");
        assert_eq!(parsed.tns, vec!["+15551234567".to_string()]);
        assert_eq!(parsed.tn_ranges, vec![("+15558000000".to_string(), 1000)]);
        assert!(parsed.spcs.is_empty());
    }

    #[test]
    fn tnauth_rejects_unknown_tag() {
        let bogus = tlv(0x83, b"oops");
        let outer = tlv(0x30, &bogus);
        let err = parse_tnauth_list(&outer).unwrap_err();
        assert!(err.contains("unknown TNEntry"));
    }

    #[test]
    fn tnauth_rejects_non_sequence_outer() {
        let outer = tlv(0x04, b"abc"); // OCTET STRING — wrong outer tag
        let err = parse_tnauth_list(&outer).unwrap_err();
        assert!(err.contains("outer SEQUENCE"));
    }

    #[test]
    fn jcc_attest_permitted_values_decodes() {
        // permittedValues = [ { "attest", ["A", "B"] } ]
        let val_a = tlv(0x0C, b"A");
        let val_b = tlv(0x0C, b"B");
        let values_seq_body = [&val_a[..], &val_b[..]].concat();
        let values_seq = tlv(0x30, &values_seq_body);
        let claim_name = tlv(0x16, b"attest");
        let entry_body = [&claim_name[..], &values_seq[..]].concat();
        let entry = tlv(0x30, &entry_body);
        let permitted = tlv(0xA1, &entry);
        let outer = tlv(0x30, &permitted);

        let parsed = parse_jwt_claim_constraints(&outer).expect("parse");
        assert_eq!(parsed.permitted.len(), 1);
        assert_eq!(parsed.permitted[0].0, "attest");
        assert_eq!(
            parsed.permitted[0].1,
            vec!["A".to_string(), "B".to_string()]
        );
        assert_eq!(
            parsed.permitted_for("attest"),
            Some(&["A".to_string(), "B".to_string()][..])
        );
    }

    #[test]
    fn jcc_with_must_include_skips_it() {
        // mustInclude = [ "iat" ]; permittedValues absent
        let mi_name = tlv(0x16, b"iat");
        let must_include = tlv(0xA0, &mi_name);
        let outer = tlv(0x30, &must_include);
        let parsed = parse_jwt_claim_constraints(&outer).expect("parse");
        assert!(parsed.is_empty());
    }
}
