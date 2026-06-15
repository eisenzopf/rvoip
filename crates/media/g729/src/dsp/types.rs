/// Public struct `Word16`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Word16(pub i16);

/// Public struct `Word32`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Word32(pub i32);

/// Public struct `DspContext`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DspContext {
    pub overflow: bool,
    pub carry: bool,
}

impl DspContext {
    /// Public constant `fn`.
    pub const fn new() -> Self {
        Self {
            overflow: false,
            carry: false,
        }
    }

    /// Public function `reset_flags`.
    pub fn reset_flags(&mut self) {
        self.overflow = false;
        self.carry = false;
    }
}

/// Public constant `MAX_16`.
pub const MAX_16: i16 = i16::MAX;
/// Public constant `MIN_16`.
pub const MIN_16: i16 = i16::MIN;
/// Public constant `MAX_32`.
pub const MAX_32: i32 = i32::MAX;
/// Public constant `MIN_32`.
pub const MIN_32: i32 = i32::MIN;
