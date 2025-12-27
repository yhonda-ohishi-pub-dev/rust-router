use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    // Configure tonic-build
    let mut config = tonic_build::configure()
        .build_server(true)
        .build_client(true);

    // Enable file descriptor set for reflection if the feature is enabled
    #[cfg(feature = "reflection")]
    {
        config = config.file_descriptor_set_path(out_dir.join("gateway_descriptor.bin"));
    }

    // Collect proto files to compile based on features
    let mut protos = Vec::new();

    #[cfg(feature = "gateway")]
    protos.push("proto/gateway.proto");

    #[cfg(feature = "scraper")]
    protos.push("proto/scraper.proto");

    // #[cfg(feature = "timecard")]
    // protos.push("proto/timecard.proto");

    #[cfg(feature = "pdf")]
    protos.push("proto/pdf.proto");

    // If no feature is enabled, compile all protos (for development)
    if protos.is_empty() {
        protos.push("proto/gateway.proto");
        protos.push("proto/scraper.proto");
        protos.push("proto/pdf.proto");
    }

    config.compile_protos(&protos, &["proto"])?;

    Ok(())
}
