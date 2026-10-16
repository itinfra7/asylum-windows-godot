//! Headless end-to-end test of the whole core: start-up loading (as the Godot host does it),
//! the App state machine driven by synthetic input events, frame building and finalisation,
//! audio outputs, in all three languages and several viewport shapes.

use std::path::PathBuf;

use asylum_core::assets::RawFiles;
use asylum_core::config::Config;
use asylum_core::frontend::{App, AppInit, AppOutputs};
use asylum_core::i18n::Lang;
use asylum_core::input::{key, InputEvent, HELD_FIRE, HELD_RIGHT};
use asylum_core::render::{finalize, RenderCtx, RenderFrame, FLOATS_PER_INSTANCE};
use asylum_core::runtime::{load, LoadInput, Loaded};
use asylum_core::storage::Storage;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn temp_userdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("asylum-e2e-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn load_all(tag: &str) -> Loaded {
    let r = root();
    let input = LoadInput {
        files: RawFiles::from_dir(&r.join("game/data")).unwrap(),
        strings_csv: std::fs::read_to_string(r.join("game/locale/strings.csv")).unwrap(),
        bdf_ko: std::fs::read_to_string(r.join("game/fonts/asylum-ko.bdf")).unwrap(),
        bdf_zh: std::fs::read_to_string(r.join("game/fonts/asylum-zh_hans.bdf")).unwrap(),
        userdir: temp_userdir(tag),
        os_locale: "en_US".into(),
    };
    load(input).expect("load")
}

fn new_app(l: &Loaded, lang: Lang, view: (f32, f32)) -> App {
    let mut config = Config { language: Some(lang), ..Config::default() };
    config.sound_mode = 2;
    App::new(AppInit {
        assets: l.assets.clone(),
        ctx: RenderCtx { atlas: l.atlas.clone(), strings: l.strings.clone(), glyphs: l.glyphs.clone() },
        storage: Storage::new(l.userdir.clone()),
        config,
        view,
        seed: 1234,
    })
}

fn key_down(code: i64, ch: Option<char>) -> InputEvent {
    InputEvent::Key { code, ch, echo: false }
}

/// Ticks the app and checks every frame can be built and finalised.
fn pump(app: &mut App, ticks: u32, held: u32, frame: &mut RenderFrame, out: &mut AppOutputs, buf: &mut Vec<f32>) -> usize {
    let mut max_inst = 0;
    for i in 0..ticks {
        app.tick(held);
        app.render(frame);
        for alpha in [0.0f32, 0.5, 1.0] {
            let n = finalize(frame, alpha, buf);
            assert!(buf.len() >= n * FLOATS_PER_INSTANCE);
            for v in &buf[..n * FLOATS_PER_INSTANCE] {
                assert!(v.is_finite(), "NaN/inf in instance data at tick {i}");
            }
            max_inst = max_inst.max(n);
        }
        app.drain(out);
    }
    max_inst
}

#[test]
fn full_flow_all_languages_and_shapes() {
    let l = load_all("flow");
    assert!(l.atlas.width >= 512 && l.atlas.height >= 256);
    for (lang, view) in [(Lang::En, (320.0, 256.0)), (Lang::Ko, (455.0, 256.0)), (Lang::ZhCn, (1820.0, 768.0)), (Lang::En, (853.0, 640.0))] {
        let mut app = new_app(&l, lang, view);
        let mut frame = RenderFrame::default();
        let mut out = AppOutputs::default();
        let mut buf = Vec::new();
        app.set_sfx_ready();
        // loading + prelude
        let n = pump(&mut app, 120, 0, &mut frame, &mut out, &mut buf);
        assert!(n > 10, "prelude draws the chat screen and texts");
        assert!(out.requests.iter().any(|r| matches!(r, asylum_core::frontend::HostRequest::ApplyVideo(_))));
        // any key → main menu
        app.push_event(key_down(key::SPACE, Some(' ')));
        pump(&mut app, 10, 0, &mut frame, &mut out, &mut buf);
        // walk through the tune menus and back
        for code in [key::KEY_0 + 2, key::KEY_0 + 3, key::ESCAPE, key::KEY_0 + 4, key::DOWN, key::ESCAPE, key::ESCAPE] {
            app.push_event(key_down(code, None));
            pump(&mut app, 5, 0, &mut frame, &mut out, &mut buf);
        }
        // play (Fire / Enter on the main menu)
        app.push_event(key_down(key::ENTER, None));
        let n = pump(&mut app, 400, HELD_RIGHT | HELD_FIRE, &mut frame, &mut out, &mut buf);
        assert!(n > 100, "game frame draws world + sprites ({n})");
        // pause and resume
        app.push_event(key_down(key::ESCAPE, None));
        pump(&mut app, 70, 0, &mut frame, &mut out, &mut buf);
        app.push_event(key_down(key::A + (b'R' - b'A') as i64, Some('r')));
        pump(&mut app, 200, HELD_RIGHT, &mut frame, &mut out, &mut buf);
        // pause and abandon
        app.push_event(key_down(key::ESCAPE, None));
        pump(&mut app, 70, 0, &mut frame, &mut out, &mut buf);
        app.push_event(key_down(key::ESCAPE, None));
        pump(&mut app, 20, 0, &mut frame, &mut out, &mut buf);
        assert!(!app.wants_quit());
        assert_eq!(app.lang(), lang);
    }
    let _ = std::fs::remove_dir_all(&l.userdir);
}

#[test]
fn language_change_is_instant_and_persisted() {
    let l = load_all("lang");
    let mut app = new_app(&l, Lang::En, (455.0, 256.0));
    let mut frame = RenderFrame::default();
    let mut out = AppOutputs::default();
    let mut buf = Vec::new();
    app.set_sfx_ready();
    pump(&mut app, 30, 0, &mut frame, &mut out, &mut buf);
    app.push_event(key_down(key::SPACE, Some(' ')));
    pump(&mut app, 5, 0, &mut frame, &mut out, &mut buf);
    // Options → 2 Tune Game → 4 Language → 2 Korean
    for code in [key::KEY_0 + 2, key::KEY_0 + 4, key::KEY_0 + 2] {
        app.push_event(key_down(code, None));
        pump(&mut app, 3, 0, &mut frame, &mut out, &mut buf);
    }
    assert_eq!(app.lang(), Lang::Ko);
    let saved = Storage::new(l.userdir.clone()).load_config();
    assert_eq!(saved.language, Some(Lang::Ko));
    let _ = std::fs::remove_dir_all(&l.userdir);
}
