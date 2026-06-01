//! SIP_API_DESIGN_2 §10 test #6 — header policy classification unit
//! tests. Verifies the `HeaderPolicy::classify` matrix on a
//! representative set of method × header pairs.
//!
//! The matrix being asserted:
//!
//! | Header | INVITE | REGISTER | SUBSCRIBE | NOTIFY | MESSAGE | REFER | BYE |
//! |---|---|---|---|---|---|---|---|
//! | Call-ID | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged |
//! | CSeq | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged |
//! | Via | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged |
//! | Max-Forwards | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged |
//! | Route | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged | StackManaged |
//! | Contact | MethodShaped | MethodShaped | MethodShaped | AppControlled | AppControlled | AppControlled | AppControlled |
//! | Authorization | MethodShaped | MethodShaped | MethodShaped | AppControlled | MethodShaped | MethodShaped | AppControlled |
//! | Expires | AppControlled | MethodShaped | MethodShaped | AppControlled | AppControlled | AppControlled | AppControlled |
//! | Refer-To | AppControlled | AppControlled | AppControlled | AppControlled | AppControlled | MethodShaped | AppControlled |
//! | Event | AppControlled | AppControlled | MethodShaped | MethodShaped | AppControlled | AppControlled | AppControlled |
//! | X-* | AppControlled | AppControlled | AppControlled | AppControlled | AppControlled | AppControlled | AppControlled |

use rvoip_sip::api::headers::policy::{classify, HeaderRole};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::Method;

fn assert_stack(method: Method, name: HeaderName) {
    assert_eq!(
        classify(method.clone(), &name),
        HeaderRole::StackManaged,
        "expected {:?} on {} to be StackManaged",
        name,
        method
    );
}

fn assert_method_shaped(method: Method, name: HeaderName) {
    match classify(method.clone(), &name) {
        HeaderRole::MethodShaped { .. } => {}
        other => panic!(
            "expected {:?} on {} to be MethodShaped, got {:?}",
            name, method, other
        ),
    }
}

fn assert_app(method: Method, name: HeaderName) {
    assert_eq!(
        classify(method.clone(), &name),
        HeaderRole::ApplicationControlled,
        "expected {:?} on {} to be ApplicationControlled",
        name,
        method
    );
}

#[test]
fn stack_managed_headers_apply_across_every_method() {
    let methods = [
        Method::Invite,
        Method::Register,
        Method::Subscribe,
        Method::Notify,
        Method::Message,
        Method::Refer,
        Method::Bye,
        Method::Cancel,
        Method::Info,
        Method::Options,
        Method::Update,
    ];
    let stack = [
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::Via,
        HeaderName::MaxForwards,
        HeaderName::ContentLength,
        HeaderName::RecordRoute,
        HeaderName::Route,
    ];
    for m in methods {
        for h in stack.iter().cloned() {
            assert_stack(m.clone(), h);
        }
    }
}

#[test]
fn contact_is_method_shaped_only_on_dialog_creating_methods() {
    assert_method_shaped(Method::Invite, HeaderName::Contact);
    assert_method_shaped(Method::Register, HeaderName::Contact);
    assert_method_shaped(Method::Subscribe, HeaderName::Contact);
    // In-dialog methods see Contact as application-controlled
    // (re-classification kicks in at the in-dialog builder layer).
    assert_app(Method::Notify, HeaderName::Contact);
    assert_app(Method::Refer, HeaderName::Contact);
    assert_app(Method::Bye, HeaderName::Contact);
}

#[test]
fn authorization_is_method_shaped_on_uac_request_methods() {
    assert_method_shaped(Method::Invite, HeaderName::Authorization);
    assert_method_shaped(Method::Register, HeaderName::Authorization);
    assert_method_shaped(Method::Subscribe, HeaderName::Authorization);
    assert_method_shaped(Method::Message, HeaderName::Authorization);
    assert_method_shaped(Method::Options, HeaderName::Authorization);
    assert_method_shaped(Method::Refer, HeaderName::Authorization);
}

#[test]
fn expires_is_method_shaped_only_on_register_and_subscribe() {
    assert_method_shaped(Method::Register, HeaderName::Expires);
    assert_method_shaped(Method::Subscribe, HeaderName::Expires);
    assert_app(Method::Invite, HeaderName::Expires);
    assert_app(Method::Notify, HeaderName::Expires);
}

#[test]
fn refer_to_is_method_shaped_only_on_refer() {
    assert_method_shaped(Method::Refer, HeaderName::ReferTo);
    assert_app(Method::Invite, HeaderName::ReferTo);
    assert_app(Method::Notify, HeaderName::ReferTo);
}

#[test]
fn event_is_method_shaped_only_on_subscribe_and_notify() {
    assert_method_shaped(Method::Subscribe, HeaderName::Event);
    assert_method_shaped(Method::Notify, HeaderName::Event);
    assert_app(Method::Invite, HeaderName::Event);
    assert_app(Method::Refer, HeaderName::Event);
}

#[test]
fn subscription_state_is_method_shaped_only_on_notify() {
    assert_method_shaped(Method::Notify, HeaderName::SubscriptionState);
    assert_app(Method::Subscribe, HeaderName::SubscriptionState);
}

#[test]
fn application_x_headers_are_application_controlled_everywhere() {
    let custom = HeaderName::Other("X-Tenant-ID".to_string());
    for m in [
        Method::Invite,
        Method::Register,
        Method::Subscribe,
        Method::Notify,
        Method::Message,
        Method::Refer,
        Method::Bye,
        Method::Info,
        Method::Options,
        Method::Cancel,
        Method::Update,
    ] {
        assert_app(m, custom.clone());
    }
}
