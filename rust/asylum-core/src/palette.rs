//! The Acorn VIDC default 256-colour palette used by mode 13 (vdu.c `init_palette`).
//!
//! Bit layout of a colour index: b7 = blue high, b6 = green high, b5 = green low,
//! b4 = red high, b3 = blue low, b2 = red low, b1..b0 = tint added to all channels.

/// RGB of palette entry `i`.
pub const fn rgb(i: u8) -> [u8; 3] {
    let i = i as u32;
    let tint = (i & 3) * 0x11;
    let r = (if i & 0x10 != 0 { 0x88 } else { 0 }) + (if i & 0x04 != 0 { 0x44 } else { 0 }) + tint;
    let g = (if i & 0x40 != 0 { 0x88 } else { 0 }) + (if i & 0x20 != 0 { 0x44 } else { 0 }) + tint;
    let b = (if i & 0x80 != 0 { 0x88 } else { 0 }) + (if i & 0x08 != 0 { 0x44 } else { 0 }) + tint;
    [r as u8, g as u8, b as u8]
}

/// Opaque RGBA8 of palette entry `i`.
pub const fn rgba(i: u8) -> [u8; 4] {
    let c = rgb(i);
    [c[0], c[1], c[2], 255]
}

/// Palette index of the bright green used by the letter glyphs (#33BB33).
pub const FONT_GREEN: u8 = 67;
/// Palette index of the dark green outline of the letter glyphs (#337733).
pub const FONT_GREEN_DARK: u8 = 35;
/// Palette index of the bright orange used by the digit glyphs (#FF7733).
pub const FONT_ORANGE: u8 = 55;
/// Palette index of the dark orange outline of the digit glyphs (#CC4400).
pub const FONT_ORANGE_DARK: u8 = 52;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_colours() {
        assert_eq!(rgb(FONT_GREEN), [0x33, 0xbb, 0x33]);
        assert_eq!(rgb(FONT_GREEN_DARK), [0x33, 0x77, 0x33]);
        assert_eq!(rgb(FONT_ORANGE), [0xff, 0x77, 0x33]);
        assert_eq!(rgb(FONT_ORANGE_DARK), [0xcc, 0x44, 0x00]);
        assert_eq!(rgb(0), [0, 0, 0]);
        assert_eq!(rgb(255), [0xff, 0xff, 0xff]);
    }
}
