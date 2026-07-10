//! Direct media bridge between two `MediaStream`s, with transcoding.
//!
//! This uses the same reusable `MediaGraph` path as
//! `rvoip_core::Orchestrator::bridge_connections`, but operates directly on
//! two `Arc<dyn MediaStream>` values so the connector can bridge an inbound
//! SIP leg (G.711) to the Amazon Connect / Chime leg (Opus) without
//! registering both in an Orchestrator connection table.

use std::sync::Arc;

use rvoip_core::media_graph::{
    start_media_graph, validate_media_graph_codec, MediaGraphHandle, MediaGraphPolicy,
};
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
/// Each stream's inbound receiver is single-take. A repeated bridge attempt is
/// rejected rather than receiving an already-closed compatibility channel.
pub fn bridge_streams(a: Arc<dyn MediaStream>, b: Arc<dyn MediaStream>) -> Result<StreamBridge> {
    let a_codec = a.codec();
    let b_codec = b.codec();
    // Reject unsupported codecs before either destructive receiver take. This
    // keeps a validation failure from permanently consuming a usable stream.
    validate_media_graph_codec(&a_codec)
        .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    validate_media_graph_codec(&b_codec)
        .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let a_graph = start_media_graph(
        a.try_frames_in()
            .map_err(|error| ConnectError::Mapping(error.to_string()))?,
        a_codec.clone(),
        MediaGraphPolicy::default(),
    )
    .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let b_graph = start_media_graph(
        b.try_frames_in()
            .map_err(|error| ConnectError::Mapping(error.to_string()))?,
        b_codec.clone(),
        MediaGraphPolicy::default(),
    )
    .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let a_to_b = a_graph
        .add_sink(b_codec, b.frames_out())
        .map_err(|error| ConnectError::Mapping(error.to_string()))?;
    let b_to_a = match b_graph.add_sink(a_codec, a.frames_out()) {
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
