use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    MoqNamespace, AUDIO_TRACK, EVENTS_TRACK, OPUS_SAMPLE_RATE, SANITIZED_EVENTS_EVENT_TYPE,
};

/// Catalog version value required by MSF draft-01 section 5.1.1.
pub const MSF_CATALOG_VERSION: &str = "draft-01";
const CANONICAL_PACKAGING: &str = "loc";
const CANONICAL_ROLE: &str = "audio";
const CANONICAL_CODEC: &str = "opus";
const CANONICAL_CHANNEL_CONFIG: &str = "mono";
const EVENT_TIMELINE_PACKAGING: &str = "eventtimeline";
const EVENT_TIMELINE_ROLE: &str = "eventtimeline";
const JSON_MIME_TYPE: &str = "application/json";

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

/// MSF-01 description of a canonical rvoip publication track.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsfTrack {
    namespace: MoqNamespace,
    name: String,
    packaging: String,
    is_live: bool,
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_type: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depends: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timescale: Option<u32>,
    /// Maximum encoded bitrate in bits per second (MSF-01 `bitrate`).
    #[serde(skip_serializing_if = "Option::is_none")]
    bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    samplerate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<String>,
}

/// The two catalog shapes allowed by the Bridgefu MSF-01 profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MsfCatalogState {
    /// A live catalog omits `isComplete` and advertises audio plus an optional
    /// explicitly enabled sanitized event-timeline track.
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
    event_type: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    depends: Option<Vec<String>>,
    codec: Option<String>,
    timescale: Option<u32>,
    bitrate: Option<u32>,
    samplerate: Option<u32>,
    channel_config: Option<String>,
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
            event_type: wire.event_type,
            mime_type: wire.mime_type,
            depends: wire.depends,
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
        Self::opus_audio_profile(namespace, bitrate, language, generated_at, false)
    }

    /// Construct the canonical audio catalog with the opt-in sanitized event
    /// timeline described by MSF-01 section 8.
    pub fn opus_audio_with_sanitized_events(
        namespace: &MoqNamespace,
        bitrate: u32,
        language: Option<String>,
        generated_at: i64,
    ) -> Result<Self, MsfCatalogError> {
        Self::opus_audio_profile(namespace, bitrate, language, generated_at, true)
    }

    fn opus_audio_profile(
        namespace: &MoqNamespace,
        bitrate: u32,
        language: Option<String>,
        generated_at: i64,
        sanitized_events: bool,
    ) -> Result<Self, MsfCatalogError> {
        let mut tracks = vec![MsfTrack {
            namespace: namespace.clone(),
            name: AUDIO_TRACK.to_owned(),
            packaging: CANONICAL_PACKAGING.to_owned(),
            is_live: true,
            role: CANONICAL_ROLE.to_owned(),
            event_type: None,
            mime_type: None,
            depends: None,
            codec: Some(CANONICAL_CODEC.to_owned()),
            timescale: Some(OPUS_SAMPLE_RATE),
            bitrate: Some(bitrate),
            samplerate: Some(OPUS_SAMPLE_RATE),
            channel_config: Some(CANONICAL_CHANNEL_CONFIG.to_owned()),
            lang: language,
        }];
        if sanitized_events {
            tracks.push(MsfTrack {
                namespace: namespace.clone(),
                name: EVENTS_TRACK.to_owned(),
                packaging: EVENT_TIMELINE_PACKAGING.to_owned(),
                is_live: true,
                role: EVENT_TIMELINE_ROLE.to_owned(),
                event_type: Some(SANITIZED_EVENTS_EVENT_TYPE.to_owned()),
                mime_type: Some(JSON_MIME_TYPE.to_owned()),
                depends: Some(vec![AUDIO_TRACK.to_owned()]),
                codec: None,
                timescale: None,
                bitrate: None,
                samplerate: None,
                channel_config: None,
                lang: None,
            });
        }
        let catalog = Self {
            version: MSF_CATALOG_VERSION.to_owned(),
            generated_at,
            // MSF-01 requires this field to be omitted while the live
            // publication is still capable of adding content.
            is_complete: None,
            tracks,
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
            (None, 1) => self.tracks[0].validate_audio(),
            (None, 2) => {
                self.tracks[0].validate_audio()?;
                self.tracks[1].validate_sanitized_events()
            }
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
        for track in &self.tracks {
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
    fn validate_audio(&self) -> Result<(), MsfCatalogError> {
        require_value("name", &self.name, AUDIO_TRACK)?;
        require_value("packaging", &self.packaging, CANONICAL_PACKAGING)?;
        if !self.is_live {
            return Err(MsfCatalogError::TrackMustBeLive);
        }
        require_value("role", &self.role, CANONICAL_ROLE)?;
        require_absent("eventType", self.event_type.is_some())?;
        require_absent("mimeType", self.mime_type.is_some())?;
        require_absent("depends", self.depends.is_some())?;
        require_optional_value("codec", self.codec.as_deref(), CANONICAL_CODEC)?;
        require_optional_u32("timescale", self.timescale, OPUS_SAMPLE_RATE)?;
        if self.bitrate == Some(0) {
            return Err(MsfCatalogError::ZeroBitrate);
        }
        if self.bitrate.is_none() {
            return Err(MsfCatalogError::MissingCanonicalField { field: "bitrate" });
        }
        require_optional_u32("samplerate", self.samplerate, OPUS_SAMPLE_RATE)?;
        require_optional_value(
            "channelConfig",
            self.channel_config.as_deref(),
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

    fn validate_sanitized_events(&self) -> Result<(), MsfCatalogError> {
        require_value("name", &self.name, EVENTS_TRACK)?;
        require_value("packaging", &self.packaging, EVENT_TIMELINE_PACKAGING)?;
        if !self.is_live {
            return Err(MsfCatalogError::TrackMustBeLive);
        }
        require_value("role", &self.role, EVENT_TIMELINE_ROLE)?;
        require_optional_value(
            "eventType",
            self.event_type.as_deref(),
            SANITIZED_EVENTS_EVENT_TYPE,
        )?;
        require_optional_value("mimeType", self.mime_type.as_deref(), JSON_MIME_TYPE)?;
        match self.depends.as_deref() {
            Some([dependency]) if dependency == AUDIO_TRACK => {}
            Some(actual) => {
                return Err(MsfCatalogError::InvalidDependencies {
                    actual: actual.to_vec(),
                });
            }
            None => return Err(MsfCatalogError::MissingCanonicalField { field: "depends" }),
        }
        for (field, present) in [
            ("codec", self.codec.is_some()),
            ("timescale", self.timescale.is_some()),
            ("bitrate", self.bitrate.is_some()),
            ("samplerate", self.samplerate.is_some()),
            ("channelConfig", self.channel_config.is_some()),
            ("lang", self.lang.is_some()),
        ] {
            require_absent(field, present)?;
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

    pub fn event_type(&self) -> Option<&str> {
        self.event_type.as_deref()
    }

    pub fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }

    pub fn dependencies(&self) -> &[String] {
        self.depends.as_deref().unwrap_or_default()
    }

    pub fn codec(&self) -> Option<&str> {
        self.codec.as_deref()
    }

    pub const fn timescale(&self) -> Option<u32> {
        self.timescale
    }

    pub const fn bitrate(&self) -> Option<u32> {
        self.bitrate
    }

    pub const fn samplerate(&self) -> Option<u32> {
        self.samplerate
    }

    pub fn channel_config(&self) -> Option<&str> {
        self.channel_config.as_deref()
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

fn require_optional_value(
    field: &'static str,
    actual: Option<&str>,
    expected: &'static str,
) -> Result<(), MsfCatalogError> {
    let actual = actual.ok_or(MsfCatalogError::MissingCanonicalField { field })?;
    require_value(field, actual, expected)
}

fn require_optional_u32(
    field: &'static str,
    actual: Option<u32>,
    expected: u32,
) -> Result<(), MsfCatalogError> {
    let actual = actual.ok_or(MsfCatalogError::MissingCanonicalField { field })?;
    require_u32(field, actual, expected)
}

fn require_absent(field: &'static str, present: bool) -> Result<(), MsfCatalogError> {
    if present {
        Err(MsfCatalogError::ForbiddenCanonicalField { field })
    } else {
        Ok(())
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
#[non_exhaustive]
pub enum MsfCatalogError {
    #[error("MSF catalog bitrate must be greater than zero")]
    ZeroBitrate,
    #[error("MSF catalog language is not a well-formed BCP-47 tag: {value:?}")]
    InvalidLanguage { value: String },
    #[error("unsupported MSF catalog version {offered:?}; expected draft-01")]
    UnsupportedVersion { offered: String },
    #[error("live MSF catalog must contain its audio track")]
    MissingTracks,
    #[error("canonical MSF catalog must contain audio and at most one event track, got {actual}")]
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
    #[error("canonical MSF field {field} is required")]
    MissingCanonicalField { field: &'static str },
    #[error("canonical MSF field {field} is forbidden for this track")]
    ForbiddenCanonicalField { field: &'static str },
    #[error("canonical MSF event timeline must depend only on audio/main, got {actual:?}")]
    InvalidDependencies { actual: Vec<String> },
    #[error("MSF catalog namespace must be {expected:?}, got {actual:?}")]
    NamespaceMismatch { expected: String, actual: String },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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
    fn opt_in_catalog_advertises_only_the_canonical_sanitized_event_track() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let catalog = MsfCatalog::opus_audio_with_sanitized_events(
            &namespace,
            24_000,
            Some("en-US".to_owned()),
            1234,
        )
        .unwrap();
        let json = serde_json::to_value(&catalog).unwrap();

        assert_eq!(json["tracks"].as_array().unwrap().len(), 2);
        let events = &json["tracks"][1];
        assert_eq!(events["namespace"], "tenant/broadcast");
        assert_eq!(events["name"], EVENTS_TRACK);
        assert_eq!(events["packaging"], "eventtimeline");
        assert_eq!(events["isLive"], true);
        assert_eq!(events["role"], "eventtimeline");
        assert_eq!(events["eventType"], SANITIZED_EVENTS_EVENT_TYPE);
        assert_eq!(events["mimeType"], "application/json");
        assert_eq!(events["depends"], json!([AUDIO_TRACK]));
        assert_eq!(
            events
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "depends",
                "eventType",
                "isLive",
                "mimeType",
                "name",
                "namespace",
                "packaging",
                "role",
            ])
        );
        for forbidden in [
            "codec",
            "timescale",
            "bitrate",
            "samplerate",
            "channelConfig",
            "lang",
            "callId",
            "correlationId",
            "provider",
            "headers",
            "metadata",
        ] {
            assert!(events.get(forbidden).is_none(), "exposed {forbidden}");
        }

        let decoded: MsfCatalog = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, catalog);
        let events = &decoded.tracks()[1];
        assert_eq!(events.event_type(), Some(SANITIZED_EVENTS_EVENT_TYPE));
        assert_eq!(events.mime_type(), Some("application/json"));
        assert_eq!(events.dependencies(), &[AUDIO_TRACK.to_owned()]);
        assert_eq!(events.codec(), None);
        catalog.validate_for(&namespace).unwrap();
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
    fn deserialization_cannot_expand_the_sanitized_event_profile() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let catalog =
            MsfCatalog::opus_audio_with_sanitized_events(&namespace, 24_000, None, 1234).unwrap();
        let valid = serde_json::to_value(catalog).unwrap();

        for (field, value) in [
            ("name", json!("events/private")),
            ("packaging", json!("loc")),
            ("isLive", json!(false)),
            ("role", json!("metadata")),
            ("eventType", json!("com.example.arbitrary")),
            ("mimeType", json!("text/plain")),
            ("depends", json!(["audio/main", "private/context"])),
            ("codec", json!("json")),
            ("lang", json!("en")),
        ] {
            let mut invalid = valid.clone();
            invalid["tracks"][1][field] = value;
            assert!(
                serde_json::from_value::<MsfCatalog>(invalid).is_err(),
                "event field {field} bypassed validation"
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
