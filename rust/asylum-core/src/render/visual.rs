//! Integration checks of the renderer with the real resources: the shipped string table,
//! the CJK BDF subsets re-styled by `font`, and the real atlas.

use std::sync::{Arc, OnceLock};

use crate::assets::GameAssets;
use crate::atlas::Atlas;
use crate::font;
use crate::i18n::{keys, Lang, Strings};
use crate::layout::Layout;
use crate::render::testkit::{raster, write_ppm};
use crate::render::text::{layout_line, GlyphRef};
use crate::render::{build_frame, finalize, Background, MenuLine, RenderCtx, RenderFrame, SceneDesc};
use crate::sim::{Align, TextRef, TextTable};

struct Real {
    assets: Arc<GameAssets>,
    ctx: RenderCtx,
}

fn real() -> &'static Real {
    static R: OnceLock<Real> = OnceLock::new();
    R.get_or_init(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
        let strings = Strings::parse_csv(&read("locale/strings.csv")).expect("strings.csv");
        let ko = font::parse_bdf(&read("fonts/asylum-ko.bdf")).expect("ko bdf");
        let zh = font::parse_bdf(&read("fonts/asylum-zh_hans.bdf")).expect("zh bdf");
        let glyphs = font::build_glyph_set(&strings.required_chars(), &ko, &zh);
        let assets = crate::assets::testdata::assets();
        let atlas = Atlas::build(&assets, &glyphs);
        Real { assets, ctx: RenderCtx { atlas: Arc::new(atlas), strings: Arc::new(strings), glyphs: Arc::new(glyphs) } }
    })
}

/// Every UI string of every language lays out without the '?' fallback, and every styled
/// glyph it uses sits on the original glyphs' 14-px line (centre row 7).
#[test]
fn every_string_has_glyphs_on_the_original_line() {
    let r = real();
    let (strings, glyphs, atlas) = (&r.ctx.strings, &r.ctx.glyphs, &r.ctx.atlas);
    let orig = atlas.sprite(crate::atlas::Bank::Chars, 17).copied().unwrap(); // 'A'
    let mut styled = 0;
    for key in keys::all() {
        for lang in Lang::ALL {
            for line in strings.get(lang, key).split('\n') {
                let l = layout_line(line, lang, glyphs);
                let question_marks = line.chars().filter(|&c| c == '?').count();
                let fallbacks = l.glyphs.iter().filter(|g| g.0 == GlyphRef::Orig(12)).count();
                assert_eq!(fallbacks, question_marks, "{key} {lang:?}: {line:?} has characters without glyphs");
                for &(g, _) in &l.glyphs {
                    if let GlyphRef::Styled(id) = g {
                        styled += 1;
                        let s = glyphs.glyphs[id as usize].clone();
                        if s.w == 0 {
                            continue;
                        }
                        let a = atlas.glyph(id).unwrap();
                        // the glyph box lies within the original 14-px line around the pen
                        assert!(a.yc <= orig.yc && a.h as i16 - a.yc <= orig.h as i16 - orig.yc, "{key} {lang:?} {:?}: yc {} h {}", s.ch, a.yc, a.h);
                    }
                }
            }
        }
    }
    assert!(styled > 100);
}

/// Korean / Chinese menu lines at the original positions stay inside the chat window.
#[test]
fn translated_menu_lines_fit_the_chat_window() {
    let r = real();
    let lay = Layout::original();
    let [x0, _, x1, _] = lay.chat_rect();
    // (key, original x) of the main options menu (menus.c options_menu)
    let lines = [
        (keys::MENU_OPTIONS, 128.0),
        (keys::MENU_DEFINE_CONTROLS, 32.0),
        (keys::MENU_TUNE_GAME, 32.0),
        (keys::MENU_CHOOSE_ZONE, 32.0),
        (keys::MENU_FIRE_PLAY, 88.0),
    ];
    for lang in Lang::ALL {
        for (key, x) in lines {
            let text = r.ctx.strings.get(lang, key);
            let en = r.ctx.strings.get(Lang::En, key);
            let l = layout_line(text, lang, &r.ctx.glyphs);
            let e = layout_line(en, Lang::En, &r.ctx.glyphs);
            let pen = crate::render::text::start_pen(Align::EnCentered, x, lang, l.advance, e.advance);
            let (b0, b1) = (pen - 7.0, pen - 7.0 + l.advance as f32);
            assert!(b0 >= x0 && b1 <= x1, "{key} {lang:?} box [{b0}, {b1}) outside [{x0}, {x1})");
        }
    }
}

