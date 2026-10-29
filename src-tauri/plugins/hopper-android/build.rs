// The Android side lives in ./android. It has no commands that the web page calls
// directly; Rust calls it through `call`.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).android_path("android").build();
}
