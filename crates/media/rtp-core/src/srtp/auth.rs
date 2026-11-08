use super::SrtpAuthenticationAlgorithm;
use crate::error::Error;
use crate::Result;
use hmac::{Hmac, Mac};
use sha1::Sha1;

// Define type for HMAC-SHA1
type HmacSha1 = Hmac<Sha1>;

/// An HMAC-SHA1 tag truncated to the suite's tag length, on the stack.
pub(crate) struct HmacSha1Tag {
    bytes: hmac::digest::Output<HmacSha1>,
    len: usize,
}

impl HmacSha1Tag {
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// An HMAC-SHA1 state already keyed (the inner and outer pads absorbed),
/// without a block buffer.
pub(crate) type HmacSha1Core = hmac::HmacCore<Sha1>;

/// The key of one HMAC-SHA1 tag.
#[derive(Clone, Copy)]
pub(crate) enum HmacSha1Key<'a> {
    /// The raw authentication key, keyed for this tag (two SHA-1
    /// compressions for the pads).
    Raw(&'a [u8]),
    /// A state keyed once with the authentication key (each RTP session key
    /// set keeps one in its [`SrtpAuthenticator`], hot call state one per
    /// SRTP direction); the tag is the same.
    Keyed(&'a HmacSha1Core),
}

/// Key an HMAC-SHA1 state with `key` once, for [`HmacSha1Key::Keyed`].
pub(crate) fn hmac_sha1_core(key: &[u8]) -> Result<HmacSha1Core> {
    HmacSha1::new_from_slice(key)
        .map(|mac| mac.decompose().0)
        .map_err(|e| Error::SrtpError(format!("Failed to create HMAC: {}", e)))
}

/// HMAC-SHA1 over `data` followed by `trailer` (the ROC for SRTP, nothing
/// for SRTCP), truncated to `tag_length`, with a raw key; see
/// [`hmac_sha1_tag_with`].
#[inline]
pub(crate) fn hmac_sha1_tag(
    key: &[u8],
    data: &[u8],
    trailer: &[u8],
    tag_length: usize,
) -> Result<HmacSha1Tag> {
    hmac_sha1_tag_with(HmacSha1Key::Raw(key), data, trailer, tag_length)
}

/// HMAC-SHA1 over `data` followed by `trailer` (the ROC for SRTP, nothing
/// for SRTCP), truncated to `tag_length`: the one computation behind every
/// SRTP and SRTCP authentication tag this crate creates or verifies, from a
/// raw key or from a keyed state (hot call state's packed SRTP block).
///
/// Deliberately kept out of line. The SHA extensions' compression is
/// legacy-SSE code: inlined into the AVX code of a packet path (the socket
/// owner's receive loop, the synchronous TX path, SRTCP) under fat LTO or
/// PGO it causes SSE/AVX transition stalls that cost more than the call.
#[inline(never)]
pub(crate) fn hmac_sha1_tag_with(
    key: HmacSha1Key<'_>,
    data: &[u8],
    trailer: &[u8],
    tag_length: usize,
) -> Result<HmacSha1Tag> {
    let mut mac = match key {
        HmacSha1Key::Raw(key) => HmacSha1::new_from_slice(key)
            .map_err(|e| Error::SrtpError(format!("Failed to create HMAC: {}", e)))?,
        HmacSha1Key::Keyed(core) => HmacSha1::from_core(core.clone()),
    };
    mac.update(data);
    mac.update(trailer);
    let result = mac.finalize().into_bytes();
    // Guard against a misconfigured suite whose tag_length exceeds the
    // 20-byte HMAC-SHA1 output (a slice would otherwise panic).
    if tag_length > result.len() {
        return Err(Error::SrtpError(format!(
            "auth tag_length {} exceeds HMAC-SHA1 output {}",
            tag_length,
            result.len()
        )));
    }
    Ok(HmacSha1Tag {
        bytes: result,
        len: tag_length,
    })
}

/// SRTP Authentication Handler
#[derive(Clone)]
pub struct SrtpAuthenticator {
    /// Authentication algorithm
    algorithm: SrtpAuthenticationAlgorithm,

    /// HMAC state with the key already absorbed. Each packet clones it, so
    /// the key schedule runs once per authenticator instead of once per
    /// packet. `None` for null authentication or a rejected key.
    keyed_mac: Option<HmacSha1Core>,

    /// Authentication tag length in bytes
    tag_length: usize,
}

impl std::fmt::Debug for SrtpAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The keyed state is secret material; never print it.
        f.debug_struct("SrtpAuthenticator")
            .field("algorithm", &self.algorithm)
            .field("tag_length", &self.tag_length)
            .finish_non_exhaustive()
    }
}

