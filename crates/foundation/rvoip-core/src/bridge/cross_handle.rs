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
        a_graph: MediaGraphHandle,
        b_graph: MediaGraphHandle,
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
        a_graph: MediaGraphHandle,
        b_graph: MediaGraphHandle,
        a_to_b: MediaRouteId,
        b_to_a: MediaRouteId,
    },
}

impl CrossBridgeSwapController {
    pub(crate) async fn swap_transcoders(
        self,
        mut a_to_b_swap: TranscoderSwap,
        mut b_to_a_swap: TranscoderSwap,
    ) -> Result<()> {
        match self {
            Self::MediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            } => {
                a_graph
                    .update_route(a_to_b, a_to_b_swap.new_from_pt, a_to_b_swap.new_to_pt)
                    .await?;
                b_graph
                    .update_route(b_to_a, b_to_a_swap.new_from_pt, b_to_a_swap.new_to_pt)
                    .await?;
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
                a_graph,
                b_graph,
                a_to_b: Some(a_to_b),
                b_to_a: Some(b_to_a),
            },
        }
    }

    pub fn media_route_statuses(&self) -> Option<(MediaGraphRouteStatus, MediaGraphRouteStatus)> {
        let CrossBridgeBackend::ManagedMediaGraphs { a_to_b, b_to_a, .. } = &self.backend else {
            return None;
        };
        Some((a_to_b.as_ref()?.status(), b_to_a.as_ref()?.status()))
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
            } => Ok(CrossBridgeSwapController::MediaGraphs {
                a_graph: a_graph.clone(),
                b_graph: b_graph.clone(),
                a_to_b: a_to_b
                    .as_ref()
                    .ok_or(RvoipError::InvalidState("bridge route is stopped"))?
                    .id()
                    .clone(),
                b_to_a: b_to_a
                    .as_ref()
                    .ok_or(RvoipError::InvalidState("bridge route is stopped"))?
                    .id()
                    .clone(),
            }),
            CrossBridgeBackend::LegacyMediaGraphs {
                a_graph,
                b_graph,
                a_to_b,
                b_to_a,
            } => Ok(CrossBridgeSwapController::MediaGraphs {
                a_graph: a_graph.clone(),
                b_graph: b_graph.clone(),
                a_to_b: a_to_b
                    .clone()
                    .ok_or(RvoipError::InvalidState("bridge route is stopped"))?,
                b_to_a: b_to_a
                    .clone()
                    .ok_or(RvoipError::InvalidState("bridge route is stopped"))?,
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

    /// Atomically swap the transcoders on both directions. Used by
    /// `Orchestrator::renegotiate_media` after a successful
    /// adapter-level renegotiation: the new (from_pt, to_pt) pairs
    /// reflect the post-renegotiation codecs on each leg.
    ///
    /// The swap is best-effort: if the swap channel for a direction
    /// is full or closed (pump exited), that direction is skipped
    /// and the call still returns `Ok(())` for the directions that
    /// did swap. A complete failure (no swap channels wired) returns
    /// [`RvoipError::NotImplemented`].
    /// A3 — sends the swap and awaits the per-pump ack so the caller
    /// knows the new codec is live before this returns. Per-direction
    /// ack timeout is 1 second; on timeout the swap is logged but not
    /// retried (the bridge is in degraded state). When `ack` is left
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
