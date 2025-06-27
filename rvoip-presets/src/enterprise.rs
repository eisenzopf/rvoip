//! Enterprise PBX and communication system presets

use crate::{DeploymentConfig, SecurityProfile, CapacityConfig, FeatureSet, SimpleVoipError};
use std::time::Duration;

/// Enterprise PBX system
pub struct EnterprisePbx {
    config: DeploymentConfig,
    domain: String,
    user_capacity: u32,
    encryption_required: bool,
}

impl EnterprisePbx {
    /// Create a new enterprise PBX system
    pub fn new(domain: impl Into<String>) -> EnterprisePbxBuilder {
        EnterprisePbxBuilder::new(domain.into())
    }
}

/// Builder for enterprise PBX configuration
pub struct EnterprisePbxBuilder {
    domain: String,
    user_capacity: u32,
    encryption_required: bool,
    high_availability: bool,
    recording_enabled: bool,
}

impl EnterprisePbxBuilder {
    fn new(domain: String) -> Self {
        Self {
            domain,
            user_capacity: 1000,
            encryption_required: true,
            high_availability: false,
            recording_enabled: false,
        }
    }

    /// Set the maximum number of users
    pub fn with_user_capacity(mut self, capacity: u32) -> Self {
        self.user_capacity = capacity;
        self
    }

    /// Require encryption for all communications
    pub fn with_encryption_required(mut self, required: bool) -> Self {
        self.encryption_required = required;
        self
    }

    /// Enable high availability with redundancy
    pub fn with_high_availability(mut self, enabled: bool) -> Self {
        self.high_availability = enabled;
        self
    }

    /// Enable call recording for compliance
    pub fn with_recording(mut self, enabled: bool) -> Self {
        self.recording_enabled = enabled;
        self
    }

    /// Start the enterprise PBX system
    pub async fn start(self) -> Result<EnterprisePbx, SimpleVoipError> {
        // TODO: Implement actual PBX startup using call-engine, sip-core, etc.
        
        let capacity = if self.user_capacity <= 1000 {
            CapacityConfig::medium()
        } else if self.user_capacity <= 10000 {
            CapacityConfig::large()
        } else {
            CapacityConfig::enterprise()
        };

        let security = if self.encryption_required {
            SecurityProfile::Enterprise
        } else {
            SecurityProfile::Standard
        };

        let features = FeatureSet {
            recording: self.recording_enabled,
            ..FeatureSet::full()
        };

        let config = DeploymentConfig {
            name: format!("Enterprise PBX - {}", self.domain),
            capacity,
            security,
            features,
            ..Default::default()
        };

        Ok(EnterprisePbx {
            config,
            domain: self.domain,
            user_capacity: self.user_capacity,
            encryption_required: self.encryption_required,
        })
    }
} 