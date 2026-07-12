use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::Result as CoreResult;
use rvoip_core::identity::PrincipalOwnershipKey;
use rvoip_core::ids::{ConnectionId, SessionId, StreamId};
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};
use rvoip_uctp::substrate::{
    PeerMediaConnectionKey, PeerMediaFanoutKey, PeerMediaRegistration, PeerMediaRouteKey,
    PeerMediaRouter, PeerMediaRouterError,
};
use tokio::sync::mpsc;

struct TestMediaStream {
    id: StreamId,
    output: mpsc::Sender<MediaFrame>,
}

impl TestMediaStream {
    fn new(id: StreamId) -> Arc<Self> {
        let (output, _receiver) = mpsc::channel(4);
        Arc::new(Self { id, output })
    }
}

#[async_trait]
impl MediaStream for TestMediaStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
        }
    }

    fn direction(&self) -> Direction {
        Direction::Inbound
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        mpsc::channel(1).1
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.output.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> CoreResult<()> {
        Ok(())
    }
}

fn owner(subject: &str) -> PrincipalOwnershipKey {
    PrincipalOwnershipKey {
        issuer: Some("https://issuer.example".into()),
        tenant: Some("tenant-a".into()),
        subject: subject.into(),
    }
}

fn registration(owner: PrincipalOwnershipKey, route: PeerMediaRouteKey) -> PeerMediaRegistration {
    let (ingress, _receiver) = mpsc::channel(8);
    let stream: Arc<dyn MediaStream> = TestMediaStream::new(route.stream_id.clone());
    PeerMediaRegistration::new(owner, route, stream, ingress)
}

fn commit_route(
    router: &Arc<PeerMediaRouter>,
    owner: PrincipalOwnershipKey,
    route: PeerMediaRouteKey,
) -> Arc<rvoip_uctp::substrate::PeerMediaBinding> {
    router
        .reserve()
        .unwrap()
        .commit(registration(owner, route))
        .unwrap()
}

#[test]
fn peer_ids_are_nonzero_monotonic_and_never_reused() {
    let router = PeerMediaRouter::new();
    let abandoned = router.reserve().unwrap();
    let abandoned_token = abandoned.cancellation_token();
    assert_eq!(abandoned.local_id().get(), 1);
    drop(abandoned);
    assert!(abandoned_token.is_cancelled());

    let threads: Vec<_> = (0..64)
        .map(|_| {
            let router = router.clone();
            std::thread::spawn(move || router.reserve().unwrap().local_id())
        })
        .collect();
    let allocated: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let unique: HashSet<_> = allocated.iter().copied().collect();
    assert_eq!(unique.len(), allocated.len());
    assert!(allocated.iter().all(|local_id| local_id.get() >= 2));

    let next = router.reserve().unwrap();
    assert_eq!(next.local_id().get(), 66);
    assert_eq!(router.snapshot().issued_local_ids, 66);
}

#[test]
fn route_session_and_connection_indexes_remove_exact_bindings() {
    let router = PeerMediaRouter::new();
    let session_a = SessionId::new();
    let session_b = SessionId::new();
    let connection_a = ConnectionId::new();
    let connection_b = ConnectionId::new();
    let connection_c = ConnectionId::new();

    let route_a1 = PeerMediaRouteKey::new(session_a.clone(), connection_a.clone(), StreamId::new());
    let route_a2 = PeerMediaRouteKey::new(session_a.clone(), connection_a.clone(), StreamId::new());
    let route_b = PeerMediaRouteKey::new(session_a.clone(), connection_b.clone(), StreamId::new());
    let route_c = PeerMediaRouteKey::new(session_b.clone(), connection_c.clone(), StreamId::new());

    let binding_a1 = commit_route(&router, owner("alice"), route_a1.clone());
    let binding_a2 = commit_route(&router, owner("alice"), route_a2.clone());
    let binding_b = commit_route(&router, owner("alice"), route_b.clone());
    let binding_c = commit_route(&router, owner("alice"), route_c.clone());

    assert_eq!(router.bindings_for_session(&session_a).len(), 3);
    assert_eq!(router.bindings_for_session(&session_b).len(), 1);
    let connection_a_key = PeerMediaConnectionKey::new(session_a.clone(), connection_a);
    assert_eq!(router.bindings_for_connection(&connection_a_key).len(), 2);
    assert_eq!(
        router.lookup_route(&route_b).unwrap().local_id(),
        binding_b.local_id()
    );

    let removed = router.remove_connection(&connection_a_key);
    assert_eq!(removed.len(), 2);
    assert!(binding_a1.is_cancelled());
    assert!(binding_a2.is_cancelled());
    assert!(!binding_b.is_cancelled());
    assert!(!binding_c.is_cancelled());
    assert!(router.lookup_route(&route_a1).is_none());
    assert!(router.lookup_route(&route_a2).is_none());

    let removed = router.remove_session(&session_a);
    assert_eq!(removed.len(), 1);
    assert!(binding_b.is_cancelled());
    assert_eq!(router.snapshot().session_count, 1);
    assert_eq!(router.snapshot().connection_count, 1);
}

#[test]
fn reservation_commit_is_atomic_and_owner_checks_are_explicit() {
    let router = PeerMediaRouter::new();
    let route = PeerMediaRouteKey::new(SessionId::new(), ConnectionId::new(), StreamId::new());
    let fanout = PeerMediaFanoutKey::new(
        route.session_id.clone(),
        route.connection_id.clone(),
        route.stream_id.clone(),
    );
    let first = router
        .reserve()
        .unwrap()
        .commit(registration(owner("alice"), route.clone()).with_fanout(fanout.clone()))
        .unwrap();
    assert_eq!(first.fanout(), Some(&fanout));

    let duplicate = router.reserve().unwrap();
    let duplicate_id = duplicate.local_id();
    assert!(matches!(
        duplicate.commit(registration(owner("alice"), route.clone())),
        Err(PeerMediaRouterError::DuplicateRoute {
            existing_local_id,
            ..
        }) if existing_local_id == first.local_id()
    ));
    assert!(router.lookup(duplicate_id).is_none());
    assert_eq!(router.snapshot().reserved_local_ids.len(), 0);

    assert!(matches!(
        router.lookup_owned(first.local_id(), &owner("mallory")),
        Err(PeerMediaRouterError::OwnerMismatch)
    ));
    assert!(matches!(
        router.remove_local_id_owned(first.local_id(), &owner("mallory")),
        Err(PeerMediaRouterError::OwnerMismatch)
    ));
    assert!(!first.is_cancelled());
    assert!(router
        .remove_local_id_owned(first.local_id(), &owner("alice"))
        .unwrap()
        .is_some());
    assert!(first.is_cancelled());
}

#[test]
fn shutdown_cancels_active_and_reserved_routes_and_rejects_new_work() {
    let router = PeerMediaRouter::new();
    let route = PeerMediaRouteKey::new(SessionId::new(), ConnectionId::new(), StreamId::new());
    let binding = commit_route(&router, owner("alice"), route);
    let pending = router.reserve().unwrap();
    let pending_token = pending.cancellation_token();

    let removed = router.shutdown();
    assert_eq!(removed.len(), 1);
    assert!(binding.is_cancelled());
    assert!(pending_token.is_cancelled());
    assert!(router.is_shutdown());
    assert!(matches!(
        router.reserve(),
        Err(PeerMediaRouterError::ShuttingDown)
    ));

    let snapshot = router.snapshot();
    assert!(snapshot.shutdown);
    assert!(snapshot.bindings.is_empty());
    assert!(snapshot.reserved_local_ids.is_empty());
    assert!(router.shutdown().is_empty());
}
