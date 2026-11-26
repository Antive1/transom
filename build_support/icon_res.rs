//! Vlozenie ikony aplikacie do .exe bez dalsej zavislosti: z `assets/brand/transom.ico` vyrobi binarny `.res` subor
//! (RT_ICON + RT_GROUP_ICON) a linker (MSVC `link.exe`) ho priloží. Pouziva sa z `build.rs` binarnych crate-ov
//! (`#[path = "../../build_support/icon_res.rs"] mod icon_res;`). `.ico` je generovany a commitnuty; tu sa len obali.
use std::path::Path;

/// Zaznam `.res`: hlavicka (DataSize, HeaderSize, typ, nazov, ...) + data zarovnane na 4 bajty.
fn entry(out: &mut Vec<u8>, ty: u16, id: u16, data: &[u8]) {
    let header_size = 32u32; // pevna hlavicka s ordinalovym typom a menom
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_size.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&0x1030u16.to_le_bytes()); // MemoryFlags: MOVEABLE | PURE | DISCARDABLE
    out.extend_from_slice(&0x0409u16.to_le_bytes()); // LanguageId: en-US
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// Binarny `.res` s ikonou (skupina id 1, obrazky id 1..=n) z bajtov `.ico`.
pub fn res_from_ico(ico: &[u8]) -> Vec<u8> {
    assert!(ico.len() >= 6 && ico[0..4] == [0, 0, 1, 0], "neplatny .ico");
    let n = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut res = Vec::new();
    // povinny prazdny zaznam na zaciatku suboru .res
    entry(&mut res, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(n as u16).to_le_bytes());
    for i in 0..n {
        let e = &ico[6 + 16 * i..6 + 16 * (i + 1)];
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let id = (i + 1) as u16;
        entry(&mut res, 3, id, &ico[off..off + len]); // RT_ICON
                                                      // GRPICONDIRENTRY: sirka, vyska, farby, rezervovane, planes, bity, velkost dat, id (14 bajtov)
        group.extend_from_slice(&e[0..8]);
        group.extend_from_slice(&e[8..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut res, 14, 1, &group); // RT_GROUP_ICON
    res
}

/// UTF-16 retazec s nulou na konci.
fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

/// Jeden blok struktury VERSIONINFO: wLength, wValueLength, wType, kluc, hodnota, deti (vsetko zarovnane na 4 bajty).
fn version_block(
    key: &str,
    value: &[u8],
    value_len: u16,
    text: bool,
    children: &[Vec<u8>],
) -> Vec<u8> {
    let mut b = vec![0, 0];
    b.extend_from_slice(&value_len.to_le_bytes());
    b.extend_from_slice(&(text as u16).to_le_bytes());
    b.extend_from_slice(&wide(key));
    pad4(&mut b);
    b.extend_from_slice(value);
    for c in children {
        pad4(&mut b);
        b.extend_from_slice(c);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b
}

/// Data prostriedku RT_VERSION (nazov produktu, verzia, popis) - Prieskumnik ich ukazuje vo Vlastnostiach a podpisovanie
/// (SignPath) ich vyzaduje.
pub fn version_info(version: &str, description: &str, file_name: &str) -> Vec<u8> {
    let mut parts = version
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u16>().unwrap_or(0));
    let mut n = [0u32; 4];
    for x in n.iter_mut().take(3) {
        *x = parts.next().unwrap_or(0) as u32;
    }
    let (ms, ls) = ((n[0] << 16) | n[1], (n[2] << 16) | n[3]);
    let mut fixed = Vec::new();
    for v in [
        0xFEEF_04BDu32, // podpis
        0x0001_0000,    // verzia struktury
        ms,
        ls, // verzia suboru
        ms,
        ls,          // verzia produktu
        0x3F,        // maska priznakov
        0,           // priznaky
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&v.to_le_bytes());
    }
    let strings: Vec<Vec<u8>> = [
        ("CompanyName", "transom"),
        ("FileDescription", description),
        ("FileVersion", version),
        ("InternalName", file_name.trim_end_matches(".exe")),
        ("LegalCopyright", "MIT License"),
        ("OriginalFilename", file_name),
        ("ProductName", "transom"),
        ("ProductVersion", version),
    ]
    .iter()
    .map(|(k, v)| {
        let w = wide(v);
        version_block(k, &w, (w.len() / 2) as u16, true, &[])
    })
    .collect();
    let table = version_block("040904B0", &[], 0, true, &strings);
    let sfi = version_block("StringFileInfo", &[], 0, true, &[table]);
    let translation = [0x09u8, 0x04, 0xB0, 0x04];
    let var = version_block("Translation", &translation, 4, false, &[]);
    let vfi = version_block("VarFileInfo", &[], 0, true, &[var]);
    version_block(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len() as u16,
        false,
        &[sfi, vfi],
    )
}

/// Zapise `.res` (ikona, ak je zadana, a vzdy VERSIONINFO z verzie balika) do `out` a vypise riadky pre cargo (linker +
/// sledovanie zmeny `.ico`). Pri inom nez MSVC cielovom prostredi sa nic neprikladá (ikona je stale nastavena za behu cez
/// `WM_SETICON`).
pub fn emit(ico_path: Option<&Path>, out: &Path, description: &str, file_name: &str) {
    println!("cargo:rerun-if-changed=../../build_support/icon_res.rs");
    let mut res = match ico_path {
        Some(p) => {
            println!("cargo:rerun-if-changed={}", p.display());
            res_from_ico(&std::fs::read(p).expect("assets/brand/transom.ico"))
        }
        None => {
            let mut r = Vec::new();
            entry(&mut r, 0, 0, &[]);
            r
        }
    };
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    entry(
        &mut res,
        16,
        1,
        &version_info(&version, description, file_name),
    ); // RT_VERSION
    std::fs::write(out, res).expect("zapis .res");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins={}", out.display());
    }
}
