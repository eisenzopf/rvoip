use anyhow::Result;
use serde_json::{json, Value};

use crate::api_client::RvoipApiClient;
use super::tool_def;

/// Return tool definitions for agent-related MCP tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool_def(
            "list_agents",
            "List all registered agents in the call center",
            json!({}),
            vec![],
        ),
        tool_def(
            "create_agent",
            "Register a new agent",
            json!({
                "name":      { "type": "string", "description": "Agent display name" },
                "extension": { "type": "string", "description": "SIP extension number" },
                "skills":    { "type": "array", "items": { "type": "string" }, "description": "List of agent skills" }
            }),
            vec!["name", "extension"],
        ),
        tool_def(
            "update_agent",
            "Update an existing agent's details",
            json!({
                "agent_id": { "type": "string", "description": "The agent ID to update" },
                "name":     { "type": "string", "description": "New display name (optional)" },
                "skills":   { "type": "array", "items": { "type": "string" }, "description": "Updated skills list (optional)" }
            }),
            vec!["agent_id"],
        ),
        tool_def(
            "delete_agent",
            "Remove an agent from the system",
            json!({
                "agent_id": { "type": "string", "description": "The agent ID to delete" }
            }),
            vec!["agent_id"],
        ),
        tool_def(
            "set_agent_status",
            "Change an agent's availability status (available, busy, offline, etc.)",
            json!({
                "agent_id": { "type": "string", "description": "The agent ID" },
                "status":   { "type": "string", "description": "New status value" }
            }),
            vec!["agent_id", "status"],
        ),
        tool_def(
            "get_agent_performance",
            "Get performance metrics for a specific agent",
            json!({
                "agent_id": { "type": "string", "description": "The agent ID to query" }
            }),
            vec!["agent_id"],
        ),
    ]
}

/// Dispatch an agent-related tool invocation to the appropriate API endpoint.
pub async fn handle(api: &RvoipApiClient, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_agents" => api.get("/agents").await,

        "create_agent" => {
            let body = json!({
                "name": args["name"],
                "extension": args["extension"],
                "skills": args.get("skills").unwrap_or(&json!([])),
            });
            api.post("/agents", &body).await
        }

        "update_agent" => {
            let agent_id = args["agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
            let mut body = json!({});
            if let Some(name) = args.get("name") {
                body["name"] = name.clone();
            }
            if let Some(skills) = args.get("skills") {
                body["skills"] = skills.clone();
            }
            api.put(&format!("/agents/{}", agent_id), &body).await
        }

        "delete_agent" => {
            let agent_id = args["agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
            api.delete(&format!("/agents/{}", agent_id)).await
        }

        "set_agent_status" => {
            let agent_id = args["agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
            let status = args["status"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing status"))?;
            api.put(
                &format!("/agents/{}/status", agent_id),
                &json!({ "status": status }),
            )
            .await
        }

        "get_agent_performance" => {
            let agent_id = args["agent_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing agent_id"))?;
            api.get(&format!("/reports/agent-performance?agent_id={}", agent_id))
                .await
        }

        _ => Err(anyhow::anyhow!("unknown agents tool: {}", name)),
    }
}
