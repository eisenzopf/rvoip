use anyhow::Result;
use serde_json::{json, Value};

use crate::api_client::RvoipApiClient;
use super::tool_def;

/// Return tool definitions for department-related MCP tools.
pub fn tools() -> Vec<Value> {
    vec![
        tool_def(
            "list_departments",
            "List all departments in the organization",
            json!({}),
            vec![],
        ),
        tool_def(
            "create_department",
            "Create a new department",
            json!({
                "name":        { "type": "string", "description": "Department name" },
                "description": { "type": "string", "description": "Department description (optional)" }
            }),
            vec!["name"],
        ),
        tool_def(
            "delete_department",
            "Delete a department by ID",
            json!({
                "department_id": { "type": "string", "description": "The department ID to delete" }
            }),
            vec!["department_id"],
        ),
    ]
}

/// Dispatch a department-related tool invocation to the appropriate API endpoint.
pub async fn handle(api: &RvoipApiClient, name: &str, args: Value) -> Result<Value> {
    match name {
        "list_departments" => api.get("/departments").await,

        "create_department" => {
            let mut body = json!({ "name": args["name"] });
            if let Some(desc) = args.get("description") {
                body["description"] = desc.clone();
            }
            api.post("/departments", &body).await
        }

        "delete_department" => {
            let department_id = args["department_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing department_id"))?;
            api.delete(&format!("/departments/{}", department_id)).await
        }

        _ => Err(anyhow::anyhow!("unknown departments tool: {}", name)),
    }
}
