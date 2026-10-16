//! Text glyphs.
//!
//! 1. The ORIGINAL bitmap font (Resources/FSPChars, 48 glyphs) covers digits, A–Z (lower case
//!    maps to upper case like vdu.c `message()`), `. , ! ' ? - %`, `•` (grey ball, the C
//!    source's char 164), `←` (high-score marker, C '='), `✓`/`✗` (C chars 16/17), the
//!    ASYLUM logo (U+E000, C '#') and space.
//! 2. Every other character (Hangul, Hanzi, CJK punctuation, `:` `/` …) comes from the
//!    Fusion Pixel 12px BDF subsets shipped in `game/fonts/` (OFL) and is *re-styled exactly
//!    like the original glyphs*: the 1-px pixel-font strokes are emboldened horizontally
//!    (a pixel is added to the right when that does not close a 1-px gap) and receive a
//!    1-px 4-neighbour outline. Core = palette FONT_GREEN (#33BB33), outline = FONT_GREEN_DARK
//!    (#337733), glyph height 14 px like the original (12-px em + outline).

use std::collections::{BTreeSet, HashMap};

/// Glyph index (0..48) in the original FSPChars bank for `c`, if the original font has it.
pub fn original_glyph(c: char) -> Option<u8> {
    Some(match c {
        '0'..='9' => c as u8 - b'0',
        '•' => 10,
        '-' | '–' => 11,
        '?' => 12,
        '←' => 13,
        '%' => 14,
        '✓' => 15,
        '✗' => 16,
        'A'..='Z' => 17 + (c as u8 - b'A'),
        'a'..='z' => 17 + (c as u8 - b'a'),
        '.' => 43,
        ',' => 44,
        '!' => 45,
        '\'' | '’' => 46,
        '\u{E000}' => 47,
        _ => return None,
    })
}

/// Pen advance after drawing original glyph `g` (vdu.c texthandler: digits 16, punctuation and
/// the logo 8, everything else 14).
pub fn original_advance(g: u8) -> i32 {
    if g <= 9 {
        16
    } else if g >= 43 {
        8
    } else {
        14
    }
}

/// Advance of a space in every language (C: 0xff > 43 → 14 - 6).
pub const SPACE_ADVANCE: i32 = 8;

/// Maps a byte of the original Archimedes text encoding (as stored e.g. in high score
/// initials: '0'..'Z'+4) to the Unicode character our font tables use, reproducing the
/// character translation in vdu.c `message()`.
pub fn arc_char(b: u8) -> char {
    match b {
        b'-' => '-',
        164 | b':' => '•',
        b'\'' | b'^' => '\'',
        b'.' | b'[' => '.',
        b',' | b'\\' => ',',
        b'?' | b'<' => '?',
        b'!' | b']' => '!',
        b'%' | b'>' => '%',
        200 | 16 => '✓',
        215 | b'@' | 17 => '✗',
        b'#' | b'_' | 127 => '\u{E000}',
        b'=' => '←',
        b';' => '-',
        b' ' | 0xff => ' ',
        b'a'..=b'z' => (b - 32) as char,
        b'0'..=b'9' | b'A'..=b'Z' => b as char,
        _ => '?',
    }
}

/// Glyph-form flavour of the CJK font (Korean or Simplified Chinese shapes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Flavor {
    Ko = 0,
    ZhHans = 1,
}

