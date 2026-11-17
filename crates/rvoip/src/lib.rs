//! # rvoip — Universal real-time gateway library
//!
//! `rvoip` is the facade crate for the rvoip workspace. It always compiles the
//! voip-3 substrate (`rvoip-core` + `rvoip-core-traits` — the cross-transport
//! `Orchestrator` and the `Conversation`/`Session`/`Connection`/`Stream`/
//! `Message`/`Participant` model) and lets you opt into transports and
//! extensions behind cargo features, defaulting to the SIP product.
//!
//! ## Maturity tiers
//!
//! All workspace crates are aligned on one release version. Maturity is
//! product-specific rather than encoded in the version number: the `sip`
//! surface is beta-qualified, while `webrtc`, `uctp`, the `voip-3`
//! extensions, and `client` are available as developer previews.
//!
//! See `docs/PRD.md`, `INTERFACE_DESIGN.md`, and `CONVERSATION_PROTOCOL.md`
//! for the architectural context.
//!
//! ## Quick start
//!
//! ```no_run
//! use rvoip::{Orchestrator, Config};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // `Orchestrator::new` returns an `Arc<Orchestrator>`.
//! let orchestrator = Orchestrator::new(Config::default());
//!
//! // Register interop adapters (e.g. `rvoip::sip::SipAdapter::new(coordinator).await?`,
//! // built from a configured `UnifiedCoordinator`) via `orchestrator.register(adapter)?`.
//!
//! let mut events = orchestrator.subscribe_events();
//! while let Ok(event) = events.recv().await {
//!     // handle each orchestrator event
//!     drop(event);
//! }
//! # Ok(()) }
//! ```
//!
//! ## Cargo features
//!
//! | Feature | Default | Pulls in |
//! |---|:---:|---|
//! | `sip` | ✅ | SIP interop adapter (`rvoip::sip`) — **beta** |
//! | `webrtc` | | WebRTC interop adapter (`rvoip::webrtc`) — developer preview |
//! | `uctp` | | UCTP substrate adapters — QUIC / WebTransport / WebSocket (`rvoip::uctp`) — developer preview |
//! | `amazon-a2a` | | Amazon Connect A2A external-agent transport (`rvoip::amazon_a2a`) — developer preview |
//! | `webex-byova` | | Webex Contact Center BYOVA virtual-agent transport (`rvoip::webex_byova`) — experimental |
//! | `vapi` | | Vapi bidirectional WebSocket agent adapter (`rvoip::vapi`) — developer preview |
//! | `sip-stir-shaken` | | RFC 8224 caller-ID attestation; requires `sip` (`rvoip::stir_shaken`) — developer preview |
//! | `voip-3` | | SIP + WebRTC + UCTP **+** vCon / identity / AI-harness extensions — developer preview |
//! | `client` | | Cross-transport client SDK (`rvoip::client`) — developer preview |
//! | `app` | | High-level gateway builder (`rvoip::app`) — developer preview |
//! | `full` | | `voip-3` + `vapi` + `amazon-a2a` + `webex-byova` + `sip-stir-shaken` + `client` + `app` |
//!
//! Deployment-oriented `bundle-*` features group these flags into tested
//! starting points. See the repository's `docs/FEATURE_BUNDLES.md` for the
//! exact machine-checked matrix and native dependency boundaries.
//!
//! The `vcon`, `identity`, and `harness` conversation-model extensions are
//! transport-agnostic and reachable **only** through the `voip-3` feature.
//!
//! ## Module layout
//!
//! The unifying voip-3 nouns are re-exported at the crate root via
//! `rvoip::core_traits`; the `Orchestrator` + `Config` at the root directly.
//! Each transport/extension lives under its own feature-gated module
//! (`rvoip::sip`, `rvoip::webrtc`, `rvoip::uctp`, `rvoip::app`,
//! `rvoip::vapi`, `rvoip::client`, …).

#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

// ---------------------------------------------------------------------------
// Always compiled: voip-3 spine (Orchestrator + nouns)
// ---------------------------------------------------------------------------

