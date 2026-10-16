//! Fuzz of the whole App (state machine + rendering) with random input: keys (also auto-repeat),
//! gamepad, mouse, monitor changes and view changes, restarting after every quit. Every frame
//! must build and finalise to finite instance data without panicking.
use std::path::PathBuf;
use asylum_core::assets::RawFiles;
use asylum_core::config::Config;
use asylum_core::frontend::{App, AppInit, AppOutputs};
use asylum_core::i18n::Lang;
use asylum_core::input::{key, pad, InputEvent, HELD_ALT};
use asylum_core::render::{finalize, RenderCtx, RenderFrame, FLOATS_PER_INSTANCE};
use asylum_core::runtime::{load, LoadInput};
use asylum_core::storage::Storage;

struct X(u64);
impl X { fn n(&mut self) -> u32 { let mut x = self.0; x ^= x << 13; x ^= x >> 7; x ^= x << 17; self.0 = x; (x >> 16) as u32 } }

#[test]
fn app_fuzz() {
    let ticks_per_seed: u32 = std::env::var("ASYLUM_FUZZ_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(12_500);
    let r = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ud = std::env::temp_dir().join(format!("asylum-fuzz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ud);
    std::fs::create_dir_all(&ud).unwrap();
    let l = load(LoadInput {
        files: RawFiles::from_dir(&r.join("game/data")).unwrap(),
        strings_csv: std::fs::read_to_string(r.join("game/locale/strings.csv")).unwrap(),
        bdf_ko: std::fs::read_to_string(r.join("game/fonts/asylum-ko.bdf")).unwrap(),
        bdf_zh: std::fs::read_to_string(r.join("game/fonts/asylum-zh_hans.bdf")).unwrap(),
        userdir: ud.clone(),
        os_locale: "ko_KR".into(),
    }).unwrap();
    let keys: Vec<i64> = vec![key::SPACE, key::ENTER, key::ESCAPE, key::UP, key::DOWN, key::LEFT, key::RIGHT, key::BACKSPACE,
        key::KEY_0, key::KEY_0 + 1, key::KEY_0 + 2, key::KEY_0 + 3, key::KEY_0 + 4, key::KEY_0 + 5, key::KEY_0 + 6, key::KEY_0 + 7,
        key::A + ('o' as i64 - 'a' as i64), key::A + ('q' as i64 - 'a' as i64), key::A + ('r' as i64 - 'a' as i64),
        key::A + ('z' as i64 - 'a' as i64), key::A + ('x' as i64 - 'a' as i64), key::F1 + 10, key::KP_0 + 3, 59, 46];
    let pads = [pad::A, pad::B, pad::X, pad::START, pad::BACK, pad::DPAD_UP, pad::DPAD_DOWN, pad::DPAD_LEFT, pad::DPAD_RIGHT];
    let views = [(320.0, 256.0), (455.0, 256.0), (607.0, 256.0), (683.0, 512.0), (1820.0, 768.0), (2731.0, 768.0), (512.0, 1700.0), (1000.0, 300.0)];
    let monitors = [(2560, 1080), (1920, 1080), (1280, 720), (1024, 768), (800, 600), (3840, 2160), (1080, 1920)];
    let mut frame = RenderFrame::default();
    let mut out = AppOutputs::default();
    let mut buf = Vec::new();
    let mut quits = 0;
    let mut ticks = 0u64;
    for seed in 1..=8u64 {
        let lang = [Lang::En, Lang::Ko, Lang::ZhCn][(seed % 3) as usize];
        let mut app = App::new(AppInit {
            assets: l.assets.clone(),
            ctx: RenderCtx { atlas: l.atlas.clone(), strings: l.strings.clone(), glyphs: l.glyphs.clone() },
            storage: Storage::new(ud.clone()),
            config: Config { language: Some(lang), ..Config::default() },
            view: views[seed as usize % views.len()],
            seed: seed as u32,
        });
        app.set_sfx_ready();
        let mut rng = X(seed * 7919 + 1);
        let mut held = 0u32;
        for t in 0..ticks_per_seed {
            let v = rng.n();
            if v.is_multiple_of(7) {
                let ev = match rng.n() % 100 {
                    0..=59 => { let k = keys[(rng.n() as usize) % keys.len()]; InputEvent::Key { code: k, ch: None, echo: rng.n().is_multiple_of(5) } }
                    60..=74 => InputEvent::Pad { button: pads[(rng.n() as usize) % pads.len()] },
                    75..=84 => InputEvent::MouseMove { x: (rng.n() % 2000) as f32, y: (rng.n() % 1200) as f32 },
                    85..=94 => InputEvent::MouseDown { x: (rng.n() % 2000) as f32, y: (rng.n() % 1200) as f32, button: (1 + rng.n() % 5) as u8 },
                    95..=97 => { let m = monitors[(rng.n() as usize) % monitors.len()]; InputEvent::Monitor { width: m.0, height: m.1 } }
                    _ => InputEvent::Key { code: key::ENTER, ch: None, echo: false },
                };
                // ESC + Enter on the title menu quits; keep the run alive most of the time
                
                app.push_event(ev);
            }
            if v.is_multiple_of(997) {
                let (w, h) = views[(rng.n() as usize) % views.len()];
                app.set_view(w, h);
            }
            if v.is_multiple_of(13) {
                held = rng.n() & 0x3f;
                if rng.n().is_multiple_of(50) {
                    held |= HELD_ALT;
                }
            }
            app.tick(held);
            app.render(&mut frame);
            let n = finalize(&frame, (t % 3) as f32 * 0.5, &mut buf);
            for x in &buf[..n * FLOATS_PER_INSTANCE] { assert!(x.is_finite(), "seed {seed} tick {t}: NaN/inf"); }
            app.drain(&mut out);
            out.sfx.clear();
            out.requests.clear();
            ticks += 1;
            if app.wants_quit() {
                quits += 1;
                app = App::new(AppInit {
            assets: l.assets.clone(),
            ctx: RenderCtx { atlas: l.atlas.clone(), strings: l.strings.clone(), glyphs: l.glyphs.clone() },
            storage: Storage::new(ud.clone()),
            config: Config { language: Some(lang), ..Config::default() },
            view: views[seed as usize % views.len()],
            seed: seed as u32,
        });
                app.set_sfx_ready();
            }
        }
    }
    let _ = std::fs::remove_dir_all(&ud);
    println!("ticks {ticks}, quits {quits}");
}
