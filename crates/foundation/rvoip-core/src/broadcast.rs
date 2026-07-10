//! Common one-to-many publisher contract shared by UCTP and MOQT adapters.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::capability::CodecInfo;
use crate::error::Result;
use crate::stream::MediaFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BroadcastTransport {
    UctpQuic,
    Moqt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastDescriptor {
    pub transport: BroadcastTransport,
    pub namespace: String,
    pub audio_track: String,
    pub catalog_track: Option<String>,
    pub protocol_version: String,
}

#[async_trait]
pub trait BroadcastPublisher: Send + Sync {
    fn descriptor(&self) -> BroadcastDescriptor;
    fn codec(&self) -> CodecInfo;
    fn frames_out(&self) -> mpsc::Sender<MediaFrame>;
    async fn close(self: Arc<Self>) -> Result<()>;
}