impl SrtpAuthenticator {
    /// Create a new SRTP authenticator
    pub fn new(
        algorithm: SrtpAuthenticationAlgorithm,
        auth_key: Vec<u8>,
        tag_length: usize,
    ) -> Self {
        let keyed_mac = if algorithm == SrtpAuthenticationAlgorithm::Null {
            None
        } else {
            hmac_sha1_core(&auth_key).ok()
        };
        Self {
            algorithm,
            keyed_mac,
            tag_length,
        }
    }

    /// The HMAC-SHA1 state keyed once in [`Self::new`] (`None` for null
    /// authentication).
    pub(crate) fn keyed_mac(&self) -> Option<&HmacSha1Core> {
        self.keyed_mac.as_ref()
    }

    /// The keyed state as the key of one tag.
    fn hmac_key(&self) -> Result<HmacSha1Key<'_>> {
        self.keyed_mac
            .as_ref()
            .map(HmacSha1Key::Keyed)
            .ok_or_else(|| Error::SrtpError("Failed to create HMAC".to_string()))
    }

    /// Calculate authentication tag for an RTP packet
    pub fn calculate_auth_tag(&self, packet_data: &[u8], roc: u32) -> Result<Vec<u8>> {
        if self.algorithm == SrtpAuthenticationAlgorithm::Null {
            // Null authentication, return empty tag
            return Ok(Vec::new());
        }

        match self.algorithm {
            SrtpAuthenticationAlgorithm::HmacSha1_80 | SrtpAuthenticationAlgorithm::HmacSha1_32 => {
                // HMAC-SHA1 over the packet followed by the ROC, truncated
                // to the suite's tag length, continuing from the keyed state.
                let tag = hmac_sha1_tag_with(
                    self.hmac_key()?,
                    packet_data,
                    &roc.to_be_bytes(),
                    self.tag_length,
                )?;
                Ok(tag.as_slice().to_vec())
            }
            SrtpAuthenticationAlgorithm::Null => {
                // Should not reach here due to the first check
                Ok(Vec::new())
            }
        }
    }

    /// Verify authentication tag for an RTP packet
    pub fn verify_auth_tag(&self, packet_data: &[u8], tag: &[u8], roc: u32) -> Result<bool> {
        if self.algorithm == SrtpAuthenticationAlgorithm::Null {
            // Null authentication, always valid
            return Ok(true);
        }

        // Calculate the expected tag
        let expected_tag = self.calculate_auth_tag(packet_data, roc)?;

        // Compare with the provided tag
        if expected_tag.len() != tag.len() {
            return Err(Error::SrtpError(format!(
                "Authentication tag length mismatch: expected {}, got {}",
                expected_tag.len(),
                tag.len()
            )));
        }

        // Constant-time comparison to prevent timing attacks
        let mut result = 0;
        for (a, b) in expected_tag.iter().zip(tag.iter()) {
            result |= a ^ b;
        }

        Ok(result == 0)
    }

    /// Allocation-free twin of [`Self::verify_auth_tag`] for the executor v2
    /// inline receive path, from this authenticator's keyed state; see
    /// [`Self::verify_auth_tag_in_place_with`].
    pub(crate) fn verify_auth_tag_in_place(
        &self,
        packet_data: &[u8],
        tag: &[u8],
        roc: u32,
    ) -> Result<bool> {
        if self.algorithm == SrtpAuthenticationAlgorithm::Null {
            return Ok(true);
        }
        Self::verify_auth_tag_in_place_with(
            self.algorithm,
            self.hmac_key()?,
            self.tag_length,
            packet_data,
            tag,
            roc,
        )
    }

    /// Allocation-free twin of [`Self::verify_auth_tag`] for the executor v2
    /// inline receive path. The MAC input, truncation, error cases and the
    /// constant-time comparison are identical; the expected tag stays on the
    /// stack instead of being copied into a `Vec`.
    pub(crate) fn verify_auth_tag_in_place_with(
        algorithm: SrtpAuthenticationAlgorithm,
        key: HmacSha1Key<'_>,
        tag_length: usize,
        packet_data: &[u8],
        tag: &[u8],
        roc: u32,
    ) -> Result<bool> {
        if algorithm == SrtpAuthenticationAlgorithm::Null {
            return Ok(true);
        }
        let expected = hmac_sha1_tag_with(key, packet_data, &roc.to_be_bytes(), tag_length)?;
        let expected = expected.as_slice();
        if expected.len() != tag.len() {
            return Err(Error::SrtpError(format!(
                "Authentication tag length mismatch: expected {}, got {}",
                expected.len(),
                tag.len()
            )));
        }
        let mut difference = 0u8;
        for (a, b) in expected.iter().zip(tag.iter()) {
            difference |= a ^ b;
        }
        Ok(difference == 0)
    }

    /// Allocation-free twin of [`Self::calculate_auth_tag`] for the executor
    /// v2 TX path, from this authenticator's keyed state; see
    /// [`Self::append_auth_tag_with`].
    pub(crate) fn append_auth_tag(
        &self,
        buffer: &mut Vec<u8>,
        start: usize,
        roc: u32,
    ) -> Result<()> {
        if self.algorithm == SrtpAuthenticationAlgorithm::Null {
            return Ok(());
        }
        Self::append_auth_tag_with(
            self.algorithm,
            self.hmac_key()?,
            self.tag_length,
            buffer,
            start,
            roc,
        )
    }

    /// Allocation-free twin of [`Self::calculate_auth_tag`] for the executor
    /// v2 TX path: the tag over `buffer[start..]` followed by the ROC is
    /// appended to `buffer`. Same MAC input, truncation and errors; a Null
    /// algorithm appends nothing (its tag is empty).
    pub(crate) fn append_auth_tag_with(
        algorithm: SrtpAuthenticationAlgorithm,
        key: HmacSha1Key<'_>,
        tag_length: usize,
        buffer: &mut Vec<u8>,
        start: usize,
        roc: u32,
    ) -> Result<()> {
        if algorithm == SrtpAuthenticationAlgorithm::Null {
            return Ok(());
        }
        let tag = hmac_sha1_tag_with(key, &buffer[start..], &roc.to_be_bytes(), tag_length)?;
        buffer.extend_from_slice(tag.as_slice());
        Ok(())
    }

    /// Get the authentication tag length
    pub fn tag_length(&self) -> usize {
        self.tag_length
    }

    /// Check if authentication is enabled
    pub fn is_enabled(&self) -> bool {
        self.algorithm != SrtpAuthenticationAlgorithm::Null
    }
}

