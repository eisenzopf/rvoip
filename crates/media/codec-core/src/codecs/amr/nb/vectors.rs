//! Reader for the isolated-stage vectors in `testdata/nb_stages.txt`.
//!
//! Test-only. The file is produced by `tools/amrnb_stage_oracle.c`, which
//! drives TS 26.073's own functions directly, so a mismatch localises to one
//! function instead of to a frame.
//!
//! # Format
//!
//! Section names start in column 0; every line inside a section is indented and
//! begins with a label. A section is therefore a flat sequence of labelled
//! rows, read in order — which matters, because most sections are *replays*
//! whose rows only mean anything in sequence.
//!
//! ```text
//! phdisp
//!   seq 3 3273
//!     step 1234 9000 1
//!   x 12 -8 …
//!   inno 0 0 8191 …
//!   out 45 -12 …
//! ```
//!
//! # A passing test that compares nothing
//!
//! A bit-exact test passes vacuously if the reader silently returns no rows, so
//! [`rows`] panics on an absent or empty section rather than returning an empty
//! vector, and each consumer asserts how many cases it actually compared. Worth
//! re-checking whenever the dump format changes: a green suite is otherwise
//! indistinguishable from a suite that compared nothing.

use crate::fixed_point::random::random;
use crate::fixed_point::types::Word16;

/// The committed vectors, as produced by `tools/amrnb_stage_oracle.c`.
pub const NB_STAGES: &str = include_str!("../testdata/nb_stages.txt");

/// One labelled row of a section.
///
/// Tokens are kept as text rather than parsed on read, because a few rows mix
/// words and numbers — `seq joint 0 101` names which gain path is being
/// replayed before giving its parameters. Parsing eagerly would either lose
/// those rows or force the generator to encode names as magic numbers.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    /// The row's leading word, e.g. `case`, `out`, `step`.
    pub label: &'static str,
    /// Everything after the label, unparsed.
    pub tokens: &'static str,
}

impl Row {
    /// The row's tokens.
    pub fn parts(&self) -> impl Iterator<Item = &'static str> {
        self.tokens.split_whitespace()
    }

    /// The row's tokens as `i32`.
    ///
    /// # Panics
    ///
    /// Panics on a token that is not an integer, which means the fixture and
    /// the reader disagree about this row's shape.
    #[must_use]
    pub fn ints(&self) -> Vec<i32> {
        self.parts()
            .map(|v| {
                v.parse()
                    .unwrap_or_else(|_| panic!("{}: {v:?} is not an integer", self.label))
            })
            .collect()
    }

    /// The row's tokens as `i16`.
    ///
    /// # Panics
    ///
    /// Panics if a value does not fit. Out of range means the fixture and the
    /// reader disagree about the format, which is worth failing on rather than
    /// truncating into a plausible wrong number.
    #[must_use]
    pub fn i16s(&self) -> Vec<i16> {
        self.ints()
            .into_iter()
            .map(|v| i16::try_from(v).expect("stage vector value fits in i16"))
            .collect()
    }

    /// The row's tokens as [`Word16`].
    #[must_use]
    pub fn words(&self) -> Vec<Word16> {
        self.i16s().into_iter().map(Word16).collect()
    }

    /// The `n`th token, as text.
    ///
    /// # Panics
    ///
    /// Panics if the row is shorter than that.
    #[must_use]
    pub fn tag(&self, n: usize) -> &'static str {
        self.parts()
            .nth(n)
            .unwrap_or_else(|| panic!("{}: no token {n}", self.label))
    }

    /// Expand a `nz count pos val …` row back into a full codevector.
    ///
    /// The algebraic codevectors are 40 samples of which at most ten are
    /// non-zero, so the oracle writes them sparsely. Reconstructing here rather
    /// than in each consumer keeps the "what did the reference produce"
    /// question in one place.
    ///
    /// # Panics
    ///
    /// Panics if the row is not a well-formed sparse vector.
    #[must_use]
    pub fn pulses(&self, len: usize) -> Vec<Word16> {
        let v = self.ints();
        assert_eq!(self.label, "nz", "not a sparse codevector row");
        let count = usize::try_from(v[0]).expect("non-negative pulse count");
        assert_eq!(
            v.len(),
            1 + 2 * count,
            "sparse row length disagrees with its count"
        );
        let mut out = vec![Word16(0); len];
        for pair in v[1..].chunks_exact(2) {
            let pos = usize::try_from(pair[0]).expect("non-negative position");
            out[pos] = Word16(i16::try_from(pair[1]).expect("pulse fits in i16"));
        }
        out
    }
}

