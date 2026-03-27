use serde::Deserialize;
use tabled::Tabled;

use crate::api_client::ApiClient;
use crate::output::{print_error, print_json, print_success, print_table};

#[derive(Debug, Deserialize, Tabled)]
pub struct DeptRow {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Name")]
    pub name: String,
    #[tabled(rename = "Agents")]
    #[serde(default)]
    pub agent_count: u64,
}

pub async fn execute(api: &ApiClient, cmd: crate::DeptCmd) -> anyhow::Result<()> {
    match cmd {
        crate::DeptCmd::List => {
            match api.get("/departments").await {
                Ok(data) => {
                    let depts: Vec<DeptRow> =
                        serde_json::from_value(data["data"].clone()).unwrap_or_default();
                    if depts.is_empty() {
                        print_json(&data);
                    } else {
                        print_table(&depts);
                    }
                }
                Err(e) => print_error(&format!("Failed to list departments: {}", e)),
            }
        }
        crate::DeptCmd::Create { name } => {
            let body = serde_json::json!({ "name": name });
            match api.post("/departments", &body).await {
                Ok(data) => {
                    print_success(&format!("Department created: {}", data["data"]["id"].as_str().unwrap_or("unknown")));
                }
                Err(e) => print_error(&format!("Failed to create department: {}", e)),
            }
        }
        crate::DeptCmd::Delete { id } => {
            match api.delete(&format!("/departments/{}", id)).await {
                Ok(_) => print_success(&format!("Department {} deleted", id)),
                Err(e) => print_error(&format!("Failed to delete department: {}", e)),
            }
        }
    }

    Ok(())
}
