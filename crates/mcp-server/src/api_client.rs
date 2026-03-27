use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

/// HTTP client that talks to the rvoip web-console REST API.
pub struct RvoipApiClient {
    base_url: String,
    token: String,
    client: Client,
}

impl RvoipApiClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client: Client::new(),
        }
    }

    /// Send a GET request and return the response data.
    pub async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .json::<Value>()
            .await?;

        // Handle both wrapped {code,data,message} and direct responses
        if let Some(data) = resp.get("data") {
            Ok(data.clone())
        } else {
            Ok(resp)
        }
    }

    /// Send a POST request with a JSON body and return the response data.
    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?
            .json::<Value>()
            .await?;

        if let Some(data) = resp.get("data") {
            Ok(data.clone())
        } else {
            Ok(resp)
        }
    }

    /// Send a PUT request with a JSON body and return the response data.
    pub async fn put(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let resp = self
            .client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?
            .json::<Value>()
            .await?;

        if let Some(data) = resp.get("data") {
            Ok(data.clone())
        } else {
            Ok(resp)
        }
    }

    /// Send a DELETE request and return the response data.
    pub async fn delete(&self, path: &str) -> Result<Value> {
        let url = format!("{}/api/v1{}", self.base_url, path);
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await?
            .json::<Value>()
            .await?;

        if let Some(data) = resp.get("data") {
            Ok(data.clone())
        } else {
            Ok(resp)
        }
    }
}
