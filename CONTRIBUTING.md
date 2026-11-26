# Contributing to transom

Thanks for your interest! transom is an early-preview (alpha) project, so small, focused changes are easiest to review.

## Before you start

- Open an issue first for anything larger than a bug fix, so we can agree on the approach.
- transom targets **Windows 11** only and deliberately has **no telemetry and no accounts**. Its only network requests are HTTPS GETs:
  the update check on GitHub (can be turned off), calendars the user adds and the optional weather ([PRIVACY.md](PRIVACY.md)).
  Changes that send anything about the user or add another network request will not be accepted without a discussion first.
- Prefer documented Windows APIs and say in the pull request which documentation you relied on. The one undocumented interface
  (per-app audio output in the mixer, like EarTrumpet) is isolated in `transom-mixer` and listed in the README's known limitations.
- Do not add third-party logos, fonts, icons or sounds. Sounds and brand assets are generated from code in this repository.

## Build and check

Rust **1.100.0** is pinned in `rust-toolchain.toml` (MSVC toolchain on Windows).

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
cargo build --release
```

CI runs the same commands on `windows-latest`. All of them must pass; do not silence a lint without a reason in the code.

## Code guidelines

- `unsafe` blocks need a short `// SAFETY:` comment; `unsafe_op_in_unsafe_fn` is denied.
- Never panic in code that can run on a foreign thread or in a Win32 callback (release builds use `panic = "abort"`): return errors.
- Tests must be isolated: use temporary directories and the `TRANSOM_CONFIG` / `TRANSOM_DATA` variables, never the real user config, and
  never open the real audio device, change the default output or system volume from `cargo test`.
- No personal data in code, fixtures, logs or commit messages (names, e-mail addresses, machine paths, media titles, account details).
- Any user-visible text goes through `transom-i18n` (`crates/transom-i18n/locales/*.txt`); every key must exist in all 11 languages with the
  same `{n}` placeholders (a test enforces it). A native-speaker review of translations is very welcome.
- New code comments and documentation should be in **English**. Much of the existing code, comments and `docs/PLAN.md` are in Slovak
  (partly without diacritics); translating them is a welcome contribution.
- Claims in documentation must say how they were verified: live, automated test, simulation or estimate.

## Commits and pull requests

- Keep commits small and coherent; describe the *why*. Do not rewrite shared history.
- By submitting a contribution you agree that it is licensed under the MIT license of this repository.
- Describe how you tested the change, and on which Windows build for anything that touches the AppBar, DPI, audio or accessibility.
