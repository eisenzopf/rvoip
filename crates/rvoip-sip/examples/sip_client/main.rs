//! General-purpose terminal SIP client built on the Endpoint facade.

mod audio;
mod config;
mod runtime;
mod smoke;
mod ui;

use std::io;

use clap::Parser;

use crate::audio::list_audio_devices;
use crate::config::{build_runtime_options, Cli};
use crate::runtime::run_tui;
use crate::smoke::run_smoke;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let log_to_stderr = std::env::var_os("RVOIP_SIP_CLIENT_LOG").is_some();
    let subscriber = tracing_subscriber::fmt().with_env_filter(
        std::env::var("RUST_LOG").unwrap_or_else(|_| "rvoip_sip=info,warn".into()),
    );
    if log_to_stderr {
        subscriber.with_writer(io::stderr).init();
    } else {
        subscriber.with_writer(io::sink).init();
    }

    let cli = Cli::parse();
    if cli.list_devices {
        list_audio_devices()?;
        return Ok(());
    }

    let options = build_runtime_options(&cli)?;
    if let Some(role) = cli.test {
        run_smoke(role, options).await?;
        return Ok(());
    }

    run_tui(options).await
}
