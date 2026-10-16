//! "FSP1" sprite banks (vdu.c `initialize_sprites`).
//!
//! Header: u32 "FSP1", u32, u32, u32 count, then `count` u32 byte offsets (0 = empty slot).
//! Sprite at offset p: bytes w, h, xcen, ycen; then words; pixel words start at word
//! `p/4 + 2 + h` and run until the next non-zero sprite offset (or end of data). Each pixel
//! word: bits 8..27 = y*320+x, bits 0..7 = palette index. Unlisted pixels are transparent.
//! Offsets equal to the file length (trailing empty slots) mean "empty".

/// One decoded sprite. `pix[y*w+x]` = `Some(palette index)` or `None` (transparent).
#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub w: u16,
    pub h: u16,
    /// Hot spot ("centre"): the sprite is drawn with its top-left at (x - xc, y - yc).
    pub xc: i16,
    pub yc: i16,
    pub pix: Vec<Option<u8>>,
}

impl Sprite {
    /// True if the sprite has no area (drawing it is a no-op, like the GL path).
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

/// A bank with a fixed number of slots (chars 48, bloke 77, explo 32, blocks/aliens 256).
#[derive(Clone, Debug, Default)]
pub struct SpriteBank {
    pub sprites: Vec<Option<Sprite>>,
}

impl SpriteBank {
    pub fn get(&self, idx: usize) -> Option<&Sprite> {
        self.sprites.get(idx).and_then(|s| s.as_ref()).filter(|s| !s.is_empty())
    }
}

/// Parse an FSP1 bank, keeping at most `max_sprites` slots (bank is padded with `None`
/// up to `max_sprites`).
pub fn parse_fsp(data: &[u8], max_sprites: usize) -> Result<SpriteBank, String> {
    let word = |i: usize| -> u32 {
        let o = i * 4;
        if o + 4 <= data.len() {
            u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
        } else {
            0
        }
    };
    if data.len() < 16 || word(0) != 0x3150_5346 {
        return Err("not an FSP1 sprite file".into());
    }
    let end_words = data.len() / 4;
    let num = (word(3) as usize).min(max_sprites);
    let mut sprites: Vec<Option<Sprite>> = vec![None; max_sprites];
    #[allow(clippy::needless_range_loop)] // i also indexes the offset table, like the C
    for i in 0..num {
        let off = word(4 + i) as usize;
        if off == 0 || off + 4 > data.len() {
            continue;
        }
        let p = off >> 2;
        // end of this sprite: next non-zero offset (C initialize_sprites), else end of data
        let mut r = 0usize;
        if i == num - 1 {
            r = end_words;
        } else {
            let mut j = 0;
            while r == 0 && i + j < num - 1 {
                r = (word(5 + i + j) >> 2) as usize;
                j += 1;
            }
        }
        if r == 0 {
            r = end_words;
        }
        let (w, h, xc, yc) = (data[off] as usize, data[off + 1] as usize, data[off + 2], data[off + 3]);
        let mut pix = vec![None; w * h];
        let start = p + 2 + h;
        let mut q = start;
        while q < r.min(end_words) {
            let v = word(q);
            let pos = ((v & 0x0fff_ff00) >> 8) as usize;
            // C: index y*wid + x with a linear bounds check (x may spill into the next row)
            let (x, y) = (pos % 320, pos / 320);
            let idx = y * w + x;
            if idx < w * h {
                pix[idx] = Some((v & 0xff) as u8);
            }
            q += 1;
        }
        sprites[i] = Some(Sprite { w: w as u16, h: h as u16, xc: xc as i16, yc: yc as i16, pix });
    }
    Ok(SpriteBank { sprites })
}
