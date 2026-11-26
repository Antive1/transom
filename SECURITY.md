# Security policy

transom is an early-preview (alpha) project. Only the latest commit on the default branch is supported; there are no maintained
release branches yet.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's private vulnerability reporting
(*Security > Report a vulnerability* in the repository). Include the Windows build, what you did, and what you expected. Please do not
include private data, tokens or unredacted logs.

This is a volunteer project: there is no guaranteed response time, but reports are taken seriously and acknowledged on a best-effort basis.

## Scope and what is worth reporting

- Parsing of untrusted files: settings import (`transom-config`, `transom-settings`), usage logs of Claude Code / Codex (`transom-usage`),
  the statusLine bridge input.
- Anything that writes outside transom's own data, notably the optional edit of `~/.claude/settings.json` (`transom-usage/src/claude_setup.rs`).
- Misuse of `ShellExecuteW`, `SendInput` or window messages that could be triggered by another program.
- Anything that sends data over the network (by design only HTTPS GETs: the GitHub release list and installer for updates, calendar
  links the user added and the optional weather) or leaks
  private data, such as that link, into logs or exports.

## Known limitations and accepted risks (early preview)

These were found in an internal review and are tracked, not hidden:

- **Same-user local processes** can send the window messages transom uses internally (invoke a bar item, "config changed", "restore")
  or occupy its single-instance mutexes. This is equivalent to what such a process could already do through UI Automation; there is no
  privilege boundary inside one user session.
- **`settings.json` backups** written next to `~/.claude/settings.json` are not rotated and may contain secrets of other tools.
  (Since 2026-11-26 the file is written through a unique `create_new` temporary file, backups are created with `create_new`, and
  rollback only restores a `settings.json.transom-backup-*` file from the same folder.)
- Temporary files in transom's data directory have predictable names (process id and counter); they live in per-user directories.
- `--log FILE` appends without a size limit and can contain media titles and application names; it is off by default.
- The update check compares the installer's Authenticode signer with the running bar's signer by certificate display name only,
  and an unsigned (developer) build skips the signer check; the SHA-256 from the same release is always checked.
- About 250 `unsafe` blocks exist (Win32/COM); all of them in library and program code carry `SAFETY` comments (developer
  examples under `examples/` are not covered).
- Dependencies are limited to `windows`, `serde`, `serde_json` and their dependencies; `cargo deny` checks licenses, sources, yanked
  crates and advisories in CI, but CI has not run on GitHub yet.

Hardened in the 2026-11-26 audit (details and the list of open items in `docs/PLAN.md` §8.34):

- Calendar (ICS) parsing never slices strings on byte offsets, uses overflow-checked arithmetic and bounds rule lists and
  recurrence expansion work, so a hostile feed can neither crash nor freeze the bar.
- HTTPS downloads: TLS 1.2 or newer, at most 5 redirects and never from https to http (the final URL is checked), a size limit
  and a wall-clock limit for the whole download.
- Updates: the installer name and both download URLs must be exactly `https://github.com/Antive1/transom/releases/download/<tag>/<name>`
  of the same release with plain `[A-Za-z0-9._-]` names; the installer is written to a fresh folder with `create_new`, locked
  against modification, re-hashed from disk and kept locked through the signature check and launch. The release page link is
  built from the validated tag.
- `WM_SETTINGCHANGE` text is read only from committed, readable memory (`VirtualQuery`), at most 128 characters.
- The statusLine bridge reads at most 1 MiB from stdin.

Already enforced in code: size limits for imports (256 KiB) and for usage logs (256 MiB) and bridge files (64 MiB), validated
application ids before launching through the shell, no execution of any imported data, no network access except HTTPS downloads of the update check and installer (fixed GitHub release URLs, checksum and publisher
verified), connected calendars and the optional weather (size and time limits), and a fixed allow-list for
exported and imported settings.
