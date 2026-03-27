use anyhow::Result;
use serde_json::{json, Value};

use crate::api_client::RvoipApiClient;
use super::tool_def;

/// Return tool definitions for routing-related MCP tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool_def(
            "get_routing_config",
            "Get the current call routing configuration",
            json!({}),
            vec![],
        ),
        tool_def(
            "list_overflow_policies",
            "List all overflow policies (what happens when queues are full or wait time exceeds threshold)",
            json!({}),
            vec![],
        ),
        tool_def(
            "create_overflow_policy",
            "Create a new overflow policy for queue overflow handling",
            json!({
                "name":         { "type": "string", "description": "Policy name" },
                "action":       { "type": "string", "description": "Action to take: voicemail, redirect, callback" },
                "threshold":    { "type": "integer", "description": "Wait-time threshold in seconds that triggers overflow" },
                "target_queue": { "type": "string", "description": "Target queue ID for redirect action (optional)" }
            }),
            vec!["name", "action", "threshold"],
        ),
    ]
}

/// Dispatch a routing-related tool invocation to the appropriate API endpoint.
pub async fn handle(api: &RvoipApiClient, name: &str, args: Value) -> Result<Value> {
    match name {
        "get_routing_config" => api.get("/routing/config").await,

        "list_overflow_policies" => api.get("/routing/overflow/policies").await,

        "create_overflow_policy" => {
            let mut body = json!({
                "name": args["name"],
                "action": args["action"],
                "threshold": args["threshold"],
            });
            if let Some(target) = args.get("target_queue") {
                body["target_queue"] = target.clone();
            }
            api.post("/routing/overflow/policies", &body).await
        }

        _ => Err(anyhow::anyhow!("unknown routing tool: {}", name)),
    }
}
