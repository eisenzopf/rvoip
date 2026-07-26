// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::module_inception)]

//! Messages used for the MoQ Transport handshake.
//!
//! In draft-19, both peers open a unidirectional control stream and send a
//! unified SETUP message. Version negotiation is handled entirely by ALPN.

mod param_types;
mod setup;
mod version;

pub use param_types::*;
pub use setup::*;
pub use version::*;

/// Supported MoQT ALPN protocol identifiers, in preference order (most preferred first).
///
/// Used for version negotiation: the server picks the first entry that the client also offers.
/// For native QUIC, these are offered as TLS ALPN values.
/// For WebTransport, these are offered/selected via the WT-Available-Protocols / WT-Protocol headers.
pub const SUPPORTED_ALPNS: &[&str] = &["moqt-19"];

/// The preferred (most recent) ALPN, used as the default for single-version contexts.
pub const ALPN: &[u8] = b"moqt-19";

/// Select the best mutually-supported MoQT version from a list of protocols offered by a peer.
///
/// Returns the first entry in [`SUPPORTED_ALPNS`] that appears in `offered`, or `None` if
/// there is no overlap. This gives the server control over version preference ordering.
pub fn negotiate_version(offered: &[impl AsRef<str>]) -> Option<&'static str> {
    SUPPORTED_ALPNS
        .iter()
        .find(|ours| offered.iter().any(|theirs| theirs.as_ref() == **ours))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_exact_match() {
        let offered = vec!["moqt-19".to_string()];
        assert_eq!(negotiate_version(&offered), Some("moqt-19"));
    }

    #[test]
    fn negotiate_picks_our_preference() {
        let offered = vec!["moqt-18".to_string(), "moqt-19".to_string()];
        assert_eq!(negotiate_version(&offered), Some("moqt-19"));
    }

    #[test]
    fn negotiate_rejects_draft_18() {
        let offered = vec!["moqt-18".to_string()];
        assert_eq!(negotiate_version(&offered), None);
    }

    #[test]
    fn negotiate_no_overlap() {
        let offered = vec!["moqt-99".to_string()];
        assert_eq!(negotiate_version(&offered), None);
    }

    #[test]
    fn negotiate_empty_offer() {
        let offered: Vec<String> = vec![];
        assert_eq!(negotiate_version(&offered), None);
    }
}
