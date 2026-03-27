use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "get_system_health",
            "description": "Get current system health status including uptime, CPU, memory, and service states",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "get_dashboard",
            "description": "Get the main dashboard overview with key metrics and KPIs",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "get_audit_log",
            "description": "Retrieve recent audit log entries",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum number of log entries to return (default 50)" }
                },
                "required": []
            }
        }),
        json!({
            "name": "export_config",
            "description": "Export the current system configuration",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    ]
}

pub async fn handle(api: &crate::api_client::RvoipApiClient, name: &str, args: Value) -> anyhow::Result<Value> {
    match name {
        "get_system_health" => api.get("/system/health").await,
        "get_dashboard" => api.get("/dashboard").await,
        "get_audit_log" => {
            let limit = args["limit"].as_i64().unwrap_or(50);
            api.get(&format!("/system/audit/log?limit={}", limit)).await
        }
        "export_config" => api.get("/system/config").await,
        _ => anyhow::bail!("unknown tool: {}", name),
    }
}
