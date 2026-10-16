//! One RGBA8 texture atlas holding every image the game draws (uploaded to the GPU once).
//!
//! Contents: sprite banks (chars, bloke, explo, blocks×3 zones, aliens×3 zones), styled CJK
//! glyphs, GameScreen, ChatScreen, the 6 backdrop tiles, the two strength-bar noise textures
//! (vdu.c init_strengthcol: red = R 0xBB..0xFF, grey = 0x00..0x33, 108 x (32+6) with rows
//! 32..37 repeating rows 0..5) and a small opaque white block (for solid fills).
//!
//! Packing: shelf packer, width 1024 (height as needed, multiple of 4). Sprites/glyphs get a
//! 2-px fully transparent margin; opaque images (screens, backdrops, strength, white) get a
//! 2-px margin that replicates their edge pixels (needed by the sharp-pixel shader which
//! samples bilinearly within ±0.5 texel). Transparent texels are (0,0,0,0); since every
//! opaque texel has alpha 255 the data is valid premultiplied alpha.

use std::sync::Mutex;

use crate::assets::screen::{SCREEN_H, SCREEN_W};
use crate::assets::{GameAssets, Sprite, SpriteBank};
use crate::font::{GlyphSet, StyledGlyph};
use crate::palette;
use crate::rng::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bank {
    Chars,
    Bloke,
    Explo,
    Blocks(u8),
    Aliens(u8),
}

/// A sprite placed in the atlas (texel rectangle + hot spot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AtlasSprite {
    pub u: u16,
    pub v: u16,
    pub w: u16,
    pub h: u16,
    pub xc: i16,
    pub yc: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Img {
    GameScreen,
    ChatScreen,
    /// zone 0..3; neuron = Neurons/Backfile
    Backdrop { zone: u8, neuron: bool },
    StrengthRed,
    StrengthGrey,
    /// 4x4 opaque white texels (use with a colour for solid rectangles).
    White,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AtlasRect {
    pub u: u16,
    pub v: u16,
    pub w: u16,
    pub h: u16,
}

pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, `width*height*4` bytes.
    pub rgba: Vec<u8>,
    // implementation-defined lookup tables
    sprites: Vec<Vec<Option<AtlasSprite>>>,
    glyphs: Vec<AtlasSprite>,
    images: Vec<(Img, AtlasRect)>,
}

const ATLAS_W: u32 = 1024;
/// Texel coordinates reach the GPU as 16-bit floats (`render::finalize`), exact for whole
/// texels up to 2048 only. The shipped fonts need about 1000 rows.
const ATLAS_MAX_H: u32 = 2048;
/// Margin around every entry (transparent for sprites/glyphs, edge-replicated for images).
const MARGIN: u32 = 2;
const BANK_SLOTS: usize = 9;
const BACKDROP_W: usize = 48;
const BACKDROP_H: usize = 32;
/// vdu.c: strengthw = _strengthmax >> 8, height 32 noise rows + strengthh (6).
const STRENGTH_W: usize = 108;
const STRENGTH_H: usize = 32 + 6;
const WHITE_SIZE: usize = 4;

