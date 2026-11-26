//! Jadro transom bez Win32 a bez renderera: cista logika, ktoru testuju unit testy.
//!
//! - [`item`]: interny model stavovych poloziek (hodiny, bateria, media, Search, Tray, oznam). **Nestabilne, bez ABI**;
//!   ziadne pluginy, WASM ani DLL.
//! - [`layout`]: rozlozenie poloziek na plnosirkovom pase (vlavo/vpravo) vratane spravania pri uzkej sirke.
//! - [`media`]: politika vyberu GSMTC session, platnost casovej osi, ktore tlacidla zobrazit.
//! - [`image`]: obrazok obalu (premultiplikovane ARGB), zmensenie a zlozenie na pozadie.
//! - [`loader`]: obrazky nacitavane na pozadi (miniatury, ikony) s cache ohranicenou v bajtoch.
//! - [`battery`]: viditelnost a stav baterie zo surovych hodnot `GetSystemPowerStatus`.
pub mod battery;
pub mod clock;
pub mod ics;
pub mod image;
pub mod item;
pub mod layout;
pub mod loader;
pub mod marquee;
pub mod media;
pub mod schedule;
