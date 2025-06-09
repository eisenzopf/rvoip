//! Configuration management for SIPp integration tests

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Main test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    pub session_core: SessionCoreConfig,
    pub sipp: SippConfig,
    pub capture: CaptureConfig,
    pub audio: AudioConfig,
    pub reporting: ReportingConfig,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            session_core: SessionCoreConfig::default(),
            sipp: SippConfig::default(),
            capture: CaptureConfig::default(),
            audio: AudioConfig::default(),
            reporting: ReportingConfig::default(),
        }
    }
}

/// Session-core application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCoreConfig {
    pub server: ServerConfig,
    pub client: ClientConfig,
}

impl Default for SessionCoreConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            client: ClientConfig::default(),
        }
    }
}

/// SIP test server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub binary_path: String,
    pub sip_port: u16,
    pub rtp_port_range: String,
    pub auto_answer: bool,
    pub log_level: String,
    pub response_mode: ResponseMode,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            binary_path: "target/debug/sip_test_server".to_string(),
            sip_port: 5062,
            rtp_port_range: "10000-20000".to_string(),
            auto_answer: true,
            log_level: "debug".to_string(),
            response_mode: ResponseMode::AutoAnswer,
        }
    }
}

/// Server response behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseMode {
    /// Automatically answer all calls with 200 OK
    AutoAnswer,
    /// Respond with 486 Busy Here
    Busy,
    /// Respond with 404 Not Found
    NotFound,
    /// Random responses for testing
    Random,
}

/// SIP test client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub binary_path: String,
    pub local_port: u16,
    pub default_target: String,
    pub max_concurrent_calls: u32,
    pub call_rate: f64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            binary_path: "target/debug/sip_test_client".to_string(),
            local_port: 5061,
            default_target: "127.0.0.1:5060".to_string(),
            max_concurrent_calls: 100,
            call_rate: 1.0,
        }
    }
}

/// SIPp configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SippConfig {
    pub binary_path: String,
    pub scenarios_dir: PathBuf,
    pub audio_dir: PathBuf,
    pub default_rate: u32,
    pub max_concurrent: u32,
    #[serde(with = "duration_serde")]
    pub timeout: Duration,
    pub trace_msg: bool,
    pub trace_screen: bool,
}

impl Default for SippConfig {
    fn default() -> Self {
        Self {
            binary_path: "sipp".to_string(),
            scenarios_dir: PathBuf::from("./scenarios"),
            audio_dir: PathBuf::from("./audio"),
            default_rate: 1,
            max_concurrent: 100,
            timeout: Duration::from_secs(30),
            trace_msg: true,
            trace_screen: false,
        }
    }
}

/// Network capture configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub interface: String,
    pub output_dir: PathBuf,
    pub filter: String,
    pub enabled: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            interface: "lo0".to_string(), // macOS loopback
            output_dir: PathBuf::from("./captures"),
            filter: "port 5060 or port 5061 or port 5062".to_string(),
            enabled: true,
        }
    }
}

/// Audio testing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub codecs: Vec<String>,
    pub sample_rates: Vec<u32>,
    #[serde(with = "duration_serde")]
    pub test_duration: Duration,
    pub quality_threshold: f64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            codecs: vec!["PCMU".to_string(), "PCMA".to_string(), "opus".to_string()],
            sample_rates: vec![8000, 16000, 48000],
            test_duration: Duration::from_secs(10),
            quality_threshold: 95.0,
        }
    }
}

/// Test reporting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingConfig {
    pub output_dir: PathBuf,
    pub formats: Vec<ReportFormat>,
    pub include_pcap: bool,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("./reports"),
            formats: vec![ReportFormat::Html, ReportFormat::Junit, ReportFormat::Json],
            include_pcap: true,
        }
    }
}

/// Supported report formats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportFormat {
    Html,
    Junit,
    Json,
}

impl TestConfig {
    /// Load configuration from file
    pub fn load_from_file(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;
        
        let config: TestConfig = serde_yaml::from_str(&content)
            .with_context(|| "Failed to parse YAML configuration")?;
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let content = serde_yaml::to_string(self)
            .context("Failed to serialize configuration")?;
        
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;
        
        Ok(())
    }
    
    /// Create default configuration file
    pub fn create_default_config(path: &PathBuf) -> Result<()> {
        let config = TestConfig::default();
        config.save_to_file(path)
    }
}

/// Utility module for duration serialization
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    
    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }
    
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    
    #[test]
    fn test_default_config() {
        let config = TestConfig::default();
        assert_eq!(config.session_core.server.sip_port, 5062);
        assert_eq!(config.sipp.default_rate, 1);
        assert!(config.capture.enabled);
    }
    
    #[test]
    fn test_config_serialization() {
        let config = TestConfig::default();
        let yaml = serde_yaml::to_string(&config).expect("Failed to serialize");
        let deserialized: TestConfig = serde_yaml::from_str(&yaml).expect("Failed to deserialize");
        
        assert_eq!(config.session_core.server.sip_port, deserialized.session_core.server.sip_port);
    }
    
    #[test]
    fn test_config_file_operations() {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let path = temp_file.path().to_path_buf();
        
        // Create and save default config
        TestConfig::create_default_config(&path).expect("Failed to create config");
        
        // Load config back
        let loaded_config = TestConfig::load_from_file(&path).expect("Failed to load config");
        
        assert_eq!(loaded_config.session_core.server.sip_port, 5062);
    }
} 