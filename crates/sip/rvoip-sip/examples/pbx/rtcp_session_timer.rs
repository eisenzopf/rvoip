//! RTCP and RFC 4028 session-timer cells for the PBX release matrix (#318).
//!
//! Four scenarios, all on the `Endpoint` API (they test the stack's wire
//! behaviour, not an API surface, so one surface keeps the gate short):
//!
//! - `rtcp_mux`: the PBX multiplexes RTCP on the RTP port (RFC 5761). Both
//!   ends must exchange reports: rvoip's `media_quality()` must carry the
//!   peer's view (`remote_*`) and a round-trip time.
//! - `rtcp_non_mux`: the PBX declines mux and rvoip runs with
//!   `Config::rtcp_non_mux`, so RTCP goes over RTP port + 1 (`a=rtcp:`), with
//!   the same peer-report requirement.
//! - `rtcp_mux_declined`: the same non-mux PBX, rvoip with the option off.
//!   RFC 5761 §5.1.1 forbids RTCP to the RTP port, so neither side may see
//!   reports; `run.sh` checks the PBX side.
//! - `session_timer`: rvoip offers a 90 s `Session-Expires`. rvoip refreshes
//!   its UAC leg; the PBX refreshes the leg toward rvoip's UAS. FreeSWITCH's
//!   Min-SE of 120 first answers 422, which rvoip must retry.
//!
//! Every role writes a small evidence file into the cell directory. The SIP
//! traces in it come from rvoip's default redaction policy, so they keep the
//! session-timer headers and drop Request-URIs, bodies (SDES keys) and
//! credentials. The SDP evidence keeps only the `m=audio` and RTCP lines.

use super::*;

use rvoip_sip::{MediaQualityStats, SipTraceDirection};

/// Marker the caller prints once both roles hold their RTCP verdicts and the
/// call is still up. `run.sh` waits for it to snapshot the PBX's own view.
pub const PBX_EVIDENCE_MARKER: &str = "PBX_EVIDENCE_WINDOW_OPEN";

/// Whether a scenario belongs to this module.
pub fn is_rtcp_or_timer_scenario(scenario: Scenario) -> bool {
    matches!(
        scenario,
        Scenario::RtcpMux
            | Scenario::RtcpNonMux
            | Scenario::RtcpMuxDeclined
            | Scenario::SessionTimer
    )
}

/// The PBX user a role registers as in these cells.
///
/// Asterisk configures `rtcp_mux` and `timers` per endpoint, so each cell
/// family has its own users (`infra/release-runners/pbx/asterisk/config`):
/// x1yy multiplex RTCP, x2yy decline it, 23yy run session timers. They
/// borrow user x0yy's ports and password, which is safe because cells run
/// one at a time. FreeSWITCH configures both per profile instead, so its
/// users stay the same and `run.sh` points them at the dedicated profile.
pub fn scenario_username(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> String {
    let base = username_for(transport, role);
    let offset = match (provider, scenario) {
        (PbxProvider::Asterisk, Scenario::RtcpMux) => 100,
        (PbxProvider::Asterisk, Scenario::RtcpNonMux | Scenario::RtcpMuxDeclined) => 200,
        (PbxProvider::Asterisk, Scenario::SessionTimer) => 300,
        _ => 0,
    };
    match base.parse::<u16>() {
        Ok(number) if offset > 0 => (number + offset).to_string(),
        _ => base.to_string(),
    }
}

/// The base user whose ports and credentials a scenario user borrows:
/// `2101` → `2001`, `1203` → `1003`; anything else maps to itself.
pub fn base_username(username: &str) -> String {
    let bytes = username.as_bytes();
    let is_scenario_user = bytes.len() == 4
        && matches!(bytes[0], b'1' | b'2')
        && matches!(bytes[1], b'1'..=b'9')
        && bytes[2] == b'0'
        && matches!(bytes[3], b'1'..=b'3');
    if is_scenario_user {
        format!("{}0{}", bytes[0] as char, &username[2..])
    } else {
        username.to_string()
    }
}

/// The stack options a scenario needs, applied on top of the shared config.
pub fn apply_scenario_policy(scenario: Scenario, config: &mut Config) {
    match scenario {
        Scenario::RtcpNonMux => config.rtcp_non_mux = true,
        Scenario::RtcpMuxDeclined => config.rtcp_non_mux = false,
        Scenario::SessionTimer => {
            config.session_timer_secs = Some(SESSION_TIMER_OFFER_SECS);
            config.session_timer_min_se = SESSION_TIMER_OFFER_SECS;
        }
        _ => {}
    }
    if is_rtcp_or_timer_scenario(scenario) {
        // Default redaction: Request-URIs, bodies and non-allowlisted header
        // values are replaced; CSeq, Session-Expires, Min-SE, Require and
        // Supported are kept, which is exactly the evidence these cells need.
        config.sip_trace = rvoip_sip::SipTraceConfig::enabled();
    }
}

/// The RFC 4028 §5 floor, and what rvoip offers in these cells.
const SESSION_TIMER_OFFER_SECS: u32 = 90;

/// What one role of an RTCP cell must observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RtcpExpectation {
    /// The peer's SDP (answer for the caller, offer for the callee) carries
    /// `a=rtcp-mux`.
    peer_offers_mux: bool,
    /// RTCP from the peer reaches rvoip: `remote_*` and `rtt_ms` populate.
    peer_reports: bool,
}

