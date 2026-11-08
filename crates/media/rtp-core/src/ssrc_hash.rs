//! Executor v2: a fast hasher for the SSRC-keyed maps on the
//! per-packet path.
//!
//! The SRTP context keeps its rollover/replay state in `HashMap<u32, _>`
//! (three lookups per protected or unprotected packet) and every RTP session
//! keeps its receive streams in a `DashMap<u32, _>` (one lookup per inbound
//! packet). With the default `RandomState` each lookup runs SipHash-1-3 over
//! four bytes. Keys here are 32-bit SSRCs, so a keyed SplitMix64 finalizer
//! gives the same distribution for a fraction of the cost.
//!
//! The seed is drawn once per process from `RandomState`, so bucket
//! placement is not predictable from outside. That matters only for the
//! session stream map, whose entries a plain-RTP peer can create; SRTP state
//! entries are inserted only after authentication. No crate is added: the
//! hasher is the twenty lines below.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;

fn process_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(0x5352_5443_5353_5243);
        hasher.finish()
    })
}

/// SplitMix64 finalizer: a bijection on `u64` with full avalanche.
#[inline]
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `BuildHasher` for SSRC-keyed maps. The choice is made when the map is
/// built: the fast keyed mix when
/// [`MediaExecutorConfig::fast_ssrc_hash`](crate::executor::MediaExecutorConfig::fast_ssrc_hash)
/// is active, the standard SipHash `RandomState` otherwise, so a v1 process
/// hashes exactly as before.
#[derive(Clone, Debug)]
pub(crate) struct SsrcHashBuilder {
    seed: u64,
    sip: Option<RandomState>,
}

impl SsrcHashBuilder {
    pub(crate) fn new() -> Self {
        if crate::executor::fast_ssrc_hash_enabled() {
            Self::fast()
        } else {
            Self {
                seed: 0,
                sip: Some(RandomState::new()),
            }
        }
    }

    pub(crate) fn fast() -> Self {
        Self {
            seed: process_seed(),
            sip: None,
        }
    }

    #[cfg(test)]
    fn is_fast(&self) -> bool {
        self.sip.is_none()
    }
}

impl Default for SsrcHashBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildHasher for SsrcHashBuilder {
    type Hasher = SsrcHasher;

    #[inline]
    fn build_hasher(&self) -> SsrcHasher {
        match &self.sip {
            None => SsrcHasher::Fast(self.seed),
            Some(sip) => SsrcHasher::Sip(sip.build_hasher()),
        }
    }
}

/// Hasher state. `u32` keys take the `write_u32` fast path; any other input
/// is folded eight bytes at a time, so the hasher stays correct (if slower)
/// for other key types.
pub(crate) enum SsrcHasher {
    Fast(u64),
    Sip(std::collections::hash_map::DefaultHasher),
}

impl Hasher for SsrcHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        match self {
            Self::Fast(state) => {
                for chunk in bytes.chunks(8) {
                    let mut word = [0u8; 8];
                    word[..chunk.len()].copy_from_slice(chunk);
                    *state = mix(*state ^ u64::from_le_bytes(word));
                }
            }
            Self::Sip(sip) => sip.write(bytes),
        }
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        match self {
            Self::Fast(state) => *state = mix(*state ^ u64::from(value)),
            Self::Sip(sip) => sip.write_u32(value),
        }
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        match self {
            Self::Fast(state) => *state = mix(*state ^ value),
            Self::Sip(sip) => sip.write_u64(value),
        }
    }

    #[inline]
    fn finish(&self) -> u64 {
        match self {
            Self::Fast(state) => *state,
            Self::Sip(sip) => sip.finish(),
        }
    }
}

/// `HashMap` keyed by SSRC with the fast hasher.
pub(crate) type SsrcHashMap<V> = std::collections::HashMap<u32, V, SsrcHashBuilder>;

pub(crate) fn ssrc_hash_map<V>() -> SsrcHashMap<V> {
    std::collections::HashMap::with_hasher(SsrcHashBuilder::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn hash_of(builder: &SsrcHashBuilder, value: u32) -> u64 {
        let mut hasher = builder.build_hasher();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn v1_keeps_siphash_and_v2_uses_the_fast_mix() {
        {
            let _v1 = crate::executor::override_media_executor_config_on_this_thread(
                crate::executor::MediaExecutorConfig::v1(),
            );
            assert!(!SsrcHashBuilder::new().is_fast());
        }
        let _v2 = crate::executor::override_media_executor_config_on_this_thread(
            crate::executor::MediaExecutorConfig::v2(),
        );
        // On by default under v2 unless the process disabled the switch.
        let fast = SsrcHashBuilder::new().is_fast();
        assert_eq!(fast, crate::executor::fast_ssrc_hash_enabled());
    }

    #[test]
    fn equal_keys_hash_equally_and_neighbours_spread() {
        assert_eq!(
            SsrcHashBuilder::fast().seed,
            SsrcHashBuilder::fast().seed,
            "one seed per process"
        );
        // A fixed seed keeps the spread check deterministic.
        let builder = SsrcHashBuilder {
            seed: 0x1234_5678_9abc_def0,
            sip: None,
        };
        assert_eq!(hash_of(&builder, 7), hash_of(&builder, 7));
        // Sequential SSRCs land in distinct low-bit buckets (a 64-bucket table).
        let buckets: std::collections::BTreeSet<u64> = (0..64u32)
            .map(|ssrc| hash_of(&builder, ssrc) & 63)
            .collect();
        assert!(buckets.len() > 32, "poor spread: {}", buckets.len());
    }

    #[test]
    fn the_map_behaves_like_a_std_map() {
        let mut map = ssrc_hash_map::<u8>();
        for ssrc in [0u32, 1, u32::MAX, 0xDEAD_BEEF] {
            map.insert(ssrc, (ssrc & 0xFF) as u8);
        }
        assert_eq!(map.get(&u32::MAX), Some(&0xFF));
        assert_eq!(map.get(&0xDEAD_BEEF), Some(&0xEF));
        assert_eq!(map.remove(&1), Some(1));
        assert_eq!(map.len(), 3);
    }
}
