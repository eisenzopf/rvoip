use rvoip_orchestration_core::prelude::*;
use rvoip_session_core::types::{DialogId, IncomingCallInfo, SessionId};

pub mod perf;

pub fn support_call() -> Call {
    Call::inbound(
        CallerIdentity::new("sip:caller@example.com"),
        "sip:support@example.com",
    )
}

pub fn available_human(id: &str) -> Agent {
    let mut agent = Agent::human(id, format!("sip:{id}@127.0.0.1:5071"));
    agent.state = AgentState::Available;
    agent.skills.push(Skill::from("support"));
    agent
}

pub fn available_ai(id: &str) -> Agent {
    let mut agent = Agent::voice_ai(id, format!("{id}-runtime"));
    agent.state = AgentState::Available;
    agent.skills.push(Skill::from("support"));
    agent
}

pub fn incoming_call(to: &str) -> IncomingCallInfo {
    IncomingCallInfo {
        session_id: SessionId::new(),
        dialog_id: DialogId::new(),
        from: "sip:caller@example.com".to_string(),
        to: to.to_string(),
        call_id: format!("sip-call-{}", uuid::Uuid::new_v4()),
        p_asserted_identity: Some("\"Caller\" <sip:caller@example.com>".to_string()),
    }
}
