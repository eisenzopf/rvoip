//! One high-level rvoip voice-agent server for SIP or WebRTC callers.
//!
//! Only the listener configuration changes with `--transport`; admission,
//! Vapi attachment, bridging, events, and teardown all use the same code.

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
use rvoip::app::{
    AppEvent, AssignmentPolicy, Capability, CustomerPolicy, EmployeePolicy, Role, RvoipApp,
    SipConfig, WebRtcConfig,
};
use rvoip::core_traits::adapter::EndReason;
use rvoip::vapi::{
    VapiAdapter, VapiApiKey, VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiConfig,
};
use rvoip::Orchestrator;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

type AnyError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InboundTransport {
    Sip,
    Webrtc,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AudioMode {
    /// PCMU for SIP and PCM16 for WebRTC.
    Auto,
    /// Raw 8 kHz G.711 μ-law.
    Mulaw,
    /// Raw signed little-endian 16 kHz PCM.
    Pcm16,
}

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Accept SIP or WebRTC calls and attach a Vapi voice agent"
)]
struct Args {
    /// Caller-facing transport.
    #[arg(long, value_enum, default_value_t = InboundTransport::Sip)]
    transport: InboundTransport,

    /// Listener bind address. Defaults to 127.0.0.1:5060 for SIP and
    /// 127.0.0.1:8081 for WebRTC WebSocket signaling.
    #[arg(long)]
    bind: Option<SocketAddr>,

    /// Concrete SIP Via/Contact and RTP SDP address. Required with an
    /// unspecified SIP bind such as 0.0.0.0:5060.
    #[arg(long)]
    sip_advertise: Option<SocketAddr>,

    /// Optional public RTP address for SIP SDP. Port 0 retains each locally
    /// allocated media port. Defaults to the --sip-advertise IP.
    #[arg(long)]
    rtp_advertise: Option<SocketAddr>,

    /// SIP realm/domain used by the high-level server.
    #[arg(long, default_value = "vapi.local")]
    sip_domain: String,

    /// Saved Vapi assistant ID. Falls back to VAPI_ASSISTANT_ID.
    #[arg(long)]
    assistant_id: Option<String>,

    /// Vapi WebSocket audio mode.
    #[arg(long, value_enum, default_value_t = AudioMode::Auto)]
    audio: AudioMode,
}

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rvoip_sip_dialog=warn,webrtc=warn".into()),
        )
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();
    let api_key = VapiApiKey::new(std::env::var("VAPI_API_KEY").map_err(|_| {
        "VAPI_API_KEY is required; create a private API key in the Vapi dashboard"
    })?)?;
    let assistant = load_assistant(args.assistant_id.clone())?;
    let audio_format = resolve_audio_format(args.transport, args.audio);

    let app = build_app(&args).await?;
    // Subscribe before any remaining setup so an immediately arriving caller
    // cannot lose the transport-neutral accepted-call event.
    let events = app.subscribe_events();
    let orchestrator = app.orchestrator();
    let vapi = VapiAdapter::new(VapiConfig::new(api_key))?;

    // Register the exact shared adapter before accepting calls. attach_agent()
    // would register lazily, but eager registration also makes simultaneous
    // first calls deterministic.
    orchestrator.register(vapi.clone())?;

    print_ready(&app, &args, audio_format);
    serve_calls(app, events, orchestrator, vapi, assistant, audio_format).await
}

async fn build_app(args: &Args) -> Result<RvoipApp, AnyError> {
    let builder = RvoipApp::builder()
        // RvoipApp currently models an assigned service target even when an
        // extension such as Vapi owns the agent leg.
        .employees(EmployeePolicy::named(["vapi-agent"]))
        .assignment(AssignmentPolicy::fixed("vapi-agent"));

    let app = match args.transport {
        InboundTransport::Sip => {
            let bind = args
                .bind
                .unwrap_or_else(|| "127.0.0.1:5060".parse().expect("valid SIP default"));
            if bind.ip().is_unspecified() && args.sip_advertise.is_none() {
                return Err(
                    "--sip-advertise is required when --bind uses an unspecified SIP address"
                        .into(),
                );
            }
            let mut sip = SipConfig::bind(bind.to_string())
                .domain(args.sip_domain.clone())
                .allow(Role::Customer, [Capability::Voice]);
            if let Some(advertised) = args.sip_advertise {
                sip = sip.advertised_addr(advertised);
            }
            if let Some(media_public) = args.rtp_advertise {
                sip = sip.media_public_addr(media_public);
            }
            builder
                .customers(CustomerPolicy::sip_only())
                .sip(sip)
                .build()
                .await?
        }
        InboundTransport::Webrtc => {
            if args.sip_advertise.is_some() || args.rtp_advertise.is_some() {
                return Err(
                    "--sip-advertise and --rtp-advertise are only valid with --transport sip"
                        .into(),
                );
            }
            let bind = args
                .bind
                .unwrap_or_else(|| "127.0.0.1:8081".parse().expect("valid WebRTC default"));
            builder
                .customers(CustomerPolicy::webrtc_only())
                .webrtc(
                    WebRtcConfig::ws(bind.to_string()).allow(Role::Customer, [Capability::Voice]),
                )
                .build()
                .await?
        }
    };
    Ok(app)
}

