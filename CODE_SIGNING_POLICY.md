# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

## What is signed

Only transom's own executables (`transom-app.exe`, `transom-settings.exe`, `transom-statusline.exe`) and its installer
(`transom-setup-<version>-x64.exe`), as published on the [releases page](https://github.com/Antive1/transom/releases).

Every signed file is built from the tagged source in this repository by the
[release workflow](.github/workflows/release.yml) on a GitHub-hosted runner. Nothing built on a personal computer is signed.
Each signing request is approved manually.

## Team roles

| Role | Members |
|---|---|
| Committers and reviewers | [Antive1](https://github.com/Antive1) |
| Approvers | [Antive1](https://github.com/Antive1) |

Changes from anyone outside this list come in through pull requests and are reviewed by a committer before they are merged.
All team members use multi-factor authentication for GitHub and SignPath.

## Privacy

transom has no telemetry. Its network requests (the update check on GitHub, which can be turned off, and calendars or weather the
user sets up) are described in the [privacy policy](PRIVACY.md).
