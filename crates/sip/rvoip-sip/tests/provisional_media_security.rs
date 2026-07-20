//! Reliable early-media security must become observable before final answer.

use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::{
    AudioSource, CallHandlerDecision, CallState, CallbackPeer, Config, Event, MediaSecurityProfile,
    UnifiedCoordinator,
};
use tokio::sync::Notify;

fn reserve_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("reserve UDP port")
        .local_addr()
        .expect("reserved UDP address")
        .port()
}

fn secure_config(name: &str, port: u16) -> Config {
    let mut config = Config::local(name, port);
    config.offered_codecs = vec![0, 101];
    config.offer_srtp = true;
    config.srtp_required = true;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ringing_then_183_installs_and_publishes_srtp_before_final_answer() {
    let caller_port = reserve_udp_port();
    let destination_port = reserve_udp_port();
    let allow_early_media = Arc::new(Notify::new());
    let early_media_started = Arc::new(Notify::new());
    let allow_final_answer = Arc::new(Notify::new());

    let destination = CallbackPeer::builder(secure_config(
        "early-security-destination",
        destination_port,
    ))
    .on_incoming({
        let allow_early_media = Arc::clone(&allow_early_media);
        let early_media_started = Arc::clone(&early_media_started);
        let allow_final_answer = Arc::clone(&allow_final_answer);
        move |incoming| {
            let allow_early_media = Arc::clone(&allow_early_media);
            let early_media_started = Arc::clone(&early_media_started);
            let allow_final_answer = Arc::clone(&allow_final_answer);
            async move {
                allow_early_media.notified().await;
                incoming
                    .send_early_media_with_source(
                        None,
                        AudioSource::Tone {
                            frequency: 700.0,
                            amplitude: 0.5,
                        },
                    )
                    .await
                    .expect("send reliable 183 with SRTP tone");
                early_media_started.notify_one();
                allow_final_answer.notified().await;
                CallHandlerDecision::Accept
            }
        }
    })
    .build()
    .await
    .expect("build early-media destination");
    let destination_shutdown = destination.shutdown_handle();
    let destination_task = tokio::spawn(destination.run());

    let caller = UnifiedCoordinator::new(secure_config("early-security-caller", caller_port))
        .await
        .expect("build early-media caller");
    let session_id = caller
        .invite(
            Some(format!("sip:caller@127.0.0.1:{caller_port}")),
            format!("sip:destination@127.0.0.1:{destination_port}"),
        )
        .send()
        .await
        .expect("send early-media INVITE");
    let handle = caller.session(&session_id);

    let ringing = handle
        .wait_for_progress(
            |event| {
                matches!(
                    event,
                    Event::CallProgress {
                        status_code: 180,
                        sdp: None,
                        ..
                    }
                )
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("observe SDP-free 180");
    assert!(matches!(
        ringing,
        Event::CallProgress {
            status_code: 180,
            sdp: None,
            ..
        }
    ));
    assert_eq!(
        handle.state().await.expect("ringing state"),
        CallState::Ringing
    );
    assert_eq!(
        handle.media_security().await.expect("ringing security"),
        None
    );
    assert!(
        handle
            .wait_for_media_security(Some(Duration::from_millis(150)))
            .await
            .is_err(),
        "180 without SDP must not publish media-security readiness"
    );

    allow_early_media.notify_one();
    tokio::time::timeout(Duration::from_secs(5), early_media_started.notified())
        .await
        .expect("destination never started early media");
    handle
        .wait_for_progress(
            |event| {
                matches!(
                    event,
                    Event::CallProgress {
                        status_code: 183,
                        sdp: Some(sdp),
                        ..
                    } if sdp.contains("RTP/SAVP") && sdp.contains("a=crypto:")
                )
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("observe SRTP 183 SDP");
    let security = handle
        .wait_for_media_security(Some(Duration::from_secs(5)))
        .await
        .expect("provisional SRTP readiness");
    assert_eq!(security.profile, MediaSecurityProfile::RtpSavp);
    assert!(security.contexts_installed);
    assert_eq!(
        handle.state().await.expect("early-media state"),
        CallState::EarlyMedia
    );
    assert!(
        handle
            .wait_for_answered(Some(Duration::from_millis(150)))
            .await
            .is_err(),
        "SRTP readiness must not emit a final-answer lifecycle"
    );

    let (_audio_tx, mut audio_rx) = handle
        .audio()
        .await
        .expect("provisional caller audio")
        .split();
    let early_frame = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = audio_rx
                .recv()
                .await
                .expect("early-media stream remained open");
            if frame
                .samples
                .iter()
                .any(|sample| sample.unsigned_abs() > 500)
            {
                return frame;
            }
        }
    })
    .await
    .expect("caller received no decrypted early SRTP");
    assert_eq!(early_frame.sample_rate, 8_000);
    assert_eq!(early_frame.channels, 1);

    allow_final_answer.notify_one();
    let answered = handle
        .wait_for_answered(Some(Duration::from_secs(5)))
        .await
        .expect("final answer after provisional media");
    answered
        .hangup_and_wait(Some(Duration::from_secs(5)))
        .await
        .expect("hang up early-media test call");

    caller
        .shutdown_gracefully(Some(Duration::from_secs(3)))
        .await
        .expect("shutdown early-media caller");
    destination_shutdown.shutdown();
    destination_task
        .await
        .expect("destination task")
        .expect("destination shutdown");
}
