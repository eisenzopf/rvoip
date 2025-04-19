// Tests for the Param enum and related logic

use rvoip_sip_core::types::Param;
use std::net::IpAddr;
use std::str::FromStr;

#[test]
fn test_param_display() {
    /// RFC 3261 Section 20.42 Via Parameters (branch)
    assert_eq!(Param::Branch("z9hG4bK123".to_string()).to_string(), ";branch=z9hG4bK123");
    
    /// RFC 3261 Section 20.20 From Parameters (tag)
    assert_eq!(Param::Tag("abc-def".to_string()).to_string(), ";tag=abc-def");
    
    /// RFC 3261 Section 20.10 Contact Parameters (expires)
    assert_eq!(Param::Expires(3600).to_string(), ";expires=3600");
    
    /// RFC 3261 Section 20.42 Via Parameters (received)
    let ip_rec: IpAddr = FromStr::from_str("192.0.2.1").unwrap();
    assert_eq!(Param::Received(ip_rec).to_string(), ";received=192.0.2.1");
    
    /// RFC 3261 Section 20.42 Via Parameters (maddr)
    assert_eq!(Param::Maddr("224.2.0.1".to_string()).to_string(), ";maddr=224.2.0.1");
    
    /// RFC 3261 Section 20.42 Via Parameters (ttl)
    assert_eq!(Param::Ttl(64).to_string(), ";ttl=64");
    
    /// RFC 3261 Section 20.42 Via Parameters (lr)
    assert_eq!(Param::Lr.to_string(), ";lr");
    
    /// RFC 3261 Section 20.10 Contact Parameters (q)
    assert_eq!(Param::Q(0.8).to_string(), ";q=0.8");
    assert_eq!(Param::Q(1.0).to_string(), ";q=1.0");
    assert_eq!(Param::Q(0.123).to_string(), ";q=0.1"); // Check formatting precision
    
    /// RFC 3261 Section 19.1.4 URI Parameters (transport)
    assert_eq!(Param::Transport("tcp".to_string()).to_string(), ";transport=tcp");
    
    /// RFC 3261 Section 19.1.4 URI Parameters (user)
    assert_eq!(Param::User("phone".to_string()).to_string(), ";user=phone");
    
    /// RFC 3261 Section 19.1.4 URI Parameters (method)
    assert_eq!(Param::Method("REGISTER".to_string()).to_string(), ";method=REGISTER");
    
    /// Generic parameter with value
    assert_eq!(Param::Other("foo".to_string(), Some("bar".to_string())).to_string(), ";foo=bar");
    
    /// Generic parameter without value (flag)
    assert_eq!(Param::Other("baz".to_string(), None).to_string(), ";baz");
} 