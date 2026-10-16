//! Animated message screens that reuse the sim's text table like the C code: menus.c
//! `prelude()` and `escapehandler()`. Positions are 320x256-space; `Sim::message_hud` adds
//! `lay.hx`/`lay.cy` (centred), `message_hud_bottom` `lay.hx`/`lay.hy` (the caller sets
//! `sim.lay` first).

use crate::i18n::{keys, Lang, Strings};
use crate::sim::{Align, Sim, TextRef};

/// prelude(): `_x`, the start y of the scroll, and `_v`, its speed.
const PX: i32 = 250;
const PV: f32 = -1.0;
/// Original first-glyph x of the six English story lines.
const STORY_X: [i32; 6] = [40, 56, 56, 28, 36, 100];

/// Ticks the prelude scrolls (`scroll = 256+8`) and in cheat mode (`scroll = 1024`).
pub const PRELUDE_SCROLL: i32 = 256 + 8;
pub const PRELUDE_CHEAT_SCROLL: i32 = 1024;
/// escapehandler: `r9 = 64` animation frames.
pub const PAUSE_ANIM: i32 = 64;

/// menus.c `prelude()` messages (x > 2000 = permanent).
///
/// The scroll is anchored at the bottom of the screen and keeps the original speed, so the
/// bottom 256 lines show exactly the original scroll on taller screens (the cheat lines stay
/// below the screen unless the cheat scroll runs).
pub fn prelude(sim: &mut Sim, strings: &Strings, lang: Lang) {
    sim.texts.wipe();
    let pv = PV;
    let key = |sim: &mut Sim, x: i32, y: i32, k: &'static str| {
        sim.message_hud_bottom(2000 + x, y, 0.0, pv, TextRef::Key(k), Align::EnCentered);
    };
    key(sim, 48, PX, keys::PRELUDE_DP);
    key(sim, 108, PX + 24, keys::PRELUDE_PRESENTS);
    sim.message_hud_bottom(2152, PX + (24 * 5) / 2, 0.0, pv, TextRef::Raw("\u{E000}".into()), Align::Orig);
    let story: Vec<&str> = strings.get(lang, keys::PRELUDE_STORY).split('\n').map(str::trim_end).collect();
    let orig_x = lang == Lang::En && story.len() == STORY_X.len();
    for (i, line) in story.iter().enumerate() {
        let y = PX + (4 + i as i32) * 24;
        let text = TextRef::Raw(line.to_string());
        if orig_x {
            sim.message_hud_bottom(2000 + STORY_X[i], y, 0.0, pv, text, Align::Orig);
        } else {
            let cx = (160 + sim.lay.hx) as f32;
            sim.message_hud_bottom(2160, y, 0.0, pv, text, Align::Center(cx));
        }
    }
    key(sim, 96, PX + 10 * 24, keys::PRELUDE_PRESS);
    key(sim, 88, PX + 12 * 24, keys::PRELUDE_CHEAT);
    key(sim, 28, PX + 14 * 24, keys::PRELUDE_CHEAT_WEAPONS);
    key(sim, 28, PX + 15 * 24, keys::PRELUDE_CHEAT_STRENGTH);
    key(sim, 96, PX + 17 * 24, keys::PRELUDE_FUN);
}

/// Final y of the four action lines of the escape menu (ESC, Q, R, O), 320x256-space
/// (add `lay.cy`).
pub const PAUSE_ROWS: [f32; 4] = [104.0, 136.0, 168.0, 200.0];