fn rtcp_expectation(provider: PbxProvider, scenario: Scenario) -> ExampleResult<RtcpExpectation> {
    match (provider, scenario) {
        (_, Scenario::RtcpMux) => Ok(RtcpExpectation {
            peer_offers_mux: true,
            peer_reports: true,
        }),
        (PbxProvider::Asterisk, Scenario::RtcpNonMux) => Ok(RtcpExpectation {
            peer_offers_mux: false,
            peer_reports: true,
        }),
        (PbxProvider::Asterisk, Scenario::RtcpMuxDeclined) => Ok(RtcpExpectation {
            peer_offers_mux: false,
            peer_reports: false,
        }),
        (provider, scenario) => Err(format!(
            "{:?} has no {:?} cell: its release profile does not configure one",
            provider, scenario
        )
        .into()),
    }
}

/// The RTCP-relevant lines of an SDP body. Only these are written to the
/// evidence: a full SDP would carry the SDES keys of the TLS cells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SdpRtcpFacts {
    media_line: Option<String>,
    rtcp_mux: bool,
    rtcp_attribute: Option<String>,
}

impl SdpRtcpFacts {
    fn parse(sdp: &str) -> Self {
        let mut facts = Self::default();
        for line in sdp.lines().map(str::trim) {
            if line.starts_with("m=audio") && facts.media_line.is_none() {
                facts.media_line = Some(line.to_string());
            } else if line == "a=rtcp-mux" {
                facts.rtcp_mux = true;
            } else if let Some(value) = line.strip_prefix("a=rtcp:") {
                facts.rtcp_attribute = Some(value.to_string());
            }
        }
        facts
    }
}

/// One redacted SIP message, reduced to the fields the timer cells assert.
#[derive(Debug, Clone)]
struct TraceLine {
    elapsed: Duration,
    inbound: bool,
    start_line: String,
    call_id: Option<String>,
    cseq: Option<(u32, String)>,
    session_expires: Option<String>,
    min_se: Option<String>,
    require: Option<String>,
}

impl TraceLine {
    fn from_trace(started: Instant, trace: &rvoip_sip::SipTrace) -> Self {
        let mut line = Self {
            elapsed: started.elapsed(),
            inbound: matches!(trace.direction, SipTraceDirection::Inbound),
            start_line: trace.start_line.clone(),
            call_id: trace.sip_call_id.clone(),
            cseq: None,
            session_expires: None,
            min_se: None,
            require: None,
        };
        for raw in trace.raw_message.lines().skip(1) {
            let raw = raw.trim_end();
            if raw.is_empty() {
                break;
            }
            let Some((name, value)) = raw.split_once(':') else {
                continue;
            };
            let value = value.trim().to_string();
            match name.trim().to_ascii_lowercase().as_str() {
                "cseq" => {
                    let mut parts = value.split_whitespace();
                    if let (Some(number), Some(method)) = (parts.next(), parts.next()) {
                        if let Ok(number) = number.parse() {
                            line.cseq = Some((number, method.to_ascii_uppercase()));
                        }
                    }
                }
                "session-expires" | "x" => line.session_expires = Some(value),
                "min-se" => line.min_se = Some(value),
                "require" => line.require = Some(value),
                _ => {}
            }
        }
        line
    }

    fn is_request(&self) -> bool {
        !self.start_line.starts_with("SIP/2.0")
    }

