use rvoip_sip_core::prelude::*;
use rvoip_sip_dialog::transaction::utils::response_builders::{
    create_response, create_response_for_transaction_generation,
};
use rvoip_sip_dialog::TransactionKey;

fn request(branch: &str, call_id: &str, to: To) -> Request {
    let mut from_address =
        Address::new_with_display_name("Alice", Uri::sip("example.com").with_user("alice"));
    from_address.set_tag("alice-tag");

    RequestBuilder::new(Method::Invite, "sip:bob@example.net")
        .expect("valid request URI")
        .header(TypedHeader::Via(
            Via::new(
                "SIP",
                "2.0",
                "UDP",
                "client.example.com",
                Some(5060),
                vec![Param::branch(branch)],
            )
            .expect("valid Via"),
        ))
        .header(TypedHeader::From(From::new(from_address)))
        .header(TypedHeader::To(to))
        .header(TypedHeader::CallId(CallId::new(call_id)))
        .header(TypedHeader::CSeq(CSeq::new(42, Method::Invite)))
        .build()
}

fn to_header(tag: Option<&str>) -> To {
    let mut address =
        Address::new_with_display_name("Bob B.", Uri::sip("example.net").with_user("bob"));
    address
        .params
        .push(Param::Other("x-route".to_string(), Some("west".into())));
    if let Some(tag) = tag {
        address.set_tag(tag);
    }
    To::new(address)
}

fn untagged_request(branch: &str, call_id: &str) -> Request {
    request(branch, call_id, to_header(None))
}

fn response_to(response: &Response) -> &To {
    response
        .typed_header::<To>()
        .expect("response must contain To")
}

#[test]
fn untagged_trying_remains_untagged_but_pretagged_trying_is_preserved() {
    let untagged = untagged_request("z9hG4bK-trying-untagged", "trying-untagged");
    assert!(response_to(&create_response(&untagged, StatusCode::Trying))
        .tag()
        .is_none());

    let tagged_to = to_header(Some("existing"));
    let tagged = request("z9hG4bK-trying-tagged", "trying-tagged", tagged_to.clone());
    assert_eq!(
        response_to(&create_response(&tagged, StatusCode::Trying)),
        &tagged_to
    );
}

#[test]
fn every_local_response_after_trying_has_a_stable_tag() {
    let request = untagged_request("z9hG4bK-stable", "stable-call");
    let ringing = create_response(&request, StatusCode::Ringing);
    let denied = create_response(&request, StatusCode::Unauthorized);
    let first_tag = response_to(&ringing).tag().expect("Ringing To tag");

    assert_eq!(
        response_to(&denied).tag(),
        Some(first_tag),
        "different responses in the same transaction must reuse one local tag"
    );
    assert!(first_tag.starts_with("rvoip-"));
    assert_eq!(first_tag.len(), "rvoip-".len() + 32);
}

#[test]
fn existing_to_is_copied_in_full_without_retagging() {
    let tagged_to = to_header(Some("existing"));
    let request = request("z9hG4bK-existing", "existing-call", tagged_to.clone());

    for status in [
        StatusCode::Trying,
        StatusCode::Ringing,
        StatusCode::Forbidden,
    ] {
        assert_eq!(
            response_to(&create_response(&request, status)),
            &tagged_to,
            "status {status} changed the supplied To header"
        );
    }
}

#[test]
fn materially_distinct_wire_transactions_receive_distinct_tags() {
    let first = untagged_request("z9hG4bK-first", "shared-call");
    let second = untagged_request("z9hG4bK-second", "shared-call");

    assert_ne!(
        response_to(&create_response(&first, StatusCode::Forbidden)).tag(),
        response_to(&create_response(&second, StatusCode::Forbidden)).tag()
    );
}

#[test]
fn exact_transaction_generation_is_stable_and_collision_safe() {
    let request = untagged_request("z9hG4bK-wire-collision", "collision-call");
    let first_key = TransactionKey::new("z9hG4bK-wire-collision".to_string(), Method::Invite, true);
    let collision_key = TransactionKey::new(
        "z9hG4bK-wire-collision~rvoip-server-2".to_string(),
        Method::Invite,
        true,
    );

    let first = create_response_for_transaction_generation(
        &request,
        StatusCode::Unauthorized,
        &first_key,
        7,
    );
    let repeat =
        create_response_for_transaction_generation(&request, StatusCode::Forbidden, &first_key, 7);
    let replacement = create_response_for_transaction_generation(
        &request,
        StatusCode::Unauthorized,
        &first_key,
        8,
    );
    let collision = create_response_for_transaction_generation(
        &request,
        StatusCode::Unauthorized,
        &collision_key,
        7,
    );

    assert_eq!(response_to(&first).tag(), response_to(&repeat).tag());
    assert_ne!(response_to(&first).tag(), response_to(&replacement).tag());
    assert_ne!(response_to(&first).tag(), response_to(&collision).tag());
}

#[test]
fn every_via_header_line_is_preserved_in_wire_order() {
    let mut request = untagged_request("z9hG4bK-top", "multi-via-call");
    request.headers.push(TypedHeader::Via(
        Via::new(
            "SIP",
            "2.0",
            "TCP",
            "edge.example.net",
            Some(5070),
            vec![Param::branch("z9hG4bK-edge")],
        )
        .expect("valid second Via"),
    ));

    let response = create_response(&request, StatusCode::Unauthorized);
    let request_vias: Vec<_> = request
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::Via(_)))
        .collect();
    let response_vias: Vec<_> = response
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::Via(_)))
        .collect();

    assert_eq!(response_vias, request_vias);
    assert_eq!(response_vias.len(), 2);
}