// The implementation crate. Always pulled in (the facade depends on it
// directly per `[bans.wrappers]` in workspace `deny.toml`).
pub use rvoip_core::{Config, Orchestrator};

// The shared trait / data surface. Adapter crates depend on this rather than
// on `rvoip-core` to avoid pulling in the orchestrator implementation.
pub use rvoip_core_traits as core_traits;

// ---------------------------------------------------------------------------
// SIP (beta)
// ---------------------------------------------------------------------------

/// SIP interop adapter — bridges SIP/RTP into the voip-3 `Session`
/// abstraction. See `rvoip-sip` for the full surface.
#[cfg(feature = "sip")]
pub mod sip {
    pub use rvoip_sip::*;
}

/// STIR/SHAKEN (RFC 8224) caller-ID attestation for SIP — `PASSporT`
/// signing/verification plugged into the SIP dialog layer. SIP-only;
/// enabled by the `sip-stir-shaken` feature (which implies `sip`).
#[cfg(feature = "sip-stir-shaken")]
pub mod stir_shaken {
    pub use rvoip_stir_shaken::*;
}

// ---------------------------------------------------------------------------
// WebRTC (developer preview)
// ---------------------------------------------------------------------------

/// WebRTC interop adapter — bridges DTLS-SRTP / ICE peers into the voip-3
/// `Session` abstraction. Off by default; enable the `webrtc` feature.
#[cfg(feature = "webrtc")]
pub mod webrtc {
    pub use rvoip_webrtc::*;
}

/// Bearer-credential validation shared by transport auth hooks.
///
/// An app owner implements [`auth::BearerValidator`] and hands it to a
/// transport hook (for WebRTC, `webrtc::signaling::auth::AuthCoreHook`)
/// so signaling upgrades authenticate against the owner's own control
/// plane. Exposed with the `webrtc` feature because that is the transport
/// whose app-level config accepts a hook today.
#[cfg(feature = "webrtc")]
pub mod auth {
    pub use rvoip_auth_core::*;
}

// ---------------------------------------------------------------------------
// UCTP substrates (developer preview)
// ---------------------------------------------------------------------------

/// UCTP substrate adapters and protocol primitives. Per
/// `CONVERSATION_PROTOCOL.md` §4, UCTP runs over QUIC, WebTransport, and
/// WebSocket substrates; this module re-exports all three plus the
/// wire-level protocol from `rvoip-uctp`. Enable the `uctp` feature.
#[cfg(feature = "uctp")]
pub mod uctp {
    /// UCTP-over-QUIC substrate adapter.
    pub use rvoip_quic as quic;
    /// Envelope encode/decode, capability negotiation, session state machine.
    pub use rvoip_uctp as protocol;
    /// UCTP-over-WebSocket substrate adapter.
    pub use rvoip_websocket as websocket;
    /// UCTP-over-WebTransport substrate adapter.
    pub use rvoip_webtransport as webtransport;
}

// ---------------------------------------------------------------------------
// Vapi agent transport (developer preview)
// ---------------------------------------------------------------------------

/// Vapi bidirectional WebSocket agent adapter. It lets rvoip retain ownership
/// of a SIP or WebRTC caller leg while Vapi runs the remote voice-AI pipeline.
/// Off by default; enable the `vapi` feature.
#[cfg(feature = "vapi")]
pub mod vapi {
    pub use rvoip_vapi::*;
}

/// Amazon Connect A2A external-agent transport (developer preview).
#[cfg(feature = "amazon-a2a")]
pub mod amazon_a2a {
    pub use rvoip_amazon_a2a::*;
}

/// Webex Contact Center BYOVA (Bring Your Own Virtual Agent) transport
/// (experimental). The facade enables the WebSocket adapter; the crate's
/// optional `grpc` feature needs Cisco's protobuf files at build time.
#[cfg(feature = "webex-byova")]
pub mod webex_byova {
    pub use rvoip_webex_byova::*;
}

