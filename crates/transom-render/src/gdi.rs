//! Backend R0: GDI (vybrany predbezne podla merani S6/S7, PLAN §7.8).
//!
//! `GdiPainter` kresli na uz existujuci `HDC` (typicky z `BeginPaint`); okno ani spravy nevlastni.
//! Pismo vybera `fonts` (Segoe UI Variable s optickymi velkostami a skutocnym polotucnym rezom, zaloha Segoe UI);
//! `CLEARTYPE_QUALITY` (lista je nepriehladna, takze ClearType je spravne), pri priehladnej liste sive vyhladzovanie;
//! ikony Segoe Fluent Icons s `ANTIALIASED_QUALITY`.
use crate::painter::{Color, Font, HAlign, Painter, Rect, Size, TextStyle};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, FillRect, GetClipBox, GetStockObject, GetTextExtentPoint32W, IntersectClipRect,
    RestoreDC, SaveDC, SelectObject, SetBkMode, SetDCBrushColor, SetDIBitsToDevice, SetTextColor,
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLENDFUNCTION, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DC_BRUSH, DEFAULT_CHARSET,
    DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS,
    TRANSPARENT,
};

/// (nazov pisma, velkost px, vaha, sive vyhladzovanie) -> HFONT
type FontKey = (&'static str, i32, i32, bool);
/// pismo -> text -> rozmer (`GetTextExtentPoint32W` zavisi len od pisma a textu).
type MeasureCache = std::collections::HashMap<FontKey, std::collections::HashMap<String, Size>>;
/// Najviac zapamatanych rozmerov textov (spolu pre vsetky pisma).
const MEASURED_MAX: usize = 1024;

/// Text v UTF-16 pre volania GDI: kratky v zasobniku (bez alokacie), dlhsi vo `Vec`.
fn with_wide<R>(text: &str, f: impl FnOnce(&mut [u16]) -> R) -> R {
    let mut stack = [0u16; 256];
    if text.len() <= stack.len() {
        // UTF-16 nema viac jednotiek nez UTF-8 bajtov
        let mut n = 0;
        for (d, u) in stack.iter_mut().zip(text.encode_utf16()) {
            *d = u;
            n += 1;
        }
        f(&mut stack[..n])
    } else {
        let mut v: Vec<u16> = text.encode_utf16().collect();
        f(&mut v)
    }
}

/// (polomer, hrubka obrysu) -> maska rohov.
type CornerCache = std::collections::HashMap<(i32, i32), std::rc::Rc<Vec<u8>>>;

thread_local! {
    static FONTS: std::cell::RefCell<std::collections::HashMap<FontKey, isize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// Masky rohov (polomer, hrubka obrysu; 0 = vypln) -> pokrytie stvorca 2r x 2r: vyhladzuju sa len rohy.
    static CORNERS: std::cell::RefCell<CornerCache> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// Namerane rozmery textov podla pisma (rozlozenie listy meria vsetky texty pri kazdej snimke posuvu a ekvalizera).
    static MEASURED: std::cell::RefCell<MeasureCache> = std::cell::RefCell::new(std::collections::HashMap::new());
    /// Buffer pixelov pre `blit` (pouziva sa znova, najviac `BLIT_KEEP_PX`).
    static BLIT: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Docasny DIB na miesanie s alfou (rohy, znacky, ikony): jeden na vlakno, rastie najviac po `SCRATCH_MAX_*`.
    /// Predtym sa pre kazde miesanie vytvaral a rusil DC aj DIB (zaoblena vypln = 4 rohy = 4 pary za snimku).
    static SCRATCH: std::cell::RefCell<Option<Scratch>> = const { std::cell::RefCell::new(None) };
}

/// Najvacsi zaobleny obdlznik (px plochy) kresleny jednym miesanim: pri vacsom je rychlejsie kreslenie po castiach
/// (namerane: vypln 200 x 60 px 17,5 us jednym miesanim vs. 15,7 us po castiach, 24 x 24 px 4,1 us vs. 15,1 us).
const ONE_BLEND_MAX_PX: i32 = 4096;
/// Najdlhsi text (bajty UTF-8), ktoreho rozmer sa pamata: texty listy su kratke; dlhe texty panelov (schranka,
/// poznamky) sa v pamati nedrzia.
const MEASURED_MAX_LEN: usize = 128;
/// Najvacsi buffer `blit`, ktory sa drzi medzi kresleniami (px).
const BLIT_KEEP_PX: usize = 64 * 64;
/// Najvacsi docasny DIB, ktory sa drzi medzi kresleniami (px); vacsie miesanie si vytvori vlastny na jedno pouzitie.
const SCRATCH_MAX_W: i32 = 256;
const SCRATCH_MAX_H: i32 = 128;

/// 32-bitovy DIB zhora nadol s vlastnym pamatovym DC (zdroj pre `AlphaBlend`).
struct Scratch {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Scratch {
    fn new(w: i32, h: i32) -> Option<Scratch> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: pamatovy DC a DIB sekcia; pri chybe sa uvolni, co sa vytvorilo. `bits` zije s `bmp`.
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut p, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            if p.is_null() {
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp.into());
            Some(Scratch {
                dc,
                bmp,
                old,
                bits: p as *mut u32,
                w,
                h,
            })
        }
    }

    /// Pixely DIB (`w * h`, riadok ma `w` px).
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `bits` ukazuje na `w * h` px DIB sekcie, ktora zije, kym zije `Scratch`; vyhradne cez `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // SAFETY: objekty vytvorene v `new`; povodny objekt DC sa vrati pred zrusenim.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Pokrytie stvorca `2r x 2r` (vypln s polomerom `r`, alebo obrys hrubky `border`): rohy zaobleneho obdlznika su jeho
/// stvrtiny. Rovnake hodnoty ako `round_rect_coverage` celeho obdlznika (stred oblucika je na tom istom mieste).
fn corner_mask(r: i32, border: i32) -> std::rc::Rc<Vec<u8>> {
    if let Some(m) = CORNERS.with(|c| c.borrow().get(&(r, border)).cloned()) {
        return m;
    }
    let d = 2 * r;
    let mut m = crate::icons::round_rect_coverage(d, d, r as f32);
    if border > 0 && d > 2 * border {
        let iw = d - 2 * border;
        let inner = crate::icons::round_rect_coverage(iw, iw, (r - border).max(0) as f32);
        for y in 0..iw {
            for x in 0..iw {
                let o = ((y + border) * d + x + border) as usize;
                m[o] = m[o].saturating_sub(inner[(y * iw + x) as usize]);
            }
        }
    }
    let m = std::rc::Rc::new(m);
    CORNERS.with(|c| c.borrow_mut().insert((r, border), m.clone()));
    m
}

impl GdiPainter {
    /// Obdlznik je cely mimo oblasti kreslenia (prekreslovany vyrez, posunuty obsah stranky): netreba ho kreslit.
    fn hidden(&self, r: Rect) -> bool {
        let mut rc = RECT::default();
        // SAFETY: platny HDC; `rc` je vystupny parameter.
        let kind = unsafe { GetClipBox(self.hdc, &mut rc) };
        match kind.0 {
            1 => true,
            2 | 3 => {
                r.x >= rc.right || r.y >= rc.bottom || r.x + r.w <= rc.left || r.y + r.h <= rc.top
            }
            _ => false,
        }
    }

    /// Jeden roh (stvrtina masky) vyfarbeny `color`: `qx`, `qy` = ktora stvrtina (0 = lava / horna).
    fn corner(&mut self, mask: &[u8], r: i32, at: (i32, i32), q: (i32, i32), color: Color) {
        let d = 2 * r;
        self.blend_rows(Rect::new(at.0, at.1, r, r), |y, row| {
            let from = ((y + q.1 * r) * d + q.0 * r) as usize;
            for (o, &a) in row.iter_mut().zip(&mask[from..from + r as usize]) {
                *o = premul(color.r, color.g, color.b, a);
            }
        });
    }

    /// Maly zaobleny obdlznik jednym miesanim (rohy zo stvrtin masky `2r x 2r`, ostatok z `rest(x, y)`; 255 = farba,
    /// 0 = bez zmeny) namiesto troch az styroch vyplni a styroch miesani rohov - pri ekvalizeri 5 stlpcov x 7 volani GDI
    /// za snimku. Vysledok RGB je ten isty (plne kryty pixel = presne farba, nulovy = podklad; nepouzivany horny bajt
    /// BGRX je 255 ako pri rohoch, nie 0 ako po `FillRect`). `false` = obdlznik je
    /// prilis velky pre docasny DIB (kresli sa po castiach).
    fn round_in_one_blend(
        &mut self,
        rect: Rect,
        r: i32,
        mask: &[u8],
        color: Color,
        rest: impl Fn(i32, i32) -> u8,
    ) -> bool {
        if !self.one_blend
            || rect.w > SCRATCH_MAX_W
            || rect.h > SCRATCH_MAX_H
            || rect.w * rect.h > ONE_BLEND_MAX_PX
        {
            return false;
        }
        let (w, h, d) = (rect.w, rect.h, 2 * r);
        let full = premul(color.r, color.g, color.b, 255);
        self.blend_rows(rect, |y, row| {
            // riadok rohov: horne (y < r) a dolne (y >= h - r) stvrtiny masky
            let my = if y < r {
                Some(y)
            } else if y >= h - r {
                Some(y - (h - r) + r)
            } else {
                None
            };
            for (x, o) in row.iter_mut().enumerate() {
                let x = x as i32;
                let a = match my {
                    Some(my) if x < r => mask[(my * d + x) as usize],
                    Some(my) if x >= w - r => mask[(my * d + x - (w - r) + r) as usize],
                    _ => rest(x, y),
                };
                *o = if a == 255 {
                    full
                } else {
                    premul(color.r, color.g, color.b, a)
                };
            }
        });
        true
    }

    /// Zmiesa obdlznik nad obsah DC: `row(y, px)` naplni riadok `y` (sirka `rect.w`) premultiplikovanymi ARGB pixelmi.
    /// Zdroj je docasny DIB vlakna (alebo jednorazovy pri velkom obdlzniku); cely mimo orezania sa nerobi nic.
    fn blend_rows(&mut self, rect: Rect, mut row: impl FnMut(i32, &mut [u32])) {
        if rect.w <= 0 || rect.h <= 0 || self.hidden(rect) {
            return;
        }
        let fits = rect.w <= SCRATCH_MAX_W && rect.h <= SCRATCH_MAX_H;
        SCRATCH.with(|cell| {
            let mut cell = cell.borrow_mut();
            let mut once;
            let s = if fits {
                if cell.as_ref().is_none_or(|s| s.w < rect.w || s.h < rect.h) {
                    // rastie po mocninach dvoch (najviac po hranicu), aby sa pri roznych velkostiach nevytvaral stale znova
                    let (ow, oh) = cell.as_ref().map_or((0, 0), |s| (s.w, s.h));
                    let grow = |v: i32, max: i32| {
                        (v.max(16) as u32).next_power_of_two().min(max as u32) as i32
                    };
                    let (w, h) = (
                        grow(ow.max(rect.w), SCRATCH_MAX_W),
                        grow(oh.max(rect.h), SCRATCH_MAX_H),
                    );
                    *cell = None;
                    *cell = Scratch::new(w, h);
                }
                cell.as_mut()
            } else {
                once = Scratch::new(rect.w, rect.h);
                once.as_mut()
            };
            let Some(s) = s else {
                return;
            };
            let stride = s.w as usize;
            // pamatovy DC (vsetky nase buffre): miesanie v softveri - `AlphaBlend` niektore ovladace (napr. pri HDR)
            // vykonaju bez alfy a priehladne body (rohy kariet a tlacidiel) by ostali cierne
            // SAFETY: platny HDC; typ objektu sa len cita.
            let memory = unsafe {
                windows::Win32::Graphics::Gdi::GetObjectType(HGDIOBJ(self.hdc.0))
                    == windows::Win32::Graphics::Gdi::OBJ_MEMDC.0 as u32
            };
            if memory {
                let n = rect.w as usize;
                let mut src = vec![0u32; n * rect.h as usize];
                for y in 0..rect.h {
                    let at = y as usize * n;
                    row(y, &mut src[at..at + n]);
                }
                // SAFETY: oba DC su platne; obdlznik lezi v DIB (`rect.w <= s.w`, `rect.h <= s.h`).
                let read = unsafe {
                    windows::Win32::Graphics::Gdi::BitBlt(
                        s.dc,
                        0,
                        0,
                        rect.w,
                        rect.h,
                        Some(self.hdc),
                        rect.x,
                        rect.y,
                        windows::Win32::Graphics::Gdi::SRCCOPY,
                    )
                    .is_ok()
                };
                if read {
                    // SAFETY: dokoncenie GDI operacii pred citanim bodov DIB.
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::GdiFlush();
                    }
                    let px = s.pixels();
                    for y in 0..rect.h as usize {
                        for x in 0..n {
                            let d = &mut px[y * stride + x];
                            *d = over_premul(src[y * n + x], *d);
                        }
                    }
                    // SAFETY: ako vyssie; zapis do nasho DC respektuje jeho orezanie.
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::BitBlt(
                            self.hdc,
                            rect.x,
                            rect.y,
                            rect.w,
                            rect.h,
                            Some(s.dc),
                            0,
                            0,
                            windows::Win32::Graphics::Gdi::SRCCOPY,
                        );
                    }
                    return;
                }
                // citanie zlyhalo: spat na `AlphaBlend` s uz pripravenymi bodmi
                let px = s.pixels();
                for y in 0..rect.h as usize {
                    px[y * stride..y * stride + n].copy_from_slice(&src[y * n..(y + 1) * n]);
                }
            } else {
                let px = s.pixels();
                for y in 0..rect.h {
                    let at = y as usize * stride;
                    row(y, &mut px[at..at + rect.w as usize]);
                }
            }
            let bf = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            // SAFETY: oba DC su platne; zdrojovy obdlznik lezi v DIB (`rect.w <= s.w`, `rect.h <= s.h`).
            unsafe {
                let _ = AlphaBlend(
                    self.hdc, rect.x, rect.y, rect.w, rect.h, s.dc, 0, 0, rect.w, rect.h, bf,
                );
            }
        });
    }
}

/// Premultiplikovany bod `src` nad `dst` (ako `AlphaBlend` s `AC_SRC_ALPHA`, vratane alfy ciela).
fn over_premul(src: u32, dst: u32) -> u32 {
    let sa = src >> 24;
    if sa == 0 {
        return dst;
    }
    if sa == 255 {
        return src;
    }
    let inv = 255 - sa;
    let ch = |sh: u32| {
        let v = ((src >> sh) & 0xFF) + (((dst >> sh) & 0xFF) * inv + 127) / 255;
        v.min(255) << sh
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Premultiplikovany ARGB pixel (`AlphaBlend` s `AC_SRC_ALPHA`).
fn premul(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let a = a as u32;
    let m = |v: u8| (v as u32 * a + 127) / 255;
    a << 24 | m(r) << 16 | m(g) << 8 | m(b)
}

pub struct GdiPainter {
    hdc: HDC,
    size: Size,
    dpi: u32,
    /// Sive vyhladzovanie namiesto ClearType: pre vrstvene okno (priehladne pozadie), kde sa farba pixelu
    /// pocita voci znamemu pozadiu a ClearType (farebne okraje podla subpixelov) by nesedel.
    grayscale: bool,
    /// Male zaoblene obdlzniky jednym miesanim (vypnute len v testoch na porovnanie s kreslenim po castiach).
    one_blend: bool,
}

fn colorref(c: Color) -> COLORREF {
    COLORREF(c.r as u32 | (c.g as u32) << 8 | (c.b as u32) << 16)
}

fn to_rect(r: Rect) -> RECT {
    RECT {
        left: r.x,
        top: r.y,
        right: r.x + r.w,
        bottom: r.y + r.h,
    }
}

impl GdiPainter {
    /// `hdc` musi byt platny po celu zivotnost painteru; `size` je rozmer povrchu v px, `dpi` DPI monitora.
    pub fn new(hdc: HDC, size: Size, dpi: u32) -> Self {
        GdiPainter {
            hdc,
            size,
            dpi,
            grayscale: false,
            one_blend: true,
        }
    }

    /// Sive vyhladzovanie textu namiesto ClearType (pozri pole `grayscale`).
    pub fn grayscale(mut self, on: bool) -> Self {
        self.grayscale = on;
        self
    }

    /// Font z cache (vlakno UI): vytvara sa len raz pre kombinaciu (pismo, velkost v px, vaha, vyhladzovanie). Pocet
    /// kombinacii je maly a ohraniceny (niekolko velkosti x DPI monitorov), fonty zostavaju do konca procesu.
    fn make_font(&self, style: &TextStyle, text: &str) -> HFONT {
        self.font_for(self.font_key(style, text), style.font)
    }

    /// Kluc pisma pre text: (nazov, velkost v px, vaha, sive vyhladzovanie).
    fn font_key(&self, style: &TextStyle, text: &str) -> FontKey {
        let px = (style.size_lp * self.scale()).round() as i32;
        // sive vyhladzovanie (priehladna lista nad lubovolnou plochou): klasicke Segoe UI - jeho hinting drzi tahy na celych
        // pixeloch, slabo hintovane Segoe UI Variable je v sivom vyhladzovani pri 13 px citelne tensie a bledsie
        let family = if self.grayscale {
            crate::fonts::Family::Classic
        } else {
            crate::fonts::family()
        };
        let (name, weight) =
            crate::fonts::face_for_text(family, style.font, style.size_lp, style.bold, text);
        (name, px, weight, self.grayscale)
    }

    fn font_for(&self, key: FontKey, font: Font) -> HFONT {
        if let Some(h) = FONTS.with(|c| c.borrow().get(&key).copied()) {
            return HFONT(h as *mut _);
        }
        let f = self.create_font(font, key.0, key.2, key.1);
        FONTS.with(|c| c.borrow_mut().insert(key, f.0 as isize));
        f
    }

    fn create_font(&self, font: Font, name: &str, weight: i32, px: i32) -> HFONT {
        let face: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: nazov pisma je retazec ukonceny nulou zijuci pocas volania; pismo vlastni cache malovania.
        unsafe {
            CreateFontW(
                -px,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                // ikony v odtienoch sivej: ClearType na tenkych zvislych tahoch (pauza) robi farebne okraje
                match font {
                    Font::Ui | Font::Tabular if !self.grayscale => CLEARTYPE_QUALITY,
                    _ => ANTIALIASED_QUALITY,
                },
                DEFAULT_PITCH.0 as u32,
                PCWSTR(face.as_ptr()),
            )
        }
    }
}

impl Painter for GdiPainter {
    fn size(&self) -> Size {
        self.size
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let rc = to_rect(rect);
        // SAFETY: platny HDC; stetec DC (DC_BRUSH) je systemovy objekt - nic sa nevytvara ani neuvolnuje.
        unsafe {
            SetDCBrushColor(self.hdc, colorref(color));
            FillRect(self.hdc, &rc, HBRUSH(GetStockObject(DC_BRUSH).0));
        }
    }

    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size {
        if text.is_empty() {
            return Size { w: 0, h: 0 };
        }
        let key = self.font_key(style, text);
        if let Some(sz) = MEASURED.with(|m| m.borrow().get(&key).and_then(|t| t.get(text)).copied())
        {
            return sz;
        }
        let mut sz = SIZE::default();
        // SAFETY: pismo sa vyberie a vrati spat; text je rez s jeho dlzkou.
        with_wide(text, |wide| unsafe {
            let font = self.font_for(key, style.font);
            let old = SelectObject(self.hdc, font.into());
            let _ = GetTextExtentPoint32W(self.hdc, wide, &mut sz);
            SelectObject(self.hdc, old);
            // font patri cache, neuvolnovat
        });
        let sz = Size { w: sz.cx, h: sz.cy };
        if text.len() > MEASURED_MAX_LEN {
            return sz;
        }
        MEASURED.with(|m| {
            let mut m = m.borrow_mut();
            // ohranicene: pri prilis mnohych textoch (vysledky hladania, schranka) sa pamat zacne odznova
            if m.values().map(|t| t.len()).sum::<usize>() >= MEASURED_MAX {
                m.clear();
            }
            m.entry(key).or_default().insert(text.to_string(), sz);
        });
        sz
    }

    fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle) {
        // prazdny text sa nekresli: DrawTextW s prazdnym buffrom (dangling ukazovatel) padne (overene)
        if text.is_empty() || self.hidden(rect) {
            return;
        }
        let mut rc = to_rect(rect);
        let align = match style.align {
            HAlign::Left => DT_LEFT,
            HAlign::Center => DT_CENTER,
            HAlign::Right => DT_RIGHT,
        };
        // SAFETY: pismo sa vyberie a vrati spat; text je rez s jeho dlzkou, obdlznik je lokalny.
        with_wide(text, |wide| unsafe {
            let font = self.make_font(style, text);
            let old = SelectObject(self.hdc, font.into());
            SetBkMode(self.hdc, TRANSPARENT);
            SetTextColor(self.hdc, colorref(style.color));
            DrawTextW(
                self.hdc,
                wide,
                &mut rc,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | align,
            );
            SelectObject(self.hdc, old);
            // font patri cache, neuvolnovat
        });
    }

    fn draw_text_scrolled(&mut self, text: &str, clip: Rect, style: &TextStyle, offset_px: i32) {
        if clip.w <= 0 || clip.h <= 0 || text.is_empty() || self.hidden(clip) {
            return;
        }
        // sirka celeho textu: kreslime ho cely, orezanie robi clip region (bez "..." na okraji)
        let full = self.measure_text(text, style).w;
        let mut rc = RECT {
            left: clip.x - offset_px,
            top: clip.y,
            right: clip.x - offset_px + full + 4,
            bottom: clip.y + clip.h,
        };
        // SAFETY: stav DC sa ulozi a obnovi (SaveDC/RestoreDC); pismo sa vrati spat.
        with_wide(text, |wide| unsafe {
            let saved = SaveDC(self.hdc);
            IntersectClipRect(self.hdc, clip.x, clip.y, clip.x + clip.w, clip.y + clip.h);
            let font = self.make_font(style, text);
            let old = SelectObject(self.hdc, font.into());
            SetBkMode(self.hdc, TRANSPARENT);
            SetTextColor(self.hdc, colorref(style.color));
            DrawTextW(
                self.hdc,
                wide,
                &mut rc,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT,
            );
            SelectObject(self.hdc, old);
            let _ = RestoreDC(self.hdc, saved);
        });
    }

    fn blend_rgba(&mut self, rect: Rect, pixels: &[crate::painter::Rgba]) {
        if rect.w <= 0 || rect.h <= 0 || pixels.len() != (rect.w * rect.h) as usize {
            return;
        }
        let w = rect.w as usize;
        self.blend_rows(rect, |y, row| {
            let src = &pixels[y as usize * w..(y as usize + 1) * w];
            for (o, c) in row.iter_mut().zip(src) {
                *o = premul(c.r, c.g, c.b, c.a);
            }
        });
    }

    fn blend_mask(&mut self, rect: Rect, cov: &[u8], color: Color) {
        if rect.w <= 0 || rect.h <= 0 || cov.len() != (rect.w * rect.h) as usize {
            return;
        }
        let w = rect.w as usize;
        self.blend_rows(rect, |y, row| {
            let src = &cov[y as usize * w..(y as usize + 1) * w];
            for (o, &a) in row.iter_mut().zip(src) {
                *o = premul(color.r, color.g, color.b, a);
            }
        });
    }

    fn push_clip(&mut self, rect: Rect) {
        // SAFETY: platny HDC; SaveDC/RestoreDC tvoria zasobnik (parovy `pop_clip`).
        unsafe {
            SaveDC(self.hdc);
            IntersectClipRect(self.hdc, rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
        }
    }

    fn pop_clip(&mut self) {
        // SAFETY: parovy k `push_clip`; -1 = posledne ulozeny stav.
        unsafe {
            let _ = RestoreDC(self.hdc, -1);
        }
    }

    fn clip_box(&self) -> Option<Rect> {
        let mut rc = RECT::default();
        // SAFETY: platny HDC po celu zivotnost painteru; `rc` je vystupny parameter.
        let kind = unsafe { GetClipBox(self.hdc, &mut rc) };
        // 0 = chyba, 1 = prazdna oblast (NULLREGION): v oboch pripadoch sa nic neprekresluje / nevieme
        match kind.0 {
            2 | 3 => Some(Rect::new(
                rc.left,
                rc.top,
                rc.right - rc.left,
                rc.bottom - rc.top,
            )),
            _ => None,
        }
    }

    /// Zaoblena vypln: vnutro obycajnymi obdlznikmi (rychle), vyhladzuju sa len styri rohy (maska z cache).
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        if rect.w <= 0 || rect.h <= 0 || self.hidden(rect) {
            return;
        }
        let r = radius.clamp(0, rect.w.min(rect.h) / 2);
        if r <= 0 {
            self.fill_rect(rect, color);
            return;
        }
        let m = corner_mask(r, 0);
        if self.round_in_one_blend(rect, r, &m, color, |_, _| 255) {
            return;
        }
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        self.fill_rect(Rect::new(x, y + r, w, h - 2 * r), color);
        self.fill_rect(Rect::new(x + r, y, w - 2 * r, r), color);
        self.fill_rect(Rect::new(x + r, y + h - r, w - 2 * r, r), color);
        self.corner(&m, r, (x, y), (0, 0), color);
        self.corner(&m, r, (x + w - r, y), (1, 0), color);
        self.corner(&m, r, (x, y + h - r), (0, 1), color);
        self.corner(&m, r, (x + w - r, y + h - r), (1, 1), color);
    }

    /// Obrys zaobleneho obdlznika: rovne hrany obdlznikmi, vyhladene len rohy.
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, border: i32, color: Color) {
        if rect.w <= 2 * border || rect.h <= 2 * border || border <= 0 || self.hidden(rect) {
            return;
        }
        let r = radius.clamp(0, rect.w.min(rect.h) / 2).max(border);
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let m = corner_mask(r, border);
        let edge = |px: i32, py: i32| {
            let band = py < border || py >= h - border || px < border || px >= w - border;
            if band {
                255
            } else {
                0
            }
        };
        if self.round_in_one_blend(rect, r, &m, color, edge) {
            return;
        }
        self.fill_rect(Rect::new(x + r, y, w - 2 * r, border), color);
        self.fill_rect(Rect::new(x + r, y + h - border, w - 2 * r, border), color);
        self.fill_rect(Rect::new(x, y + r, border, h - 2 * r), color);
        self.fill_rect(Rect::new(x + w - border, y + r, border, h - 2 * r), color);
        self.corner(&m, r, (x, y), (0, 0), color);
        self.corner(&m, r, (x + w - r, y), (1, 0), color);
        self.corner(&m, r, (x, y + h - r), (0, 1), color);
        self.corner(&m, r, (x + w - r, y + h - r), (1, 1), color);
    }

    fn blit(&mut self, rect: Rect, pixels: &[Color]) {
        if rect.w <= 0
            || rect.h <= 0
            || pixels.len() != (rect.w * rect.h) as usize
            || self.hidden(rect)
        {
            return;
        }
        // 32 bpp BGRX, zaporna vyska = riadky zhora nadol; buffer vlakna sa pouziva znova (ikony a obal pri kazdej snimke)
        let mut bits = BLIT.with(|b| std::mem::take(&mut *b.borrow_mut()));
        bits.clear();
        bits.extend(
            pixels
                .iter()
                .map(|c| (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32),
        );
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rect.w,
                biHeight: -rect.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: zdrojove pixely maju `w * h` prvkov podla hlavicky BITMAPINFO.
        unsafe {
            SetDIBitsToDevice(
                self.hdc,
                rect.x,
                rect.y,
                rect.w as u32,
                rect.h as u32,
                0,
                0,
                0,
                rect.h as u32,
                bits.as_ptr() as *const _,
                &bmi,
                DIB_RGB_COLORS,
            );
        }
        // velky buffer (cely obrazok) sa nedrzi
        if bits.capacity() <= BLIT_KEEP_PX {
            BLIT.with(|b| *b.borrow_mut() = bits);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(colorref(Color::hex(0x1E2228)).0, 0x0028_221E);
    }

    #[test]
    fn empty_text_is_safe_to_draw_and_measure() {
        // regresia: DrawTextW s prazdnym buffrom padal (prazdny nazov skladby)
        use windows::Win32::Graphics::Gdi::{CreateCompatibleDC, DeleteDC};
        // SAFETY: pamatovy DC pre test, uvolni sa na konci.
        let dc = unsafe { CreateCompatibleDC(None) };
        let mut p = GdiPainter::new(dc, Size { w: 100, h: 28 }, 96);
        let st = TextStyle {
            font: Font::Ui,
            size_lp: 12.0,
            bold: false,
            color: Color::rgb(0, 0, 0),
            align: HAlign::Left,
        };
        p.draw_text("", Rect::new(0, 0, 100, 28), &st);
        p.draw_text_scrolled("", Rect::new(0, 0, 100, 28), &st, 5);
        assert_eq!(p.measure_text("", &st), Size { w: 0, h: 0 });
        // SAFETY: DC vytvoreny vyssie v tomto teste, uvolni sa raz.
        let _ = unsafe { DeleteDC(dc) };
    }

    /// Maly DIB (BGRX) s DC pre testy kreslenia.
    fn dib(w: i32, h: i32) -> (HDC, windows::Win32::Graphics::Gdi::HBITMAP, *mut u32) {
        use windows::Win32::Graphics::Gdi::{CreateCompatibleDC, CreateDIBSection};
        // SAFETY: DC a DIB pre test; uvolni ich volajuci test.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            SelectObject(dc, bmp.into());
            (dc, bmp, bits as *mut u32)
        }
    }

    #[test]
    fn software_blending_matches_premultiplied_source_over() {
        // nulova alfa = ciel nezmeneny (rohy kariet nesmu sciernet), plna = zdroj, polovica = zmes
        assert_eq!(over_premul(0, 0x80FF_FFFF), 0x80FF_FFFF);
        assert_eq!(over_premul(0xFF10_2030, 0xFFFF_FFFF), 0xFF10_2030);
        let m = over_premul(premul(0, 0, 0, 128), 0xFFFF_FFFF);
        assert_eq!(m >> 24, 255);
        assert!(((m & 0xFF) as i32 - 127).abs() <= 1, "{m:#x}");
    }

    #[test]
    fn blend_rgba_mixes_over_existing_content_and_respects_clip() {
        use crate::painter::Rgba;
        let (dc, bmp, bits) = dib(8, 8);
        // SAFETY: `i` je v rozsahu 8 x 8 pixelov DIB z `dib(8, 8)`.
        let px = |i: usize| unsafe { *bits.add(i) } & 0x00FF_FFFF;
        let mut p = GdiPainter::new(dc, Size { w: 8, h: 8 }, 96);
        p.fill_rect(Rect::new(0, 0, 8, 8), Color::rgb(255, 255, 255));
        // polopriehladna cierna nad bielou = ~sedá; plne krytá = cierna; nulova alfa = nezmenene
        let c = |r, g, b, a| Rgba { r, g, b, a };
        let mut img = vec![c(0, 0, 0, 0); 4];
        img[0] = c(0, 0, 0, 128);
        img[1] = c(0, 0, 0, 255);
        img[2] = c(255, 0, 0, 255);
        p.blend_rgba(Rect::new(0, 0, 2, 2), &img);
        let g = px(0) & 0xFF;
        assert!((g as i32 - 127).abs() <= 2, "{g:#x}");
        assert_eq!(px(1), 0);
        assert_eq!(px(8), 0x00FF_0000, "cervena (BGRX: R v hornych 8 bitoch)");
        assert_eq!(px(9), 0x00FF_FFFF, "alfa 0 nezmeni podklad");
        // zly pocet pixelov = nic
        p.blend_rgba(Rect::new(4, 4, 2, 2), &img[..3]);
        assert_eq!(px(4 * 8 + 4), 0x00FF_FFFF);
        // orezanie: kreslenie mimo obdlznika sa nepreja; po pop_clip sa kresli vsade
        p.push_clip(Rect::new(0, 0, 4, 4));
        p.fill_rect(Rect::new(0, 0, 8, 8), Color::rgb(0, 0, 255));
        p.pop_clip();
        assert_eq!(px(0), 0x0000_00FF);
        assert_eq!(px(7 * 8 + 7), 0x00FF_FFFF, "mimo orezu ostalo");
        p.fill_rect(Rect::new(6, 6, 2, 2), Color::rgb(0, 255, 0));
        assert_eq!(px(7 * 8 + 7), 0x0000_FF00, "po pop_clip sa kresli normalne");
        // zaoblena vypln: stred plny, roh zmiesany s podkladom
        p.fill_rect(Rect::new(0, 0, 8, 8), Color::rgb(255, 255, 255));
        p.fill_round_rect(Rect::new(0, 0, 8, 8), 3, Color::rgb(0, 0, 0));
        assert_eq!(px(4 * 8 + 4), 0);
        assert!(px(0) & 0xFF > 150, "roh ostal svetly");
        // SAFETY: objekty z `dib`, uvolnia sa raz.
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(dc);
        }
    }

    #[test]
    fn blends_through_the_shared_and_the_one_off_buffer_give_the_same_pixels() {
        let (w, h) = (SCRATCH_MAX_W + 40, 6);
        let (dc, bmp, bits) = dib(w, h);
        let px = |i: i32| unsafe { *bits.add(i as usize) } & 0x00FF_FFFF;
        let mut p = GdiPainter::new(dc, Size { w, h }, 96);
        p.fill_rect(Rect::new(0, 0, w, h), Color::rgb(255, 255, 255));
        let cov: Vec<u8> = (0..w * 3).map(|i| (i % 256) as u8).collect();
        // siroky pas (jednorazovy DIB), potom ten isty riadok po kuskoch cez zdielany DIB
        p.blend_mask(Rect::new(0, 0, w, 3), &cov, Color::rgb(0, 0, 0));
        for x0 in (0..w).step_by(50) {
            let n = 50.min(w - x0);
            let part: Vec<u8> = (0..3)
                .flat_map(|y| cov[(y * w + x0) as usize..(y * w + x0 + n) as usize].to_vec())
                .collect();
            p.blend_mask(Rect::new(x0, 3, n, 3), &part, Color::rgb(0, 0, 0));
        }
        for y in 0..3 {
            for x in 0..w {
                assert_eq!(px(y * w + x), px((y + 3) * w + x), "x={x} y={y}");
            }
        }
        assert_eq!(px(0), 0x00FF_FFFF, "alfa 0 nezmeni podklad");
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(dc);
        }
    }

    #[test]
    fn remembered_text_sizes_match_gdi_and_stay_bounded() {
        use windows::Win32::Graphics::Gdi::{CreateCompatibleDC, DeleteDC};
        let dc = unsafe { CreateCompatibleDC(None) };
        let mut p = GdiPainter::new(dc, Size { w: 100, h: 28 }, 144);
        let st = |font, bold| TextStyle {
            font,
            size_lp: 12.0,
            bold,
            color: Color::rgb(0, 0, 0),
            align: HAlign::Left,
        };
        for (text, style) in [
            ("14:05", st(Font::Tabular, false)),
            ("An extremely long title", st(Font::Ui, true)),
            ("Spotify 日本語", st(Font::Ui, false)),
            ("\u{E767}", st(Font::Icons, false)),
        ] {
            let key = p.font_key(&style, text);
            let wide: Vec<u16> = text.encode_utf16().collect();
            let mut direct = SIZE::default();
            unsafe {
                let old = SelectObject(dc, p.font_for(key, style.font).into());
                let _ = GetTextExtentPoint32W(dc, &wide, &mut direct);
                SelectObject(dc, old);
            }
            let first = p.measure_text(text, &style);
            assert_eq!((first.w, first.h), (direct.cx, direct.cy), "{text}");
            assert_eq!(p.measure_text(text, &style), first, "z pamate: {text}");
            // ina vaha je iny kluc (ikony vahu nemaju)
            assert!(
                style.font == Font::Icons
                    || p.font_key(&style, text) != p.font_key(&st(style.font, !style.bold), text)
            );
        }
        let long = "x".repeat(MEASURED_MAX_LEN + 1);
        p.measure_text(&long, &st(Font::Ui, false));
        let kept = MEASURED.with(|m| m.borrow().values().any(|t| t.contains_key(&long)));
        assert!(!kept, "dlhy text sa nepamata");
        for i in 0..MEASURED_MAX + 50 {
            p.measure_text(&format!("t{i}"), &st(Font::Ui, false));
        }
        let n = MEASURED.with(|m| m.borrow().values().map(|t| t.len()).sum::<usize>());
        assert!(n <= MEASURED_MAX, "{n}");
        let _ = unsafe { DeleteDC(dc) };
    }

    #[test]
    fn rounded_fills_and_outlines_in_one_blend_equal_the_piecewise_drawing() {
        let (w, h) = (120, 60);
        let (dc, bmp, bits) = dib(w, h);
        // len RGB: horny bajt (X v BGRX) GDI pri FillRect nuluje a AlphaBlend plni; ziadna cesta ho necita (vrstvene
        // okno a sklo skladaju alfu z dvoch prechodov RGB)
        let snap = || unsafe {
            std::slice::from_raw_parts(bits, (w * h) as usize)
                .iter()
                .map(|p| p & 0x00FF_FFFF)
                .collect::<Vec<u32>>()
        };
        let mut cases = vec![];
        for (rw, rh) in [
            (3, 2),
            (3, 14),
            (5, 5),
            (17, 9),
            (40, 24),
            (64, 64),
            (100, 50),
        ] {
            for radius in [0, 1, 2, 4, 7, 30] {
                for border in [0, 1, 2, 3] {
                    cases.push((rw, rh, radius, border));
                }
            }
        }
        for (rw, rh, radius, border) in cases {
            let mut out = vec![];
            for one in [true, false] {
                let mut p = GdiPainter::new(dc, Size { w, h }, 96);
                p.one_blend = one;
                p.fill_rect(Rect::new(0, 0, w, h), Color::rgb(30, 60, 90));
                let r = Rect::new(7, 3, rw, rh);
                if border == 0 {
                    p.fill_round_rect(r, radius, Color::rgb(250, 200, 10));
                } else {
                    p.stroke_round_rect(r, radius, border, Color::rgb(250, 200, 10));
                }
                // castocne orezane
                p.push_clip(Rect::new(0, 0, w, 20));
                p.fill_round_rect(
                    Rect::new(60, 10, rw.min(50), rh),
                    radius,
                    Color::rgb(9, 9, 9),
                );
                p.pop_clip();
                out.push(snap());
            }
            let diff: Vec<(usize, u32, u32)> = (0..out[0].len())
                .filter(|&i| out[0][i] != out[1][i])
                .map(|i| (i, out[0][i], out[1][i]))
                .take(6)
                .collect();
            assert!(
                diff.is_empty(),
                "{rw}x{rh} r={radius} b={border}: {diff:x?}"
            );
        }
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(dc);
        }
    }

    #[test]
    fn scale_follows_dpi() {
        let p = GdiPainter::new(HDC::default(), Size { w: 1, h: 1 }, 144);
        assert!((p.scale() - 1.5).abs() < f32::EPSILON);
    }
}
