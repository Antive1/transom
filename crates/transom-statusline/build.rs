//! Vlozi VERSIONINFO (nazov produktu, verzia) do .exe.
#[path = "../../build_support/icon_res.rs"]
mod icon_res;

fn main() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("transom-icon.res");
    icon_res::emit(
        None,
        &out,
        "transom Claude Code statusLine bridge",
        "transom-statusline.exe",
    );
}
