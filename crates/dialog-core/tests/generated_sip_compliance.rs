use std::net::SocketAddr;

use rvoip_dialog_core::transaction::client::builders::{InviteBuilder, RegisterBuilder};
use rvoip_dialog_core::transaction::dialog::{
    bye_for_dialog, info_for_dialog, message_for_dialog, message_out_of_dialog, notify_for_dialog,
    prack_for_dialog, refer_for_dialog, reinvite_for_dialog, response_for_dialog_transaction,
    subscribe_out_of_dialog, update_for_dialog,
};
use rvoip_dialog_core::transaction::method::ack::{
    create_ack_for_2xx, create_ack_for_error_response,
};
use rvoip_dialog_core::transaction::method::cancel::create_cancel_request;
use rvoip_dialog_core::transaction::server::builders::{InviteResponseBuilder, ResponseBuilder};
use rvoip_dialog_core::transaction::utils::request_builders::create_ack_from_invite;
use rvoip_dialog_core::transaction::utils::response_builders::{
    create_ok_response, create_ok_response_for_bye, create_ok_response_for_cancel,
    create_ok_response_for_options, create_response, create_ringing_response,
    create_trying_response,
};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::types::{
    outbound::{mark_uri_as_outbound, set_outbound_contact_params, OutboundContactParams},
    route::Route,
    Address, Contact, ContactParamInfo, HeaderName, Method, StatusCode, TypedHeader, Uri,
};
use rvoip_sip_core::validation::{validate_generated_request, validate_generated_response};

const SDP: &str = "v=0\r\no=alice 1 1 IN IP4 127.0.0.1\r\ns=-\r\n";

fn local_addr() -> SocketAddr {
    "127.0.0.1:5060".parse().unwrap()
}

fn invite_request() -> rvoip_sip_core::Request {
    InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr())
        .contact("sip:alice@127.0.0.1:5060")
        .call_id("dialog-invite-call")
        .cseq(42)
        .with_sdp(SDP)
        .build()
        .unwrap()
}

fn established_args() -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u32,
    SocketAddr,
) {
    (
        "dialog-call",
        "sip:alice@example.com",
        "alice-tag",
        "sip:bob@example.com",
        "bob-tag",
        2,
        local_addr(),
    )
}

fn outbound_contact() -> Contact {
    let mut address = Address::new("sip:alice@127.0.0.1:5060".parse::<Uri>().unwrap());
    mark_uri_as_outbound(&mut address);
    set_outbound_contact_params(
        &mut address,
        &OutboundContactParams {
            instance_urn: "urn:uuid:00000000-0000-0000-0000-000000000001".to_string(),
            reg_id: 1,
        },
    );
    Contact::new_params(vec![ContactParamInfo { address }])
}

#[test]
fn generated_sip_compliance_dialog_client_builders_generate_valid_requests() {
    let invite = invite_request();
    let parsed_invite = validate_generated_request(&invite).unwrap();
    assert_eq!(parsed_invite.method(), Method::Invite);
    assert!(parsed_invite.header(&HeaderName::Contact).is_some());
    assert_eq!(
        parsed_invite
            .raw_header_value(&HeaderName::ContentType)
            .as_deref(),
        Some("application/sdp")
    );

    let register = RegisterBuilder::new()
        .registrar("sip:registrar.example.com")
        .aor("sip:alice@example.com")
        .user_info("sip:alice@example.com", "Alice")
        .contact("sip:alice@127.0.0.1:5060")
        .local_address(local_addr())
        .expires(3600)
        .call_id("register-call")
        .cseq(3)
        .build()
        .unwrap();
    let parsed_register = validate_generated_request(&register).unwrap();
    assert_eq!(
        parsed_register.uri().to_string(),
        "sip:registrar.example.com"
    );
    assert_eq!(
        parsed_register.to().unwrap().address().uri.to_string(),
        "sip:alice@example.com"
    );
    assert_eq!(
        parsed_register.from().unwrap().address().uri.to_string(),
        "sip:alice@example.com"
    );
    assert!(parsed_register.from().unwrap().tag().is_some());
    assert_eq!(
        parsed_register
            .raw_header_value(&HeaderName::Contact)
            .as_deref(),
        Some("<sip:alice@127.0.0.1:5060>")
    );
    assert_eq!(
        parsed_register
            .raw_header_value(&HeaderName::Expires)
            .as_deref(),
        Some("3600")
    );

    let outbound_register = RegisterBuilder::new()
        .registrar("sip:registrar.example.com")
        .aor("sip:alice@example.com")
        .user_info("sip:alice@example.com", "Alice")
        .contact_header(outbound_contact())
        .local_address(local_addr())
        .expires(3600)
        .call_id("outbound-register-call")
        .cseq(4)
        .build()
        .unwrap();
    let parsed_outbound = validate_generated_request(&outbound_register).unwrap();
    let contact = parsed_outbound
        .raw_header_value(&HeaderName::Contact)
        .unwrap();
    assert!(contact.contains(";ob"));
    assert!(contact.contains("+sip.instance"));
    assert!(contact.contains("reg-id=1"));
}

