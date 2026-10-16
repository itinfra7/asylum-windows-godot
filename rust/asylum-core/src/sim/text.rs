//! vdu.c message table (`texttabofs[_textno]`, `message`, `texthandler`, `wipetexttab`).
//!
//! Text is stored as a key or raw string (not as translated Archimedes codes); the renderer
//! localises and lays it out (font.rs / render/text.rs). Lifetime encoding of `message()`:
//! x > 1000 → time 50 (x -= 1000); then x > 1000 again → permanent 0x10000; default 400.

use crate::i18n::keys;
use crate::layout::Layout;
use crate::sim::Sim;

/// What to display.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum TextRef {
    #[default]
    Empty,
    /// Translation key (see i18n::keys).
    Key(&'static str),
    /// Literal text (scores, initials…) — rendered with the font tables as is.
    Raw(String),
}

/// Horizontal alignment of a text item (docs §7).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Align {
    /// `x` is the centre of the first glyph (original semantics) in every language.
    #[default]
    Orig,
    /// English: identical to `Orig`. Other languages: centred on the centre of the English
    /// text placed at `x` with `Orig` semantics.
    EnCentered,
    /// Centred on content x = value.
    Center(f32),
    /// The text box ends at content x = value (menu values).
    Right(f32),
}

/// What a text is positioned relative to, so a layout change (window resized, display mode or
/// scale changed) moves it along instead of leaving it at stale coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anchor {
    /// The centred original 320x256 band (`hx`, `cy`): pause screen, potion, completion.
    #[default]
    Band,
    /// The bottom of the screen (`hx`, `hy`): bonus messages rising from the HUD, the intro.
    Bottom,
    /// The right edge and bottom of the game window: `message_scroll`.
    GameRight,
    /// The centre and bottom of the game window: death and game-over messages.
    GameCentre,
    /// The centre and lower third of the game window: "Let's Go!".
    GameThird,
}

impl Anchor {
    /// Content-coordinate move of a text with this anchor from layout `a` to layout `b`.
    pub fn shift(self, a: &Layout, b: &Layout) -> (i32, i32) {
        match self {
            Anchor::Band => (b.hx - a.hx, b.cy - a.cy),
            Anchor::Bottom => (b.hx - a.hx, b.hy - a.hy),
            Anchor::GameRight => (b.gamew - a.gamew, b.gameh - a.gameh),
            Anchor::GameCentre => (b.gamew / 2 - a.gamew / 2, b.gameh - a.gameh),
            Anchor::GameThird => (b.gamew / 2 - a.gamew / 2, b.gameh * 2 / 3 - a.gameh * 2 / 3),
        }
    }
}

/// One `textinfo` entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSlot {
    pub count: i32,
    /// 8.8 fixed point position (content coordinates) and velocity.
    pub x: i32,
    pub y: i32,
    pub dx: i32,
    pub dy: i32,
    pub text: TextRef,
    pub align: Align,
    /// Incremented whenever the slot is (re)used — interpolation must not blend across.
    pub gen: u32,
    /// Position before the last animation step (for interpolation), 8.8.
    pub prev_x: i32,
    pub prev_y: i32,
    pub anchor: Anchor,
}

#[derive(Clone, Debug)]
pub struct TextTable {
    pub slots: Vec<TextSlot>,
}

impl Default for TextTable {
    fn default() -> Self {
        TextTable { slots: vec![TextSlot::default(); crate::sim::TEXTNO] }
    }
}

/// Lifetime decoding of `message()`'s x: returns (x, time).
fn decode_x(x: i32) -> (i32, i32) {
    let (mut x, mut time) = (x, 400);
    if x > 1000 {
        x -= 1000;
        time = 50;
    }
    if x > 1000 {
        x -= 1000;
        time = 0x10000;
    }
    (x, time)
}

impl TextTable {
    /// vdu.c `message(x, y, xv, yv, a)`: allocate the first free slot (count == 0); silently
    /// ignored when the table is full.
    pub fn message(&mut self, x: i32, y: i32, xv: f32, yv: f32, text: TextRef, align: Align) {
        let (x, time) = decode_x(x);
        self.alloc(time, x, y, xv, yv, text, align, Anchor::Band);
    }