    fn status(&self) -> Option<u16> {
        self.start_line
            .strip_prefix("SIP/2.0 ")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|code| code.parse().ok())
    }

    fn method(&self) -> Option<&str> {
        self.cseq.as_ref().map(|(_, method)| method.as_str())
    }

    fn is_dialog_method(&self) -> bool {
        matches!(self.method(), Some("INVITE" | "UPDATE" | "ACK" | "BYE"))
    }

    fn session_expires_secs(&self) -> Option<u32> {
        self.session_expires
            .as_deref()
            .and_then(|value| value.split(';').next())
            .and_then(|secs| secs.trim().parse().ok())
    }

    fn refresher(&self) -> Option<String> {
        self.session_expires.as_deref().and_then(|value| {
            value.split(';').skip(1).find_map(|param| {
                let (name, value) = param.split_once('=')?;
                (name.trim().eq_ignore_ascii_case("refresher"))
                    .then(|| value.trim().to_ascii_lowercase())
            })
        })
    }

    fn render(&self) -> String {
        let mut out = format!(
            "+{:>7.3}s {} {}",
            self.elapsed.as_secs_f64(),
            if self.inbound { "<-" } else { "->" },
            self.start_line
        );
        if let Some((number, method)) = &self.cseq {
            out.push_str(&format!(" | CSeq: {number} {method}"));
        }
        if let Some(value) = &self.session_expires {
            out.push_str(&format!(" | Session-Expires: {value}"));
        }
        if let Some(value) = &self.min_se {
            out.push_str(&format!(" | Min-SE: {value}"));
        }
        if let Some(value) = &self.require {
            out.push_str(&format!(" | Require: {value}"));
        }
        out
    }
}

/// What the event collector has seen so far.
#[derive(Default)]
struct Collected {
    traces: Vec<TraceLine>,
    refreshed: Vec<(Duration, u32)>,
    refresh_failed: Vec<String>,
}

/// Collects redacted SIP traces and session-refresh events for one process.
/// Each role process owns exactly one call, so no Call-ID filtering is
/// needed beyond ignoring non-dialog methods (REGISTER, OPTIONS).
struct EventCollector {
    started: Instant,
    collected: Arc<Mutex<Collected>>,
    task: JoinHandle<()>,
}

impl EventCollector {
    async fn start(coordinator: &Arc<UnifiedCoordinator>) -> ExampleResult<Self> {
        let mut events = coordinator.events().await?;
        let started = Instant::now();
        let collected = Arc::new(Mutex::new(Collected::default()));
        let sink = collected.clone();
        let task = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let Ok(mut sink) = sink.lock() else {
                    return;
                };
                match event {
                    Event::SipTrace(trace) => {
                        let line = TraceLine::from_trace(started, &trace);
                        if line.is_dialog_method() {
                            sink.traces.push(line);
                        }
                    }
                    Event::SessionRefreshed { expires_secs, .. } => {
                        sink.refreshed.push((started.elapsed(), expires_secs));
                    }
                    Event::SessionRefreshFailed { reason, .. } => {
                        sink.refresh_failed.push(reason);
                    }
                    _ => {}
                }
            }
        });
        Ok(Self {
            started,
            collected,
            task,
        })
    }

    fn traces(&self) -> Vec<TraceLine> {
        self.collected
            .lock()
            .map(|collected| collected.traces.clone())
            .unwrap_or_default()
    }

    fn refreshed(&self) -> Vec<(Duration, u32)> {
        self.collected
            .lock()
            .map(|collected| collected.refreshed.clone())
            .unwrap_or_default()
    }

    fn refresh_failed(&self) -> Vec<String> {
        self.collected
            .lock()
            .map(|collected| collected.refresh_failed.clone())
            .unwrap_or_default()
    }

    fn stop(self) {
        self.task.abort();
    }
}

/// Entry point from `run_endpoint` for the four scenarios above.
pub async fn run_endpoint_role(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    endpoint: &mut Endpoint,
) -> ExampleResult<()> {
    let collector = EventCollector::start(endpoint.control().coordinator()).await?;
    let result = match role {
        Role::Caller => {
            settle_after_register(provider).await;
            let target = scenario_username(provider, scenario, transport, Role::Callee);
            let call =
                endpoint_call_and_wait(endpoint, cfg, &target, remote_test_timeout(provider)?)
                    .await?;
            let handle = call.as_session_handle();
            if scenario == Scenario::SessionTimer {
                session_timer_role(provider, cfg, handle, Role::Caller, &collector).await
            } else {
                let answer = handle
                    .lifecycle()
                    .await?
                    .answered
                    .and_then(|answered| answered.sdp);
                rtcp_role(
                    provider,
                    scenario,
                    cfg,
                    handle,
                    transport,
                    Role::Caller,
                    answer,
                )
                .await
            }
        }
        Role::Callee => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let offer = incoming.offer_sdp().map(str::to_owned);
            let call = incoming.accept().await?;
            let handle = call.as_session_handle();
            if scenario == Scenario::SessionTimer {
                session_timer_role(provider, cfg, handle, Role::Callee, &collector).await
            } else {
                rtcp_role(
                    provider,
                    scenario,
                    cfg,
                    handle,
                    transport,
                    Role::Callee,
                    offer,
                )
                .await
            }
        }
        other => Err(format!("{:?} has no {:?} role", scenario, other).into()),
    };
    write_trace_evidence(cfg, role, &collector);
    collector.stop();
    result
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Caller => "caller",
        Role::Callee => "callee",
        _ => "other",
    }
}

