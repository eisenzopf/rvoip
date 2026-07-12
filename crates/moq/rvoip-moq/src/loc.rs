use bytes::Bytes;
use rvoip_core_traits::stream::{MediaFrame, StreamKind};

/// LOC-03 Timestamp object property identifier.
pub const LOC_TIMESTAMP_PROPERTY: u64 = 0x0a;
/// LOC-03 Timescale track/object property identifier.
pub const LOC_TIMESCALE_PROPERTY: u64 = 0x08;
pub const OPUS_SAMPLE_RATE: u32 = 48_000;
pub const OPUS_CHANNELS: u8 = 1;
pub const OPUS_FRAME_DURATION_MS: u16 = 20;
pub const OPUS_RTP_TIMESTAMP_STEP: u32 = 960;

/// Transport-independent LOC integer property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocProperty {
    pub id: u64,
    pub value: u64,
}

/// One canonical LOC-03 Opus object, before conversion to MOQT wire types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocAudioObject {
    pub group_id: u64,
    pub object_id: u64,
    /// Extended RTP timestamp used by LOC.
    ///
    /// RTP supplies a wrapping 32-bit timestamp. LOC integer properties can
    /// carry the monotonically extended value, so a long-running publication
    /// does not jump backwards roughly every 24 hours and 51 minutes at
    /// 48 kHz.
    pub timestamp: u64,
    pub timescale: u32,
    pub payload: Bytes,
}

impl LocAudioObject {
    pub fn properties(&self) -> [LocProperty; 2] {
        [
            LocProperty {
                id: LOC_TIMESTAMP_PROPERTY,
                value: self.timestamp,
            },
            LocProperty {
                id: LOC_TIMESCALE_PROPERTY,
                value: u64::from(self.timescale),
            },
        ]
    }
}

/// An accepted RTP timestamp that did not follow the canonical 20 ms cadence.
///
/// Discontinuities are metadata, not packetization failures. The valid Opus
/// frame that establishes the new cadence is returned alongside this value so
/// callers can publish it while making the discontinuity observable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocTimestampDiscontinuity {
    pub expected_rtp_timestamp: u32,
    pub actual_rtp_timestamp: u32,
}

/// One accepted LOC object and any RTP timestamp discontinuity it introduced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocPacketizedFrame {
    pub object: LocAudioObject,
    pub discontinuity: Option<LocTimestampDiscontinuity>,
}

/// Stateful validator/packetizer for the Bridgefu 1.0 broadcast profile.
///
/// Each 20 ms Opus packet becomes Object 0 in its own monotonically numbered
/// MOQT Group. RTP timestamp advances other than exactly 960 at 48 kHz are
/// surfaced as discontinuities without discarding the recovery frame.
#[derive(Clone, Debug, Default)]
pub struct LocOpusPacketizer {
    next_group_id: u64,
    last_rtp_timestamp: Option<u32>,
    last_loc_timestamp: Option<u64>,
}

impl LocOpusPacketizer {
    pub const fn new() -> Self {
        Self {
            next_group_id: 0,
            last_rtp_timestamp: None,
            last_loc_timestamp: None,
        }
    }

    pub const fn with_group_id(next_group_id: u64) -> Self {
        Self {
            next_group_id,
            last_rtp_timestamp: None,
            last_loc_timestamp: None,
        }
    }

