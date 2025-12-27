fn main() {
    // Embed Windows manifest for UAC elevation (requireAdministrator)
    #[cfg(windows)]
    embed_resource::compile("gateway.rc", embed_resource::NONE);
}
