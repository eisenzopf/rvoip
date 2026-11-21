//! The RTP receive loop's call state and its per-datagram step, out of the
//! loop's `async` block.
//!
//! The receive loop of `UdpRtpTransport::start_receiver` used to keep all of
//! this in locals of its future: the inline ingress, the owner SRTP context,
//! the hot call state entry, the symmetric-RTP learner, the receive buffer
//! and the drop counters. [`RxCore`] holds them instead, and
//! [`RxCore::on_datagram`] is the loop body for one received datagram, moved
//! here unchanged (`continue` became `return`). The async loop calls it, and
//! so does a receive loop that graduated into its lane's table
//! (`super::callback_rx`), so both receive drivers share every per-packet
//! step by construction: mux classification, STUN/DTLS/RTCP, SRTP in place,
//! the hot-state gates, the symmetric-RTP latch, DTMF and the inline
//! ingress.
//!
//! `on_datagram` is always inlined into each driver (the async loop runs it
//! inside its own `poll`, as before), it reads the configuration through
//! references where each field is used instead of copying fields at entry,
//! and the loop's future holds this state once ([`RxCore::unplaced`], placed
//! on the first poll by [`RxCore::place`]).

use super::*;

/// What the receive loop is given when it starts (cloned from the transport
/// by `start_receiver`, or built by a test).
pub(in crate::transport::udp) struct RxConfig {
    pub(in crate::transport::udp) event_tx: broadcast::Sender<RtpEvent>,
    pub(in crate::transport::udp) active_state: Arc<AtomicBool>,
    pub(in crate::transport::udp) srtp_recv: Arc<SrtpRecvShared>,
    pub(in crate::transport::udp) secure_media_required: Arc<AtomicBool>,
    pub(in crate::transport::udp) dtmf_seen: Arc<DashMap<(SocketAddr, u32, u32), Instant>>,
    pub(in crate::transport::udp) symmetric_rtp_policy: SymmetricRtpPolicy,
    pub(in crate::transport::udp) symmetric_rtp_counters: Arc<SymmetricRtpCounters>,
    pub(in crate::transport::udp) symmetric_rtp_latched: Arc<AtomicBool>,
    pub(in crate::transport::udp) symmetric_rtp_generation: Arc<AtomicU64>,
    pub(in crate::transport::udp) remote_rtp_addr: Arc<ArcSwapOption<SocketAddr>>,
    pub(in crate::transport::udp) remote_rtcp_addr: Arc<ArcSwapOption<SocketAddr>>,
    pub(in crate::transport::udp) rtcp_mux: Arc<AtomicBool>,
    pub(in crate::transport::udp) latched_rtp_ssrc: Arc<AtomicU64>,
    /// Hot call state: the doorbell gates the per-datagram control loads
    /// (`None` when hot state is off).
    pub(in crate::transport::udp) hot_doorbell: Option<Arc<HotDoorbell>>,
    #[cfg(feature = "dtls-webrtc")]
    pub(in crate::transport::udp) dtls_tx: Arc<parking_lot::Mutex<Option<mpsc::Sender<Bytes>>>>,
    pub(in crate::transport::udp) srtp_diagnostics: bool,
    pub(in crate::transport::udp) rtp_diagnostics: bool,
    pub(in crate::transport::udp) local_rtp_addr: Option<SocketAddr>,
    pub(in crate::transport::udp) recv_buffer_size: usize,
    pub(in crate::transport::udp) inline_slot: Arc<HandoffSlot<Box<dyn InlineRtpIngress>>>,
    /// Executor v2: the loop runs inside its lane's poller.
    pub(in crate::transport::udp) on_lane: bool,
    /// The loop owns the inbound SRTP context (it is that context's only
    /// datagram user).
    pub(in crate::transport::udp) owner_srtp: bool,
}

