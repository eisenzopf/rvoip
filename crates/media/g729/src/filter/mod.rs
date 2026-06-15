#![allow(clippy::manual_memcpy)]
#![allow(clippy::needless_range_loop)]

/// Public module `convolve`.
pub mod convolve;
/// Public module `preemph`.
pub mod preemph;
/// Public module `resid`.
pub mod resid;
/// Public module `syn`.
pub mod syn;

/// Public re-export.
pub use convolve::{convolve, convolve_with_ctx};
/// Public re-export.
pub use preemph::preemphasis_with_mem;
/// Public re-export.
pub use resid::{residu, residu_with_ctx};
/// Public re-export.
pub use syn::{syn_filt, syn_filt_with_ctx};
