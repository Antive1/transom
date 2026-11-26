# Third-party notices

transom itself is licensed under the [MIT license](LICENSE). It depends on the following third-party Rust crates (from `Cargo.lock`,
all from crates.io; the list was generated with `cargo metadata --offline --locked`). Most likely only `windows*`, `serde*`, `itoa`, `memchr` and
`zmij` end up in the binaries and `syn`, `quote`, `proc-macro2` and `unicode-ident` are build-time (procedural macro) dependencies;
the exact set linked into a binary has not been verified yet.

| Crate | Version | License (SPDX) | Source |
|---|---|---|---|
| itoa | 1.0.19 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| memchr | 2.8.3 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| quote | 1.0.47 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| syn | 2.0.119 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 3.0.10 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| windows | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| zmij | 1.0.23 | MIT | https://github.com/dtolnay/zmij |

`windows` and the `windows-*` crates are published by Microsoft (windows-rs). No Microsoft fonts, icons or binaries are bundled: the UI
refers to the system fonts Segoe UI and Segoe Fluent Icons by name.

## Credits: device protocols (peripheral batteries)

`crates/transom-peripherals` contains **no third-party code**. It was written from protocol facts (message layouts, command numbers)
that these projects and documents made public; facts are credited here, nothing was copied:

- **[OpenRazer](https://github.com/openrazer/openrazer)** (GPL-2.0): the Razer 90-byte report layout, the checksum, transaction ids and
  the battery (0x07 0x80) and charging (0x07 0x84) commands. Verified on a Razer DeathAdder V4 Pro.
- **Logitech HID++ 2.0** as described in Logitech's public HID++ specification documents and by the
  **[Solaar](https://github.com/pwr-Solaar/Solaar)** project (GPL-2.0): message format, the ROOT feature and the DEVICE_NAME, BATTERY_STATUS,
  BATTERY_VOLTAGE and UNIFIED_BATTERY features. Not yet verified on Logitech hardware.
- **[Logitech Battery Tray](https://windhawk.net/mods/logitech-battery-tray)** Windhawk mod by uMk0 (GPL-3.0): inspiration for showing
  peripheral batteries next to the clock. None of its code is used.
- **NuPhy WH80**: the two read commands (GetBase 0x04, GetFunc 0x05), the battery byte and the cable report were found by observing the
  keyboard (2026-11-23) for the project author's own Windhawk mod; no NuPhy software or code is used.
- The Bluetooth battery level is the device property Windows itself shows in *Settings › Bluetooth & devices*
  (`{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2`); this key and the connection flag are not in the Windows SDK headers.

## Status of this file

- This is a **source-repository notice**: it names each dependency and its license. The license texts live in the respective crates
  (see the source links) and at <https://spdx.org/licenses/> (MIT, Apache-2.0, Unlicense, Unicode-3.0).
- **Before distributing binaries**, ship a generated file that includes the full license and copyright texts of every dependency that
  is linked into the binary (for example with `cargo about` or `cargo bundle-licenses`), as MIT and Apache-2.0 require. This has not been
  produced yet; no binaries are published.
- `cargo deny check` (see `deny.toml`) enforces that only MIT, Apache-2.0 and Unicode-3.0 licenses (as accepted choices of the SPDX
  expressions above) are used.
- The repository does not contain third-party sounds, fonts or logos: sounds are generated by `crates/transom-sound/src/synth.rs`,
  and the logo/icons in `assets/brand` are generated from code (`brand_export`, see `docs/DESIGN.md`).
