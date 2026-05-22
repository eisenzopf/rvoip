//! Optional pcap capture hook.
//!
//! When `RVOIP_PERF_PCAP=1` is set, [`PcapRecorder::start`] spawns a
//! `tcpdump` (Linux) / `tcpdump` on `lo0` (macOS) child process that
//! captures the wire for the duration of the run, writing to
//! `target/perf-results/<scenario>.pcap`. Failure to spawn is logged
//! but not fatal — the perf run continues without the capture.
//!
//! Useful as a credibility artifact (Wireshark-loadable proof of what
//! actually crossed the wire) alongside the JSON metrics.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct PcapRecorder {
    child: Option<Child>,
    path: PathBuf,
}

impl PcapRecorder {
    /// Returns `Some(recorder)` if `RVOIP_PERF_PCAP=1` is set and
    /// tcpdump spawned successfully; `None` otherwise. Pass the
    /// scenario name so the file lands at
    /// `target/perf-results/<scenario>.pcap`.
    pub fn maybe_start(scenario: &str, target_dir: &Path) -> Option<Self> {
        if std::env::var("RVOIP_PERF_PCAP").ok().as_deref() != Some("1") {
            return None;
        }
        let path = target_dir.join(format!("{scenario}.pcap"));

        // macOS / Linux loopback interface differs ("lo0" vs "lo"); try
        // both. We don't filter by port so the entire loopback stream
        // is captured; perf runs are short enough that this is fine.
        let iface = if cfg!(target_os = "macos") { "lo0" } else { "lo" };
        let result = Command::new("tcpdump")
            .args(["-i", iface, "-w"])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match result {
            Ok(child) => {
                eprintln!(
                    "[pcap] capturing on {iface} → {} (env RVOIP_PERF_PCAP=1)",
                    path.display()
                );
                Some(Self {
                    child: Some(child),
                    path,
                })
            }
            Err(e) => {
                eprintln!(
                    "[pcap] requested via RVOIP_PERF_PCAP=1 but tcpdump spawn failed: {e}",
                );
                None
            }
        }
    }

    /// Stop the capture. Best-effort — if `tcpdump` is missing /
    /// privileged-only on this host the spawn already failed earlier.
    pub fn stop(mut self) -> PathBuf {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.path
    }
}
