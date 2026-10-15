fn main() -> Result<(), Box<dyn std::error::Error>> {
    // gRPC protos only with `--features extras` (optional tonic-build).
    #[cfg(feature = "extras")]
    {
        tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .compile_protos(&["proto/agent_brain/v1/routing.proto"], &["proto"])?;
    }
    Ok(())
}
