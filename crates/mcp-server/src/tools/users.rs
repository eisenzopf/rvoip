use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_users",
            "description": "List users with optional role and search filters",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "role": { "type": "string", "description": "Filter by role (admin/agent/supervisor)" },
                    "search": { "type": "string", "description": "Search by name or email" }
                },
                "required": []
            }
        }),
        json!({
            "name": "create_user",
            "description": "Create a new user account",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Full name of the user" },
                    "email": { "type": "string", "description": "Email address" },
                    "role": { "type": "string", "description": "User role (admin/agent/supervisor)" },
                    "password": { "type": "string", "description": "Initial password" }
                },
                "required": ["name", "email", "role", "password"]
            }
        }),
        json!({
            "name": "update_user_roles",
            "description": "Update a user's assigned roles",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "user_id": { "type": "string", "description": "The user ID" },
                    "roles": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of roles to assign"
                    }
                },
                "required": ["user_id", "roles"]
            }
        }),
        json!({
            "name": "delete_user",
            "description": "Delete a user account",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "user_id": { "type": "string", "description": "The user ID to delete" }
                },
                "required": ["user_id"]
            }
        }),
    ]
}

pub async fn handle(api: &crate::api_client::RvoipApiClient, name: &str, args: Value) -> anyhow::Result<Value> {
    match name {
        "list_users" => {
            let role = args["role"].as_str().unwrap_or_default();
            let search = args["search"].as_str().unwrap_or_default();
            let mut path = String::from("/users");
            let mut params = Vec::new();
            if !role.is_empty() {
                params.push(format!("role={}", role));
            }
            if !search.is_empty() {
                params.push(format!("search={}", search));
            }
            if !params.is_empty() {
                path.push('?');
                path.push_str(&params.join("&"));
            }
            api.get(&path).await
        }
        "create_user" => {
            let body = json!({
                "name": args["name"].as_str().unwrap_or_default(),
                "email": args["email"].as_str().unwrap_or_default(),
                "role": args["role"].as_str().unwrap_or_default(),
                "password": args["password"].as_str().unwrap_or_default(),
            });
            api.post("/users", &body).await
        }
        "update_user_roles" => {
            let user_id = args["user_id"].as_str().unwrap_or_default();
            let roles = args.get("roles").cloned().unwrap_or_default();
            api.put(&format!("/users/{}/roles", user_id), &json!({ "roles": roles })).await
        }
        "delete_user" => {
            let user_id = args["user_id"].as_str().unwrap_or_default();
            api.delete(&format!("/users/{}", user_id)).await
        }
        _ => anyhow::bail!("unknown tool: {}", name),
    }
}