#[test]
fn generated_sip_compliance_in_dialog_quick_builders_generate_valid_requests() {
    let (call_id, from_uri, from_tag, to_uri, to_tag, cseq, addr) = established_args();

    let bye = bye_for_dialog(
        call_id, from_uri, from_tag, to_uri, to_tag, cseq, addr, None, None,
    )
    .unwrap();
    validate_generated_request(&bye).unwrap();

    let refer = refer_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        "sip:carol@example.com",
        cseq + 1,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&refer).unwrap();
    assert!(refer.header(&HeaderName::ReferTo).is_some());

    let update = update_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        Some(SDP.to_string()),
        cseq + 2,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&update).unwrap();
    assert_eq!(
        update.raw_header_value(&HeaderName::ContentType).as_deref(),
        Some("application/sdp")
    );

    let update_empty = update_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        None,
        cseq + 3,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&update_empty).unwrap();

    let info = info_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        "signal=on",
        None,
        cseq + 4,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&info).unwrap();
    assert!(info.header(&HeaderName::ContentType).is_some());

    let notify = notify_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        "dialog",
        Some("<dialog-info/>".to_string()),
        Some("active;expires=3600".to_string()),
        cseq + 5,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&notify).unwrap();
    assert!(notify.header(&HeaderName::Event).is_some());
    assert!(notify.header(&HeaderName::SubscriptionState).is_some());
    assert_eq!(
        notify.raw_header_value(&HeaderName::ContentType).as_deref(),
        Some("application/dialog-info+xml")
    );

    let message = message_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        "hello",
        None,
        cseq + 6,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&message).unwrap();
    assert!(message.header(&HeaderName::ContentType).is_some());

    let reinvite = reinvite_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        SDP,
        cseq + 7,
        addr,
        None,
        Some("sip:alice@127.0.0.1:5060".to_string()),
    )
    .unwrap();
    validate_generated_request(&reinvite).unwrap();
    assert!(reinvite.header(&HeaderName::Contact).is_some());

    let prack = prack_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        10,
        42,
        cseq + 8,
        addr,
        None,
    )
    .unwrap();
    validate_generated_request(&prack).unwrap();
    assert!(prack.header(&HeaderName::RAck).is_some());
}

#[test]
fn generated_sip_compliance_out_of_dialog_builders_generate_valid_requests() {
    let subscribe = subscribe_out_of_dialog(
        "sip:bob@example.com",
        "sip:alice@example.com",
        "sip:alice@127.0.0.1:5060",
        "presence",
        3600,
        1,
        local_addr(),
    )
    .unwrap();
    validate_generated_request(&subscribe).unwrap();
    assert!(subscribe.header(&HeaderName::Event).is_some());
    assert!(subscribe.header(&HeaderName::Expires).is_some());
    assert!(subscribe.header(&HeaderName::Contact).is_some());

    let message = message_out_of_dialog(
        "sip:bob@example.com",
        "sip:alice@example.com",
        "hello",
        1,
        local_addr(),
    )
    .unwrap();
    validate_generated_request(&message).unwrap();
    assert_eq!(
        message
            .raw_header_value(&HeaderName::ContentType)
            .as_deref(),
        Some("text/plain")
    );
}

