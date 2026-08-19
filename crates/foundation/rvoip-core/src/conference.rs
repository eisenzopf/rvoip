//! N-way audio conferencing.
//!
//! [`Orchestrator::bridge_connections`] joins exactly two connections by
//! pumping frames between them. That shape does not extend to three: a
//! conference has to *sum* audio, and every participant needs a different
//! sum — their own voice removed, or they hear themselves echoed a packet
//! late. So this is a mixer rather than a wider bridge.
//!
//! One task per conference owns the mix. On each 20 ms tick it takes what
//! every member has said, sums it once, and hands each member that sum minus
//! their own contribution. Doing the sum once and subtracting is what keeps
//! the work linear in members rather than quadratic.
//!
//! Members may arrive on different codecs and different clock rates — a G.711
//! carrier leg and an Opus browser leg in the same conference is the ordinary
//! case, not the exotic one. Each member therefore decodes into the
//! conference's own rate on the way in and encodes back to their own on the
//! way out, resampling at both boundaries when the rates differ.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

use crate::error::{Result, RvoipError};
use crate::ids::ConnectionId;
use crate::stream::MediaFrame;
use rvoip_core_traits::capability::CodecInfo;
use rvoip_media_core::AudioFrame;

/// Identifies one live conference.
///
/// Shaped like every other id in `rvoip-core-traits` — same prefix
/// convention, same redacted `Debug` so accidental structured-log capture
/// stays metadata-only — but declared here because that crate's `id_type!`
/// macro is private to its own module.
#[derive(Clone, Eq, Hash, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize)]
pub struct ConferenceId(pub String);

impl ConferenceId {
    #[must_use]
    pub fn new() -> Self {
        Self(format!("conf_{}", uuid::Uuid::new_v4().simple()))
    }

    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConferenceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::fmt::Debug for ConferenceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ConferenceId([redacted])")
    }
}

impl Default for ConferenceId {
    fn default() -> Self {
        Self::new()
    }
}

/// How often the mixer produces a frame for every member.
///
/// 20 ms is the packetization every telephony codec here defaults to, so a
/// tick corresponds to one outbound packet per member and no member has to
/// buffer a partial frame.
const MIX_INTERVAL: Duration = Duration::from_millis(20);

/// Bound on a member's undelivered inbound frames.
///
/// Deep enough to ride out scheduling jitter, shallow enough that a stalled
/// member's audio is dropped rather than delivered seconds late — in a live
/// conversation, stale audio is worse than absent audio.
const MEMBER_TAP_CAPACITY: usize = 16;

/// Ceiling on decoded audio held for one member between ticks.
///
/// A member sending faster than real time cannot grow the mixer's memory;
/// past this, their oldest audio is discarded.
const MAX_PENDING_SAMPLES: usize = 8_000;

/// What the mixer needs to move audio to and from one member.
pub(crate) struct ConferenceMember {
    pub(crate) connection_id: ConnectionId,
    pub(crate) inbound: mpsc::Receiver<MediaFrame>,
    pub(crate) outbound: mpsc::Sender<MediaFrame>,
    pub(crate) codec: CodecInfo,
    pub(crate) decoder: Box<dyn rvoip_media_core::codec::audio::common::AudioCodec>,
    pub(crate) encoder: Box<dyn rvoip_media_core::codec::audio::common::AudioCodec>,
    /// Decoded audio at the conference's rate, awaiting the next tick.
    pending: Vec<i16>,
    /// Resamplers, present only when this member's rate differs from the mix.
    to_mix: Option<rvoip_media_core::processing::format::resampler::Resampler>,
    from_mix: Option<rvoip_media_core::processing::format::resampler::Resampler>,
    /// RTP timestamp for this member's outbound stream, advanced per tick so
    /// the receiver sees a continuous timeline rather than a reset one.
    timestamp: u32,
    /// The member's tap on the media graph. Owned here so removing the
    /// member tears the tap down with it, rather than leaving a route
    /// feeding a receiver nobody reads.
    _tap: Option<Box<dyn std::any::Any + Send + Sync>>,
    /// Whether this member's voice enters the mix.
    ///
    /// A member who hears the conference without contributing to it is a
    /// supervisor monitoring a call. Muting at the mixer rather than at the
    /// member's transport means the rest of the conference cannot tell that
    /// anyone is listening, which is the point of monitoring.
    pub(crate) contributes: bool,
}