fn write_trace_evidence(cfg: &EndpointConfig, role: Role, collector: &EventCollector) {
    let mut out = String::from(
        "# SIP trace (rvoip default redaction: no Request-URIs, bodies, or credentials)\n",
    );
    for line in collector.traces() {
        out.push_str(&line.render());
        out.push('\n');
    }
    for (at, secs) in collector.refreshed() {
        out.push_str(&format!(
            "+{:>7.3}s event SessionRefreshed expires_secs={secs}\n",
            at.as_secs_f64()
        ));
    }
    for reason in collector.refresh_failed() {
        out.push_str(&format!("event SessionRefreshFailed reason={reason}\n"));
    }
    let _ = std::fs::create_dir_all(&cfg.output_dir);
    let _ = std::fs::write(
        cfg.output_dir
            .join(format!("{}_sip_trace.txt", role_label(role))),
        out,
    );
}

/// The file a role writes when it holds its verdict, so the other role (a
/// separate process) knows it may end the call.
fn done_marker(cfg: &EndpointConfig, role: Role) -> PathBuf {
    cfg.output_dir.join(format!("{}.done", role_label(role)))
}

fn write_done(cfg: &EndpointConfig, role: Role, verdict: &ExampleResult<()>) {
    let text = match verdict {
        Ok(()) => "PASS\n".to_string(),
        Err(error) => format!("FAIL {error}\n"),
    };
    let _ = std::fs::create_dir_all(&cfg.output_dir);
    let _ = std::fs::write(done_marker(cfg, role), text);
}

async fn wait_for_done(cfg: &EndpointConfig, role: Role, limit: Duration) -> ExampleResult<()> {
    let path = done_marker(cfg, role);
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if text.starts_with("PASS") {
                return Ok(());
            }
            if text.starts_with("FAIL") {
                return Err(format!("{} reported {}", role_label(role), text.trim_end()).into());
            }
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!(
        "{} did not report a verdict within {:?}",
        role_label(role),
        limit
    )
    .into())
}

/// Keep the call up while `run.sh` takes the PBX's view of it.
///
/// Prints [`PBX_EVIDENCE_MARKER`], then, under `run.sh`
/// (`PBX_EVIDENCE_HANDSHAKE=1`), waits for the runner to write `pbx.done`
/// once the PBX's own log is complete -- its reports are periodic, so it
/// can trail rvoip by an interval. Run by hand, it just holds a few seconds.
async fn hold_for_pbx_evidence(cfg: &EndpointConfig) {
    println!("{PBX_EVIDENCE_MARKER}");
    if !env_bool("PBX_EVIDENCE_HANDSHAKE", false).unwrap_or(false) {
        sleep(env_secs("PBX_EVIDENCE_HOLD_SECS", 5)).await;
        return;
    }
    let done = cfg.output_dir.join("pbx.done");
    let deadline = Instant::now() + env_secs("PBX_EVIDENCE_WAIT_SECS", 60);
    while !done.exists() && Instant::now() < deadline {
        sleep(Duration::from_millis(250)).await;
    }
}

fn env_secs(key: &str, default: u64) -> Duration {
    env_duration_secs(key, default)
}

fn quality_json(quality: Option<&MediaQualityStats>) -> serde_json::Value {
    match quality {
        None => serde_json::Value::Null,
        Some(q) => serde_json::json!({
            "packets_sent": q.packets_sent,
            "packets_received": q.packets_received,
            "packets_lost": q.packets_lost,
            "jitter_ms": q.jitter_ms,
            "mos": q.mos,
            "rtt_ms": q.rtt_ms,
            "remote_packet_loss_percent": q.remote_packet_loss_percent,
            "remote_packets_lost": q.remote_packets_lost,
            "remote_jitter_ms": q.remote_jitter_ms,
        }),
    }
}