#[test]
fn generated_sip_compliance_transaction_special_method_helpers_generate_valid_requests() {
    let route: Uri = "sip:proxy.example.com;lr".parse().unwrap();
    let invite = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr())
        .contact("sip:alice@127.0.0.1:5060")
        .call_id("special-call")
        .cseq(77)
        .add_route(route.clone())
        .build()
        .unwrap();
    validate_generated_request(&invite).unwrap();

    let ok = InviteResponseBuilder::ok_for_dialog(
        &invite,
        Some("dialog-1"),
        SDP.to_string(),
        "sip:bob@127.0.0.1:5062".to_string(),
    )
    .build()
    .unwrap();
    validate_generated_response(&ok).unwrap();

    let error = InviteResponseBuilder::error_for_dialog(
        &invite,
        StatusCode::BusyHere,
        Some("Busy Here".to_string()),
    )
    .build()
    .unwrap();
    validate_generated_response(&error).unwrap();

    let ack_2xx = create_ack_for_2xx(&invite, &ok, &local_addr()).unwrap();
    validate_generated_request(&ack_2xx).unwrap();
    assert_eq!(ack_2xx.cseq().unwrap().method, Method::Ack);
    assert_ne!(
        invite.first_via().unwrap().branch(),
        ack_2xx.first_via().unwrap().branch()
    );

    let ack_error = create_ack_for_error_response(&invite, &error).unwrap();
    validate_generated_request(&ack_error).unwrap();
    assert_eq!(ack_error.cseq().unwrap().method, Method::Ack);
    assert_eq!(
        invite.first_via().unwrap().branch(),
        ack_error.first_via().unwrap().branch()
    );

    let legacy_ack = create_ack_from_invite(&invite, &ok).unwrap();
    validate_generated_request(&legacy_ack).unwrap();
    assert_eq!(legacy_ack.cseq().unwrap().method, Method::Ack);

    let cancel = create_cancel_request(&invite, &local_addr()).unwrap();
    validate_generated_request(&cancel).unwrap();
    assert_eq!(cancel.uri(), invite.uri());
    assert_eq!(cancel.call_id().unwrap(), invite.call_id().unwrap());
    assert_eq!(cancel.from().unwrap(), invite.from().unwrap());
    assert_eq!(cancel.to().unwrap(), invite.to().unwrap());
    assert_eq!(cancel.cseq().unwrap().seq, invite.cseq().unwrap().seq);
    assert_eq!(cancel.cseq().unwrap().method, Method::Cancel);
    assert_eq!(
        cancel.first_via().unwrap().branch(),
        invite.first_via().unwrap().branch()
    );
    assert!(cancel.headers(&HeaderName::Route).iter().any(
        |h| matches!(h, TypedHeader::Route(r) if r.to_string().contains("proxy.example.com"))
    ));
}

#[test]
fn generated_sip_compliance_dialog_response_builders_generate_valid_responses() {
    let invite = invite_request();
    let bye = bye_for_dialog(
        "dialog-call",
        "sip:alice@example.com",
        "alice-tag",
        "sip:bob@example.com",
        "bob-tag",
        2,
        local_addr(),
        None,
        None,
    )
    .unwrap();
    let cancel = create_cancel_request(&invite, &local_addr()).unwrap();
    let register = RegisterBuilder::new()
        .registrar("sip:registrar.example.com")
        .aor("sip:alice@example.com")
        .user_info("sip:alice@example.com", "Alice")
        .contact("sip:alice@127.0.0.1:5060")
        .local_address(local_addr())
        .expires(3600)
        .build()
        .unwrap();
    let options = SimpleRequestBuilder::new(Method::Options, "sip:alice@example.com")
        .unwrap()
        .from("Server", "sip:server@example.com", Some("server-tag"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("options-call")
        .cseq(1)
        .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-options"))
        .max_forwards(70)
        .build();

    let responses = vec![
        create_trying_response(&invite),
        create_ringing_response(&invite),
        create_ok_response(&register),
        create_ok_response_for_bye(&bye),
        create_ok_response_for_cancel(&cancel),
        create_ok_response_for_options(&options, &[Method::Invite, Method::Ack, Method::Options]),
        create_response(&invite, StatusCode::ServerInternalError),
        ResponseBuilder::from_request_with_dialog_detection(StatusCode::Ok, &bye)
            .build()
            .unwrap(),
        InviteResponseBuilder::trying_for_dialog(&invite)
            .build()
            .unwrap(),
        InviteResponseBuilder::ringing_for_dialog(&invite, Some("dialog-1"), None)
            .build()
            .unwrap(),
        InviteResponseBuilder::ok_for_dialog(
            &invite,
            Some("dialog-1"),
            SDP.to_string(),
            "sip:bob@127.0.0.1:5062".to_string(),
        )
        .build()
        .unwrap(),
        response_for_dialog_transaction(
            "tx-1",
            invite.clone(),
            Some("dialog-1".to_string()),
            StatusCode::Ok,
            local_addr(),
            Some(SDP.to_string()),
            None,
        )
        .unwrap(),
    ];

    for response in responses {
        validate_generated_response(&response)
            .unwrap_or_else(|e| panic!("{} failed generated validation: {}", response, e));
        assert!(response.header(&HeaderName::Via).is_some());
        assert!(response.header(&HeaderName::From).is_some());
        assert!(response.header(&HeaderName::To).is_some());
        assert!(response.header(&HeaderName::CallId).is_some());
        assert!(response.header(&HeaderName::CSeq).is_some());
    }
}
