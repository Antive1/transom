# Releasing transom

Repository: <https://github.com/Antive1/transom>. Releases are built and signed by GitHub Actions (`release` workflow); `ci` only checks.

## Making a release

1. Bump `version` in the root `Cargo.toml` (workspace version), add a `## [<version>] - <date>` section to
   [CHANGELOG.md](../CHANGELOG.md) (move the *Unreleased* entries there; `### Added`, `### Changed`, `### Fixed`, short
   user-facing `-` bullets) and commit. The release workflow copies that section to the top of the release notes and
   **fails on a tag without it**; the bar shows those bullets in its update panel (Added and Changed as "New", Fixed as
   "Fixes").
2. Run the checks: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
   `cargo test --workspace --locked`, `cargo deny check`.
3. Tag and push: `git tag -a v<version> -m "transom <version>"`, `git push origin v<version>`.
4. The [release workflow](../.github/workflows/release.yml) builds on GitHub, sends the executables and then the installer to
   SignPath, waits for the approval in SignPath (approve each request there), and publishes the release with the installer,
   the portable zip and `SHA256SUMS.txt`. Without the SignPath variables it publishes unsigned files.
5. Install the setup once: the bar starts, Settings opens, uninstall removes the files and the autostart entry and keeps
   `%APPDATA%\transom`.

Local build (unsigned, for testing): `tools\build-installer.ps1` writes the same files into `dist\`.

## SignPath setup (one time)

1. Apply at <https://signpath.org/apply> (repository must be public, OSI license, MFA on GitHub).
2. In SignPath: create project `transom`, link GitHub.com as trusted build system, install the SignPath GitHub App on the repo,
   add artifact configurations `binaries` and `installer` from `.signpath/artifact-configurations/`, use policy `release-signing`.
3. In GitHub (*Settings > Secrets and variables > Actions*): secret `SIGNPATH_API_TOKEN`, variable `SIGNPATH_ORGANIZATION_ID`
   (other slugs only if they differ from the defaults listed in the workflow).

## Open items

- Signing through SignPath Foundation is prepared but waits for the application to be approved; SmartScreen reputation builds over
  several signed releases.
- `THIRD_PARTY_NOTICES.md` lists dependencies; generate full license texts (`cargo about`) before going public.
- Enable *private vulnerability reporting* in the repository settings when it becomes public.
- Live checks still missing: real volume/mute on an active output, Narrator/NVDA, Logitech hardware.
- Translations other than `sk` and `en` are AI-drafted and unreviewed.
