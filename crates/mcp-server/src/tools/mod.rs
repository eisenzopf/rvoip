pub mod agents;
pub mod calls;
pub mod departments;
pub mod knowledge;
pub mod queues;
pub mod reports;
pub mod routing;
pub mod system;
pub mod users;

use serde_json::{json, Value};

/// Build a JSON Schema tool definition for the MCP tools/list response.
pub fn tool_def(name: &str, description: &str, properties: Value, required: Vec<&str>) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

/// Collect all tool definitions from every module.
pub fn all_tools() -> Vec<Value> {
    let mut tools = Vec::new();
    tools.extend(calls::tools());
    tools.extend(agents::tools());
    tools.extend(queues::tools());
    tools.extend(routing::tools());
    tools.extend(departments::tools());
    tools.extend(knowledge::tools());
    tools.extend(system::tools());
    tools.extend(users::tools());
    tools.extend(reports::tools());
    tools
}

/// Dispatch a tool call to the correct module handler.
pub async fn handle_tool(
    api: &crate::api_client::RvoipApiClient,
    name: &str,
    args: Value,
) -> anyhow::Result<Value> {
    // Try each module in turn; modules return Err for unknown tool names.
    if let Ok(result) = calls::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = agents::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = queues::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = routing::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = departments::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = knowledge::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = system::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = users::handle(api, name, args.clone()).await {
        return Ok(result);
    }
    if let Ok(result) = reports::handle(api, name, args).await {
        return Ok(result);
    }
    anyhow::bail!("unknown tool: {}", name)
}
