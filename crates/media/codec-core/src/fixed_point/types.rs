/// A 16-bit fixed-point value, the ETSI `Word16`.
///
/// A newtype rather than a bare `i16` so that ordinary Rust arithmetic — which
/// wraps or panics — cannot be used by accident where the specification
/// requires a saturating basic operator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Word16(pub i16);

/// A 32-bit fixed-point value, the ETSI `Word32`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Word32(pub i32);

/// The ETSI basic-operator status flags.
///
/// The reference C carries these as globals (`Overflow`, `Carry`) that
/// operators set as a side effect and a few algorithms then read. Threading
/// them explicitly keeps that observable behaviour without global mutable
/// state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DspContext {
    /// Set when an operation saturated. Some algorithms branch on this, so it
    /// is part of the specified behaviour rather than diagnostics.
    pub overflow: bool,
    /// Carry out of a 32-bit add or subtract, used by the `_c` operator
    /// variants that implement extended-precision arithmetic.
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
