# Changelog

All notable changes to transom are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and transom uses [semantic versioning](https://semver.org/).

Each release's section is copied into its GitHub release, and the bar shows it when it offers the update: under
**Added** and **Changed** as "New", under **Fixed** as "Fixes". Write short, user-facing lines, one `-` bullet each.

## [Unreleased]

### Fixed
- The Welcome page now really opens on the first start (the request was queued before the bar could handle it).

## [0.1.1] - 2026-11-26

### Added
- A Welcome page on the first start: start with Windows, the island, glass, AI usage (Claude Code, Codex), weather and
  updates in one place. It stays in Settings › System › Welcome.
- Start with Windows can now be switched on and off in the app (also for the portable copy).

## [0.1.0] - 2026-11-26

### Added
- First public preview: a full-width top bar (AppBar) for Windows 11 – maximized windows stay below it.
- Clock with a calendar popover and events from a private ICS link.
- Media controls with cover art and smoothly scrolling titles.
- A keyboard-first search launcher (Alt+Space) for apps, files, Settings pages and notes.
- Laptop and wireless device batteries (Bluetooth, Razer, Logitech, NuPhy, standard HID).
- Audio panel with output device, volume, per-app mixer, sound profiles and Bluetooth audio.
- Optional extras: system monitor, keyboard layout, timer and pomodoro, monitor brightness, microphone and camera in use,
  weather.
- Optional AI usage estimates from local Claude Code and Codex logs.
- Tools: stashed files, clipboard history, quick notes and a screenshots library.
- Light, dark and custom colours, glass panels, OLED protection, a drag-and-drop bar editor, multi-monitor support and
  11 languages.
- Automatic updates from GitHub releases: a notification and a download icon on the bar open an update panel with what is
  new, and transom downloads, verifies and installs the new version.
- Portable copies open the release page instead of installing.

[Unreleased]: https://github.com/Antive1/transom/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/Antive1/transom/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Antive1/transom/releases/tag/v0.1.0