/// One glyph of a BDF font.
#[derive(Clone, Debug)]
pub struct BdfGlyph {
    pub dwidth: i32,
    /// BBX w h xoff yoff
    pub bbx: (i32, i32, i32, i32),
    /// One entry per bitmap row, MSB = leftmost pixel, already left-aligned to 32 bits.
    pub rows: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct BdfFont {
    pub ascent: i32,
    pub descent: i32,
    pub glyphs: HashMap<u32, BdfGlyph>,
}

/// Minimal BDF 2.1 parser (STARTFONT/FONT_ASCENT/FONT_DESCENT/STARTCHAR/ENCODING/DWIDTH/BBX/BITMAP).
pub fn parse_bdf(text: &str) -> Result<BdfFont, String> {
    fn ints<const N: usize>(args: &[&str], ln: usize, kw: &str) -> Result<[i32; N], String> {
        let mut out = [0i32; N];
        if args.len() < N {
            return Err(format!("BDF line {ln}: {kw} needs {N} values"));
        }
        for (o, a) in out.iter_mut().zip(args) {
            *o = a.parse().map_err(|_| format!("BDF line {ln}: bad {kw} value '{a}'"))?;
        }
        Ok(out)
    }
    /// One hex bitmap row, left-aligned into 32 bits (columns beyond 32 are dropped).
    fn hex_row(s: &str, ln: usize) -> Result<u32, String> {
        if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("BDF line {ln}: bad bitmap row '{s}'"));
        }
        let digits = &s[..s.len().min(8)];
        let v = u32::from_str_radix(digits, 16).unwrap_or(0);
        Ok(v << (32 - 4 * digits.len() as u32))
    }

    let mut font = BdfFont::default();
    let mut started = false;
    // Current glyph: (encoding, glyph, in BITMAP section)
    let mut cur: Option<(Option<u32>, BdfGlyph, bool)> = None;
    for (i, line) in text.lines().enumerate() {
        let ln = i + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_ascii_whitespace();
        let kw = parts.next().unwrap_or("");
        if !started {
            if kw != "STARTFONT" {
                return Err("not a BDF font (missing STARTFONT)".into());
            }
            started = true;
            continue;
        }
        if let Some((enc, g, in_bitmap)) = cur.as_mut() {
            if kw == "ENDCHAR" {
                let (enc, mut g, _) = cur.take().unwrap();
                let (w, h) = (g.bbx.0.clamp(0, 32), g.bbx.1.max(0));
                // Clear padding bits beyond the BBX width and force exactly h rows.
                let mask = if w == 0 { 0 } else { u32::MAX << (32 - w as u32) };
                g.rows.resize(h as usize, 0);
                for r in &mut g.rows {
                    *r &= mask;
                }
                if let Some(e) = enc {
                    font.glyphs.insert(e, g);
                }
                continue;
            }
            if *in_bitmap {
                g.rows.push(hex_row(kw, ln)?);
                continue;
            }
            let args: Vec<&str> = parts.collect();
            match kw {
                // "ENCODING -1 [n]" (non-standard encoding) is left unencoded and dropped.
                "ENCODING" => *enc = u32::try_from(ints::<1>(&args, ln, kw)?[0]).ok(),
                "DWIDTH" => g.dwidth = ints::<1>(&args, ln, kw)?[0],
                "BBX" => {
                    let [w, h, x, y] = ints::<4>(&args, ln, kw)?;
                    g.bbx = (w, h, x, y);
                }
                "BITMAP" => *in_bitmap = true,
                _ => {}
            }
            continue;
        }
        let args: Vec<&str> = parts.collect();
        match kw {
            "FONT_ASCENT" => font.ascent = ints::<1>(&args, ln, kw)?[0],
            "FONT_DESCENT" => font.descent = ints::<1>(&args, ln, kw)?[0],
            "STARTCHAR" => cur = Some((None, BdfGlyph { dwidth: 0, bbx: (0, 0, 0, 0), rows: Vec::new() }, false)),
            _ => {}
        }
    }
    if !started {
        return Err("not a BDF font (empty)".into());
    }
    if cur.is_some() {
        return Err("BDF: unterminated glyph at end of file".into());
    }
    Ok(font)
}

/// A styled glyph ready for the atlas. `pix`: 0 transparent, 1 core (FONT_GREEN),
/// 2 outline (FONT_GREEN_DARK). Drawn like original glyphs: centre (xc, yc) at the pen.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledGlyph {
    pub ch: char,
    pub w: u16,
    pub h: u16,
    pub xc: i16,
    pub yc: i16,
    /// Pen advance (px) after this glyph.
    pub advance: i16,
    pub pix: Vec<u8>,
}

/// All CJK/extra glyphs needed by the current string table, per flavour.
#[derive(Clone, Debug, Default)]
pub struct GlyphSet {
    pub glyphs: Vec<StyledGlyph>,
    pub index: HashMap<(Flavor, char), u32>,
}

