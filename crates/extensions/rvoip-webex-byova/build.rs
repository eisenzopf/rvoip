//! The optional `grpc` feature compiles Cisco's Voice Virtual Agent protobuf
//! contract. Cisco's files are not redistributed with this crate: the build
//! reads them from `RVOIP_WEBEX_BYOVA_PROTO_DIR`, or, inside the rvoip
//! workspace checkout, from `third_party/cisco-webex-byova-proto`. Default
//! builds need neither.

#[cfg(feature = "grpc")]
const PROTO_DIR_ENV: &str = "RVOIP_WEBEX_BYOVA_PROTO_DIR";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=RVOIP_WEBEX_BYOVA_PROTO_DIR");
    #[cfg(feature = "grpc")]
    compile_protos()?;
    Ok(())
}

#[cfg(feature = "grpc")]
fn compile_protos() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    const FILES: [&str; 2] = ["voicevirtualagent.proto", "byova_common.proto"];
    let (dir, source) = match std::env::var_os(PROTO_DIR_ENV) {
        Some(dir) => (PathBuf::from(dir), PROTO_DIR_ENV),
        None => (
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?)
                .join("../../../third_party/cisco-webex-byova-proto"),
            "the rvoip workspace checkout",
        ),
    };
    let missing: Vec<&str> = FILES
        .iter()
        .copied()
        .filter(|file| !dir.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        panic!(
            "\n\nrvoip-webex-byova: the `grpc` feature needs Cisco's Voice Virtual Agent \
             protobuf files, which this crate does not redistribute.\n\
             Missing from {} (from {source}): {}\n\n\
             Download voicevirtualagent.proto and byova_common.proto from\n\
             https://github.com/webex/dataSourceSchemas (tested at commit \
             fca090dfa806e28695acb145e927ad0d557ce7fe, directory\n\
             Services/VoiceVirtualAgent/5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto)\n\
             and set {PROTO_DIR_ENV} to the directory that contains them.\n\
             Builds without the `grpc` feature need neither.\n",
            dir.display(),
            missing.join(", "),
        );
    }
    for file in FILES {
        println!("cargo:rerun-if-changed={}", dir.join(file).display());
    }
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    tonic_build::configure().compile_protos(
        &[dir.join("voicevirtualagent.proto")],
        &[dir, protoc_bin_vendored::include_path()?],
    )?;
    Ok(())
}
