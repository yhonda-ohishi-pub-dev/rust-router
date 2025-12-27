use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    // Configure tonic-build
    let mut config = tonic_build::configure()
        .build_server(true)
        .build_client(true);

    // Enable file descriptor set for reflection if the feature is enabled
    // Note: In build.rs, features are checked via environment variables
    if env::var("CARGO_FEATURE_REFLECTION").is_ok() {
        config = config.file_descriptor_set_path(out_dir.join("gateway_descriptor.bin"));
    }

    // Collect proto files to compile based on features
    let mut protos = Vec::new();

    if env::var("CARGO_FEATURE_GATEWAY").is_ok() {
        protos.push("proto/gateway.proto");
    }

    if env::var("CARGO_FEATURE_SCRAPER").is_ok() {
        protos.push("proto/scraper.proto");
    }

    // if env::var("CARGO_FEATURE_TIMECARD").is_ok() {
    //     protos.push("proto/timecard.proto");
    // }

    if env::var("CARGO_FEATURE_PDF").is_ok() {
        protos.push("proto/pdf.proto");
    }

    // If no feature is enabled, compile all protos (for development)
    if protos.is_empty() {
        protos.push("proto/gateway.proto");
        protos.push("proto/scraper.proto");
        protos.push("proto/pdf.proto");
    }

    config.compile_protos(&protos, &["proto"])?;

    Ok(())
}
