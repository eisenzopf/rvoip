use serde::{Deserialize, Serialize};

/// MOQT transport draft supported by this rvoip-moq release.
pub const MOQT_DRAFT_NUMBER: u16 = 19;
/// MSF draft supported by this rvoip-moq release.
pub const MSF_DRAFT_NUMBER: u16 = 1;
/// LOC draft supported by this rvoip-moq release.
pub const LOC_DRAFT_NUMBER: u16 = 3;

pub const MOQT_DRAFT: &str = "draft-ietf-moq-transport-19";
/// Compatibility alias retained for callers which previously distinguished
/// the runtime draft from the target draft. They are now intentionally equal.
pub const TARGET_MOQT_DRAFT: &str = MOQT_DRAFT;
pub const MSF_DRAFT: &str = "draft-ietf-moq-msf-01";
pub const LOC_DRAFT: &str = "draft-ietf-moq-loc-03";

/// Complete protocol tuple declared for one MOQT publication.
///
/// The MOQT transport version is negotiated by the wire session. MSF and LOC
/// versions are configured by the publisher and declared to subscribers; they
/// are included here because a matching transport draft does not imply a
/// compatible catalog or object format.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoqProtocolVersion {
    pub transport: u16,
    pub msf: u16,
    pub loc: u16,
}

impl MoqProtocolVersion {
    /// The sole production profile supported by Bridgefu 1.0.
    pub const PINNED: Self = Self {
        transport: MOQT_DRAFT_NUMBER,
        msf: MSF_DRAFT_NUMBER,
        loc: LOC_DRAFT_NUMBER,
    };

    pub const fn new(transport: u16, msf: u16, loc: u16) -> Self {
        Self {
            transport,
            msf,
            loc,
        }
    }

    pub fn transport_draft(self) -> String {
        format!("draft-ietf-moq-transport-{:02}", self.transport)
    }

    pub fn msf_draft(self) -> String {
        format!("draft-ietf-moq-msf-{:02}", self.msf)
    }

    pub fn loc_draft(self) -> String {
        format!("draft-ietf-moq-loc-{:02}", self.loc)
    }
}

impl Default for MoqProtocolVersion {
    fn default() -> Self {
        Self::PINNED
    }
}

impl std::fmt::Display for MoqProtocolVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}; {}; {}",
            self.transport_draft(),
            self.msf_draft(),
            self.loc_draft()
        )
    }
}

/// Exact compatibility policy for this release.
///
/// rvoip deliberately supports one production draft tuple at a time. An
/// incompatible declaration is rejected rather than silently downgraded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoqCompatibility {
    pub supported: MoqProtocolVersion,
}

impl MoqCompatibility {
    pub const PINNED: Self = Self {
        supported: MoqProtocolVersion::PINNED,
    };

    pub const fn new(supported: MoqProtocolVersion) -> Self {
        Self { supported }
    }

    pub fn require(
        &self,
        offered: MoqProtocolVersion,
    ) -> Result<MoqProtocolVersion, MoqCompatibilityError> {
        if offered.transport != self.supported.transport {
            return Err(MoqCompatibilityError::Transport {
                supported: self.supported.transport,
                offered: offered.transport,
            });
        }
        if offered.msf != self.supported.msf {
            return Err(MoqCompatibilityError::Msf {
                supported: self.supported.msf,
                offered: offered.msf,
            });
        }
        if offered.loc != self.supported.loc {
            return Err(MoqCompatibilityError::Loc {
                supported: self.supported.loc,
                offered: offered.loc,
            });
        }
        Ok(offered)
    }

    pub fn supports(&self, offered: MoqProtocolVersion) -> bool {
        self.require(offered).is_ok()
    }
}

impl Default for MoqCompatibility {
    fn default() -> Self {
        Self::PINNED
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqCompatibilityError {
    #[error("incompatible MOQT transport draft: supported -{supported:02}, offered -{offered:02}")]
    Transport { supported: u16, offered: u16 },
    #[error("incompatible MSF draft: supported -{supported:02}, offered -{offered:02}")]
    Msf { supported: u16, offered: u16 },
    #[error("incompatible LOC draft: supported -{supported:02}, offered -{offered:02}")]
    Loc { supported: u16, offered: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_profile_is_authoritative_and_serializable() {
        let version = MoqProtocolVersion::PINNED;
        assert_eq!(
            version.to_string(),
            format!("{MOQT_DRAFT}; {MSF_DRAFT}; {LOC_DRAFT}")
        );
        assert_eq!(serde_json::to_value(version).unwrap()["transport"], 19);
        assert!(MoqCompatibility::PINNED.supports(version));
    }

    #[test]
    fn each_incompatible_layer_has_an_explicit_error() {
        assert_eq!(
            MoqCompatibility::PINNED
                .require(MoqProtocolVersion::new(18, 1, 3))
                .unwrap_err(),
            MoqCompatibilityError::Transport {
                supported: 19,
                offered: 18
            }
        );
        assert!(matches!(
            MoqCompatibility::PINNED.require(MoqProtocolVersion::new(19, 2, 3)),
            Err(MoqCompatibilityError::Msf { .. })
        ));
        assert!(matches!(
            MoqCompatibility::PINNED.require(MoqProtocolVersion::new(19, 1, 4)),
            Err(MoqCompatibilityError::Loc { .. })
        ));
    }
}