    /// `messageproc` part of `message()` with the lifetime already decoded.
    #[allow(clippy::too_many_arguments)]
    fn alloc(&mut self, time: i32, x: i32, y: i32, xv: f32, yv: f32, text: TextRef, align: Align, anchor: Anchor) {
        // loopa8:
        let Some(r11) = self.slots.iter_mut().find(|s| s.count == 0) else {
            return;
        };
        r11.count = time;
        r11.x = x << 8;
        r11.y = y << 8;
        r11.dx = (xv * 256.0) as i32;
        r11.dy = (yv * 256.0) as i32;
        r11.text = text;
        r11.align = align;
        r11.anchor = anchor;
        r11.gen = r11.gen.wrapping_add(1);
        r11.prev_x = r11.x;
        r11.prev_y = r11.y;
    }

    /// vdu.c `wipetexttab()`.
    pub fn wipe(&mut self) {
        for s in self.slots.iter_mut() {
            let gen = s.gen;
            *s = TextSlot::default();
            s.gen = gen.wrapping_add(1);
        }
    }

    /// Animation part of vdu.c `texthandler(do_animation)`: decrement counters and move
    /// (frameinc is 1). Records `prev_x/prev_y` first.
    pub fn animate(&mut self, frameinc: i32) {
        for r11 in self.slots.iter_mut() {
            if r11.count == 0 {
                continue;
            }
            r11.prev_x = r11.x;
            r11.prev_y = r11.y;
            // C `frameinc` is an unsigned char
            let r4 = (frameinc as u8).min(4) as i32;
            r11.count -= r4;
            if r11.count < 0 {
                r11.count = 0;
            }
            for _ in 0..r4 {
                r11.x = r11.x.wrapping_add(r11.dx);
                r11.y = r11.y.wrapping_add(r11.dy);
            }
        }
    }

    /// Marks every slot as not moving this tick (prev = current), for ticks on which the
    /// owner does not call `animate` (static menus, finished slide-ins). Without it the
    /// renderer would keep interpolating the last animation step.
    pub fn hold(&mut self) {
        for s in self.slots.iter_mut() {
            s.prev_x = s.x;
            s.prev_y = s.y;
        }
    }

    /// Iterate visible slots.
    pub fn visible(&self) -> impl Iterator<Item = &TextSlot> {
        self.slots.iter().filter(|s| s.count != 0)
    }

    /// Moves every text from layout `a` to layout `b` along its anchor (position, previous
    /// position and absolute alignment x).
    pub fn relayout(&mut self, a: &Layout, b: &Layout) {
        for s in self.slots.iter_mut() {
            let (dx, dy) = s.anchor.shift(a, b);
            if dx == 0 && dy == 0 {
                continue;
            }
            s.x = s.x.wrapping_add(dx << 8);
            s.prev_x = s.prev_x.wrapping_add(dx << 8);
            s.y = s.y.wrapping_add(dy << 8);
            s.prev_y = s.prev_y.wrapping_add(dy << 8);
            if let Align::Center(c) | Align::Right(c) = &mut s.align {
                *c += dx as f32;
            }
        }
    }
}

/// Lifetime of a message that travels `extra` pixels further than in the original 288x192
/// window at `speed` px/tick: the original 400 ticks plus the time for the extra distance, so
/// it still leaves the window before it disappears.
fn travel_time(extra: i32, speed: i32) -> i32 {
    400 + (extra.max(0) + speed - 1) / speed
}

impl Sim {
    // The game-window messages below take positions from the window size, which can exceed
    // 1000 in large views: they are allocated directly, never through message()'s lifetime
    // encoding (which would read such an x as "50 ticks" or "permanent").

    /// vdu.c `message_scroll(a)`: scrolls in from the right edge of the game window (lives
    /// long enough to cross a wider window).
    pub fn message_scroll(&mut self, text: TextRef) {
        let l = self.lay;
        let time = travel_time(l.gamew - 288, 3);
        self.texts.alloc(time, l.gamex + l.gamew + 32, l.gamey + l.gameh - 8, -3.0, 0.0, text, Align::Orig, Anchor::GameRight);
    }

    /// vdu.c `startmessage()` — "Let's Go!" (centred in the game window, 50 ticks).
    pub fn startmessage(&mut self) {
        let l = self.lay;
        let (x, y) = (l.gamex + l.gamew / 2 - 32, l.gamey + (l.gameh * 2) / 3);
        self.texts.alloc(50, x, y, 0.0, 0.0, TextRef::Key(keys::MSG_LETS_GO), Align::EnCentered, Anchor::GameThird);
    }

