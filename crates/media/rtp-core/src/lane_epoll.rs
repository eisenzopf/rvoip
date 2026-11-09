//! A lane's level-triggered epoll for its graduated receive entries
//! ([`MediaExecutorConfig::lane_rx_readiness`], Linux).
//!
//! A receive loop that graduated into its lane's table
//! ([`MediaExecutorConfig::callback_rx_kernel`]) reads its socket the way the
//! async loop does: on its readiness edge it reads until the socket is
//! empty, so every edge ends with one read that finds nothing (`EAGAIN`),
//! and an owner deadline served between edges reads once more. With one
//! datagram per edge, the common case for a 20 ms media stream, that is an
//! empty read per datagram (production measured about 1.55 empty reads per
//! received datagram on this path).
//!
//! Here the lane keeps one epoll descriptor of its own, with each graduated
//! socket registered *level-triggered* for `EPOLLIN` and keyed by its table
//! slot and generation. A pass asks it which sockets hold a datagram
//! (`epoll_wait` with a zero timeout) and reads each reported socket once.
//! Level triggering reports a socket again while it still holds data, so the
//! pass asks again until the answer is empty (one empty `epoll_wait` per pass
//! that read, not one empty `recvfrom` per datagram) or its budget is spent.
//! A read is made only for a report, so it never finds the socket empty, and
//! an owner deadline never reads the socket at all.
//!
//! The epoll descriptor itself is registered with the lane's Tokio reactor
//! ([`AsyncFd`], edge-triggered there): a datagram arriving at any of its
//! sockets makes it readable, which wakes the lane between passes. The
//! readiness Tokio records for it is cleared only after an `epoll_wait`
//! returned nothing, through the guard of the poll that saw it set (Tokio's
//! tick check), so a datagram that arrives between that empty wait and the
//! re-registration of the waker is reported by the reactor's next turn and
//! never lost.
//!
//! The sockets stay registered with the Tokio reactor too (the transport
//! sends on them); nothing polls their read readiness while they are
//! registered here. A reported socket is read with a direct `recvfrom`
//! ([`recv_from_ready`]), not through Tokio's readiness check, which may not
//! have seen the datagram's edge yet.
//!
//! Elsewhere than Linux the switch reads as off and this module is a stub
//! that never creates an epoll, so entries keep the stage-3 reads.
//!
//! The `unsafe` here is system calls on descriptors this module owns or
//! borrows for the duration of the call; see each `SAFETY` note.
//!
//! [`AsyncFd`]: tokio::io::unix::AsyncFd
//! [`MediaExecutorConfig::lane_rx_readiness`]: crate::executor::MediaExecutorConfig::lane_rx_readiness
//! [`MediaExecutorConfig::callback_rx_kernel`]: crate::executor::MediaExecutorConfig::callback_rx_kernel

use std::io;
use std::net::SocketAddr;
use std::task::{Context, Poll};

/// A socket's raw descriptor (`-1`: none).
pub(crate) type SocketFd = i32;

/// Reports one `epoll_wait` returns at most (the receive service budget of a
/// pass).
pub(crate) const MAX_EVENTS: usize = 256;

/// The 64-bit epoll key of a table entry: its slot and generation.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[inline]
pub(crate) fn key(slot: u32, generation: u32) -> u64 {
    (u64::from(generation) << 32) | u64::from(slot)
}