impl GlyphSet {
    /// Glyph for `c` in `flavor`, falling back to the other flavour.
    pub fn find(&self, flavor: Flavor, c: char) -> Option<u32> {
        self.index.get(&(flavor, c)).copied().or_else(|| {
            let other = if flavor == Flavor::Ko { Flavor::ZhHans } else { Flavor::Ko };
            self.index.get(&(other, c)).copied()
        })
    }
}

/// Builds styled glyphs for every character in `chars[flavor]` that the original font does
/// not cover. Characters missing from both fonts are skipped (the text layout then uses '?').
///
/// Order: flavours Ko then ZhHans, characters in `BTreeSet` order. A glyph whose styled
/// result is identical to one already built for the same character (e.g. Hangul taken from
/// either font, or a fallback to the same font) shares that glyph id.
pub fn build_glyph_set(chars: &HashMap<Flavor, BTreeSet<char>>, ko: &BdfFont, zh: &BdfFont) -> GlyphSet {
    let mut set = GlyphSet::default();
    let mut by_char: HashMap<char, Vec<u32>> = HashMap::new();
    for flavor in [Flavor::Ko, Flavor::ZhHans] {
        let Some(list) = chars.get(&flavor) else { continue };
        let (own, other) = if flavor == Flavor::Ko { (ko, zh) } else { (zh, ko) };
        for &ch in list {
            if ch == ' ' || original_glyph(ch).is_some() {
                continue;
            }
            let (font, g) = match own.glyphs.get(&(ch as u32)) {
                Some(g) => (own, g),
                None => match other.glyphs.get(&(ch as u32)) {
                    Some(g) => (other, g),
                    None => continue,
                },
            };
            let styled = style_glyph(ch, g, font.ascent);
            let ids = by_char.entry(ch).or_default();
            let id = match ids.iter().copied().find(|&id| set.glyphs[id as usize] == styled) {
                Some(id) => id,
                None => {
                    let id = set.glyphs.len() as u32;
                    set.glyphs.push(styled);
                    ids.push(id);
                    id
                }
            };
            set.index.insert((flavor, ch), id);
        }
    }
    set
}

/// Pen-relative x of the BDF origin (outline of an xoff=0 glyph starts at pen-7, like the
/// original 14-px digit boxes).
const ORIGIN_X: i32 = -6;
/// Line-centre-relative row of the first row below the baseline (the 12-px em spans rows
/// [-6, 6) around the line centre).
const BASELINE_Y: i32 = 4;
/// Minimum vertical extent of a styled glyph: rows [-7, 7) around the line centre, the
/// 14-px box of the original glyphs (yc = 7).
const CELL_TOP: i32 = -7;
const CELL_BOTTOM: i32 = 7;

