use std::str;

use crate::parser::ParseResult;
use crate::types::uri::Host;

// RFC 1034/1035 compliant hostname parser
// hostname = *( domainlabel "." ) toplabel [ "." ]
// domainlabel = alphanum / alphanum *( alphanum / "-" ) alphanum
// toplabel = ALPHA / ALPHA *( alphanum / "-" ) alphanum

// Parse a hostname including handling of trailing dot for FQDN
fn parse_hostname(input: &[u8]) -> ParseResult<'_, &[u8]> {
    // hostname    = *( domainlabel "." ) toplabel [ "." ]
    // domainlabel = alphanum / alphanum *( alphanum / "-" ) alphanum
    // toplabel    = ALPHA / ALPHA *( alphanum / "-" ) alphanum
    // (RFC 3261 §25.1). The hostname is the longest run of these characters;
    // whatever follows (port, parameters, whitespace, ...) is for the caller.
    let error = || nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag));
    let length = input
        .iter()
        .take_while(|&&c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
        .count();
    let candidate = &input[..length];
    let labels = candidate.strip_suffix(b".").unwrap_or(candidate);
    if labels.is_empty() {
        return Err(error());
    }
    let labels_valid = labels
        .split(|&c| c == b'.')
        .all(|label| !label.is_empty() && label[0] != b'-' && label[label.len() - 1] != b'-');
    let top_label_valid = labels
        .rsplit(|&c| c == b'.')
        .next()
        .is_some_and(|top| top[0].is_ascii_alphabetic());
    if !labels_valid || !top_label_valid {
        return Err(error());
    }
    Ok((&input[length..], candidate))
}

