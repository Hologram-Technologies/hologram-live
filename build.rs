fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    tonic_prost_build::configure().compile_protos(
        &[
            "proto/hologram/live/v1/live.proto",
            "proto/hologram/live/plugin/v1/plugin.proto",
        ],
        &["proto"],
    )?;
    println!("cargo:rerun-if-changed=proto/hologram/live/v1/live.proto");
    println!("cargo:rerun-if-changed=proto/hologram/live/plugin/v1/plugin.proto");
    Ok(())
}