    pub fn packetize(&mut self, frame: &MediaFrame) -> Result<LocPacketizedFrame, LocError> {
        if frame.kind != StreamKind::Audio {
            return Err(LocError::NotAudio);
        }
        validate_opus_20ms_mono(&frame.payload)?;

        let expected_timestamp = self.expected_timestamp();
        let discontinuity = expected_timestamp
            .filter(|expected| *expected != frame.timestamp_rtp)
            .map(|expected_rtp_timestamp| LocTimestampDiscontinuity {
                expected_rtp_timestamp,
                actual_rtp_timestamp: frame.timestamp_rtp,
            });

        let timestamp = match (self.last_rtp_timestamp, self.last_loc_timestamp) {
            (Some(previous_rtp), Some(previous_loc)) => {
                let forward_delta = frame.timestamp_rtp.wrapping_sub(previous_rtp);
                // A delta in the forward half of the RTP sequence space is an
                // ordinary advance, including a 32-bit wrap. A duplicate or a
                // backwards/reset timestamp is rebased by one canonical frame
                // so LOC remains strictly monotonic while the discontinuity is
                // reported to the caller.
                let extension_delta = if forward_delta == 0 || forward_delta >= (1_u32 << 31) {
                    u64::from(OPUS_RTP_TIMESTAMP_STEP)
                } else {
                    u64::from(forward_delta)
                };
                previous_loc
                    .checked_add(extension_delta)
                    .ok_or(LocError::TimestampOverflow)?
            }
            (None, None) => u64::from(frame.timestamp_rtp),
            _ => unreachable!("RTP and LOC timestamp state are updated together"),
        };

        let next_group_id = self
            .next_group_id
            .checked_add(1)
            .ok_or(LocError::GroupIdExhausted)?;

        let object = LocAudioObject {
            group_id: self.next_group_id,
            object_id: 0,
            timestamp,
            timescale: OPUS_SAMPLE_RATE,
            payload: frame.payload.clone(),
        };
        self.next_group_id = next_group_id;
        self.last_rtp_timestamp = Some(frame.timestamp_rtp);
        self.last_loc_timestamp = Some(timestamp);
        Ok(LocPacketizedFrame {
            object,
            discontinuity,
        })
    }

    pub fn next_group_id(&self) -> u64 {
        self.next_group_id
    }

    pub fn expected_timestamp(&self) -> Option<u32> {
        self.last_rtp_timestamp
            .map(|timestamp| timestamp.wrapping_add(OPUS_RTP_TIMESTAMP_STEP))
    }

    pub fn last_loc_timestamp(&self) -> Option<u64> {
        self.last_loc_timestamp
    }
}

