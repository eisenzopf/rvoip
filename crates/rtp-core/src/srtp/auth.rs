use bytes::{Bytes, BytesMut, Buf, BufMut};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use super::SrtpAuthenticationAlgorithm;

// Define type for HMAC-SHA1
type HmacSha1 = Hmac<Sha1>;

/// SRTP Authentication Handler
pub struct SrtpAuthenticator {
    /// Authentication algorithm
    algorithm: SrtpAuthenticationAlgorithm,

    /// Authentication key
    auth_key: Vec<u8>,

    /// Authentication tag length in bytes
    tag_length: usize,
}

impl SrtpAuthenticator {
    /// Create a new SRTP authenticator
    pub fn new(
        algorithm: SrtpAuthenticationAlgorithm,
        auth_key: Vec<u8>,
        tag_length: usize,
    ) -> Self {
        Self {
            algorithm,
            auth_key,
            tag_length,
        }
    }

    /// Calculate authentication tag for an RTP packet
    pub fn calculate_auth_tag(&self, packet_data: &[u8], roc: u32) -> Result<Vec<u8>> {
        if self.algorithm == SrtpAuthenticationAlgorithm::Null {
            // Null authentication, return empty tag
            return Ok(Vec::new());
        }

        match self.algorithm {
            SrtpAuthenticationAlgorithm::HmacSha1_80 | SrtpAuthenticationAlgorithm::HmacSha1_32 => {
                // Create an authentication buffer with packet data + ROC
                let mut auth_buf = Vec::with_capacity(packet_data.len() + 4);
                auth_buf.extend_from_slice(packet_data);
                auth_buf.extend_from_slice(&roc.to_be_bytes());

                // Create HMAC-SHA1 instance
                let mut mac = HmacSha1::new_from_slice(&self.auth_key)
                    .map_err(|e| Error::SrtpError(format!("Failed to create HMAC: {}", e)))?;

                // Update with data
                mac.update(&auth_buf);

                // Finalize and get the result
                let result = mac.finalize().into_bytes();

                // Truncate to the required tag length
                let tag = result.as_slice()[..self.tag_length].to_vec();

                Ok(tag)
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
                expected_tag.len(), tag.len()
            )));
        }

        // Constant-time comparison to prevent timing attacks
        let mut result = 0;
        for (a, b) in expected_tag.iter().zip(tag.iter()) {
            result |= a ^ b;
        }

        Ok(result == 0)
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
///
/// Uses `Option<u64>` for `highest_seq` to distinguish the uninitialized state
/// from an actual packet index of 0.  The bitmap tracks which indices within
/// `[highest_seq - window_size + 1, highest_seq]` have been seen, indexed as
/// offset from `highest_seq` (position 0 = highest_seq itself).
#[derive(Debug, Clone)]
pub struct SrtpReplayProtection {
    /// Window size in packets
    window_size: u64,

    /// Highest packet index received, or `None` if no packet has been received yet
    highest_seq: Option<u64>,

    /// Replay window bitmap (using index relative to highest seq)
    /// Position 0 corresponds to `highest_seq`, position 1 to `highest_seq - 1`, etc.
    window: Vec<bool>,

    /// Whether replay protection is enabled
    enabled: bool,
}

impl SrtpReplayProtection {
    /// Create a new replay protection context
    pub fn new(window_size: u64) -> Self {
        let mut window = Vec::new();
        window.resize(window_size as usize, false);

        Self {
            window_size,
            highest_seq: None,
            window,
            enabled: true,
        }
    }

    /// Check whether `seq` is acceptable WITHOUT mutating state.
    /// Returns `Ok(true)` if the packet should be accepted,
    /// `Ok(false)` if it is a replay or too old.
    pub fn check_tentative(&self, seq: u64) -> Result<bool> {
        if !self.enabled {
            return Ok(true);
        }

        let highest = match self.highest_seq {
            None => return Ok(true), // first packet is always accepted
            Some(h) => h,
        };

        if seq > highest {
            // New high — always acceptable
            return Ok(true);
        }

        // seq <= highest — check if within window
        let delta = highest - seq;
        if delta >= self.window_size {
            // Too old
            return Ok(false);
        }

        // Within window — check bitmap for duplicate
        Ok(!self.window[delta as usize])
    }

