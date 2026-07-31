//! End-to-end hold/resume coverage when neither endpoint allocates media-core RTP.

use std::net::UdpSocket;
use std::time::Duration;

use rvoip_sip::{CallState, Config, Event, SessionHandle, StreamPeer};

fn reserve_loopback_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("reserve loopback port")
        .local_addr()
        .expect("reserved loopback address")
        .port()
}

async fn wait_for_state(call: &SessionHandle, expected: CallState) {
    let wait = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if matches!(call.state().await, Ok(state) if state == expected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if wait.is_err() {
        panic!(
            "call did not reach {expected:?}; last state: {:?}",
            call.state().await
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signaling_only_hold_resume_is_negotiated_and_idempotent() {
    let server_port = reserve_loopback_port();
    let mut client_port = reserve_loopback_port();
    while client_port == server_port {
        client_port = reserve_loopback_port();
    }

    let mut server = StreamPeer::with_config(
        Config::local("signaling-only-server", server_port).with_signaling_only_media(9),
    )
    .await
    .expect("start signaling-only server");
    let server_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(8), server.wait_for_incoming())
            .await
            .map_err(|_| "incoming call timed out".to_string())?
            .map_err(|error| error.to_string())?;
        let call = incoming.accept().await.map_err(|error| error.to_string())?;
        let mut events = call.events().await.map_err(|error| error.to_string())?;
        let mut remote_holds = 0;
        let mut remote_resumes = 0;

        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(event) = events.next().await {
                match event {
                    Event::RemoteCallOnHold { .. } => remote_holds += 1,
                    Event::RemoteCallResumed { .. } => remote_resumes += 1,
                    Event::CallEnded { .. } | Event::CallFailed { .. } => break,
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| "server event loop timed out".to_string())?;
        server.shutdown().await.map_err(|error| error.to_string())?;
        Ok::<_, String>((remote_holds, remote_resumes))
    });

    let client = StreamPeer::with_config(
        Config::local("signaling-only-client", client_port).with_signaling_only_media(9),
    )
    .await
    .expect("start signaling-only client");
    let call_id = client
        .invite(format!("sip:server@127.0.0.1:{server_port}"))
        .send()
        .await
        .expect("send signaling-only INVITE");
    let call = client.coordinator().session(&call_id);
    call.wait_for_answered(Some(Duration::from_secs(8)))
        .await
        .expect("signaling-only call answered");

    call.hold().await.expect("send signaling-only hold");
    wait_for_state(&call, CallState::OnHold).await;
    call.hold().await.expect("repeat hold is idempotent");
    assert_eq!(call.state().await.expect("held state"), CallState::OnHold);

    call.resume().await.expect("send signaling-only resume");
    wait_for_state(&call, CallState::Active).await;
    call.resume().await.expect("repeat resume is idempotent");
    assert_eq!(call.state().await.expect("active state"), CallState::Active);

    call.hangup().await.expect("hang up signaling-only call");
    client.shutdown().await.expect("shutdown client");
    let (remote_holds, remote_resumes) = server_task
        .await
        .expect("join server task")
        .expect("server scenario");
    assert_eq!(remote_holds, 1, "repeated hold emitted another negotiation");
    assert_eq!(
        remote_resumes, 1,
        "repeated resume emitted another negotiation"
    );
}
