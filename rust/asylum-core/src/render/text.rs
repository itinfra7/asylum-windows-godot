//! Text layout with the original bitmap font + styled CJK glyphs (docs §7).
//!
//! A line is laid out with a pen that represents the *centre* of the current glyph
//! (original semantics): original glyph g is drawn with its sprite hot spot at the pen and
//! the pen advances by `font::original_advance(g)`; a styled glyph likewise uses its
//! `xc/yc` and `advance`; space advances `SPACE_ADVANCE`. The line's box spans
//! `[pen0 - 7, pen0 - 7 + total_advance)`. Characters without any glyph render as '?'.
//!
//! Every character the original font covers uses the original glyph in every language, so
//! digits, Latin letters and `. , ! ' ? - %` inside Korean/Chinese strings look exactly like
//! the original; Hangul/Hanzi and other characters use the styled glyphs of the language's
//! flavour (Korean or Simplified Chinese shapes, falling back to the other flavour); the
//! language names of the language menu always use their own language's shapes.
//! Multi-line strings (`\n`) draw their lines [`LINE_SPACING`] px apart (the original menus
//! use 24-px rows).

use crate::atlas::{Atlas, Bank};
use crate::font::{self, GlyphSet, SPACE_ADVANCE};
use crate::i18n::{keys, Lang, Strings};
use crate::layout::Layout;
use crate::render::{MenuLine, Quad, TexSource, WHITE};
use crate::sim::{Align, TextRef, TextTable};

