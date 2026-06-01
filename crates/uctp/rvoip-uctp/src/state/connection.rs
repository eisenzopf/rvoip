//! `UctpConnectionState` + transitions + per-Connection
//! `stream_local_id` allocator (§3.6 of `UCTP_IMPLEMENTATION_PLAN.md`).

use std::collections::HashMap;

use crate::errors::UctpError;
use crate::ids::StreamId;

/// One Stream that survived `negotiate_streams` during `connection.offer`
/// handling. Stored on the [`ConnectionMachine`] so the coordinator can
/// emit `stream.opened` envelopes when the Connection reaches
/// `connection.ready`. Per CONVERSATION_PROTOCOL.md §7.4: `stream_local_id`
/// is assigned at `connection.ready` and announced via `stream.opened`,
/// so the chosen-codec / direction / kind from negotiation must be held
/// here until the ready transition.
#[derive(Clone, Debug)]
pub struct AcceptedStream {
    /// Wire-level Stream id (the offerer-chosen `id` from the
    /// `streams_offered[*]` entry — opaque to the server).
    pub strm_id: String,
    pub kind: String,
    pub direction: String,
    /// Codec chosen by `negotiate_streams`. `None` only if the offer
    /// reached the machine without a codec — should not happen on the
    /// 488-then-no-machine code path.
    pub chosen_codec: Option<String>,
    /// Publishing Participant — taken from `connection.offer.by_participant`.
    /// Carried so `stream.opened` can announce a complete publisher
    /// identity to the `SubscriptionHandler` for `from_participant`
    /// resolution.
    pub participant: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UctpConnectionState {
    /// `connection.offer` sent, no `connection.answer` yet.
    Negotiating,
    /// `connection.ready` fired; media is flowing.
    Connected,
    /// Mid-call hold.
    OnHold,
    /// `connection.end` issued.
    Ending,
    /// Terminal.
    Ended,
}

#[derive(Clone, Copy, Debug)]
pub enum ConnectionInput {
    OfferSent,
    AnswerReceived,
    AnswerSent,
    ReadyReceived,
    ReadySent,
    HoldRequested,
    ResumeRequested,
    EndSent,
    EndReceived,
}

#[derive(Debug)]
pub struct ConnectionMachine {
    state: UctpConnectionState,
    allocator: StreamLocalIdAllocator,
    streams: HashMap<u16, StreamId>,
    /// Streams that survived negotiation at `connection.offer` time and
    /// are awaiting `stream.opened` emission on `connection.ready`.
    /// Consumed by `take_pending_streams()` so the same set is never
    /// announced twice.
    pending_streams: Vec<AcceptedStream>,
    /// `stream.opened` already emitted — used by the coordinator to be
    /// idempotent on repeated `connection.ready` envelopes (the spec
    /// §7.3 allows duplicate ready as a no-op).
    streams_announced: bool,
    /// `uctp.connection.lifetime` span (plan §3.9 / C5). Opened by the
    /// coordinator at `connection.offer` time via
    /// [`Self::new_negotiating_with_span`] and re-entered by every
    /// subsequent handler that touches this Connection
    /// (`handle_connection_answer`, `handle_connection_ready`,
    /// `handle_end`) so per-Connection tracing context spans the whole
    /// offer → ready → end lifecycle. Spans dropped here close
    /// automatically when the last clone goes out of scope (which
    /// happens when the Connection is removed from the coordinator's
    /// `connections` map at end-of-call). Defaults to
    /// [`tracing::Span::none`] for the no-tracing constructor so test
    /// code stays unchanged.
    lifetime_span: tracing::Span,
}

impl ConnectionMachine {
    pub fn new_negotiating() -> Self {
        Self::new_negotiating_with_span(tracing::Span::none())
    }

    /// Construct a `ConnectionMachine` with an explicit
    /// `uctp.connection.lifetime` span. Production callers (the
    /// coordinator's `handle_connection_offer`) build the span with
    /// `connid` / `sid` / `transport` fields; tests can use the
    /// no-span [`Self::new_negotiating`] constructor.
    pub fn new_negotiating_with_span(lifetime_span: tracing::Span) -> Self {
        Self {
            state: UctpConnectionState::Negotiating,
            allocator: StreamLocalIdAllocator::new(),
            streams: HashMap::new(),
            pending_streams: Vec::new(),
            streams_announced: false,
            lifetime_span,
        }
    }

    /// Clone of the per-Connection lifetime span. The coordinator
    /// re-enters this on every handler that operates on the Connection
    /// so per-envelope spans nest cleanly under it.
    pub fn lifetime_span(&self) -> tracing::Span {
        self.lifetime_span.clone()
    }

    pub fn state(&self) -> UctpConnectionState {
        self.state
    }