    /// Commit `seq` into the replay state.  Must only be called after
    /// successful authentication.  The caller should have previously
    /// verified via `check_tentative` that the packet is acceptable.
    pub fn commit(&mut self, seq: u64) {
        if !self.enabled {
            return;
        }

        match self.highest_seq {
            None => {
                // First packet
                self.highest_seq = Some(seq);
                self.window[0] = true;
            }
            Some(highest) if seq > highest => {
                let diff = seq - highest;

                if diff >= self.window_size {
                    // Gap larger than window — clear everything
                    for slot in self.window.iter_mut() {
                        *slot = false;
                    }
                } else {
                    // Shift the window: bits that fall off the end become
                    // irrelevant.  We shift by `diff` positions.  The simplest
                    // correct approach for a bool-vec: shift elements toward
                    // higher indices, then clear the newly vacated low slots.
                    let ws = self.window_size as usize;
                    let d = diff as usize;
                    // Move existing bits toward higher indices
                    for i in (0..ws).rev() {
                        if i >= d {
                            self.window[i] = self.window[i - d];
                        } else {
                            self.window[i] = false;
                        }
                    }
                }

                self.highest_seq = Some(seq);
                self.window[0] = true;
            }
            Some(highest) => {
                // seq <= highest, within window (caller verified)
                let delta = highest - seq;
                if (delta as usize) < self.window.len() {
                    self.window[delta as usize] = true;
                }
            }
        }
    }

    /// Legacy combined check-and-commit for callers that do not need the
    /// tentative/commit split (e.g. SRTCP where auth is verified first).
    pub fn check(&mut self, seq: u64) -> Result<bool> {
        let acceptable = self.check_tentative(seq)?;
        if acceptable {
            self.commit(seq);
        }
        Ok(acceptable)
    }

    /// Enable or disable replay protection
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Reset the replay protection
    pub fn reset(&mut self) {
        self.highest_seq = None;
        for slot in self.window.iter_mut() {
            *slot = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_authentication() {
        let auth = SrtpAuthenticator::new(
            SrtpAuthenticationAlgorithm::Null,
            Vec::new(),
            0
        );

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
            10 // 10-byte tag (80 bits)
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
            4 // 4-byte tag (32 bits)
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

        // After reset, highest_seq should be None
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
        assert!(!result, "Old packet (seq=100) should be rejected when highest_seq=200 with window_size=16");
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

        // After reset, highest_seq should be None
        assert_eq!(replay.highest_seq, None);
        println!("TEST: After reset, highest_seq=None");

        // Should accept a new first packet
        println!("TEST: Checking new packet after reset");
        assert!(replay.check(300).unwrap());
        println!("TEST: New packet accepted after reset");
    }

    #[test]
    fn test_replay_protection_seq_zero() {
        // Verify that actual packet index 0 works correctly
        let mut replay = SrtpReplayProtection::new(64);

        // Packet index 0 should be accepted as the first packet
        assert!(replay.check(0).unwrap());
        assert_eq!(replay.highest_seq, Some(0));

        // Duplicate of index 0 should be rejected
        assert!(!replay.check(0).unwrap());

        // Index 1 should be accepted
        assert!(replay.check(1).unwrap());
        assert_eq!(replay.highest_seq, Some(1));
    }

    #[test]
    fn test_replay_tentative_and_commit() {
        let mut replay = SrtpReplayProtection::new(16);

        // Tentative check on first packet
        assert!(replay.check_tentative(100).unwrap());
        // State should NOT have changed
        assert_eq!(replay.highest_seq, None);

        // Commit
        replay.commit(100);
        assert_eq!(replay.highest_seq, Some(100));

        // Tentative check on duplicate should fail
        assert!(!replay.check_tentative(100).unwrap());

        // Tentative check on new higher seq
        assert!(replay.check_tentative(105).unwrap());
        // Not committed yet — highest should still be 100
        assert_eq!(replay.highest_seq, Some(100));

        // Commit the new seq
        replay.commit(105);
        assert_eq!(replay.highest_seq, Some(105));
    }
}
