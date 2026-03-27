use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CliConfig {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:3000".into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub token: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutputConfig {
    pub format: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: "table".into(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".rvoip")
}

pub fn load_config() -> CliConfig {
    let path = config_dir().join("config.toml");
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    } else {
        CliConfig::default()
    }
}

pub fn save_config(config: &CliConfig) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let content = toml::to_string_pretty(config)?;
    std::fs::write(dir.join("config.toml"), content)?;
    Ok(())
}
