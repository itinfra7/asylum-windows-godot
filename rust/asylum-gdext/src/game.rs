//! `AsylumGame` — the main scene root. Loads everything at `ready`, starts the core runtime
//! threads, and each display frame: input → runtime, latest frame → MultiMesh, host events →
//! audio/window, music PCM → generator.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use asylum_core::assets::RawFiles;
use asylum_core::config::Config;
use asylum_core::frontend::HostRequest;
use asylum_core::input::{self as core_input, InputEvent as CoreEvent, ALT_KEYS, HELD_ALT};
use asylum_core::runtime::{self, HostEvent, LoadInput, Runtime, TICK};
use godot::classes::{
    DisplayServer, FileAccess, INode2D, Input, InputEvent, InputEventJoypadButton, InputEventKey,
    InputEventMouseButton, InputEventMouseMotion, Node2D, Os, ProjectSettings, Window,
};
use godot::global::{JoyAxis, JoyButton, Key};
use godot::obj::EngineEnum;
use godot::prelude::*;

use crate::audio::Audio;
use crate::gfx::Gfx;
use crate::video;

const STICK_DEADZONE: f32 = 0.5;
/// Joypad list refresh interval (frames); avoids a Godot array allocation every frame.
const PAD_REFRESH_FRAMES: u64 = 30;
const QUIT_DELAY_FRAMES: u32 = 6;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// Portable user-data directory (docs §9).
fn user_dir() -> PathBuf {
    if let Some(d) = env("ASYLUM_USERDIR") {
        return PathBuf::from(d);
    }
    let os = Os::singleton();
    let user = PathBuf::from(ProjectSettings::singleton().globalize_path("user://").to_string());
    if os.has_feature("editor") {
        return user;
    }
    let writable = |d: &PathBuf| {
        if std::fs::create_dir_all(d).is_err() {
            return false;
        }
        let probe = d.join(".write_test");
        let ok = std::fs::write(&probe, b"asylum").is_ok();
        let _ = std::fs::remove_file(&probe);
        ok
    };
    let exe = PathBuf::from(os.get_executable_path().to_string());
    if let Some(dir) = exe.parent() {
        let d = dir.join("userdata");
        if writable(&d) {
            return d;
        }
    }
    // Read-only install location (e.g. Program Files). The portable engine build maps user://
    // to the same read-only folder, so fall back to the roaming profile explicitly.
    if let Some(appdata) = env("APPDATA") {
        let d = PathBuf::from(appdata).join("Asylum");
        if writable(&d) {
            return d;
        }
    }
    user
}

fn read_bytes(path: &str) -> Option<Vec<u8>> {
    if !FileAccess::file_exists(path) {
        return None;
    }
    Some(FileAccess::get_file_as_bytes(path).to_vec())
}

fn read_text(path: &str) -> Result<String, String> {
    let b = read_bytes(path).ok_or_else(|| format!("missing file: {path}"))?;
    String::from_utf8(b).map_err(|_| format!("{path}: not valid UTF-8"))
}

fn gather_input() -> Result<LoadInput, String> {
    let mut files = RawFiles::default();
    for p in RawFiles::required_paths() {
        if let Some(b) = read_bytes(&format!("res://data/{p}")) {
            files.files.insert(p, b);
        }
    }
    let os_locale = env("ASYLUM_LOCALE").unwrap_or_else(|| Os::singleton().get_locale().to_string());
    Ok(LoadInput {
        files,
        strings_csv: read_text("res://locale/strings.csv")?,
        bdf_ko: read_text("res://fonts/asylum-ko.bdf")?,
        bdf_zh: read_text("res://fonts/asylum-zh_hans.bdf")?,
        userdir: user_dir(),
        os_locale,
    })
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

fn lower_char(u: u32) -> Option<char> {
    let c = char::from_u32(u).filter(|c| *c != '\0' && !c.is_control())?;
    Some(c.to_lowercase().next().unwrap_or(c))
}

/// Debug automation (see lib.rs): injected key presses and a screenshot, timed in 1/60 s
/// since loading finished.
#[derive(Default)]
struct Automation {
    /// Sorted by time.
    keys: Vec<(u64, i64)>,
    /// Keys already sent.
    sent: usize,
    shot: Option<(u64, String)>,
    start: Option<Instant>,
    /// ASYLUM_BOT: random held keys and key presses (long unattended runs).
    bot: Option<Bot>,
}

/// Random player for ASYLUM_BOT (xorshift; held keys change every 0.2..1.5 s, a menu or
/// gameplay key is pressed now and then).
struct Bot {
    rng: u64,
    held: u32,
    until: u64,
    next_key: u64,
}

impl Bot {
    fn next(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 16) as u32
    }
}

