use crate::api_client::ApiClient;
use crate::output::{print_error, print_json, print_status_box};

pub async fn execute(api: &ApiClient) -> anyhow::Result<()> {
    match api.get("/dashboard/status").await {
        Ok(data) => {
            let active_calls = data["data"]["active_calls"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into());
            let agents_online = data["data"]["agents_online"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into());
            let queued_calls = data["data"]["queued_calls"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into());
            let avg_wait = data["data"]["avg_wait_time"]
                .as_str()
                .unwrap_or("-");
            let uptime = data["data"]["uptime"]
                .as_str()
                .unwrap_or("-");

            print_status_box(
                "System Status",
                &[
                    ("Active Calls", &active_calls),
                    ("Agents Online", &agents_online),
                    ("Queued Calls", &queued_calls),
                    ("Avg Wait Time", avg_wait),
                    ("Uptime", uptime),
                ],
            );

            if data["data"].is_object() {
                print_json(&data["data"]);
            }
        }
        Err(e) => {
            print_error(&format!("Failed to fetch status: {}", e));
        }
    }

    Ok(())
}
