//! "Virtualne" subory pri pusteni (`FileGroupDescriptorW` + `FileContents`): prehliadace (obrazok alebo subor zo
//! stranky), postove programy a archivy ponukaju obsah v pamati namiesto cesty na disku. Cita sa az pri pusteni,
//! s limitmi poctu a velkosti; nazvy sa cistia (len nazov suboru, bez ciest a zakazanych znakov).
use windows::core::w;
use windows::Win32::System::Com::{
    IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, STREAM_SEEK_SET, TYMED_HGLOBAL,
    TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::ReleaseStgMedium;

/// Najviac suborov z jedneho pustenia.
pub const MAX_FILES: usize = 32;
/// Najviac bajtov jedneho suboru a vsetkych spolu.
pub const MAX_FILE_BYTES: usize = 200 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 400 * 1024 * 1024;

/// Dlzka zaznamu `FILEDESCRIPTORW` a poloha nazvu v nom.
const FD_SIZE: usize = 592;
const FD_ATTR_OFF: usize = 36;
const FD_NAME_OFF: usize = 72;
const FD_ATTRIBUTES: u32 = 0x4;
const ATTR_DIRECTORY: u32 = 0x10;

fn fmt(name: windows::core::PCWSTR, lindex: i32, tymed: u32) -> FORMATETC {
    // SAFETY: registracia nazvu formatu (opakovane volanie vrati to iste cislo).
    let cf = unsafe { RegisterClipboardFormatW(name) } as u16;
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex,
        tymed,
    }
}

fn descriptor_format() -> FORMATETC {
    fmt(w!("FileGroupDescriptorW"), -1, TYMED_HGLOBAL.0 as u32)
}

/// Ponuka objekt virtualne subory? (nic nenacitava)
pub fn available(d: &IDataObject) -> bool {
    // SAFETY: `QueryGetData` len zisti dostupnost formatu.
    unsafe { d.QueryGetData(&descriptor_format()) }.is_ok()
}

/// Nazvy suborov z bloku `FILEGROUPDESCRIPTORW` (priecinky sa preskocia): cista funkcia na testy.
pub fn parse_descriptor(bytes: &[u8]) -> Vec<(usize, String)> {
    let Some(n) = bytes
        .get(..4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..n.min(MAX_FILES * 4) {
        let off = 4 + i * FD_SIZE;
        let Some(rec) = bytes.get(off..off + FD_SIZE) else {
            break;
        };
        let u32_at = |o: usize| u32::from_le_bytes([rec[o], rec[o + 1], rec[o + 2], rec[o + 3]]);
        let flags = u32_at(0);
        if flags & FD_ATTRIBUTES != 0 && u32_at(FD_ATTR_OFF) & ATTR_DIRECTORY != 0 {
            continue;
        }
        let units: Vec<u16> = rec[FD_NAME_OFF..FD_NAME_OFF + 520]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|u| *u != 0)
            .collect();
        let name = clean_name(&String::from_utf16_lossy(&units));
        out.push((i, name));
        if out.len() >= MAX_FILES {
            break;
        }
    }
    out
}

/// Bezpecny nazov suboru: len posledna cast cesty, bez zakazanych a riadiacich znakov, najviac 120 znakov.
pub fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['\\', '/']).next().unwrap_or("");
    let s: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .take(120)
        .collect();
    let s = s.trim().trim_matches('.').trim().to_string();
    if s.is_empty() {
        "subor".into()
    } else {
        s
    }
}

fn read_hglobal(h: windows::Win32::Foundation::HGLOBAL, max: usize) -> Option<Vec<u8>> {
    // SAFETY: citanie pod zamkom s velkostou z `GlobalSize`.
    unsafe {
        let size = GlobalSize(h);
        if size == 0 || size > max {
            return None;
        }
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(h);
        Some(v)
    }
}

fn read_stream(s: &IStream, max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    // SAFETY: citanie do vlastneho bufra s jeho dlzkou.
    unsafe {
        let _ = s.Seek(0, STREAM_SEEK_SET, None);
        loop {
            let mut got = 0u32;
            let hr = s.Read(buf.as_mut_ptr() as *mut _, buf.len() as u32, Some(&mut got));
            if hr.is_err() {
                return None;
            }
            if got == 0 {
                break;
            }
            out.extend_from_slice(&buf[..got as usize]);
            if out.len() > max {
                return None;
            }
        }
    }
    Some(out)
}

