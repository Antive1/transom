//! Vygeneruje zdroje identity do `assets/brand/` (SVG, ICO) a nahladove PNG-podobne BMP na kontrolu.
//!   cargo run -p transom-render --example brand_export -- <adresar assets/brand> [adresar nahladov]
use transom_render::brand::*;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/brand".into());
    std::fs::create_dir_all(&out).unwrap();
    let w = |name: &str, data: &[u8]| {
        std::fs::write(format!("{out}/{name}"), data).unwrap();
        println!("{out}/{name} ({} B)", data.len());
    };
    w(
        "transom-mark.svg",
        svg_mark(&Palette::APP, true, "transom").as_bytes(),
    );
    w(
        "transom-mark-glyph-dark.svg",
        svg_mark(&Palette::ON_DARK, false, "transom").as_bytes(),
    );
    w(
        "transom-mark-glyph-light.svg",
        svg_mark(&Palette::ON_LIGHT, false, "transom").as_bytes(),
    );
    w(
        "transom-wordmark-dark.svg",
        svg_wordmark(&Palette::ON_DARK).as_bytes(),
    );
    w(
        "transom-wordmark-light.svg",
        svg_wordmark(&Palette::ON_LIGHT).as_bytes(),
    );
    w("transom-lockup.svg", svg_lockup(&Palette::APP).as_bytes());
    w("transom.ico", &app_ico());
    w("transom-tray-dark.ico", &tray_ico(&Palette::ON_DARK));
    w("transom-tray-light.ico", &tray_ico(&Palette::ON_LIGHT));

    if let Some(prev) = std::env::args().nth(2) {
        std::fs::create_dir_all(&prev).unwrap();
        // kontaktny harok: ikona aplikacie, tray tmavy/svetly pri 16/20/24/32/48/256, wordmark
        let bmp = |name: &str, w: u32, h: u32, px: &[Rgba], bg: Rgba| {
            let mut f = Vec::new();
            f.extend_from_slice(b"BM");
            f.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
            f.extend_from_slice(&[0; 4]);
            f.extend_from_slice(&54u32.to_le_bytes());
            f.extend_from_slice(&40u32.to_le_bytes());
            f.extend_from_slice(&(w as i32).to_le_bytes());
            f.extend_from_slice(&(-(h as i32)).to_le_bytes());
            f.extend_from_slice(&1u16.to_le_bytes());
            f.extend_from_slice(&32u16.to_le_bytes());
            f.extend_from_slice(&[0; 24]);
            for c in px {
                let a = c.a as f32 / 255.0;
                let m = |t: u8, u: u8| (t as f32 * a + u as f32 * (1.0 - a)).round() as u8;
                f.extend_from_slice(&[m(c.b, bg.b), m(c.g, bg.g), m(c.r, bg.r), 255]);
            }
            std::fs::write(format!("{prev}/{name}"), f).unwrap();
        };
        let dark = Rgba::hex(0x202020);
        let light = Rgba::hex(0xF3F3F3);
        for s in [16u32, 20, 24, 32, 48, 256] {
            bmp(
                &format!("app-{s}-dark.bmp"),
                s,
                s,
                &render_app_icon(s),
                dark,
            );
            bmp(
                &format!("app-{s}-light.bmp"),
                s,
                s,
                &render_app_icon(s),
                light,
            );
            bmp(
                &format!("tray-{s}-dark.bmp"),
                s,
                s,
                &render_mark(s, &Palette::ON_DARK),
                dark,
            );
            bmp(
                &format!("tray-{s}-light.bmp"),
                s,
                s,
                &render_mark(s, &Palette::ON_LIGHT),
                light,
            );
        }
        for (name, pal, bg) in [
            ("dark", &Palette::ON_DARK, colors::INK),
            ("light", &Palette::ON_LIGHT, colors::MIST),
        ] {
            let (img, pw, ph) = render_wordmark(96, pal);
            bmp(&format!("wordmark-{name}.bmp"), pw, ph, &img, bg);
        }
    }
}