fn render(lay: Layout, lang: Lang, bg: Background, texts: &TextTable, lines: &[MenuLine], highlight: Option<[f32; 4]>) -> Vec<[u8; 4]> {
    let r = real();
    let sc = SceneDesc { lay, lang, background: bg, sim: None, texts, lines, highlight, snap: true, no_interp: false };
    let mut f = RenderFrame::default();
    build_frame(&sc, &r.ctx, &mut f);
    let mut buf = Vec::new();
    finalize(&f, 1.0, &mut buf);
    raster(&r.ctx.atlas.rgba, r.ctx.atlas.width, r.ctx.atlas.height, &buf, lay.view_w as usize, lay.view_h as usize)
}

/// Styled glyphs use exactly the two colours of the original letters.
#[test]
fn styled_text_uses_original_letter_colours() {
    let lay = Layout::original();
    let line = |key| MenuLine { text: TextRef::Key(key), x: 32.0, y: 96.0, align: Align::EnCentered, prefix: None };
    for lang in [Lang::Ko, Lang::ZhCn] {
        let img = render(lay, lang, Background::Black, &TextTable::default(), &[line(keys::MENU_DEFINE_CONTROLS)], None);
        let green = crate::palette::rgba(crate::palette::FONT_GREEN);
        let dark = crate::palette::rgba(crate::palette::FONT_GREEN_DARK);
        let mut n = 0;
        for p in &img {
            if *p != [0, 0, 0, 255] {
                assert!(*p == green || *p == dark, "{lang:?}: unexpected text colour {p:?}");
                n += 1;
            }
        }
        assert!(n > 100, "{lang:?}: text drawn");
    }
}

/// `ASYLUM_RENDER_DUMP=<dir> cargo test -p asylum-core render::visual -- --ignored`
#[test]
#[ignore]
fn dump_menus() {
    let Ok(dir) = std::env::var("ASYLUM_RENDER_DUMP") else { return };
    let _ = &real().assets;
    let menu = |gameon: bool| -> Vec<MenuLine> {
        let l = |key, x, y| MenuLine { text: TextRef::Key(key), x, y, align: Align::EnCentered, prefix: None };
        vec![
            l(keys::MENU_OPTIONS, 128.0, 48.0),
            l(keys::MENU_DEFINE_CONTROLS, 32.0, 96.0),
            l(keys::MENU_TUNE_GAME, 32.0, 128.0),
            l(if gameon { keys::MENU_SAVE_POSITION } else { keys::MENU_CHOOSE_ZONE }, 32.0, 160.0),
            l(keys::MENU_FIRE_PLAY, 88.0, 224.0),
            MenuLine { text: TextRef::Key(keys::VIDEO_MODE_FULLSCREEN), x: 32.0, y: 192.0, align: Align::EnCentered, prefix: Some('✓') },
        ]
    };
    let mut story = TextTable::default();
    story.slots[0].count = 1;
    story.slots[0].x = 40 << 8;
    story.slots[0].y = 40 << 8;
    story.slots[0].prev_x = 40 << 8;
    story.slots[0].prev_y = 40 << 8;
    story.slots[0].text = TextRef::Key(keys::PRELUDE_STORY);
    story.slots[0].align = Align::Center(160.0);
    for lang in Lang::ALL {
        for vw in [320.0, 454.0] {
            let lay = Layout::new(vw, 256.0);
            let mut lines = menu(false);
            for l in lines.iter_mut() {
                l.x += lay.hx as f32;
            }
            let img = render(lay, lang, Background::Chat, &TextTable::default(), &lines, Some([24.0 + lay.hx as f32, 88.0, 296.0 + lay.hx as f32, 104.0]));
            write_ppm(&std::path::Path::new(&dir).join(format!("menu_{}_{}.ppm", lang.code(), lay.cw)), &img, vw as usize, 256);
        }
        let img = render(Layout::original(), lang, Background::Chat, &story, &[], None);
        write_ppm(&std::path::Path::new(&dir).join(format!("story_{}.ppm", lang.code())), &img, 320, 256);
    }
}

