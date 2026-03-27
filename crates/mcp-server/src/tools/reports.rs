use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "generate_daily_report",
            "description": "Generate a daily operations report for a specific date",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "Date in YYYY-MM-DD format (defaults to today)" }
                },
                "required": []
            }
        }),
        json!({
            "name": "generate_agent_report",
            "description": "Generate an agent performance report for a date range",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start date in YYYY-MM-DD format" },
                    "end": { "type": "string", "description": "End date in YYYY-MM-DD format" }
                },
                "required": ["start", "end"]
            }
        }),
        json!({
            "name": "generate_summary_report",
            "description": "Generate a summary report with key metrics for a date range",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start date in YYYY-MM-DD format" },
                    "end": { "type": "string", "description": "End date in YYYY-MM-DD format" }
                },
                "required": ["start", "end"]
            }
        }),
    ]
}

pub async fn handle(api: &crate::api_client::RvoipApiClient, name: &str, args: Value) -> anyhow::Result<Value> {
    match name {
        "generate_daily_report" => {
            let date = args["date"].as_str().unwrap_or_default();
            if date.is_empty() {
                api.get("/reports/daily").await
            } else {
                api.get(&format!("/reports/daily?date={}", date)).await
            }
        }
        "generate_agent_report" => {
            let start = args["start"].as_str().unwrap_or_default();
            let end = args["end"].as_str().unwrap_or_default();
            api.get(&format!("/reports/agent-performance?start={}&end={}", start, end)).await
        }
        "generate_summary_report" => {
            let start = args["start"].as_str().unwrap_or_default();
            let end = args["end"].as_str().unwrap_or_default();
            api.get(&format!("/reports/summary?start={}&end={}", start, end)).await
        }
        _ => anyhow::bail!("unknown tool: {}", name),
    }
}
