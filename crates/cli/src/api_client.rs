use anyhow::Context;

pub struct ApiClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client: reqwest::Client::new(),
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base_url, path)
    }

    fn auth_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.token.is_empty() {
            builder
        } else {
            builder.bearer_auth(&self.token)
        }
    }

    pub async fn get(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let url = self.api_url(path);
        let req = self.client.get(&url);
        let resp = self
            .auth_request(req)
            .send()
            .await
            .context("failed to send GET request")?;
        let status = resp.status();
        let body = resp
            .json::<serde_json::Value>()
            .await
            .context("failed to parse response JSON")?;
        if !status.is_success() {
            let msg = body["message"]
                .as_str()
                .unwrap_or("unknown error");
            anyhow::bail!("API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    pub async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let url = self.api_url(path);
        let req = self.client.post(&url).json(body);
        let resp = self
            .auth_request(req)
            .send()
            .await
            .context("failed to send POST request")?;
        let status = resp.status();
        let resp_body = resp
            .json::<serde_json::Value>()
            .await
            .context("failed to parse response JSON")?;
        if !status.is_success() {
            let msg = resp_body["message"]
                .as_str()
                .unwrap_or("unknown error");
            anyhow::bail!("API error ({}): {}", status, msg);
        }
        Ok(resp_body)
    }

    pub async fn put(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let url = self.api_url(path);
        let req = self.client.put(&url).json(body);
        let resp = self
            .auth_request(req)
            .send()
            .await
            .context("failed to send PUT request")?;
        let status = resp.status();
        let resp_body = resp
            .json::<serde_json::Value>()
            .await
            .context("failed to parse response JSON")?;
        if !status.is_success() {
            let msg = resp_body["message"]
                .as_str()
                .unwrap_or("unknown error");
            anyhow::bail!("API error ({}): {}", status, msg);
        }
        Ok(resp_body)
    }

    pub async fn delete(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        let url = self.api_url(path);
        let req = self.client.delete(&url);
        let resp = self
            .auth_request(req)
            .send()
            .await
            .context("failed to send DELETE request")?;
        let status = resp.status();
        let resp_body = resp
            .json::<serde_json::Value>()
            .await
            .context("failed to parse response JSON")?;
        if !status.is_success() {
            let msg = resp_body["message"]
                .as_str()
                .unwrap_or("unknown error");
            anyhow::bail!("API error ({}): {}", status, msg);
        }
        Ok(resp_body)
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/api/v1/auth/login", self.base_url);
        let body = serde_json::json!({ "username": username, "password": password });
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("failed to send login request")?;
        let status = resp.status();
        let resp_body = resp
            .json::<serde_json::Value>()
            .await
            .context("failed to parse login response")?;
        if !status.is_success() {
            let msg = resp_body["message"]
                .as_str()
                .unwrap_or("login failed");
            anyhow::bail!("Login error ({}): {}", status, msg);
        }
        Ok(resp_body)
    }
}