// ---------------------------------------------------------------------------
// voip-3 conversation-model extensions (developer preview) — reachable only via `voip-3`
// ---------------------------------------------------------------------------

/// vCon (IETF Virtualized Conversations) container builder, signer, and store —
/// emitted per Session regardless of transport. Part of the `voip-3` feature.
#[cfg(feature = "voip-3")]
pub mod vcon {
    pub use rvoip_vcon::*;
}

/// `IdentityProvider` backends — bearer, OAuth 2.1 + DPoP, OIDC, passkeys,
/// SIP Digest, AAuth. Transport-agnostic; part of the `voip-3` feature.
#[cfg(feature = "voip-3")]
pub mod identity {
    pub use rvoip_identity::*;
}

/// AI voice harness — in-process ASR / TTS / Dialog runtime that attaches to a
/// `Connection` via the orchestrator. Part of the `voip-3` feature.
#[cfg(feature = "voip-3")]
pub mod harness {
    pub use rvoip_harness::*;
}

// ---------------------------------------------------------------------------
// Client SDK (developer preview)
// ---------------------------------------------------------------------------

/// Client-side API for mobile / web / desktop / embedded apps, wrapping the
/// SIP / WebRTC / UCTP transports behind one surface. See `rvoip-client`.
#[cfg(feature = "client")]
pub mod client {
    pub use rvoip_client::*;
}

// ---------------------------------------------------------------------------
// High-level app/gateway API (developer preview)
// ---------------------------------------------------------------------------

/// High-level server/gateway API for building practical cross-transport VoIP
/// apps without manually wiring adapters, registrar state, core event loops,
/// and media bridges.
#[cfg(feature = "app")]
pub mod app;

