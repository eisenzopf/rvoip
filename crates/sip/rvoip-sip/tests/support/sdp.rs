use rvoip_sip_core::types::{ContentLength, ContentType, Response, TypedHeader};

/// Choose a distinct fixture media port without overflowing when an
/// ephemeral signaling socket is allocated near the top of the u16 range.
pub fn fixture_media_port(signaling_port: u16) -> u16 {
    const OFFSET: u16 = 1_000;
    if signaling_port <= u16::MAX - OFFSET {
        signaling_port + OFFSET
    } else {
        signaling_port - OFFSET
    }
}

/// Attach a minimal valid PCMU answer to a raw-UAS 200 OK fixture.
pub fn attach_pcmu_sdp_answer(response: &mut Response, media_port: u16) {
    response.body = format!(
        "v=0\r\no=test-uas 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n"
    )
    .into_bytes()
    .into();
    response
        .headers
        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
    response
        .headers
        .push(TypedHeader::ContentLength(ContentLength::new(
            response.body.len() as u32,
        )));
    response
        .headers
        .push(TypedHeader::ContentType(ContentType::sdp()));
}