impl ConferenceMember {
    /// Drain everything this member has said since the last tick.
    ///
    /// A frame that fails to decode is skipped rather than fatal: one bad
    /// packet from one member must not silence the conference.
    fn collect(&mut self) {
        while let Ok(frame) = self.inbound.try_recv() {
            let Ok(decoded) = self.decoder.decode(&frame.payload) else {
                continue;
            };
            let at_mix_rate = match self.to_mix.as_mut() {
                Some(resampler) => match resampler.resample(&decoded.samples) {
                    Ok(samples) => samples,
                    Err(_) => continue,
                },
                None => decoded.samples,
            };
            self.pending.extend_from_slice(&at_mix_rate);
        }
        if self.pending.len() > MAX_PENDING_SAMPLES {
            let excess = self.pending.len() - MAX_PENDING_SAMPLES;
            self.pending.drain(..excess);
        }
    }

    /// Take exactly `samples` of this member's audio, padding with silence.
    ///
    /// Silence for a member who has said nothing is what makes the sum
    /// well-defined every tick regardless of who is talking.
    fn take(&mut self, samples: usize) -> Vec<i16> {
        let available = self.pending.len().min(samples);
        let mut taken: Vec<i16> = self.pending.drain(..available).collect();
        taken.resize(samples, 0);
        taken
    }
}

/// One live conference and the task mixing it.
pub(crate) struct Conference {
    pub(crate) members: Arc<Mutex<HashMap<ConnectionId, ConferenceMember>>>,
    pub(crate) mix_rate_hz: u32,
    pub(crate) task: JoinHandle<()>,
}

impl Conference {
    /// Start a conference mixing at `mix_rate_hz`.
    pub(crate) fn start(mix_rate_hz: u32) -> Self {
        let members: Arc<Mutex<HashMap<ConnectionId, ConferenceMember>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let task_members = Arc::clone(&members);
        let samples_per_tick = (mix_rate_hz / 50).max(1) as usize;
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(MIX_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let mut members = task_members.lock().await;
                if members.is_empty() {
                    continue;
                }
                mix_once(&mut members, samples_per_tick);
            }
        });
        Self {
            members,
            mix_rate_hz,
            task,
        }
    }
}

impl Drop for Conference {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Produce one tick of audio for every member.
///
/// Split out from the task so it can be tested directly: given known input
/// per member, the assertion is that each member's output is the sum of the
/// *others* and never includes themselves.
pub(crate) fn mix_once(
    members: &mut HashMap<ConnectionId, ConferenceMember>,
    samples_per_tick: usize,
) {
    let mut contributions: HashMap<ConnectionId, Vec<i16>> = HashMap::new();
    // Summed in i32: a dozen members at full scale overflow i16 long before
    // they overflow this, so clipping is a decision made once at the end
    // rather than an accident partway through.
    let mut total = vec![0_i32; samples_per_tick];
    for (connection_id, member) in members.iter_mut() {
        member.collect();
        // A silenced member still has their audio drained, so unmuting
        // resumes from live speech rather than replaying a backlog.
        let taken = member.take(samples_per_tick);
        let taken = if member.contributes {
            taken
        } else {
            vec![0_i16; samples_per_tick]
        };
        for (slot, sample) in total.iter_mut().zip(taken.iter()) {
            *slot += i32::from(*sample);
        }
        contributions.insert(connection_id.clone(), taken);
    }

    let mut dead = Vec::new();
    for (connection_id, member) in members.iter_mut() {
        let own = contributions
            .get(connection_id)
            .expect("every member contributed above");
        // Mix-minus: the conference minus this member, so nobody hears
        // themselves returned to them a packet late.
        let mixed: Vec<i16> = total
            .iter()
            .zip(own.iter())
            .map(|(sum, mine)| {
                let without_me = sum - i32::from(*mine);
                // Saturate rather than wrap: a wrapped sum is a loud click,
                // and clipping is what a mixing desk does.
                without_me.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
            })
            .collect();

        let at_member_rate = match member.from_mix.as_mut() {
            Some(resampler) => match resampler.resample(&mixed) {
                Ok(samples) => samples,
                Err(_) => continue,
            },
            None => mixed,
        };
        let outbound_frame = AudioFrame {
            sample_rate: member.codec.clock_rate_hz,
            channels: member.codec.channels,
            duration: MIX_INTERVAL,
            timestamp: member.timestamp,
            samples: at_member_rate,
        };
        let sample_count = outbound_frame.samples.len();
        let Ok(payload) = member.encoder.encode(&outbound_frame) else {
            continue;
        };
        let frame = MediaFrame {
            stream_id: crate::ids::StreamId::from_string(format!(
                "conference:{}",
                member.connection_id
            )),
            kind: crate::stream::StreamKind::Audio,
            payload: bytes::Bytes::from(payload),
            timestamp_rtp: member.timestamp,
            captured_at: chrono::Utc::now(),
            payload_type: member.codec.payload_type,
        };
        member.timestamp = member
            .timestamp
            .wrapping_add(u32::try_from(sample_count).unwrap_or_default());
        // A member whose transport has gone is removed rather than retried:
        // the conference continues for everyone still present.
        if member.outbound.try_send(frame).is_err() && member.outbound.is_closed() {
            dead.push(connection_id.clone());
        }
    }
    for connection_id in dead {
        members.remove(&connection_id);
    }
}

/// The conventional payload type for an encoding name.
///
/// Mirrors the media graph's own payload-type table so a conference and a
/// bridge agree on what a name means. AMR is deliberately absent: it is
/// dynamically assigned, so a name alone cannot identify its payload type
/// and the negotiated number must be supplied.
fn static_payload_type(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "pcmu" => Some(0),
        "pcma" => Some(8),
        "g729" => Some(18),
        "opus" => Some(111),
        _ => None,
    }
}

