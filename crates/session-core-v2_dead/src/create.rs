/// Generate SDP offer (placeholder for API compatibility)
pub fn generate_sdp_offer(_local_addr: &str, _remote_addr: &str) -> String {
    "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\n".to_string()
}

/// Generate SDP answer (placeholder for API compatibility)
pub fn generate_sdp_answer(_offer: &str, _local_addr: &str) -> String {
    "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5006 RTP/AVP 0\r\n".to_string()
}