impl Automation {
    fn from_env() -> Automation {
        let keys = env("ASYLUM_KEYS")
            .map(|s| {
                s.split(',')
                    .filter_map(|it| {
                        let (f, k) = it.trim().split_once(':')?;
                        Some((f.trim().parse().ok()?, k.trim().parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut keys: Vec<(u64, i64)> = keys;
        keys.sort_by_key(|k| k.0);
        let shot = env("ASYLUM_SCREENSHOT")
            .map(|p| (env("ASYLUM_SCREENSHOT_FRAME").and_then(|f| f.parse().ok()).unwrap_or(180), p));
        let bot = env("ASYLUM_BOT").and_then(|s| s.parse::<u64>().ok()).map(|seed| Bot { rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, held: 0, until: 0, next_key: 0 });
        Automation { keys, sent: 0, shot, start: None, bot }
    }

    /// Time since loading finished, in 1/60 s.
    fn now(&mut self) -> u64 {
        (self.start.get_or_insert_with(Instant::now).elapsed().as_secs_f64() * 60.0) as u64
    }
}

/// Everything that exists once loading succeeded.
struct Host {
    rt: Runtime,
    gfx: Gfx,
    audio: Audio,
    window: Gd<Window>,
    video: video::Video,
    /// `override.cfg` next to the executable (portable install only) and its last text.
    start_cfg: Option<(PathBuf, String)>,
    events: Vec<HostEvent>,
    view: Vector2,
    pads: Vec<i32>,
    sfx_loaded: bool,
}

#[derive(GodotClass)]
#[class(base = Node2D)]
pub struct AsylumGame {
    base: Base<Node2D>,
    host: Option<Host>,
    frames: u64,
    auto: Automation,
    /// Frames left before `SceneTree::quit` once quitting started (lets the audio server
    /// retire the stopped playbacks first) and the exit code.
    quit_in: Option<(u32, i32)>,
    /// Global (screen) cursor position of the last forwarded mouse motion. Godot reports a
    /// motion whenever the cursor's window-relative position changes — also when the window
    /// itself appears or is moved under a resting cursor. Only real cursor movement may move
    /// the menu highlight, so motions are forwarded only when this changes.
    last_cursor: Option<Vector2i>,
}

#[godot_api]
impl INode2D for AsylumGame {
    fn init(base: Base<Node2D>) -> Self {
        AsylumGame { base, host: None, frames: 0, auto: Automation::default(), quit_in: None, last_cursor: None }
    }

    fn ready(&mut self) {
        self.base().get_tree().set_auto_accept_quit(false);
        self.auto = Automation::from_env();
        let t0 = Instant::now();
        let loaded = match gather_input().and_then(|input| {
            catch_unwind(AssertUnwindSafe(|| runtime::load(input))).unwrap_or_else(|p| Err(panic_text(p.as_ref())))
        }) {
            Ok(l) => l,
            Err(e) => {
                self.fatal(&format!("Asylum could not start:\n{e}"));
                return;
            }
        };
        godot_print!(
            "Asylum: loaded in {} ms (atlas {}x{}, {} styled glyphs, userdata {})",
            t0.elapsed().as_millis(),
            loaded.atlas.width,
            loaded.atlas.height,
            loaded.glyphs.glyphs.len(),
            loaded.userdir.display()
        );

        let Some(mut window) = self.base().get_window() else {
            self.fatal("Asylum: no window");
            return;
        };
        // A signal rather than `on_notification`: Godot sends some notifications (window focus)
        // synchronously while the window mode changes, i.e. while this node is borrowed.
        let _ = window.connect("close_requested", &Callable::from_object_method(&self.to_gd(), "on_close_requested"));
        let mut video = video::Video::new(!Os::singleton().has_feature("embedded_in_editor"), &window);
        video.apply(&mut window, &loaded.config);

        let gfx = match Gfx::new(self.base().get_canvas_item(), &loaded.atlas) {
            Ok(g) => g,
            Err(e) => {
                self.fatal(&format!("Asylum could not start:\n{e}"));
                return;
            }
        };
        let mut audio = Audio::new(|n| self.base_mut().add_child(&n));
        audio.apply(loaded.config.sound_mode, loaded.config.sfx_volume, loaded.config.music_volume);

        let view = self.visible_size();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let seed = (now.as_secs() as u32) ^ now.subsec_nanos();
        let rt = Runtime::start(&loaded, (view.x, view.y), seed);
        let (width, height) = video.monitor();
        rt.send(CoreEvent::Monitor { width, height });
        let start_cfg = start_cfg_path(&loaded.userdir).map(|p| {
            let old = std::fs::read_to_string(&p).unwrap_or_default();
            (p, old)
        });
        self.host = Some(Host {
            rt,
            gfx,
            audio,
            window,
            video,
            start_cfg,
            events: Vec::with_capacity(64),
            view,
            pads: Vec::new(),
            sfx_loaded: false,
        });
    }

    fn process(&mut self, _delta: f64) {
        if let Some((left, code)) = self.quit_in {
            if left == 0 {
                self.base().get_tree().quit_ex().exit_code(code).done();
            }
            self.quit_in = Some((left.saturating_sub(1), code));
            return;
        }
        if self.host.is_none() {
            return;
        }
        self.frames += 1;
        let frame_no = self.frames;
        self.automate();
        let Some(h) = self.host.as_mut() else { return };

        if let Some(msg) = h.rt.failure() {
            self.fatal(&format!("Asylum stopped because of an internal error:\n{msg}"));
            return;
        }

        // Host events (sound effects, settings, quit).
        h.events.clear();
        h.rt.poll_events(&mut h.events);
        let mut video_changed = false;
        for ev in &h.events {
            match ev {
                HostEvent::Sfx(e) => h.audio.play(e),
                HostEvent::Request(HostRequest::StopSfx) => h.audio.stop_all(),
                HostEvent::Request(HostRequest::ApplyAudio { sound_mode, sfx_volume, music_volume }) => {
                    h.audio.apply(*sound_mode, *sfx_volume, *music_volume)
                }
                HostEvent::Request(HostRequest::ApplyVideo(cfg)) => {
                    h.video.apply(&mut h.window, cfg);
                    video_changed = true;
                }
                HostEvent::Request(HostRequest::Quit) => {}
            }
        }
        // window moved to another monitor: settings re-applied there, lists updated
        if let Some((width, height)) = h.video.poll(&mut h.window) {
            h.rt.send(CoreEvent::Monitor { width, height });
            video_changed = true;
        }
        if video_changed {
            write_start_cfg(h);
        }
        // the view after this frame's video changes, so the core lays out for it at once; a
        // degenerate window (0-px client) is ignored
        let view = h.window.get_visible_rect().size;
        let view_ok = view.x.is_finite() && view.y.is_finite() && view.x >= 1.0 && view.y >= 1.0;
        if h.rt.quit_requested() {
            // a game-thread panic sets the failure, then quit: it may land after the check
            // above (the failure is visible once quit is)
            match h.rt.failure() {
                Some(msg) => self.fatal(&format!("Asylum stopped because of an internal error:\n{msg}")),
                None => self.quit(),
            }
            return;
        }

        // Held input.
        if frame_no % PAD_REFRESH_FRAMES == 1 {
            h.pads.clear();
            h.pads.extend(Input::singleton().get_connected_joypads().iter_shared().map(|d| d as i32));
        }
        let cfg = h.rt.config();
        h.rt.set_held(held_mask(&cfg, &h.pads) | self.auto.bot.as_ref().map_or(0, |b| b.held));

        if view_ok && view != h.view {
            h.view = view;
            h.rt.set_view(view.x, view.y);
        }
        let view = h.view;

        if !h.sfx_loaded {
            if let Some(bank) = h.rt.take_sfx_bank() {
                h.audio.set_bank(&bank);
                h.sfx_loaded = true;
            }
        }

        // Draw the latest tick, interpolated.
        if let Some(pf) = h.rt.latest_frame() {
            let alpha = if pf.frame.no_interp {
                1.0
            } else {
                (Instant::now().saturating_duration_since(pf.tick_time).as_secs_f32() / TICK.as_secs_f32()).clamp(0.0, 1.0)
            };
            let gfx = &mut h.gfx;
            if let Err(p) = catch_unwind(AssertUnwindSafe(|| gfx.draw(&pf, alpha, (view.x, view.y)))) {
                let msg = panic_text(p.as_ref());
                self.fatal(&format!("Asylum stopped because of an internal error:\nrender: {msg}"));
                return;
            }
        }

        h.audio.pump_music(&h.rt);
        self.screenshot();
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        let Some(h) = self.host.as_ref() else { return };
        if let Some(ev) = translate_event(event) {
            if let CoreEvent::MouseMove { .. } = ev {
                let cursor = DisplayServer::singleton().mouse_get_position();
                let first = self.last_cursor.is_none();
                if self.last_cursor == Some(cursor) {
                    return;
                }
                self.last_cursor = Some(cursor);
                if first {
                    // where the cursor rested when the window opened: not a movement
                    return;
                }
            }
            h.rt.send(ev);
        }
    }

    fn exit_tree(&mut self) {
        self.teardown();
    }
}

#[godot_api]
impl AsylumGame {
    /// The window's close button / Alt+F4 (auto-accept-quit is off).
    #[func]
    fn on_close_requested(&mut self) {
        self.quit();
    }
}

impl AsylumGame {
    fn visible_size(&self) -> Vector2 {
        self.base().get_viewport().map(|v| v.get_visible_rect().size).unwrap_or(Vector2::new(320.0, 256.0))
    }

    /// Stops the runtime (joins threads, saves config) and frees rendering objects.
    fn teardown(&mut self) {
        if let Some(mut h) = self.host.take() {
            h.rt.shutdown();
            h.audio.shutdown();
            h.gfx.free();
        }
    }

    fn begin_quit(&mut self, code: i32) {
        self.teardown();
        if self.quit_in.is_none() {
            self.quit_in = Some((QUIT_DELAY_FRAMES, code));
        }
    }

    fn quit(&mut self) {
        self.begin_quit(0);
    }

    fn fatal(&mut self, msg: &str) {
        godot_error!("{msg}");
        if self.quit_in.is_none() && DisplayServer::singleton().get_name() != "headless" {
            Os::singleton().alert_ex(msg).title("Asylum").done();
        }
        self.begin_quit(1);
    }

    fn automate(&mut self) {
        self.bot();
        if self.auto.keys.len() == self.auto.sent {
            return;
        }
        let now = self.auto.now();
        let Some(h) = self.host.as_ref() else { return };
        while let Some(&(at, code)) = self.auto.keys.get(self.auto.sent) {
            if at > now {
                break;
            }
            self.auto.sent += 1;
            let ch = u32::try_from(code).ok().filter(|c| *c < 0x10000).and_then(lower_char);
            h.rt.send(CoreEvent::Key { code, ch, echo: false });
        }
    }

    /// ASYLUM_BOT step: new held keys when due, and sometimes a key press.
    fn bot(&mut self) {
        if self.auto.bot.is_none() {
            return;
        }
        let now = self.auto.now();
        let Some(h) = self.host.as_ref() else { return };
        let Some(b) = self.auto.bot.as_mut() else { return };
        if now >= b.until {
            let v = b.next();
            let mut held = 0;
            if v & 3 == 1 {
                held |= core_input::HELD_LEFT;
            }
            if v & 3 == 2 {
                held |= core_input::HELD_RIGHT;
            }
            if v & 4 != 0 {
                held |= core_input::HELD_FIRE;
            }
            if v & 0x18 == 0x08 {
                held |= core_input::HELD_UP;
            }
            if v & 0x60 == 0x20 {
                held |= core_input::HELD_DOWN;
            }
            b.held = held;
            b.until = now + 12 + (b.next() % 78) as u64;
        }
        if now >= b.next_key {
            b.next_key = now + 20 + (b.next() % 100) as u64;
            // mostly what moves the game on (start, high scores, menus); Escape and F11 rarely
            const KEYS: [i64; 16] = [32, 32, 4194309, 4194309, 4194309, 49, 50, 51, 52, 53, 54, 79, 81, 82, 4194320, 4194322];
            let r = b.next() % 200;
            let code = match r {
                0..=1 => 4194305,      // Escape
                2 => 4194342,          // F11
                _ => KEYS[(r as usize) % KEYS.len()],
            };
            let ch = u32::try_from(code).ok().filter(|c| *c < 0x10000).and_then(lower_char);
            godot_print!("Asylum bot: key {code} at {now}");
            h.rt.send(CoreEvent::Key { code, ch, echo: false });
        }
    }

    fn screenshot(&mut self) {
        let Some(at) = self.auto.shot.as_ref().map(|s| s.0) else { return };
        if self.auto.now() < at {
            return;
        }
        let Some((_, path)) = self.auto.shot.take() else { return };
        let img = self.base().get_viewport().and_then(|v| v.get_texture()).and_then(|t| t.get_image());
        match img {
            Some(img) => {
                let err = img.save_png(path.as_str());
                godot_print!("Asylum: screenshot {path}: {err:?}");
            }
            None => godot_warn!("Asylum: screenshot not available (headless?)"),
        }
        self.quit();
    }
}

/// `override.cfg` beside the executable, for a portable install (user data in
/// `<exe dir>/userdata`); never for debug runs with ASYLUM_USERDIR or in the editor.
fn start_cfg_path(userdir: &std::path::Path) -> Option<PathBuf> {
    if env("ASYLUM_USERDIR").is_some() || Os::singleton().has_feature("editor") {
        return None;
    }
    let exe = PathBuf::from(Os::singleton().get_executable_path().to_string());
    let dir = exe.parent()?;
    (userdir == dir.join("userdata")).then(|| dir.join("override.cfg"))
}

/// Records how the window should open next time (only when that changed).
fn write_start_cfg(h: &mut Host) {
    let Some(w) = h.video.start_window(&h.window) else { return };
    let Some((path, last)) = h.start_cfg.as_mut() else { return };
    let text = video::start_window_cfg(&w);
    if *last != text && std::fs::write(&*path, &text).is_ok() {
        *last = text;
    }
}

/// Held-action mask from the keyboard (bound + alternative keys) and joypads.
fn held_mask(cfg: &Config, pads: &[i32]) -> u32 {
    let input = Input::singleton();
    let key = |code: i64| input.is_physical_key_pressed(Key::from_ord(code as i32));
    let mut held = 0u32;
    for (a, &bound) in cfg.keys.iter().enumerate() {
        let mut on = key(bound);
        for &alt in ALT_KEYS[a] {
            // An alternative key is inactive when the player bound it to another action.
            if core_input::alt_active(&cfg.keys, a, alt) && key(alt) {
                on = true;
            }
        }
        if on {
            held |= 1 << a;
        }
    }
    for &d in pads {
        let b = |btn: JoyButton| input.is_joy_button_pressed(d, btn);
        let x = input.get_joy_axis(d, JoyAxis::LEFT_X);
        let y = input.get_joy_axis(d, JoyAxis::LEFT_Y);
        if b(JoyButton::DPAD_LEFT) || x < -STICK_DEADZONE {
            held |= core_input::HELD_LEFT;
        }
        if b(JoyButton::DPAD_RIGHT) || x > STICK_DEADZONE {
            held |= core_input::HELD_RIGHT;
        }
        if b(JoyButton::DPAD_UP) || y < -STICK_DEADZONE || b(JoyButton::A) {
            held |= core_input::HELD_UP;
        }
        if b(JoyButton::DPAD_DOWN) || y > STICK_DEADZONE {
            held |= core_input::HELD_DOWN;
        }
        if b(JoyButton::X) || b(JoyButton::B) {
            held |= core_input::HELD_FIRE;
        }
    }
    if input.is_physical_key_pressed(Key::ALT) {
        held |= HELD_ALT;
    }
    // cheat keys are polled as held, like keyboard.c cheatread
    for (k, bit) in [(Key::F1, core_input::HELD_F1), (Key::F2, core_input::HELD_F2), (Key::F3, core_input::HELD_F3)] {
        if input.is_physical_key_pressed(k) {
            held |= bit;
        }
    }
    held
}

/// Godot input event → core event (key/pad presses, mouse in logical viewport coordinates).
fn translate_event(event: Gd<InputEvent>) -> Option<CoreEvent> {
    let event = match event.try_cast::<InputEventKey>() {
        Ok(k) => {
            if !k.is_pressed() {
                return None;
            }
            let mut code = k.get_physical_keycode().ord() as i64;
            if code == 0 {
                code = k.get_keycode().ord() as i64;
            }
            let code = core_input::alt_enter_as_f11(code, k.is_alt_pressed());
            return Some(CoreEvent::Key { code, ch: lower_char(k.get_unicode()), echo: k.is_echo() });
        }
        Err(e) => e,
    };
    let event = match event.try_cast::<InputEventJoypadButton>() {
        // the raw index: an unmapped joystick (HOTAS, button box) reports buttons 27..127,
        // which gdext's JoyButton cannot represent (get_button_index() would panic)
        Ok(j) => return j.is_pressed().then(|| j.get("button_index").try_to::<i64>().ok()).flatten().map(|button| CoreEvent::Pad { button }),
        Err(e) => e,
    };
    let event = match event.try_cast::<InputEventMouseMotion>() {
        Ok(m) => {
            let p = m.get_position();
            return Some(CoreEvent::MouseMove { x: p.x, y: p.y });
        }
        Err(e) => e,
    };
    match event.try_cast::<InputEventMouseButton>() {
        Ok(m) if m.is_pressed() => {
            let p = m.get_position();
            // raw as well: X11 reports extra mouse buttons past XBUTTON2
            let button = m.get("button_index").try_to::<i64>().ok()?;
            u8::try_from(button).ok().map(|button| CoreEvent::MouseDown { x: p.x, y: p.y, button })
        }
        _ => None,
    }
}
