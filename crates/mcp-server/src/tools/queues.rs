use anyhow::Result;
use serde_json::{json, Value};

use crate::api_client::RvoipApiClient;
use super::tool_def;

/// Return tool definitions for queue-related MCP tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool_def(
            "list_queues",
            "List all call queues",
            json!({}),
            vec![],
        ),
        tool_def(
            "create_queue",
            "Create a new call queue",
            json!({
                "name":        { "type": "string", "description": "Queue name" },
                "strategy":    { "type": "string", "description": "Routing strategy (round-robin, longest-idle, skills-based)" },
                "max_wait":    { "type": "integer", "description": "Max wait time in seconds before overflow" }
            }),
            vec!["name"],
        ),
        tool_def(
            "get_queue_status",
            "Get current status of a specific queue (waiting callers, available agents, etc.)",
            json!({
                "queue_id": { "type": "string", "description": "The queue ID" }
            }),
            vec!["queue_id"],
        ),
        tool_def(
            "assign_call_to_agent",
            "Manually assign a queued call to a specific agent",
            json!({
                "queue_id": { "type": "string", "description": "The queue ID" },
                "call_id":  { "type": "string", "description": "The call ID to assign" },
                "agent_id": { "type": "string", "description": "The target agent ID" }
            }),
            vec!["queue_id", "call_id", "agent_id"],
        ),
        tool_def(
            "get_queue_performance",
            "Get performance metrics across all queues",
            json!({}),
            vec![],
        ),
    ]
}

/// Dispatch a queue-related tool invocation to the appropriate API endpoint.
pub async fn handle(api: &RvoipApiClient, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_queues" => api.get("/queues").await,

        "create_queue" => {
            let mut body = json!({
                "name": args["name"],
            });
            if let Some(strategy) = args.get("strategy") {
                body["strategy"] = strategy.clone();
            }
            if let Some(max_wait) = args.get("max_wait") {
                body["max_wait"] = max_wait.clone();
            }
            api.post("/queues", &body).await
        }

        "get_queue_status" => {
            let queue_id = args["queue_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing queue_id"))?;
            api.get(&format!("/queues/{}", queue_id)).await
        }

        "assign_call_to_agent" => {
            let queue_id = args["queue_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing queue_id"))?;
            let call_id = args["call_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing call_id"))?;
            let agent_id = args["agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
            api.post(
                &format!("/queues/{}/calls/{}/assign", queue_id, call_id),
                &json!({ "agent_id": agent_id }),
            )
            .await
        }

        "get_queue_performance" => api.get("/reports/queue-performance").await,

        _ => Err(anyhow::anyhow!("unknown queues tool: {}", name)),
    }
}