/// [`key`] back to (slot, generation).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[inline]
pub(crate) fn decode(key: u64) -> (u32, u32) {
    (key as u32, (key >> 32) as u32)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::sync::Arc;
    use tokio::io::unix::{AsyncFd, AsyncFdReadyGuard};

    /// The lane's epoll (see the module docs).
    pub(crate) struct LaneEpoll {
        fd: Arc<AsyncFd<OwnedFd>>,
        events: Vec<libc::epoll_event>,
        /// The socket registered for each table slot (`-1`: none).
        registered: Vec<RawFd>,
        count: usize,
    }

    impl LaneEpoll {
        /// A new epoll descriptor, registered with the current runtime's
        /// reactor. `Err` outside a runtime with I/O, or when the kernel
        /// refuses a descriptor.
        pub(crate) fn new() -> io::Result<Self> {
            if tokio::runtime::Handle::try_current().is_err() {
                return Err(io::Error::other("no Tokio runtime on this thread"));
            }
            // SAFETY: `epoll_create1` takes no pointers; it returns a new
            // descriptor or -1.
            let raw = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `raw` was just created by `epoll_create1` and nothing
            // else owns it; `OwnedFd` closes it once.
            let owned = unsafe { OwnedFd::from_raw_fd(raw) };
            // Tokio panics (rather than failing) on a runtime built without
            // I/O; this must never take the lane down. `with_interest` is
            // deprecated in newer Tokio in favour of an `unsafe` constructor
            // whose contract (the descriptor stays open and the same for the
            // registration's lifetime) `OwnedFd` already guarantees; it is
            // kept for the crate's minimum Tokio version.
            #[allow(deprecated)]
            let registered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                AsyncFd::with_interest(owned, tokio::io::Interest::READABLE)
            }))
            .map_err(|_| io::Error::other("the runtime has no I/O driver"))??;
            Ok(Self {
                fd: Arc::new(registered),
                events: vec![libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS],
                registered: Vec::new(),
                count: 0,
            })
        }

        fn raw(&self) -> RawFd {
            self.fd.get_ref().as_raw_fd()
        }

        /// Register `socket` for table entry (`slot`, `generation`),
        /// level-triggered for input. A socket already registered (a missed
        /// removal) gets the new key.
        pub(crate) fn add(
            &mut self,
            slot: u32,
            generation: u32,
            socket: SocketFd,
        ) -> io::Result<()> {
            let mut event = libc::epoll_event {
                events: libc::EPOLLIN as u32,
                u64: key(slot, generation),
            };
            // SAFETY: `self.raw()` is our open epoll descriptor; `socket` is
            // the caller's open socket, which it keeps open until `remove`;
            // `event` is a valid `epoll_event` for the duration of the call.
            let mut rc =
                unsafe { libc::epoll_ctl(self.raw(), libc::EPOLL_CTL_ADD, socket, &mut event) };
            if rc < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
                // SAFETY: as above.
                rc =
                    unsafe { libc::epoll_ctl(self.raw(), libc::EPOLL_CTL_MOD, socket, &mut event) };
            }
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            let index = slot as usize;
            if self.registered.len() <= index {
                self.registered.resize(index + 1, -1);
            }
            if self.registered[index] < 0 {
                self.count += 1;
            }
            self.registered[index] = socket;
            Ok(())
        }

        /// Remove `slot`'s socket (while the socket is still open); `false`
        /// when it had none.
        pub(crate) fn remove(&mut self, slot: u32) -> bool {
            let Some(registered) = self.registered.get_mut(slot as usize) else {
                return false;
            };
            if *registered < 0 {
                return false;
            }
            let socket = std::mem::replace(registered, -1);
            self.count -= 1;
            // SAFETY: `self.raw()` is our open epoll descriptor and `socket`
            // is still open (the caller removes before it releases the
            // socket); a null event is allowed for `EPOLL_CTL_DEL` (Linux
            // 2.6.9 and later). A failure (the socket was closed already,
            // which unregisters it) leaves nothing to undo.
            unsafe {
                libc::epoll_ctl(
                    self.raw(),
                    libc::EPOLL_CTL_DEL,
                    socket,
                    std::ptr::null_mut(),
                );
            }
            true
        }

        /// Sockets registered.
        pub(crate) fn registered(&self) -> usize {
            self.count
        }

        /// The sockets holding a datagram now, at most `max` (and
        /// [`MAX_EVENTS`]); never blocks. Read them with [`Self::report`].
        pub(crate) fn wait(&mut self, max: usize) -> io::Result<usize> {
            let max = max.clamp(1, self.events.len());
            // SAFETY: `self.raw()` is our open epoll descriptor; `events`
            // holds `self.events.len()` initialised entries and the kernel
            // writes at most `max` of them; a zero timeout never blocks.
            let ready = unsafe {
                libc::epoll_wait(self.raw(), self.events.as_mut_ptr(), max as libc::c_int, 0)
            };
            if ready < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(ready as usize)
        }

        /// The (slot, generation) of report `index` of the last
        /// [`Self::wait`].
        pub(crate) fn report(&self, index: usize) -> (u32, u32) {
            // `epoll_event` is packed on x86-64: copy the field out.
            let data = self.events[index].u64;
            decode(data)
        }

        /// The reactor's view of the epoll descriptor (cloned, so a guard
        /// does not borrow the table that owns this).
        pub(crate) fn readiness(&self) -> Readiness {
            Readiness(Arc::clone(&self.fd))
        }
    }

    /// The epoll descriptor as registered with the reactor.
    pub(crate) struct Readiness(Arc<AsyncFd<OwnedFd>>);

    /// The reactor reported the epoll descriptor readable.
    pub(crate) struct ReadyGuard<'a>(AsyncFdReadyGuard<'a, OwnedFd>);

    impl Readiness {
        /// `Ready` when the reactor saw a datagram arrive since the last
        /// [`ReadyGuard::clear`]; otherwise `cx`'s waker is registered for
        /// the next one. `Ready(None)`: the reactor is gone (shutdown).
        pub(crate) fn poll(&self, cx: &mut Context<'_>) -> Poll<Option<ReadyGuard<'_>>> {
            self.0
                .poll_read_ready(cx)
                .map(|ready| ready.ok().map(ReadyGuard))
        }
    }

    impl ReadyGuard<'_> {
        /// An `epoll_wait` after this readiness was seen returned nothing:
        /// clear it (unless the reactor reported a newer edge since, which
        /// Tokio's tick check keeps).
        pub(crate) fn clear(mut self) {
            self.0.clear_ready();
        }
    }

    /// One `recvfrom` on a socket its lane epoll reported readable,
    /// bypassing Tokio's readiness flag (which may not have seen this
    /// datagram's edge yet). The socket is non-blocking, so a report that
    /// went stale returns `WouldBlock`. Truncates like `try_recv_from`.
    pub(crate) fn recv_from_ready(
        socket: &tokio::net::UdpSocket,
        buffer: &mut [u8],
    ) -> io::Result<(usize, SocketAddr)> {
        // SAFETY: the descriptor belongs to `socket`, which the caller
        // borrows for this whole call, so it stays open; `ManuallyDrop`
        // keeps this temporary handle from closing it. `recv_from` only
        // reads the descriptor.
        let socket = std::mem::ManuallyDrop::new(unsafe {
            std::net::UdpSocket::from_raw_fd(socket.as_raw_fd())
        });
        socket.recv_from(buffer)
    }

    /// A Tokio socket's descriptor.
    pub(crate) fn socket_fd(socket: &tokio::net::UdpSocket) -> SocketFd {
        socket.as_raw_fd()
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    //! Linux only: the switch reads as off elsewhere and an epoll is never
    //! created, so entries keep the stage-3 reads.
    use super::*;

    pub(crate) struct LaneEpoll;

    impl LaneEpoll {
        pub(crate) fn new() -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "lane receive readiness needs Linux epoll",
            ))
        }
        pub(crate) fn add(&mut self, _: u32, _: u32, _: SocketFd) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        pub(crate) fn remove(&mut self, _: u32) -> bool {
            false
        }
        pub(crate) fn registered(&self) -> usize {
            0
        }
        pub(crate) fn wait(&mut self, _: usize) -> io::Result<usize> {
            Ok(0)
        }
        pub(crate) fn report(&self, _: usize) -> (u32, u32) {
            (u32::MAX, u32::MAX)
        }
        pub(crate) fn readiness(&self) -> Readiness {
            Readiness
        }
    }

    pub(crate) struct Readiness;

    pub(crate) struct ReadyGuard<'a>(std::marker::PhantomData<&'a ()>);

    impl Readiness {
        pub(crate) fn poll(&self, _: &mut Context<'_>) -> Poll<Option<ReadyGuard<'_>>> {
            Poll::Pending
        }
    }

    impl ReadyGuard<'_> {
        pub(crate) fn clear(self) {}
    }

    pub(crate) fn recv_from_ready(
        socket: &tokio::net::UdpSocket,
        buffer: &mut [u8],
    ) -> io::Result<(usize, SocketAddr)> {
        socket.try_recv_from(buffer)
    }

    pub(crate) fn socket_fd(_: &tokio::net::UdpSocket) -> SocketFd {
        -1
    }
}

