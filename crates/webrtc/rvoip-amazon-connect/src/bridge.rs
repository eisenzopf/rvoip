//! Direct media bridge between two `MediaStream`s, with transcoding.
//!
//! This replicates the recipe `rvoip_core::Orchestrator::bridge_connections`
//! uses (one [`Transcoder`] per direction when the RTP payload types differ,
//! plus a [`spawn_pump`] each way), but operates directly on two
//! `Arc<dyn MediaStream>` so the connector can bridge an inbound SIP leg (G.711)
//! to the Amazon Connect / Chime leg (Opus) without registering both in an
//! Orchestrator connection table.

use std::sync::Arc;

use rvoip_core::media_graph::{start_media_graph, MediaGraphHandle, MediaGraphPolicy};
use rvoip_core::stream::MediaStream;

use crate::errors::{ConnectError, Result};

/// Handle to a running bidirectional bridge. Dropping it (or calling
/// [`StreamBridge::stop`]) aborts both pump tasks.
pub struct StreamBridge {
    a_graph: MediaGraphHandle,
    b_graph: MediaGraphHandle,
    a_to_b: rvoip_core::ids::MediaRouteId,
    b_to_a: rvoip_core::ids::MediaRouteId,
}

impl StreamBridge {
    /// Abort both pump directions.
    pub fn stop(self) {
        drop(self);
    }

    pub fn a_graph(&self) -> MediaGraphHandle {
        self.a_graph.clone()
    }

    pub fn b_graph(&self) -> MediaGraphHandle {
        self.b_graph.clone()
    }
}

impl Drop for StreamBridge {
    fn drop(&mut self) {
        self.a_graph.remove_sink(self.a_to_b.clone());
        self.b_graph.remove_sink(self.b_to_a.clone());
        self.a_graph.shutdown();
        self.b_graph.shutdown();
    }
}

/// Bridge two media streams bidirectionally, transcoding when their codecs map
/// to different RTP payload types (e.g. G.711-mu ⟷ Opus).
///
/// Each stream's `frames_in()`/`frames_out()` channels are single-take, so this
/// must be called exactly once per stream pair.
pub fn bridge_streams(a: Arc<dyn MediaStream>, b: Arc<dyn MediaStream>) -> Result<StreamBridge> {
    let a_codec = a.codec();
    let b_codec = b.codec();
    let a_graph = start_media_graph(a.frames_in(), a_codec, MediaGraphPolicy::default())
        .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let b_graph = start_media_graph(b.frames_in(), b_codec, MediaGraphPolicy::default())
        .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let a_to_b = a_graph
        .add_sink(b.codec(), b.frames_out())
        .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let b_to_a = match b_graph.add_sink(a.codec(), a.frames_out()) {
        Ok(route) => route,
        Err(error) => {
            a_graph.remove_sink(a_to_b);
            return Err(ConnectError::Mapping(error.to_string()));
        }
    };

    Ok(StreamBridge {
        a_graph,
        b_graph,
        a_to_b,
        b_to_a,
    })
}