/// One role of an RTCP cell.
async fn rtcp_role(
    provider: PbxProvider,
    scenario: Scenario,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    role: Role,
    peer_sdp: Option<String>,
) -> ExampleResult<()> {
    let expectation = rtcp_expectation(provider, scenario)?;
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let tone = match role {
        Role::Caller => tone_for_caller(transport),
        _ => tone_for_callee(transport),
    };
    // RTCP senders' reports need RTP to describe, and RTT needs the peer to
    // reflect one of our SRs, so both ends stream a tone throughout.
    let recorder = start_tone_recorder(handle, tone).await?;
    let started = Instant::now();
    let facts = SdpRtcpFacts::parse(peer_sdp.as_deref().unwrap_or_default());

    let verdict: ExampleResult<Option<MediaQualityStats>> = async {
        recorder
            .wait_for_received_samples(TONE_ANALYSIS_WINDOW_SAMPLES, Duration::from_secs(8))
            .await?;
        if peer_sdp.is_none() {
            return Err("the peer's SDP was not available to check rtcp-mux".into());
        }
        if facts.rtcp_mux != expectation.peer_offers_mux {
            return Err(format!(
                "peer SDP {} a=rtcp-mux, expected {}",
                if facts.rtcp_mux { "carries" } else { "lacks" },
                if expectation.peer_offers_mux {
                    "it"
                } else {
                    "none"
                }
            )
            .into());
        }
        if expectation.peer_reports {
            // RFC 3550 reports go out every ~2-6 s once media flows; RTT also
            // needs the peer to answer one of our SRs.
            let limit = env_secs("PBX_RTCP_REPORT_DEADLINE_SECS", 30);
            let deadline = Instant::now() + limit;
            loop {
                let quality = handle.media_quality().await;
                if let Some(q) = quality.as_ref() {
                    if q.has_peer_report() && q.rtt_ms.is_some() {
                        return Ok(quality);
                    }
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "no peer RTCP report with RTT within {:?}; last media_quality: {}",
                        limit,
                        quality_json(quality.as_ref())
                    )
                    .into());
                }
                sleep(Duration::from_millis(500)).await;
            }
        } else {
            // Two-plus RFC 3550 intervals of silence: a report would have
            // arrived by now had the peer been able to reach us.
            let quiet = env_secs("PBX_RTCP_QUIET_SECS", 14);
            sleep(quiet).await;
            let quality = handle.media_quality().await;
            match quality.as_ref() {
                Some(q) if q.has_peer_report() || q.rtt_ms.is_some() => Err(format!(
                    "peer RTCP arrived although mux was declined and rtcp_non_mux is off: {}",
                    quality_json(quality.as_ref())
                )
                .into()),
                Some(q) if q.packets_received == 0 => {
                    Err("no RTP received; the quiet window proves nothing".into())
                }
                Some(_) => Ok(quality),
                None => Err("media_quality() returned None on an active call".into()),
            }
        }
    }
    .await;

    let (verdict, quality) = match verdict {
        Ok(quality) => (Ok(()), quality),
        Err(error) => (Err(error), None),
    };
    let evidence = serde_json::json!({
        "provider": provider.env_name(),
        "scenario": format!("{:?}", scenario),
        "role": role_label(role),
        "transport": transport.env_value(),
        "expected_peer_rtcp_mux": expectation.peer_offers_mux,
        "expected_peer_reports": expectation.peer_reports,
        "peer_sdp_m_audio": facts.media_line,
        "peer_sdp_rtcp_mux": facts.rtcp_mux,
        "peer_sdp_rtcp_attribute": facts.rtcp_attribute,
        "observed_after_ms": started.elapsed().as_millis() as u64,
        "media_quality": quality_json(quality.as_ref()),
        "verdict": match &verdict { Ok(()) => "PASS".to_string(), Err(e) => format!("FAIL: {e}") },
    });
    let _ = std::fs::create_dir_all(&cfg.output_dir);
    let _ = std::fs::write(
        cfg.output_dir
            .join(format!("{}_rtcp_evidence.json", role_label(role))),
        serde_json::to_string_pretty(&evidence).unwrap_or_default(),
    );
    println!("[{}] RTCP evidence: {}", cfg.username, evidence);
    write_done(cfg, role, &verdict);

    let outcome = match role {
        Role::Caller => {
            let peer = if verdict.is_ok() {
                wait_for_done(
                    cfg,
                    Role::Callee,
                    env_secs("PBX_RTCP_PEER_VERDICT_SECS", 45),
                )
                .await
            } else {
                Ok(())
            };
            if verdict.is_ok() && peer.is_ok() {
                hold_for_pbx_evidence(cfg).await;
            }
            handle
                .hangup_and_wait(Some(Duration::from_secs(8)))
                .await
                .ok();
            verdict.and(peer)
        }
        _ => {
            handle
                .wait_for_end(Some(
                    remote_test_timeout(provider)? + Duration::from_secs(60),
                ))
                .await
                .ok();
            verdict
        }
    };
    recorder
        .stop_and_save(
            &cfg.output_dir,
            &format!("{}_{}_received.wav", role_label(role), cfg.username),
        )
        .await?;
    outcome
}

