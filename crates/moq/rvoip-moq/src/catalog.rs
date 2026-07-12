use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{MoqNamespace, AUDIO_TRACK, OPUS_SAMPLE_RATE};

/// Catalog version value required by MSF draft-01 section 5.1.1.
pub const MSF_CATALOG_VERSION: &str = "draft-01";
const CANONICAL_PACKAGING: &str = "loc";
const CANONICAL_ROLE: &str = "audio";
const CANONICAL_CODEC: &str = "opus";
const CANONICAL_CHANNEL_CONFIG: &str = "mono";

/// Minimal independent MSF-01 catalog used by an rvoip audio publication.
///
/// Fields are private so callers cannot construct a catalog that advertises a
/// different media profile than the LOC packetizer actually emits. JSON
/// deserialization performs the same validation as the constructor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsfCatalog {
    version: String,
    generated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_complete: Option<bool>,
    tracks: Vec<MsfTrack>,
}

/// MSF-01 description of the canonical Opus audio track.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsfTrack {
    namespace: MoqNamespace,
    name: String,
    packaging: String,
    is_live: bool,
    role: String,
    codec: String,
    timescale: u32,
    /// Maximum encoded bitrate in bits per second (MSF-01 `bitrate`).
    bitrate: u32,
    samplerate: u32,
    channel_config: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<String>,
}

/// The two catalog shapes allowed by the Bridgefu MSF-01 profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MsfCatalogState {
    /// A live catalog omits `isComplete` and advertises exactly one audio track.
    Live,
    /// A permanently completed catalog sets `isComplete` to true and has no tracks.
    PermanentlyCompleted,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsfCatalogWire {
    version: String,
    generated_at: i64,
    is_complete: Option<bool>,
    tracks: Vec<MsfTrackWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MsfTrackWire {
    namespace: MoqNamespace,
    name: String,
    packaging: String,
    is_live: bool,
    role: String,
    codec: String,
    timescale: u32,
    bitrate: u32,
    samplerate: u32,
    channel_config: String,
    lang: Option<String>,
}

impl<'de> Deserialize<'de> for MsfCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MsfCatalogWire::deserialize(deserializer)?;
        let catalog = Self {
            version: wire.version,
            generated_at: wire.generated_at,
            is_complete: wire.is_complete,
            tracks: wire.tracks.into_iter().map(MsfTrack::from).collect(),
        };
        catalog.validate().map_err(serde::de::Error::custom)?;
        Ok(catalog)
    }
}

impl From<MsfTrackWire> for MsfTrack {
    fn from(wire: MsfTrackWire) -> Self {
        Self {
            namespace: wire.namespace,
            name: wire.name,
            packaging: wire.packaging,
            is_live: wire.is_live,
            role: wire.role,
            codec: wire.codec,
            timescale: wire.timescale,
            bitrate: wire.bitrate,
            samplerate: wire.samplerate,
            channel_config: wire.channel_config,
            lang: wire.lang,
        }
    }
}

impl MsfCatalog {
    pub fn opus_audio(
        namespace: &MoqNamespace,
        bitrate: u32,
        language: Option<String>,
        generated_at: i64,
    ) -> Result<Self, MsfCatalogError> {
        let catalog = Self {
            version: MSF_CATALOG_VERSION.to_owned(),
            generated_at,
            // MSF-01 requires this field to be omitted while the live
            // publication is still capable of adding content.
            is_complete: None,
            tracks: vec![MsfTrack {
                namespace: namespace.clone(),
                name: AUDIO_TRACK.to_owned(),
                packaging: CANONICAL_PACKAGING.to_owned(),
                is_live: true,
                role: CANONICAL_ROLE.to_owned(),
                codec: CANONICAL_CODEC.to_owned(),
                timescale: OPUS_SAMPLE_RATE,
                bitrate,
                samplerate: OPUS_SAMPLE_RATE,
                channel_config: CANONICAL_CHANNEL_CONFIG.to_owned(),
                lang: language,
            }],
        };
        catalog.validate_for(namespace)?;
        Ok(catalog)
    }

