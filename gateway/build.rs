fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile proto files
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &["proto/gateway.proto"],
            &["proto"],
        )?;
    Ok(())
}
