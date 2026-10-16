//! Test helpers for the renderer: a fake atlas packed straight from the decoded assets (the
//! same texels the real atlas holds, without margins), a small CPU rasteriser for finalized
//! instance data, and a string table / glyph set with known CJK glyphs.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::assets::{GameAssets, Sprite};
use crate::atlas::{AtlasRect, AtlasSprite, Bank, Img};
use crate::font::{Flavor, GlyphSet, StyledGlyph};
use crate::i18n::{Lang, Strings};
use crate::palette;
use crate::render::{half, TexSource, FLOATS_PER_INSTANCE};

pub(crate) struct FakeAtlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    sprites: HashMap<(Bank, usize), AtlasSprite>,
    glyphs: Vec<Option<AtlasSprite>>,
    images: HashMap<Img, AtlasRect>,
}

impl TexSource for FakeAtlas {
    fn spr(&self, bank: Bank, idx: usize) -> Option<AtlasSprite> {
        self.sprites.get(&(bank, idx)).copied()
    }
    fn glyph_spr(&self, id: u32) -> Option<AtlasSprite> {
        self.glyphs.get(id as usize).copied().flatten()
    }
    fn img(&self, img: Img) -> AtlasRect {
        self.images.get(&img).copied().unwrap_or_default()
    }
}

const W: u32 = 1024;

struct Packer {
    x: u32,
    y: u32,
    row_h: u32,
    rgba: Vec<u8>,
}

impl Packer {
    /// Places a w x h image (texel function) and returns its top-left.
    fn place(&mut self, w: u32, h: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> (u16, u16) {
        if self.x + w > W {
            self.x = 0;
            self.y += self.row_h + 1;
            self.row_h = 0;
        }
        let (u, v) = (self.x, self.y);
        let need = ((v + h + 1) * W * 4) as usize;
        if self.rgba.len() < need {
            self.rgba.resize(need, 0);
        }
        for y in 0..h {
            for x in 0..w {
                let o = (((v + y) * W + u + x) * 4) as usize;
                self.rgba[o..o + 4].copy_from_slice(&texel(x, y));
            }
        }
        self.x += w + 1;
        self.row_h = self.row_h.max(h);
        (u as u16, v as u16)
    }
}

fn sprite_texel(s: &Sprite) -> impl Fn(u32, u32) -> [u8; 4] + '_ {
    move |x, y| s.pix[(y * s.w as u32 + x) as usize].map_or([0; 4], palette::rgba)
}

impl FakeAtlas {
    fn build(a: &GameAssets, glyphs: &GlyphSet) -> FakeAtlas {
        let mut p = Packer { x: 0, y: 0, row_h: 0, rgba: Vec::new() };
        let mut images = HashMap::new();
        let mut img = |p: &mut Packer, i: Img, w: u32, h: u32, f: &dyn Fn(u32, u32) -> [u8; 4]| {
            let (u, v) = p.place(w, h, f);
            images.insert(i, AtlasRect { u, v, w: w as u16, h: h as u16 });
        };
        img(&mut p, Img::GameScreen, 320, 256, &|x, y| palette::rgba(a.game_screen.pix[(y * 320 + x) as usize]));
        img(&mut p, Img::ChatScreen, 320, 256, &|x, y| palette::rgba(a.chat_screen.pix[(y * 320 + x) as usize]));
        for zone in 0..3u8 {
            for neuron in [false, true] {
                let z = &a.zones[zone as usize];
                let b = if neuron { &z.neuron_backdrop } else { &z.backdrop };
                img(&mut p, Img::Backdrop { zone, neuron }, 48, 32, &|x, y| palette::rgba(b.pix[(y * 48 + x) as usize]));
            }
        }
        img(&mut p, Img::StrengthRed, 108, 38, &|x, y| [(0x11 * (11 + (x * 7 + (y % 32) * 3) % 5)) as u8, 0, 0, 255]);
        img(&mut p, Img::StrengthGrey, 108, 38, &|x, y| {
            let g = (0x11 * ((x + y % 32) & 3)) as u8;
            [g, g, g, 255]
        });
        img(&mut p, Img::White, 4, 4, &|_, _| [255; 4]);

        let mut sprites = HashMap::new();
        let mut banks: Vec<(Bank, &crate::assets::SpriteBank)> = vec![(Bank::Chars, &a.chars), (Bank::Bloke, &a.bloke), (Bank::Explo, &a.explo)];
        for z in 0..3u8 {
            banks.push((Bank::Blocks(z), &a.zones[z as usize].blocks));
            banks.push((Bank::Aliens(z), &a.zones[z as usize].aliens));
        }
        for (bank, sb) in banks {
            for i in 0..sb.sprites.len() {
                if let Some(s) = sb.get(i) {
                    let (u, v) = p.place(s.w as u32, s.h as u32, sprite_texel(s));
                    sprites.insert((bank, i), AtlasSprite { u, v, w: s.w, h: s.h, xc: s.xc, yc: s.yc });
                }
            }
        }
        let mut gl = Vec::new();
        for g in &glyphs.glyphs {
            if g.w == 0 || g.h == 0 {
                gl.push(None);
                continue;
            }
            let (u, v) = p.place(g.w as u32, g.h as u32, |x, y| match g.pix[(y * g.w as u32 + x) as usize] {
                1 => palette::rgba(palette::FONT_GREEN),
                2 => palette::rgba(palette::FONT_GREEN_DARK),
                _ => [0; 4],
            });
            gl.push(Some(AtlasSprite { u, v, w: g.w, h: g.h, xc: g.xc, yc: g.yc }));
        }
        let height = p.y + p.row_h + 1;
        p.rgba.resize((height * W * 4) as usize, 0);
        FakeAtlas { width: W, height, rgba: p.rgba, sprites, glyphs: gl, images }
    }