/// The receive loop's call state (see the module docs).
pub(in crate::transport::udp) struct RxCore {
    pub(in crate::transport::udp) cfg: RxConfig,
    pub(in crate::transport::udp) first_inbound_rtp_logged: bool,
    pub(in crate::transport::udp) srtp_unprotect_failures: u64,
    pub(in crate::transport::udp) non_rtp_drop_count: u64,
    pub(in crate::transport::udp) malformed_rtp_drop_count: u64,
    pub(in crate::transport::udp) symmetric_rtp: SymmetricRtpLearner,
    pub(in crate::transport::udp) observed_symmetric_generation: u64,
    /// One receive buffer serves the whole loop. The inline path
    /// authenticates and decrypts RTP in place in it; on the v1 path every
    /// event copies the bytes it keeps, and each datagram reads only the
    /// `size` bytes the receive returned.
    pub(in crate::transport::udp) recv_storage: Vec<u8>,
    /// Executor v2 (None under v1): see `InlineRx`.
    pub(in crate::transport::udp) inline: Option<InlineRx>,
    /// The inbound SRTP context this loop holds (`None` under v1 or without
    /// `owner_srtp`). Dropped with the loop's state, which gives the context
    /// back to the transport.
    pub(in crate::transport::udp) owner_rx: Option<OwnerSrtpRx>,
    /// Hot call state (`None` when off): the per-check doorbell snapshots
    /// and the symmetric-RTP learner, in the lane's slab when this loop runs
    /// on a dedicated current-thread lane.
    pub(in crate::transport::udp) rx_hot: Option<RxHot>,
}

impl RxCore {
    /// The loop's state before its first poll: only the configuration and
    /// what needs no thread (the receive future holds this one copy and
    /// places it on its first poll).
    pub(in crate::transport::udp) fn unplaced(cfg: RxConfig) -> Self {
        let symmetric_rtp = SymmetricRtpLearner::new(cfg.symmetric_rtp_policy);
        Self {
            cfg,
            first_inbound_rtp_logged: false,
            srtp_unprotect_failures: 0,
            non_rtp_drop_count: 0,
            malformed_rtp_drop_count: 0,
            symmetric_rtp,
            observed_symmetric_generation: 0,
            recv_storage: Vec::new(),
            inline: None,
            owner_rx: None,
            rx_hot: None,
        }
    }

    /// The loop's first poll, on the thread that polls it, as the loop's
    /// locals were built: the receive buffer is allocated here, the owner
    /// SRTP context taken and the hot call state entry placed in this lane's
    /// slab.
    pub(in crate::transport::udp) fn place(&mut self) {
        self.observed_symmetric_generation =
            self.cfg.symmetric_rtp_generation.load(Ordering::Acquire);
        self.owner_rx = self
            .cfg
            .owner_srtp
            .then(|| OwnerSrtpRx::new(self.cfg.srtp_recv.clone()));
        self.rx_hot = self
            .cfg
            .hot_doorbell
            .as_ref()
            .map(|_| RxHot::new(self.cfg.symmetric_rtp_policy));
        self.recv_storage = vec![0u8; self.cfg.recv_buffer_size];
    }

    /// The top of the loop: whether the transport stopped (hot call state:
    /// the `active` flag is loaded only when the doorbell rang since the
    /// last check).
    #[inline(always)]
    pub(in crate::transport::udp) fn stopped(&mut self) -> bool {
        let check_active = match (self.rx_hot.as_mut(), self.cfg.hot_doorbell.as_deref()) {
            (Some(hot), Some(doorbell)) => hot.gate_top(doorbell.read()),
            _ => true,
        };
        check_active && !self.cfg.active_state.load(Ordering::Acquire)
    }

    /// The loop owns everything a lane-table entry needs: the session's
    /// inline ingress was adopted (at the first datagram it could be given).
    #[inline]
    pub(in crate::transport::udp) fn graduation_ready(&self) -> bool {
        self.inline.is_some()
    }

    /// A receive error: logged and published, as the loop always did (the
    /// loop then pauses 10 ms before it receives again).
    pub(in crate::transport::udp) fn on_error(&self, e: std::io::Error) {
        error!("Error receiving packet: {}", e);

        // Send error event
        let err_event = RtpEvent::Error(Error::Transport(format!("Socket error: {}", e)));
        if self.cfg.event_tx.receiver_count() > 0 {
            let _ = self.cfg.event_tx.send(err_event);
        }
    }

