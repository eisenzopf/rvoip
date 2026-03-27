use serde_json::{json, Value};

pub fn list_prompts() -> Vec<Value> {
    vec![
        json!({
            "name": "call_center_manager",
            "description": "Act as a call center operations manager. Help with agent management, queue monitoring, and daily operations.",
            "arguments": [
                { "name": "task", "description": "What you need help with", "required": true }
            ]
        }),
        json!({
            "name": "quality_reviewer",
            "description": "Review call quality. Analyze call transcripts and provide quality scores based on templates.",
            "arguments": [
                { "name": "call_id", "description": "Call ID to review", "required": true }
            ]
        }),
        json!({
            "name": "report_analyst",
            "description": "Analyze call center performance data and generate insights.",
            "arguments": [
                { "name": "period", "description": "Time period (today/week/month)", "required": false }
            ]
        }),
    ]
}

pub fn get_prompt(name: &str, args: &Value) -> anyhow::Result<Vec<Value>> {
    match name {
        "call_center_manager" => {
            let task = args["task"].as_str().unwrap_or("general operations");
            Ok(vec![json!({
                "role": "user",
                "content": { "type": "text", "text": format!(
                    "You are an expert call center operations manager for the rvoip platform. \
                    You have access to tools for managing agents, calls, queues, and system configuration. \
                    Task: {}", task
                )}
            })])
        }
        "quality_reviewer" => {
            let call_id = args["call_id"].as_str().unwrap_or_default();
            Ok(vec![json!({
                "role": "user",
                "content": { "type": "text", "text": format!(
                    "You are a call quality reviewer. Review call {} and score it based on the quality templates. \
                    Use the get_call_detail and quality scoring tools.", call_id
                )}
            })])
        }
        "report_analyst" => {
            let period = args["period"].as_str().unwrap_or("today");
            Ok(vec![json!({
                "role": "user",
                "content": { "type": "text", "text": format!(
                    "You are a call center analytics expert. Analyze performance data for '{}' period. \
                    Use the report generation tools to pull data, then provide insights and recommendations.", period
                )}
            })])
        }
        _ => anyhow::bail!("unknown prompt: {}", name),
    }
}
