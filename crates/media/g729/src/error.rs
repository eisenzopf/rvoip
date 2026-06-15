use core::fmt;

/// Public enum `CodecError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    InvalidPcmLength {
        expected: usize,
        got: usize,
    },
    InvalidBitstreamLength {
        expected: &'static [usize],
        got: usize,
    },
    InvalidParameterLength {
        expected: usize,
        got: usize,
    },
    InvalidFrameType {
        got: i16,
    },
    IoUnavailable,
    BackendFailure,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::InvalidPcmLength { expected, got } => {
                write!(f, "invalid PCM length: expected {expected}, got {got}")
            }
            CodecError::InvalidBitstreamLength { expected, got } => {
                write!(
                    f,
                    "invalid bitstream length: expected one of {expected:?}, got {got}"
                )
            }
            CodecError::InvalidParameterLength { expected, got } => {
                write!(
                    f,
                    "invalid parameter buffer length: expected at least {expected}, got {got}"
                )
            }
            CodecError::InvalidFrameType { got } => {
                write!(f, "invalid frame type code: {got}")
            }
            CodecError::IoUnavailable => write!(f, "std I/O unavailable in this build"),
            CodecError::BackendFailure => write!(f, "backend process failed"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CodecError {}