/// One role of the session-timer cell.
///
/// The caller is the refresher on its own (UAC) leg: the PBX's 2xx must name
/// `refresher=uac`, and rvoip must refresh at least once. The callee answers
/// the PBX's timer-capable INVITE; rvoip names the PBX (the UAC of that leg)
/// as refresher, and must receive and accept at least one refresh. Each role
/// then holds the call past the point where a missed refresh on its leg
/// would have ended it (RFC 4028 §10: the non-refresher sends BYE at
/// `interval - min(32, interval / 3)`).
async fn session_timer_role(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    role: Role,
    collector: &EventCollector,
) -> ExampleResult<()> {
    let recorder = start_tone_recorder(handle, tone_for_caller(TransportMode::Udp)).await?;
    let answered_at = Instant::now();
    let limit = env_secs("PBX_SESSION_TIMER_DEADLINE_SECS", 150);

    let verdict: ExampleResult<String> = async {
        // The interval this leg agreed on, from the final 2xx to the initial
        // INVITE (the one with the lowest CSeq that got a 2xx).
        let negotiated = loop {
            if let Some(found) = negotiated_interval(&collector.traces(), role) {
                break found;
            }
            if answered_at.elapsed() > Duration::from_secs(10) {
                return Err("no 2xx to the initial INVITE in the SIP trace".into());
            }
            sleep(Duration::from_millis(100)).await;
        };
        let (interval, refresher) = negotiated;
        // RFC 4028 §7.4 roles: rvoip refreshes the leg it originated; the
        // PBX refreshes the leg it originated toward rvoip.
        if refresher.as_deref() != Some("uac") {
            return Err(format!(
                "2xx to the initial INVITE named refresher={:?}, expected uac on this leg",
                refresher
            )
            .into());
        }
        let survive =
            Duration::from_secs(u64::from(interval.saturating_sub(32.min(interval / 3)) + 5));
        loop {
            match handle.state().await {
                Ok(state) if !state.is_final() && state != CallState::Terminating => {}
                Ok(state) => {
                    return Err(format!(
                        "call left Active ({state:?}) {:.1}s after answer; refreshes so far: {}",
                        answered_at.elapsed().as_secs_f64(),
                        refresh_summary(collector, role)
                    )
                    .into())
                }
                Err(error) => {
                    return Err(format!(
                        "call ended {:.1}s after answer ({error}); refreshes so far: {}",
                        answered_at.elapsed().as_secs_f64(),
                        refresh_summary(collector, role)
                    )
                    .into())
                }
            }
            let refreshes = refreshes_on_leg(&collector.traces(), role);
            let refreshed_event = role != Role::Caller || !collector.refreshed().is_empty();
            if !collector.refresh_failed().is_empty() {
                return Err(
                    format!("SessionRefreshFailed: {:?}", collector.refresh_failed()).into(),
                );
            }
            if refreshes >= 1 && refreshed_event && answered_at.elapsed() >= survive {
                return Ok(format!(
                    "interval={interval}s refresher=uac refreshes={refreshes} \
                     survived={:.1}s (non-refresher BYE would have been due at {}s)",
                    answered_at.elapsed().as_secs_f64(),
                    interval.saturating_sub(32.min(interval / 3))
                ));
            }
            if answered_at.elapsed() > limit {
                return Err(format!(
                    "no completed refresh on this leg within {:?}: {}",
                    limit,
                    refresh_summary(collector, role)
                )
                .into());
            }
            sleep(Duration::from_millis(500)).await;
        }
    }
    .await;

    // The 422/Min-SE exchange is FreeSWITCH's (Min-SE 120 > our 90). Asterisk
    // accepts 90 and must not send one.
    let verdict = verdict.and_then(|summary| {
        if role != Role::Caller {
            return Ok(summary);
        }
        let traces = collector.traces();
        let rejected = traces
            .iter()
            .find(|line| line.inbound && line.status() == Some(422));
        match (provider, rejected) {
            (PbxProvider::FreeSwitch, None) => {
                Err("FreeSWITCH (Min-SE 120) did not answer the 90 s offer with 422".into())
            }
            (PbxProvider::FreeSwitch, Some(rejected)) => {
                let min_se = rejected
                    .min_se
                    .as_deref()
                    .and_then(|value| value.split(';').next())
                    .and_then(|value| value.trim().parse::<u32>().ok())
                    .ok_or("422 carried no Min-SE")?;
                let retried = traces.iter().any(|line| {
                    !line.inbound
                        && line.is_request()
                        && line.method() == Some("INVITE")
                        && line
                            .cseq
                            .as_ref()
                            .zip(rejected.cseq.as_ref())
                            .is_some_and(|(retry, first)| retry.0 > first.0)
                        && line
                            .session_expires_secs()
                            .is_some_and(|secs| secs >= min_se)
                });
                if retried {
                    Ok(format!("{summary} 422 Min-SE={min_se} retried"))
                } else {
                    Err(
                        format!("no INVITE retry with Session-Expires >= {min_se} after 422")
                            .into(),
                    )
                }
            }
            (_, Some(_)) => Err("unexpected 422 Session Interval Too Small".into()),
            (_, None) => Ok(summary),
        }
    });

    let verdict = match verdict {
        Ok(summary) => {
            println!("[{}] session timer: PASS {summary}", cfg.username);
            Ok(())
        }
        Err(error) => Err(error),
    };
    write_done(cfg, role, &verdict);
    let outcome = match role {
        Role::Caller => {
            let peer = if verdict.is_ok() {
                wait_for_done(cfg, Role::Callee, limit).await
            } else {
                Ok(())
            };
            if verdict.is_ok() && peer.is_ok() {
                hold_for_pbx_evidence(cfg).await;
            }
            handle
                .hangup_and_wait(Some(Duration::from_secs(8)))
                .await
                .ok();
            verdict.and(peer)
        }
        _ => {
            handle
                .wait_for_end(Some(limit + Duration::from_secs(60)))
                .await
                .ok();
            verdict
        }
    };
    recorder
        .stop_and_save(
            &cfg.output_dir,
            &format!("{}_{}_received.wav", role_label(role), cfg.username),
        )
        .await?;
    outcome
}

