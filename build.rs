fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    tonic_prost_build::compile_protos("proto/hologram/live/v1/live.proto")?;
    println!("cargo:rerun-if-changed=proto/hologram/live/v1/live.proto");
    Ok(())
}
