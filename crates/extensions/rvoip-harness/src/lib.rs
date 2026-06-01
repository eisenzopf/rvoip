//! # rvoip-harness
//!
//! Re-exports the provider trait surface defined in
//! [`rvoip_core_traits::harness`] (post-V2.A) and supplies no-op
//! default implementations useful for tests + harness-disabled
//! builds.
//!
//! Per `rvoip-core/INTERFACE_DESIGN.md` §2.1 the trait shapes live in
//! `rvoip-core-traits` (so both the Orchestrator and external
//! provider crates can depend on them without cycling), and concrete
//! provider crates depend on this re-export crate.

pub use rvoip_core_traits::harness::*;

use async_trait::async_trait;
use rvoip_core_traits::error::Result;
use rvoip_core_traits::ids::ConnectionId;
use rvoip_core_traits::stream::MediaFrame;
use std::sync::Mutex;

pub struct NoOpAsrProvider;
pub struct NoOpAsrStream;

#[async_trait]
impl AsrProvider for NoOpAsrProvider {
    async fn open_stream(
        &self,
        _conn: ConnectionId,
        _config: AsrConfig,
    ) -> Result<Box<dyn AsrStream>> {
        Ok(Box::new(NoOpAsrStream))
    }
}

#[async_trait]
impl AsrStream for NoOpAsrStream {
    async fn push(&self, _frame: MediaFrame) -> Result<()> {
        Ok(())
    }
    async fn next(&self) -> Option<AsrResult> {
        None
    }
    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct NoOpTtsProvider;
pub struct NoOpTtsPlayback;

#[async_trait]
impl TtsProvider for NoOpTtsProvider {
    async fn synthesize(&self, _request: TtsRequest) -> Result<Box<dyn TtsPlayback>> {
        Ok(Box::new(NoOpTtsPlayback))
    }
}

#[async_trait]
impl TtsPlayback for NoOpTtsPlayback {
    async fn next_frame(&self) -> Option<MediaFrame> {
        None
    }
    async fn cancel(&self) -> Result<()> {
        Ok(())
    }
}

pub struct ListenOnlyDialog;

#[async_trait]
impl DialogManager for ListenOnlyDialog {
    async fn turn(&self, _t: &AsrResult) -> Result<DialogAction> {
        Ok(DialogAction::Listen)
    }
}

pub struct VecRecordingSink {
    inner: Mutex<Vec<u8>>,
    url: String,
}

impl VecRecordingSink {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            url: url.into(),
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.inner
            .lock()
            .expect("vec recording sink lock poisoned")
            .clone()
    }
}

#[async_trait]
impl RecordingSink for VecRecordingSink {
    async fn write(&self, frame: MediaFrame) -> Result<()> {
        self.inner
            .lock()
            .expect("vec recording sink lock poisoned")
            .extend_from_slice(&frame.payload);
        Ok(())
    }
    async fn close(&self) -> Result<RecordingArtifact> {
        let g = self.inner.lock().expect("vec recording sink lock poisoned");
        Ok(RecordingArtifact {
            url: self.url.clone(),
            bytes_written: g.len() as u64,
            duration_ms: 0,
            content_hash: String::new(),
        })
    }
}
