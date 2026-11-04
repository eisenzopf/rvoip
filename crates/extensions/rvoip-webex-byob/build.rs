fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    #[cfg(feature = "grpc")]
    {
        std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
        tonic_build::configure().compile_protos(
            &["proto/voicevirtualagent.proto"],
            &[
                std::path::PathBuf::from("proto"),
                protoc_bin_vendored::include_path()?,
            ],
        )?;
    }
    Ok(())
}