/// Validate the self-describing portion of an Opus packet for the canonical
/// mono, 20 ms profile.
pub fn validate_opus_20ms_mono(packet: &[u8]) -> Result<(), LocError> {
    let toc = *packet.first().ok_or(LocError::EmptyPacket)?;
    if toc & 0x04 != 0 {
        return Err(LocError::StereoPacket);
    }

    let configuration = toc >> 3;
    let frame_code = toc & 0x03;
    let frame_count = match frame_code {
        0 => 1,
        1 | 2 => 2,
        3 => {
            let count = packet.get(1).ok_or(LocError::MissingFrameCount)? & 0x3f;
            if count == 0 {
                return Err(LocError::InvalidFrameCount { count });
            }
            count
        }
        _ => unreachable!("two-bit Opus frame code"),
    };

    // Half-millisecond units avoid floating point for CELT's 2.5 ms mode.
    let frame_duration_half_ms: u16 = match configuration {
        0..=11 => [20, 40, 80, 120][usize::from(configuration % 4)],
        12..=15 => [20, 40][usize::from(configuration % 2)],
        16..=31 => [5, 10, 20, 40][usize::from(configuration % 4)],
        _ => unreachable!("five-bit Opus configuration"),
    };
    let duration_half_ms = frame_duration_half_ms * u16::from(frame_count);
    let expected_half_ms = OPUS_FRAME_DURATION_MS * 2;
    if duration_half_ms != expected_half_ms {
        return Err(LocError::PacketDuration {
            expected_ms: OPUS_FRAME_DURATION_MS,
            actual_half_ms: duration_half_ms,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LocError {
    #[error("LOC audio packetizer only accepts audio frames")]
    NotAudio,
    #[error("Opus packet is empty")]
    EmptyPacket,
    #[error("Bridgefu broadcast profile requires mono Opus packets")]
    StereoPacket,
    #[error("Opus arbitrary-frame-count packet is missing its frame count byte")]
    MissingFrameCount,
    #[error("Opus packet has invalid frame count {count}")]
    InvalidFrameCount { count: u8 },
    #[error(
        "Bridgefu broadcast profile requires {expected_ms} ms Opus packets, got {actual_half_ms}/2 ms"
    )]
    PacketDuration {
        expected_ms: u16,
        actual_half_ms: u16,
    },
    #[error("LOC timestamp exhausted its 64-bit sequence space")]
    TimestampOverflow,
    #[error("LOC group ID exhausted its 64-bit sequence space")]
    GroupIdExhausted,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rvoip_core_traits::ids::StreamId;

    use super::*;

    fn frame(timestamp: u32, payload: &'static [u8]) -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(payload),
            timestamp_rtp: timestamp,
            captured_at: Utc::now(),
            payload_type: Some(111),
        }
    }

    #[test]
    fn validates_single_and_multi_frame_twenty_millisecond_opus() {
        // Hybrid config 15: one 20 ms mono frame.
        validate_opus_20ms_mono(&[0x78, 0x00]).unwrap();
        // Hybrid config 14: two 10 ms mono frames.
        validate_opus_20ms_mono(&[0x71, 0x00]).unwrap();

        assert_eq!(
            validate_opus_20ms_mono(&[0x7c, 0x00]).unwrap_err(),
            LocError::StereoPacket
        );
        assert!(matches!(
            validate_opus_20ms_mono(&[0x70, 0x00]),
            Err(LocError::PacketDuration { .. })
        ));
        assert_eq!(
            validate_opus_20ms_mono(&[0x7b]).unwrap_err(),
            LocError::MissingFrameCount
        );
    }

    #[test]
    fn emits_one_object_per_group_and_reports_cadence_discontinuities() {
        let mut packetizer = LocOpusPacketizer::new();
        let first = packetizer.packetize(&frame(10, &[0x78, 0x00])).unwrap();
        let second = packetizer.packetize(&frame(970, &[0x78, 0x01])).unwrap();
        assert_eq!((first.object.group_id, first.object.object_id), (0, 0));
        assert_eq!((second.object.group_id, second.object.object_id), (1, 0));
        assert_eq!(first.discontinuity, None);
        assert_eq!(second.discontinuity, None);
        assert_eq!(
            first.object.properties(),
            [
                LocProperty {
                    id: 0x0a,
                    value: 10
                },
                LocProperty {
                    id: 0x08,
                    value: 48_000
                }
            ]
        );
        let recovery = packetizer.packetize(&frame(1931, &[0x78, 0x02])).unwrap();
        assert_eq!(recovery.object.group_id, 2);
        assert_eq!(recovery.object.timestamp, 1931);
        assert_eq!(
            recovery.discontinuity,
            Some(LocTimestampDiscontinuity {
                expected_rtp_timestamp: 1930,
                actual_rtp_timestamp: 1931,
            })
        );

        // The frame that established the new cadence was not dropped, and the
        // immediately following timestamp is accepted normally.
        let after_recovery = packetizer.packetize(&frame(2891, &[0x78, 0x03])).unwrap();
        assert_eq!(after_recovery.object.group_id, 3);
        assert_eq!(after_recovery.discontinuity, None);
        assert_eq!(packetizer.next_group_id(), 4);
    }

    #[test]
    fn rtp_timestamp_wrap_is_valid_cadence() {
        let mut packetizer = LocOpusPacketizer::new();
        let before_wrap = packetizer
            .packetize(&frame(u32::MAX - 959, &[0x78, 0x00]))
            .unwrap();
        let after_wrap = packetizer.packetize(&frame(0, &[0x78, 0x01])).unwrap();
        assert_eq!(before_wrap.object.timestamp, u64::from(u32::MAX) - 959);
        assert_eq!(after_wrap.object.timestamp, 1_u64 << 32);
        assert!(after_wrap.object.timestamp > before_wrap.object.timestamp);
        assert_eq!(after_wrap.discontinuity, None);
        assert_eq!(packetizer.last_loc_timestamp(), Some(1_u64 << 32));
    }

    #[test]
    fn backwards_rtp_reset_is_rebased_without_regressing_loc_time() {
        let mut packetizer = LocOpusPacketizer::new();
        let first = packetizer
            .packetize(&frame(100_000, &[0x78, 0x00]))
            .unwrap();
        let reset = packetizer.packetize(&frame(0, &[0x78, 0x01])).unwrap();

        assert_eq!(first.object.timestamp, 100_000);
        assert_eq!(reset.object.timestamp, 100_960);
        assert_eq!(
            reset.discontinuity,
            Some(LocTimestampDiscontinuity {
                expected_rtp_timestamp: 100_960,
                actual_rtp_timestamp: 0,
            })
        );
    }
}
