# Changelog

All notable changes to transom are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and transom uses [semantic versioning](https://semver.org/).

Each release's section is copied into its GitHub release, and the bar shows it when it offers the update: under
**Added** and **Changed** as "New", under **Fixed** as "Fixes". Write short, user-facing lines, one `-` bullet each.

## [Unreleased]

### Added
- Colours panel: pick a colour anywhere on the screen with a real eyedropper cursor and a magnifier, see its HEX, RGB and
  HSL, copy it, and keep a history of the colours you picked.
- Settings > Updates offers *Update now* as soon as the bar has found a new version.
- Rain and snow on the bar: small falling drops or flakes under the cloud; sun and storm get their own icons.

### Changed
- The battery is green while it charges and red below 20 %.
- Glass in your own bar colour keeps the colour much better: a light colour gives light glass with dark text, and a dark
  colour no longer turns almost black.
- A fully or mostly transparent bar picks dark or light text by what is under it, so it stays readable on a light wallpaper.
- The colour picker no longer shows in the island; it opens the Colours panel.

### Fixed
- Clicking an empty spot or another item on the bar now closes an open panel or island menu.
- On some PCs the rounded corners of cards and buttons in Settings and panels showed as black wedges.
- The bar no longer disappears while the colour picker is active.
- The island missed Codex working when you continued an older Codex thread (for example in VS Code).

## [0.1.2] - 2026-11-26

### Changed
- The glass under panels (and under the bar) takes the hue of your own bar colour, kept dark or light enough for the text.
- Spotify is shown on the bar before a browser when both play or nothing plays (a video that plays on its own still shows).

### Fixed
- The island menu opened as an empty black box on PCs with Windows animations turned off.
- A thin line showed between the bar and the island menu while it opened.
- Panels could flash for a moment while opening.
- After skipping a song the new title and cover could take long to appear (Spotify reports them late; transom now
  reads them again shortly after the change).
- Dragging something to the top of the screen over a browser left the file drop area open; it now closes when the drag
  ends, also when the app you drag from does not report the end (Chromium browsers).
- The Welcome page now really opens on the first start.

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

[Unreleased]: https://github.com/Antive1/transom/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/Antive1/transom/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Antive1/transom/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Antive1/transom/releases/tag/v0.1.0