/// A positioned glyph quad source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlyphRef {
    /// Original FSPChars glyph index.
    Orig(u8),
    /// Index into `GlyphSet::glyphs`.
    Styled(u32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaidOut {
    /// (glyph, pen x relative to the line start pen).
    pub glyphs: Vec<(GlyphRef, i32)>,
    /// Sum of advances.
    pub advance: i32,
}

/// Distance between successive lines of a multi-line string (original menu row pitch).
pub const LINE_SPACING: i32 = 24;

/// Distance from a line's start pen (first glyph centre) to the left edge of its box.
const BOX_LEFT: f32 = 7.0;

/// Original glyph drawn for characters no font covers.
const UNKNOWN: u8 = 12; // '?'

/// Characters that take no space and draw nothing.
fn zero_width(c: char) -> bool {
    c.is_control() || matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// Glyph (None = blank) and pen advance of `c`, or None for zero-width characters.
fn resolve_char(c: char, lang: Lang, glyphs: &GlyphSet) -> Option<(Option<GlyphRef>, i32)> {
    if let Some(g) = font::original_glyph(c) {
        return Some((Some(GlyphRef::Orig(g)), font::original_advance(g)));
    }
    if c == ' ' {
        return Some((None, SPACE_ADVANCE));
    }
    if let Some(id) = glyphs.find(lang.flavor(), c) {
        if let Some(sg) = glyphs.glyphs.get(id as usize) {
            return Some((Some(GlyphRef::Styled(id)), sg.advance as i32));
        }
    }
    if zero_width(c) {
        return None;
    }
    if c.is_whitespace() {
        return Some((None, SPACE_ADVANCE));
    }
    Some((Some(GlyphRef::Orig(UNKNOWN)), font::original_advance(UNKNOWN)))
}

/// Lays out `text` into `out` (cleared first); returns the total advance.
pub(crate) fn layout_into(text: &str, lang: Lang, glyphs: &GlyphSet, out: &mut Vec<(GlyphRef, i32)>) -> i32 {
    out.clear();
    let mut pen = 0i32;
    for c in text.chars() {
        if let Some((g, adv)) = resolve_char(c, lang, glyphs) {
            if let Some(g) = g {
                out.push((g, pen));
            }
            pen += adv;
        }
    }
    pen
}

/// Total advance of `text` without recording glyphs.
fn advance_of(text: &str, lang: Lang, glyphs: &GlyphSet) -> i32 {
    text.chars().filter_map(|c| resolve_char(c, lang, glyphs)).map(|(_, a)| a).sum()
}

/// Lay out one line (no newlines) for `lang`.
pub fn layout_line(text: &str, lang: Lang, glyphs: &GlyphSet) -> LaidOut {
    let mut v = Vec::with_capacity(text.len());
    let advance = layout_into(text, lang, glyphs, &mut v);
    LaidOut { glyphs: v, advance }
}

/// Starting pen x (glyph-centre semantics) for a line given its alignment.
/// `en_advance` is the advance of the English text (for `Align::EnCentered`).
///
/// Offsets are floored to whole pixels so glyphs stay on the pixel grid.
pub fn start_pen(align: crate::sim::Align, x: f32, lang: Lang, advance: i32, en_advance: i32) -> f32 {
    match align {
        Align::Orig => x,
        Align::EnCentered if lang == Lang::En => x,
        // centre of the English box [x-7, x-7+en) → own box start = centre - advance/2
        Align::EnCentered => x + ((en_advance - advance) as f32 * 0.5).floor(),
        Align::Center(cx) => (cx - advance as f32 * 0.5).floor() + BOX_LEFT,
        Align::Right(rx) => (rx - advance as f32).floor() + BOX_LEFT,
    }
}

/// Atlas sprite of a glyph.
pub fn glyph_sprite(atlas: &Atlas, g: GlyphRef) -> Option<&crate::atlas::AtlasSprite> {
    match g {
        GlyphRef::Orig(i) => atlas.sprite(crate::atlas::Bank::Chars, i as usize),
        GlyphRef::Styled(id) => atlas.glyph(id),
    }
}

/// Emits text quads for one frame (message table + menu lines).
pub(crate) struct TextEmitter<'a> {
    pub lay: &'a Layout,
    pub lang: Lang,
    pub tex: &'a dyn TexSource,
    pub strings: &'a Strings,
    pub glyphs: &'a GlyphSet,
    /// Clip index of every text quad.
    pub clip: u16,
}

impl TextEmitter<'_> {
    fn text<'s>(&'s self, t: &'s TextRef, lang: Lang) -> &'s str {
        match t {
            TextRef::Empty => "",
            TextRef::Key(k) => self.strings.get(lang, k),
            TextRef::Raw(s) => s.as_str(),
        }
    }

    /// Language whose glyph forms draw `text`: the language names of the language menu are
    /// autonyms and use their own forms in every UI language (e.g. 文 differs between the
    /// Korean and the Simplified Chinese font).
    fn glyph_lang(&self, text: &TextRef) -> Lang {
        match text {
            TextRef::Key(k) if *k == keys::LANG_ZH => Lang::ZhCn,
            TextRef::Key(k) if *k == keys::LANG_KO => Lang::Ko,
            _ => self.lang,
        }
    }

    /// vdu.c `texthandler` drawing part: every slot with a non-zero count, at `x>>8, y>>8`.
    pub fn emit_table(&self, texts: &TextTable, out: &mut Vec<Quad>) {
        let mut buf = Vec::with_capacity(64);
        for s in texts.visible() {
            // in the whole pixels the text is drawn at: a sub-pixel delta would make texts at
            // fractional speeds jump back at every tick boundary
            let delta = ((s.prev_x >> 8).wrapping_sub(s.x >> 8) as f32, (s.prev_y >> 8).wrapping_sub(s.y >> 8) as f32);
            self.emit(&s.text, (s.x >> 8) as f32, (s.y >> 8) as f32, s.align, None, delta, &mut buf, out);
        }
    }

    /// Static menu lines (no interpolation).
    pub fn emit_lines(&self, lines: &[MenuLine], out: &mut Vec<Quad>) {
        let mut buf = Vec::with_capacity(64);
        for l in lines {
            self.emit(&l.text, l.x, l.y, l.align, l.prefix, (0.0, 0.0), &mut buf, out);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(&self, text: &TextRef, x: f32, y: f32, align: Align, prefix: Option<char>, delta: (f32, f32), buf: &mut Vec<(GlyphRef, i32)>, out: &mut Vec<Quad>) {
        let s = self.text(text, self.lang);
        let gl = self.glyph_lang(text);
        let prefix = prefix.and_then(|c| resolve_char(c, gl, self.glyphs));
        let prefix_adv = prefix.map_or(0, |p| p.1);
        // English reference for EnCentered: line by line when the line counts agree,
        // else the widest English line (the centre of the English block).
        let en_gl = if gl == self.lang { Lang::En } else { gl };
        let en_ref = (align == Align::EnCentered && self.lang != Lang::En).then(|| {
            let en = self.text(text, Lang::En);
            let per_line = en.split('\n').count() == s.split('\n').count();
            let widest = en.split('\n').map(|l| advance_of(l, en_gl, self.glyphs)).max().unwrap_or(0);
            (en, per_line, widest)
        });
        for (i, line) in s.split('\n').enumerate() {
            let first = i == 0;
            let pre = if first { prefix_adv } else { 0 };
            let adv = layout_into(line, gl, self.glyphs, buf) + pre;
            let en_adv = match &en_ref {
                Some((en, true, _)) => en.split('\n').nth(i).map_or(adv, |l| advance_of(l, en_gl, self.glyphs) + pre),
                Some((_, false, widest)) => widest + prefix_adv,
                None => adv,
            };
            let pen0 = start_pen(align, x, self.lang, adv, en_adv);
            let ly = y + (LINE_SPACING * i as i32) as f32;
            if first {
                if let Some((Some(g), _)) = prefix {
                    self.glyph(g, pen0, ly, delta, out);
                }
            }
            let pen = pen0 + pre as f32;
            for &(g, rel) in buf.iter() {
                self.glyph(g, pen + rel as f32, ly, delta, out);
            }
        }
    }

    /// Glyph `g` with its hot spot at content position (x, y).
    fn glyph(&self, g: GlyphRef, x: f32, y: f32, delta: (f32, f32), out: &mut Vec<Quad>) {
        let s = match g {
            GlyphRef::Orig(i) => self.tex.spr(Bank::Chars, i as usize),
            GlyphRef::Styled(id) => self.tex.glyph_spr(id),
        };
        let Some(s) = s else { return };
        out.push(Quad {
            x: self.lay.vx(x - s.xc as f32),
            y: self.lay.vy(y - s.yc as f32),
            w: s.w as f32,
            h: s.h as f32,
            u: s.u as f32,
            v: s.v as f32,
            uw: s.w as f32,
            vh: s.h as f32,
            dx: delta.0,
            dy: delta.1,
            color: WHITE,
            clip: self.clip,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::testkit::*;
    use crate::render::{build_frame_with, Background, RenderFrame, SceneDesc, CLIP_CHAT, CLIP_NONE};
    use crate::sim::TextSlot;

    fn advances(l: &LaidOut) -> Vec<i32> {
        l.glyphs.iter().map(|g| g.1).collect()
    }

    #[test]
    fn layout_uses_original_advances() {
        let g = GlyphSet::default();
        // 'SCORE 10': letters 14, space 8, digits 16
        let l = layout_line("SCORE 10", Lang::En, &g);
        assert_eq!(advances(&l), vec![0, 14, 28, 42, 56, 78, 94]);
        assert_eq!(l.advance, 5 * 14 + 8 + 2 * 16);
        assert_eq!(l.glyphs[0].0, GlyphRef::Orig(17 + 18)); // 'S'
        assert_eq!(l.glyphs[5].0, GlyphRef::Orig(1));
        // lower case maps to the upper-case glyphs, punctuation advances 8
        let l = layout_line("Let's Go!", Lang::En, &g);
        assert_eq!(l.glyphs.iter().map(|g| g.0).collect::<Vec<_>>()[..4], [GlyphRef::Orig(28), GlyphRef::Orig(21), GlyphRef::Orig(36), GlyphRef::Orig(46)]);
        assert_eq!(advances(&l), vec![0, 14, 28, 42, 50, 72, 86, 100]);
        assert_eq!(l.advance, 108);
    }

    /// The pen positions reproduce vdu.c texthandler's `XxX` sequence for `message()` codes.
    #[test]
    fn layout_matches_texthandler_loop() {
        let g = GlyphSet::default();
        let s = "\u{2022} Snuffed It! \u{2022}";
        // C: message() translation (164 → ':', '!' → '}', >= 96 → -32, space → 0xff, else
        // -47), then texthandler: XxX += 14, +2 if <= 10, -6 if > 43
        let codes: Vec<u8> = s
            .chars()
            .map(|c| {
                let mut q = match c {
                    '\u{2022}' => b':',
                    '!' => b'}',
                    c => c as u8,
                };
                if q >= 96 {
                    q -= 32;
                }
                if q == b' ' {
                    0xff
                } else {
                    q - 47
                }
            })
            .collect();
        let mut x = 0;
        let mut want = vec![];
        for &c in &codes {
            if c.wrapping_sub(1) < 48 {
                want.push((c - 1, x));
            }
            x += 14;
            if c <= 10 {
                x += 2;
            }
            if c > 43 {
                x -= 6;
            }
        }
        let l = layout_line(s, Lang::En, &g);
        let got: Vec<(u8, i32)> = l.glyphs.iter().map(|&(g, p)| (if let GlyphRef::Orig(i) = g { i } else { 255 }, p)).collect();
        assert_eq!(got, want);
        assert_eq!(l.advance, x);
    }

    #[test]
    fn layout_styled_unknown_and_zero_width() {
        let tk = TestKit::get();
        let g = &tk.glyphs;
        // Hangul from the Korean flavour, ':' styled, digits original, unknown → '?'
        let l = layout_line("가:1\u{263A}\u{200B}\r", Lang::Ko, g);
        let ga = g.find(crate::font::Flavor::Ko, '가').unwrap();
        let colon = g.find(crate::font::Flavor::Ko, ':').unwrap();
        assert_eq!(l.glyphs, vec![(GlyphRef::Styled(ga), 0), (GlyphRef::Styled(colon), 15), (GlyphRef::Orig(1), 15 + 7), (GlyphRef::Orig(12), 15 + 7 + 16)]);
        assert_eq!(l.advance, 15 + 7 + 16 + 14);
        // Chinese flavour picks its own shape, falls back to the Korean set otherwise
        let zh = layout_line("中가", Lang::ZhCn, g);
        assert_eq!(zh.glyphs[0].0, GlyphRef::Styled(g.find(crate::font::Flavor::ZhHans, '中').unwrap()));
        assert_eq!(zh.glyphs[1].0, GlyphRef::Styled(ga));
        let colon_zh = layout_line(":", Lang::ZhCn, g);
        assert_ne!(colon_zh.glyphs[0].0, GlyphRef::Styled(colon), "per-flavour glyph");
        // ideographic space from the glyph set (blank, advances), NBSP like a space
        let sp = layout_line("가\u{3000}가\u{a0}1", Lang::Ko, g);
        assert_eq!(advances(&sp), vec![0, 15, 30, 53]);
        assert_eq!(sp.advance, 15 * 3 + 8 + 16);
        // without a glyph set every uncovered character is '?'
        let none = layout_line("가", Lang::Ko, &GlyphSet::default());
        assert_eq!(none.glyphs, vec![(GlyphRef::Orig(12), 0)]);
    }

    #[test]
    fn start_pen_rules() {
        // Orig: x is the first glyph centre in every language
        assert_eq!(start_pen(Align::Orig, 32.0, Lang::Ko, 200, 100), 32.0);
        // EnCentered: English identical to Orig
        assert_eq!(start_pen(Align::EnCentered, 88.0, Lang::En, 150, 150), 88.0);
        // other languages: box centred on the English box centre
        // English box [81, 81+150) centre 156; KO advance 90 → box [111, 201) → pen 118
        assert_eq!(start_pen(Align::EnCentered, 88.0, Lang::Ko, 90, 150), 118.0);
        assert_eq!(start_pen(Align::EnCentered, 88.0, Lang::ZhCn, 151, 150), 87.0, "floored");
        // Center: box [cx - adv/2, ...) → pen = box start + 7
        assert_eq!(start_pen(Align::Center(160.0), 0.0, Lang::En, 100, 0), 117.0);
        assert_eq!(start_pen(Align::Center(160.0), 0.0, Lang::Ko, 101, 0), 116.0);
        // the box of a laid out line is centred
        let g = GlyphSet::default();
        let l = layout_line("OPTIONS", Lang::En, &g);
        let pen = start_pen(Align::Center(160.0), 0.0, Lang::En, l.advance, 0);
        assert_eq!(pen - 7.0, 160.0 - l.advance as f32 / 2.0);
    }

    fn text_quads(lang: Lang, bg: Background, texts: &TextTable, lines: &[MenuLine]) -> Vec<Quad> {
        let tk = TestKit::get();
        let sc = SceneDesc { lay: Layout::original(), lang, background: bg, sim: None, texts, lines, highlight: None, snap: false, no_interp: false };
        let mut f = RenderFrame::default();
        build_frame_with(&sc, &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        let mut screen = Vec::new();
        match bg {
            Background::Chat => crate::render::frame::chat_screen_quads(&Layout::original(), &tk.atlas, &mut screen),
            Background::GameFrame | Background::Game => crate::render::frame::game_screen_quads(&Layout::original(), &tk.atlas, &mut screen),
            Background::Black => {}
        }
        f.quads.split_off(screen.len())
    }

    fn slot(x: i32, y: i32, text: TextRef, align: Align) -> TextSlot {
        TextSlot { count: 400, x: x << 8, y: y << 8, prev_x: x << 8, prev_y: y << 8, text, align, ..TextSlot::default() }
    }

    #[test]
    fn message_table_draws_like_texthandler() {
        let tk = TestKit::get();
        let mut t = TextTable::default();
        t.slots[3] = slot(128, 48, TextRef::Key("T_OPTIONS"), Align::Orig);
        t.slots[3].prev_x = (128 + 3) << 8; // moved 3 px left this tick
        t.slots[5] = TextSlot { count: 0, ..slot(10, 10, TextRef::Raw("X".into()), Align::Orig) };
        let q = text_quads(Lang::En, Background::Chat, &t, &[]);
        assert_eq!(q.len(), 7, "OPTIONS; the slot with count 0 is not drawn");
        let o = tk.atlas.spr(Bank::Chars, 17 + 14).unwrap();
        assert_eq!((q[0].x, q[0].y), (128.0 - o.xc as f32, 48.0 - o.yc as f32));
        assert_eq!((q[0].dx, q[0].dy, q[0].clip), (3.0, 0.0, CLIP_CHAT));
        let p = tk.atlas.spr(Bank::Chars, 17 + 15).unwrap();
        assert_eq!(q[1].x, 128.0 + 14.0 - p.xc as f32);
        // in game the texts are clipped to the game window, elsewhere unclipped
        let q = text_quads(Lang::En, Background::GameFrame, &t, &[]);
        assert_eq!(q[0].clip, CLIP_NONE);
    }

    #[test]
    fn fractional_speed_texts_move_continuously() {
        // a text rising 0.5 px per tick (MSG_POTION / MSG_RISE): where tick n ends (alpha 1)
        // is where tick n+1 starts (alpha 0), and nothing ever moves backwards
        let mut last_end: Option<f32> = None;
        let mut y = (24 << 8) + 128;
        for _ in 0..8 {
            let (prev, cur) = (y, y + 128);
            y = cur;
            let mut t = TextTable::default();
            t.slots[0] = TextSlot { prev_y: prev, y: cur, ..slot(100, 0, TextRef::Raw("X".into()), Align::Orig) };
            let q = text_quads(Lang::En, Background::Chat, &t, &[]);
            let start = q[0].y + q[0].dy; // alpha 0: the previous position
            let end = q[0].y; // alpha 1
            if let Some(e) = last_end {
                assert_eq!(start, e, "tick boundary at {cur:#x}");
            }
            assert!(end >= start);
            last_end = Some(end);
        }
    }

    #[test]
    fn multiline_and_en_centered() {
        let tk = TestKit::get();
        let mut t = TextTable::default();
        t.slots[0] = slot(32, 96, TextRef::Key("T_TWO_LINES"), Align::EnCentered);
        let en = text_quads(Lang::En, Background::Black, &t, &[]);
        // EN: "AB" / "C" at x = 32, rows 24 px apart
        assert_eq!(en.len(), 3);
        let a = tk.atlas.spr(Bank::Chars, 17).unwrap();
        let c = tk.atlas.spr(Bank::Chars, 19).unwrap();
        assert_eq!((en[0].x, en[0].y), (32.0 - a.xc as f32, 96.0 - a.yc as f32));
        assert_eq!((en[2].x, en[2].y), (32.0 - c.xc as f32, 120.0 - c.yc as f32));
        // KO: "가가가" / "가" centred line by line on the English lines
        let ko = text_quads(Lang::Ko, Background::Black, &t, &[]);
        assert_eq!(ko.len(), 4);
        let ga = tk.atlas.glyph_spr(tk.glyphs.find(crate::font::Flavor::Ko, '가').unwrap()).unwrap();
        let pen0 = start_pen(Align::EnCentered, 32.0, Lang::Ko, 45, 28);
        assert_eq!(pen0, 23.0, "32 + floor((28 - 45) / 2)");
        assert_eq!((ko[0].x, ko[0].y), (pen0 - ga.xc as f32, 96.0 - ga.yc as f32));
        let pen1 = start_pen(Align::EnCentered, 32.0, Lang::Ko, 15, 14);
        assert_eq!((ko[3].x, ko[3].y), (pen1 - ga.xc as f32, 120.0 - ga.yc as f32));
    }

    #[test]
    fn menu_line_prefix_replaces_first_character() {
        let tk = TestKit::get();
        let t = TextTable::default();
        // original: message(32, 96, "-1. Full Screen") with '-' replaced by a tick
        let lines = [MenuLine { text: TextRef::Raw("1. Full".into()), x: 32.0, y: 96.0, align: Align::Orig, prefix: Some('✓') }];
        let q = text_quads(Lang::En, Background::Chat, &t, &lines);
        let tick = tk.atlas.spr(Bank::Chars, 15).unwrap();
        let one = tk.atlas.spr(Bank::Chars, 1).unwrap();
        assert_eq!(q[0].x, 32.0 - tick.xc as f32);
        assert_eq!(q[1].x, 46.0 - one.xc as f32);
        assert_eq!(q.len(), 7);
        // the prefix alone (text Empty) still draws
        let lines = [MenuLine { text: TextRef::Empty, x: 48.0, y: 96.0, align: Align::Orig, prefix: Some('✗') }];
        assert_eq!(text_quads(Lang::En, Background::Chat, &t, &lines).len(), 1);
        // centred lines include the prefix in their width
        let lines = [MenuLine { text: TextRef::Raw("AB".into()), x: 0.0, y: 96.0, align: Align::Center(160.0), prefix: Some('✓') }];
        let q = text_quads(Lang::En, Background::Chat, &t, &lines);
        assert_eq!(q[0].x, start_pen(Align::Center(160.0), 0.0, Lang::En, 42, 0) - tick.xc as f32);
    }

    #[test]
    fn language_names_use_their_own_glyph_forms() {
        let tk = TestKit::get();
        let t = TextTable::default();
        let ko_colon = tk.atlas.glyph_spr(tk.glyphs.find(crate::font::Flavor::Ko, ':').unwrap()).unwrap();
        let zh_colon = tk.atlas.glyph_spr(tk.glyphs.find(crate::font::Flavor::ZhHans, ':').unwrap()).unwrap();
        let line = |key| [MenuLine { text: TextRef::Key(key), x: 64.0, y: 96.0, align: Align::Orig, prefix: None }];
        // the test table spells both names ":" so only the glyph forms differ
        for lang in Lang::ALL {
            let zh = text_quads(lang, Background::Chat, &t, &line(keys::LANG_ZH));
            assert_eq!((zh[0].u, zh[0].w), (zh_colon.u as f32, zh_colon.w as f32), "{lang:?}");
            let ko = text_quads(lang, Background::Chat, &t, &line(keys::LANG_KO));
            assert_eq!((ko[0].u, ko[0].w), (ko_colon.u as f32, ko_colon.w as f32), "{lang:?}");
        }
        // ordinary keys follow the UI language
        let opt = [MenuLine { text: TextRef::Raw(":".into()), x: 64.0, y: 96.0, align: Align::Orig, prefix: None }];
        assert_eq!(text_quads(Lang::En, Background::Chat, &t, &opt)[0].u, ko_colon.u as f32);
        assert_eq!(text_quads(Lang::ZhCn, Background::Chat, &t, &opt)[0].u, zh_colon.u as f32);
    }

    #[test]
    fn mixed_script_line_is_contiguous() {
        // Latin glyphs inside a Korean string stay original, styled glyphs follow the pen
        let tk = TestKit::get();
        let mut t = TextTable::default();
        t.slots[0] = slot(40, 60, TextRef::Raw("1가A".into()), Align::Orig);
        let q = text_quads(Lang::Ko, Background::Black, &t, &[]);
        assert_eq!(q.len(), 3);
        let ga = tk.atlas.glyph_spr(tk.glyphs.find(crate::font::Flavor::Ko, '가').unwrap()).unwrap();
        let a = tk.atlas.spr(Bank::Chars, 17).unwrap();
        assert_eq!(q[1].x, 40.0 + 16.0 - ga.xc as f32);
        assert_eq!(q[2].x, 40.0 + 16.0 + 15.0 - a.xc as f32);
        // all glyphs share the original vertical centre
        for (quad, yc) in q.iter().zip([7, ga.yc, a.yc]) {
            assert_eq!(quad.y + yc as f32, 60.0);
        }
    }
}
