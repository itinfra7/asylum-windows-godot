//! Every frontend text, in every language, laid out with the real string table and the
//! styled CJK glyphs, must stay inside the chat window (original 20..299 x 20..235).

use std::sync::{Arc, OnceLock};

use super::highscore::HsState;
use super::menus::{self, MenuCtx, MenuId};
use super::messages;
use super::state::{Core, Screen};
use crate::config::Config;
use crate::font::{self, GlyphSet};
use crate::i18n::{Lang, Strings};
use crate::render::text::{layout_line, start_pen};
use crate::render::MenuLine;
use crate::sim::{Align, TextRef};
use crate::storage::default_highscores;

struct Res {
    strings: Arc<Strings>,
    glyphs: GlyphSet,
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        let read = |p: &str| std::fs::read_to_string(root.join(p)).expect(p);
        let strings = Strings::parse_csv(&read("locale/strings.csv")).expect("csv");
        let ko = font::parse_bdf(&read("fonts/asylum-ko.bdf")).expect("ko bdf");
        let zh = font::parse_bdf(&read("fonts/asylum-zh_hans.bdf")).expect("zh bdf");
        let glyphs = font::build_glyph_set(&strings.required_chars(), &ko, &zh);
        Res { strings: Arc::new(strings), glyphs }
    })
}

fn resolve(t: &TextRef, lang: Lang, s: &Strings) -> String {
    match t {
        TextRef::Empty => String::new(),
        TextRef::Key(k) => s.get(lang, k).to_string(),
        TextRef::Raw(r) => r.clone(),
    }
}

/// Horizontal box `[x0, x1)` of every rendered line (content coordinates) and its y.
fn boxes(text: &TextRef, x: f32, y: f32, align: Align, prefix: bool, lang: Lang) -> Vec<(f32, f32, f32, String)> {
    let r = res();
    let s = resolve(text, lang, &r.strings);
    let en = resolve(text, Lang::En, &r.strings);
    let pre = if prefix { font::original_advance(font::original_glyph('✓').unwrap()) } else { 0 };
    let en_lines: Vec<&str> = en.split('\n').collect();
    s.split('\n')
        .enumerate()
        .map(|(i, line)| {
            let adv = layout_line(line, lang, &r.glyphs).advance + if i == 0 { pre } else { 0 };
            let en_line = en_lines.get(i).copied().unwrap_or(en_lines[0]);
            let en_adv = layout_line(en_line, Lang::En, &r.glyphs).advance + if i == 0 { pre } else { 0 };
            let pen = start_pen(align, x, lang, adv, en_adv);
            (pen - 7.0, pen - 7.0 + adv as f32, y + 24.0 * i as f32, line.to_string())
        })
        .collect()
}

fn check(lines: &[MenuLine], lang: Lang, what: &str, bad: &mut Vec<String>) {
    for l in lines {
        for (x0, x1, y, t) in boxes(&l.text, l.x, l.y, l.align, l.prefix.is_some(), lang) {
            if x0 < 20.0 || x1 > 300.0 || y - 7.0 < 20.0 || y + 7.0 > 235.0 {
                bad.push(format!("{lang:?} {what}: {t:?} spans x {x0}..{x1} at y {y}"));
            }
        }
    }
}

/// A large monitor: every resolution and scale of the lists.
const MONITOR: (i32, i32) = (7680, 4320);

