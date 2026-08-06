//! RFC 3261 §10.3 step 7: where a REGISTER's requested expiration comes from.
//!
//! > -  If the field value has an "expires" parameter, that value MUST be
//! >    taken as the requested expiration.
//! > -  If there is no such parameter, but the request has an Expires header
//! >    field, that value MUST be taken as the requested expiration.
//! > -  If there is neither, a locally-configured default value MUST be taken
//! >    as the requested expiration.
//!
//! The `Contact` parameter used to be skipped, which inverted the meaning of
//! the most common way to de-register: `Contact: <sip:alice@host>;expires=0`
//! with no `Expires` header was read as the one hour default. The registrar
//! answered 200 OK and kept a binding the device had just asked it to drop.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use rvoip_sip_core::{Message, Request};
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_dialog::DialogManager;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug)]
struct SilentTransport {
    local_addr: SocketAddr,
    sent: Mutex<Vec<Message>>,
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for SilentTransport {
    async fn send_message(
        &self,
        message: Message,
        _destination: SocketAddr,
    ) -> Result<(), rvoip_sip_transport::Error> {
        self.sent.lock().await.push(message);
        Ok(())
    }

    fn local_addr(&self) -> Result<SocketAddr, rvoip_sip_transport::Error> {
        Ok(self.local_addr)
    }

    async fn close(&self) -> Result<(), rvoip_sip_transport::Error> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        false
    }
}

async fn manager() -> Arc<DialogManager> {
    let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    let transport = Arc::new(SilentTransport {
        local_addr,
        sent: Mutex::new(Vec::new()),
    });
    let (_tx, transport_rx) = mpsc::channel(8);
    let (transaction_manager, _events) = TransactionManager::new(transport, transport_rx, Some(16))
        .await
        .expect("build TransactionManager");
    Arc::new(
        DialogManager::new(Arc::new(transaction_manager), local_addr)
            .await
            .expect("build DialogManager"),
    )
}

/// Build a REGISTER from the wire, so the headers under test are exactly what
/// a peer would send rather than what a builder happens to produce.
fn register_with(headers: &str) -> Request {
    let raw = format!(
        "REGISTER sip:example.test SIP/2.0\r\n\
         Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:alice@example.test>;tag=alice-tag\r\n\
         To: <sip:alice@example.test>\r\n\
         Call-ID: register-expires-precedence\r\n\
         CSeq: 2 REGISTER\r\n\
         {}\
         Content-Length: 0\r\n\
         \r\n",
        headers
    );
    match rvoip_sip_core::parse_message(raw.as_bytes()).expect("parse REGISTER") {
        Message::Request(request) => request,
        Message::Response(_) => panic!("expected a request"),
    }
}

/// The case that was inverted. A de-registration expressed the common way must
/// read as zero, not as the default.
#[tokio::test]
async fn a_contact_expires_of_zero_is_an_unregister() {
    let manager = manager().await;
    let request = register_with("Contact: <sip:alice@192.0.2.10:5060>;expires=0\r\n");

    assert_eq!(
        manager.extract_expires(&request),
        0,
        "a Contact expires=0 asks for removal; reading it as the default keeps a \
         binding the device asked to drop, and the registrar still answers 200 OK"
    );
}

/// The Contact parameter wins over the header when both are present.
#[tokio::test]
async fn the_contact_parameter_takes_precedence_over_the_expires_header() {
    let manager = manager().await;
    let request = register_with(
        "Contact: <sip:alice@192.0.2.10:5060>;expires=120\r\n\
         Expires: 3600\r\n",
    );

    assert_eq!(manager.extract_expires(&request), 120);
}

/// Including when the parameter is the one asking for removal.
#[tokio::test]
async fn a_contact_expires_of_zero_beats_a_nonzero_header() {
    let manager = manager().await;
    let request = register_with(
        "Contact: <sip:alice@192.0.2.10:5060>;expires=0\r\n\
         Expires: 3600\r\n",
    );

    assert_eq!(manager.extract_expires(&request), 0);
}

/// With no parameter, the header is used.
#[tokio::test]
async fn the_expires_header_is_used_when_the_contact_has_no_parameter() {
    let manager = manager().await;
    let request = register_with(
        "Contact: <sip:alice@192.0.2.10:5060>\r\n\
         Expires: 600\r\n",
    );

    assert_eq!(manager.extract_expires(&request), 600);
}

/// An `Expires: 0` header alone still unregisters, which is the form that
/// happened to work before and must keep working.
#[tokio::test]
async fn an_expires_header_of_zero_is_still_an_unregister() {
    let manager = manager().await;
    let request = register_with(
        "Contact: <sip:alice@192.0.2.10:5060>\r\n\
         Expires: 0\r\n",
    );

    assert_eq!(manager.extract_expires(&request), 0);
}

/// With neither, the local default applies.
#[tokio::test]
async fn the_local_default_applies_when_the_request_states_neither() {
    let manager = manager().await;
    let request = register_with("Contact: <sip:alice@192.0.2.10:5060>\r\n");

    assert_eq!(manager.extract_expires(&request), 3600);
}