    /// vdu.c `deathmessage()` — "• Snuffed It! •" (rises out of a taller window too).
    pub fn deathmessage(&mut self) {
        let l = self.lay;
        let time = travel_time(l.gameh - 192, 1);
        let (x, y) = (l.gamex + l.gamew / 2 - 88, l.gamey + l.gameh + 8);
        self.texts.alloc(time, x, y, 0.0, -1.0, TextRef::Key(keys::MSG_SNUFFED), Align::EnCentered, Anchor::GameCentre);
    }

    /// vdu.c `endgamemessage()` — "-  GAME OVER  -".
    pub fn endgamemessage(&mut self) {
        let l = self.lay;
        let time = travel_time(l.gameh - 192, 1);
        let (x, y) = (l.gamex + l.gamew / 2 - 88, l.gamey + l.gameh + 56);
        self.texts.alloc(time, x, y, 0.0, -1.0, TextRef::Key(keys::MSG_GAME_OVER), Align::EnCentered, Anchor::GameCentre);
    }

    /// `message()` with an x/y given in the original 320x256 screen, for texts placed around
    /// the screen centre (potion, completion, pause menu): offsets x by `lay.hx` and y by
    /// `lay.cy` (keeps lifetime-encoding offsets of +1000/+2000 working).
    pub fn message_hud(&mut self, x: i32, y: i32, xv: f32, yv: f32, text: TextRef, align: crate::sim::Align) {
        let (x, time) = decode_x(x);
        self.message_band(time, x, y, xv, yv, text, align);
    }

    /// [`Sim::message_hud`] with an explicit lifetime (and no lifetime encoding of x).
    #[allow(clippy::too_many_arguments)]
    pub fn message_band(&mut self, time: i32, x: i32, y: i32, xv: f32, yv: f32, text: TextRef, align: crate::sim::Align) {
        let l = self.lay;
        self.texts.alloc(time, x + l.hx, y + l.cy, xv, yv, text, align, Anchor::Band);
    }

