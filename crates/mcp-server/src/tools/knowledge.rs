use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "search_knowledge",
            "description": "Search knowledge base articles by keyword and optional category",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "search": { "type": "string", "description": "Search query string" },
                    "category": { "type": "string", "description": "Optional category filter" }
                },
                "required": ["search"]
            }
        }),
        json!({
            "name": "get_article",
            "description": "Get a specific knowledge base article by ID",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "article_id": { "type": "string", "description": "The article ID" }
                },
                "required": ["article_id"]
            }
        }),
        json!({
            "name": "list_talk_scripts",
            "description": "List talk scripts with optional category filter",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "category": { "type": "string", "description": "Optional category filter" }
                },
                "required": []
            }
        }),
        json!({
            "name": "suggest_response",
            "description": "Suggest talk scripts matching a given scenario or keyword",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scenario": { "type": "string", "description": "The scenario or keyword to match scripts against" }
                },
                "required": ["scenario"]
            }
        }),
    ]
}

pub async fn handle(api: &crate::api_client::RvoipApiClient, name: &str, args: Value) -> anyhow::Result<Value> {
    match name {
        "search_knowledge" => {
            let search = args["search"].as_str().unwrap_or_default();
            let category = args["category"].as_str().unwrap_or_default();
            let mut path = format!("/knowledge/articles?search={}", search);
            if !category.is_empty() {
                path.push_str(&format!("&category={}", category));
            }
            api.get(&path).await
        }
        "get_article" => {
            let article_id = args["article_id"].as_str().unwrap_or_default();
            api.get(&format!("/knowledge/articles/{}", article_id)).await
        }
        "list_talk_scripts" => {
            let category = args["category"].as_str().unwrap_or_default();
            if category.is_empty() {
                api.get("/knowledge/scripts").await
            } else {
                api.get(&format!("/knowledge/scripts?category={}", category)).await
            }
        }
        "suggest_response" => {
            let scenario = args["scenario"].as_str().unwrap_or_default();
            api.get(&format!("/knowledge/scripts?category={}", scenario)).await
        }
        _ => anyhow::bail!("unknown tool: {}", name),
    }
}