/// Pixel source of one atlas entry.
enum Src<'a> {
    Sprite(&'a Sprite),
    Glyph(&'a StyledGlyph),
    /// Opaque palette-indexed image, `w*h` indices.
    Indexed(&'a [u8]),
    /// Opaque RGBA8 image, `w*h*4` bytes.
    Rgba(&'a [u8]),
}

#[derive(Clone, Copy)]
enum Key {
    Sprite { slot: usize, idx: usize },
    Glyph(usize),
    Image(Img),
}

struct Item<'a> {
    w: u32,
    h: u32,
    xc: i16,
    yc: i16,
    src: Src<'a>,
    key: Key,
    /// Top-left of the padded rectangle (margin included), set by the packer.
    x: u32,
    y: u32,
}

/// A horizontal band of the atlas; its items all start at row `y`.
struct Shelf {
    y: u32,
    h: u32,
    items: Vec<usize>,
}

impl Atlas {
    /// Build the atlas. May use worker threads for rasterisation (std::thread::scope).
    pub fn build(assets: &GameAssets, glyphs: &GlyphSet) -> Atlas {
        let (red, grey) = strength_textures();
        let white = vec![255u8; WHITE_SIZE * WHITE_SIZE * 4];

        let mut items: Vec<Item> = Vec::new();
        let mut sprites: Vec<Vec<Option<AtlasSprite>>> = vec![Vec::new(); BANK_SLOTS];
        let mut banks: Vec<(Bank, &SpriteBank)> = vec![(Bank::Chars, &assets.chars), (Bank::Bloke, &assets.bloke), (Bank::Explo, &assets.explo)];
        for (z, zone) in assets.zones.iter().enumerate() {
            banks.push((Bank::Blocks(z as u8), &zone.blocks));
            banks.push((Bank::Aliens(z as u8), &zone.aliens));
        }
        for (bank, sb) in banks {
            let slot = Atlas::bank_slot(bank);
            sprites[slot] = vec![None; sb.sprites.len()];
            for idx in 0..sb.sprites.len() {
                if let Some(s) = sb.get(idx) {
                    debug_assert_eq!(s.pix.len(), s.w as usize * s.h as usize);
                    items.push(Item::new(s.w as u32, s.h as u32, s.xc, s.yc, Src::Sprite(s), Key::Sprite { slot, idx }));
                }
            }
        }

        let mut glyph_tab = vec![AtlasSprite::default(); glyphs.glyphs.len()];
        for (i, g) in glyphs.glyphs.iter().enumerate() {
            glyph_tab[i].xc = g.xc;
            glyph_tab[i].yc = g.yc;
            if g.w > 0 && g.h > 0 {
                assert_eq!(g.pix.len(), g.w as usize * g.h as usize, "styled glyph {:?} pixel count", g.ch);
                items.push(Item::new(g.w as u32, g.h as u32, g.xc, g.yc, Src::Glyph(g), Key::Glyph(i)));
            }
        }

        fn opaque(img: Img, w: usize, h: usize, src: Src<'_>) -> Item<'_> {
            let need = match src {
                Src::Rgba(_) => w * h * 4,
                _ => w * h,
            };
            if let Src::Indexed(p) | Src::Rgba(p) = src {
                assert!(p.len() >= need, "{img:?}: {} bytes, need {need}", p.len());
            }
            Item::new(w as u32, h as u32, 0, 0, src, Key::Image(img))
        }
        let mut imgs = vec![
            opaque(Img::GameScreen, SCREEN_W, SCREEN_H, Src::Indexed(&assets.game_screen.pix)),
            opaque(Img::ChatScreen, SCREEN_W, SCREEN_H, Src::Indexed(&assets.chat_screen.pix)),
        ];
        for (z, zone) in assets.zones.iter().enumerate() {
            let z = z as u8;
            imgs.push(opaque(Img::Backdrop { zone: z, neuron: false }, BACKDROP_W, BACKDROP_H, Src::Indexed(&zone.backdrop.pix)));
            imgs.push(opaque(Img::Backdrop { zone: z, neuron: true }, BACKDROP_W, BACKDROP_H, Src::Indexed(&zone.neuron_backdrop.pix)));
        }
        imgs.push(opaque(Img::StrengthRed, STRENGTH_W, STRENGTH_H, Src::Rgba(&red)));
        imgs.push(opaque(Img::StrengthGrey, STRENGTH_W, STRENGTH_H, Src::Rgba(&grey)));
        imgs.push(opaque(Img::White, WHITE_SIZE, WHITE_SIZE, Src::Rgba(&white)));
        items.extend(imgs);

        let (shelves, height) = pack(&mut items);
        let mut rgba = vec![0u8; ATLAS_W as usize * height as usize * 4];
        rasterise(&items, &shelves, &mut rgba);

        let mut images = Vec::new();
        for it in &items {
            let (u, v) = ((it.x + MARGIN) as u16, (it.y + MARGIN) as u16);
            let (w, h) = (it.w as u16, it.h as u16);
            match it.key {
                Key::Sprite { slot, idx } => sprites[slot][idx] = Some(AtlasSprite { u, v, w, h, xc: it.xc, yc: it.yc }),
                Key::Glyph(i) => glyph_tab[i] = AtlasSprite { u, v, w, h, xc: it.xc, yc: it.yc },
                Key::Image(img) => images.push((img, AtlasRect { u, v, w, h })),
            }
        }
        Atlas { width: ATLAS_W, height, rgba, sprites, glyphs: glyph_tab, images }
    }

    /// Internal: index into `sprites` for a bank.
    fn bank_slot(bank: Bank) -> usize {
        match bank {
            Bank::Chars => 0,
            Bank::Bloke => 1,
            Bank::Explo => 2,
            Bank::Blocks(z) => 3 + z as usize,
            Bank::Aliens(z) => 6 + z as usize,
        }
    }

    /// Sprite `idx` of `bank`, `None` for empty slots (drawing an empty sprite is a no-op).
    pub fn sprite(&self, bank: Bank, idx: usize) -> Option<&AtlasSprite> {
        self.sprites.get(Atlas::bank_slot(bank)).and_then(|b| b.get(idx)).and_then(|s| s.as_ref())
    }

    /// Styled CJK glyph `id` (index into `GlyphSet::glyphs`).
    pub fn glyph(&self, id: u32) -> Option<&AtlasSprite> {
        self.glyphs.get(id as usize)
    }

    /// Rectangle of a full image.
    pub fn image(&self, img: Img) -> AtlasRect {
        self.images.iter().find(|(i, _)| *i == img).map(|(_, r)| *r).unwrap_or_default()
    }
}

impl<'a> Item<'a> {
    fn new(w: u32, h: u32, xc: i16, yc: i16, src: Src<'a>, key: Key) -> Item<'a> {
        Item { w, h, xc, yc, src, key, x: 0, y: 0 }
    }
}

/// Shelf packing, tallest first (ties: wider first, then creation order — deterministic).
/// Returns the shelves (contiguous from row 0) and the atlas height.
fn pack(items: &mut [Item]) -> (Vec<Shelf>, u32) {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| items[b].h.cmp(&items[a].h).then(items[b].w.cmp(&items[a].w)).then(a.cmp(&b)));
    let mut shelves: Vec<Shelf> = Vec::new();
    let mut x = 0u32;
    for i in order {
        let (pw, ph) = (items[i].w + 2 * MARGIN, items[i].h + 2 * MARGIN);
        assert!(pw <= ATLAS_W, "atlas entry {}x{} wider than the atlas", items[i].w, items[i].h);
        if shelves.is_empty() || x + pw > ATLAS_W {
            let y = shelves.last().map_or(0, |s| s.y + s.h);
            shelves.push(Shelf { y, h: ph, items: Vec::new() });
            x = 0;
        }
        let shelf = shelves.last_mut().unwrap();
        debug_assert!(ph <= shelf.h);
        items[i].x = x;
        items[i].y = shelf.y;
        shelf.items.push(i);
        x += pw;
    }
    let used = shelves.last().map_or(0, |s| s.y + s.h);
    let height = used.div_ceil(4).max(1) * 4;
    assert!(height <= ATLAS_MAX_H, "atlas needs {height} rows (max {ATLAS_MAX_H})");
    (shelves, height)
}

/// Rasterise all items. Shelves are disjoint row bands of `rgba`, handed out to worker
/// threads through a shared queue.
fn rasterise(items: &[Item], shelves: &[Shelf], rgba: &mut [u8]) {
    let stride = ATLAS_W as usize * 4;
    let mut jobs: Vec<(&mut [u8], &[usize])> = Vec::with_capacity(shelves.len());
    let mut rest = rgba;
    for s in shelves {
        let (band, tail) = std::mem::take(&mut rest).split_at_mut(s.h as usize * stride);
        jobs.push((band, s.items.as_slice()));
        rest = tail;
    }
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8).min(jobs.len().max(1));
    let queue = Mutex::new(jobs.into_iter());
    std::thread::scope(|sc| {
        for _ in 0..workers {
            sc.spawn(|| loop {
                let job = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                let Some((band, ids)) = job else { break };
                for &i in ids {
                    draw_item(&items[i], band, stride);
                }
            });
        }
    });
}

/// Draw one item into its shelf band (band row 0 = the item's padded top row).
fn draw_item(it: &Item, band: &mut [u8], stride: usize) {
    let (w, h, m) = (it.w as usize, it.h as usize, MARGIN as usize);
    let x0 = it.x as usize;
    let mut put = |px: usize, py: usize, c: [u8; 4]| {
        let o = py * stride + (x0 + px) * 4;
        band[o..o + 4].copy_from_slice(&c);
    };
    match it.src {
        Src::Sprite(s) => {
            for y in 0..h {
                for x in 0..w {
                    if let Some(c) = s.pix[y * w + x] {
                        put(m + x, m + y, palette::rgba(c));
                    }
                }
            }
        }
        Src::Glyph(g) => {
            let core = palette::rgba(palette::FONT_GREEN);
            let outline = palette::rgba(palette::FONT_GREEN_DARK);
            for y in 0..h {
                for x in 0..w {
                    match g.pix[y * w + x] {
                        1 => put(m + x, m + y, core),
                        2 => put(m + x, m + y, outline),
                        _ => {}
                    }
                }
            }
        }
        Src::Indexed(_) | Src::Rgba(_) => {
            // Opaque: fill content + margin, the margin replicating the nearest edge texel.
            for py in 0..h + 2 * m {
                let sy = py.saturating_sub(m).min(h - 1);
                for px in 0..w + 2 * m {
                    let sx = px.saturating_sub(m).min(w - 1);
                    let i = sy * w + sx;
                    let c = match it.src {
                        Src::Indexed(p) => palette::rgba(p[i]),
                        Src::Rgba(p) => [p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]],
                        _ => unreachable!(),
                    };
                    put(px, py, c);
                }
            }
        }
    }
}

/// vdu.c `init_strengthcol`: per texel of the 32 noise rows one `random()` for the red shade
/// `0x11*(11+random()%5)` then one for the grey `0x111111*(3&random())`; rows 32..37 repeat
/// rows 0..5. Uses its own fixed-seed generator so building the atlas never touches the game
/// RNG (the C code drew from the global `random()` stream at start-up).
fn strength_textures() -> (Vec<u8>, Vec<u8>) {
    let mut rng = Rng::new(0x5EED);
    let row = STRENGTH_W * 4;
    let mut red = vec![0u8; STRENGTH_H * row];
    let mut grey = vec![0u8; STRENGTH_H * row];
    for j in 0..32 {
        for i in 0..STRENGTH_W {
            let o = j * row + i * 4;
            let r = (0x11 * (11 + rng.random() % 5)) as u8;
            red[o..o + 4].copy_from_slice(&[r, 0, 0, 255]);
            let g = (0x11 * (3 & rng.random())) as u8;
            grey[o..o + 4].copy_from_slice(&[g, g, g, 255]);
        }
    }
    for tex in [&mut red, &mut grey] {
        tex.copy_within(0..(STRENGTH_H - 32) * row, 32 * row);
    }
    (red, grey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::testdata;
    use crate::font::{style_glyph, BdfGlyph};
    use std::sync::OnceLock;

    const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

    fn test_glyphs() -> GlyphSet {
        let ring = BdfGlyph { dwidth: 12, bbx: (3, 3, 1, 0), rows: vec![0xE000_0000, 0xA000_0000, 0xE000_0000] };
        let space = BdfGlyph { dwidth: 12, bbx: (0, 0, 0, 0), rows: vec![] };
        let mut set = GlyphSet::default();
        set.glyphs.push(style_glyph('ㅁ', &ring, 13));
        set.glyphs.push(style_glyph('\u{3000}', &space, 13));
        set.index.insert((crate::font::Flavor::Ko, 'ㅁ'), 0);
        set.index.insert((crate::font::Flavor::Ko, '\u{3000}'), 1);
        set
    }

    fn atlas() -> &'static Atlas {
        static CELL: OnceLock<Atlas> = OnceLock::new();
        CELL.get_or_init(|| Atlas::build(&testdata::assets(), &test_glyphs()))
    }

    fn texel(a: &Atlas, x: i32, y: i32) -> [u8; 4] {
        assert!(x >= 0 && y >= 0 && (x as u32) < a.width && (y as u32) < a.height, "texel ({x},{y}) outside");
        let o = (y as usize * a.width as usize + x as usize) * 4;
        [a.rgba[o], a.rgba[o + 1], a.rgba[o + 2], a.rgba[o + 3]]
    }

    fn all_banks(assets: &GameAssets) -> Vec<(Bank, &SpriteBank)> {
        let mut v = vec![(Bank::Chars, &assets.chars), (Bank::Bloke, &assets.bloke), (Bank::Explo, &assets.explo)];
        for z in 0..3u8 {
            v.push((Bank::Blocks(z), &assets.zones[z as usize].blocks));
            v.push((Bank::Aliens(z), &assets.zones[z as usize].aliens));
        }
        v
    }

    #[test]
    fn dimensions() {
        let a = atlas();
        assert_eq!(a.width, 1024);
        assert!(a.height.is_multiple_of(4) && a.height > 0 && a.height <= ATLAS_MAX_H, "height {}", a.height);
        assert_eq!(a.rgba.len(), (a.width * a.height * 4) as usize);
    }

    #[test]
    fn every_sprite_present_with_palette_texels() {
        let a = atlas();
        let expected_len = [48, 77, 32, 256, 256, 256, 256, 256, 256];
        let mut count = 0;
        let assets = testdata::assets();
        for (bank, sb) in all_banks(&assets) {
            assert_eq!(sb.sprites.len(), expected_len[Atlas::bank_slot(bank)]);
            for idx in 0..sb.sprites.len() + 2 {
                let (Some(s), Some(t)) = (sb.get(idx), a.sprite(bank, idx)) else {
                    assert!(sb.get(idx).is_none() && a.sprite(bank, idx).is_none(), "{bank:?} {idx}");
                    continue;
                };
                count += 1;
                assert_eq!((t.w, t.h, t.xc, t.yc), (s.w, s.h, s.xc, s.yc), "{bank:?} {idx}");
                let (u, v, w, h) = (t.u as i32, t.v as i32, t.w as i32, t.h as i32);
                for y in -2..h + 2 {
                    for x in -2..w + 2 {
                        let got = texel(a, u + x, v + y);
                        let want = if x < 0 || y < 0 || x >= w || y >= h {
                            TRANSPARENT
                        } else {
                            s.pix[(y * w + x) as usize].map_or(TRANSPARENT, palette::rgba)
                        };
                        assert_eq!(got, want, "{bank:?} {idx} at ({x},{y})");
                    }
                }
            }
        }
        assert!(count > 500, "{count} sprites");
        assert!(a.sprite(Bank::Bloke, 76).is_none());
    }

    #[test]
    fn placed_rects_do_not_overlap() {
        let a = atlas();
        let mut rects: Vec<(u32, u32, u32, u32)> = Vec::new();
        let mut add = |u: u16, v: u16, w: u16, h: u16| {
            let (u, v) = (u as u32 - MARGIN, v as u32 - MARGIN);
            rects.push((u, v, u + w as u32 + 2 * MARGIN, v + h as u32 + 2 * MARGIN));
        };
        let assets = testdata::assets();
        for (bank, sb) in all_banks(&assets) {
            for idx in 0..sb.sprites.len() {
                if let Some(t) = a.sprite(bank, idx) {
                    add(t.u, t.v, t.w, t.h);
                }
            }
        }
        for id in 0..2 {
            let g = a.glyph(id).unwrap();
            if g.w > 0 {
                add(g.u, g.v, g.w, g.h);
            }
        }
        for (_, r) in &a.images {
            add(r.u, r.v, r.w, r.h);
        }
        assert_eq!(a.images.len(), 11);
        for &(_, _, x1, y1) in &rects {
            assert!(x1 <= a.width && y1 <= a.height);
        }
        for (i, r) in rects.iter().enumerate() {
            for s in &rects[i + 1..] {
                let disjoint = r.2 <= s.0 || s.2 <= r.0 || r.3 <= s.1 || s.3 <= r.1;
                assert!(disjoint, "{r:?} overlaps {s:?}");
            }
        }
    }

    #[test]
    fn images_match_sources_with_edge_margins() {
        let a = atlas();
        let assets = testdata::assets();
        let check = |img: Img, w: usize, h: usize, pix: &[u8]| {
            let r = a.image(img);
            assert_eq!((r.w as usize, r.h as usize), (w, h), "{img:?}");
            let (u, v) = (r.u as i32, r.v as i32);
            for y in -2..h as i32 + 2 {
                for x in -2..w as i32 + 2 {
                    let (sx, sy) = (x.clamp(0, w as i32 - 1) as usize, y.clamp(0, h as i32 - 1) as usize);
                    assert_eq!(texel(a, u + x, v + y), palette::rgba(pix[sy * w + sx]), "{img:?} at ({x},{y})");
                }
            }
        };
        check(Img::GameScreen, 320, 256, &assets.game_screen.pix);
        check(Img::ChatScreen, 320, 256, &assets.chat_screen.pix);
        for z in 0..3u8 {
            let zone = &assets.zones[z as usize];
            check(Img::Backdrop { zone: z, neuron: false }, 48, 32, &zone.backdrop.pix);
            check(Img::Backdrop { zone: z, neuron: true }, 48, 32, &zone.neuron_backdrop.pix);
        }
        let wr = a.image(Img::White);
        assert_eq!((wr.w, wr.h), (4, 4));
        for y in -2..6 {
            for x in -2..6 {
                assert_eq!(texel(a, wr.u as i32 + x, wr.v as i32 + y), [255; 4]);
            }
        }
    }

    #[test]
    fn strength_textures_follow_init_strengthcol() {
        let a = atlas();
        let red = a.image(Img::StrengthRed);
        let grey = a.image(Img::StrengthGrey);
        assert_eq!((red.w, red.h, grey.w, grey.h), (108, 38, 108, 38));
        let at = |r: AtlasRect, x: i32, y: i32| texel(a, r.u as i32 + x, r.v as i32 + y);
        for y in 0..38 {
            for x in 0..108 {
                let (cr, cg) = (at(red, x, y), at(grey, x, y));
                assert!(matches!(cr, [0xbb | 0xcc | 0xdd | 0xee | 0xff, 0, 0, 255]), "{cr:?}");
                assert!(matches!(cg[0], 0 | 0x11 | 0x22 | 0x33) && cg[0] == cg[1] && cg[1] == cg[2] && cg[3] == 255);
                if y >= 32 {
                    assert_eq!(cr, at(red, x, y - 32));
                    assert_eq!(cg, at(grey, x, y - 32));
                }
            }
        }
        // Interleaved draws, red first (one random() each per texel).
        let mut rng = Rng::new(0x5EED);
        for x in 0..3 {
            let r = (0x11 * (11 + rng.random() % 5)) as u8;
            let g = (0x11 * (3 & rng.random())) as u8;
            assert_eq!(at(red, x, 0), [r, 0, 0, 255]);
            assert_eq!(at(grey, x, 0), [g, g, g, 255]);
        }
        // Edge-replicated margin
        assert_eq!(at(red, -2, -2), at(red, 0, 0));
        assert_eq!(at(grey, 109, 39), at(grey, 107, 37));
    }

    #[test]
    fn glyphs_use_font_colours() {
        let a = atlas();
        let set = test_glyphs();
        let g = &set.glyphs[0];
        let t = a.glyph(0).unwrap();
        assert_eq!((t.w, t.h, t.xc, t.yc), (g.w, g.h, g.xc, g.yc));
        assert_eq!(t.h, 14);
        let (mut core, mut outline) = (0, 0);
        for y in -2..g.h as i32 + 2 {
            for x in -2..g.w as i32 + 2 {
                let got = texel(a, t.u as i32 + x, t.v as i32 + y);
                let inside = x >= 0 && y >= 0 && x < g.w as i32 && y < g.h as i32;
                let want = match if inside { g.pix[(y * g.w as i32 + x) as usize] } else { 0 } {
                    1 => {
                        core += 1;
                        palette::rgba(palette::FONT_GREEN)
                    }
                    2 => {
                        outline += 1;
                        palette::rgba(palette::FONT_GREEN_DARK)
                    }
                    _ => TRANSPARENT,
                };
                assert_eq!(got, want, "({x},{y})");
            }
        }
        assert!(core > 0 && outline > 0);
        let empty = a.glyph(1).unwrap();
        assert_eq!((empty.w, empty.h), (0, 0));
        assert!(a.glyph(2).is_none());
    }

    #[test]
    fn many_cjk_glyphs_fit() {
        // Several times the glyphs the UI strings need (about 700) still fit in 2048 rows.
        let full = BdfGlyph { dwidth: 12, bbx: (11, 11, 0, -1), rows: vec![0xFFE0_0000; 11] };
        let mut set = GlyphSet::default();
        for i in 0..3000u32 {
            set.glyphs.push(style_glyph(char::from_u32(0x4E00 + i).unwrap(), &full, 13));
        }
        let b = Atlas::build(&testdata::assets(), &set);
        assert!(b.height <= ATLAS_MAX_H && b.height > atlas().height, "{} vs {}", b.height, atlas().height);
        let last = b.glyph(2999).unwrap();
        assert_eq!((last.w, last.h), (14, 14));
        exact_in_half_precision(&b);
    }

    /// Every rectangle edge survives the GPU's 16-bit float packing unchanged.
    fn exact_in_half_precision(a: &Atlas) {
        let mut rects: Vec<(u16, u16, u16, u16)> = Vec::new();
        rects.extend(a.sprites.iter().flatten().flatten().map(|s| (s.u, s.v, s.w, s.h)));
        rects.extend(a.glyphs.iter().map(|s| (s.u, s.v, s.w, s.h)));
        rects.extend(a.images.iter().map(|(_, r)| (r.u, r.v, r.w, r.h)));
        assert!(rects.len() > 100, "{} rects", rects.len());
        for (u, v, w, h) in rects {
            for x in [u, v, w, h, u + w, v + h] {
                assert_eq!(crate::render::half(x as f32), x as f32, "rect ({u},{v},{w},{h})");
            }
        }
    }

    #[test]
    fn coordinates_are_exact_in_half_precision() {
        exact_in_half_precision(atlas());
        // so are the frame's fractional samples (gap fills inside texel centres, mirrored HUD)
        for (w, h) in [(320.0, 256.0), (1820.0, 768.0), (2731.0, 1536.0)] {
            let lay = crate::layout::Layout::new(w, h);
            let mut quads = Vec::new();
            crate::render::frame::game_screen_quads(&lay, atlas(), &mut quads);
            crate::render::frame::chat_screen_quads(&lay, atlas(), &mut quads);
            for q in quads {
                for x in [q.u, q.v, q.uw, q.vh, q.u + q.uw, q.v + q.vh] {
                    assert_eq!(crate::render::half(x), x, "{w}x{h}: {q:?}");
                }
            }
        }
        // the next whole texels past the limit would not be
        assert_ne!(crate::render::half((ATLAS_MAX_H + 1) as f32), (ATLAS_MAX_H + 1) as f32);
    }

    #[test]
    fn build_is_deterministic() {
        let b = Atlas::build(&testdata::assets(), &test_glyphs());
        let a = atlas();
        assert_eq!((b.width, b.height), (a.width, a.height));
        assert!(b.rgba == a.rgba);
        assert_eq!(b.sprites, a.sprites);
        assert_eq!(b.glyphs, a.glyphs);
        assert_eq!(b.images, a.images);
    }
}
