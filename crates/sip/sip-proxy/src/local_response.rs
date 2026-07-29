//! Local proxy response construction.
//!
//! A proxy forwarding a downstream response must preserve that response's
//! `To` tag. When the proxy itself generates a response, however, RFC 3261
//! section 8.2.6.2 requires every response other than `100 Trying` to carry a
//! `To` tag if the request did not already contain one.

use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::{Request, Response, StatusCode, TypedHeader};
use rvoip_sip_dialog::transaction::TransactionKey;
use sha2::{Digest, Sha256};

/// Build a response generated locally by the proxy.
///
/// `server_transaction_id` must identify the exact admitted server-transaction
/// generation, not merely a transaction key reconstructed from untrusted wire
/// data. Including that generation in the tag makes colliding requests owned by
/// different authenticated peers distinct while keeping retries of one exact
/// response stable.
///
/// An existing request `To` tag is copied unchanged, including on `100 Trying`.
/// An untagged request gains a deterministic tag for every response above
/// `100`; an untagged `100 Trying` remains untagged as required by RFC 3261
/// section 8.2.6.2.
pub fn local_response_from_request(
    request: &Request,
    server_transaction_id: &TransactionKey,
    status: StatusCode,
    reason: Option<&str>,
) -> Response {
    let mut response =
        SimpleResponseBuilder::response_from_request(request, status, reason).build();

    if let Some(request_to) = request.to().cloned() {
        let response_to = if status.as_u16() > 100 && request_to.tag().is_none() {
            let tag = stable_local_to_tag(request, server_transaction_id);
            request_to.with_tag(tag)
        } else {
            request_to
        };

        if let Some(to_index) = response
            .headers
            .iter()
            .position(|header| matches!(header, TypedHeader::To(_)))
        {
            response.headers[to_index] = TypedHeader::To(response_to);
        } else {
            response.headers.push(TypedHeader::To(response_to));
        }
    }

    response
}

fn stable_local_to_tag(request: &Request, server_transaction_id: &TransactionKey) -> String {
    fn part(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
        hasher.update((label.len() as u32).to_be_bytes());
        hasher.update(label);
        hasher.update((value.len() as u32).to_be_bytes());
        hasher.update(value);
    }

    let mut hasher = Sha256::new();
    part(
        &mut hasher,
        b"transaction-branch",
        server_transaction_id.branch.as_bytes(),
    );
    part(
        &mut hasher,
        b"transaction-method",
        server_transaction_id.method.to_string().as_bytes(),
    );
    part(
        &mut hasher,
        b"transaction-direction",
        if server_transaction_id.is_server {
            b"server"
        } else {
            b"client"
        },
    );
    if let Some(call_id) = request.call_id() {
        part(&mut hasher, b"call-id", call_id.to_string().as_bytes());
    }
    if let Some(from) = request.from() {
        part(&mut hasher, b"from", from.to_string().as_bytes());
    }
    if let Some(to) = request.to() {
        part(&mut hasher, b"to", to.to_string().as_bytes());
    }
    if let Some(cseq) = request.cseq() {
        part(&mut hasher, b"cseq", cseq.to_string().as_bytes());
    }

    let digest = hasher.finalize();
    // 128 bits is ample collision resistance for a SIP dialog identifier and
    // keeps the on-wire tag compact.
    let encoded = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("rvoip-proxy-{encoded}")
}

#[cfg(test)]
mod tests {
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::content_length::ContentLength;
    use rvoip_sip_core::types::param::Param;
    use rvoip_sip_core::types::via::Via;
    use rvoip_sip_core::types::TypedHeader;
    use rvoip_sip_core::Method;

    use super::*;

    fn request(to_tag: Option<&str>) -> Request {
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.net")
            .unwrap()
            .from("Alice", "sip:alice@example.org", Some("alice-tag"))
            .to("Bob", "sip:bob@example.net", to_tag)
            .call_id("local-response-tag")
            .cseq(42)
            .header(TypedHeader::Via(
                Via::new(
                    "SIP",
                    "2.0",
                    "UDP",
                    "edge.example.org",
                    Some(5060),
                    vec![Param::branch("z9hG4bK-wire-branch")],
                )
                .unwrap(),
            ))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }

    fn generation(branch: &str) -> TransactionKey {
        TransactionKey::new(branch.to_owned(), Method::Invite, true)
    }

    #[test]
    fn generated_tag_is_stable_for_one_exact_server_generation() {
        let request = request(None);
        let generation = generation("z9hG4bK-wire-branch~rvoip-server-7");

        let first = local_response_from_request(&request, &generation, StatusCode::NotFound, None);
        let replay = local_response_from_request(&request, &generation, StatusCode::NotFound, None);

        let first_tag = first.to_tag().expect("local final has a To tag");
        assert!(first_tag.starts_with("rvoip-proxy-"));
        assert_eq!(Some(first_tag), replay.to_tag());
    }

    #[test]
    fn colliding_wire_request_generations_get_distinct_tags() {
        let request = request(None);
        let first = local_response_from_request(
            &request,
            &generation("z9hG4bK-wire-branch~rvoip-server-7"),
            StatusCode::NotFound,
            None,
        );
        let second = local_response_from_request(
            &request,
            &generation("z9hG4bK-wire-branch~rvoip-server-8"),
            StatusCode::NotFound,
            None,
        );

        assert_ne!(first.to_tag(), second.to_tag());
    }

    #[test]
    fn trying_is_untagged_and_existing_tag_is_preserved() {
        let generation = generation("z9hG4bK-wire-branch");
        let untagged = request(None);
        let trying = local_response_from_request(&untagged, &generation, StatusCode::Trying, None);
        assert_eq!(trying.to_tag(), None);

        let tagged = request(Some("upstream-tag"));
        let tagged_trying =
            local_response_from_request(&tagged, &generation, StatusCode::Trying, None);
        assert_eq!(tagged_trying.to_tag(), Some("upstream-tag".to_owned()));

        let final_response =
            local_response_from_request(&tagged, &generation, StatusCode::NotFound, None);
        assert_eq!(final_response.to_tag(), Some("upstream-tag".to_owned()));
    }

    #[test]
    fn adding_tag_preserves_the_complete_to_value() {
        let mut request = request(None);
        let to_index = request
            .headers
            .iter()
            .position(|header| matches!(header, TypedHeader::To(_)))
            .expect("request has To");
        let TypedHeader::To(mut original_to) = request.headers[to_index].clone() else {
            unreachable!("the selected request header is To");
        };
        original_to.0.uri.parameters.push(Param::transport("tcp"));
        original_to.0.params.push(Param::Other(
            "x-dialog-param".to_owned(),
            Some("preserve-me".into()),
        ));
        request.headers[to_index] = TypedHeader::To(original_to.clone());

        let response = local_response_from_request(
            &request,
            &generation("z9hG4bK-wire-branch~rvoip-server-7"),
            StatusCode::NotFound,
            None,
        );
        let response_to = response
            .headers
            .iter()
            .find_map(|header| match header {
                TypedHeader::To(to) => Some(to),
                _ => None,
            })
            .expect("response has To");
        let expected = original_to.with_tag(
            response_to
                .tag()
                .expect("locally generated response has a tag"),
        );

        assert_eq!(response_to, &expected);
    }
}