/// SRTP Replay Protection
pub struct SrtpReplayProtection {
    /// Window size in packets
    window_size: usize,

    /// Highest sequence number received
    highest_seq: Option<u64>,

    /// Replay window bitmap (using index relative to highest seq)
    window: Vec<bool>,

    /// Whether replay protection is enabled
    enabled: bool,
}

impl SrtpReplayProtection {
    /// Create a new replay protection context
    pub fn new(window_size: u64) -> Self {
        // A zero-sized replay window cannot remember even the packet that
        // established it. Preserve the infallible constructor while treating
        // zero as the smallest useful window.
        let window_size = usize::try_from(window_size.max(1)).unwrap_or(usize::MAX);
        let window = vec![false; window_size];

        Self {
            window_size,
            highest_seq: None,
            window,
            enabled: true,
        }
    }

    /// Check if a packet is a replay
    pub fn check(&mut self, seq: u64) -> Result<bool> {
        if !self.enabled {
            return Ok(true); // Always allow if disabled
        }

        // Sequence number zero is valid, so absence of a highest sequence
        // must be represented explicitly rather than with a zero sentinel.
        let Some(highest_seq) = self.highest_seq else {
            self.highest_seq = Some(seq);
            // Mark first packet as received in the bitmap
            self.window[0] = true;
            return Ok(true);
        };

        // Check if the sequence number is too old (outside the replay window)
        // The window covers [highest_seq - window_size + 1, highest_seq]
        let window_lower_bound =
            highest_seq.saturating_sub(self.window_size.saturating_sub(1) as u64);
        if seq < window_lower_bound {
            // Too old, reject
            return Ok(false);
        }

        // Check if this is a higher sequence number
        if seq > highest_seq {
            let diff = seq - highest_seq;

            // Move every remembered packet back by the amount the highest
            // index advanced. Position zero always represents highest_seq.
            if diff >= self.window_size as u64 {
                // If the gap is larger than our window, clear the entire window
                self.window.fill(false);
            } else {
                let shift = diff as usize;
                let retained = self.window_size - shift;
                self.window.copy_within(..retained, shift);
                self.window[..shift].fill(false);
            }

            // Update highest sequence
            self.highest_seq = Some(seq);

            // Mark this sequence as received (position 0 in the window)
            self.window[0] = true;

            return Ok(true);
        }

        // At this point, we know:
        // 1. The sequence is not too old (within the valid window)
        // 2. It's not higher than highest_seq

        // Calculate the position in the window
        // The window is indexed relative to highest_seq
        // Position 0 = highest_seq, position 1 = highest_seq-1, etc.
        let window_pos = highest_seq - seq;

        // Check if we've already seen this sequence
        if self.window[window_pos as usize] {
            // Already received, reject as replay
            return Ok(false);
        }

        // Mark as received and allow
        self.window[window_pos as usize] = true;
        Ok(true)
    }

