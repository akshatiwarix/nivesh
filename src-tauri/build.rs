/// Commands the webview may invoke. tauri-build generates `allow-<command>`
/// and `deny-<command>` permissions for each, and `capabilities/main.json`
/// grants them explicitly. Keep in sync with `generate_handler!` in `lib.rs`.
const COMMANDS: &[&str] = &["app_info"];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(err) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {err:#}");
    }
}
