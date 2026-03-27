use serde::Deserialize;
use tabled::Tabled;

use crate::api_client::ApiClient;
use crate::output::{print_error, print_json, print_success, print_table};

#[derive(Debug, Deserialize, Tabled)]
pub struct AgentRow {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Name")]
    pub name: String,
    #[tabled(rename = "Status")]
    pub status: String,
    #[tabled(rename = "Department")]
    #[serde(default)]
    pub department: String,
    #[tabled(rename = "Extension")]
    #[serde(default)]
    pub extension: String,
}

pub async fn execute(api: &ApiClient, cmd: crate::AgentCmd) -> anyhow::Result<()> {
    match cmd {
        crate::AgentCmd::List { status, dept } => {
            let mut path = "/agents".to_string();
            let mut params = Vec::new();
            if let Some(s) = &status {
                params.push(format!("status={}", s));
            }
            if let Some(d) = &dept {
                params.push(format!("department={}", d));
            }
            if !params.is_empty() {
                path = format!("{}?{}", path, params.join("&"));
            }

            match api.get(&path).await {
                Ok(data) => {
                    let agents: Vec<AgentRow> =
                        serde_json::from_value(data["data"].clone()).unwrap_or_default();
                    if agents.is_empty() {
                        print_json(&data);
                    } else {
                        print_table(&agents);
                    }
                }
                Err(e) => print_error(&format!("Failed to list agents: {}", e)),
            }
        }
        crate::AgentCmd::Create { name, dept } => {
            let mut body = serde_json::json!({ "name": name });
            if let Some(d) = dept {
                body["department"] = serde_json::Value::String(d);
            }
            match api.post("/agents", &body).await {
                Ok(data) => {
                    print_success(&format!(
                        "Agent created: {}",
                        data["data"]["id"].as_str().unwrap_or("unknown")
                    ));
                    print_json(&data["data"]);
                }
                Err(e) => print_error(&format!("Failed to create agent: {}", e)),
            }
        }
        crate::AgentCmd::Delete { id } => {
            match api.delete(&format!("/agents/{}", id)).await {
                Ok(_) => print_success(&format!("Agent {} deleted", id)),
                Err(e) => print_error(&format!("Failed to delete agent: {}", e)),
            }
        }
        crate::AgentCmd::Status { id, new_status } => {
            let body = serde_json::json!({ "status": new_status });
            match api.put(&format!("/agents/{}/status", id), &body).await {
                Ok(_) => print_success(&format!("Agent {} status set to {}", id, new_status)),
                Err(e) => print_error(&format!("Failed to update agent status: {}", e)),
            }
        }
    }

    Ok(())
}
