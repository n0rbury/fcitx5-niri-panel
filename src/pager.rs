use cosmic_text::{
    Attrs, Buffer as TextBuffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache,
};

pub fn pager_hit(has_previous: bool, has_next: bool, w: u32, x: u32) -> Option<&'static str> {
    if !has_previous && !has_next {
        return None;
    }
    let start = w.saturating_sub(96);
    if x < start {
        return None;
    }
    let zone = (x - start) / 48;
    match (zone, has_previous, has_next) {
        (0, true, _) => Some("LookupTablePageUp"),
        (1, _, true) => Some("LookupTablePageDown"),
        _ => None,
    }
}
pub const PAGER_ACTIVE: TextColor = cosmic_text::Color(0xFF_B9_C8_DD);

/// Rasterize a single glyph into the BGRA pixel buffer at an explicit offset.
#[allow(clippy::too_many_arguments)]
pub fn draw_glyph(
    px: &mut [u8],
    w: u32,
    height: u32,
    font_system: &mut FontSystem,
    swash: &mut SwashCache,
    ch: char,
    x: i32,
    y: i32,
    color: TextColor,
) {
    let mut glyph_line = TextBuffer::new(font_system, Metrics::new(15.0, 22.0));
    glyph_line.set_size(font_system, Some(w as f32), Some(height as f32));
    let mut text = String::new();
    text.push(ch);
    glyph_line.set_text(font_system, &text, Attrs::new().color(color), Shaping::Advanced);
    glyph_line.draw(font_system, swash, color, |gx, gy, gw, gh, gc| {
        let raw = gc.0;
        let a = ((raw >> 24) & 0xff) as u8;
        let r = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let b = (raw & 0xff) as u8;
        for yy in (y + gy)..(y + gy + gh as i32) {
            if yy < 0 || yy as u32 >= height {
                continue;
            }
            for xx in (x + gx)..(x + gx + gw as i32) {
                if xx < 0 || xx as u32 >= w {
                    continue;
                }
                let idx = ((yy as u32) * w + xx as u32) as usize * 4;
                let dst_a = px[idx + 3];
                let ai = a as u16;
                let out_a = ai + (dst_a as u16 * (255 - ai)) / 255;
                for (c, off) in [(r, 2usize), (g, 1), (b, 0)] {
                    px[idx + off] =
                        ((c as u16 * ai + px[idx + off] as u16 * (255 - ai)) / 255).min(255) as u8;
                }
                px[idx + 3] = out_a.min(255) as u8;
            }
        }
    });
}