pub(crate) use imp::{recv_from_ready, socket_fd, LaneEpoll};

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn keys_round_trip() {
        for (slot, generation) in [(0, 0), (7, 1), (u32::MAX, u32::MAX), (12, 0xdead_beef)] {
            assert_eq!(decode(key(slot, generation)), (slot, generation));
        }
    }

    /// Level-triggered: a socket is reported while it holds a datagram (one
    /// read per report, never an empty one), not after its registration is
    /// removed, and an empty wait returns nothing.
    #[tokio::test]
    async fn reports_sockets_while_they_hold_data_and_never_after_removal() {
        let mut epoll = LaneEpoll::new().expect("an epoll on Linux");
        let a = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let b = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        epoll.add(3, 9, socket_fd(&a)).unwrap();
        epoll.add(4, 1, socket_fd(&b)).unwrap();
        assert_eq!(epoll.registered(), 2);
        assert_eq!(epoll.wait(MAX_EVENTS).unwrap(), 0, "nothing queued");
        // Loopback datagrams are queued on the receiving socket when the
        // send returns.
        for value in [1u8, 2] {
            peer.send_to(&[value], a.local_addr().unwrap())
                .await
                .unwrap();
        }
        peer.send_to(&[7], b.local_addr().unwrap()).await.unwrap();
        let mut buffer = [0u8; 16];
        let mut got = Vec::new();
        loop {
            let ready = epoll.wait(MAX_EVENTS).unwrap();
            if ready == 0 {
                break;
            }
            let mut reports: Vec<(u32, u32)> =
                (0..ready).map(|index| epoll.report(index)).collect();
            reports.sort_unstable();
            for (slot, generation) in reports {
                let socket = match (slot, generation) {
                    (3, 9) => &a,
                    (4, 1) => &b,
                    other => panic!("unexpected report {other:?}"),
                };
                let (size, _) = recv_from_ready(socket, &mut buffer).expect("a report has data");
                got.push((slot, buffer[..size].to_vec()));
            }
        }
        assert_eq!(got, vec![(3, vec![1]), (4, vec![7]), (3, vec![2])]);
        assert!(epoll.remove(3));
        assert!(!epoll.remove(3), "removed once");
        peer.send_to(&[3], a.local_addr().unwrap()).await.unwrap();
        assert_eq!(
            epoll.wait(MAX_EVENTS).unwrap(),
            0,
            "a removed socket is not reported"
        );
        assert_eq!(epoll.registered(), 1);
        // A re-registration with a new key (a re-graduation) reports the
        // datagram that waited meanwhile.
        epoll.add(3, 10, socket_fd(&a)).unwrap();
        assert_eq!(epoll.wait(MAX_EVENTS).unwrap(), 1);
        assert_eq!(epoll.report(0), (3, 10));
    }

    /// The reactor wakes the lane for the epoll descriptor; the readiness it
    /// recorded is cleared only through the guard of the poll that saw it,
    /// so a datagram that arrives after the last empty wait wakes the lane
    /// again.
    #[tokio::test]
    async fn the_reactor_wakes_the_lane_again_after_a_clear() {
        struct Count(AtomicUsize);
        impl std::task::Wake for Count {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let waker = std::task::Waker::from(Arc::clone(&count));
        let mut cx = Context::from_waker(&waker);
        let mut epoll = LaneEpoll::new().unwrap();
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        epoll.add(0, 0, socket_fd(&socket)).unwrap();
        let readiness = epoll.readiness();
        // Nothing yet: the waker is registered.
        if let Poll::Ready(Some(guard)) = readiness.poll(&mut cx) {
            // A fresh registration may report once; an empty wait clears it.
            assert_eq!(epoll.wait(MAX_EVENTS).unwrap(), 0);
            guard.clear();
            assert!(readiness.poll(&mut cx).is_pending());
        }
        let woken = count.0.load(Ordering::SeqCst);
        for round in 0..3u8 {
            peer.send_to(&[round], socket.local_addr().unwrap())
                .await
                .unwrap();
            // The reactor turns while this test sleeps; bounded generously
            // so a starved machine still passes.
            for _ in 0..10_000 {
                if count.0.load(Ordering::SeqCst) > woken + usize::from(round) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            assert_eq!(
                count.0.load(Ordering::SeqCst),
                woken + usize::from(round) + 1,
                "round {round}: the arrival woke the lane"
            );
            let Poll::Ready(Some(guard)) = readiness.poll(&mut cx) else {
                panic!("round {round}: readable after the wake");
            };
            assert_eq!(epoll.wait(MAX_EVENTS).unwrap(), 1);
            let mut buffer = [0u8; 4];
            assert_eq!(recv_from_ready(&socket, &mut buffer).unwrap().0, 1);
            assert_eq!(epoll.wait(MAX_EVENTS).unwrap(), 0, "drained");
            guard.clear();
            assert!(readiness.poll(&mut cx).is_pending(), "re-registered");
        }
    }
}