/// The session interval and refresher of this leg, from the 2xx to its
/// initial INVITE. On the caller the 2xx is inbound; on the callee rvoip
/// sends it.
fn negotiated_interval(traces: &[TraceLine], role: Role) -> Option<(u32, Option<String>)> {
    let response_inbound = role == Role::Caller;
    let initial = traces
        .iter()
        .filter(|line| {
            line.inbound == response_inbound
                && line.method() == Some("INVITE")
                && line.status().is_some_and(|code| (200..300).contains(&code))
        })
        .min_by_key(|line| line.cseq.as_ref().map(|(number, _)| *number))?;
    Some((initial.session_expires_secs()?, initial.refresher()))
}

/// Completed refreshes on this role's leg: an in-dialog INVITE or UPDATE
/// after the initial INVITE, answered 2xx with a Session-Expires. The
/// caller sends them; the callee receives them.
fn refreshes_on_leg(traces: &[TraceLine], role: Role) -> usize {
    let request_inbound = role != Role::Caller;
    let Some(first_ok) = traces
        .iter()
        .filter(|line| {
            line.method() == Some("INVITE")
                && line.status().is_some_and(|code| (200..300).contains(&code))
        })
        .filter_map(|line| line.cseq.as_ref().map(|(number, _)| *number))
        .min()
    else {
        return 0;
    };
    traces
        .iter()
        .filter(|request| {
            request.inbound == request_inbound
                && request.is_request()
                && matches!(request.method(), Some("INVITE" | "UPDATE"))
                && request.cseq.as_ref().is_some_and(|(number, _)| {
                    // On the callee the PBX's CSeq space is its own, so any
                    // later request than its initial INVITE counts.
                    *number > first_ok
                })
        })
        .filter(|request| {
            traces.iter().any(|response| {
                response.inbound != request_inbound
                    && response.cseq == request.cseq
                    && response
                        .status()
                        .is_some_and(|code| (200..300).contains(&code))
                    && response.session_expires.is_some()
            })
        })
        .count()
}

