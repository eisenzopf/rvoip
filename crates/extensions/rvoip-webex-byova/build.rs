//! The optional `grpc` feature compiles Cisco's Voice Virtual Agent protobuf
//! contract. Cisco publishes it without a licence, so rvoip neither packages
//! nor commits it: the build reads it from `RVOIP_WEBEX_BYOVA_PROTO_DIR`, or,
//! inside the rvoip workspace checkout, from
//! `target/third_party/cisco-webex-byova-proto`, where
//! `scripts/ci/fetch_webex_byova_protos.py` places the pinned, verified files.
//! Default builds need neither.

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
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let workspace = manifest.join("../../..");
    let in_workspace = workspace
        .join("scripts/ci/fetch_webex_byova_protos.py")
        .is_file();
    let dir = match std::env::var_os(PROTO_DIR_ENV) {
        Some(dir) => PathBuf::from(dir),
        None => workspace.join("target/third_party/cisco-webex-byova-proto"),
    };
    let missing: Vec<&str> = FILES
        .iter()
        .copied()
        .filter(|file| !dir.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        let how = if std::env::var_os(PROTO_DIR_ENV).is_none() && in_workspace {
            "In the rvoip workspace, fetch the pinned, checksum-verified files with\n\
             \n    python3 scripts/ci/fetch_webex_byova_protos.py\n\n\
             (or set RVOIP_WEBEX_BYOVA_PROTO_DIR to a directory that holds them)."
                .to_owned()
        } else {
            format!(
                "Download voicevirtualagent.proto and byova_common.proto from\n\
                 https://github.com/webex/dataSourceSchemas (tested at commit\n\
                 fca090dfa806e28695acb145e927ad0d557ce7fe, directory\n\
                 Services/VoiceVirtualAgent/5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto)\n\
                 and set {PROTO_DIR_ENV} to the directory that contains them."
            )
        };
        panic!(
            "\n\nrvoip-webex-byova: the `grpc` feature needs Cisco's Voice Virtual Agent \
             protobuf files, which rvoip does not redistribute.\n\
             Missing from {}: {}\n\n{how}\n\
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