async fn serve_calls(
    _app: RvoipApp,
    mut events: broadcast::Receiver<AppEvent>,
    orchestrator: Arc<Orchestrator>,
    vapi: Arc<VapiAdapter>,
    assistant: VapiAssistant,
    audio_format: VapiAudioFormat,
) -> Result<(), AnyError> {
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown requested");
                return Ok(());
            }
            event = events.recv() => {
                match event {
                    Ok(AppEvent::InboundCallAccepted { connection_id, transport }) => {
                        info!(%connection_id, ?transport, ?audio_format, "caller accepted; attaching Vapi");
                        spawn_agent_call(
                            Arc::clone(&orchestrator),
                            Arc::clone(&vapi),
                            assistant.clone(),
                            audio_format,
                            connection_id,
                        );
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "high-level app event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err("high-level app event stream closed".into());
                    }
                }
            }
        }
    }
}

fn spawn_agent_call(
    orchestrator: Arc<Orchestrator>,
    vapi: Arc<VapiAdapter>,
    assistant: VapiAssistant,
    audio_format: VapiAudioFormat,
    caller_connection: rvoip::core_traits::ids::ConnectionId,
) {
    tokio::spawn(async move {
        let options = VapiCallOptions::new(assistant).with_audio_format(audio_format);
        let mut call = match vapi
            .attach_agent(&orchestrator, caller_connection.clone(), options)
            .await
        {
            Ok(call) => call,
            Err(error) => {
                error!(%caller_connection, error = %error, "failed to attach Vapi agent");
                let _ = orchestrator
                    .end_connection(
                        caller_connection,
                        EndReason::Failed {
                            detail: "Vapi attachment failed".into(),
                        },
                    )
                    .await;
                return;
            }
        };

        info!(
            caller = %call.caller_connection_id(),
            vapi = %call.vapi_connection_id(),
            bridge = %call.bridge_id(),
            "Vapi agent bridge active"
        );
        let outcome = call.wait().await;
        info!(?outcome, "Vapi agent call finished");
    });
}

fn load_assistant(cli_id: Option<String>) -> Result<VapiAssistant, AnyError> {
    if let Some(id) = cli_id.or_else(|| std::env::var("VAPI_ASSISTANT_ID").ok()) {
        if id.trim().is_empty() {
            return Err("the Vapi assistant ID cannot be empty".into());
        }
        return Ok(VapiAssistant::saved(id));
    }

    if let Ok(raw) = std::env::var("VAPI_TRANSIENT_ASSISTANT_JSON") {
        let definition: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| "VAPI_TRANSIENT_ASSISTANT_JSON must contain a JSON object")?;
        if !definition.is_object() {
            return Err("VAPI_TRANSIENT_ASSISTANT_JSON must contain a JSON object".into());
        }
        return Ok(VapiAssistant::transient(definition));
    }

    Err("set VAPI_ASSISTANT_ID, pass --assistant-id, or set VAPI_TRANSIENT_ASSISTANT_JSON".into())
}

fn resolve_audio_format(transport: InboundTransport, audio: AudioMode) -> VapiAudioFormat {
    match audio {
        AudioMode::Auto => match transport {
            InboundTransport::Sip => VapiAudioFormat::MuLaw8Khz,
            InboundTransport::Webrtc => VapiAudioFormat::PcmS16Le16Khz,
        },
        AudioMode::Mulaw => VapiAudioFormat::MuLaw8Khz,
        AudioMode::Pcm16 => VapiAudioFormat::PcmS16Le16Khz,
    }
}

fn print_ready(app: &RvoipApp, args: &Args, audio_format: VapiAudioFormat) {
    let addresses = app.addresses();
    println!();
    println!("=== rvoip Vapi voice-agent server ===");
    println!("transport:    {:?}", args.transport);
    println!("audio format: {audio_format:?}");
    match args.transport {
        InboundTransport::Sip => {
            let bind = addresses.sip.expect("configured SIP address");
            let advertised = args.sip_advertise.unwrap_or(bind);
            println!("SIP bind:      {bind}");
            println!("SIP address:   sip:vapi@{advertised}");
        }
        InboundTransport::Webrtc => {
            let bind = addresses
                .webrtc_ws
                .expect("configured WebRTC signaling address");
            println!("WebRTC WS:     ws://{bind}");
        }
    }
    println!("waiting for callers; Ctrl-C stops the server");
    println!();
}
