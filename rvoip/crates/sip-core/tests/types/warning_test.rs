// Tests for Warning type

use crate::common::{uri, assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::Warning;
use rvoip_sip_core::uri::Uri;
use std::str::FromStr;

#[test]
fn test_warning_display_parse_roundtrip() {
    let warn1 = Warning {
        code: 307, 
        agent: uri("sip:isi.edu"), // Use helper
        text: "Session parameter 'foo' not understood".to_string()
    };
    assert_display_parses_back(&warn1);

    let warn2 = Warning {
        code: 301, 
        agent: uri("sip:example.com"), 
        text: "Redirected".to_string()
    };
    assert_display_parses_back(&warn2);
    
    // Test FromStr directly
    assert_parses_ok("307 isi.edu \"Session parameter 'foo' not understood\"", warn1);
    assert_parses_ok("301 example.com \"Redirected\"", warn2);

    // Test failure
    assert_parse_fails::<Warning>("307 isi.edu NoQuotes");
    assert_parse_fails::<Warning>("badcode isi.edu \"Text\"");
    assert_parse_fails::<Warning>("307\"Text\""); // Missing agent
}

// Removed old separate tests 