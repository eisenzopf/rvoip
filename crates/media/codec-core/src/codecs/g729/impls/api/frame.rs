/// Encoded frame classification used by public API encode/decode calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    /// Regular 10-byte G.729 speech frame.
    Speech,
    /// 2-byte SID frame used for comfort noise updates.
    Sid,
    /// No payload frame (DTX no-transmit period / erasure concealment path).
    NoData,
}

impl FrameType {
    /// Packed payload size in bytes for this frame type.
    pub const fn byte_len(self) -> usize {
        match self {
            Self::Speech => 10,
            Self::Sid => 2,
            Self::NoData => 0,
        }
    }

    /// ITU serial payload size in bits for this frame type.
    pub const fn bit_len(self) -> usize {
        match self {
            Self::Speech => 80,
            Self::Sid => 15,
            Self::NoData => 0,
        }
    }
}
