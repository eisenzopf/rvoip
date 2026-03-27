use serde::Deserialize;
use tabled::Tabled;

use crate::api_client::ApiClient;
use crate::output::{print_error, print_json, print_success, print_table};

#[derive(Debug, Deserialize, Tabled)]
pub struct QueueRow {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Name")]
    pub name: String,
    #[tabled(rename = "Waiting")]
    #[serde(default)]
    pub waiting: u64,
    #[tabled(rename = "Agents")]
    #[serde(default)]
    pub agents: u64,
}

pub async fn execute(api: &ApiClient, cmd: crate::QueueCmd) -> anyhow::Result<()> {
    match cmd {
        crate::QueueCmd::List => {
            match api.get("/queues").await {
                Ok(data) => {
                    let queues: Vec<QueueRow> =
                        serde_json::from_value(data["data"].clone()).unwrap_or_default();
                    if queues.is_empty() {
                        print_json(&data);
                    } else {
                        print_table(&queues);
                    }
                }
                Err(e) => print_error(&format!("Failed to list queues: {}", e)),
            }
        }
        crate::QueueCmd::Status { id } => {
            match api.get(&format!("/queues/{}", id)).await {
                Ok(data) => print_json(&data["data"]),
                Err(e) => print_error(&format!("Failed to fetch queue status: {}", e)),
            }
        }
        crate::QueueCmd::Create { name } => {
            let body = serde_json::json!({ "name": name });
            match api.post("/queues", &body).await {
                Ok(data) => {
                    print_success(&format!("Queue created: {}", data["data"]["id"].as_str().unwrap_or("unknown")));
                }
                Err(e) => print_error(&format!("Failed to create queue: {}", e)),
            }
        }
    }

    Ok(())
}
