use serde::Deserialize;
use tabled::Tabled;

use crate::api_client::ApiClient;
use crate::output::{print_error, print_json, print_success, print_table};

#[derive(Debug, Deserialize, Tabled)]
pub struct UserRow {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Username")]
    pub username: String,
    #[tabled(rename = "Role")]
    #[serde(default)]
    pub role: String,
    #[tabled(rename = "Email")]
    #[serde(default)]
    pub email: String,
}

pub async fn execute(api: &ApiClient, cmd: crate::UserCmd) -> anyhow::Result<()> {
    match cmd {
        crate::UserCmd::List => {
            match api.get("/users").await {
                Ok(data) => {
                    let users: Vec<UserRow> =
                        serde_json::from_value(data["data"].clone()).unwrap_or_default();
                    if users.is_empty() {
                        print_json(&data);
                    } else {
                        print_table(&users);
                    }
                }
                Err(e) => print_error(&format!("Failed to list users: {}", e)),
            }
        }
        crate::UserCmd::Create { username, role, email } => {
            let mut body = serde_json::json!({ "username": username });
            if let Some(r) = role {
                body["role"] = serde_json::Value::String(r);
            }
            if let Some(e) = email {
                body["email"] = serde_json::Value::String(e);
            }
            match api.post("/users", &body).await {
                Ok(data) => {
                    print_success(&format!("User created: {}", data["data"]["id"].as_str().unwrap_or("unknown")));
                }
                Err(e) => print_error(&format!("Failed to create user: {}", e)),
            }
        }
        crate::UserCmd::Delete { id } => {
            match api.delete(&format!("/users/{}", id)).await {
                Ok(_) => print_success(&format!("User {} deleted", id)),
                Err(e) => print_error(&format!("Failed to delete user: {}", e)),
            }
        }
        crate::UserCmd::Roles => {
            match api.get("/users/roles").await {
                Ok(data) => print_json(&data["data"]),
                Err(e) => print_error(&format!("Failed to fetch roles: {}", e)),
            }
        }
    }

    Ok(())
}