// Public hostname parser function
pub fn hostname(input: &[u8]) -> ParseResult<'_, Host> {
    // Handle the case where the input is a single dot
    if input == b"." {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    // Use the internal parser to handle the parsing
    let (remaining, host_bytes) = parse_hostname(input)?;

    // Convert the parsed bytes to a domain name
    let domain = match str::from_utf8(host_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Char,
            )))
        }
    };

    Ok((remaining, Host::Domain(domain)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostname_basic() {
        // Basic valid hostnames
        assert_eq!(
            hostname(b"example.com"),
            Ok((&[][..], Host::Domain("example.com".to_string())))
        );
        assert_eq!(
            hostname(b"host1.subdomain.example.co.uk"),
            Ok((
                &[][..],
                Host::Domain("host1.subdomain.example.co.uk".to_string())
            ))
        );
        assert_eq!(
            hostname(b"a-valid-host.net"),
            Ok((&[][..], Host::Domain("a-valid-host.net".to_string())))
        );
        assert_eq!(
            hostname(b"xn--ls8h.example"),
            Ok((&[][..], Host::Domain("xn--ls8h.example".to_string())))
        ); // IDN

        // Trailing dot (RFC 1035, FQDN format)
        assert_eq!(
            hostname(b"example.com."),
            Ok((&[][..], Host::Domain("example.com.".to_string())))
        );
    }

    #[test]
    fn test_hostname_invalid_basic() {
        // Invalid cases
        assert!(hostname(b"-invalid.start").is_err());
        assert!(hostname(b"invalid.end-").is_err());
        assert!(hostname(b"invalid..dot").is_err()); // Consecutive dots (empty label)
        assert!(hostname(b".").is_err()); // Just a dot
        assert!(hostname(b".invalid.start").is_err()); // Leading dot
    }

    #[test]
    fn test_hostname_rfc3261_compliance() {
        // RFC 3261 Section 25.1 (Core Syntax)
        // The hostname syntax follows RFC 1034/1035 with modifications for SIP

        // Valid examples from RFC 3261
        assert_eq!(
            hostname(b"atlanta.com"),
            Ok((&[][..], Host::Domain("atlanta.com".to_string())))
        );
        assert_eq!(
            hostname(b"biloxi.com"),
            Ok((&[][..], Host::Domain("biloxi.com".to_string())))
        );

        // Single-label hostnames are valid in RFC 3261
        assert_eq!(
            hostname(b"localhost"),
            Ok((&[][..], Host::Domain("localhost".to_string())))
        );

        // Domain labels can be up to 63 characters per RFC 1034
        let long_label = b"abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijk.com"; // 63 chars + .com
        assert_eq!(
            hostname(long_label),
            Ok((
                &[][..],
                Host::Domain(String::from_utf8_lossy(long_label).to_string())
            ))
        );
    }

    #[test]
    fn test_hostname_edge_cases() {
        // RFC 1034/1035 edge cases

        // Labels should only contain alphanumeric characters and hyphens
        assert!(nom::combinator::all_consuming(hostname)(b"invalid_underscore.com").is_err());

        // Modern DNS implementations sometimes allow underscores in certain contexts
        // SIP hostnames are more strict and don't allow this
        assert!(hostname(b"_sip._tcp.example.com").is_err());

        // Double dots (empty labels) are invalid
        assert!(hostname(b"example..com").is_err());
        assert!(hostname(b".example.com").is_err()); // Leading dot (empty first label)
    }

    #[test]
    fn test_hostname_domain_labels() {
        // RFC 1034/1035 domain label tests

        // Domain labels can't start or end with hyphens
        assert!(hostname(b"-invalid.com").is_err());
        assert!(hostname(b"invalid-.com").is_err());
        assert!(hostname(b"valid.com-").is_err());

        // Valid domain label with hyphens in the middle
        assert_eq!(
            hostname(b"this-is-valid.com"),
            Ok((&[][..], Host::Domain("this-is-valid.com".to_string())))
        );
    }

    #[test]
    fn test_hostname_rfc5890_idn() {
        // RFC 5890-5894 Internationalized Domain Names
        // IDNs in ASCII-compatible encoding (Punycode with xn-- prefix)

        // Valid IDNs in Punycode
        assert_eq!(
            hostname(b"xn--bcher-kva.example"),
            Ok((&[][..], Host::Domain("xn--bcher-kva.example".to_string())))
        ); // bücher.example
        assert_eq!(
            hostname(b"xn--caf-dma.example"),
            Ok((&[][..], Host::Domain("xn--caf-dma.example".to_string())))
        ); // café.example
    }

    #[test]
    fn test_hostname_not_ipv4() {
        // Dotted numbers that are not IPv4 addresses are not hostnames either:
        // the top label must start with ALPHA (RFC 3261 §25.1).

        assert!(
            hostname(b"999.888.777.666").is_err(),
            "toplabel must start with ALPHA"
        );
        assert!(
            hostname(b"192.168.1").is_err(),
            "toplabel must start with ALPHA"
        );
        assert!(
            hostname(b"192.168.1.").is_err(),
            "toplabel must start with ALPHA"
        );
    }

    #[test]
    fn test_hostname_numeric_labels() {
        // Numeric labels are valid except in the top label.
        assert!(
            hostname(b"987.13.55.44").is_err(),
            "toplabel must start with ALPHA"
        );
        assert_eq!(
            hostname(b"555.example.com"),
            Ok((&[][..], Host::Domain("555.example.com".to_string())))
        );
    }

    #[test]
    fn test_hostname_with_port_suffix() {
        // Test hostnames with port suffix to ensure proper parsing
        assert_eq!(
            hostname(b"example.com:5060"),
            Ok((&b":5060"[..], Host::Domain("example.com".to_string())))
        );
        assert_eq!(
            hostname(b"localhost:5060"),
            Ok((&b":5060"[..], Host::Domain("localhost".to_string())))
        );
    }

    #[test]
    fn test_hostname_fqdn_format() {
        // Test fully qualified domain names with trailing dots
        assert_eq!(
            hostname(b"example.com."),
            Ok((&[][..], Host::Domain("example.com.".to_string())))
        );
        assert_eq!(
            hostname(b"a.b.c.d.e.f."),
            Ok((&[][..], Host::Domain("a.b.c.d.e.f.".to_string())))
        );
    }

    #[test]
    fn test_hostname_with_semicolon() {
        // Test that semicolons are properly recognized as terminators
        let (rem, host) = hostname(b"example.com;transport=tcp").unwrap();
        assert_eq!(rem, b";transport=tcp");
        assert_eq!(host, Host::Domain("example.com".to_string()));

        // Test with both port and params
        let (rem, host) = hostname(b"example.com:5060;transport=tcp").unwrap();
        assert_eq!(rem, b":5060;transport=tcp");
        assert_eq!(host, Host::Domain("example.com".to_string()));

        // Test with multiple params
        let (rem, host) = hostname(b"example.com;transport=tcp;lr").unwrap();
        assert_eq!(rem, b";transport=tcp;lr");
        assert_eq!(host, Host::Domain("example.com".to_string()));

        // Test with question marks for headers
        let (rem, host) = hostname(b"example.com?Subject=Hello").unwrap();
        assert_eq!(rem, b"?Subject=Hello");
        assert_eq!(host, Host::Domain("example.com".to_string()));

        // Test with both param and headers
        let (rem, host) = hostname(b"example.com;transport=tcp?Subject=Hello").unwrap();
        assert_eq!(rem, b";transport=tcp?Subject=Hello");
        assert_eq!(host, Host::Domain("example.com".to_string()));
    }
}
