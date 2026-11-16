//! Local hangup and hold/resume control from every established state.
//!
//! A raw-UDP capture UAS answers one call from a `StreamPeer` and then
//! scripts the hold/resume re-INVITEs, so the UAC can be left in
//! `HoldPending`, `OnHold` or `Resuming` deterministically:
//!
//! 1. A local hangup in each of those states sends an in-dialog BYE
//!    (RFC 3261 §15.1) and completes only after the peer answers it.
//! 2. A hold or resume re-INVITE that cannot be dispatched leaves the call
//!    where it was (`Active` or `OnHold`) instead of stranding it in
//!    `HoldPending`/`Resuming` with no transaction to complete it. The
//!    capture UAS forces the dispatch failure by refreshing the remote
//!    target to a `;transport=tcp` Contact with no TCP listener.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

use rvoip_sip::api::unified::Config;
use rvoip_sip::{CallState, SessionHandle, StreamPeer};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderValue;
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const STEP: Duration = Duration::from_secs(5);
const UAS_TAG: &str = "established-hangup-uas";

#[derive(Clone, Copy)]
enum ContactTransport {
    Udp,
    /// Nothing listens on TCP at the capture port, so the next in-dialog
    /// request fails before it reaches the wire.
    UnreachableTcp,
}

struct CaptureUas {
    sip: UdpSocket,
    _media: UdpSocket,
    media_port: u16,
}

impl CaptureUas {
    async fn bind() -> Self {
        let sip = UdpSocket::bind("127.0.0.1:0").await.expect("capture UAS");
        let media = UdpSocket::bind("127.0.0.1:0").await.expect("capture media");
        let media_port = media.local_addr().expect("media address").port();
        Self {
            sip,
            _media: media,
            media_port,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.sip.local_addr().expect("capture address")
    }

    /// Next request of `method`. ACKs and retransmitted INVITEs are skipped.
    async fn request(&self, method: Method) -> (Request, SocketAddr) {
        timeout(STEP, async {
            let mut packet = vec![0u8; 16_384];
            loop {
                let (size, from) = self.sip.recv_from(&mut packet).await.expect("recv");
                let Ok(Message::Request(request)) = parse_message(&packet[..size]) else {
                    continue;
                };
                if request.method() == method {
                    return (request, from);
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{method} did not reach the capture UAS"))
    }

    async fn answer(&self, request: &Request, from: SocketAddr, contact: ContactTransport) {
        let mut response = create_response(request, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = response
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            to.set_tag(UAS_TAG);
        }
        let contact = match contact {
            ContactTransport::Udp => format!("<sip:target@{}>", self.addr()),
            ContactTransport::UnreachableTcp => {
                format!("<sip:target@{};transport=tcp>", self.addr())
            }
        };
        response.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(contact.into_bytes()),
        ));
        if request.method() == Method::Invite {
            // A held (sendonly) offer is answered recvonly.
            let direction = if String::from_utf8_lossy(&request.body).contains("a=sendonly") {
                "recvonly"
            } else {
                "sendrecv"
            };
            let version = request.cseq().expect("CSeq").sequence();
            response.headers.push(TypedHeader::ContentType(
                rvoip_sip_core::types::ContentType::sdp(),
            ));
            response.body = bytes::Bytes::from(format!(
                "v=0\r\no=capture 1 {version} IN IP4 127.0.0.1\r\ns=capture\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na={direction}\r\n",
                self.media_port
            ));
            response
                .headers
                .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
            response.headers.push(TypedHeader::ContentLength(
                rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
            ));
        }
        self.sip
            .send_to(&Message::Response(response).to_bytes(), from)
            .await
            .expect("send response");
    }
}

struct Fixture {
    uas: CaptureUas,
    uac: StreamPeer,
    call: SessionHandle,
}

impl Fixture {
    async fn establish(name: &str, contact: ContactTransport) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::WARN)
            .try_init();
        let uas = CaptureUas::bind().await;
        let uac = StreamPeer::with_config(Config::local(name, 0))
            .await
            .expect("UAC");
        let target = format!("sip:target@{}", uas.addr());
        let send = tokio::spawn(uac.invite(target).send());
        let (invite, from) = uas.request(Method::Invite).await;
        uas.answer(&invite, from, contact).await;
        let call_id = timeout(STEP, send)
            .await
            .expect("INVITE send deadline")
            .expect("INVITE task")
            .expect("INVITE");
        let call = uac.session(&call_id);
        call.wait_for_answered(Some(STEP))
            .await
            .expect("call answered");
        wait_for_state(&call, CallState::Active).await;
        Self { uas, uac, call }
    }

    /// Hold (or resume) and leave its re-INVITE unanswered.
    async fn start_direction_change(&self, hold: bool) -> (Request, SocketAddr) {
        if hold {
            self.call.hold().await.expect("hold re-INVITE dispatched");
        } else {
            self.call
                .resume()
                .await
                .expect("resume re-INVITE dispatched");
        }
        self.uas.request(Method::Invite).await
    }

    /// Hang up, require one BYE on the wire, and require the hangup to wait
    /// for the peer's answer to it.
    async fn hang_up_with_bye(self) {
        let call = self.call.clone();
        let hangup = tokio::spawn(async move { call.hangup().await });
        let (bye, from) = self.uas.request(Method::Bye).await;
        assert_eq!(bye.to().and_then(|to| to.tag()), Some(UAS_TAG));
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !hangup.is_finished(),
            "hangup must wait for the peer's response to its BYE"
        );
        self.uas.answer(&bye, from, ContactTransport::Udp).await;
        timeout(STEP, hangup)
            .await
            .expect("hangup deadline")
            .expect("hangup task")
            .expect("hangup confirmed by the BYE response");
        let _ = timeout(STEP, self.uac.shutdown()).await;
    }
}

