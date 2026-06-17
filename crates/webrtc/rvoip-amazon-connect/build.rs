//! Compile the vendored Amazon Chime SDK signaling schema into Rust types.
//!
//! The schema is a verbatim copy of `protocol/SignalingProtocol.proto` from
//! <https://github.com/aws/amazon-chime-sdk-js>. It is `proto2`. The generated
//! module is included by `src/signaling/proto.rs`.

fn main() {
    println!("cargo:rerun-if-changed=proto/SignalingProtocol.proto");

    let mut config = prost_build::Config::new();
    // The schema declares no `package`, so prost would emit `_.rs`. Pin a stable
    // filename that `src/signaling/proto.rs` can `include!`.
    config.default_package_filename("chime_signaling");
    // proto2 optional fields map to Rust `Option<T>` already; nothing special
    // required. Keep the default output dir (OUT_DIR).
    config
        .compile_protos(&["proto/SignalingProtocol.proto"], &["proto"])
        .expect("failed to compile SignalingProtocol.proto");
}
