//! `CrossBridgeHandle` — the cross-transport sibling of the SIP-fast-path
//! `BridgeHandle` re-exported in `super`.
//!
//! Owns the abort handles for the two frame-pump tasks that copy media
//! between the bridged Connections. `Drop` aborts both pumps so an
//! `unbridge_connections` call (or the Orchestrator going away) tears
//! the bridge down promptly.
//!
//! Gap plan §4.2 v1 punch list — also holds the per-direction swap
//! channels used by `Self::swap_transcoders` to hot-swap the pump
//! transcoders after a mid-call codec renegotiation. Senders are
//! `Some(_)` for bridges built via the swap-aware path; the legacy
//! `new` constructor leaves them `None` for backward compatibility.

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use super::frame_pump::TranscoderSwap;
use crate::error::{Result, RvoipError};
use crate::ids::{BridgeId, ConnectionId, MediaRouteId};
use crate::media_graph::{
    ManagedMediaRoute, MediaGraphHandle, MediaGraphRouteState, MediaGraphRouteStatus,
};

enum CrossBridgeBackend {
    Pumps {
        a_to_b: AbortHandle,
        b_to_a: AbortHandle,
        swap_a_to_b: Option<mpsc::Sender<TranscoderSwap>>,
        swap_b_to_a: Option<mpsc::Sender<TranscoderSwap>>,
    },
    ManagedMediaGraphs {
        a_graph: Option<MediaGraphHandle>,
        b_graph: Option<MediaGraphHandle>,
        a_to_b: Option<ManagedMediaRoute>,
        b_to_a: Option<ManagedMediaRoute>,
    },
    LegacyMediaGraphs {
        a_graph: MediaGraphHandle,
        b_graph: MediaGraphHandle,
        a_to_b: Option<MediaRouteId>,
        b_to_a: Option<MediaRouteId>,
    },
}

/// Cloneable snapshot of only the state needed for a transcoder swap. The
/// Orchestrator captures this while holding its bridge-map guard, then drops
/// the guard before any channel or media-graph await.
#[derive(Clone)]
pub(crate) enum CrossBridgeSwapController {
    Pumps {
        a_to_b: mpsc::Sender<TranscoderSwap>,
        b_to_a: mpsc::Sender<TranscoderSwap>,
    },
    MediaGraphs {
        a_to_b: Option<(MediaGraphHandle, MediaRouteId)>,
        b_to_a: Option<(MediaGraphHandle, MediaRouteId)>,
    },
}

impl CrossBridgeSwapController {
    pub(crate) async fn swap_transcoders(
        self,
        mut a_to_b_swap: TranscoderSwap,
        mut b_to_a_swap: TranscoderSwap,
    ) -> Result<()> {
        match self {
            Self::MediaGraphs { a_to_b, b_to_a } => {
                if let Some((graph, route)) = a_to_b {
                    graph
                        .update_route(route, a_to_b_swap.new_from_pt, a_to_b_swap.new_to_pt)
                        .await?;
                }
                if let Some((graph, route)) = b_to_a {
                    graph
                        .update_route(route, b_to_a_swap.new_from_pt, b_to_a_swap.new_to_pt)
                        .await?;
                }
                Ok(())
            }
            Self::Pumps { a_to_b, b_to_a } => {
                // Await acknowledgements from the pumps so successful return
                // means both directions observed the new codec state.
                let (a_ack_tx, a_ack_rx) = tokio::sync::oneshot::channel();
                let (b_ack_tx, b_ack_rx) = tokio::sync::oneshot::channel();
                a_to_b_swap.ack = Some(a_ack_tx);
                b_to_a_swap.ack = Some(b_ack_tx);

                // A closed receiver means that direction is already ending.
                // Preserve the established best-effort behavior.
                let a_send_ok = a_to_b.send(a_to_b_swap).await.is_ok();
                let b_send_ok = b_to_a.send(b_to_a_swap).await.is_ok();

                let timeout = std::time::Duration::from_secs(1);
                if a_send_ok {
                    let _ = tokio::time::timeout(timeout, a_ack_rx).await;
                }
                if b_send_ok {
                    let _ = tokio::time::timeout(timeout, b_ack_rx).await;
                }
                Ok(())
            }
        }
    }
}