/// Every row of one named section, in file order.
///
/// # Panics
///
/// Panics if the section is absent or empty. Both mean the fixture was
/// regenerated with a different set of sections, and a test that silently
/// compared nothing is worse than one that fails.
#[must_use]
pub fn rows(section: &str) -> Vec<Row> {
    let mut out = Vec::new();
    let mut inside = false;

    for line in NB_STAGES.lines() {
        if line.starts_with(char::is_whitespace) {
            if inside {
                let trimmed = line.trim_start();
                let (label, tokens) = trimmed
                    .split_once(char::is_whitespace)
                    .unwrap_or((trimmed, ""));
                out.push(Row {
                    label,
                    tokens: tokens.trim(),
                });
            }
        } else if inside {
            break;
        } else {
            inside = line.trim_end() == section;
        }
    }

    assert!(
        !out.is_empty(),
        "nb_stages.txt has no section {section:?} — regenerate it with \
         tools/build-amrnb-reference.sh"
    );
    out
}

/// Regenerate the pseudo-random input the oracle used, from the seed it
/// recorded.
///
/// The oracle prints its seed rather than its inputs, so the two sides share a
/// generator instead of a copy of the data. The recurrence is the ETSI one,
/// `seed = (seed * 31821 + 13849) mod 2^16`.
#[must_use]
pub fn noise(seed: i16, count: usize, shift: u32) -> Vec<Word16> {
    let mut s = seed;
    (0..count)
        .map(|_| Word16(random(&mut s) >> shift))
        .collect()
}

/// Advance a shared generator by one step, for sections that interleave
/// scalars and vectors drawn from the same stream.
pub fn next_noise(seed: &mut i16) -> i16 {
    random(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_the_oracle_writes_is_readable() {
        // Named explicitly rather than discovered, so a section quietly
        // dropped from the generator fails here instead of taking its tests
        // with it.
        for section in [
            "lag3",
            "lag6",
            "predlt3",
            "predlt6",
            "cb2i40_9",
            "cb2i40_11",
            "cb3i40_14",
            "cb4i40_17",
            "cb8i40_31",
            "cb10i40_35",
            "gains",
            "conceal",
            "synfilt",
            "agc",
            "agc2",
            "weightai",
            "residu",
            "preemph",
            "postfilter",
            "postproc",
            "phdisp",
            "plsf5",
            "intlsf",
            "lspavg",
            "cbgainav",
            "bgnscd",
            "exctrl",
        ] {
            let r = rows(section);
            assert!(!r.is_empty(), "{section} is empty");
        }
    }

    #[test]
    fn sections_do_not_bleed_into_each_other() {
        // `lag6` immediately follows `lag3`, so a reader that keeps going past
        // the section end would silently double `lag3`'s row count and every
        // comparison would still pass.
        let lag3 = rows("lag3");
        assert!(lag3.iter().all(|r| r.label == "case" || r.label == "out"));
        let lag6 = rows("lag6");
        assert!(lag6.len() < lag3.len(), "lag6 is the smaller sweep");
    }

    #[test]
    fn the_noise_generator_matches_the_oracles() {
        // The oracle's first pseudo-random draw for the `predlt3` case at
        // T0 = 20, frac = 0 is seeded 1234 + 20*7 + 0 = 1374. If the two
        // generators ever disagree, every replayed section compares against
        // inputs the reference never saw — and the failures would look like
        // DSP bugs.
        let v = noise(1374, 4, 3);
        assert_eq!(v.len(), 4);
        let mut s = 1374i16;
        for w in v {
            let want = ((i32::from(s).wrapping_mul(31821) + 13849) & 0xFFFF) as i16;
            s = want;
            assert_eq!(w.0, want >> 3);
        }
    }
}
