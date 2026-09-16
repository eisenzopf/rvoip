//! Orchestrator-backed fulfillment for UCTP conversation envelopes.
//!
//! Substrate adapters call [`consume_conversation_event`] from their
//! coordinator event loop so `conversation.create` / `list` / `close`
//! oneshots are never dropped.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use rvoip_core::conversation::{ConversationPolicy as CorePolicy, ConversationState};
use rvoip_core::ids::{ConversationId, TenantId};
use rvoip_core::participant::ParticipantKind;
use rvoip_core::Orchestrator;

use crate::errors::UctpError;
use crate::payloads::conversation::{
    ConversationPolicy, InitialParticipant, Participant as WireParticipant,
};
use crate::state::events::{
    ConversationClosedReply, ConversationListReply, ConversationOpenedReply, UctpSessionEvent,
};

/// If `event` is a conversation command, fulfill it against `orchestrator`
/// and return `None`. Otherwise return the event for the adapter to map.
pub async fn consume_conversation_event(
    orchestrator: Option<&Arc<Orchestrator>>,
    event: UctpSessionEvent,
) -> Option<UctpSessionEvent> {
    match event {
        UctpSessionEvent::ConversationCreate {
            cid,
            tenant_id,
            policy,
            idle_close_secs,
            metadata,
            initial_participants,
            reply,
            ..
        } => {
            let _ = reply.send(
                fulfill_conversation_create(
                    orchestrator,
                    cid,
                    tenant_id,
                    policy,
                    idle_close_secs,
                    metadata,
                    initial_participants,
                )
                .await,
            );
            None
        }
        UctpSessionEvent::ConversationList {
            filter,
            limit,
            reply,
            ..
        } => {
            let _ = reply.send(fulfill_conversation_list(orchestrator, filter, limit).await);
            None
        }
        UctpSessionEvent::ConversationClose {
            cid,
            reason_code,
            reason,
            reply,
            ..
        } => {
            let _ = reply
                .send(fulfill_conversation_close(orchestrator, cid, reason_code, reason).await);
            None
        }
        other => Some(other),
    }
}

async fn fulfill_conversation_create(
    orchestrator: Option<&Arc<Orchestrator>>,
    cid: Option<String>,
    tenant_id: String,
    policy: ConversationPolicy,
    idle_close_secs: Option<u32>,
    metadata: serde_json::Value,
    _initial_participants: Vec<InitialParticipant>,
) -> Result<ConversationOpenedReply, UctpError> {
    let orch = orchestrator.ok_or(UctpError::Closed)?;
    let core_policy = match policy {
        ConversationPolicy::Persistent => CorePolicy::Persistent,
        ConversationPolicy::Ephemeral => CorePolicy::Ephemeral {
            idle_close_secs: u64::from(idle_close_secs.unwrap_or(30)),
        },
    };
    let meta = metadata_to_map(&metadata);
    let tenant = if tenant_id.is_empty() {
        TenantId::new()
    } else {
        TenantId::from_string(tenant_id.clone())
    };
    let conversation_id = if let Some(cid) = cid.filter(|cid| !cid.is_empty()) {
        orch.open_conversation_with_id(ConversationId::from_string(cid), tenant, core_policy, meta)
            .await
            .map_err(|_| UctpError::Closed)?
    } else {
        orch.open_conversation(tenant, core_policy, meta)
            .await
            .map_err(|_| UctpError::Closed)?
    };
    snapshot_opened(orch, &conversation_id, policy, idle_close_secs, metadata)
}

async fn fulfill_conversation_list(
    orchestrator: Option<&Arc<Orchestrator>>,
    filter: serde_json::Value,
    limit: Option<u32>,
) -> Result<ConversationListReply, UctpError> {
    let orch = orchestrator.ok_or(UctpError::Closed)?;
    let tenant_filter = filter
        .get("tenant_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let limit = limit.unwrap_or(50) as usize;
    let mut conversations = Vec::new();
    for id in orch.live_conversation_ids() {
        if conversations.len() >= limit {
            break;
        }
        let Some(handle) = orch.conversation(&id) else {
            continue;
        };
        let conv = handle.read().map_err(|_| UctpError::Closed)?;
        if conv.state != ConversationState::Open {
            continue;
        }
        if let Some(tenant_id) = tenant_filter.as_ref() {
            if conv.tenant_id.as_str() != tenant_id {
                continue;
            }
        }
        let policy = match &conv.policy {
            CorePolicy::Persistent => ConversationPolicy::Persistent,
            CorePolicy::Ephemeral { .. } => ConversationPolicy::Ephemeral,
        };
        let idle_close_secs = match &conv.policy {
            CorePolicy::Ephemeral { idle_close_secs } => u32::try_from(*idle_close_secs).ok(),
            CorePolicy::Persistent => None,
        };
        drop(conv);
        conversations.push(snapshot_opened(
            orch,
            &id,
            policy,
            idle_close_secs,
            serde_json::Value::Null,
        )?);
    }
    Ok(ConversationListReply {
        conversations,
        next_cursor: None,
    })
}

async fn fulfill_conversation_close(
    orchestrator: Option<&Arc<Orchestrator>>,
    cid: Option<String>,
    reason_code: Option<u16>,
    reason: Option<String>,
) -> Result<ConversationClosedReply, UctpError> {
    let orch = orchestrator.ok_or(UctpError::Closed)?;
    let cid = cid.ok_or(UctpError::MissingField("cid"))?;
    let id = ConversationId::from_string(cid.clone());
    orch.close_conversation(id, true)
        .await
        .map_err(|_| UctpError::Closed)?;
    Ok(ConversationClosedReply {
        cid,
        reason_code: reason_code.unwrap_or(200),
        reason: reason.unwrap_or_else(|| "explicit-close".into()),
        closed_at: Utc::now(),
    })
}

fn snapshot_opened(
    orch: &Orchestrator,
    conversation_id: &ConversationId,
    policy: ConversationPolicy,
    idle_close_secs: Option<u32>,
    metadata: serde_json::Value,
) -> Result<ConversationOpenedReply, UctpError> {
    let handle = orch
        .conversation(conversation_id)
        .ok_or(UctpError::Closed)?;
    let conv = handle.read().map_err(|_| UctpError::Closed)?;
    let participants = conv
        .participants
        .iter()
        .map(|participant| WireParticipant {
            participant_id: participant.id.to_string(),
            identity_id: participant
                .identity_ref
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            kind: match participant.kind {
                ParticipantKind::Human => "human".into(),
                ParticipantKind::Ai => "ai".into(),
                ParticipantKind::System => "system".into(),
                ParticipantKind::External => "external".into(),
            },
            role: format!("{:?}", participant.role).to_lowercase(),
            display_name: participant.display_name.clone(),
        })
        .collect();
    Ok(ConversationOpenedReply {
        cid: conversation_id.to_string(),
        tenant_id: conv.tenant_id.to_string(),
        policy,
        idle_close_secs,
        participants,
        opened_at: conv.opened_at,
        metadata,
    })
}

fn metadata_to_map(value: &serde_json::Value) -> HashMap<String, String> {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|text| (key.clone(), text.to_owned())))
            .collect(),
        _ => HashMap::new(),
    }
}
