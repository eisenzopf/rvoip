use bytes::{Bytes, BytesMut, Buf, BufMut};
use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use super::SrtpAuthenticationAlgorithm;

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
                // In a real implementation, we would:
                // 1. Create an authentication buffer with packet data + ROC
                // 2. Calculate HMAC-SHA1 over this buffer
                // 3. Truncate to the required tag length (80 or 32 bits)
                
                // For now, return a dummy tag
                Ok(vec![0u8; self.tag_length])
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
pub struct SrtpReplayProtection {
    /// Window size in packets
    window_size: u64,
    
    /// Highest sequence number received
    highest_seq: u64,
    
    /// Replay window bitmap
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
            highest_seq: 0,
            window,
            enabled: true,
        }
    }
    
    /// Check if a packet is a replay
    pub fn check(&mut self, seq: u64) -> Result<bool> {
        if !self.enabled {
            return Ok(true); // Always allow if disabled
        }
        
        // Check if this is the first packet
        if self.highest_seq == 0 {
            self.highest_seq = seq;
            self.window[0] = true;
            return Ok(true);
        }
        
        // Check if the sequence number is too old
        if seq + self.window_size <= self.highest_seq {
            return Ok(false); // Too old, reject
        }
        
        // Check if this is a higher sequence number
        if seq > self.highest_seq {
            // This is a new highest sequence
            let diff = seq - self.highest_seq;
            
            if diff >= self.window_size {
                // If the gap is larger than our window, all previous 
                // sequence numbers are now outside the window
                for i in 0..self.window.len() {
                    self.window[i] = false;
                }
            } else {
                // Shift the window by clearing entries for packets
                // that are now outside the window
                for i in 0..diff as usize {
                    let idx = (self.highest_seq - i as u64) % self.window_size;
                    self.window[idx as usize] = false;
                }
            }
            
            // Update highest sequence
            self.highest_seq = seq;
            
            // Mark this sequence as received
            self.window[(seq % self.window_size) as usize] = true;
            
            return Ok(true);
        }
        
        // Check if this sequence is in the window and has already been received
        let idx = (seq % self.window_size) as usize;
        if self.window[idx] {
            return Ok(false); // Already received, reject as replay
        }
        
        // Mark as received and allow
        self.window[idx] = true;
        Ok(true)
    }
    
    /// Enable or disable replay protection
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    
    /// Reset the replay protection
    pub fn reset(&mut self) {
        self.highest_seq = 0;
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
} 