    /// Rasterises finalized instances onto a black `w` x `h` image (see [`raster`]).
    pub fn raster(&self, inst: &[f32], w: usize, h: usize) -> Vec<[u8; 4]> {
        raster(&self.rgba, self.width, self.height, inst, w, h)
    }
}

/// Rasterises finalized instances sampling an RGBA8 atlas (nearest texel at pixel centres,
/// straight alpha blending, colour modulation) onto a black `w` x `h` image. The atlas
/// rectangle goes through the GPU's 16-bit float packing first, as in the game.
pub(crate) fn raster(rgba: &[u8], aw: u32, ah: u32, inst: &[f32], w: usize, h: usize) -> Vec<[u8; 4]> {
    let texel = |u: i64, v: i64| -> [u8; 4] {
        let u = u.clamp(0, aw as i64 - 1) as usize;
        let v = v.clamp(0, ah as i64 - 1) as usize;
        let o = (v * aw as usize + u) * 4;
        [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
    };
    let mut img = vec![[0, 0, 0, 255]; w * h];
    for i in inst.as_chunks::<FLOATS_PER_INSTANCE>().0 {
        let (qw, qx, qh, qy) = (i[0], i[3], i[5], i[7]);
        let col = [i[8], i[9], i[10], i[11]];
        let (u, v, uw, vh) = (half(i[12]), half(i[13]), half(i[14]), half(i[15]));
        let (px0, px1) = ((qx - 0.5).ceil().max(0.0) as usize, ((qx + qw - 0.5).ceil().max(0.0) as usize).min(w));
        let (py0, py1) = ((qy - 0.5).ceil().max(0.0) as usize, ((qy + qh - 0.5).ceil().max(0.0) as usize).min(h));
        for py in py0..py1 {
            let t = (py as f32 + 0.5 - qy) / qh;
            let tv = (v + t * vh).floor() as i64;
            for px in px0..px1 {
                let s = (px as f32 + 0.5 - qx) / qw;
                let c = texel((u + s * uw).floor() as i64, tv);
                let a = c[3] as f32 / 255.0 * col[3];
                let d = &mut img[py * w + px];
                for k in 0..3 {
                    let src = c[k] as f32 * col[k];
                    d[k] = (src * a + d[k] as f32 * (1.0 - a)).round() as u8;
                }
            }
        }
    }
    img
}

/// A styled-looking test glyph: outline ring around a core block.
fn test_glyph(ch: char, w: u16, advance: i16) -> StyledGlyph {
    let h = 14u16;
    let mut pix = vec![0u8; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            pix[(y * w + x) as usize] = if edge { 2 } else { 1 };
        }
    }
    StyledGlyph { ch, w, h, xc: (w / 2) as i16, yc: 7, advance, pix }
}

fn test_glyph_set() -> GlyphSet {
    let mut set = GlyphSet::default();
    let mut add = |f: Flavor, g: StyledGlyph| {
        set.index.insert((f, g.ch), set.glyphs.len() as u32);
        set.glyphs.push(g);
    };
    for c in ['가', '옵', '션'] {
        add(Flavor::Ko, test_glyph(c, 14, 15));
    }
    for c in ['中', '选', '项'] {
        add(Flavor::ZhHans, test_glyph(c, 14, 15));
    }
    add(Flavor::Ko, test_glyph(':', 4, 7));
    add(Flavor::ZhHans, test_glyph(':', 5, 7));
    add(Flavor::Ko, StyledGlyph { ch: '\u{3000}', w: 0, h: 0, xc: 0, yc: 0, advance: 15, pix: vec![] });
    set
}

fn test_strings() -> Strings {
    let mut s = Strings::default();
    // the language names spell ":" so that only their (per-flavour) glyph forms differ
    let rows: [(&str, [&str; 3]); 4] = [
        ("T_OPTIONS", ["Options", "옵션", "选项"]),
        ("T_TWO_LINES", ["AB\nC", "가가가\n가", "中中\n中"]),
        (crate::i18n::keys::LANG_KO, [":", ":", ":"]),
        (crate::i18n::keys::LANG_ZH, [":", ":", ":"]),
    ];
    for (key, texts) in rows {
        for lang in Lang::ALL {
            s.tables[lang as usize].insert(key.to_string(), texts[lang as usize].to_string());
        }
    }
    s
}

pub(crate) struct TestKit {
    pub assets: Arc<GameAssets>,
    pub atlas: FakeAtlas,
    pub strings: Strings,
    pub glyphs: GlyphSet,
}

impl TestKit {
    pub fn get() -> &'static TestKit {
        static KIT: OnceLock<TestKit> = OnceLock::new();
        KIT.get_or_init(|| {
            let assets = crate::assets::testdata::assets();
            let glyphs = test_glyph_set();
            let atlas = FakeAtlas::build(&assets, &glyphs);
            TestKit { assets, atlas, strings: test_strings(), glyphs }
        })
    }
}

/// Writes a binary PPM (for eyeballing compositions while developing).
#[allow(dead_code)]
pub(crate) fn write_ppm(path: &std::path::Path, img: &[[u8; 4]], w: usize, h: usize) {
    let mut data = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in img {
        data.extend_from_slice(&p[..3]);
    }
    std::fs::write(path, data).expect("write ppm");
}