    /// Enable or disable replay protection
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Reset the replay protection
    pub fn reset(&mut self) {
        self.highest_seq = None;
        self.window.fill(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A keyed state (hot call state) produces exactly the raw key's tags
    /// and errors, for random keys and data, at every tag length.
    #[test]
    fn a_keyed_hmac_state_tags_exactly_like_its_raw_key() {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        for round in 0..64 {
            let key: Vec<u8> = (0..20).map(|_| rng.gen()).collect();
            let data: Vec<u8> = (0..rng.gen_range(0..300)).map(|_| rng.gen()).collect();
            let roc: u32 = rng.gen();
            let core = hmac_sha1_core(&key).unwrap();
            for tag_length in [4, 10, 20] {
                let raw = hmac_sha1_tag(&key, &data, &roc.to_be_bytes(), tag_length).unwrap();
                let keyed = hmac_sha1_tag_with(
                    HmacSha1Key::Keyed(&core),
                    &data,
                    &roc.to_be_bytes(),
                    tag_length,
                )
                .unwrap();
                assert_eq!(keyed.as_slice(), raw.as_slice(), "round {round}");
            }
            let raw = hmac_sha1_tag(&key, &data, &[], 21).map(|tag| tag.as_slice().to_vec());
            let keyed = hmac_sha1_tag_with(HmacSha1Key::Keyed(&core), &data, &[], 21)
                .map(|tag| tag.as_slice().to_vec());
            assert_eq!(format!("{keyed:?}"), format!("{raw:?}"));
        }
    }

    /// Independent vectors (Python `hmac`/`hashlib` over the packet followed
    /// by the big-endian ROC) for the cached keyed state. They cover empty
    /// input, SHA-1 block and padding boundaries, and ordinary and large
    /// packets, for both negotiated tag sizes.
    #[test]
    fn keyed_tag_matches_independent_hmac_vectors() {
        let vectors = [
            (0, 0, "82b98e6f0074333226dc7576871d0d3cd5dd8d0f"),
            (55, 1, "5923beac82dc3bcafaad8cbdac9d4a175941bbea"),
            (60, 0x0102_0304, "16322449ae2fae1da080602aff42e4b0f381af24"),
            (64, u32::MAX, "46a5ad3e4bfad35e3df08eb73fe9f8f545270777"),
            (160, 7, "4095ba5ebacb5918e3dd27e87d77e8d90ea834d7"),
            (1200, 65536, "02b986d5976d8009a64b7528a5a8594ccb6e82ac"),
        ];
        for (algorithm, tag_length) in [
            (SrtpAuthenticationAlgorithm::HmacSha1_80, 10),
            (SrtpAuthenticationAlgorithm::HmacSha1_32, 4),
        ] {
            let auth = SrtpAuthenticator::new(algorithm, (0..20).collect(), tag_length);
            for (len, roc, expected_hex) in vectors {
                let packet: Vec<u8> = (0..len).map(|n| n as u8).collect();
                let expected: Vec<u8> = (0..tag_length)
                    .map(|i| u8::from_str_radix(&expected_hex[2 * i..2 * i + 2], 16).unwrap())
                    .collect();
                assert_eq!(auth.calculate_auth_tag(&packet, roc).unwrap(), expected);
                assert!(auth.verify_auth_tag(&packet, &expected, roc).unwrap());
                assert!(!auth.verify_auth_tag(&packet, &expected, roc ^ 1).unwrap());
                let mut changed_packet = packet.clone();
                changed_packet.push(0);
                assert!(!auth
                    .verify_auth_tag(&changed_packet, &expected, roc)
                    .unwrap());
                let mut changed_tag = expected;
                changed_tag[0] ^= 1;
                assert!(!auth.verify_auth_tag(&packet, &changed_tag, roc).unwrap());
            }
        }
    }

    /// The cached state gives the same tag as keying HMAC from scratch for
    /// every packet, and reusing it carries no state between packets.
    #[test]
    fn keyed_tag_matches_one_shot_hmac_and_is_reusable() {
        let key: Vec<u8> = (0..20).map(|n| (n * 7 + 3) as u8).collect();
        let auth =
            SrtpAuthenticator::new(SrtpAuthenticationAlgorithm::HmacSha1_80, key.clone(), 10);
        let clone = auth.clone();
        for round in 0..3_usize {
            for len in [0_usize, 1, 12, 63, 64, 65, 172, 1500] {
                let packet: Vec<u8> = (0..len).map(|n| (n * 31 + round) as u8).collect();
                let roc = ((len as u32) << 8) | round as u32;
                let mut data = packet.clone();
                data.extend_from_slice(&roc.to_be_bytes());
                let mut one_shot = HmacSha1::new_from_slice(&key).unwrap();
                one_shot.update(&data);
                let expected = one_shot.finalize().into_bytes()[..10].to_vec();
                assert_eq!(auth.calculate_auth_tag(&packet, roc).unwrap(), expected);
                assert_eq!(clone.calculate_auth_tag(&packet, roc).unwrap(), expected);
            }
        }
        assert!(!format!("{auth:?}").contains("keyed_mac"));
    }

    /// The allocation-free twins of the executor v2 paths use the same
    /// cached keyed state and agree with the allocating functions.
    #[test]
    fn in_place_twins_match_the_allocating_path_with_the_keyed_state() {
        use rand::Rng;
        let key: Vec<u8> = (0..20).map(|_| rand::thread_rng().gen()).collect();
        for (algorithm, tag_length) in [
            (SrtpAuthenticationAlgorithm::HmacSha1_80, 10),
            (SrtpAuthenticationAlgorithm::HmacSha1_32, 4),
            (SrtpAuthenticationAlgorithm::Null, 0),
        ] {
            let auth = SrtpAuthenticator::new(algorithm, key.clone(), tag_length);
            for len in [0_usize, 12, 63, 64, 65, 172, 1500] {
                let packet: Vec<u8> = (0..len).map(|n| (n * 13 + 5) as u8).collect();
                let roc = (len as u32) << 4 | 3;
                let expected = auth.calculate_auth_tag(&packet, roc).unwrap();

                let mut buffer = vec![0xEE; 7];
                buffer.extend_from_slice(&packet);
                auth.append_auth_tag(&mut buffer, 7, roc).unwrap();
                assert_eq!(&buffer[7 + len..], expected.as_slice());
                assert_eq!(&buffer[7..7 + len], packet.as_slice());

                assert!(auth
                    .verify_auth_tag_in_place(&packet, &expected, roc)
                    .unwrap());
                if algorithm != SrtpAuthenticationAlgorithm::Null {
                    let mut changed = expected.clone();
                    changed[0] ^= 1;
                    assert!(!auth
                        .verify_auth_tag_in_place(&packet, &changed, roc)
                        .unwrap());
                    assert!(!auth
                        .verify_auth_tag_in_place(&packet, &expected, roc ^ 1)
                        .unwrap());
                }
            }
        }
    }

    #[test]
    fn test_null_authentication() {
        let auth = SrtpAuthenticator::new(SrtpAuthenticationAlgorithm::Null, Vec::new(), 0);

        // Null authentication should return empty tag
        let tag = auth.calculate_auth_tag(&[0, 1, 2, 3], 0).unwrap();
        assert!(tag.is_empty());

        // Verification should always succeed
        let result = auth.verify_auth_tag(&[0, 1, 2, 3], &[], 0).unwrap();
        assert!(result);
    }

    #[test]
    fn test_hmac_authentication() {
        let auth = SrtpAuthenticator::new(
            SrtpAuthenticationAlgorithm::HmacSha1_80,
            vec![0; 20], // 20-byte key
            10,          // 10-byte tag (80 bits)
        );

        // Calculate a tag
        let tag = auth.calculate_auth_tag(&[0, 1, 2, 3], 0).unwrap();
        assert_eq!(tag.len(), 10);

        // Verification should succeed with the same tag
        let result = auth.verify_auth_tag(&[0, 1, 2, 3], &tag, 0).unwrap();
        assert!(result);

        // Verification should fail with a different tag
        let wrong_tag = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let result = auth.verify_auth_tag(&[0, 1, 2, 3], &wrong_tag, 0).unwrap();
        assert!(!result);

        // Test with different ROC values
        let tag1 = auth.calculate_auth_tag(&[0, 1, 2, 3], 0).unwrap();
        let tag2 = auth.calculate_auth_tag(&[0, 1, 2, 3], 1).unwrap();

        // Tags should be different for different ROC values
        assert_ne!(tag1, tag2);

        // Test with HMAC-SHA1-32
        let auth32 = SrtpAuthenticator::new(
            SrtpAuthenticationAlgorithm::HmacSha1_32,
            vec![0; 20], // 20-byte key
            4,           // 4-byte tag (32 bits)
        );

        // Calculate a tag
        let tag32 = auth32.calculate_auth_tag(&[0, 1, 2, 3], 0).unwrap();
        assert_eq!(tag32.len(), 4);

        // First 4 bytes should match the HMAC-SHA1-80 tag
        assert_eq!(tag32, tag1[0..4]);
    }

    #[test]
    fn test_replay_protection() {
        // Create a custom implementation for testing
        struct TestReplayProtection {
            highest_seq: u64,
            seen_packets: Vec<u64>,
        }

        impl TestReplayProtection {
            fn new() -> Self {
                Self {
                    highest_seq: 0,
                    seen_packets: Vec::new(),
                }
            }

            fn check(&mut self, seq: u64) -> bool {
                // First packet always accepted
                if self.highest_seq == 0 {
                    self.highest_seq = seq;
                    self.seen_packets.push(seq);
                    return true;
                }

                // Check if this is a duplicate
                if self.seen_packets.contains(&seq) {
                    return false;
                }

                // Check if packet is too old (outside window)
                if seq + 64 <= self.highest_seq {
                    return false;
                }

                // If higher sequence, update highest and add to seen
                if seq > self.highest_seq {
                    // If much higher, clear old packets
                    if seq >= self.highest_seq + 64 {
                        self.seen_packets.clear();
                    }
                    self.highest_seq = seq;
                }

                // Record packet as seen
                self.seen_packets.push(seq);
                true
            }
        }

        // Run the test with our special implementation
        let mut replay = TestReplayProtection::new();

        // First packet should always be accepted
        assert!(replay.check(1000));
        assert_eq!(replay.highest_seq, 1000);

        // Duplicate packet should be rejected
        assert!(!replay.check(1000));

        // Higher sequence should be accepted
        assert!(replay.check(1001));
        assert_eq!(replay.highest_seq, 1001);

        // Out of order but within window should be accepted if not seen before
        assert!(replay.check(999));

        // Already seen packet should be rejected, even if in window
        assert!(!replay.check(999));

        // Too old (outside window) should be rejected
        assert!(!replay.check(900));

        // Much higher sequence should be accepted and reset window
        assert!(replay.check(2000));
        assert_eq!(replay.highest_seq, 2000);

        // Now old packets in previous window should be rejected
        assert!(!replay.check(1000));
    }

    #[test]
    fn test_real_replay_protection() {
        // Create a replay protection with a small window size for easier testing
        let mut replay = SrtpReplayProtection::new(16);

        // First packet should always be accepted
        assert!(replay.check(100).unwrap());
        assert_eq!(replay.highest_seq, Some(100));

        // Duplicate packet should be rejected
        assert!(!replay.check(100).unwrap());

        // Higher sequence should be accepted
        assert!(replay.check(101).unwrap());
        assert_eq!(replay.highest_seq, Some(101));

        // Lower but still in window should be accepted (if not seen before)
        assert!(replay.check(90).unwrap());

        // Same lower packet should be rejected (duplicate)
        assert!(!replay.check(90).unwrap());

        // Jump ahead to force window shift
        assert!(replay.check(200).unwrap());
        assert_eq!(replay.highest_seq, Some(200));

        // Old packet should be rejected (outside window)
        assert!(!replay.check(90).unwrap());

        // Disable replay protection
        replay.set_enabled(false);

        // With protection disabled, duplicates should be accepted
        assert!(replay.check(200).unwrap());

        // Re-enable and reset
        replay.set_enabled(true);
        replay.reset();

        // After reset, no highest sequence should be recorded.
        assert_eq!(replay.highest_seq, None);

        // Should accept a new first packet
        assert!(replay.check(300).unwrap());
    }

    #[test]
    fn test_real_replay_protection_basic() {
        // Create a replay protection with a small window size for easier testing
        let mut replay = SrtpReplayProtection::new(16);
        println!("TEST: Created replay protection with window size 16");

        // First packet should always be accepted
        println!("TEST: Checking first packet seq=100");
        assert!(replay.check(100).unwrap());
        assert_eq!(replay.highest_seq, Some(100));
        println!("TEST: First packet accepted, highest_seq=100");

        // Duplicate packet should be rejected
        println!("TEST: Checking duplicate packet seq=100");
        assert!(!replay.check(100).unwrap());
        println!("TEST: Duplicate packet rejected");

        // Higher sequence should be accepted
        println!("TEST: Checking higher sequence seq=101");
        assert!(replay.check(101).unwrap());
        assert_eq!(replay.highest_seq, Some(101));
        println!("TEST: Higher sequence accepted, highest_seq=101");

        // Jump ahead to force window shift
        println!("TEST: Jumping ahead to seq=200");
        assert!(replay.check(200).unwrap());
        assert_eq!(replay.highest_seq, Some(200));
        println!("TEST: Jump accepted, highest_seq=200");

        // Old packet should be rejected (outside window)
        println!("TEST: Checking old packet seq=100 (should be rejected)");
        let result = replay.check(100).unwrap();
        println!("TEST: Old packet check result: {}", result);
        assert!(
            !result,
            "Old packet (seq=100) should be rejected when highest_seq=200 with window_size=16"
        );
        println!("TEST: Old packet rejected successfully");

        // Disable replay protection
        println!("TEST: Disabling replay protection");
        replay.set_enabled(false);

        // With protection disabled, duplicates should be accepted
        println!("TEST: Checking duplicate with protection disabled");
        assert!(replay.check(200).unwrap());
        println!("TEST: Duplicate accepted with protection disabled");

        // Re-enable and reset
        println!("TEST: Re-enabling protection and resetting");
        replay.set_enabled(true);
        replay.reset();

        // After reset, no highest sequence should be recorded.
        assert_eq!(replay.highest_seq, None);
        println!("TEST: After reset, highest_seq=None");

        // Should accept a new first packet
        println!("TEST: Checking new packet after reset");
        assert!(replay.check(300).unwrap());
        println!("TEST: New packet accepted after reset");
    }

    #[test]
    fn replay_bitmap_tracks_zero_advances_duplicates_and_age() {
        let mut replay = SrtpReplayProtection::new(4);

        assert!(replay.check(0).unwrap(), "packet index zero is valid");
        assert!(!replay.check(0).unwrap(), "zero must not be a sentinel");

        assert!(replay.check(1).unwrap());
        assert!(
            !replay.check(0).unwrap(),
            "advancing the window must retain the preceding packet bit"
        );

        assert!(replay.check(3).unwrap());
        assert!(replay.check(2).unwrap(), "unseen in-window packet is valid");
        assert!(!replay.check(2).unwrap(), "in-window duplicate is rejected");

        assert!(replay.check(4).unwrap());
        assert!(
            !replay.check(0).unwrap(),
            "packet outside window is rejected"
        );

        let mut minimum_window = SrtpReplayProtection::new(0);
        assert!(minimum_window.check(0).unwrap());
        assert!(!minimum_window.check(0).unwrap());
        assert!(minimum_window.check(1).unwrap());
        assert!(!minimum_window.check(0).unwrap());
    }
}
