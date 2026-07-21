fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let mut prost_config = prost_build::Config::new();
    prost_config.protoc_executable(protoc);
    tonic_prost_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_with_config(
            prost_config,
            &["../../proto/context_hub/v1/context_hub.proto"],
            &["../../proto"],
        )?;
    println!("cargo:rerun-if-changed=../../proto/context_hub/v1/context_hub.proto");
    Ok(())
}