    /// [`Sim::message_hud`] for texts anchored at the bottom of the screen (bonus messages
    /// rising from the HUD panel, the intro scroll): y is offset by `lay.hy`; a moving text of
    /// normal lifetime lives long enough to leave a taller screen at the top.
    pub fn message_hud_bottom(&mut self, x: i32, y: i32, xv: f32, yv: f32, text: TextRef, align: crate::sim::Align) {
        let l = self.lay;
        let (x, mut time) = decode_x(x);
        if time == 400 && yv < 0.0 {
            time = time.max(((y + l.hy + 16) as f32 / -yv) as i32 + 1);
        }
        self.texts.alloc(time, x + l.hx, y + l.hy, xv, yv, text, align, Anchor::Bottom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use crate::sim::TEXTNO;

    #[test]
    fn lifetime_encoding() {
        let mut t = TextTable::default();
        t.message(100, 50, 0.0, 0.0, TextRef::Raw("A".into()), Align::Orig);
        t.message(1100, 50, 0.0, 0.0, TextRef::Raw("B".into()), Align::Orig);
        t.message(2100, 50, 0.0, 0.0, TextRef::Raw("C".into()), Align::Orig);
        t.message(1000, 50, 0.0, 0.0, TextRef::Raw("D".into()), Align::Orig);
        t.message(2000, 50, 0.0, 0.0, TextRef::Raw("E".into()), Align::Orig);
        let got: Vec<(i32, i32)> = t.slots[..5].iter().map(|s| (s.count, s.x >> 8)).collect();
        assert_eq!(got, vec![(400, 100), (50, 100), (0x10000, 100), (400, 1000), (50, 1000)]);
        assert_eq!(t.slots[0].y, 50 << 8);
        assert_eq!(t.slots[5].count, 0);
    }

    #[test]
    fn allocation_first_free_and_full_table() {
        let mut t = TextTable::default();
        for i in 0..TEXTNO as i32 {
            t.message(i, 0, 0.0, 0.0, TextRef::Empty, Align::Orig);
        }
        assert_eq!(t.visible().count(), TEXTNO);
        let before: Vec<TextSlot> = t.slots.clone();
        t.message(500, 0, 0.0, 0.0, TextRef::Raw("X".into()), Align::Orig);
        assert_eq!(t.slots, before, "full table: message ignored");
        // free slot 3, the next message takes it and bumps its generation
        t.slots[3].count = 0;
        let g = t.slots[3].gen;
        t.message(77, 9, 1.5, -2.0, TextRef::Key("K"), Align::EnCentered);
        let s = &t.slots[3];
        assert_eq!((s.x, s.y, s.dx, s.dy), (77 << 8, 9 << 8, 384, -512));
        assert_eq!((s.prev_x, s.prev_y), (s.x, s.y));
        assert_eq!(s.gen, g + 1);
        assert_eq!(s.text, TextRef::Key("K"));
        assert_eq!(s.align, Align::EnCentered);
        // velocities truncate like the C double->int cast
        t.slots[4].count = 0;
        t.message(0, 0, -0.999, 0.004, TextRef::Empty, Align::Orig);
        assert_eq!((t.slots[4].dx, t.slots[4].dy), (-255, 1));
    }

    #[test]
    fn animation_moves_and_expires() {
        let mut t = TextTable::default();
        t.message(1010, 20, -3.0, 0.5, TextRef::Empty, Align::Orig); // time 50
        t.animate(1);
        let s = &t.slots[0];
        assert_eq!(s.count, 49);
        assert_eq!((s.prev_x, s.prev_y), (10 << 8, 20 << 8));
        assert_eq!((s.x, s.y), ((10 << 8) - 768, (20 << 8) + 128));
        // frameinc is capped at 4 steps per call
        t.animate(9);
        let s = &t.slots[0];
        assert_eq!(s.count, 45);
        assert_eq!(s.x, (10 << 8) - 5 * 768);
        assert_eq!(s.prev_x, (10 << 8) - 768);
        for _ in 0..45 {
            t.animate(1);
        }
        assert_eq!(t.slots[0].count, 0);
        assert_eq!(t.visible().count(), 0);
        // expired slots no longer move; the count never goes negative
        let x = t.slots[0].x;
        t.slots[1].count = 2;
        t.animate(4);
        assert_eq!(t.slots[0].x, x);
        assert_eq!(t.slots[1].count, 0);
    }

    #[test]
    fn wipe_clears_and_bumps_gen() {
        let mut t = TextTable::default();
        t.message(5, 5, 0.0, 0.0, TextRef::Raw("Z".into()), Align::Orig);
        let g = t.slots[0].gen;
        t.wipe();
        assert_eq!(t.visible().count(), 0);
        assert_eq!(t.slots[0].gen, g + 1);
        assert_eq!(t.slots[0].text, TextRef::Empty);
    }

    #[test]
    fn game_messages_original_positions() {
        let mut s = crate::sim::testutil::sim();
        s.startmessage();
        s.deathmessage();
        s.endgamemessage();
        s.message_scroll(TextRef::Raw("GUN".into()));
        let v: Vec<(i32, i32, i32, i32, Align)> =
            s.texts.slots[..4].iter().map(|t| (t.count, t.x >> 8, t.y >> 8, t.dy, t.align)).collect();
        // vduvar: gamex 16, gamey 8, gamew 288, gameh 192
        assert_eq!(v[0], (50, 128, 136, 0, Align::EnCentered));
        assert_eq!(v[1], (400, 72, 208, -256, Align::EnCentered));
        assert_eq!(v[2], (400, 72, 256, -256, Align::EnCentered));
        assert_eq!(v[3], (400, 336, 192, 0, Align::Orig));
        assert_eq!(s.texts.slots[3].dx, -768);
        assert_eq!(s.texts.slots[0].text, TextRef::Key(keys::MSG_LETS_GO));
        assert_eq!(s.texts.slots[1].text, TextRef::Key(keys::MSG_SNUFFED));
        assert_eq!(s.texts.slots[2].text, TextRef::Key(keys::MSG_GAME_OVER));
    }

    #[test]
    fn wide_layout_messages_and_hud_offset() {
        let mut s = crate::sim::testutil::sim();
        s.lay = Layout::new(454.0, 256.0); // cw 454, hx 67, gamew 422
        s.startmessage();
        s.message_scroll(TextRef::Empty);
        s.message_hud(96, 224, 0.0, -2.0, TextRef::Empty, Align::EnCentered);
        s.message_hud(1000 + 96, 224, 0.0, 0.0, TextRef::Empty, Align::Orig);
        s.message_hud(2000 + 990, 224, 0.0, 0.0, TextRef::Empty, Align::Orig);
        let v: Vec<(i32, i32)> = s.texts.slots[..5].iter().map(|t| (t.count, t.x >> 8)).collect();
        assert_eq!(v[0], (50, 16 + 211 - 32));
        // the scroll lives long enough to cross the wider window: 400 + (422 - 288) / 3
        assert_eq!(v[1], (445, 16 + 422 + 32));
        assert_eq!(v[2], (400, 96 + 67));
        assert_eq!(v[3], (50, 96 + 67));
        assert_eq!(v[4], (0x10000, 990 + 67));
    }

    #[test]
    fn large_window_messages_skip_the_lifetime_encoding() {
        let mut s = crate::sim::testutil::sim();
        s.lay = Layout::new(1820.0, 768.0); // game window 1788x704
        s.message_scroll(TextRef::Empty);
        s.startmessage();
        s.deathmessage();
        s.endgamemessage();
        let v: Vec<(i32, i32, i32)> = s.texts.slots[..4].iter().map(|t| (t.count, t.x >> 8, t.y >> 8)).collect();
        assert_eq!(v[0], (900, 16 + 1788 + 32, 8 + 704 - 8), "scroll from the right edge");
        assert_eq!(v[1], (50, 16 + 894 - 32, 8 + 469), "Let's Go! for 50 ticks");
        assert_eq!(v[2], (912, 16 + 894 - 88, 8 + 704 + 8), "Snuffed It!");
        assert_eq!(v[3], (912, 16 + 894 - 88, 8 + 704 + 56), "GAME OVER");
        // each moving text has left the window when it expires, like in the original
        let scroll_end = v[0].1 - 3 * v[0].0;
        assert!(scroll_end < 16 - 400, "scroll ends at {scroll_end}");
        assert!(v[2].2 - v[2].0 < 8 - 16, "death text ends inside the window");
        // even wider windows never turn "Let's Go!" into a permanent text
        s.texts.wipe();
        s.lay = Layout::new(2731.0, 768.0);
        s.startmessage();
        assert_eq!(s.texts.slots[0].count, 50);
    }

    #[test]
    fn texts_follow_their_anchor_when_the_layout_changes() {
        let (a, b) = (Layout::new(1138.0, 640.0), Layout::new(1820.0, 768.0));
        let mut s = crate::sim::testutil::sim();
        s.lay = a;
        s.message_hud(100, 100, 0.0, 0.0, TextRef::Empty, Align::Orig);
        s.message_hud_bottom(96, 224, 0.0, -2.0, TextRef::Empty, Align::EnCentered);
        s.message_scroll(TextRef::Empty);
        s.deathmessage();
        s.startmessage();
        s.message_band(400, 0, 50, 0.0, 0.0, TextRef::Empty, Align::Center((160 + a.hx) as f32));
        let rel = |t: &TextSlot, l: &Layout| -> (i32, i32) {
            let (x, y) = (t.x >> 8, t.y >> 8);
            match t.anchor {
                Anchor::Band => (x - l.hx, y - l.cy),
                Anchor::Bottom => (x - l.hx, y - l.hy),
                Anchor::GameRight => (x - (l.gamex + l.gamew), y - (l.gamey + l.gameh)),
                Anchor::GameCentre => (x - l.gamew / 2, y - (l.gamey + l.gameh)),
                Anchor::GameThird => (x - l.gamew / 2, y - l.gameh * 2 / 3),
            }
        };
        let before: Vec<(i32, i32)> = s.texts.slots[..6].iter().map(|t| rel(t, &a)).collect();
        s.texts.relayout(&a, &b);
        let after: Vec<(i32, i32)> = s.texts.slots[..6].iter().map(|t| rel(t, &b)).collect();
        assert_eq!(before, after);
        let anchors: Vec<Anchor> = s.texts.slots[..6].iter().map(|t| t.anchor).collect();
        assert_eq!(anchors, [Anchor::Band, Anchor::Bottom, Anchor::GameRight, Anchor::GameCentre, Anchor::GameThird, Anchor::Band]);
        // a centred alignment moves with the band
        assert_eq!(s.texts.slots[5].align, Align::Center((160 + b.hx) as f32));
        // and the previous positions move too (no interpolation across the change)
        assert!(s.texts.slots[..6].iter().all(|t| t.prev_x == t.x && t.prev_y == t.y));
    }
}