/// Style one BDF glyph (embolden + outline) — exposed for tests and tools.
///
/// Placement is baseline-relative (row r of the BBX lands at line row
/// `BASELINE_Y - (yoff + h) + r`), so the font ascent is not needed.
pub fn style_glyph(ch: char, g: &BdfGlyph, _ascent: i32) -> StyledGlyph {
    let (bw, bh, xoff, yoff) = g.bbx;
    let bw = bw.clamp(0, 32);
    let bh = bh.max(0);
    let advance = if g.dwidth == 0 { 0 } else { (g.dwidth + 3) as i16 };
    // Work grid: 1 column of outline on the left, emboldening + outline on the right,
    // 1 row of outline above and below. Core pixel (c, r) sits at grid (c + 1, r + 1).
    let gw = (bw + 3) as usize;
    let gh = (bh + 2) as usize;
    let mut core = vec![false; gw * gh];
    for r in 0..bh as usize {
        let bits = g.rows.get(r).copied().unwrap_or(0);
        for c in 0..bw as usize {
            if bits & (0x8000_0000 >> c) != 0 {
                core[(r + 1) * gw + c + 1] = true;
            }
        }
    }
    // Embolden: (x+1, y) unless (x+2, y) is core, which keeps 1-px gaps open.
    let mut bold = core.clone();
    for y in 1..gh - 1 {
        for x in 1..gw - 2 {
            if core[y * gw + x] && !core[y * gw + x + 2] {
                bold[y * gw + x + 1] = true;
            }
        }
    }
    // Classify: 1 = core, 2 = outline (non-core 4-adjacent to core).
    let mut cls = vec![0u8; gw * gh];
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..gh {
        for x in 0..gw {
            let i = y * gw + x;
            let c = if bold[i] {
                1
            } else if (x > 0 && bold[i - 1])
                || (x + 1 < gw && bold[i + 1])
                || (y > 0 && bold[i - gw])
                || (y + 1 < gh && bold[i + gw])
            {
                2
            } else {
                0
            };
            if c != 0 {
                cls[i] = c;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        return StyledGlyph { ch, w: 0, h: 0, xc: 0, yc: 0, advance, pix: Vec::new() };
    }
    // Grid -> pen-relative coordinates.
    let dx = ORIGIN_X + xoff - 1;
    let dy = BASELINE_Y - (yoff + bh) - 1;
    let (px0, px1) = (x0 as i32 + dx, x1 as i32 + dx);
    let py0 = (y0 as i32 + dy).min(CELL_TOP);
    let py1 = (y1 as i32 + dy).max(CELL_BOTTOM);
    let (w, h) = ((px1 - px0) as usize, (py1 - py0) as usize);
    let mut pix = vec![0u8; w * h];
    for y in y0..y1 {
        let oy = (y as i32 + dy - py0) as usize;
        for x in x0..x1 {
            pix[oy * w + (x as i32 + dx - px0) as usize] = cls[y * gw + x];
        }
    }
    StyledGlyph { ch, w: w as u16, h: h as u16, xc: (-px0) as i16, yc: (-py0) as i16, advance, pix }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_mapping_covers_message_charset() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz.,!'?-%•←✓✗".chars() {
            assert!(original_glyph(c).is_some(), "{c}");
        }
        assert_eq!(original_glyph('A'), Some(17));
        assert_eq!(original_glyph('Z'), Some(42));
        assert_eq!(original_glyph(':'), None);
        assert_eq!(original_advance(0), 16);
        assert_eq!(original_advance(17), 14);
        assert_eq!(original_advance(43), 8);
        assert_eq!(arc_char(b'='), '←');
        assert_eq!(arc_char(b'['), '.');
    }

    const SMALL_BDF: &str = "STARTFONT 2.1\r
FONT -Test-Small\r
SIZE 12 75 75\r
STARTPROPERTIES 2\r
FONT_ASCENT 10\r
FONT_DESCENT 2\r
ENDPROPERTIES\r
CHARS 3\r
STARTCHAR bar\r
ENCODING 124\r
SWIDTH 500 0\r
DWIDTH 3 0\r
BBX 1 4 1 0\r
BITMAP\r
80\r
80\r
80\r
80\r
ENDCHAR\r
STARTCHAR wide\r
ENCODING 12354\r
DWIDTH 12 0\r
BBX 10 2 0 -1\r
BITMAP\r
FC40\r
0460\r
ENDCHAR\r
STARTCHAR unencoded\r
ENCODING -1\r
DWIDTH 4 0\r
BBX 1 1 0 0\r
BITMAP\r
80\r
ENDCHAR\r
ENDFONT\r
";

    #[test]
    fn parses_small_bdf() {
        let f = parse_bdf(SMALL_BDF).unwrap();
        assert_eq!((f.ascent, f.descent), (10, 2));
        assert_eq!(f.glyphs.len(), 2, "unencoded glyph dropped");
        let bar = &f.glyphs[&124];
        assert_eq!(bar.dwidth, 3);
        assert_eq!(bar.bbx, (1, 4, 1, 0));
        assert_eq!(bar.rows, vec![0x8000_0000; 4]);
        let wide = &f.glyphs[&12354];
        assert_eq!(wide.bbx, (10, 2, 0, -1));
        // left aligned; padding bits beyond w=10 (0x0020 in "0460") are cleared
        assert_eq!(wide.rows, vec![0xFC40_0000, 0x0440_0000]);
        assert!(parse_bdf("hello\nworld").is_err());
        assert!(parse_bdf("STARTFONT 2.1\nSTARTCHAR x\nENCODING 65\n").is_err());
        assert!(parse_bdf("STARTFONT 2.1\nSTARTCHAR x\nENCODING 65\nBBX 1 1 0 0\nBITMAP\nZZ\nENDCHAR\n").is_err());
    }

    /// The STARTCHAR block of `enc` from a (large) BDF file, wrapped into a tiny font.
    fn extract_block(path: &str, enc: u32) -> BdfFont {
        // any line ending (a checkout may have CRLF)
        let text = std::fs::read_to_string(path).expect("BDF readable").replace("\r\n", "\n");
        let key = format!("\nENCODING {enc}\n");
        let at = text.find(&key).expect("glyph present");
        let start = text[..at].rfind("STARTCHAR").unwrap();
        let end = start + text[start..].find("ENDCHAR").unwrap() + "ENDCHAR".len();
        let mini = format!("STARTFONT 2.1\nFONT_ASCENT 13\nFONT_DESCENT 3\n{}\nENDFONT\n", &text[start..end]);
        parse_bdf(&mini).unwrap()
    }

    fn fusion(name: &str) -> String {
        format!("{}/../../third_party/fusion-pixel-font-12px-proportional/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    fn rows_of(g: &StyledGlyph) -> Vec<String> {
        (0..g.h as usize)
            .map(|y| {
                (0..g.w as usize)
                    .map(|x| match g.pix[y * g.w as usize + x] {
                        1 => '#',
                        2 => 'o',
                        _ => '.',
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn styles_hangul_ga_like_original() {
        let f = extract_block(&fusion("fusion-pixel-12px-proportional-ko.bdf"), 0xAC00);
        let g = style_glyph('가', &f.glyphs[&0xAC00], f.ascent);
        assert_eq!((g.w, g.h, g.xc, g.yc, g.advance), (14, 14, 7, 7, 15));
        assert_eq!(g.pix.len(), 14 * 14);
        assert!(g.pix.contains(&1) && g.pix.contains(&2));
        let rows = rows_of(&g);
        // BDF row 0 "FC40" = ######...#. -> emboldened, outlined, one row below the box top.
        assert_eq!(rows[0], ".ooooooo..oo..");
        assert_eq!(rows[1], "o#######oo##o.");
        // em rows [-6, 5) carry the core; the cell keeps the original 14-px box
        assert!(rows[13].chars().all(|c| c == '.'));
        // every core pixel's 4-neighbours are core or outline
        let (w, h) = (g.w as usize, g.h as usize);
        for y in 0..h {
            for x in 0..w {
                if g.pix[y * w + x] == 1 {
                    assert!(x > 0 && y > 0 && x + 1 < w && y + 1 < h);
                    for n in [y * w + x - 1, y * w + x + 1, (y - 1) * w + x, (y + 1) * w + x] {
                        assert_ne!(g.pix[n], 0);
                    }
                }
            }
        }
    }

    #[test]
    fn stroke_profile_matches_original_font() {
        // A 1-px vertical stroke becomes the original 2-px stem with a 1-px outline:
        // FSPChars 'I' (glyph 25) rows 4..9 read "....o##o....".
        let a = crate::assets::testdata::assets();
        let i = a.chars.get(25).unwrap();
        let orig: String = (0..i.w as usize)
            .map(|x| match i.pix[5 * i.w as usize + x] {
                Some(crate::palette::FONT_GREEN) => '#',
                Some(crate::palette::FONT_GREEN_DARK) => 'o',
                Some(_) => '?',
                None => '.',
            })
            .collect();
        assert_eq!(orig, "....o##o....");
        let stem = BdfGlyph { dwidth: 6, bbx: (1, 10, 4, 0), rows: vec![0x8000_0000; 10] };
        let g = style_glyph('|', &stem, 10);
        let rows = rows_of(&g);
        assert_eq!(g.w, 4);
        assert_eq!(g.xc, 7 - 4, "outline column at pen - 7 + xoff");
        assert_eq!(rows[(g.yc - 2) as usize], "o##o");
        assert_eq!(rows[(g.yc - 7) as usize], ".oo.", "top cap like the original stems");
        assert_eq!(g.advance, 9);
    }

    #[test]
    fn emboldening_keeps_one_pixel_gaps() {
        // "#.#..#" -> the first stroke must not grow into the gap.
        let g = style_glyph('x', &BdfGlyph { dwidth: 7, bbx: (6, 1, 0, 3), rows: vec![0xA400_0000] }, 10);
        let rows = rows_of(&g);
        assert_eq!(rows[g.yc as usize], "o#o##o##o");
        // empty bitmap: no pixels, but still advances
        let e = style_glyph('\u{3000}', &BdfGlyph { dwidth: 12, bbx: (0, 0, 0, 0), rows: vec![] }, 10);
        assert_eq!((e.w, e.h, e.advance), (0, 0, 15));
        let z = style_glyph('\u{200B}', &BdfGlyph { dwidth: 0, bbx: (0, 0, 0, 0), rows: vec![] }, 10);
        assert_eq!(z.advance, 0);
    }

    #[test]
    fn glyph_set_selection_order_and_fallback() {
        let mut ko = BdfFont { ascent: 13, descent: 3, glyphs: HashMap::new() };
        let mut zh = BdfFont { ascent: 13, descent: 3, glyphs: HashMap::new() };
        let a = BdfGlyph { dwidth: 12, bbx: (2, 2, 0, 0), rows: vec![0xC000_0000, 0x4000_0000] };
        let b = BdfGlyph { dwidth: 12, bbx: (2, 2, 0, 0), rows: vec![0x8000_0000, 0xC000_0000] };
        ko.glyphs.insert('가' as u32, a.clone());
        zh.glyphs.insert('가' as u32, a.clone()); // identical shape -> shared id
        ko.glyphs.insert('中' as u32, a.clone());
        zh.glyphs.insert('中' as u32, b.clone()); // different shape per flavour
        zh.glyphs.insert('：' as u32, b.clone()); // only in zh -> Ko falls back
        ko.glyphs.insert('A' as u32, a.clone()); // covered by the original font
        let mut chars = HashMap::new();
        chars.insert(Flavor::Ko, "가中：A ?☃".chars().collect::<BTreeSet<char>>());
        chars.insert(Flavor::ZhHans, "中가：".chars().collect::<BTreeSet<char>>());
        let set = build_glyph_set(&chars, &ko, &zh);
        // Ko in BTreeSet order: 中(U+4E2D) 가(U+AC00) ：(U+FF1A); then Zh adds its own 中.
        let order: Vec<char> = set.glyphs.iter().map(|g| g.ch).collect();
        assert_eq!(order, vec!['中', '가', '：', '中']);
        assert_eq!(set.index.len(), 6);
        assert_eq!(set.index[&(Flavor::Ko, '가')], set.index[&(Flavor::ZhHans, '가')]);
        assert_eq!(set.index[&(Flavor::Ko, '：')], set.index[&(Flavor::ZhHans, '：')]);
        assert_ne!(set.index[&(Flavor::Ko, '中')], set.index[&(Flavor::ZhHans, '中')]);
        assert_eq!(set.glyphs[set.index[&(Flavor::ZhHans, '中')] as usize], style_glyph('中', &b, 13));
        assert!(set.find(Flavor::Ko, 'A').is_none() && set.find(Flavor::Ko, ' ').is_none());
        assert!(set.find(Flavor::Ko, '☃').is_none());
        // find() falls back to the other flavour
        let mut only_zh = HashMap::new();
        only_zh.insert(Flavor::ZhHans, BTreeSet::from(['中']));
        let s2 = build_glyph_set(&only_zh, &ko, &zh);
        assert_eq!(s2.find(Flavor::Ko, '中'), Some(0));
        // deterministic
        let again = build_glyph_set(&chars, &ko, &zh);
        assert_eq!(again.glyphs, set.glyphs);
        assert_eq!(again.index, set.index);
    }

    #[test]
    fn real_fonts_parse_and_cover_ui_scripts() {
        // Full parse of both shipped source fonts (~37k glyphs each).
        for (name, sample) in [
            ("fusion-pixel-12px-proportional-ko.bdf", '한'),
            ("fusion-pixel-12px-proportional-zh_hans.bdf", '简'),
        ] {
            let f = parse_bdf(&std::fs::read_to_string(fusion(name)).unwrap()).unwrap();
            assert_eq!((f.ascent, f.descent), (13, 3));
            assert!(f.glyphs.len() > 30000);
            let g = style_glyph(sample, &f.glyphs[&(sample as u32)], f.ascent);
            assert_eq!((g.h, g.yc), (14, 7), "{name}");
            assert!(g.w <= 14 && g.pix.contains(&1) && g.pix.contains(&2));
        }
    }
}