/// Every Tune Video row, for every display mode, aspect, resolution and scale, in every
/// language: inside the chat window, and the labels never touch their right-aligned values.
#[test]
fn video_settings_fit_in_every_language() {
    use crate::display;
    let r = res();
    let mut bad = Vec::new();
    for lang in Lang::ALL {
        for mode in 0..=2u8 {
            for aspect in 0..display::ASPECTS.len() as u8 {
                for &res in display::RESOLUTIONS[aspect as usize] {
                    for scale in display::SCALE_MIN..=display::SCALE_MAX {
                        let cfg = Config { display_mode: mode, window_aspect: aspect, window_resolution: res, screen_scale: scale, ..Config::default() };
                        let c = MenuCtx { strings: &r.strings, lang, config: &cfg, hx: 0, cy: 0, monitor: MONITOR, gameon: true, save_exists: true, saved_flash: false };
                        let e = menus::entries(MenuId::Video, &c);
                        let lines: Vec<MenuLine> = e.iter().map(|x| x.line.clone()).collect();
                        check(&lines, lang, "Video", &mut bad);
                        // label | value pairs share a row
                        for w in e.windows(2) {
                            let (a, b) = (&w[0].line, &w[1].line);
                            if a.y == b.y && matches!(b.align, Align::Right(_)) {
                                let la = boxes(&a.text, a.x, a.y, a.align, a.prefix.is_some(), lang)[0].1;
                                let lb = boxes(&b.text, b.x, b.y, b.align, false, lang)[0].0;
                                if la + 8.0 > lb {
                                    bad.push(format!("{lang:?} {:?} touches {:?} ({la} vs {lb})", resolve(&a.text, lang, &r.strings), resolve(&b.text, lang, &r.strings)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    bad.dedup();
    assert!(bad.is_empty(), "video menu problems:
{}", bad.join("
"));
}

fn core(lang: Lang) -> Core {
    let cfg = Config { language: Some(lang), id_permit: true, ..Config::default() };
    Core::new(crate::assets::testdata::assets(), res().strings.clone(), None, cfg, (320.0, 256.0), 1)
}

#[test]
fn menus_fit_in_every_language() {
    let r = res();
    let mut bad = Vec::new();
    let ids = [
        MenuId::Options,
        MenuId::Controls,
        MenuId::PadInfo,
        MenuId::Tune,
        MenuId::Sound,
        MenuId::Volume,
        MenuId::Video,
        MenuId::Language,
        MenuId::Zone,
        MenuId::IdLock,
        MenuId::Quit,
    ];
    for lang in Lang::ALL {
        let cfg = Config { sfx_volume: 10, music_volume: 10, ..Config::default() };
        for gameon in [false, true] {
            let c = MenuCtx { strings: &r.strings, lang, config: &cfg, hx: 0, cy: 0, monitor: MONITOR, gameon, save_exists: true, saved_flash: true };
            for id in ids {
                let lines: Vec<MenuLine> = menus::entries(id, &c).into_iter().map(|e| e.line).collect();
                check(&lines, lang, &format!("{id:?}"), &mut bad);
            }
        }
    }
    assert!(bad.is_empty(), "lines outside the chat window:\n{}", bad.join("\n"));
}

#[test]
fn screens_fit_in_every_language() {
    let mut bad = Vec::new();
    for lang in Lang::ALL {
        let mut c = core(lang);
        let mut lines = Vec::new();
        c.scene(&mut lines);
        check(&lines, lang, "loading", &mut bad);
        for step in 0..5 {
            c.screen = Screen::DefineKeys { step, keys: c.config.keys };
            c.scene(&mut lines);
            check(&lines, lang, "define keys", &mut bad);
        }
        let mut hs = HsState::new(default_highscores(), &[9; 8], *b"PSY", 1, true);
        c.screen = Screen::HighScore(Box::new(hs.clone()));
        c.scene(&mut lines);
        check(&lines, lang, "high scores", &mut bad);
        hs.phase = super::highscore::HsPhase::Done;
        c.screen = Screen::HighScore(Box::new(hs));
        c.scene(&mut lines);
        check(&lines, lang, "high scores", &mut bad);
        // escape menu texts at their final positions
        messages::pause(&mut c.sim);
        for _ in 0..messages::PAUSE_ANIM {
            c.sim.texts.animate(1);
        }
        let as_lines = |c: &Core| -> Vec<MenuLine> {
            c.sim
                .texts
                .visible()
                .map(|s| MenuLine { text: s.text.clone(), x: (s.x >> 8) as f32, y: (s.y >> 8) as f32, align: s.align, prefix: None })
                .collect()
        };
        check(&as_lines(&c), lang, "escape menu", &mut bad);
        // prelude lines as they pass through the middle of the window (horizontal only)
        messages::prelude(&mut c.sim, &res().strings, lang);
        let mut l = as_lines(&c);
        for x in l.iter_mut() {
            x.y = 128.0;
        }
        check(&l, lang, "prelude", &mut bad);
    }
    assert!(bad.is_empty(), "lines outside the chat window:\n{}", bad.join("\n"));
}

#[test]
fn cjk_menu_texts_use_styled_glyphs() {
    // every character of the localised menus is either an original glyph or a styled one
    let r = res();
    let cfg = Config::default();
    for lang in [Lang::Ko, Lang::ZhCn] {
        let c = MenuCtx { strings: &r.strings, lang, config: &cfg, hx: 0, cy: 0, monitor: MONITOR, gameon: false, save_exists: true, saved_flash: false };
        for id in [MenuId::Options, MenuId::Tune, MenuId::Language, MenuId::Zone] {
            for e in menus::entries(id, &c) {
                let s = resolve(&e.line.text, lang, &r.strings);
                for ch in s.chars().filter(|ch| *ch != ' ' && *ch != '\n') {
                    let ok = font::original_glyph(ch).is_some() || r.glyphs.find(lang.flavor(), ch).is_some();
                    assert!(ok, "{lang:?} {id:?}: no glyph for {ch:?} in {s:?}");
                }
            }
        }
    }
}