/// A real game (Ego, then Psyche) after some ticks of running right and firing; also checks
/// that consecutive frames interpolate consistently (no delta above SNAP_PX survives, and the
/// interpolated position at alpha 0 equals the previous frame's position for static blocks).
#[test]
fn real_game_frames_build_and_interpolate() {
    use crate::sim::game::GameStart;
    use crate::sim::{Sim, TickInput};
    let r = real();
    for (zone, vw) in [(1u8, 320.0), (2, 454.0)] {
        let lay = Layout::new(vw, 256.0);
        let mut sim = Sim::new(r.assets.clone(), lay, 99);
        sim.start_game(GameStart { mentalzone: zone, ..GameStart::default() }).unwrap();
        let mut f = RenderFrame::default();
        let mut prev_world: Vec<(f32, f32, f32, f32)> = Vec::new();
        for t in 0..300 {
            sim.begin_tick();
            let held = if t % 50 < 30 { crate::input::HELD_RIGHT | crate::input::HELD_FIRE } else { crate::input::HELD_LEFT };
            sim.tick(TickInput { held, cheat_keys: 0 });
            let texts = sim.texts.clone();
            let sc = SceneDesc { lay, lang: Lang::Ko, background: Background::Game, sim: Some(&sim), texts: &texts, lines: &[], highlight: None, snap: false, no_interp: false };
            build_frame(&sc, &r.ctx, &mut f);
            assert!(f.quads.iter().all(|q| q.dx.abs() <= crate::render::SNAP_PX && q.dy.abs() <= crate::render::SNAP_PX));
            // foreground blocks: (u, v, x, y) at alpha 0 must match last frame's (u, v, x, y)
            let world: Vec<(f32, f32, f32, f32)> =
                f.quads.iter().filter(|q| q.clip == crate::render::CLIP_GAME && q.w == 16.0).map(|q| (q.u, q.v, q.x + q.dx, q.y + q.dy)).collect();
            let cur: Vec<(f32, f32, f32, f32)> = f.quads.iter().filter(|q| q.clip == crate::render::CLIP_GAME && q.w == 16.0).map(|q| (q.u, q.v, q.x, q.y)).collect();
            if !prev_world.is_empty() && sim.draw.cam != sim.cam_prev {
                let matched = world.iter().filter(|w| prev_world.contains(w)).count();
                assert!(matched * 10 >= world.len() * 7, "tick {t}: {matched}/{} blocks continue from the previous frame", world.len());
            }
            prev_world = cur;
            let mut buf = Vec::new();
            let n = finalize(&f, 0.5, &mut buf);
            assert!(n > 50, "tick {t}: {n} instances");
        }
    }
}

/// `ASYLUM_RENDER_DUMP=<dir> cargo test -p asylum-core render::visual -- --ignored`
#[test]
#[ignore]
fn dump_game() {
    use crate::sim::game::GameStart;
    use crate::sim::{Sim, TickInput};
    let Ok(dir) = std::env::var("ASYLUM_RENDER_DUMP") else { return };
    let r = real();
    for (zone, vw) in [(1u8, 320.0), (2, 454.0), (3, 640.0)] {
        let lay = Layout::new(vw, 256.0);
        let mut sim = Sim::new(r.assets.clone(), lay, 7);
        sim.start_game(GameStart { mentalzone: zone, ..GameStart::default() }).unwrap();
        for t in 0..140 {
            sim.begin_tick();
            let held = if t < 100 { crate::input::HELD_RIGHT | crate::input::HELD_FIRE } else { crate::input::HELD_FIRE };
            sim.tick(TickInput { held, cheat_keys: 0 });
        }
        let texts = sim.texts.clone();
        for lang in [Lang::En, Lang::Ko] {
            let sc = SceneDesc { lay, lang, background: Background::Game, sim: Some(&sim), texts: &texts, lines: &[], highlight: None, snap: true, no_interp: false };
            let mut f = RenderFrame::default();
            build_frame(&sc, &r.ctx, &mut f);
            let mut buf = Vec::new();
            finalize(&f, 1.0, &mut buf);
            let img = raster(&r.ctx.atlas.rgba, r.ctx.atlas.width, r.ctx.atlas.height, &buf, vw as usize, 256);
            write_ppm(&std::path::Path::new(&dir).join(format!("game_{zone}_{}_{}.ppm", lay.cw, lang.code())), &img, vw as usize, 256);
        }
    }
}

/// "简体中文" is drawn with the Simplified Chinese glyph forms in every UI language (文 has a
/// different Korean form), "한국어" with the Korean ones.
#[test]
fn language_names_use_their_own_fonts() {
    use crate::font::Flavor;
    let r = real();
    let (g, atlas) = (&r.ctx.glyphs, &r.ctx.atlas);
    let (zh, ko) = (g.find(Flavor::ZhHans, '文').unwrap(), g.find(Flavor::Ko, '文').unwrap());
    assert_ne!(zh, ko, "the fonts differ for 文");
    let want = atlas.glyph(zh).unwrap();
    for lang in Lang::ALL {
        let lines = [MenuLine { text: TextRef::Key(keys::LANG_ZH), x: 100.0, y: 100.0, align: Align::Orig, prefix: None }];
        let sc = SceneDesc { lay: Layout::original(), lang, background: Background::Black, sim: None, texts: &TextTable::default(), lines: &lines, highlight: None, snap: false, no_interp: false };
        let mut f = RenderFrame::default();
        build_frame(&sc, &r.ctx, &mut f);
        assert_eq!(f.quads.len(), 4);
        assert_eq!((f.quads[3].u, f.quads[3].v), (want.u as f32, want.v as f32), "{lang:?}");
    }
}
