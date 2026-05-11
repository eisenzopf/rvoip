//! General-purpose terminal SIP client built on the
//! [`Endpoint`](rvoip_sip::Endpoint) facade.
//!
//! Acts as both an interactive softphone (ratatui TUI with CPAL audio I/O)
//! and a non-interactive smoke harness used by the integration suites.
//! Lighter-weight examples for the other peer APIs live in the sibling
//! lanes under `examples/`; this one demonstrates an end-to-end softphone
//! built on `Endpoint`.
//!
//! Quick start:
//!
//! ```sh
//! cargo run -p rvoip-sip --example sip_client -- --help
//! cargo run -p rvoip-sip --example sip_client -- --preset asterisk-1001
//! cargo run -p rvoip-sip --example sip_client -- --config my-config.json
//! cargo run -p rvoip-sip --example sip_client -- --list-devices
//! ```
//!
//! Set `RVOIP_SIP_CLIENT_LOG=1` to send `tracing` output to stderr (otherwise
//! it is discarded so it does not corrupt the TUI). See
//! `examples/sip_client/README.md` for presets, JSON config layout, smoke-test
//! flags, and audio device selection.

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
