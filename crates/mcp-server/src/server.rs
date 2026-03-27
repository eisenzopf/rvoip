use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::api_client::RvoipApiClient;
use crate::prompts;
use crate::resources;
use crate::tools;

/// Run the MCP server over stdio using JSON-RPC 2.0 (one JSON object per line).
pub async fn run_stdio_server(api: RvoipApiClient) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                write_response(&mut stdout, &err_resp).await?;
                continue;
            }
        };

        let method = request["method"].as_str().unwrap_or("");
        let id = &request["id"];

        let response = match method {
            "initialize" => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {},
                            "resources": {},
                            "prompts": {}
                        },
                        "serverInfo": {
                            "name": "rvoip",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                })
            }

            "notifications/initialized" => {
                // Client acknowledgment — no response required
                continue;
            }

            "tools/list" => {
                let tool_list = tools::all_tools();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tool_list }
                })
            }

            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                let args = request["params"]["arguments"].clone();
                let result = handle_tool(&api, name, args).await;
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": result
                        }]
                    }
                })
            }

            "resources/list" => {
                let resource_list = resources::list_resources();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "resources": resource_list }
                })
            }

            "resources/read" => {
                let uri = request["params"]["uri"].as_str().unwrap_or("");
                match resources::read_resource(&api, uri).await {
                    Ok(data) => {
                        let text = serde_json::to_string_pretty(&data).unwrap_or_else(|e| {
                            format!("{{\"error\": \"serialization failed: {}\"}}", e)
                        });
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "contents": [{
                                    "uri": uri,
                                    "mimeType": "application/json",
                                    "text": text
                                }]
                            }
                        })
                    }
                    Err(e) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32602, "message": format!("{}", e) }
                        })
                    }
                }
            }

            "prompts/list" => {
                let prompt_list = prompts::list_prompts();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "prompts": prompt_list }
                })
            }

            "prompts/get" => {
                let prompt_name = request["params"]["name"].as_str().unwrap_or("");
                let prompt_args = request["params"]["arguments"].clone();
                match prompts::get_prompt(prompt_name, &prompt_args) {
                    Ok(messages) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "description": prompt_name,
                                "messages": messages
                            }
                        })
                    }
                    Err(e) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32602, "message": format!("{}", e) }
                        })
                    }
                }
            }

            _ => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("Method not found: {}", method)
                    }
                })
            }
        };

        write_response(&mut stdout, &response).await?;
    }

    Ok(())
}

async fn write_response(
    stdout: &mut tokio::io::Stdout,
    response: &Value,
) -> anyhow::Result<()> {
    let mut out = serde_json::to_string(response)?;
    out.push('\n');
    stdout.write_all(out.as_bytes()).await?;
    stdout.flush().await?;
    Ok(())
}

/// Route a tool call to the correct handler module.
async fn handle_tool(api: &RvoipApiClient, name: &str, args: Value) -> String {
    let result = match name {
        // Calls domain
        "list_active_calls" | "get_call_detail" | "hangup_call" | "get_call_history"
        | "get_call_stats" | "transfer_call" => tools::calls::handle(api, name, args).await,

        // Agents domain
        "list_agents" | "create_agent" | "update_agent" | "delete_agent"
        | "set_agent_status" | "get_agent_performance" => {
            tools::agents::handle(api, name, args).await
        }

        // Queues domain
        "list_queues" | "create_queue" | "get_queue_status" | "assign_call_to_agent"
        | "get_queue_performance" => tools::queues::handle(api, name, args).await,

        // Routing domain
        "get_routing_config" | "list_overflow_policies" | "create_overflow_policy" => {
            tools::routing::handle(api, name, args).await
        }

        // Departments domain
        "list_departments" | "create_department" | "delete_department"
        | "get_department_detail" => {
            tools::departments::handle(api, name, args).await
        }

        // Knowledge domain
        "search_knowledge" | "get_article" | "list_talk_scripts" | "suggest_response" => {
            tools::knowledge::handle(api, name, args).await
        }

        // System domain
        "get_system_health" | "get_dashboard" | "get_audit_log" | "export_config" => {
            tools::system::handle(api, name, args).await
        }

        // Users domain
        "list_users" | "create_user" | "update_user_roles" | "delete_user" => {
            tools::users::handle(api, name, args).await
        }

        // Reports domain
        "generate_daily_report" | "generate_agent_report" | "generate_summary_report" => {
            tools::reports::handle(api, name, args).await
        }

        _ => Err(anyhow::anyhow!("unknown tool: {}", name)),
    };

    match result {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|e| {
            format!("{{\"error\": \"serialization failed: {}\"}}", e)
        }),
        Err(e) => format!("{{\"error\": \"{}\"}}", e),
    }
}
