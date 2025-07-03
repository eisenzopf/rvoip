//! ZRTP hash implementations
//!
//! This module provides hash functions used in ZRTP.

use sha2::{Sha256, Sha384, Digest};

/// ZRTP hash calculation
pub struct ZrtpHash;

impl ZrtpHash {
    /// Calculate a SHA-256 hash
    pub fn sha256(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }
    
    /// Calculate a SHA-384 hash
    pub fn sha384(data: &[u8]) -> [u8; 48] {
        let mut hasher = Sha384::new();
        hasher.update(data);
        let result = hasher.finalize();
        
        let mut hash = [0u8; 48];
        hash.copy_from_slice(&result);
        hash
    }
} 