async fn wait_for_state(call: &SessionHandle, expected: CallState) {
    let reached = timeout(STEP, async {
        loop {
            if matches!(call.state().await, Ok(state) if state == expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if reached.is_err() {
        panic!(
            "call did not reach {expected:?}; last state: {:?}",
            call.state().await
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hangup_while_hold_pending_sends_bye() {
    let fixture = Fixture::establish("hangup-hold-pending", ContactTransport::Udp).await;
    let _pending_hold = fixture.start_direction_change(true).await;
    wait_for_state(&fixture.call, CallState::HoldPending).await;
    fixture.hang_up_with_bye().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hangup_while_on_hold_sends_bye() {
    let fixture = Fixture::establish("hangup-on-hold", ContactTransport::Udp).await;
    let (hold, from) = fixture.start_direction_change(true).await;
    fixture.uas.answer(&hold, from, ContactTransport::Udp).await;
    wait_for_state(&fixture.call, CallState::OnHold).await;
    fixture.hang_up_with_bye().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hangup_while_resuming_sends_bye() {
    let fixture = Fixture::establish("hangup-resuming", ContactTransport::Udp).await;
    let (hold, from) = fixture.start_direction_change(true).await;
    fixture.uas.answer(&hold, from, ContactTransport::Udp).await;
    wait_for_state(&fixture.call, CallState::OnHold).await;
    let _pending_resume = fixture.start_direction_change(false).await;
    wait_for_state(&fixture.call, CallState::Resuming).await;
    fixture.hang_up_with_bye().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn undispatchable_hold_returns_the_call_to_active() {
    let fixture =
        Fixture::establish("hold-dispatch-failure", ContactTransport::UnreachableTcp).await;
    for attempt in 1..=2 {
        assert!(
            fixture.call.hold().await.is_err(),
            "hold attempt {attempt} must report its dispatch failure"
        );
        assert_eq!(
            fixture.call.state().await.expect("state after failed hold"),
            CallState::Active,
            "a hold that never reached the wire must not leave the call in HoldPending"
        );
    }
    let _ = timeout(STEP, fixture.call.hangup()).await;
    let _ = timeout(STEP, fixture.uac.shutdown()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn undispatchable_resume_keeps_the_call_on_hold() {
    let fixture = Fixture::establish("resume-dispatch-failure", ContactTransport::Udp).await;
    let (hold, from) = fixture.start_direction_change(true).await;
    // The hold 2xx refreshes the remote target to an unreachable Contact.
    fixture
        .uas
        .answer(&hold, from, ContactTransport::UnreachableTcp)
        .await;
    wait_for_state(&fixture.call, CallState::OnHold).await;
    for attempt in 1..=2 {
        assert!(
            fixture.call.resume().await.is_err(),
            "resume attempt {attempt} must report its dispatch failure"
        );
        assert_eq!(
            fixture
                .call
                .state()
                .await
                .expect("state after failed resume"),
            CallState::OnHold,
            "a resume that never reached the wire must not leave the call in Resuming"
        );
    }
    let _ = timeout(STEP, fixture.call.hangup()).await;
    let _ = timeout(STEP, fixture.uac.shutdown()).await;
}
