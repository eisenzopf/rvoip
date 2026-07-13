//! `connect-probe` — prove the gateway can establish a live WebRTC/Chime
//! connection to Amazon Connect, in isolation (no SIP, no Vapi).
//!
//! Runs a staged ladder and prints PASS/FAIL per stage:
//!   A. control plane   — StartWebRTCContact returns ConnectionData
//!   B. signaling       — Chime JOIN → JOIN_ACK (TURN credentials)
//!   C. media           — SUBSCRIBE → SDP answer → peer connection Connected
//!   D. audio (opt.)    — count inbound RTP / send Opus fixture (needs an answer)
//! then cleans up (LEAVE + StopContact).
//!
//! ```bash
//! AWS_REGION=us-west-2 \
//! AMAZON_CONNECT_INSTANCE_ID=<uuid> \
//! AMAZON_CONNECT_FLOW_ID=<uuid> \
//!   cargo run --bin connect-probe --features aws-control -- [--dump-frames] [--audio-secs N]
//! ```
//! (`--dump-frames` prints the exact JOIN/SUBSCRIBE base64 for `chime-decode`.)

use std::time::Duration;

use rvoip_amazon_connect::control::{
    AwsConnectStarter, ConnectContactStarter, StartContactRequest,
};
use rvoip_amazon_connect::signaling::chime::{
    build_join_frame, build_subscribe_frame, frame_to_base64, redacted_signaling_url,
    ChimeSignalingClient,
};
use rvoip_core::capability::CodecInfo;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::MediaStream;
use rvoip_webrtc::media::{fixtures::opus_rtp_packet_for_ssrc, from_tracks};
use rvoip_webrtc::{PeerRole, RvoipPeerConnection, WebRtcConfig};