/// Build a member, wiring its codecs and any rate conversion it needs.
pub(crate) fn build_member(
    connection_id: ConnectionId,
    inbound: mpsc::Receiver<MediaFrame>,
    outbound: mpsc::Sender<MediaFrame>,
    codec: CodecInfo,
    mix_rate_hz: u32,
    tap: Option<Box<dyn std::any::Any + Send + Sync>>,
) -> Result<ConferenceMember> {
    use rvoip_media_core::codec::factory::CodecFactory;
    use rvoip_media_core::processing::format::resampler::Resampler;

    // A stream often reports its encoding name without the payload type,
    // because for a statically assigned codec the name *is* the identity.
    // Fall back to the same name/number table the media graph uses, so a
    // member is refused only when neither actually names a codec.
    let payload_type = codec
        .payload_type
        .or_else(|| static_payload_type(&codec.name))
        .ok_or(RvoipError::AdmissionRejected(
            "conference member codec has neither a payload type nor a known name",
        ))?;
    let make = |what: &'static str| {
        CodecFactory::create_negotiated_codec(
            payload_type,
            &codec.name,
            Some(codec.clock_rate_hz),
            Some(codec.channels.into()),
            codec.fmtp.as_deref(),
        )
        .map_err(|_| {
            let _ = what;
            RvoipError::AdmissionRejected("conference member codec is not supported")
        })
    };
    // Independent instances: a codec carries encoder and decoder state, and
    // sharing one across both directions corrupts both.
    let decoder = make("decoder")?;
    let encoder = make("encoder")?;

    let resampler = |from: u32, to: u32| -> Result<Option<Resampler>> {
        if from == to {
            return Ok(None);
        }
        Resampler::new(from, to, 5).map(Some).map_err(|_| {
            RvoipError::AdmissionRejected("conference member rate cannot be converted")
        })
    };
    let to_mix = resampler(codec.clock_rate_hz, mix_rate_hz)?;
    let from_mix = resampler(mix_rate_hz, codec.clock_rate_hz)?;
    Ok(ConferenceMember {
        connection_id,
        inbound,
        outbound,
        to_mix,
        from_mix,
        codec,
        decoder,
        encoder,
        pending: Vec::new(),
        timestamp: 0,
        _tap: tap,
        contributes: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcmu_codec() -> CodecInfo {
        CodecInfo {
            name: "pcmu".into(),
            clock_rate_hz: 8_000,
            channels: 1,
            fmtp: None,
            payload_type: Some(0),
        }
    }

    /// Build a member whose inbound already holds one frame of `sample`.
    fn seeded_member(
        id: &str,
        sample: i16,
        samples: usize,
    ) -> (ConferenceMember, mpsc::Receiver<MediaFrame>) {
        use rvoip_media_core::codec::factory::CodecFactory;

        let (inbound_tx, inbound_rx) = mpsc::channel(8);
        let (outbound_tx, outbound_rx) = mpsc::channel(8);
        let mut member = build_member(
            ConnectionId::from_string(id),
            inbound_rx,
            outbound_tx,
            pcmu_codec(),
            8_000,
            None,
        )
        .expect("member");

        // Encode the constant tone through the same codec the member uses,
        // so what the mixer decodes is exactly what a peer would have sent.
        let mut source = CodecFactory::create_codec(0, Some(8_000), Some(1)).expect("codec");
        let payload = source
            .encode(&AudioFrame {
                samples: vec![sample; samples],
                sample_rate: 8_000,
                channels: 1,
                duration: MIX_INTERVAL,
                timestamp: 0,
            })
            .expect("encode");
        inbound_tx
            .try_send(MediaFrame {
                stream_id: crate::ids::StreamId::from_string(id),
                kind: crate::stream::StreamKind::Audio,
                payload: bytes::Bytes::from(payload),
                timestamp_rtp: 0,
                captured_at: chrono::Utc::now(),
                payload_type: Some(0),
            })
            .expect("seed frame");
        member.timestamp = 0;
        (member, outbound_rx)
    }

    /// Decode one produced frame back to PCM for assertion.
    fn decode_one(receiver: &mut mpsc::Receiver<MediaFrame>) -> Vec<i16> {
        use rvoip_media_core::codec::factory::CodecFactory;
        let frame = receiver.try_recv().expect("a frame was produced");
        let mut decoder = CodecFactory::create_codec(0, Some(8_000), Some(1)).expect("codec");
        decoder.decode(&frame.payload).expect("decode").samples
    }

    #[tokio::test]
    async fn each_member_hears_the_others_and_never_themselves() {
        let samples = 160;
        let (first, mut first_out) = seeded_member("a", 4_000, samples);
        let (second, mut second_out) = seeded_member("b", 1_000, samples);
        let (third, mut third_out) = seeded_member("c", 0, samples);

        let mut members = HashMap::new();
        members.insert(first.connection_id.clone(), first);
        members.insert(second.connection_id.clone(), second);
        members.insert(third.connection_id.clone(), third);

        mix_once(&mut members, samples);

        // G.711 is lossy, so assert the mix is near the expected sum rather
        // than exactly it. The point being proven is *which* voices are in
        // each stream, and the gap between 1000 and 5000 is far wider than
        // companding error.
        let near = |actual: i16, expected: i16| {
            let error = i32::from(actual) - i32::from(expected);
            assert!(
                error.abs() < 300,
                "expected about {expected}, got {actual}"
            );
        };

        // The loud member hears only the quiet one.
        near(decode_one(&mut first_out)[10], 1_000);
        // The quiet member hears only the loud one.
        near(decode_one(&mut second_out)[10], 4_000);
        // The silent member hears both.
        near(decode_one(&mut third_out)[10], 5_000);
    }

    #[tokio::test]
    async fn a_lone_member_hears_silence_rather_than_themselves() {
        let samples = 160;
        let (only, mut only_out) = seeded_member("solo", 6_000, samples);
        let mut members = HashMap::new();
        members.insert(only.connection_id.clone(), only);

        mix_once(&mut members, samples);

        let heard = decode_one(&mut only_out);
        assert!(
            heard.iter().all(|sample| sample.abs() < 300),
            "a member alone in a conference must not hear their own voice back"
        );
    }

    #[tokio::test]
    async fn a_departed_member_is_dropped_rather_than_retried() {
        let samples = 160;
        let (present, _present_out) = seeded_member("present", 1_000, samples);
        let (gone, gone_out) = seeded_member("gone", 1_000, samples);
        drop(gone_out);

        let mut members = HashMap::new();
        members.insert(present.connection_id.clone(), present);
        members.insert(gone.connection_id.clone(), gone);

        mix_once(&mut members, samples);

        assert_eq!(members.len(), 1, "a closed transport leaves the conference");
        assert!(members.contains_key(&ConnectionId::from_string("present")));
    }

    #[tokio::test]
    async fn the_sum_clips_instead_of_wrapping() {
        let samples = 160;
        // Three members near full scale sum well past i16::MAX. Wrapping
        // would invert the waveform and produce a loud click.
        let (a, _a_out) = seeded_member("a", 30_000, samples);
        let (b, _b_out) = seeded_member("b", 30_000, samples);
        let (c, mut c_out) = seeded_member("c", 0, samples);

        let mut members = HashMap::new();
        members.insert(a.connection_id.clone(), a);
        members.insert(b.connection_id.clone(), b);
        members.insert(c.connection_id.clone(), c);

        mix_once(&mut members, samples);

        let heard = decode_one(&mut c_out);
        assert!(
            heard.iter().all(|sample| *sample > 20_000),
            "a clipped sum stays loud and positive; a wrapped one would go negative"
        );
    }
}
