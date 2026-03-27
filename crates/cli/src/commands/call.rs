use serde::Deserialize;
use tabled::Tabled;

use crate::api_client::ApiClient;
use crate::output::{print_error, print_json, print_success, print_table};

#[derive(Debug, Deserialize, Tabled)]
pub struct CallRow {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "From")]
    #[serde(default)]
    pub from: String,
    #[tabled(rename = "To")]
    #[serde(default)]
    pub to: String,
    #[tabled(rename = "Status")]
    #[serde(default)]
    pub status: String,
    #[tabled(rename = "Duration")]
    #[serde(default)]
    pub duration: String,
}

pub async fn execute(api: &ApiClient, cmd: crate::CallCmd) -> anyhow::Result<()> {
    match cmd {
        crate::CallCmd::List { status } => {
            let path = match &status {
                Some(s) => format!("/calls?status={}", s),
                None => "/calls".to_string(),
            };
            match api.get(&path).await {
                Ok(data) => {
                    let calls: Vec<CallRow> =
                        serde_json::from_value(data["data"].clone()).unwrap_or_default();
                    if calls.is_empty() {
                        print_json(&data);
                    } else {
                        print_table(&calls);
                    }
                }
                Err(e) => print_error(&format!("Failed to list calls: {}", e)),
            }
        }
        crate::CallCmd::History { limit } => {
            let path = format!("/calls/history?limit={}", limit.unwrap_or(50));
            match api.get(&path).await {
                Ok(data) => {
                    let calls: Vec<CallRow> =
                        serde_json::from_value(data["data"].clone()).unwrap_or_default();
                    if calls.is_empty() {
                        print_json(&data);
                    } else {
                        print_table(&calls);
                    }
                }
                Err(e) => print_error(&format!("Failed to fetch call history: {}", e)),
            }
        }
        crate::CallCmd::Hangup { id } => {
            match api.post(&format!("/calls/{}/hangup", id), &serde_json::json!({})).await {
                Ok(_) => print_success(&format!("Call {} hung up", id)),
                Err(e) => print_error(&format!("Failed to hang up call: {}", e)),
            }
        }
        crate::CallCmd::Stats => {
            match api.get("/calls/stats").await {
                Ok(data) => print_json(&data["data"]),
                Err(e) => print_error(&format!("Failed to fetch call stats: {}", e)),
            }
        }
    }

    Ok(())
}
