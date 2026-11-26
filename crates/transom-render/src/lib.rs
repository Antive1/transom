//! Renderer transom. Prenosna cast (`painter`, `theme`, `scene`) nezavisi od Win32 ani od spravy okien;
//! backend R0 (GDI) je v `gdi` a existuje len na Windows. Rozhranie a zmluva: [`painter::Painter`].
pub mod audit;
pub mod brand;
pub mod calendar;
pub mod design;
pub mod fonts;
pub mod icons;
mod memo;
pub mod navicons;
#[cfg(windows)]
pub mod offscreen;
pub mod painter;
pub mod record;
pub mod resulticons;
pub mod scene;
pub mod theme;

#[cfg(windows)]
pub mod gdi;

pub use painter::{Color, Font, HAlign, Painter, Rect, Size, TextStyle};
pub use scene::{render_bar, render_bar_with, Interaction, MarqueeDraw, RenderInfo, TextOverflow};
pub use theme::{BarText, Theme, ThemeMode};

#[cfg(windows)]
pub use gdi::GdiPainter;
