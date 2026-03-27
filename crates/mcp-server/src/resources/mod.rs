use serde_json::{json, Value};

pub fn list_resources() -> Vec<Value> {
    vec![
        json!({ "uri": "rvoip://calls/active", "name": "Active Calls", "description": "Real-time list of active calls", "mimeType": "application/json" }),
        json!({ "uri": "rvoip://agents/online", "name": "Online Agents", "description": "Currently online agents", "mimeType": "application/json" }),
        json!({ "uri": "rvoip://queues/status", "name": "Queue Status", "description": "Real-time queue depths and SLA", "mimeType": "application/json" }),
        json!({ "uri": "rvoip://system/health", "name": "System Health", "description": "System health status", "mimeType": "application/json" }),
        json!({ "uri": "rvoip://config/current", "name": "Configuration", "description": "Current system configuration", "mimeType": "application/json" }),
    ]
}

pub async fn read_resource(api: &crate::api_client::RvoipApiClient, uri: &str) -> anyhow::Result<Value> {
    match uri {
        "rvoip://calls/active" => api.get("/calls").await,
        "rvoip://agents/online" => api.get("/agents").await,
        "rvoip://queues/status" => api.get("/queues").await,
        "rvoip://system/health" => api.get("/system/health").await,
        "rvoip://config/current" => api.get("/system/config").await,
        _ => anyhow::bail!("unknown resource: {}", uri),
    }
}
