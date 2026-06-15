#![allow(dead_code)]
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

//! Annex B VAD decision stage.

use crate::dsp::arith32::{l_add, l_deposit_h, l_mac, l_mult};
use crate::dsp::shift::l_shr;
use crate::dsp::types::{DspContext, Word16};
use crate::tables::vad::{NOISE, VOICE};

pub(super) fn make_dec_impl(dsle: i16, dse: i16, sd: i16, dszc: i16) -> i16 {
    let mut ctx = DspContext::default();

    let mut acc0 = l_mult(&mut ctx, Word16(dszc), Word16(-14680));
    acc0 = l_mac(&mut ctx, acc0, Word16(8192), Word16(-28521));
    acc0 = l_shr(&mut ctx, acc0, 8);
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(sd)));
    if acc0.0 > 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dszc), Word16(19065));
    acc0 = l_mac(&mut ctx, acc0, Word16(8192), Word16(-19446));
    acc0 = l_shr(&mut ctx, acc0, 7);
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(sd)));
    if acc0.0 > 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dszc), Word16(20480));
    acc0 = l_mac(&mut ctx, acc0, Word16(8192), Word16(16384));
    acc0 = l_shr(&mut ctx, acc0, 2);
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(dse)));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dszc), Word16(-16384));
    acc0 = l_mac(&mut ctx, acc0, Word16(8192), Word16(19660));
    acc0 = l_shr(&mut ctx, acc0, 2);
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(dse)));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dse), Word16(32767));
    acc0 = l_mac(&mut ctx, acc0, Word16(1024), Word16(30802));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(sd), Word16(-28160));
    acc0 = l_mac(&mut ctx, acc0, Word16(64), Word16(19988));
    acc0 = l_mac(&mut ctx, acc0, Word16(dse), Word16(512));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(sd), Word16(32767));
    acc0 = l_mac(&mut ctx, acc0, Word16(32), Word16(-30199));
    if acc0.0 > 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dszc), Word16(-20480));
    acc0 = l_mac(&mut ctx, acc0, Word16(8192), Word16(22938));
    acc0 = l_shr(&mut ctx, acc0, 2);
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(dse)));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dszc), Word16(23831));
    acc0 = l_mac(&mut ctx, acc0, Word16(4096), Word16(31576));
    acc0 = l_shr(&mut ctx, acc0, 2);
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(dse)));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dse), Word16(32767));
    acc0 = l_mac(&mut ctx, acc0, Word16(2048), Word16(17367));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(sd), Word16(-22400));
    acc0 = l_mac(&mut ctx, acc0, Word16(32), Word16(25395));
    acc0 = l_mac(&mut ctx, acc0, Word16(dsle), Word16(256));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dse), Word16(-30427));
    acc0 = l_mac(&mut ctx, acc0, Word16(256), Word16(-29959));
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(dsle)));
    if acc0.0 > 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dse), Word16(-23406));
    acc0 = l_mac(&mut ctx, acc0, Word16(512), Word16(28087));
    acc0 = l_add(&mut ctx, acc0, l_deposit_h(Word16(dsle)));
    if acc0.0 < 0 {
        return VOICE;
    }

    acc0 = l_mult(&mut ctx, Word16(dse), Word16(24576));
    acc0 = l_mac(&mut ctx, acc0, Word16(1024), Word16(29491));
    acc0 = l_mac(&mut ctx, acc0, Word16(dsle), Word16(16384));
    if acc0.0 < 0 {
        return VOICE;
    }

    NOISE
}

#[inline(always)]
pub(crate) fn make_decision(dsle: i16, dse: i16, sd: i16, dszc: i16) -> i16 {
    make_dec_impl(dsle, dse, sd, dszc)
}