#[tokio::main]
async fn main() {
    // Install a subscriber so metadata-only library diagnostics surface. Wire
    // logs contain frame type and size, never payload/SDP/ICE. Exact base64 is
    // available only through the owner-invoked `--dump-frames` path below.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rvoip_amazon_connect=debug".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let dump_frames = args.iter().any(|a| a == "--dump-frames");
    let audio_secs: u64 = args
        .iter()
        .position(|a| a == "--audio-secs")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if dump_frames {
        // JOIN is static; emit it now (before requiring AWS env) for diffing
        // against the browser capture via `chime-decode`.
        println!("tx:{}", frame_to_base64(&build_join_frame()));
    }

    let instance_id = req_env("AMAZON_CONNECT_INSTANCE_ID");
    let flow_id = req_env("AMAZON_CONNECT_FLOW_ID");
    let region = std::env::var("AWS_REGION").ok();

    match run(instance_id, flow_id, region, dump_frames, audio_secs).await {
        Ok(()) => println!("\n✅ connect-probe: all stages passed"),
        Err(e) => {
            eprintln!("\n❌ connect-probe failed: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(
    instance_id: String,
    flow_id: String,
    region: Option<String>,
    dump_frames: bool,
    audio_secs: u64,
) -> Result<(), String> {
    // ---- Stage A: control plane ------------------------------------------------
    stage("A", "control plane — StartWebRTCContact");
    let starter = AwsConnectStarter::from_env(region.clone()).await;
    let conn = starter
        .start_webrtc_contact(StartContactRequest {
            instance_id: instance_id.clone(),
            contact_flow_id: flow_id,
            display_name: "connect-probe".into(),
            attributes: [("HostedWidget-probe".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
            description: Some("rvoip connect-probe".into()),
            client_token: None,
        })
        .await
        .map_err(|e| format!("Stage A (StartWebRTCContact): {e}"))?;

    println!(
        "   response validated: contact={} meeting={} attendee={} region={}",
        !conn.contact_id.is_empty(),
        !conn.meeting_id.is_empty(),
        !conn.attendee_id.is_empty(),
        !conn.media_region.is_empty()
    );
    println!(
        "   endpoints: signaling={} turn_control={}",
        !conn.media_placement.signaling_url.is_empty(),
        conn.media_placement.turn_control_url.is_some()
    );
    if dump_frames {
        println!(
            "# signaling URL (token redacted): {}",
            redacted_signaling_url(&conn.media_placement.signaling_url, &conn.join_token)
        );
    }
    pass("A");

    // Ensure we always try to end the contact, even on later failure.
    let result = run_media(&conn, dump_frames, audio_secs).await;
    println!("\n— cleanup —");
    if let Err(e) = starter
        .stop_contact(conn.contact_id.clone(), instance_id)
        .await
    {
        eprintln!("   StopContact failed (non-fatal): {e}");
    } else {
        println!("   StopContact ok");
    }
    result
}

async fn run_media(
    conn: &rvoip_amazon_connect::control::ConnectionData,
    dump_frames: bool,
    audio_secs: u64,
) -> Result<(), String> {
    // ---- Stage B: signaling JOIN → JOIN_ACK -----------------------------------
    stage("B", "signaling — JOIN / JOIN_ACK (TURN)");
    let join = ChimeSignalingClient::join(conn, Duration::from_secs(15))
        .await
        .map_err(|e| format!("Stage B (Chime join): {e}"))?;
    let ice = join.ice_servers();
    if ice.is_empty() {
        println!("   ⚠ JOIN_ACK returned no TURN credentials (relay may be unavailable)");
    } else {
        println!("   TURN server count: {}", ice.len());
    }
    pass("B");

    // ---- Stage C: media — SUBSCRIBE / answer / Connected ----------------------
    stage("C", "media — SUBSCRIBE / SDP answer / peer Connected");
    let webrtc = WebRtcConfig {
        trickle_ice: false,
        ice_servers: ice,
        ..WebRtcConfig::default()
    };
    let peer = RvoipPeerConnection::new(&webrtc, PeerRole::Offerer)
        .await
        .map_err(|e| format!("Stage C (peer connection): {e}"))?;
    peer.add_local_audio_track()
        .await
        .map_err(|e| format!("Stage C (add audio track): {e}"))?;
    let offer_sdp = peer
        .create_offer_and_gather()
        .await
        .map_err(|e| format!("Stage C (create offer): {e}"))?;

    if dump_frames {
        println!(
            "tx:{}",
            frame_to_base64(&build_subscribe_frame(
                offer_sdp.clone(),
                conn.media_placement.audio_host_url.clone(),
                conn.attendee_id.clone()
            ))
        );
    }

    let (answer_sdp, session) = join
        .subscribe(offer_sdp, Duration::from_secs(15), Duration::from_secs(10))
        .await
        .map_err(|e| format!("Stage C (SUBSCRIBE): {e}"))?;
    println!("   SDP answer received ({} chars)", answer_sdp.len());
    peer.set_remote_answer(&answer_sdp)
        .await
        .map_err(|e| format!("Stage C (set answer): {e}"))?;

    peer.wait_connected(Duration::from_secs(30))
        .await
        .map_err(|e| format!("Stage C (wait_connected): {e}"))?;
    println!("   peer connection state: Connected");
    pass("C");

    // ---- Stage D: audio (optional) --------------------------------------------
    if audio_secs > 0 {
        stage("D", "audio — inbound count / outbound Opus");
        let remote = peer.wait_remote_track(Duration::from_secs(3)).await;
        if remote.is_none() {
            println!("   ⚠ no remote track within 3s (did an agent answer?) — skipping audio");
        } else {
            let local = peer
                .local_audio_track()
                .ok_or("Stage D: no local audio track")?;
            let ssrc = peer.local_audio_ssrc().unwrap_or(0);
            let stream = from_tracks(
                StreamId::new(),
                CodecInfo {
                    name: "opus".into(),
                    clock_rate_hz: 48000,
                    channels: 2,
                    fmtp: None,
                },
                local,
                ssrc,
                111,
                remote,
            );
            let mut frames_in = stream.frames_in();
            let frames_out = stream.frames_out();

            let deadline = tokio::time::Instant::now() + Duration::from_secs(audio_secs);
            let mut seq: u16 = 1;
            let mut inbound = 0u64;
            let mut ticker = tokio::time::interval(Duration::from_millis(20));
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        // Drive outbound Opus so the SFU sees us as an active sender.
                        let pkt = opus_rtp_packet_for_ssrc(ssrc, seq, seq as u32 * 960);
                        let frame = rvoip_core::stream::MediaFrame {
                            stream_id: stream.id(),
                            kind: rvoip_core::stream::StreamKind::Audio,
                            payload: pkt.payload,
                            timestamp_rtp: pkt.header.timestamp,
                            captured_at: chrono::Utc::now(),
                            payload_type: Some(111),
                        };
                        let _ = frames_out.try_send(frame);
                        seq = seq.wrapping_add(1);
                    }
                    f = frames_in.recv() => {
                        if f.is_some() { inbound += 1; }
                    }
                    _ = tokio::time::sleep_until(deadline) => break,
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            println!("   inbound media frames in {audio_secs}s: {inbound}");
            if inbound == 0 {
                println!(
                    "   ⚠ no inbound audio (agent not answered / one-way) — connection still OK"
                );
            }
            pass("D");
        }
    }

    // Graceful LEAVE.
    session.shutdown().await;
    Ok(())
}

fn stage(id: &str, what: &str) {
    println!("\n▶ Stage {id}: {what}");
}
fn pass(id: &str) {
    println!("   ✅ Stage {id} passed");
}
fn req_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| {
        eprintln!("connect-probe: missing required env var {key}");
        std::process::exit(2);
    })
}
