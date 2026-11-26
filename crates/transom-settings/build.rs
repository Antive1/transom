//! Vlozi ikonu transom (`assets/brand/transom.ico`) a VERSIONINFO (nazov produktu, verzia) do .exe.
#[path = "../../build_support/icon_res.rs"]
mod icon_res;

fn main() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("transom-icon.res");
    icon_res::emit(
        Some(std::path::Path::new("../../assets/brand/transom.ico")),
        &out,
        "transom settings",
        "transom-settings.exe",
    );
}
