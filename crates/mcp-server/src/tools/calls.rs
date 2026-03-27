use anyhow::Result;
use serde_json::{json, Value};

use crate::api_client::RvoipApiClient;
use super::tool_def;

/// Return tool definitions for call-related MCP tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool_def(
            "list_active_calls",
            "List all currently active calls in the call center",
            json!({}),
            vec![],
        ),
        tool_def(
            "get_call_detail",
            "Get detailed information about a specific call",
            json!({
                "call_id": { "type": "string", "description": "The unique call identifier" }
            }),
            vec!["call_id"],
        ),
        tool_def(
            "hangup_call",
            "Hang up / terminate an active call",
            json!({
                "call_id": { "type": "string", "description": "The unique call identifier" }
            }),
            vec!["call_id"],
        ),
        tool_def(
            "get_call_history",
            "Retrieve historical call records with pagination",
            json!({
                "limit":  { "type": "integer", "description": "Max records to return (default 50)" },
                "offset": { "type": "integer", "description": "Offset for pagination (default 0)" }
            }),
            vec![],
        ),
        tool_def(
            "get_call_stats",
            "Get aggregated call statistics from the dashboard",
            json!({}),
            vec![],
        ),
        tool_def(
            "transfer_call",
            "Transfer an active call to another agent or extension (placeholder — not yet in API)",
            json!({
                "call_id": { "type": "string", "description": "The call to transfer" },
                "target":  { "type": "string", "description": "Target agent ID or extension" }
            }),
            vec!["call_id", "target"],
        ),
    ]
}

/// Dispatch a call-related tool invocation to the appropriate API endpoint.
pub async fn handle(api: &RvoipApiClient, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_active_calls" => api.get("/calls").await,

        "get_call_detail" => {
            let call_id = args["call_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing call_id"))?;
            api.get(&format!("/calls/{}", call_id)).await
        }

        "hangup_call" => {
            let call_id = args["call_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing call_id"))?;
            api.post(&format!("/calls/{}/hangup", call_id), &json!({}))
                .await
        }

        "get_call_history" => {
            let limit = args["limit"].as_i64().unwrap_or(50);
            let offset = args["offset"].as_i64().unwrap_or(0);
            api.get(&format!("/calls/history?limit={}&offset={}", limit, offset))
                .await
        }

        "get_call_stats" => api.get("/dashboard").await,

        "transfer_call" => {
            let call_id = args["call_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing call_id"))?;
            let target = args["target"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing target"))?;
            api.post(
                &format!("/calls/{}/transfer", call_id),
                &json!({ "target": target }),
            )
            .await
        }

        _ => Err(anyhow::anyhow!("unknown calls tool: {}", name)),
    }
}
