//! Direct media bridge between two `MediaStream`s, with transcoding.
//!
//! This replicates the recipe `rvoip_core::Orchestrator::bridge_connections`
//! uses (one [`Transcoder`] per direction when the RTP payload types differ,
//! plus a [`spawn_pump`] each way), but operates directly on two
//! `Arc<dyn MediaStream>` so the connector can bridge an inbound SIP leg (G.711)
//! to the Amazon Connect / Chime leg (Opus) without registering both in an
//! Orchestrator connection table.

use std::sync::Arc;

use rvoip_core::bridge::codec_to_pt;
use rvoip_core::bridge::frame_pump::spawn_pump;
use rvoip_core::stream::MediaStream;
use rvoip_media_core::codec::transcoding::Transcoder;
use rvoip_media_core::processing::format::FormatConverter;
use tokio::sync::RwLock as TokioRwLock;
use tokio::task::JoinHandle;

use crate::errors::{ConnectError, Result};

/// Handle to a running bidirectional bridge. Dropping it (or calling
/// [`StreamBridge::stop`]) aborts both pump tasks.
pub struct StreamBridge {
    a_to_b: JoinHandle<()>,
    b_to_a: JoinHandle<()>,
}

impl StreamBridge {
    /// Abort both pump directions.
    pub fn stop(self) {
        self.a_to_b.abort();
        self.b_to_a.abort();
    }
}

impl Drop for StreamBridge {
    fn drop(&mut self) {
        self.a_to_b.abort();
        self.b_to_a.abort();
    }
}

/// Bridge two media streams bidirectionally, transcoding when their codecs map
/// to different RTP payload types (e.g. G.711-mu ⟷ Opus).
///
/// Each stream's `frames_in()`/`frames_out()` channels are single-take, so this
/// must be called exactly once per stream pair.
pub fn bridge_streams(a: Arc<dyn MediaStream>, b: Arc<dyn MediaStream>) -> Result<StreamBridge> {
    let a_name = a.codec().name;
    let b_name = b.codec().name;
    let a_pt = codec_to_pt(&a_name)
        .ok_or_else(|| ConnectError::Mapping(format!("unbridgeable codec: {a_name}")))?;
    let b_pt = codec_to_pt(&b_name)
        .ok_or_else(|| ConnectError::Mapping(format!("unbridgeable codec: {b_name}")))?;

    // One transcoder per direction with its own FormatConverter (the converter
    // caches a resampler keyed by input rate; sharing would thrash it on every
    // 8 kHz⟷48 kHz flip). No transcoder needed when both sides share a PT.
    let (transcoder_a_to_b, transcoder_b_to_a) = if a_pt != b_pt {
        (
            Some(Transcoder::new(Arc::new(TokioRwLock::new(
                FormatConverter::new(),
            )))),
            Some(Transcoder::new(Arc::new(TokioRwLock::new(
                FormatConverter::new(),
            )))),
        )
    } else {
        (None, None)
    };

    let a_to_b = spawn_pump(
        "connect:a->b",
        a.frames_in(),
        b.frames_out(),
        transcoder_a_to_b,
        a_pt,
        b_pt,
    );
    let b_to_a = spawn_pump(
        "connect:b->a",
        b.frames_in(),
        a.frames_out(),
        transcoder_b_to_a,
        b_pt,
        a_pt,
    );

    Ok(StreamBridge { a_to_b, b_to_a })
}