/// Nacita virtualne subory (nazov, obsah). Prazdne = nic pouzitelne (alebo prekrocene limity).
pub fn read(d: &IDataObject) -> Vec<(String, Vec<u8>)> {
    // SAFETY: `GetData` vrati vlastnictvo mediumu, ktore sa uvolni `ReleaseStgMedium`.
    let names = unsafe {
        let Ok(mut m) = d.GetData(&descriptor_format()) else {
            return Vec::new();
        };
        let n = if m.tymed == TYMED_HGLOBAL.0 as u32 {
            read_hglobal(m.u.hGlobal, 4 + FD_SIZE * MAX_FILES * 4)
                .map(|b| parse_descriptor(&b))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        ReleaseStgMedium(&mut m);
        n
    };
    let mut total = 0usize;
    let mut out = Vec::new();
    for (i, name) in names {
        let f = fmt(
            w!("FileContents"),
            i as i32,
            (TYMED_HGLOBAL.0 | TYMED_ISTREAM.0) as u32,
        );
        let left = MAX_TOTAL_BYTES.saturating_sub(total).min(MAX_FILE_BYTES);
        // SAFETY: medium podla `tymed`; uvolni sa vzdy.
        let bytes = unsafe {
            let Ok(mut m) = d.GetData(&f) else {
                continue;
            };
            let b = if m.tymed == TYMED_HGLOBAL.0 as u32 {
                read_hglobal(m.u.hGlobal, left)
            } else if m.tymed == TYMED_ISTREAM.0 as u32 {
                m.u.pstm.as_ref().and_then(|s| read_stream(s, left))
            } else {
                None
            };
            ReleaseStgMedium(&mut m);
            b
        };
        if let Some(b) = bytes {
            total += b.len();
            out.push((name, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, dir: bool) -> Vec<u8> {
        let mut r = vec![0u8; FD_SIZE];
        if dir {
            r[..4].copy_from_slice(&FD_ATTRIBUTES.to_le_bytes());
            r[FD_ATTR_OFF..FD_ATTR_OFF + 4].copy_from_slice(&ATTR_DIRECTORY.to_le_bytes());
        }
        for (k, u) in name.encode_utf16().enumerate() {
            r[FD_NAME_OFF + 2 * k..FD_NAME_OFF + 2 * k + 2].copy_from_slice(&u.to_le_bytes());
        }
        r
    }

    #[test]
    fn descriptors_give_clean_file_names_and_skip_folders() {
        let mut b = 3u32.to_le_bytes().to_vec();
        b.extend(record("fotka.jpg", false));
        b.extend(record("priecinok", true));
        b.extend(record("..\\..\\evil:name?.png", false));
        let v = parse_descriptor(&b);
        assert_eq!(v, vec![(0, "fotka.jpg".into()), (2, "evilname.png".into())]);
        // skrateny blok: len cele zaznamy
        assert_eq!(parse_descriptor(&b[..4 + FD_SIZE + 10]).len(), 1);
        assert!(parse_descriptor(&[1, 0]).is_empty());
    }

    #[test]
    fn names_are_never_paths_or_empty() {
        assert_eq!(clean_name("C:\\x\\y.txt"), "y.txt");
        assert_eq!(clean_name("a/b/c.pdf"), "c.pdf");
        assert_eq!(clean_name(".."), "subor");
        assert_eq!(clean_name(""), "subor");
        assert_eq!(clean_name(&"x".repeat(500)).len(), 120);
    }

    /// Prehliadac: obrazok zo stranky ako `FileGroupDescriptorW` + `FileContents` (bez cesty na disku).
    #[test]
    fn a_browser_like_virtual_file_is_read_with_its_name_and_content() {
        use windows::core::implement;
        use windows::Win32::Foundation::{DV_E_FORMATETC, E_NOTIMPL, S_OK};
        use windows::Win32::System::Com::{IDataObject_Impl, STGMEDIUM, STGMEDIUM_0};
        use windows::Win32::System::Memory::{GlobalAlloc, GMEM_MOVEABLE};

        #[implement(IDataObject)]
        struct Fake {
            desc: Vec<u8>,
            body: Vec<u8>,
        }
        fn global(bytes: &[u8]) -> windows::core::Result<STGMEDIUM> {
            // SAFETY: kopia do novej globalnej pamate, ktoru uvolni prijemca (`ReleaseStgMedium`).
            unsafe {
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
                let p = GlobalLock(h) as *mut u8;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                let _ = GlobalUnlock(h);
                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: h },
                    pUnkForRelease: std::mem::ManuallyDrop::new(None),
                })
            }
        }
        #[allow(non_snake_case)]
        impl IDataObject_Impl for Fake_Impl {
            fn GetData(&self, f: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
                // SAFETY: platny FORMATETC od volajuceho.
                let fe = unsafe { &*f };
                if fe.cfFormat == descriptor_format().cfFormat {
                    global(&self.desc)
                } else if fe.cfFormat == fmt(w!("FileContents"), 0, 0).cfFormat && fe.lindex == 0 {
                    global(&self.body)
                } else {
                    Err(DV_E_FORMATETC.into())
                }
            }
            fn GetDataHere(
                &self,
                _: *const FORMATETC,
                _: *mut STGMEDIUM,
            ) -> windows::core::Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn QueryGetData(&self, f: *const FORMATETC) -> windows::core::HRESULT {
                // SAFETY: platny FORMATETC od volajuceho.
                if unsafe { (*f).cfFormat } == descriptor_format().cfFormat {
                    S_OK
                } else {
                    DV_E_FORMATETC
                }
            }
            fn GetCanonicalFormatEtc(
                &self,
                _: *const FORMATETC,
                _: *mut FORMATETC,
            ) -> windows::core::HRESULT {
                E_NOTIMPL
            }
            fn SetData(
                &self,
                _: *const FORMATETC,
                _: *const STGMEDIUM,
                _: windows::core::BOOL,
            ) -> windows::core::Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn EnumFormatEtc(
                &self,
                _: u32,
            ) -> windows::core::Result<windows::Win32::System::Com::IEnumFORMATETC> {
                Err(E_NOTIMPL.into())
            }
            fn DAdvise(
                &self,
                _: *const FORMATETC,
                _: u32,
                _: windows::core::Ref<'_, windows::Win32::System::Com::IAdviseSink>,
            ) -> windows::core::Result<u32> {
                Err(E_NOTIMPL.into())
            }
            fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
                Err(E_NOTIMPL.into())
            }
            fn EnumDAdvise(
                &self,
            ) -> windows::core::Result<windows::Win32::System::Com::IEnumSTATDATA> {
                Err(E_NOTIMPL.into())
            }
        }
        let mut desc = 1u32.to_le_bytes().to_vec();
        desc.extend(record("obrazok zo stranky.png", false));
        let d: IDataObject = Fake {
            desc,
            body: b"PNGDATA".to_vec(),
        }
        .into();
        assert!(available(&d));
        assert_eq!(
            read(&d),
            vec![("obrazok zo stranky.png".to_string(), b"PNGDATA".to_vec())]
        );
    }
}
