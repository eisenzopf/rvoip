//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{MSIZE, NB_POS};

pub(crate) const RRI0I0: usize = 0;
pub(crate) const RRI1I1: usize = RRI0I0 + NB_POS;
pub(crate) const RRI2I2: usize = RRI1I1 + NB_POS;
pub(crate) const RRI3I3: usize = RRI2I2 + NB_POS;
pub(crate) const RRI4I4: usize = RRI3I3 + NB_POS;
pub(crate) const RRI0I1: usize = RRI4I4 + NB_POS;
pub(crate) const RRI0I2: usize = RRI0I1 + MSIZE;
pub(crate) const RRI0I3: usize = RRI0I2 + MSIZE;
pub(crate) const RRI0I4: usize = RRI0I3 + MSIZE;
pub(crate) const RRI1I2: usize = RRI0I4 + MSIZE;
pub(crate) const RRI1I3: usize = RRI1I2 + MSIZE;
pub(crate) const RRI1I4: usize = RRI1I3 + MSIZE;
pub(crate) const RRI2I3: usize = RRI1I4 + MSIZE;
pub(crate) const RRI2I4: usize = RRI2I3 + MSIZE;
