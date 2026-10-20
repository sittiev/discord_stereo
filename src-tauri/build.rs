fn main() {
    // capabilities/dev/ holds dev-only capabilities (MCP bridge). They are only
    // picked up when the `dev-tools` feature is enabled, so release builds
    // never reference the bridge plugin (which is not compiled there).
    println!("cargo:rerun-if-changed=capabilities");
    let capabilities_pattern = if std::env::var_os("CARGO_FEATURE_DEV_TOOLS").is_some() {
        "./capabilities/**/*.json"
    } else {
        "./capabilities/*.json"
    };
    tauri_build::try_build(
        tauri_build::Attributes::new().capabilities_path_pattern(capabilities_pattern),
    )
    .expect("failed to run tauri-build");
}
