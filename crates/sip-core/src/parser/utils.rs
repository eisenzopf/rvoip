// Utility functions for parsing

/// Unfolds Linear White Space (LWS) according to RFC 3261.
/// Replaces CRLF followed immediately by WSP (SP/HTAB) with a single SP.
/// Also compresses consecutive non-folding WSP into a single SP.
/// Returns a new Vec<u8> with the unfolded bytes.
pub fn unfold_lws(input: &[u8]) -> Vec<u8> {
    let mut unfolded = Vec::with_capacity(input.len());
    let mut i = 0;
    let len = input.len();
    let mut last_was_wsp = false;

    while i < len {
        // Check for CRLF
        if input[i] == b'\r' && i + 1 < len && input[i+1] == b'\n' {
            // Check if followed by WSP
            if i + 2 < len && (input[i+2] == b' ' || input[i+2] == b'\t') {
                // It's folding LWS: skip CRLF, process subsequent WSP as a single SP
                i += 2; // Skip CR LF
                if !last_was_wsp { // Only add SP if not already preceded by WSP
                    unfolded.push(b' ');
                    last_was_wsp = true;
                }
                // Skip all subsequent WSP characters in this folding sequence
                while i < len && (input[i] == b' ' || input[i] == b'\t') {
                    i += 1;
                }
            } else {
                // Not folding LWS, treat CRLF as normal characters (e.g., part of quoted string content)
                // Or potentially an error depending on context, but unfold just passes them through.
                unfolded.push(input[i]);
                unfolded.push(input[i+1]);
                i += 2;
                last_was_wsp = false;
            }
        } 
        // Check for standalone LF (more lenient)
        else if input[i] == b'\n' {
             // Check if followed by WSP
             if i + 1 < len && (input[i+1] == b' ' || input[i+1] == b'\t') {
                 // Folding LWS
                 i += 1; // Skip LF
                 if !last_was_wsp {
                     unfolded.push(b' ');
                     last_was_wsp = true;
                 }
                 while i < len && (input[i] == b' ' || input[i] == b'\t') {
                     i += 1;
                 }
             } else {
                 // Normal LF
                 unfolded.push(input[i]);
                 i += 1;
                 last_was_wsp = false;
             }
        }
        // Check for WSP (SP or HTAB)
        else if input[i] == b' ' || input[i] == b'\t' {
            // If the last character added wasn't WSP, add a single space
            if !last_was_wsp {
                unfolded.push(b' ');
                last_was_wsp = true;
            }
            // Skip consecutive WSP
            while i < len && (input[i] == b' ' || input[i] == b'\t') {
                i += 1;
            }
        } 
        // Other character
        else {
            unfolded.push(input[i]);
            i += 1;
            last_was_wsp = false;
        }
    }

    unfolded
}

/// Decodes URI percent-encoding (%HH) within a byte slice.
/// Returns a Result<String, Error>.
pub fn unescape_uri_component(input: &[u8]) -> crate::error::Result<String> {
    let mut unescaped: Vec<u8> = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        match input[i] {
            b'%' => {
                if i + 2 < input.len() {
                    let h1 = input[i + 1];
                    let h2 = input[i + 2];
                    if let (Some(v1), Some(v2)) = (hex_val(h1), hex_val(h2)) {
                        unescaped.push((v1 << 4) | v2);
                        i += 3;
                    } else {
                        // Invalid hex digits after %
                        return Err(crate::error::Error::ParseError(
                            format!("Invalid hex sequence: %{}{}", h1 as char, h2 as char)
                        ));
                    }
                } else {
                    // Incomplete escape sequence
                    return Err(crate::error::Error::ParseError(
                        "Incomplete escape sequence at end of input".to_string()
                    ));
                }
            }
            _ => {
                unescaped.push(input[i]);
                i += 1;
            }
        }
    }

    String::from_utf8(unescaped).map_err(|e| crate::error::Error::ParseError(
        format!("UTF-8 error after URI unescaping: {}", e)
    ))
}

// Helper to convert a hex character (byte) to its value (0-15)
fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unfold_lws_simple() {
        assert_eq!(unfold_lws(b"Value"), b"Value");
        assert_eq!(unfold_lws(b"Two Words"), b"Two Words");
        assert_eq!(unfold_lws(b"Many   Spaces Between"), b"Many Spaces Between");
        assert_eq!(unfold_lws(b"\tTabs\t Too\t"), b" Tabs Too "); // Note: leading/trailing handled
    }

    #[test]
    fn test_unfold_lws_folding() {
        assert_eq!(unfold_lws(b"Line 1\r\n Line 2"), b"Line 1 Line 2");
        assert_eq!(unfold_lws(b"Line 1\r\n\tLine 2"), b"Line 1 Line 2");
        assert_eq!(unfold_lws(b"Line 1 \r\n Line 2"), b"Line 1 Line 2"); // Space before fold
        assert_eq!(unfold_lws(b"Line 1\r\n  \t Line 2"), b"Line 1 Line 2"); // Multiple WSP after fold
        assert_eq!(unfold_lws(b"Line 1\r\nLine 2"), b"Line 1\r\nLine 2"); // No WSP after CRLF (not folding)
    }

    #[test]
    fn test_unfold_lws_mixed() {
        assert_eq!(unfold_lws(b"Folded\r\n  Here and   also \t here."), b"Folded Here and also here.");
        assert_eq!(unfold_lws(b" Leading\r\n space\r\n\tand trailing \t"), b" Leading space and trailing ");
    }
    
     #[test]
    fn test_unfold_lenient_lf() {
        assert_eq!(unfold_lws(b"Line 1\n Line 2"), b"Line 1 Line 2");
        assert_eq!(unfold_lws(b"Line 1\n\tLine 2"), b"Line 1 Line 2");
        assert_eq!(unfold_lws(b"Line 1\nLine 2"), b"Line 1\nLine 2"); // Not folding
    }

    #[test]
    fn test_unescape_uri_component() {
        assert_eq!(unescape_uri_component(b"simple").unwrap(), "simple");
        assert_eq!(unescape_uri_component(b"%20").unwrap(), " ");
        assert_eq!(unescape_uri_component(b"a%20b%20c").unwrap(), "a b c");
        assert_eq!(unescape_uri_component(b"%41%42%43").unwrap(), "ABC");
        assert_eq!(unescape_uri_component(b"%c3%a9").unwrap(), "é"); // UTF-8
        assert_eq!(unescape_uri_component(b"%25").unwrap(), "%"); // Escaped percent
    }

    #[test]
    fn test_unescape_uri_component_invalid() {
        assert!(unescape_uri_component(b"%").is_err()); // Incomplete
        assert!(unescape_uri_component(b"%2").is_err()); // Incomplete
        assert!(unescape_uri_component(b"%G0").is_err()); // Invalid hex
        assert!(unescape_uri_component(b"%2G").is_err()); // Invalid hex
        assert!(unescape_uri_component(b"%AF%").is_err()); // Incomplete at end
        // Test invalid UTF-8 after decoding (e.g., %C0%80)
        assert!(unescape_uri_component(b"%C0%80").is_err()); 
    }
} 