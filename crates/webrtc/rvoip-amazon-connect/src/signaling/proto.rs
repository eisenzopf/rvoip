//! Generated Amazon Chime SDK signaling types.
//!
//! Produced by `build.rs` from the vendored `proto/SignalingProtocol.proto`
//! (a verbatim copy of the schema in <https://github.com/aws/amazon-chime-sdk-js>).
//! The schema declares no package, so all messages live at the root of the
//! generated file; we surface them under this module.
#![allow(clippy::all)]
#![allow(missing_docs)]

include!(concat!(env!("OUT_DIR"), "/chime_signaling.rs"));