/// menus.c `escapehandler()` messages sliding in from ±256 px at 4 px/tick (on a larger
/// screen from just as far outside its edges, proportionally faster, still landing after
/// `PAUSE_ANIM` ticks). The four action lines stay left-aligned in every language (the English
/// spacing aligns them at x=40).
pub fn pause(sim: &mut Sim) {
    sim.texts.wipe();
    let (h, v) = (256 + sim.lay.hx, 256 + sim.lay.cy);
    let (hv, vv) = (h as f32 / PAUSE_ANIM as f32, v as f32 / PAUSE_ANIM as f32);
    let m = |sim: &mut Sim, x: i32, y: i32, xv: f32, yv: f32, k: &'static str, a: Align| {
        sim.message_band(400, x, y, xv, yv, TextRef::Key(k), a);
    };
    m(sim, 36, 40 - v, 0.0, vv, keys::ESC_INTERRUPTED, Align::EnCentered);
    m(sim, 32 - h, 72, hv, 0.0, keys::ESC_SELECT, Align::EnCentered);
    m(sim, 46 + h, 104, -hv, 0.0, keys::ESC_ABANDON, Align::Orig);
    m(sim, 40 - h, 136, hv, 0.0, keys::ESC_LOSE_LIFE, Align::Orig);
    m(sim, 40, 168 + v, 0.0, -vv, keys::ESC_RETURN, Align::Orig);
    m(sim, 40, 200 + v, 0.0, -vv, keys::ESC_OPTIONS, Align::Orig);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;

    #[test]
    fn pause_messages_land_on_original_positions() {
        let mut sim = crate::sim::testutil::sim();
        pause(&mut sim);
        for _ in 0..PAUSE_ANIM {
            sim.texts.animate(1);
        }
        let pos: Vec<(i32, i32)> = sim.texts.visible().map(|s| (s.x >> 8, s.y >> 8)).collect();
        assert_eq!(pos, vec![(36, 40), (32, 72), (46, 104), (40, 136), (40, 168), (40, 200)]);
        // 400-tick lifetime minus the animation: still visible while the menu waits
        assert!(sim.texts.visible().all(|s| s.count == 400 - PAUSE_ANIM));
        // a larger screen keeps the menu around its centre
        sim.lay = Layout::new(1365.0, 768.0);
        pause(&mut sim);
        for _ in 0..PAUSE_ANIM {
            sim.texts.animate(1);
        }
        let pos: Vec<(i32, i32)> = sim.texts.visible().map(|s| (s.x >> 8, s.y >> 8)).collect();
        assert_eq!(pos[2], (46 + 522, 104 + 256));
    }

    #[test]
    fn prelude_is_permanent_and_scrolls_up() {
        let mut sim = crate::sim::testutil::sim();
        sim.lay = Layout::new(454.0, 256.0);
        let mut s = Strings::default();
        s.tables[0].insert(keys::PRELUDE_STORY.into(), "a\nb\nc\nd\ne\nf".into());
        s.tables[1].insert(keys::PRELUDE_STORY.into(), "가\n나\n다\n라".into());
        prelude(&mut sim, &s, Lang::En);
        let v: Vec<_> = sim.texts.visible().cloned().collect();
        assert_eq!(v.len(), 8 + 6);
        assert!(v.iter().all(|t| t.count == 0x10000 && t.dy == -256));
        assert_eq!((v[0].x >> 8, v[0].y >> 8), (48 + 67, 250));
        assert_eq!((v[3].x >> 8, v[3].y >> 8, v[3].align), (40 + 67, 250 + 96, Align::Orig));
        assert_eq!(v[8].x >> 8, 100 + 67);
        prelude(&mut sim, &s, Lang::Ko);
        let v: Vec<_> = sim.texts.visible().cloned().collect();
        assert_eq!(v.len(), 8 + 4);
        assert_eq!(v[3].align, Align::Center(227.0));
        assert_eq!(v[7].text, TextRef::Key(keys::PRELUDE_PRESS));
        // a 768-line screen: anchored at its bottom, original speed; when the normal scroll
        // ends the cheat lines are still below the screen, like on the original 256 lines
        sim.lay = Layout::new(1365.0, 768.0);
        prelude(&mut sim, &s, Lang::En);
        for _ in 0..PRELUDE_SCROLL {
            sim.texts.animate(1);
        }
        let v: Vec<_> = sim.texts.visible().cloned().collect();
        assert!(v.iter().all(|t| t.dy == -256));
        let cheat = v.iter().find(|t| t.text == TextRef::Key(keys::PRELUDE_CHEAT)).unwrap();
        assert!((cheat.y >> 8) - 7 >= sim.lay.ch, "cheat line at {}", cheat.y >> 8);
        assert_eq!(v[0].y >> 8, 250 + 512 - PRELUDE_SCROLL);
    }
}
