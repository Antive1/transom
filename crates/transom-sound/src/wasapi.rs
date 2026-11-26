//! Skutocne zariadenie: zdielany rezim WASAPI na predvolenom vystupe s **vlastnou relaciou transom**
//! (pevne GUID relacie, nazov "transom UI sounds") a hlasitostou relacie `ISimpleAudioVolume`.
//!
//! Dolezite pravidla:
//! * Nikdy sa nepouziva `IAudioEndpointVolume` (hlavna hlasitost zariadenia ani jeho stlmenie), ani ziadna sprava
//!   hlasitosti ci vyberu vystupu inej aplikacie. Meni sa len hlasitost relacie tohto procesu.
//! * Hlasitost relacie sa nastavi az po vytvoreni relacie a **pred** prvym spustenim prudu a potom len pri zmene
//!   hodnoty z nastaveni (zmena v miesadle Windows pocas behu sa teda neprepisuje pri kazdom kliknuti).
//! * Pouziva sa vlastny format (16-bitove PCM, 44,1 kHz, stereo) s `AUTOCONVERTPCM`; prevod na format zariadenia
//!   urobi Windows.
//! * Chyby sa nikdy nevyhadzuju ako panika (vydanie je `panic = "abort"`): vratia sa ako `BackendError`.
use crate::engine::{Backend, BackendError};
use crate::synth;
use std::time::{Duration, Instant};
use windows::core::{w, GUID};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioClient, IAudioRenderClient, IAudioSessionControl, IMMDeviceEnumerator,
    ISimpleAudioVolume, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
    WAVE_FORMAT_PCM,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// Pevne GUID relacie transom: vsetky zvuky rozhrania tohto procesu su jedna relacia v miesadle hlasitosti.
pub const SESSION_GUID: GUID = GUID::from_u128(0x6f1d3c7e_52a4_4b8e_9d10_7a35c2e4b1f8);

/// Zobrazovany nazov relacie.
const SESSION_NAME: windows::core::PCWSTR = w!("transom UI sounds");

/// Velkost zdielaneho buffera (100 ns jednotky): 200 ms, viac nez zvuk.
const BUFFER_100NS: i64 = 2_000_000;

/// Dalsie cakanie po vyprazdneni buffera: engine este dohrava posledne ms (zvuk konci tichom).
const DRAIN_TAIL: Duration = Duration::from_millis(30);

struct Stream {
    client: IAudioClient,
    render: IAudioRenderClient,
    volume: ISimpleAudioVolume,
    frames: usize,
}

pub struct WasapiBackend {
    com: bool,
    stream: Option<Stream>,
    /// Hlasitost relacie, ktoru sme naposledy nastavili tejto relacii.
    applied: Option<f32>,
}

fn unavailable(e: windows::core::Error) -> BackendError {
    BackendError::Unavailable(format!("{e}"))
}

fn failed(e: windows::core::Error) -> BackendError {
    BackendError::Failed(format!("{e}"))
}

