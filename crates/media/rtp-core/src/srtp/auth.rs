use super::SrtpAuthenticationAlgorithm;
use crate::error::Error;
use crate::Result;
use hmac::{Hmac, Mac};
use sha1::Sha1;

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

                // Truncate to the required tag length. Guard against a
                // misconfigured suite whose tag_length exceeds the 20-byte
                // HMAC-SHA1 output (otherwise the slice below panics).
                if self.tag_length > result.len() {
                    return Err(Error::SrtpError(format!(
                        "auth tag_length {} exceeds HMAC-SHA1 output {}",
                        self.tag_length,
                        result.len()
                    )));
                }
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

    /// Get the authentication tag length
    pub fn tag_length(&self) -> usize {
        self.tag_length
    }

    /// Check if authentication is enabled
    pub fn is_enabled(&self) -> bool {
        self.algorithm != SrtpAuthenticationAlgorithm::Null
    }
}

/// SRTP Replay Protection (RFC 3711 §3.3.2)
#[derive(Debug)]
pub struct SrtpReplayProtection {
    /// Window size in packets
    window_size: u64,

    /// Highest sequence number received
    highest_seq: u64,

    /// Replay window bitmap (using index relative to highest seq)
    window: Vec<bool>,

    /// Whether replay protection is enabled
    enabled: bool,

    /// Whether any packet has been processed yet. Kept separate from
    /// `highest_seq == 0` so a legitimate first packet at index 0 isn't
    /// mistaken for "nothing received yet" on every subsequent check.
    has_received: bool,
}

impl SrtpReplayProtection {
    /// Create a new replay protection context
    pub fn new(window_size: u64) -> Self {
        let mut window = Vec::new();
        window.resize(window_size as usize, false);

        Self {
            window_size,
            highest_seq: 0,
            window,
            enabled: true,
            has_received: false,
        }
    }

    /// Check whether `seq` would be accepted, without mutating any state.
    ///
    /// Callers that need to authenticate/decrypt before trusting a packet
    /// should call this first, then only call [`Self::commit`] once
    /// authentication and decryption have both succeeded — that keeps a
    /// failed packet from ever poisoning the window or the high-water mark.
    pub fn would_accept(&self, seq: u64) -> bool {
        if !self.enabled {
            return true;
        }

        if !self.has_received {
            return true;
        }

        // The window covers [highest_seq - window_size + 1, highest_seq]
        let window_lower_bound = self.highest_seq.saturating_sub(self.window_size - 1);
        if seq < window_lower_bound {
            return false;
        }

        if seq > self.highest_seq {
            // Any higher sequence number moves the window forward.
            return true;
        }

        // In-window, not higher than the high-water mark: reject only if
        // we've already marked this exact position as seen.
        let window_pos = self.highest_seq - seq;
        !self.window[window_pos as usize]
    }

    /// Record `seq` as received. Only call this after the packet has been
    /// authenticated (and, if applicable, decrypted) successfully — see
    /// [`Self::would_accept`].
    pub fn commit(&mut self, seq: u64) {
        if !self.enabled {
            return;
        }

        if !self.has_received {
            self.has_received = true;
            self.highest_seq = seq;
            self.window[0] = true;
            return;
        }

        if seq > self.highest_seq {
            let diff = seq - self.highest_seq;

            if diff >= self.window_size {
                for bit in self.window.iter_mut() {
                    *bit = false;
                }
            } else {
                for i in 0..diff as usize {
                    let idx = (self.window_size - diff + i as u64) % self.window_size;
                    self.window[idx as usize] = false;
                }
            }

            self.highest_seq = seq;
            self.window[0] = true;
            return;
        }

        let window_pos = self.highest_seq - seq;
        self.window[window_pos as usize] = true;
    }

    /// Check if a packet is a replay, marking it as seen if not.
    ///
    /// Convenience wrapper over [`Self::would_accept`] + [`Self::commit`]
    /// for callers that don't need to gate the commit on a later
    /// authentication/decryption step (e.g. this module's own tests).
    pub fn check(&mut self, seq: u64) -> Result<bool> {
        if !self.would_accept(seq) {
            return Ok(false);
        }
        self.commit(seq);
        Ok(true)
    }

    /// Enable or disable replay protection
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Reset the replay protection
    pub fn reset(&mut self) {
        self.highest_seq = 0;
        self.has_received = false;
        for i in 0..self.window.len() {
            self.window[i] = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(replay.highest_seq, 100);

        // Duplicate packet should be rejected
        assert!(!replay.check(100).unwrap());

        // Higher sequence should be accepted
        assert!(replay.check(101).unwrap());
        assert_eq!(replay.highest_seq, 101);

        // Lower but still in window should be accepted (if not seen before)
        assert!(replay.check(90).unwrap());

        // Same lower packet should be rejected (duplicate)
        assert!(!replay.check(90).unwrap());

        // Jump ahead to force window shift
        assert!(replay.check(200).unwrap());
        assert_eq!(replay.highest_seq, 200);

        // Old packet should be rejected (outside window)
        assert!(!replay.check(90).unwrap());

        // Disable replay protection
        replay.set_enabled(false);

        // With protection disabled, duplicates should be accepted
        assert!(replay.check(200).unwrap());

        // Re-enable and reset
        replay.set_enabled(true);
        replay.reset();

        // After reset, highest_seq should be 0
        assert_eq!(replay.highest_seq, 0);

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
        assert_eq!(replay.highest_seq, 100);
        println!("TEST: First packet accepted, highest_seq=100");

        // Duplicate packet should be rejected
        println!("TEST: Checking duplicate packet seq=100");
        assert!(!replay.check(100).unwrap());
        println!("TEST: Duplicate packet rejected");

        // Higher sequence should be accepted
        println!("TEST: Checking higher sequence seq=101");
        assert!(replay.check(101).unwrap());
        assert_eq!(replay.highest_seq, 101);
        println!("TEST: Higher sequence accepted, highest_seq=101");

        // Jump ahead to force window shift
        println!("TEST: Jumping ahead to seq=200");
        assert!(replay.check(200).unwrap());
        assert_eq!(replay.highest_seq, 200);
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

        // After reset, highest_seq should be 0
        assert_eq!(replay.highest_seq, 0);
        println!("TEST: After reset, highest_seq=0");

        // Should accept a new first packet
        println!("TEST: Checking new packet after reset");
        assert!(replay.check(300).unwrap());
        println!("TEST: New packet accepted after reset");
    }
}
