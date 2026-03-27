use crate::api_client::ApiClient;
use crate::output::{print_error, print_json};

pub async fn execute(api: &ApiClient, cmd: crate::ReportCmd) -> anyhow::Result<()> {
    match cmd {
        crate::ReportCmd::Daily { date } => {
            let path = match &date {
                Some(d) => format!("/reports/daily?date={}", d),
                None => "/reports/daily".to_string(),
            };
            match api.get(&path).await {
                Ok(data) => print_json(&data["data"]),
                Err(e) => print_error(&format!("Failed to fetch daily report: {}", e)),
            }
        }
        crate::ReportCmd::Agent { id } => {
            match api.get(&format!("/reports/agent/{}", id)).await {
                Ok(data) => print_json(&data["data"]),
                Err(e) => print_error(&format!("Failed to fetch agent report: {}", e)),
            }
        }
        crate::ReportCmd::Summary => {
            match api.get("/reports/summary").await {
                Ok(data) => print_json(&data["data"]),
                Err(e) => print_error(&format!("Failed to fetch summary: {}", e)),
            }
        }
        crate::ReportCmd::Export { format } => {
            let fmt = format.as_deref().unwrap_or("json");
            match api.get(&format!("/reports/export?format={}", fmt)).await {
                Ok(data) => print_json(&data),
                Err(e) => print_error(&format!("Failed to export report: {}", e)),
            }
        }
    }

    Ok(())
}
