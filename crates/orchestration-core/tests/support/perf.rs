use rvoip_orchestration_core::prelude::*;
use rvoip_orchestration_core::{AgentOfferId, BridgeId};
use rvoip_session_core::{Config, SessionId, UnifiedCoordinator};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{sleep, timeout};

pub const ACTIVE_CALL_COUNTS: [usize; 3] = [100, 500, 1000];
pub const DEFAULT_LIVE_SIP_RTP_CALLS: usize = 5;
pub const DEFAULT_LIVE_SIP_RTP_HOLD_SECS: u64 = 5;
const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;
const FRAME_MS: u64 = 20;
static NEXT_PORT_BLOCK: AtomicUsize = AtomicUsize::new(24_000);

pub struct ActiveCallScenario {
    pub orchestrator: Orchestrator,
    pub handle: OrchestrationHandle,
    pub queue_id: QueueId,
    pub call_ids: Vec<CallId>,
    pub offer_ids: Vec<AgentOfferId>,
}

#[derive(Clone)]
struct RoundRobinAgentRouter {
    agent_ids: Arc<Vec<AgentId>>,
    next: Arc<AtomicUsize>,
}

impl RoundRobinAgentRouter {
    fn new(agent_ids: Vec<AgentId>) -> Self {
        Self {
            agent_ids: Arc::new(agent_ids),
            next: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Router for RoundRobinAgentRouter {
    async fn route(&self, _request: RouteRequest) -> Result<RouteDecision> {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.agent_ids.len();
        Ok(RouteDecision::OfferAgent {
            agent_id: self.agent_ids[index].clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ActiveCallProfile {
    pub active_calls: usize,
    pub connected_calls: usize,
    pub queued_calls_remaining: usize,
    pub wall_time: Duration,
    pub cpu_time: Option<Duration>,
    pub rss_before_bytes: Option<u64>,
    pub rss_after_bytes: Option<u64>,
    pub rss_delta_bytes: Option<i64>,
    pub bytes_per_active_call: Option<f64>,
}

impl ActiveCallProfile {
    pub fn calls_per_second_wall(&self) -> f64 {
        self.active_calls as f64 / self.wall_time.as_secs_f64().max(f64::EPSILON)
    }

    pub fn calls_per_second_cpu(&self) -> Option<f64> {
        self.cpu_time
            .map(|cpu| self.active_calls as f64 / cpu.as_secs_f64().max(f64::EPSILON))
    }

    pub fn report_line(&self) -> String {
        format!(
            "active_calls={} connected={} queued_remaining={} wall_ms={:.2} cpu_ms={} calls_per_sec_wall={:.2} calls_per_sec_cpu={} rss_before={} rss_after={} rss_delta={} bytes_per_call={}",
            self.active_calls,
            self.connected_calls,
            self.queued_calls_remaining,
            millis(self.wall_time),
            format_optional_millis(self.cpu_time),
            self.calls_per_second_wall(),
            format_optional_f64(self.calls_per_second_cpu()),
            format_optional_bytes(self.rss_before_bytes.map(|bytes| bytes as i64)),
            format_optional_bytes(self.rss_after_bytes.map(|bytes| bytes as i64)),
            format_optional_bytes(self.rss_delta_bytes),
            format_optional_f64(self.bytes_per_active_call),
        )
    }
}

#[derive(Debug, Clone)]
pub struct LiveSipRtpProfile {
    pub active_calls: usize,
    pub hold_duration: Duration,
    pub connected_calls: usize,
    pub active_bridges: usize,
    pub caller_received_frames: usize,
    pub agent_received_frames: usize,
    pub setup_wall_time: Duration,
    pub media_wall_time: Duration,
    pub total_wall_time: Duration,
    pub setup_and_media_cpu_time: Option<Duration>,
    pub media_cpu_time: Option<Duration>,
    pub rss_before_bytes: Option<u64>,
    pub rss_active_bytes: Option<u64>,
    pub rss_after_media_bytes: Option<u64>,
    pub rss_active_delta_bytes: Option<i64>,
    pub rss_after_media_delta_bytes: Option<i64>,
    pub active_bytes_per_call: Option<f64>,
}

impl LiveSipRtpProfile {
    pub fn report_line(&self) -> String {
        format!(
            "live_sip_rtp_calls={} connected={} active_bridges={} hold_secs={:.2} setup_ms={:.2} media_wall_ms={:.2} total_wall_ms={:.2} total_cpu_ms={} media_cpu_ms={} caller_rx_frames={} agent_rx_frames={} rss_before={} rss_active={} rss_after_media={} rss_active_delta={} rss_after_media_delta={} active_bytes_per_call={}",
            self.active_calls,
            self.connected_calls,
            self.active_bridges,
            self.hold_duration.as_secs_f64(),
            millis(self.setup_wall_time),
            millis(self.media_wall_time),
            millis(self.total_wall_time),
            format_optional_millis(self.setup_and_media_cpu_time),
            format_optional_millis(self.media_cpu_time),
            self.caller_received_frames,
            self.agent_received_frames,
            format_optional_bytes(self.rss_before_bytes.map(|bytes| bytes as i64)),
            format_optional_bytes(self.rss_active_bytes.map(|bytes| bytes as i64)),
            format_optional_bytes(self.rss_after_media_bytes.map(|bytes| bytes as i64)),
            format_optional_bytes(self.rss_active_delta_bytes),
            format_optional_bytes(self.rss_after_media_delta_bytes),
            format_optional_f64(self.active_bytes_per_call),
        )
    }

    pub fn media_cpu_per_call(&self) -> Option<Duration> {
        self.media_cpu_time
            .map(|cpu| Duration::from_secs_f64(cpu.as_secs_f64() / self.active_calls.max(1) as f64))
    }

    pub fn active_memory_per_call_bytes(&self) -> Option<f64> {
        self.active_bytes_per_call
    }
}

pub async fn build_active_call_scenario(active_calls: usize) -> Result<ActiveCallScenario> {
    let mut config = OrchestrationConfig::default();
    config.events.channel_capacity = active_calls.saturating_mul(8).max(1024);
    config.assignment.max_attempts_per_call = active_calls as u32 + 1;

    let orchestrator = Orchestrator::builder().with_config(config).build().await?;
    let handle = orchestrator.handle();
    let queue_id = QueueId::from(format!("perf-active-{active_calls}"));
    let support_skill = Skill::from("support");

    handle
        .upsert_queue(Queue::new(queue_id.clone(), "Performance Queue"))
        .await?;

    for index in 0..active_calls {
        let agent_id = format!("perf-agent-{index:04}");
        let mut agent = Agent::human(
            agent_id.clone(),
            format!("sip:{agent_id}@agents.perf.invalid"),
        );
        agent.state = AgentState::Available;
        agent.skills.push(support_skill.clone());
        handle.upsert_agent(agent).await?;
    }

    let mut call_ids = Vec::with_capacity(active_calls);
    for index in 0..active_calls {
        let mut call = Call::inbound(
            CallerIdentity::new(format!("sip:caller-{index:04}@customers.perf.invalid")),
            "sip:support@example.com",
        );
        call.context.external_ref = Some(format!("perf-call-{index:04}"));
        call.context
            .metadata
            .insert("perf_index".to_string(), index.to_string());
        let call_id = call.id.clone();
        handle.create_call(call).await?;
        handle
            .enqueue_call(
                call_id.clone(),
                QueueTarget {
                    queue_id: queue_id.clone(),
                    required_skills: vec![support_skill.clone()],
                    ..QueueTarget::default()
                },
            )
            .await?;
        call_ids.push(call_id);
    }

    let mut offer_ids = Vec::with_capacity(active_calls);
    for _ in 0..active_calls {
        let assignment = handle.assign_next_call(&queue_id).await?.ok_or_else(|| {
            OrchestrationError::InvalidState(format!(
                "expected assignment while building {active_calls} active calls"
            ))
        })?;
        handle.accept_offer(&assignment.offer_id).await?;
        offer_ids.push(assignment.offer_id);
    }

    Ok(ActiveCallScenario {
        orchestrator,
        handle,
        queue_id,
        call_ids,
        offer_ids,
    })
}

pub async fn profile_active_calls(active_calls: usize) -> Result<ActiveCallProfile> {
    let rss_before_bytes = current_rss_bytes();
    let cpu_before = process_cpu_time();
    let started_at = Instant::now();

    let scenario = build_active_call_scenario(active_calls).await?;
    let wall_time = started_at.elapsed();
    let cpu_time = cpu_before
        .zip(process_cpu_time())
        .and_then(|(before, after)| after.checked_sub(before));

    let mut connected_calls = 0;
    for call_id in &scenario.call_ids {
        let call = scenario
            .handle
            .get_call(call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        if call.status == CallStatus::Connected {
            connected_calls += 1;
        }
    }

    let queued_calls_remaining = scenario
        .handle
        .get_queue_stats(&scenario.queue_id)
        .await?
        .queued_calls;

    let rss_after_bytes = current_rss_bytes();
    let rss_delta_bytes = rss_before_bytes
        .zip(rss_after_bytes)
        .map(|(before, after)| after as i64 - before as i64);
    let bytes_per_active_call =
        rss_delta_bytes.map(|delta| delta as f64 / active_calls.max(1) as f64);

    std::hint::black_box(&scenario.orchestrator);
    std::hint::black_box(&scenario.offer_ids);

    Ok(ActiveCallProfile {
        active_calls,
        connected_calls,
        queued_calls_remaining,
        wall_time,
        cpu_time,
        rss_before_bytes,
        rss_after_bytes,
        rss_delta_bytes,
        bytes_per_active_call,
    })
}

struct LiveSession {
    call_id: CallId,
    caller_session_id: SessionId,
    agent_session_id: SessionId,
    bridge_id: BridgeId,
    caller_coord: Arc<UnifiedCoordinator>,
    agent_coord: Arc<UnifiedCoordinator>,
}

pub fn live_sip_rtp_counts_from_env() -> Vec<usize> {
    std::env::var("RVOIP_LIVE_SIP_RTP_COUNTS")
        .or_else(|_| std::env::var("RVOIP_LIVE_SIP_RTP_CALLS"))
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse::<usize>().ok())
                .filter(|count| *count > 0)
                .collect::<Vec<_>>()
        })
        .filter(|counts| !counts.is_empty())
        .unwrap_or_else(|| vec![DEFAULT_LIVE_SIP_RTP_CALLS])
}

pub fn live_sip_rtp_hold_duration_from_env() -> Duration {
    let secs = std::env::var("RVOIP_LIVE_SIP_RTP_HOLD_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LIVE_SIP_RTP_HOLD_SECS)
        .max(DEFAULT_LIVE_SIP_RTP_HOLD_SECS);
    Duration::from_secs(secs)
}

pub async fn profile_live_sip_rtp(
    active_calls: usize,
    hold_duration: Duration,
) -> Result<LiveSipRtpProfile> {
    let hold_duration = hold_duration.max(Duration::from_secs(DEFAULT_LIVE_SIP_RTP_HOLD_SECS));
    let total_started_at = Instant::now();
    let rss_before_bytes = current_rss_bytes();
    let cpu_before = process_cpu_time();

    let orchestrator_config = live_session_config("perf-orchestrator", active_calls, 4);

    // One simulator endpoint per call. Each caller and each agent has its own
    // UnifiedCoordinator with its own SIP/media ports — no shared incoming-call
    // mpsc on the simulator side, so concurrent setup is bounded only by the
    // orchestrator's processing capacity.
    let mut coord_tasks: Vec<
        tokio::task::JoinHandle<rvoip_session_core::Result<(Arc<UnifiedCoordinator>, Arc<UnifiedCoordinator>, Config, Config)>>,
    > = Vec::with_capacity(active_calls);
    for index in 0..active_calls {
        let caller_cfg = live_session_config(&format!("perf-caller-{index:04}"), 1, 2);
        let agent_cfg = live_session_config(&format!("perf-agent-{index:04}"), 1, 2);
        let caller_cfg_clone = caller_cfg.clone();
        let agent_cfg_clone = agent_cfg.clone();
        coord_tasks.push(tokio::spawn(async move {
            let caller = UnifiedCoordinator::new(caller_cfg_clone.clone()).await?;
            let agent = UnifiedCoordinator::new(agent_cfg_clone.clone()).await?;
            Ok((caller, agent, caller_cfg_clone, agent_cfg_clone))
        }));
        std::hint::black_box(&caller_cfg);
        std::hint::black_box(&agent_cfg);
    }
    let mut caller_coords: Vec<Arc<UnifiedCoordinator>> = Vec::with_capacity(active_calls);
    let mut agent_coords: Vec<Arc<UnifiedCoordinator>> = Vec::with_capacity(active_calls);
    let mut caller_configs: Vec<Config> = Vec::with_capacity(active_calls);
    let mut agent_configs: Vec<Config> = Vec::with_capacity(active_calls);
    for task in coord_tasks {
        let (caller, agent, caller_cfg, agent_cfg) = task
            .await
            .map_err(|error| OrchestrationError::InvalidState(error.to_string()))??;
        caller_coords.push(caller);
        agent_coords.push(agent);
        caller_configs.push(caller_cfg);
        agent_configs.push(agent_cfg);
    }

    // Reservations made early in the ramp must survive until the last call's
    // bridge completes. Inbound delivery currently paces at ~500 ms / INVITE in
    // session-core, so for N=1000 the slowest path can exceed the 30 s default.
    // Scale the timeouts with N (with a generous floor) instead of relying on
    // the default.
    let setup_timeout = live_setup_timeout(active_calls);
    let assignment_budget = setup_timeout.max(Duration::from_secs(60));
    let mut orch_config = OrchestrationConfig {
        session: orchestrator_config.clone(),
        ..OrchestrationConfig::default()
    };
    orch_config.assignment.offer_timeout = assignment_budget;
    orch_config.assignment.outbound_answer_timeout = assignment_budget;

    let mut agent_ids = Vec::with_capacity(active_calls);
    let mut builder = Orchestrator::builder()
        .with_config(orch_config)
        .with_session_config(orchestrator_config.clone());

    for index in 0..active_calls {
        let agent_id = AgentId::from(format!("live-agent-{index:04}"));
        let agent_uri = format!("sip:{agent_id}@127.0.0.1:{}", agent_configs[index].sip_port);
        let mut human = Agent::human(agent_id.clone(), agent_uri);
        human.state = AgentState::Available;
        human.skills.push(Skill::from("support"));
        agent_ids.push(agent_id);
        builder = builder.with_agent(human);
    }
    builder = builder.with_router(RoundRobinAgentRouter::new(agent_ids));

    let orchestrator = Arc::new(builder.build().await?);
    let handle = orchestrator.handle();
    let events = orchestrator.events();
    let run_task = {
        let orchestrator = orchestrator.clone();
        tokio::spawn(async move { orchestrator.run().await })
    };

    let mut event_rx = events.subscribe();

    let phase_start = Instant::now();
    let mark_phase = |start: &mut Instant, label: &str| {
        let now = Instant::now();
        eprintln!(
            "perf phase {label}: {:.2} ms",
            now.duration_since(*start).as_secs_f64() * 1000.0
        );
        *start = now;
    };
    let mut phase = phase_start;

    // Phase A instrumentation: at N=1, log per-call timeline so we can localize
    // the 2 s setup floor. Set `RVOIP_LIVE_SIP_RTP_TRACE=1` to enable.
    let trace_enabled = std::env::var("RVOIP_LIVE_SIP_RTP_TRACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let trace_t0 = phase_start;
    let trace = move |label: String| {
        if trace_enabled {
            let elapsed_ms = trace_t0.elapsed().as_secs_f64() * 1000.0;
            eprintln!("[trace {elapsed_ms:>8.2} ms] {label}");
        }
    };

    let mut make_call_tasks = Vec::with_capacity(active_calls);
    for index in 0..active_calls {
        let caller = caller_coords[index].clone();
        let from = caller_configs[index].local_uri.clone();
        let to = format!(
            "sip:support-{index:04}@127.0.0.1:{}",
            orchestrator_config.sip_port
        );
        let to_for_task = to.clone();
        let trace = trace.clone();
        make_call_tasks.push(tokio::spawn(async move {
            trace(format!("make_call started to={to_for_task}"));
            let result = caller.make_call(&from, &to_for_task).await;
            trace(format!("make_call returned to={to_for_task}"));
            result.map(|session_id| (to_for_task, caller, session_id))
        }));
    }

    mark_phase(&mut phase, "spawn_make_calls");

    // Pipeline three rate-limited streams in parallel:
    //   - orchestrator side: AgentReserved events arrive on event_rx as the orchestrator
    //     ingests inbound INVITEs from the caller. As each lands, immediately spawn the
    //     connect_agent_offer + wait_for_agent_offer_outcome chain so the orchestrator
    //     starts INVITE-ing the agent without waiting for all N reservations.
    //   - agent side: pull each incoming INVITE and accept concurrently; this overlaps
    //     with the reservation/connect ramp on the orchestrator side.
    //   - caller side: collect each caller session_id keyed by To URI for media routing.
    // Without this pipelining, the three phases serialize and total wall time is
    // roughly N * (orchestrator_ingest_per_call + agent_ingest_per_call).
    let orchestrator_trace = trace.clone();
    let orchestrator_pipeline = async {
        let mut outcome_tasks: Vec<tokio::task::JoinHandle<Result<(CallId, BridgeId)>>> =
            Vec::with_capacity(active_calls);
        let mut received = 0;
        while received < active_calls {
            let envelope = event_rx.recv().await.map_err(|error| {
                OrchestrationError::InvalidState(format!(
                    "orchestration event stream failed: {error}"
                ))
            })?;
            if let OrchestrationEvent::AgentReserved {
                call_id, offer_id, ..
            } = envelope.event
            {
                orchestrator_trace(format!("AgentReserved received call={call_id}"));
                let handle = handle.clone();
                let outcome_trace = orchestrator_trace.clone();
                let offer_id_for_trace = offer_id.clone();
                outcome_tasks.push(tokio::spawn(async move {
                    outcome_trace(format!("connect_agent_offer started offer={offer_id_for_trace}"));
                    handle.connect_agent_offer(&offer_id).await?;
                    outcome_trace(format!(
                        "connect_agent_offer returned offer={offer_id_for_trace}"
                    ));
                    let bridge_id = handle
                        .wait_for_agent_offer_outcome(&offer_id)
                        .await?
                        .ok_or_else(|| {
                            OrchestrationError::InvalidState(
                                "agent offer completed without bridge".to_string(),
                            )
                        })?;
                    outcome_trace(format!(
                        "wait_for_agent_offer_outcome returned offer={offer_id_for_trace}"
                    ));
                    Ok::<(CallId, BridgeId), OrchestrationError>((call_id, bridge_id))
                }));
                received += 1;
            }
        }
        Ok::<_, OrchestrationError>(outcome_tasks)
    };

    // Each agent_coord receives exactly one INVITE. Spawning per-coord tasks
    // means every agent get_incoming_call runs concurrently — no shared mpsc.
    let agent_trace = trace.clone();
    let agent_pipeline = async {
        let mut pull_tasks: Vec<
            tokio::task::JoinHandle<Result<(String, Arc<UnifiedCoordinator>, SessionId)>>,
        > = Vec::with_capacity(active_calls);
        for agent_coord in &agent_coords {
            let agent_coord = agent_coord.clone();
            let agent_trace = agent_trace.clone();
            pull_tasks.push(tokio::spawn(async move {
                let incoming = timeout(setup_timeout, agent_coord.get_incoming_call())
                    .await
                    .map_err(|_| {
                        OrchestrationError::InvalidState(
                            "timed out waiting for agent INVITE".into(),
                        )
                    })?
                    .ok_or_else(|| {
                        OrchestrationError::InvalidState("agent incoming-call stream closed".into())
                    })?;
                agent_trace(format!("agent get_incoming returned to={}", incoming.to));
                let session_id_for_trace = incoming.session_id.clone();
                agent_coord.accept_call(&incoming.session_id).await?;
                agent_trace(format!("agent accept_call returned session={session_id_for_trace}"));
                Ok::<_, OrchestrationError>((
                    canonical_sip_uri(&incoming.to),
                    agent_coord,
                    incoming.session_id,
                ))
            }));
        }
        let mut agent_session_ids: HashMap<String, (Arc<UnifiedCoordinator>, SessionId)> =
            HashMap::with_capacity(active_calls);
        for task in pull_tasks {
            let (uri, coord, session_id) = task
                .await
                .map_err(|error| OrchestrationError::InvalidState(error.to_string()))??;
            agent_session_ids.insert(uri, (coord, session_id));
        }
        Ok::<_, OrchestrationError>(agent_session_ids)
    };

    let caller_pipeline = async {
        let mut caller_session_ids: HashMap<String, (Arc<UnifiedCoordinator>, SessionId)> =
            HashMap::with_capacity(active_calls);
        for task in make_call_tasks {
            let (to_uri, coord, session_id) = task
                .await
                .map_err(|error| OrchestrationError::InvalidState(error.to_string()))??;
            caller_session_ids.insert(canonical_sip_uri(&to_uri), (coord, session_id));
        }
        Ok::<_, OrchestrationError>(caller_session_ids)
    };

    let (outcome_tasks, agent_session_ids, caller_session_ids) =
        tokio::try_join!(orchestrator_pipeline, agent_pipeline, caller_pipeline)?;
    mark_phase(&mut phase, "ingest_pipeline");

    let mut sessions = Vec::with_capacity(active_calls);
    for task in outcome_tasks {
        let (call_id, bridge_id) = timeout(setup_timeout, task)
            .await
            .map_err(|_| {
                OrchestrationError::InvalidState("timed out waiting for bridge outcome".into())
            })?
            .map_err(|error| OrchestrationError::InvalidState(error.to_string()))??;
        let call = handle
            .get_call(&call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(call_id.clone()))?;
        let dialed_key = canonical_sip_uri(&call.dialed_uri);
        let (caller_coord, caller_session_id) =
            caller_session_ids.get(&dialed_key).cloned().ok_or_else(|| {
                OrchestrationError::InvalidState(format!(
                    "no caller session captured for dialed_uri {} (canonical {dialed_key})",
                    call.dialed_uri
                ))
            })?;
        let agent_uri = call
            .legs
            .iter()
            .find(|leg| leg.role == CallLegRole::HumanAgent)
            .map(|leg| leg.uri.clone())
            .ok_or_else(|| {
                OrchestrationError::InvalidState(format!("call {call_id} has no agent leg"))
            })?;
        let agent_key = canonical_sip_uri(&agent_uri);
        let (agent_coord, agent_session_id) =
            agent_session_ids.get(&agent_key).cloned().ok_or_else(|| {
                OrchestrationError::InvalidState(format!(
                    "no agent session captured for agent uri {agent_uri} (canonical {agent_key})"
                ))
            })?;
        sessions.push(LiveSession {
            call_id,
            caller_session_id,
            agent_session_id,
            bridge_id,
            caller_coord,
            agent_coord,
        });
    }
    mark_phase(&mut phase, "outcome_and_bridge");

    let setup_wall_time = total_started_at.elapsed();
    let rss_active_bytes = current_rss_bytes();
    let media_cpu_before = process_cpu_time();
    let media_started_at = Instant::now();

    let mut receive_tasks = Vec::with_capacity(active_calls * 2);
    for session in &sessions {
        receive_tasks.push(tokio::spawn(count_received_audio_frames(
            session
                .caller_coord
                .subscribe_to_audio(&session.caller_session_id)
                .await?,
            hold_duration + Duration::from_secs(1),
        )));
        receive_tasks.push(tokio::spawn(count_received_audio_frames(
            session
                .agent_coord
                .subscribe_to_audio(&session.agent_session_id)
                .await?,
            hold_duration + Duration::from_secs(1),
        )));
    }

    let mut send_tasks = Vec::with_capacity(active_calls);
    for (index, session) in sessions.iter().enumerate() {
        send_tasks.push(tokio::spawn(send_bidirectional_tone(
            session.caller_coord.clone(),
            session.caller_session_id.clone(),
            440.0 + (index % 5) as f32 * 20.0,
            session.agent_coord.clone(),
            session.agent_session_id.clone(),
            880.0 + (index % 5) as f32 * 20.0,
            hold_duration,
        )));
    }

    for task in send_tasks {
        task.await
            .map_err(|error| OrchestrationError::InvalidState(error.to_string()))??;
    }
    sleep(Duration::from_millis(500)).await;

    let mut caller_received_frames = 0;
    let mut agent_received_frames = 0;
    for (index, task) in receive_tasks.into_iter().enumerate() {
        let frames = task
            .await
            .map_err(|error| OrchestrationError::InvalidState(error.to_string()))?;
        if index % 2 == 0 {
            caller_received_frames += frames;
        } else {
            agent_received_frames += frames;
        }
    }

    let media_wall_time = media_started_at.elapsed();
    let media_cpu_time = media_cpu_before
        .zip(process_cpu_time())
        .and_then(|(before, after)| after.checked_sub(before));
    let rss_after_media_bytes = current_rss_bytes();

    let mut connected_calls = 0;
    for session in &sessions {
        let call = handle
            .get_call(&session.call_id)
            .await?
            .ok_or_else(|| OrchestrationError::CallNotFound(session.call_id.clone()))?;
        if call.status == CallStatus::Connected {
            connected_calls += 1;
        }
        std::hint::black_box(&session.bridge_id);
    }
    let active_bridges = handle.active_bridge_count().await;

    let mut hangup_tasks = Vec::with_capacity(active_calls);
    for session in &sessions {
        let coord = session.caller_coord.clone();
        let sid = session.caller_session_id.clone();
        hangup_tasks.push(tokio::spawn(async move { coord.hangup(&sid).await }));
    }
    for task in hangup_tasks {
        let _ = task.await;
    }
    sleep(Duration::from_millis(200)).await;

    let mut shutdown_tasks = Vec::with_capacity(active_calls * 2 + 1);
    for coord in caller_coords.iter().chain(agent_coords.iter()) {
        let coord = coord.clone();
        shutdown_tasks.push(tokio::spawn(async move {
            coord
                .shutdown_gracefully(Some(Duration::from_secs(0)))
                .await
        }));
    }
    if let Some(coordinator) = handle.coordinator().cloned() {
        shutdown_tasks.push(tokio::spawn(async move {
            coordinator
                .shutdown_gracefully(Some(Duration::from_secs(0)))
                .await
        }));
    }
    for task in shutdown_tasks {
        let _ = task.await;
    }
    let _ = timeout(Duration::from_secs(2), run_task).await;

    let setup_and_media_cpu_time = cpu_before
        .zip(process_cpu_time())
        .and_then(|(before, after)| after.checked_sub(before));
    let rss_active_delta_bytes = rss_before_bytes
        .zip(rss_active_bytes)
        .map(|(before, after)| after as i64 - before as i64);
    let rss_after_media_delta_bytes = rss_before_bytes
        .zip(rss_after_media_bytes)
        .map(|(before, after)| after as i64 - before as i64);
    let active_bytes_per_call =
        rss_active_delta_bytes.map(|delta| delta as f64 / active_calls.max(1) as f64);

    Ok(LiveSipRtpProfile {
        active_calls,
        hold_duration,
        connected_calls,
        active_bridges,
        caller_received_frames,
        agent_received_frames,
        setup_wall_time,
        media_wall_time,
        total_wall_time: total_started_at.elapsed(),
        setup_and_media_cpu_time,
        media_cpu_time,
        rss_before_bytes,
        rss_active_bytes,
        rss_after_media_bytes,
        rss_active_delta_bytes,
        rss_after_media_delta_bytes,
        active_bytes_per_call,
    })
}

pub fn current_rss_bytes() -> Option<u64> {
    current_rss_bytes_from_proc().or_else(current_rss_bytes_from_ps)
}

#[cfg(target_os = "linux")]
fn current_rss_bytes_from_proc() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}

#[cfg(not(target_os = "linux"))]
fn current_rss_bytes_from_proc() -> Option<u64> {
    None
}

fn current_rss_bytes_from_ps() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let kib = stdout.split_whitespace().next()?.parse::<u64>().ok()?;
    Some(kib * 1024)
}

fn canonical_sip_uri(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(start) = trimmed.find('<') {
        if let Some(end_offset) = trimmed[start + 1..].find('>') {
            return trimmed[start + 1..start + 1 + end_offset].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn live_setup_timeout(active_calls: usize) -> Duration {
    let secs = std::env::var("RVOIP_LIVE_SIP_RTP_SETUP_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| (active_calls as u64).saturating_mul(2).max(60));
    Duration::from_secs(secs)
}

fn live_session_config(name: &str, active_calls: usize, media_ports_per_call: usize) -> Config {
    let sip_port = reserve_udp_port_block(1).0;
    let media_width = active_calls
        .saturating_mul(media_ports_per_call)
        .saturating_add(64)
        .max(128);
    let (media_port_start, media_port_end) = reserve_udp_port_block(media_width);
    let mut config = Config::local(name, sip_port);
    config.media_port_start = media_port_start;
    config.media_port_end = media_port_end;
    config.unregister_on_shutdown_timeout_secs = 0;
    config
}

fn reserve_udp_port_block(width: usize) -> (u16, u16) {
    let width = width.max(1);
    for _ in 0..4096 {
        let start = NEXT_PORT_BLOCK.fetch_add(width + 17, Ordering::Relaxed);
        if start + width + 1 >= 60_000 {
            NEXT_PORT_BLOCK.store(24_000, Ordering::Relaxed);
            continue;
        }
        if udp_port_block_is_available(start as u16, width) {
            return (start as u16, (start + width - 1) as u16);
        }
    }
    panic!("unable to reserve UDP port block of width {width}");
}

fn udp_port_block_is_available(start: u16, width: usize) -> bool {
    let mut sockets = Vec::with_capacity(width);
    for offset in 0..width {
        let port = start.saturating_add(offset as u16);
        match std::net::UdpSocket::bind(("127.0.0.1", port)) {
            Ok(socket) => sockets.push(socket),
            Err(_) => return false,
        }
    }
    true
}

async fn send_bidirectional_tone(
    caller: Arc<UnifiedCoordinator>,
    caller_session_id: SessionId,
    caller_hz: f32,
    agent: Arc<UnifiedCoordinator>,
    agent_session_id: SessionId,
    agent_hz: f32,
    duration: Duration,
) -> Result<()> {
    let frames = (duration.as_millis() / FRAME_MS as u128) as usize;
    let mut caller_phase = 0.0;
    let mut agent_phase = 0.0;
    for frame_index in 0..frames {
        let timestamp = (frame_index * FRAME_SAMPLES) as u32;
        caller
            .send_audio(
                &caller_session_id,
                rvoip_media_core::types::AudioFrame::new(
                    tone_frame(caller_hz, &mut caller_phase),
                    SAMPLE_RATE,
                    1,
                    timestamp,
                ),
            )
            .await?;
        agent
            .send_audio(
                &agent_session_id,
                rvoip_media_core::types::AudioFrame::new(
                    tone_frame(agent_hz, &mut agent_phase),
                    SAMPLE_RATE,
                    1,
                    timestamp,
                ),
            )
            .await?;
        sleep(Duration::from_millis(FRAME_MS)).await;
    }
    Ok(())
}

async fn count_received_audio_frames(
    mut subscriber: rvoip_session_core::types::AudioFrameSubscriber,
    duration: Duration,
) -> usize {
    let deadline = tokio::time::Instant::now() + duration;
    let mut frames = 0;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return frames;
        }
        match timeout(deadline - now, subscriber.recv()).await {
            Ok(Some(_)) => frames += 1,
            Ok(None) | Err(_) => return frames,
        }
    }
}

fn tone_frame(freq_hz: f32, phase: &mut f32) -> Vec<i16> {
    let phase_step = std::f32::consts::TAU * freq_hz / SAMPLE_RATE as f32;
    let mut samples = Vec::with_capacity(FRAME_SAMPLES);
    for _ in 0..FRAME_SAMPLES {
        samples.push((0.30 * phase.sin() * i16::MAX as f32) as i16);
        *phase += phase_step;
        if *phase >= std::f32::consts::TAU {
            *phase -= std::f32::consts::TAU;
        }
    }
    samples
}

#[cfg(unix)]
pub fn process_cpu_time() -> Option<Duration> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(timeval_to_duration(usage.ru_utime) + timeval_to_duration(usage.ru_stime))
}

#[cfg(not(unix))]
pub fn process_cpu_time() -> Option<Duration> {
    None
}

#[cfg(unix)]
fn timeval_to_duration(value: libc::timeval) -> Duration {
    let secs = if value.tv_sec < 0 {
        0
    } else {
        value.tv_sec as u64
    };
    let micros = if value.tv_usec < 0 {
        0
    } else {
        value.tv_usec as u32
    };
    Duration::new(secs, micros.saturating_mul(1000))
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn format_optional_millis(value: Option<Duration>) -> String {
    value
        .map(|duration| format!("{:.2}", millis(duration)))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_optional_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn format_optional_bytes(value: Option<i64>) -> String {
    value.map(format_bytes).unwrap_or_else(|| "n/a".to_string())
}

fn format_bytes(bytes: i64) -> String {
    let sign = if bytes < 0 { "-" } else { "" };
    let abs = bytes.unsigned_abs() as f64;
    if abs >= 1024.0 * 1024.0 {
        format!("{sign}{:.2} MiB", abs / 1024.0 / 1024.0)
    } else if abs >= 1024.0 {
        format!("{sign}{:.2} KiB", abs / 1024.0)
    } else {
        format!("{sign}{} B", abs as u64)
    }
}