fn refresh_summary(collector: &EventCollector, role: Role) -> String {
    format!(
        "{} completed on this leg, {} SessionRefreshed event(s)",
        refreshes_on_leg(&collector.traces(), role),
        collector.refreshed().len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_users_borrow_their_base_users() {
        assert_eq!(
            scenario_username(
                PbxProvider::Asterisk,
                Scenario::RtcpMux,
                TransportMode::Udp,
                Role::Caller
            ),
            "2101"
        );
        assert_eq!(
            scenario_username(
                PbxProvider::Asterisk,
                Scenario::RtcpMuxDeclined,
                TransportMode::TlsSrtp,
                Role::Callee
            ),
            "1202"
        );
        assert_eq!(
            scenario_username(
                PbxProvider::Asterisk,
                Scenario::SessionTimer,
                TransportMode::Udp,
                Role::Callee
            ),
            "2302"
        );
        // FreeSWITCH keeps its users and switches profile instead.
        assert_eq!(
            scenario_username(
                PbxProvider::FreeSwitch,
                Scenario::SessionTimer,
                TransportMode::Udp,
                Role::Caller
            ),
            "2001"
        );
        assert_eq!(base_username("2101"), "2001");
        assert_eq!(base_username("1203"), "1003");
        assert_eq!(base_username("2302"), "2002");
        assert_eq!(base_username("2001"), "2001");
        assert_eq!(base_username("12345"), "12345");
    }

    #[test]
    fn sdp_facts_keep_only_rtcp_lines() {
        let facts = SdpRtcpFacts::parse(
            "v=0\r\nm=audio 10000 RTP/SAVP 0 101\r\na=rtcp:10001 IN IP4 10.0.0.1\r\n\
             a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:secret\r\na=rtcp-mux\r\n",
        );
        assert_eq!(
            facts.media_line.as_deref(),
            Some("m=audio 10000 RTP/SAVP 0 101")
        );
        assert!(facts.rtcp_mux);
        assert_eq!(
            facts.rtcp_attribute.as_deref(),
            Some("10001 IN IP4 10.0.0.1")
        );
    }

    fn line(inbound: bool, start: &str, cseq: (u32, &str), se: Option<&str>) -> TraceLine {
        TraceLine {
            elapsed: Duration::ZERO,
            inbound,
            start_line: start.to_string(),
            call_id: Some("c".into()),
            cseq: Some((cseq.0, cseq.1.to_string())),
            session_expires: se.map(str::to_string),
            min_se: None,
            require: None,
        }
    }

    #[test]
    fn refresh_counting_follows_the_leg_direction() {
        // Caller leg after a 422 retry: CSeq 1 rejected, 2 answered, the
        // UPDATE at 3 is rvoip's refresh.
        let caller = vec![
            line(
                false,
                "INVITE <redacted> SIP/2.0",
                (1, "INVITE"),
                Some("90"),
            ),
            line(
                true,
                "SIP/2.0 422 Session Interval Too Small",
                (1, "INVITE"),
                None,
            ),
            line(
                false,
                "INVITE <redacted> SIP/2.0",
                (2, "INVITE"),
                Some("120"),
            ),
            line(
                true,
                "SIP/2.0 200 OK",
                (2, "INVITE"),
                Some("120;refresher=uac"),
            ),
            line(
                false,
                "UPDATE <redacted> SIP/2.0",
                (3, "UPDATE"),
                Some("120;refresher=uac"),
            ),
            line(
                true,
                "SIP/2.0 200 OK",
                (3, "UPDATE"),
                Some("120;refresher=uac"),
            ),
        ];
        assert_eq!(
            negotiated_interval(&caller, Role::Caller),
            Some((120, Some("uac".into())))
        );
        assert_eq!(refreshes_on_leg(&caller, Role::Caller), 1);
        assert_eq!(refreshes_on_leg(&caller, Role::Callee), 0);

        // Callee leg: the PBX re-INVITEs and rvoip answers.
        let callee = vec![
            line(
                true,
                "INVITE <redacted> SIP/2.0",
                (100, "INVITE"),
                Some("90"),
            ),
            line(
                false,
                "SIP/2.0 200 OK",
                (100, "INVITE"),
                Some("90;refresher=uac"),
            ),
            line(
                true,
                "INVITE <redacted> SIP/2.0",
                (101, "INVITE"),
                Some("90;refresher=uac"),
            ),
            line(
                false,
                "SIP/2.0 200 OK",
                (101, "INVITE"),
                Some("90;refresher=uac"),
            ),
        ];
        assert_eq!(
            negotiated_interval(&callee, Role::Callee),
            Some((90, Some("uac".into())))
        );
        assert_eq!(refreshes_on_leg(&callee, Role::Callee), 1);
    }
}