impl WasapiBackend {
    /// Vola sa na pracovnom vlakne: inicializuje COM (MTA) pre toto vlakno.
    pub fn new() -> WasapiBackend {
        // SAFETY: inicializacia COM pre aktualne (nove) vlakno; parter `CoUninitialize` je v `Drop`.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        WasapiBackend {
            com,
            stream: None,
            applied: None,
        }
    }

    fn open() -> Result<Stream, BackendError> {
        // SAFETY: volania COM na vlakne s inicializovanym COM; vsetky ukazovatele su platne pocas volania.
        unsafe {
            let en: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(unavailable)?;
            let dev = en
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(unavailable)?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(unavailable)?;
            let fmt = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: synth::CHANNELS as u16,
                nSamplesPerSec: synth::RATE,
                nAvgBytesPerSec: synth::RATE * synth::CHANNELS as u32 * 2,
                nBlockAlign: synth::CHANNELS as u16 * 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    BUFFER_100NS,
                    0,
                    &fmt,
                    Some(&SESSION_GUID),
                )
                .map_err(failed)?;
            let frames = client.GetBufferSize().map_err(failed)? as usize;
            let render: IAudioRenderClient = client.GetService().map_err(failed)?;
            let volume: ISimpleAudioVolume = client.GetService().map_err(failed)?;
            if let Ok(ctl) = client.GetService::<IAudioSessionControl>() {
                let _ = ctl.SetDisplayName(SESSION_NAME, std::ptr::null());
            }
            Ok(Stream {
                client,
                render,
                volume,
                frames,
            })
        }
    }

    /// Zapise zvuk do buffera po castiach, pusti prud, pocka na dohranie a prud zastavi (relacia ostava otvorena).
    fn render(s: &Stream, clip: &[i16]) -> Result<(), BackendError> {
        let total = clip.len() / synth::CHANNELS;
        let limit = Instant::now() + Duration::from_millis(synth::duration_ms(clip) as u64 + 600);
        let mut done = 0usize;
        let mut started = false;
        // SAFETY: `GetBuffer` vrati zapisovatelny blok `n` ramcov; kopirujeme presne `n * 4` bajtov z platneho
        // useku `clip` (4 bajty = stereo 16 bit) a hned ho uvolnime cez `ReleaseBuffer`.
        unsafe {
            while done < total {
                if Instant::now() > limit {
                    let _ = s.client.Stop();
                    return Err(BackendError::Failed("casovy limit zapisu".into()));
                }
                let pad = s.client.GetCurrentPadding().map_err(failed)? as usize;
                let n = s.frames.saturating_sub(pad).min(total - done);
                if n == 0 {
                    std::thread::sleep(Duration::from_millis(4));
                    continue;
                }
                let p = s.render.GetBuffer(n as u32).map_err(failed)?;
                std::ptr::copy_nonoverlapping(
                    clip.as_ptr().add(done * synth::CHANNELS).cast::<u8>(),
                    p,
                    n * synth::CHANNELS * 2,
                );
                s.render.ReleaseBuffer(n as u32, 0).map_err(failed)?;
                done += n;
                if !started {
                    s.client.Start().map_err(failed)?;
                    started = true;
                }
            }
            while s.client.GetCurrentPadding().map_err(failed)? > 0 {
                if Instant::now() > limit {
                    break;
                }
                std::thread::sleep(Duration::from_millis(4));
            }
            std::thread::sleep(DRAIN_TAIL);
            s.client.Stop().map_err(failed)?;
            s.client.Reset().map_err(failed)?;
        }
        Ok(())
    }
}

impl Default for WasapiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for WasapiBackend {
    fn play(&mut self, clip: &[i16], volume: f32) -> Result<(), BackendError> {
        let volume = volume.clamp(0.0, 1.0);
        // pri zmene predvoleneho zariadenia (alebo jeho odpojeni) je ulozeny prud neplatny: jeden pokus na novo
        for attempt in 0..2 {
            let reused = self.stream.is_some();
            if self.stream.is_none() {
                self.stream = Some(Self::open()?);
                self.applied = None;
            }
            let Some(s) = self.stream.as_ref() else {
                return Err(BackendError::Failed("ziadny prud".into()));
            };
            if self.applied != Some(volume) {
                // SAFETY: platne rozhranie relacie; null = bez kontextu udalosti. Len hlasitost relacie transom.
                match unsafe { s.volume.SetMasterVolume(volume, std::ptr::null()) } {
                    Ok(()) => self.applied = Some(volume),
                    Err(e) => {
                        self.release();
                        if reused && attempt == 0 {
                            continue;
                        }
                        return Err(failed(e));
                    }
                }
            }
            match Self::render(s, clip) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    self.release();
                    if reused && attempt == 0 {
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(BackendError::Failed("prehratie zlyhalo".into()))
    }

    fn release(&mut self) {
        if let Some(s) = self.stream.take() {
            // SAFETY: platny klient; po zastaveni sa rozhrania uvolnia (Drop).
            unsafe {
                let _ = s.client.Stop();
            }
        }
        self.applied = None;
    }
}

impl Drop for WasapiBackend {
    fn drop(&mut self) {
        self.release();
        if self.com {
            // SAFETY: parter k uspesnemu `CoInitializeEx` na tomto vlakne; rozhrania su uz uvolnene.
            unsafe { CoUninitialize() };
        }
    }
}