pub struct CrossBridgeHandle {
    pub id: BridgeId,
    pub a: ConnectionId,
    pub b: ConnectionId,
    pub created_at: DateTime<Utc>,
    backend: CrossBridgeBackend,
}

impl CrossBridgeHandle {
    pub fn new(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_to_b: AbortHandle,
        b_to_a: AbortHandle,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            backend: CrossBridgeBackend::Pumps {
                a_to_b,
                b_to_a,
                swap_a_to_b: None,
                swap_b_to_a: None,
            },
        }
    }

    /// Gap plan §4.2 v1 punch list — variant of [`Self::new`] that
    /// captures the per-direction swap-channel senders so the bridge
    /// can hot-swap its transcoders after a codec renegotiation.
    pub fn with_swap_channels(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_to_b: AbortHandle,
        b_to_a: AbortHandle,
        swap_a_to_b: mpsc::Sender<TranscoderSwap>,
        swap_b_to_a: mpsc::Sender<TranscoderSwap>,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            backend: CrossBridgeBackend::Pumps {
                a_to_b,
                b_to_a,
                swap_a_to_b: Some(swap_a_to_b),
                swap_b_to_a: Some(swap_b_to_a),
            },
        }
    }

    /// Build a bridge whose two directions are routes on reusable
    /// single-consumer media graphs. Removing this handle detaches only the
    /// peer routes; observers and broadcast publishers remain attached.
    pub fn with_media_graphs(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_graph: MediaGraphHandle,
        b_graph: MediaGraphHandle,
        a_to_b: MediaRouteId,
        b_to_a: MediaRouteId,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            backend: CrossBridgeBackend::LegacyMediaGraphs {
                a_graph,
                b_graph,
                a_to_b: Some(a_to_b),
                b_to_a: Some(b_to_a),
            },
        }
    }

    /// Managed-route variant used by the Orchestrator. The original
    /// `with_media_graphs` constructor remains source compatible for callers
    /// that still own route IDs directly.
    pub fn with_managed_media_graphs(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_graph: MediaGraphHandle,
        b_graph: MediaGraphHandle,
        a_to_b: ManagedMediaRoute,
        b_to_a: ManagedMediaRoute,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            backend: CrossBridgeBackend::ManagedMediaGraphs {
                a_graph: Some(a_graph),
                b_graph: Some(b_graph),
                a_to_b: Some(a_to_b),
                b_to_a: Some(b_to_a),
            },
        }
    }

    /// Managed-route variant in which either media direction may be absent.
    /// A graph is retained only for an enabled source direction. Construction
    /// is crate-private because the Orchestrator validates the complete plan
    /// and transport sinks before it transfers source-receiver ownership.
    pub(crate) fn with_directional_managed_media_graphs(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_to_b: Option<(MediaGraphHandle, ManagedMediaRoute)>,
        b_to_a: Option<(MediaGraphHandle, ManagedMediaRoute)>,
    ) -> Self {
        let (a_graph, a_to_b) = match a_to_b {
            Some((graph, route)) => (Some(graph), Some(route)),
            None => (None, None),
        };
        let (b_graph, b_to_a) = match b_to_a {
            Some((graph, route)) => (Some(graph), Some(route)),
            None => (None, None),
        };
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            backend: CrossBridgeBackend::ManagedMediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            },
        }
    }

    pub fn media_route_statuses(&self) -> Option<(MediaGraphRouteStatus, MediaGraphRouteStatus)> {
        let CrossBridgeBackend::ManagedMediaGraphs { a_to_b, b_to_a, .. } = &self.backend else {
            return None;
        };
        Some((a_to_b.as_ref()?.status(), b_to_a.as_ref()?.status()))
    }

    pub(crate) fn managed_media_route_statuses(&self) -> Vec<MediaGraphRouteStatus> {
        let CrossBridgeBackend::ManagedMediaGraphs { a_to_b, b_to_a, .. } = &self.backend else {
            return Vec::new();
        };
        [a_to_b, b_to_a]
            .into_iter()
            .filter_map(|route| route.as_ref().map(ManagedMediaRoute::status))
            .collect()
    }

    /// Capture the swap channels or graph route IDs without retaining a
    /// reference to this handle. This is crate-private so the public bridge
    /// API and constructors remain unchanged.
    pub(crate) fn swap_controller(&self) -> Result<CrossBridgeSwapController> {
        match &self.backend {
            CrossBridgeBackend::Pumps {
                swap_a_to_b,
                swap_b_to_a,
                ..
            } => Ok(CrossBridgeSwapController::Pumps {
                a_to_b: swap_a_to_b.clone().ok_or(RvoipError::NotImplemented(
                    "CrossBridgeHandle::swap_transcoders — bridge built without swap channels",
                ))?,
                b_to_a: swap_b_to_a.clone().ok_or(RvoipError::NotImplemented(
                    "CrossBridgeHandle::swap_transcoders — bridge built without swap channels",
                ))?,
            }),
            CrossBridgeBackend::ManagedMediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            } => {
                let a_to_b = match (a_graph, a_to_b) {
                    (Some(graph), Some(route)) => Some((graph.clone(), route.id().clone())),
                    (None, None) => None,
                    _ => {
                        return Err(RvoipError::InvalidState(
                            "A-to-B graph and route ownership diverged",
                        ));
                    }
                };
                let b_to_a = match (b_graph, b_to_a) {
                    (Some(graph), Some(route)) => Some((graph.clone(), route.id().clone())),
                    (None, None) => None,
                    _ => {
                        return Err(RvoipError::InvalidState(
                            "B-to-A graph and route ownership diverged",
                        ));
                    }
                };
                if a_to_b.is_none() && b_to_a.is_none() {
                    return Err(RvoipError::InvalidState("bridge routes are stopped"));
                }
                Ok(CrossBridgeSwapController::MediaGraphs { a_to_b, b_to_a })
            }
            CrossBridgeBackend::LegacyMediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            } => Ok(CrossBridgeSwapController::MediaGraphs {
                a_to_b: Some((
                    a_graph.clone(),
                    a_to_b
                        .clone()
                        .ok_or(RvoipError::InvalidState("bridge route is stopped"))?,
                )),
                b_to_a: Some((
                    b_graph.clone(),
                    b_to_a
                        .clone()
                        .ok_or(RvoipError::InvalidState("bridge route is stopped"))?,
                )),
            }),
        }
    }

    /// Converge both media directions before the orchestrator reports the
    /// bridge removed. Drop remains a best-effort fallback for cancellation.
    pub async fn stop(&mut self) -> Result<()> {
        match &mut self.backend {
            CrossBridgeBackend::Pumps { a_to_b, b_to_a, .. } => {
                a_to_b.abort();
                b_to_a.abort();
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while !a_to_b.is_finished() || !b_to_a.is_finished() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .map_err(|_| RvoipError::InvalidState("bridge pump shutdown timed out"))?;
                Ok(())
            }
            CrossBridgeBackend::ManagedMediaGraphs { a_to_b, b_to_a, .. } => {
                let a = a_to_b.take();
                let b = b_to_a.take();
                let a_status = a.as_ref().map(ManagedMediaRoute::status);
                let b_status = b.as_ref().map(ManagedMediaRoute::status);
                let (a_result, b_result) = tokio::join!(
                    async move {
                        match a {
                            Some(route) => route.remove().await,
                            None => Ok(false),
                        }
                    },
                    async move {
                        match b {
                            Some(route) => route.remove().await,
                            None => Ok(false),
                        }
                    }
                );
                for (result, status) in [(a_result, a_status), (b_result, b_status)] {
                    if let Err(error) = result {
                        let already_terminal = status.is_some_and(|status| {
                            matches!(status.state(), MediaGraphRouteState::Terminal(_))
                        });
                        if !already_terminal {
                            return Err(error);
                        }
                    }
                }
                Ok(())
            }
            CrossBridgeBackend::LegacyMediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            } => {
                let a = a_to_b.take();
                let b = b_to_a.take();
                let (a_result, b_result) = tokio::join!(
                    async {
                        match a {
                            Some(route) => a_graph.remove_sink_and_wait(route).await.map(|_| ()),
                            None => Ok(()),
                        }
                    },
                    async {
                        match b {
                            Some(route) => b_graph.remove_sink_and_wait(route).await.map(|_| ()),
                            None => Ok(()),
                        }
                    }
                );
                a_result?;
                b_result?;
                Ok(())
            }
        }
    }

    /// Swap the transcoder on every enabled media direction. Used by
    /// `Orchestrator::renegotiate_media` after a successful
    /// adapter-level renegotiation: the new (from_pt, to_pt) pairs
    /// reflect the post-renegotiation codecs on each leg.
    ///
    /// Disabled graph directions are intentionally absent and skipped. An
    /// enabled direction that cannot be updated returns an error so the
    /// caller cannot report a successful renegotiation with stale media.
    /// Pump-backed bridges send the swap and await each per-pump ack so the
    /// caller knows the new codec is live before this returns. Per-direction
    /// ack timeout is 1 second; on timeout the swap is not retried because
    /// the bridge may already be in a degraded state. When `ack` is left
    /// `None` on the inputs, the call returns immediately after the
    /// send (legacy fire-and-forget).
    pub async fn swap_transcoders(
        &self,
        a_to_b_swap: TranscoderSwap,
        b_to_a_swap: TranscoderSwap,
    ) -> Result<()> {
        self.swap_controller()?
            .swap_transcoders(a_to_b_swap, b_to_a_swap)
            .await
    }
}