/// The version of the rvoip facade crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod feature_bundle_contract_tests {
    //! Each deployment bundle's contract has two halves.
    //!
    //! * **What it turns on.** Checked twice: against the bundle's declared
    //!   feature closure in this crate's own manifest (every build), and with
    //!   `cfg!` in a build that selects the bundle, which proves the declared
    //!   surface is actually compiled in.
    //! * **What it must leave off.** Checked against the declared closure only.
    //!   A `cfg!(feature = "opus")` check would be wrong under
    //!   `--all-features`, where Cargo unifies every feature into one build
    //!   whatever the bundle declares. The closure is independent of which
    //!   features happen to be active, so the check runs (and can fail) in
    //!   default, per-bundle and `--all-features` builds alike.

    use std::collections::BTreeMap;

    type FeatureTable = BTreeMap<String, Vec<String>>;

    /// The `[features]` table of the manifest this test was compiled from.
    fn manifest_features() -> FeatureTable {
        let manifest: toml::Table =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
                .parse()
                .expect("crates/rvoip/Cargo.toml parses as TOML");
        let features = manifest
            .get("features")
            .and_then(toml::Value::as_table)
            .expect("crates/rvoip/Cargo.toml has a [features] table");
        features
            .iter()
            .map(|(name, members)| {
                let members = members
                    .as_array()
                    .unwrap_or_else(|| panic!("feature `{name}` is not an array"))
                    .iter()
                    .map(|member| {
                        member
                            .as_str()
                            .unwrap_or_else(|| panic!("feature `{name}` has a non-string member"))
                            .to_owned()
                    })
                    .collect();
                (name.clone(), members)
            })
            .collect()
    }

    /// Everything a feature turns on, as declared in the manifest.
    #[derive(Debug, Default)]
    struct DeclaredClosure {
        /// Local feature names reached, each mapped to the feature that listed
        /// it (the root maps to itself), so a violation can name its path.
        local: BTreeMap<String, String>,
        /// `dep/feature` and `dep?/feature` entries reached, with the local
        /// feature that listed each one.
        forwarded: Vec<(String, String)>,
    }

    impl DeclaredClosure {
        /// `root -> ... -> name` for a local feature in the closure.
        fn path_to(&self, name: &str) -> String {
            let mut chain = vec![name];
            let mut current = name;
            while let Some(parent) = self.local.get(current) {
                if parent == current {
                    break;
                }
                chain.push(parent);
                current = parent;
            }
            chain.reverse();
            chain.join(" -> ")
        }
    }

    /// The transitive closure of `root` over this crate's own feature names.
    ///
    /// Only local names are followed. `dep:x` enables an optional dependency,
    /// not a feature, so it is skipped. `x/y` and `x?/y` enable feature `y` of
    /// dependency `x`; they are recorded (so a forbidden feature forwarded
    /// straight to a dependency is still caught) but not followed, since `y`
    /// names a feature of another crate.
    fn declared_closure(features: &FeatureTable, root: &str) -> DeclaredClosure {
        assert!(
            features.contains_key(root),
            "`{root}` is not a feature of crates/rvoip/Cargo.toml"
        );
        let mut closure = DeclaredClosure::default();
        closure.local.insert(root.to_owned(), root.to_owned());
        let mut pending = vec![root.to_owned()];
        while let Some(current) = pending.pop() {
            for entry in features.get(&current).into_iter().flatten() {
                if entry.starts_with("dep:") {
                    continue;
                }
                if entry.contains('/') {
                    closure.forwarded.push((entry.clone(), current.clone()));
                    continue;
                }
                if !closure.local.contains_key(entry) {
                    closure.local.insert(entry.clone(), current.clone());
                    pending.push(entry.clone());
                }
            }
        }
        closure
    }

    /// Every way `closure` reaches one of the `excluded` features: as a local
    /// feature, or forwarded to a dependency under the same feature name
    /// (`rvoip-sip/opus` turns native Opus on just as `opus` does).
    fn excluded_violations(closure: &DeclaredClosure, excluded: &[&str]) -> Vec<String> {
        let mut violations = Vec::new();
        for &name in excluded {
            if closure.local.contains_key(name) {
                violations.push(format!("`{name}` via {}", closure.path_to(name)));
            }
            for (entry, listed_by) in &closure.forwarded {
                let forwarded_feature = entry.rsplit_once('/').map_or(entry.as_str(), |(_, f)| f);
                if forwarded_feature == name {
                    violations.push(format!(
                        "`{entry}` via {} -> {entry}",
                        closure.path_to(listed_by)
                    ));
                }
            }
        }
        violations
    }

    fn assert_declares(features: &FeatureTable, bundle: &str, enabled: &[&str], excluded: &[&str]) {
        let closure = declared_closure(features, bundle);
        let missing: Vec<&str> = enabled
            .iter()
            .copied()
            .filter(|name| !closure.local.contains_key(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "{bundle} no longer declares {missing:?}; its closure is {:?}",
            closure.local.keys().collect::<Vec<_>>()
        );
        let violations = excluded_violations(&closure, excluded);
        assert!(
            violations.is_empty(),
            "{bundle} must not enable {excluded:?}, but its declared closure reaches {violations:?}"
        );
    }

    #[test]
    fn sip_endpoint_bundle_enables_only_its_declared_facade_surface() {
        assert_declares(
            &manifest_features(),
            "bundle-sip-endpoint",
            &["sip"],
            &["webrtc", "uctp", "dtls-srtp", "opus"],
        );

        #[cfg(feature = "bundle-sip-endpoint")]
        assert!(cfg!(feature = "sip"));
    }

    #[test]
    fn carrier_bundle_enables_security_identity_and_pure_rust_telephony_codecs() {
        assert_declares(
            &manifest_features(),
            "bundle-carrier-sip",
            &[
                "sip",
                "sip-stir-shaken",
                "dtls-srtp",
                "g729",
                "amr-nb",
                "amr-wb",
            ],
            &["opus"],
        );

        #[cfg(feature = "bundle-carrier-sip")]
        {
            assert!(cfg!(feature = "sip"));
            assert!(cfg!(feature = "sip-stir-shaken"));
            assert!(cfg!(feature = "dtls-srtp"));
            assert!(cfg!(feature = "g729"));
            assert!(cfg!(feature = "amr-nb"));
            assert!(cfg!(feature = "amr-wb"));
        }
    }

    #[test]
    fn browser_gateway_bundle_enables_app_transports_and_opus() {
        assert_declares(
            &manifest_features(),
            "bundle-browser-gateway",
            &["app", "sip", "webrtc", "uctp", "opus"],
            &[],
        );

        #[cfg(feature = "bundle-browser-gateway")]
        {
            assert!(cfg!(feature = "app"));
            assert!(cfg!(feature = "sip"));
            assert!(cfg!(feature = "webrtc"));
            assert!(cfg!(feature = "uctp"));
            assert!(cfg!(feature = "opus"));
        }
    }

    #[test]
    fn ai_conversation_bundle_enables_app_conversation_and_agent_surfaces() {
        assert_declares(
            &manifest_features(),
            "bundle-ai-conversation",
            &["app", "voip-3", "vapi", "opus"],
            &[],
        );

        #[cfg(feature = "bundle-ai-conversation")]
        {
            assert!(cfg!(feature = "app"));
            assert!(cfg!(feature = "voip-3"));
            assert!(cfg!(feature = "vapi"));
            assert!(cfg!(feature = "opus"));
        }
    }

    #[test]
    fn pure_rust_full_bundle_keeps_native_opus_off() {
        assert_declares(
            &manifest_features(),
            "bundle-full-pure-rust",
            &["full", "dtls-srtp", "g729", "amr-nb", "amr-wb"],
            &["opus"],
        );

        #[cfg(feature = "bundle-full-pure-rust")]
        {
            assert!(cfg!(feature = "full"));
            assert!(cfg!(feature = "dtls-srtp"));
            assert!(cfg!(feature = "g729"));
            assert!(cfg!(feature = "amr-nb"));
            assert!(cfg!(feature = "amr-wb"));
        }
    }

    #[test]
    fn native_full_bundle_enables_every_mainline_codec() {
        assert_declares(
            &manifest_features(),
            "bundle-full-native",
            &["full", "dtls-srtp", "g729", "amr-nb", "amr-wb", "opus"],
            &[],
        );

        #[cfg(feature = "bundle-full-native")]
        {
            assert!(cfg!(feature = "full"));
            assert!(cfg!(feature = "dtls-srtp"));
            assert!(cfg!(feature = "g729"));
            assert!(cfg!(feature = "amr-nb"));
            assert!(cfg!(feature = "amr-wb"));
            assert!(cfg!(feature = "opus"));
        }
    }

    /// The closure walker itself: it follows local names through meta-features,
    /// skips `dep:` entries, does not follow forwarded entries, and reports a
    /// forbidden feature reached either way.
    #[test]
    fn declared_closure_follows_local_names_and_catches_forwarded_features() {
        let features: FeatureTable = [
            ("bundle", vec!["meta", "dep:quiet", "codec-crate?/weak"]),
            ("meta", vec!["leaf", "codec-crate/opus"]),
            ("leaf", vec![]),
            ("opus", vec!["dep:codec-crate"]),
            ("weak", vec![]),
            ("reaches-opus", vec!["meta-opus"]),
            ("meta-opus", vec!["opus"]),
        ]
        .into_iter()
        .map(|(name, members)| {
            (
                name.to_owned(),
                members.into_iter().map(str::to_owned).collect(),
            )
        })
        .collect();

        let closure = declared_closure(&features, "bundle");
        assert_eq!(
            closure.local.keys().map(String::as_str).collect::<Vec<_>>(),
            ["bundle", "leaf", "meta"],
            "`dep:` and forwarded entries are not local features and are not followed"
        );
        assert_eq!(
            excluded_violations(&closure, &["opus"]),
            ["`codec-crate/opus` via bundle -> meta -> codec-crate/opus"]
        );
        assert!(excluded_violations(&closure, &["quiet", "codec-crate"]).is_empty());

        let closure = declared_closure(&features, "reaches-opus");
        assert_eq!(
            excluded_violations(&closure, &["opus"]),
            ["`opus` via reaches-opus -> meta-opus -> opus"]
        );
    }
}