    /// Record the streams that passed negotiation. Called from the
    /// coordinator's `handle_connection_offer` after `negotiate_streams`
    /// returns `Ok`. Drains anything previously set (a fresh offer
    /// supersedes prior partial state).
    pub fn set_pending_streams(&mut self, streams: Vec<AcceptedStream>) {
        self.pending_streams = streams;
        self.streams_announced = false;
    }

    /// Drain the pending streams set and allocate a `stream_local_id`
    /// for each, returning `(AcceptedStream, allocated_id)` pairs.
    /// Idempotent — second call after the first allocation returns an
    /// empty Vec (so duplicate `connection.ready` envelopes don't
    /// re-announce streams).
    pub fn take_pending_streams(&mut self) -> Result<Vec<(AcceptedStream, u16)>, UctpError> {
        if self.streams_announced {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(self.pending_streams.len());
        for stream in std::mem::take(&mut self.pending_streams) {
            let local_id = self.allocator.next()?;
            self.streams
                .insert(local_id, StreamId::from_string(stream.strm_id.clone()));
            out.push((stream, local_id));
        }
        self.streams_announced = true;
        Ok(out)
    }

    /// Allocate a new stream-local id for a given Stream ID, and record
    /// the mapping so inbound datagrams can resolve back.
    pub fn open_stream(&mut self, strm_id: StreamId) -> Result<u16, UctpError> {
        let local_id = self.allocator.next()?;
        self.streams.insert(local_id, strm_id);
        Ok(local_id)
    }

    /// Look up which `StreamId` an inbound datagram's
    /// `stream_local_id` belongs to.
    pub fn resolve_stream(&self, local_id: u16) -> Option<&StreamId> {
        self.streams.get(&local_id)
    }

    /// Apply an input. Returns the new state or an illegal-transition error.
    pub fn apply(&mut self, input: ConnectionInput) -> Result<UctpConnectionState, UctpError> {
        let next = match (self.state, input) {
            (UctpConnectionState::Negotiating, ConnectionInput::AnswerReceived)
            | (UctpConnectionState::Negotiating, ConnectionInput::AnswerSent)
            | (UctpConnectionState::Negotiating, ConnectionInput::ReadyReceived)
            | (UctpConnectionState::Negotiating, ConnectionInput::ReadySent) => {
                UctpConnectionState::Connected
            }

            (UctpConnectionState::Connected, ConnectionInput::HoldRequested) => {
                UctpConnectionState::OnHold
            }

            (UctpConnectionState::OnHold, ConnectionInput::ResumeRequested) => {
                UctpConnectionState::Connected
            }

            (UctpConnectionState::Negotiating, ConnectionInput::EndSent)
            | (UctpConnectionState::Negotiating, ConnectionInput::EndReceived)
            | (UctpConnectionState::Connected, ConnectionInput::EndSent)
            | (UctpConnectionState::Connected, ConnectionInput::EndReceived)
            | (UctpConnectionState::OnHold, ConnectionInput::EndSent)
            | (UctpConnectionState::OnHold, ConnectionInput::EndReceived) => {
                UctpConnectionState::Ending
            }

            (UctpConnectionState::Ending, ConnectionInput::EndReceived) => {
                UctpConnectionState::Ended
            }

            (state, input) => {
                return Err(UctpError::IllegalTransition {
                    state: state_name(state),
                    event: input_name(input),
                });
            }
        };
        self.state = next;
        Ok(next)
    }
}

/// Monotonically-increasing per-Connection 16-bit handle allocator.
/// Returns [`UctpError::StreamHandleExhausted`] at u16::MAX wrap.
#[derive(Debug)]
struct StreamLocalIdAllocator {
    next: u16,
}

impl StreamLocalIdAllocator {
    fn new() -> Self {
        Self { next: 1 }
    }

    fn next(&mut self) -> Result<u16, UctpError> {
        let id = self.next;
        if id == 0 {
            return Err(UctpError::StreamHandleExhausted);
        }
        // u16::MAX → 0 wrap signals exhaustion on next call.
        self.next = self.next.wrapping_add(1);
        Ok(id)
    }
}

fn state_name(s: UctpConnectionState) -> &'static str {
    match s {
        UctpConnectionState::Negotiating => "Negotiating",
        UctpConnectionState::Connected => "Connected",
        UctpConnectionState::OnHold => "OnHold",
        UctpConnectionState::Ending => "Ending",
        UctpConnectionState::Ended => "Ended",
    }
}

fn input_name(i: ConnectionInput) -> &'static str {
    match i {
        ConnectionInput::OfferSent => "OfferSent",
        ConnectionInput::AnswerReceived => "AnswerReceived",
        ConnectionInput::AnswerSent => "AnswerSent",
        ConnectionInput::ReadyReceived => "ReadyReceived",
        ConnectionInput::ReadySent => "ReadySent",
        ConnectionInput::HoldRequested => "HoldRequested",
        ConnectionInput::ResumeRequested => "ResumeRequested",
        ConnectionInput::EndSent => "EndSent",
        ConnectionInput::EndReceived => "EndReceived",
    }
}
