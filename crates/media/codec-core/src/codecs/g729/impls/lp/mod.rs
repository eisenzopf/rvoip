//! LP analysis and transform modules.
//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

/// Public module `autocorr`.
pub mod autocorr;
/// Public module `az_lsp`.
pub mod az_lsp;
pub(crate) mod chebyshev;
/// Public module `interp`.
pub mod interp;
/// Public module `levinson`.
pub mod levinson;
/// Public module `lsf`.
pub mod lsf;
/// Public module `lsp_az`.
pub mod lsp_az;
/// Public module `weight`.
pub mod weight;
/// Public module `window`.
pub mod window;