    /// One datagram of `size` bytes from `addr`, received into
    /// `recv_storage`: the receive loop's body, unchanged.
    ///
    /// Always inlined, so the async loop runs this body inside its own
    /// `poll`, and every configuration field is read where it is used,
    /// through a reference: the cold ones (`local_rtp_addr`, the
    /// diagnostics switches) are touched only on the paths that use them.
    #[inline(always)]
    pub(in crate::transport::udp) fn on_datagram(&mut self, size: usize, addr: SocketAddr) {
        let RxConfig {
            event_tx,
            srtp_recv,
            secure_media_required,
            dtmf_seen,
            symmetric_rtp_counters,
            symmetric_rtp_latched,
            symmetric_rtp_generation,
            remote_rtp_addr,
            remote_rtcp_addr,
            rtcp_mux,
            latched_rtp_ssrc,
            hot_doorbell,
            #[cfg(feature = "dtls-webrtc")]
            dtls_tx,
            srtp_diagnostics,
            rtp_diagnostics,
            local_rtp_addr,
            inline_slot,
            on_lane,
            symmetric_rtp_policy,
            ..
        } = &self.cfg;
        let first_inbound_rtp_logged = &mut self.first_inbound_rtp_logged;
        let srtp_unprotect_failures = &mut self.srtp_unprotect_failures;
        let non_rtp_drop_count = &mut self.non_rtp_drop_count;
        let malformed_rtp_drop_count = &mut self.malformed_rtp_drop_count;
        let symmetric_rtp = &mut self.symmetric_rtp;
        let observed_symmetric_generation = &mut self.observed_symmetric_generation;
        let inline = &mut self.inline;
        let owner_rx = &mut self.owner_rx;
        let rx_hot = &mut self.rx_hot;
        let buffer: &mut [u8] = &mut self.recv_storage;

        if *on_lane {
            crate::lane_poller::note_lane_datagram();
        }
        // Hot call state: one doorbell read per datagram, after it was
        // received; each gated check below compares it with its own
        // snapshot.
        let bell = hot_doorbell.as_deref().map_or(0, HotDoorbell::read);
        trace!("UDP recv_from returned {} bytes from {}", size, addr);

        let packet_class = classify_rtp_mux_packet(&buffer[..size]);
        if packet_class == RtpMuxPacketClass::Stun {
            // ICE connectivity checks share the media port by design.
            // Forward them to whoever subscribed (the ICE agent's pump);
            // with no subscriber this send fails silently, which is exactly
            // the old drop behavior.
            let Some(local) = *local_rtp_addr else {
                return;
            };
            let _ = event_tx.send(RtpEvent::StunPacket {
                local,
                source: addr,
                payload: Bytes::copy_from_slice(&buffer[..size]),
            });
            return;
        }

        #[cfg(feature = "dtls-webrtc")]
        if packet_class == RtpMuxPacketClass::Dtls {
            let forwarded = dtls_tx
                .lock()
                .as_ref()
                .map(|tx| tx.try_send(Bytes::copy_from_slice(&buffer[..size])).is_ok())
                .unwrap_or(false);
            if !forwarded {
                // No handshake in flight (or its receiver is backed up) to
                // hand this to; nothing else can use it either.
                *non_rtp_drop_count = non_rtp_drop_count.saturating_add(1);
                log_dropped_non_rtp_packet(
                    *rtp_diagnostics,
                    *non_rtp_drop_count,
                    *local_rtp_addr,
                    addr,
                    size,
                    packet_class,
                    &buffer[..size],
                );
            }
            return;
        }

        if !packet_class.is_media() {
            *non_rtp_drop_count = non_rtp_drop_count.saturating_add(1);
            log_dropped_non_rtp_packet(
                *rtp_diagnostics,
                *non_rtp_drop_count,
                *local_rtp_addr,
                addr,
                size,
                packet_class,
                &buffer[..size],
            );
            return;
        }

        // Check if it's RTCP according to RFC 5761
        if packet_class == RtpMuxPacketClass::Rtcp {
            // Once an RTP tuple is latched, multiplexed RTCP must arrive on
            // that tuple too. A new tuple earns trust through RTP probation,
            // never through a single unauthenticated RTCP packet.
            if rtcp_mux.load(Ordering::Acquire)
                && symmetric_rtp_policy.enabled
                && symmetric_rtp_latched.load(Ordering::Acquire)
                && remote_rtp_addr
                    .load()
                    .as_deref()
                    .is_some_and(|expected| *expected != addr)
            {
                symmetric_rtp_counters
                    .rejected_packets
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
            debug!("Received RTCP packet, type: {}", buffer[1] & 0x7F);
            let rtcp_data = if secure_media_required.load(Ordering::Acquire) {
                let mut recv = RecvContext::open(
                    owner_rx,
                    srtp_recv,
                    rx_hot.as_mut().is_none_or(|hot| hot.gate_srtp(bell)),
                );
                let Some(context) = recv.get() else {
                    trace!(
                        "SRTCP is required but no receive context is installed; dropping packet"
                    );
                    return;
                };
                match context.unprotect_rtcp(&buffer[..size]) {
                    Ok(plaintext) => plaintext,
                    Err(error) => {
                        trace!("SRTCP unprotect failed; dropping packet: {error}");
                        return;
                    }
                }
            } else {
                Bytes::copy_from_slice(&buffer[..size])
            };
            // Executor v2: the session's inline ingress may process RTCP
            // here (`owner_rtcp`).
            adopt_inline_ingress(inline, inline_slot);
            if let Some(rx) = inline.as_mut() {
                if rx.ingress.on_rtcp(&rtcp_data, addr, Instant::now()) {
                    return;
                }
            }
            let event = RtpEvent::RtcpReceived {
                data: rtcp_data,
                source: addr,
            };

            // Only log errors if there are receivers
            if event_tx.receiver_count() > 0 {
                if let Err(e) = event_tx.send(event) {
                    warn!("Failed to send RTCP event: {}", e);
                }
            } else {
                // Still send the event but ignore errors if no one is listening
                let _ = event_tx.send(event);
            }
            return;
        }

        // Executor v2: adopt the session's inline ingress before the first
        // RTP packet it can own.
        adopt_inline_ingress(inline, inline_slot);
        if let Some(rx) = inline.as_mut() {
            // Authenticate and decrypt in place in the receive buffer: no
            // copy and no per-packet allocation.
            let data = &mut buffer[..size];
            let parsed = {
                let mut srtp_guard = RecvContext::open(
                    owner_rx,
                    srtp_recv,
                    rx_hot.as_mut().is_none_or(|hot| hot.gate_srtp(bell)),
                );
                if *srtp_diagnostics && !*first_inbound_rtp_logged {
                    info!(
                        "SRTP_DIAG inbound_rtp_first local={:?} source={} size={} srtp_context={}",
                        local_rtp_addr,
                        addr,
                        size,
                        srtp_guard.get().is_some()
                    );
                    *first_inbound_rtp_logged = true;
                }
                if let Some(ctx) = srtp_guard.get() {
                    match ctx.unprotect_in_place(data) {
                        Ok(bounds) => Ok(bounds),
                        Err(_) => {
                            *srtp_unprotect_failures += 1;
                            if *srtp_diagnostics
                                && (*srtp_unprotect_failures <= 5
                                    || *srtp_unprotect_failures % 50 == 0)
                            {
                                info!(
                                    "SRTP_DIAG unprotect_failed local={:?} source={} size={} failures={}",
                                    local_rtp_addr,
                                    addr,
                                    size,
                                    *srtp_unprotect_failures
                                );
                            }
                            trace!("SRTP unprotect failed; dropping packet");
                            return;
                        }
                    }
                } else if secure_media_required.load(Ordering::Acquire) {
                    trace!("SRTP is required but no receive context is installed; dropping packet");
                    return;
                } else {
                    RtpPacket::parse_bounds(data)
                }
            };
            let bounds = match parsed {
                Ok(bounds) => bounds,
                Err(e) => {
                    *malformed_rtp_drop_count = malformed_rtp_drop_count.saturating_add(1);
                    log_malformed_rtp_packet(
                        *rtp_diagnostics,
                        *malformed_rtp_drop_count,
                        *local_rtp_addr,
                        addr,
                        size,
                        &e,
                        Some(&*data),
                    );
                    return;
                }
            };
            // One pass reading for the datagrams of a lane pass (bounded
            // staleness; see `lane_clock`), else a clock read.
            let now = crate::media_clock::now_std();
            let verdict = observe_symmetric_rtp(
                rx_hot.as_mut(),
                bell,
                symmetric_rtp_generation,
                symmetric_rtp,
                observed_symmetric_generation,
                addr,
                bounds.header.ssrc,
                bounds.header.sequence_number,
                now,
            );
            if !admit_symmetric_rtp(
                verdict,
                addr,
                bounds.header.ssrc,
                rtcp_mux,
                symmetric_rtp_latched,
                latched_rtp_ssrc,
                remote_rtp_addr,
                remote_rtcp_addr,
                symmetric_rtp_counters,
                hot_doorbell.as_deref(),
            ) {
                return;
            }
            let payload = &data[bounds.payload.clone()];
            if bounds.header.payload_type == 101 && payload.len() >= 4 {
                // DTMF stays on the broadcast path.
                if let Some(dtmf) = rfc4733_event(
                    payload,
                    bounds.header.timestamp,
                    bounds.header.ssrc,
                    addr,
                    dtmf_seen,
                ) {
                    publish_dtmf(event_tx, dtmf);
                }
                return;
            }
            let packet = InlineRtpPacket {
                payload_type: bounds.header.payload_type,
                sequence_number: bounds.header.sequence_number,
                timestamp: bounds.header.timestamp,
                marker: bounds.header.marker,
                ssrc: bounds.header.ssrc,
                padding_size: bounds.padding_size,
                payload,
                source: addr,
            };
            rx.ingress.on_rtp(&packet, now);
            rx.rearm(now);
            return;
        }
        // SRTP unprotect (RFC 3711 §3.4) when an inbound SrtpContext is
        // configured. Auth failures MUST be silently dropped — no event, no
        // warn-level log — to avoid leaking timing or distinguishing failure
        // modes to a network attacker.
        let mut srtp_guard = RecvContext::open(
            owner_rx,
            srtp_recv,
            rx_hot.as_mut().is_none_or(|hot| hot.gate_srtp(bell)),
        );
        if *srtp_diagnostics && !*first_inbound_rtp_logged {
            info!(
                "SRTP_DIAG inbound_rtp_first local={:?} source={} size={} srtp_context={}",
                local_rtp_addr,
                addr,
                size,
                srtp_guard.get().is_some()
            );
            *first_inbound_rtp_logged = true;
        }
        let parse_result: Result<RtpPacket> = if let Some(ctx) = srtp_guard.get() {
            match ctx.unprotect(&buffer[0..size]) {
                Ok(packet) => Ok(packet),
                Err(_) => {
                    *srtp_unprotect_failures += 1;
                    if *srtp_diagnostics
                        && (*srtp_unprotect_failures <= 5 || *srtp_unprotect_failures % 50 == 0)
                    {
                        info!(
                            "SRTP_DIAG unprotect_failed local={:?} source={} size={} failures={}",
                            local_rtp_addr, addr, size, *srtp_unprotect_failures
                        );
                    }
                    trace!("SRTP unprotect failed; dropping packet");
                    drop(srtp_guard);
                    return;
                }
            }
        } else if secure_media_required.load(Ordering::Acquire) {
            trace!("SRTP is required but no receive context is installed; dropping packet");
            drop(srtp_guard);
            return;
        } else {
            RtpPacket::parse(&buffer[..size])
        };
        drop(srtp_guard);
        match parse_result {
            Ok(packet) => {
                let verdict = observe_symmetric_rtp(
                    rx_hot.as_mut(),
                    bell,
                    symmetric_rtp_generation,
                    symmetric_rtp,
                    observed_symmetric_generation,
                    addr,
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    Instant::now(),
                );
                if !admit_symmetric_rtp(
                    verdict,
                    addr,
                    packet.header.ssrc,
                    rtcp_mux,
                    symmetric_rtp_latched,
                    latched_rtp_ssrc,
                    remote_rtp_addr,
                    remote_rtcp_addr,
                    symmetric_rtp_counters,
                    hot_doorbell.as_deref(),
                ) {
                    return;
                }

                // Log packet reception at transport level (debug only)
                debug!(
                    "Transport received packet with SSRC={:08x}, seq={}, ts={}",
                    packet.header.ssrc, packet.header.sequence_number, packet.header.timestamp
                );

                // Debug: Log SSRC demultiplexing info
                debug!(
                    "SSRC demultiplexing: Forwarding packet with SSRC={:08x}, seq={}, payload size={} bytes",
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    packet.payload.len()
                );

                // RFC 4733: PT 101 (by default) is `telephone-event` — DTMF
                // tones carried as RTP events rather than audio samples.
                // Decode the 4-byte body inline and emit a typed `DtmfEvent`
                // instead of a generic `MediaReceived`, so the media layer
                // doesn't have to re-parse and doesn't try to feed the bytes
                // to a PCMU/PCMA/Opus decoder. Oversized payloads are
                // tolerated per RFC 4733's forward-compat clause (read only
                // first 4 bytes).
                if packet.header.payload_type == 101 && packet.payload.len() >= 4 {
                    if let Some(dtmf) = rfc4733_event(
                        &packet.payload,
                        packet.header.timestamp,
                        packet.header.ssrc,
                        addr,
                        dtmf_seen,
                    ) {
                        publish_dtmf(event_tx, dtmf);
                    }
                    return;
                }

                // Create RTP event
                let event = RtpEvent::MediaReceived {
                    payload_type: packet.header.payload_type,
                    sequence_number: packet.header.sequence_number,
                    timestamp: packet.header.timestamp,
                    marker: packet.header.marker,
                    payload: packet.payload.clone(), // Use the parsed payload
                    padding_size: packet.padding_size,
                    source: addr,
                    ssrc: packet.header.ssrc, // Include the SSRC from the parsed packet
                };

                // Only log errors if there are receivers
                if event_tx.receiver_count() > 0 {
                    if let Err(e) = event_tx.send(event) {
                        warn!("Failed to send RTP event: {}", e);
                    }
                } else {
                    // Still send the event but ignore errors if no one is listening
                    let _ = event_tx.send(event);
                }
            }
            Err(e) => {
                *malformed_rtp_drop_count = malformed_rtp_drop_count.saturating_add(1);
                log_malformed_rtp_packet(
                    *rtp_diagnostics,
                    *malformed_rtp_drop_count,
                    *local_rtp_addr,
                    addr,
                    size,
                    &e,
                    Some(&buffer[..size]),
                );
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the drop counters of the last [`RxCore`] dropped on this
    /// thread (SRTP failures, malformed RTP, non-RTP drops), so a dual run
    /// can compare them after either driver ended the loop.
    static LAST_DROPPED: std::cell::Cell<Option<[u64; 3]>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
impl Drop for RxCore {
    fn drop(&mut self) {
        let counts = [
            self.srtp_unprotect_failures,
            self.malformed_rtp_drop_count,
            self.non_rtp_drop_count,
        ];
        let _ = LAST_DROPPED.try_with(|last| last.set(Some(counts)));
    }
}

#[cfg(test)]
pub(in crate::transport::udp) fn take_last_dropped_counts() -> Option<[u64; 3]> {
    LAST_DROPPED.with(|last| last.take())
}