    /// Construct the final MSF-01 catalog update for a cleanly ended publication.
    pub fn permanently_completed(generated_at: i64) -> Self {
        Self {
            version: MSF_CATALOG_VERSION.to_owned(),
            generated_at,
            is_complete: Some(true),
            tracks: Vec::new(),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub const fn generated_at(&self) -> i64 {
        self.generated_at
    }

    pub const fn is_complete(&self) -> Option<bool> {
        self.is_complete
    }

    pub fn tracks(&self) -> &[MsfTrack] {
        &self.tracks
    }

    pub const fn state(&self) -> MsfCatalogState {
        match self.is_complete {
            None => MsfCatalogState::Live,
            Some(true) => MsfCatalogState::PermanentlyCompleted,
            // Invalid catalogs cannot be constructed through public APIs or
            // deserialized, but retain a deterministic value for this total
            // accessor if the invariant is violated inside this module.
            Some(false) => MsfCatalogState::Live,
        }
    }

    /// Validate one of the two canonical Bridgefu 1.0 catalog states.
    pub fn validate(&self) -> Result<(), MsfCatalogError> {
        if self.version != MSF_CATALOG_VERSION {
            return Err(MsfCatalogError::UnsupportedVersion {
                offered: self.version.clone(),
            });
        }
        match (self.is_complete, self.tracks.len()) {
            (None, 1) => self.tracks[0].validate(),
            (None, 0) => Err(MsfCatalogError::MissingTracks),
            (None, actual) => Err(MsfCatalogError::UnexpectedTrackCount { actual }),
            (Some(false), _) => Err(MsfCatalogError::ExplicitIncompleteForbidden),
            (Some(true), 0) => Ok(()),
            (Some(true), actual) => Err(MsfCatalogError::CompletedCatalogHasTracks { actual }),
        }
    }

    /// Validate the canonical profile and bind its track to the expected
    /// publication namespace.
    pub fn validate_for(&self, namespace: &MoqNamespace) -> Result<(), MsfCatalogError> {
        self.validate()?;
        if let Some(track) = self.tracks.first() {
            let actual = &track.namespace;
            if actual != namespace {
                return Err(MsfCatalogError::NamespaceMismatch {
                    expected: namespace.to_string(),
                    actual: actual.to_string(),
                });
            }
        }
        Ok(())
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

impl MsfTrack {
    fn validate(&self) -> Result<(), MsfCatalogError> {
        require_value("name", &self.name, AUDIO_TRACK)?;
        require_value("packaging", &self.packaging, CANONICAL_PACKAGING)?;
        if !self.is_live {
            return Err(MsfCatalogError::TrackMustBeLive);
        }
        require_value("role", &self.role, CANONICAL_ROLE)?;
        require_value("codec", &self.codec, CANONICAL_CODEC)?;
        require_u32("timescale", self.timescale, OPUS_SAMPLE_RATE)?;
        if self.bitrate == 0 {
            return Err(MsfCatalogError::ZeroBitrate);
        }
        require_u32("samplerate", self.samplerate, OPUS_SAMPLE_RATE)?;
        require_value(
            "channelConfig",
            &self.channel_config,
            CANONICAL_CHANNEL_CONFIG,
        )?;
        if let Some(language) = self.lang.as_deref() {
            if !is_well_formed_bcp47(language) {
                return Err(MsfCatalogError::InvalidLanguage {
                    value: language.to_owned(),
                });
            }
        }
        Ok(())
    }

    pub fn namespace(&self) -> &MoqNamespace {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn packaging(&self) -> &str {
        &self.packaging
    }

    pub const fn is_live(&self) -> bool {
        self.is_live
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    pub const fn bitrate(&self) -> u32 {
        self.bitrate
    }

    pub const fn samplerate(&self) -> u32 {
        self.samplerate
    }

    pub fn channel_config(&self) -> &str {
        &self.channel_config
    }

    pub fn language(&self) -> Option<&str> {
        self.lang.as_deref()
    }
}

fn require_value(
    field: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<(), MsfCatalogError> {
    if actual == expected {
        Ok(())
    } else {
        Err(MsfCatalogError::InvalidCanonicalField {
            field,
            expected,
            actual: actual.to_owned(),
        })
    }
}

fn require_u32(field: &'static str, actual: u32, expected: u32) -> Result<(), MsfCatalogError> {
    if actual == expected {
        Ok(())
    } else {
        Err(MsfCatalogError::InvalidCanonicalNumber {
            field,
            expected,
            actual,
        })
    }
}

/// RFC 5646 structural validation for a BCP-47 language tag.
///
/// This intentionally validates syntax rather than maintaining a stale copy of
/// the IANA language-subtag registry. It covers langtag, private-use, and all
/// grandfathered forms; it also rejects duplicate variants and extension
/// singletons as required by RFC 5646 section 2.2.9.
fn is_well_formed_bcp47(value: &str) -> bool {
    if value.is_empty() || value.len() > 255 {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if GRANDFATHERED_TAGS.contains(&lower.as_str()) {
        return true;
    }

    let subtags: Vec<&str> = value.split('-').collect();
    if subtags.iter().any(|part| {
        part.is_empty() || part.len() > 8 || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }) {
        return false;
    }
    if subtags[0].eq_ignore_ascii_case("x") {
        return subtags.len() > 1;
    }

    let mut index = 0;
    let language = subtags[index];
    if !language.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return false;
    }
    match language.len() {
        2 | 3 => {
            index += 1;
            let mut extlangs = 0;
            while index < subtags.len()
                && subtags[index].len() == 3
                && subtags[index]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphabetic())
                && extlangs < 3
            {
                index += 1;
                extlangs += 1;
            }
        }
        4..=8 => index += 1,
        _ => return false,
    }

    if index < subtags.len()
        && subtags[index].len() == 4
        && subtags[index]
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic())
    {
        index += 1;
    }
    if index < subtags.len()
        && ((subtags[index].len() == 2
            && subtags[index]
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic()))
            || (subtags[index].len() == 3
                && subtags[index].bytes().all(|byte| byte.is_ascii_digit())))
    {
        index += 1;
    }

    let mut variants = HashSet::new();
    while index < subtags.len() && is_variant(subtags[index]) {
        if !variants.insert(subtags[index].to_ascii_lowercase()) {
            return false;
        }
        index += 1;
    }

    let mut extensions = HashSet::new();
    while index < subtags.len()
        && subtags[index].len() == 1
        && !subtags[index].eq_ignore_ascii_case("x")
    {
        let singleton = subtags[index].to_ascii_lowercase();
        if !extensions.insert(singleton) {
            return false;
        }
        index += 1;
        let start = index;
        while index < subtags.len()
            && (2..=8).contains(&subtags[index].len())
            && subtags[index]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        {
            index += 1;
        }
        if index == start {
            return false;
        }
    }

    if index < subtags.len() && subtags[index].eq_ignore_ascii_case("x") {
        index += 1;
        if index == subtags.len() {
            return false;
        }
        // The initial structural check already guarantees 1*8ALPHANUM.
        index = subtags.len();
    }
    index == subtags.len()
}

fn is_variant(value: &str) -> bool {
    ((5..=8).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        || (value.len() == 4
            && value.as_bytes()[0].is_ascii_digit()
            && value[1..].bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

const GRANDFATHERED_TAGS: &[&str] = &[
    "art-lojban",
    "cel-gaulish",
    "en-gb-oed",
    "i-ami",
    "i-bnn",
    "i-default",
    "i-enochian",
    "i-hak",
    "i-klingon",
    "i-lux",
    "i-mingo",
    "i-navajo",
    "i-pwn",
    "i-tao",
    "i-tay",
    "i-tsu",
    "no-bok",
    "no-nyn",
    "sgn-be-fr",
    "sgn-be-nl",
    "sgn-ch-de",
    "zh-guoyu",
    "zh-hakka",
    "zh-min",
    "zh-min-nan",
    "zh-xiang",
];

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MsfCatalogError {
    #[error("MSF catalog bitrate must be greater than zero")]
    ZeroBitrate,
    #[error("MSF catalog language is not a well-formed BCP-47 tag: {value:?}")]
    InvalidLanguage { value: String },
    #[error("unsupported MSF catalog version {offered:?}; expected draft-01")]
    UnsupportedVersion { offered: String },
    #[error("live MSF catalog must contain its audio track")]
    MissingTracks,
    #[error("canonical MSF audio catalog must contain exactly one track, got {actual}")]
    UnexpectedTrackCount { actual: usize },
    #[error("live MSF catalogs must omit isComplete instead of setting it to false")]
    ExplicitIncompleteForbidden,
    #[error("permanently completed MSF catalog must contain no tracks, got {actual}")]
    CompletedCatalogHasTracks { actual: usize },
    #[error("canonical MSF audio track must be live")]
    TrackMustBeLive,
    #[error("canonical MSF field {field} must be {expected:?}, got {actual:?}")]
    InvalidCanonicalField {
        field: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("canonical MSF field {field} must be {expected}, got {actual}")]
    InvalidCanonicalNumber {
        field: &'static str,
        expected: u32,
        actual: u32,
    },
    #[error("MSF catalog namespace must be {expected:?}, got {actual:?}")]
    NamespaceMismatch { expected: String, actual: String },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn emits_msf_01_opus_catalog_with_normative_field_names() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let catalog =
            MsfCatalog::opus_audio(&namespace, 24_000, Some("en-US".to_owned()), 1234).unwrap();
        let json = serde_json::to_value(&catalog).unwrap();

        assert_eq!(json["version"], "draft-01");
        assert_eq!(json["generatedAt"], 1234);
        assert!(json.get("isComplete").is_none());
        assert_eq!(json["tracks"][0]["namespace"], "tenant/broadcast");
        assert_eq!(json["tracks"][0]["name"], AUDIO_TRACK);
        assert_eq!(json["tracks"][0]["packaging"], "loc");
        assert_eq!(json["tracks"][0]["role"], "audio");
        assert_eq!(json["tracks"][0]["codec"], "opus");
        assert_eq!(json["tracks"][0]["timescale"], 48_000);
        assert_eq!(json["tracks"][0]["samplerate"], 48_000);
        assert_eq!(json["tracks"][0]["channelConfig"], "mono");
        assert_eq!(json["tracks"][0]["bitrate"], 24_000);
        assert_eq!(json["tracks"][0]["lang"], "en-US");
        assert!(json["tracks"][0].get("maxBitrate").is_none());
        catalog.validate_for(&namespace).unwrap();

        let decoded: MsfCatalog = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, catalog);
        assert_eq!(decoded.tracks()[0].namespace(), &namespace);
        assert_eq!(decoded.state(), MsfCatalogState::Live);
    }

    #[test]
    fn emits_the_only_valid_permanently_completed_catalog_shape() {
        let completed = MsfCatalog::permanently_completed(5678);
        let json = serde_json::to_value(&completed).unwrap();

        assert_eq!(completed.state(), MsfCatalogState::PermanentlyCompleted);
        assert_eq!(json["version"], "draft-01");
        assert_eq!(json["generatedAt"], 5678);
        assert_eq!(json["isComplete"], true);
        assert_eq!(json["tracks"], json!([]));
        completed.validate().unwrap();
        completed
            .validate_for(&MoqNamespace::new("tenant", "broadcast").unwrap())
            .unwrap();
        assert_eq!(
            serde_json::from_value::<MsfCatalog>(json).unwrap(),
            completed
        );
    }

    #[test]
    fn rejects_every_false_or_mixed_catalog_completion_shape() {
        let live_track = json!({
            "namespace": "tenant/broadcast",
            "name": "audio/main",
            "packaging": "loc",
            "isLive": true,
            "role": "audio",
            "codec": "opus",
            "timescale": 48000,
            "bitrate": 24000,
            "samplerate": 48000,
            "channelConfig": "mono"
        });
        for invalid in [
            json!({
                "version": "draft-01",
                "generatedAt": 1,
                "isComplete": false,
                "tracks": [live_track.clone()]
            }),
            json!({
                "version": "draft-01",
                "generatedAt": 1,
                "isComplete": true,
                "tracks": [live_track.clone()]
            }),
            json!({
                "version": "draft-01",
                "generatedAt": 1,
                "tracks": []
            }),
        ] {
            assert!(serde_json::from_value::<MsfCatalog>(invalid).is_err());
        }
    }

    #[test]
    fn accepts_structurally_well_formed_bcp47_forms() {
        for language in [
            "en",
            "en-US",
            "zh-Hant-TW",
            "sl-rozaj-biske-1994",
            "de-CH-1901",
            "en-US-u-ca-gregory",
            "x-acme-private",
            "i-klingon",
        ] {
            assert!(is_well_formed_bcp47(language), "{language}");
        }
    }

    #[test]
    fn rejects_malformed_bcp47_forms() {
        for language in [
            "",
            " ",
            "e",
            "en_US",
            "en--US",
            "en-a",
            "en-x",
            "en-1234-1234",
            "en-a-foo-a-bar",
        ] {
            assert!(!is_well_formed_bcp47(language), "{language}");
        }
    }

    #[test]
    fn rejects_invalid_catalog_profile_values() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        assert_eq!(
            MsfCatalog::opus_audio(&namespace, 0, None, 0).unwrap_err(),
            MsfCatalogError::ZeroBitrate
        );
        assert_eq!(
            MsfCatalog::opus_audio(&namespace, 1, Some("en_US".into()), 0).unwrap_err(),
            MsfCatalogError::InvalidLanguage {
                value: "en_US".into()
            }
        );
    }

    #[test]
    fn deserialization_cannot_bypass_canonical_profile_or_namespace_validation() {
        let valid = json!({
            "version": "draft-01",
            "generatedAt": 1234,
            "tracks": [{
                "namespace": "tenant/broadcast",
                "name": "audio/main",
                "packaging": "loc",
                "isLive": true,
                "role": "audio",
                "codec": "opus",
                "timescale": 48000,
                "bitrate": 24000,
                "samplerate": 48000,
                "channelConfig": "mono",
                "lang": "en-US"
            }]
        });
        serde_json::from_value::<MsfCatalog>(valid.clone()).unwrap();

        for (field, value) in [
            ("namespace", json!("tenant/other/invalid")),
            ("name", json!("audio/alternate")),
            ("packaging", json!("cmaf")),
            ("isLive", json!(false)),
            ("role", json!("alternate")),
            ("codec", json!("pcmu")),
            ("timescale", json!(90000)),
            ("bitrate", json!(0)),
            ("samplerate", json!(44100)),
            ("channelConfig", json!("stereo")),
            ("lang", json!("en_US")),
        ] {
            let mut invalid = valid.clone();
            invalid["tracks"][0][field] = value;
            assert!(
                serde_json::from_value::<MsfCatalog>(invalid).is_err(),
                "field {field} bypassed validation"
            );
        }
    }

    #[test]
    fn validate_for_rejects_a_different_but_valid_namespace() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let other = MoqNamespace::new("tenant", "other").unwrap();
        let catalog = MsfCatalog::opus_audio(&namespace, 24_000, None, 0).unwrap();
        assert_eq!(
            catalog.validate_for(&other).unwrap_err(),
            MsfCatalogError::NamespaceMismatch {
                expected: "tenant/other".into(),
                actual: "tenant/broadcast".into(),
            }
        );
    }
}
