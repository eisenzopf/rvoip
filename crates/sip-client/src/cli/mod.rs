//! Command-line interface for the RVOIP SIP client
//!
//! This module provides a comprehensive CLI tool for SIP operations.

pub mod commands;

use clap::{Parser, Subcommand};
use tracing::{info, error};

use crate::{Result, Config, SipClient};

/// RVOIP SIP Client - Make and receive SIP calls from the command line
#[derive(Parser)]
#[command(name = "rvoip-sip-client")]
#[command(about = "A simple SIP client built on the RVOIP stack")]
#[command(version = crate::VERSION)]
pub struct Cli {
    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Configuration file path
    #[arg(short, long)]
    pub config: Option<String>,

    /// Local SIP port (0 for random)
    #[arg(short, long)]
    pub port: Option<u16>,

    /// SIP username
    #[arg(short, long)]
    pub username: Option<String>,

    /// SIP domain
    #[arg(short, long)]
    pub domain: Option<String>,

    /// Command to execute
    #[command(subcommand)]
    pub command: Commands,
}

/// Available CLI commands
#[derive(Subcommand)]
pub enum Commands {
    /// Register with a SIP server
    Register {
        /// Username for registration
        username: String,
        /// Password for authentication
        password: String,
        /// SIP domain (e.g., sip.example.com)
        domain: String,
        /// Registration timeout in seconds
        #[arg(short, long, default_value = "30")]
        timeout: u64,
    },

    /// Make an outgoing call
    Call {
        /// Target URI (e.g., sip:bob@example.com)
        target: String,
        /// Call duration in seconds (0 for until hangup)
        #[arg(short, long, default_value = "0")]
        duration: u64,
        /// Auto-hangup after connection
        #[arg(short, long)]
        auto_hangup: bool,
    },

    /// Wait for and handle incoming calls
    Receive {
        /// Auto-answer incoming calls
        #[arg(short, long)]
        auto_answer: bool,
        /// Maximum call duration in seconds
        #[arg(short, long, default_value = "300")]
        max_duration: u64,
    },

    /// Show client status and statistics
    Status {
        /// Show detailed information
        #[arg(short, long)]
        detailed: bool,
        /// Refresh interval in seconds (0 for one-shot)
        #[arg(short, long, default_value = "0")]
        refresh: u64,
    },

    /// Call center agent mode
    Agent {
        /// Agent queue name
        queue: String,
        /// Call-engine server address
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        server: String,
        /// Agent ID
        #[arg(short, long)]
        agent_id: Option<String>,
    },
}

impl Cli {
    /// Load configuration, applying CLI overrides
    pub fn load_config(&self) -> Result<Config> {
        // Start with default or file config
        let mut config = if let Some(config_path) = &self.config {
            Config::from_file(config_path)?
        } else {
            Config::from_env().unwrap_or_default()
        };

        // Apply CLI overrides
        if let Some(port) = self.port {
            config = config.with_local_port(port);
        }

        if let (Some(username), Some(domain)) = (&self.username, &self.domain) {
            // For CLI, we'll prompt for password or use environment
            let password = std::env::var("SIP_PASSWORD")
                .unwrap_or_else(|_| "password".to_string()); // TODO: Prompt for password
            config = config.with_credentials(username, &password, domain);
        }

        Ok(config)
    }

    /// Setup logging based on verbosity
    pub fn setup_logging(&self) -> Result<()> {
        let level = if self.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        };

        tracing_subscriber::fmt()
            .with_max_level(level)
            .with_target(false)
            .init();

        if self.verbose {
            info!("🔍 Verbose logging enabled");
        }

        Ok(())
    }

    /// Execute the CLI command
    pub async fn execute(&self) -> Result<()> {
        self.setup_logging()?;
        let config = self.load_config()?;

        info!("🚀 Starting RVOIP SIP Client v{}", crate::VERSION);

        match &self.command {
            Commands::Register {
                username,
                password,
                domain,
                timeout,
            } => {
                commands::register::execute(username, password, domain, *timeout, &config).await
            }
            Commands::Call {
                target,
                duration,
                auto_hangup,
            } => {
                commands::call::execute(target, *duration, *auto_hangup, &config).await
            }
            Commands::Receive {
                auto_answer,
                max_duration,
            } => {
                commands::receive::execute(*auto_answer, *max_duration, &config).await
            }
            Commands::Status { detailed, refresh } => {
                commands::status::execute(*detailed, *refresh, &config).await
            }
            Commands::Agent {
                queue,
                server,
                agent_id,
            } => {
                commands::agent::execute(queue, server, agent_id.as_deref(), &config).await
            }
        }
    }
} 