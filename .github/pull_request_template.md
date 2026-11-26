## What and why

## How it was tested
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] `cargo deny check`
- [ ] Windows build / manual checks (for AppBar, DPI, audio or accessibility changes):

## Checklist
- [ ] No personal data, machine paths or real usage data in code, fixtures or messages
- [ ] No network access, telemetry or undocumented Windows interfaces
- [ ] New UI text exists in all 11 languages
- [ ] Docs updated; claims say how they were verified (live / test / simulation / estimate)