impl Drop for CrossBridgeHandle {
    fn drop(&mut self) {
        match &mut self.backend {
            CrossBridgeBackend::Pumps { a_to_b, b_to_a, .. } => {
                a_to_b.abort();
                b_to_a.abort();
            }
            CrossBridgeBackend::ManagedMediaGraphs { a_to_b, b_to_a, .. } => {
                a_to_b.take();
                b_to_a.take();
            }
            CrossBridgeBackend::LegacyMediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            } => {
                if let Some(route) = a_to_b.take() {
                    a_graph.remove_sink(route);
                }
                if let Some(route) = b_to_a.take() {
                    b_graph.remove_sink(route);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CodecInfo;
    use crate::media_graph::{start_media_graph, MediaGraphPolicy};

    fn codec(name: &str, clock_rate_hz: u32) -> CodecInfo {
        CodecInfo {
            name: name.into(),
            clock_rate_hz,
            channels: 1,
            fmtp: None,
        }
    }

    fn swap(from: u8, to: u8) -> TranscoderSwap {
        TranscoderSwap {
            new_transcoder: None,
            new_from_pt: from,
            new_to_pt: to,
            ack: None,
        }
    }

    #[tokio::test]
    async fn directional_media_graph_swap_updates_only_the_enabled_route() {
        for a_to_b_enabled in [true, false] {
            let (_source_tx, source_rx) = mpsc::channel(4);
            let (source_codec, target_codec, expected_source_pt, expected_target_pt) =
                if a_to_b_enabled {
                    (codec("PCMU", 8_000), codec("opus", 48_000), 8, 111)
                } else {
                    (codec("opus", 48_000), codec("PCMU", 8_000), 111, 8)
                };
            let graph = start_media_graph(source_rx, source_codec, MediaGraphPolicy::default())
                .expect("start graph");
            let (sink_tx, _sink_rx) = mpsc::channel(4);
            let route = graph
                .add_managed_sink(target_codec, sink_tx)
                .expect("add managed route");
            route.wait_active().await.expect("route active");

            let (a_to_b, b_to_a) = if a_to_b_enabled {
                (Some((graph.clone(), route)), None)
            } else {
                (None, Some((graph.clone(), route)))
            };
            let handle = CrossBridgeHandle::with_directional_managed_media_graphs(
                BridgeId::new(),
                ConnectionId::new(),
                ConnectionId::new(),
                a_to_b,
                b_to_a,
            );
            handle
                .swap_transcoders(swap(8, 111), swap(111, 8))
                .await
                .expect("directional swap succeeds");

            let snapshot = graph.snapshot().await;
            assert_eq!(snapshot.source_payload_type, expected_source_pt);
            assert_eq!(snapshot.sinks.len(), 1);
            assert_eq!(snapshot.sinks[0].target_payload_type, expected_target_pt);
        }
    }
}
