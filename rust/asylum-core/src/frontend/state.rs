//! The frontend state machine without rendering resources (so it is unit-testable):
//! asylum.c `init()` loop + menus.c screens, advanced one 50 Hz tick at a time.

use std::collections::VecDeque;
use std::sync::Arc;

use super::highscore::{entry_text, HsPhase, HsState};
use super::menus::{self, Act, Entry, MenuCtx, MenuId, Nav, VolOp};
use super::messages;
use super::{AppOutputs, HostRequest};
use crate::assets::GameAssets;
use crate::audio::music::MusicTrack;
use crate::audio::sfx::SfxId;
use crate::config::Config;
use crate::display;
use crate::i18n::{keys, Lang, Strings};
use crate::input::{key, pad, InputEvent, DEFAULT_KEYS, HELD_ALT};
use crate::layout::Layout;
use crate::render::{Background, MenuLine};
use crate::sim::game::{Flow, GameStart};
use crate::sim::{Align, SfxEvent, Sim, TextRef, TextTable, TickInput, TickResult};
use crate::storage::{default_highscores, Storage};

/// Minimum ticks of the loading screen.
const LOADING_MIN_TICKS: u32 = 10;
/// Cheat mode only: type-ahead window for `zonecheatread` (the original read the key typed
/// while the level loaded from disc).
const ZONE_CHEAT_TICKS: i32 = 50;
/// "Position Saved" visibility.
const SAVED_FLASH_TICKS: i32 = 100;
/// The define-keys prompts (choosekeys: selectkey(48, 128, ...)).
const KEY_PROMPTS: [&str; 5] =
    [keys::KEY_PRESS_LEFT, keys::KEY_PRESS_RIGHT, keys::KEY_PRESS_UP, keys::KEY_PRESS_DOWN, keys::KEY_PRESS_FIRE];
/// Default selection of the escape menu: "R - Return to Game".
const PAUSE_SEL_RETURN: usize = 2;
/// First cheat-key bit of the held mask (`input::HELD_F1..HELD_F3` = bits 6..8): keyboard.c
/// `cheatread` polls F1/F2/F3 as held (`osbyte_81(-SDL_SCANCODE_F1..F3)`) every frame.
const HELD_CHEAT_SHIFT: u32 = crate::input::HELD_F1.trailing_zeros();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub id: MenuId,
    pub sel: usize,
}

#[derive(Clone, Debug)]
pub enum Screen {
    /// showloading(): until the SFX bank is ready.
    Loading { ticks: u32 },
    /// prelude(): `scroll` animation ticks left.
    Prelude { scroll: i32 },
    /// The menu on top of `Core::stack`.
    Menu,
    /// choosekeys(): prompt `step`, keys collected so far.
    DefineKeys { step: usize, keys: [i64; 5] },
    /// Quit confirmation over `prev` (prelude or title menu).
    Quit { prev: Box<Screen>, sel: usize },
    /// Cheat mode only: zone-cheat window before game().
    Starting { ticks: i32 },
    Playing,
    /// escapehandler(): `anim` ticks of slide-in left, selected action row.
    Paused { anim: i32, sel: usize },
    HighScore(Box<HsState>),
}

/// What `scene()` hands to the renderer besides the lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneInfo {
    pub background: Background,
    /// Draw `sim.texts` (prelude, escape menu, game) instead of an empty table.
    pub sim_texts: bool,
    pub highlight: Option<[f32; 4]>,
}

/// Monitor size assumed until the host reports it (every resolution of the lists counts).
const MONITOR_UNKNOWN: (i32, i32) = (16384, 16384);

pub struct Core {
    pub sim: Sim,
    pub strings: Arc<Strings>,
    /// `None` in tests: no file IO.
    pub storage: Option<Storage>,
    pub config: Config,
    pub view: (f32, f32),
    pub lay: Layout,
    pub screen: Screen,
    /// Menu stack (root = Options) while `screen == Menu` / DefineKeys.
    pub stack: Vec<MenuState>,
    /// options_menu(1): the menus were opened from the escape menu.
    pub gameon: bool,
    /// options.mentalzone of the next game (1..=3, 4 = saved game; 4 is never persisted).
    pub mentalzone: u8,
    /// prelude() result.
    pub cheatpermit: u8,
    pub out: AppOutputs,
    pub empty_texts: TextTable,
    events: VecDeque<InputEvent>,
    held: u32,
    sfx_ready: bool,
    quit: bool,
    /// Track the menus play (Title; GameOver after a lost game until the next game).
    menu_music: MusicTrack,
    /// Last music command sent (`None` = nothing sent yet).
    music_sent: Option<Option<MusicTrack>>,
    /// tunevolume() started the music during options_menu(1); it plays until rejoin (mainrestart).
    volume_music_ingame: bool,
    save_exists: bool,
    saved_flash: i32,
    /// Last mouse position seen (viewport coords). A window that opens under a resting cursor
    /// reports that position once; only real movement after it may move the highlight, or
    /// Enter would activate whatever line happens to lie under the cursor.
    mouse_seen: Option<(f32, f32)>,
    /// Monitor size in pixels (`InputEvent::Monitor`; resolution lists of the video menu).
    monitor: (i32, i32),
    /// Full-screen mode F11 / Alt+Enter returns to (borderless unless full screen was used).
    last_full: u8,
}

fn is_enter(code: i64) -> bool {
    code == key::ENTER || code == key::KP_ENTER
}

/// Number key (readopt reads the character; keypad and non-QWERTY layouts via the code).
fn digit_of(code: i64, ch: Option<char>) -> Option<u8> {
    if let Some(c) = ch.filter(char::is_ascii_digit) {
        return Some(c as u8 - b'0');
    }
    if (key::KEY_0..=key::KEY_0 + 9).contains(&code) {
        return Some((code - key::KEY_0) as u8);
    }
    if (key::KP_0..=key::KP_0 + 9).contains(&code) {
        return Some((code - key::KP_0) as u8);
    }
    None
}

/// Letter key, lower case (escape menu Q/R/O; the physical key when the IME produced none).
fn letter_of(code: i64, ch: Option<char>) -> Option<char> {
    if let Some(c) = ch.filter(char::is_ascii_alphabetic) {
        return Some(c.to_ascii_lowercase());
    }
    if (key::A..=key::Z).contains(&code) {
        return Some((code as u8 as char).to_ascii_lowercase());
    }
    None
}

impl Core {
    pub fn new(assets: Arc<GameAssets>, strings: Arc<Strings>, storage: Option<Storage>, config: Config, view: (f32, f32), seed: u32) -> Core {
        let lay = Layout::new(view.0, view.1);
        let last_full = if config.display_mode == display::MODE_FULLSCREEN { display::MODE_FULLSCREEN } else { display::MODE_BORDERLESS };
        let mentalzone = if (1..=3).contains(&config.mental_zone) { config.mental_zone } else { 1 };
        let save_exists = storage.as_ref().is_some_and(|s| s.savegame_exists());
        let mut c = Core {
            sim: Sim::new(assets, lay, seed),
            strings,
            storage,
            config,
            view,
            lay,
            screen: Screen::Loading { ticks: 0 },
            stack: Vec::new(),
            gameon: false,
            mentalzone,
            cheatpermit: 0,
            out: AppOutputs::default(),
            empty_texts: TextTable::default(),
            events: VecDeque::new(),
            held: 0,
            sfx_ready: false,
            quit: false,
            menu_music: MusicTrack::Title,
            music_sent: None,
            volume_music_ingame: false,
            save_exists,
            saved_flash: 0,
            mouse_seen: None,
            monitor: MONITOR_UNKNOWN,
            last_full,
        };
        c.push_apply_video();
        c.push_apply_audio();
        c.update_music();
        c
    }

    pub fn lang(&self) -> Lang {
        self.config.language.unwrap_or_default()
    }

    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    pub fn push_event(&mut self, ev: InputEvent) {
        if !self.quit {
            self.events.push_back(ev);
        }
    }

    pub fn set_sfx_ready(&mut self) {
        self.sfx_ready = true;
    }

    pub fn set_view(&mut self, w: f32, h: f32) {
        self.view = (w, h);
        self.relayout();
    }

    /// Recomputes the layout; texts placed in 320x256-space move with the centred band.
    fn relayout(&mut self) {
        let new = Layout::new(self.view.0, self.view.1);
        self.sim.texts.relayout(&self.lay, &new);
        self.lay = new;
        self.sim.lay = new;
    }

    /// One 50 Hz tick: queued events in order, then the current screen.
    pub fn tick(&mut self, held: u32) {
        if self.quit {
            return;
        }
        self.held = held;
        while let Some(ev) = self.events.pop_front() {
            if let InputEvent::MouseMove { x, y } = ev {
                // the anchor stays until the cursor has moved a whole pixel from it, so slow
                // sub-pixel motion (high-DPI mice, scaled viewports) still adds up
                match self.mouse_seen {
                    Some((px, py)) if (x - px).abs() + (y - py).abs() >= 1.0 => self.mouse_seen = Some((x, y)),
                    Some(_) => continue,
                    None => {
                        self.mouse_seen = Some((x, y));
                        continue;
                    }
                }
            }
            if let InputEvent::MouseDown { x, y, .. } = ev {
                self.mouse_seen = Some((x, y));
            }
            self.handle_event(ev);
            if self.quit {
                self.events.clear();
                return;
            }
        }
        self.step();
        if self.saved_flash > 0 {
            self.saved_flash -= 1;
        }
        self.update_music();
    }

    pub fn drain(&mut self, out: &mut AppOutputs) {
        out.sfx.append(&mut self.out.sfx);
        out.requests.append(&mut self.out.requests);
        if let Some(m) = self.out.music.take() {
            out.music = Some(m);
        }
    }

    // ------------------------------------------------------------------ persistence

    fn save_config(&self) {
        if let Some(s) = &self.storage {
            let _ = s.save_config(&self.config);
        }
    }

    fn push_apply_audio(&mut self) {
        let c = &self.config;
        let r = HostRequest::ApplyAudio { sound_mode: c.sound_mode, sfx_volume: c.sfx_volume, music_volume: c.music_volume };
        self.out.requests.push(r);
    }

    fn push_apply_video(&mut self) {
        self.out.requests.push(HostRequest::ApplyVideo(self.config.clone()));
    }

    fn set_video(&mut self, f: impl FnOnce(&mut Config, (i32, i32))) {
        // rows 2/3 of Tune Video come and go with the window mode: keep the highlight on the
        // same option, not on the same position
        let keep = self.highlighted_act(MenuId::Video);
        f(&mut self.config, self.monitor);
        if self.config.display_mode != display::MODE_WINDOW {
            self.last_full = self.config.display_mode;
        }
        if let Some(act) = keep {
            self.highlight_act(MenuId::Video, act);
        }
        self.save_config();
        self.push_apply_video();
    }

    /// Action of the highlighted entry when menu `id` is on screen.
    fn highlighted_act(&self, id: MenuId) -> Option<Act> {
        let top = self.stack.last().filter(|t| t.id == id && matches!(self.screen, Screen::Menu))?;
        let e = self.entries(id);
        menus::selectable(&e).get(top.sel).and_then(|&i| e[i].act)
    }

    /// Moves the highlight of menu `id` to the entry with `act` (or the first one).
    fn highlight_act(&mut self, id: MenuId, act: Act) {
        let e = self.entries(id);
        let k = menus::selectable(&e).iter().position(|&i| e[i].act == Some(act)).unwrap_or(0);
        if let Some(top) = self.stack.last_mut().filter(|t| t.id == id) {
            top.sel = k;
        }
    }

    /// F11 / Alt+Enter: window ↔ the last full-screen mode.
    fn toggle_fullscreen(&mut self) {
        let last = self.last_full;
        self.set_video(|c, _| c.display_mode = if c.display_mode == display::MODE_WINDOW { last } else { display::MODE_WINDOW });
    }

    fn quit_now(&mut self) {
        self.save_config();
        self.out.requests.push(HostRequest::Quit);
        self.quit = true;
    }

    fn refresh_save_exists(&mut self) {
        self.save_exists = self.storage.as_ref().is_some_and(|s| s.savegame_exists());
    }

    // ------------------------------------------------------------------ music

    fn desired_music(&self) -> Option<MusicTrack> {
        if self.config.sound_mode != 2 {
            return None;
        }
        match &self.screen {
            Screen::Playing => Some(MusicTrack::Zone { zone: self.sim.zone, part: self.sim.music_part() }),
            // game(): swi_bodgemusic_stop() before the game / while escapehandler runs
            Screen::Paused { .. } | Screen::Starting { .. } => None,
            // tunevolume() starts the title music so the volume can be judged
            _ if self.top() == Some(MenuId::Volume) && matches!(self.screen, Screen::Menu) => Some(MusicTrack::Title),
            // ...and nothing stops it again before rejoin (mainrestart restarts the zone music)
            Screen::Menu | Screen::DefineKeys { .. } if self.gameon && !self.volume_music_ingame => None,
            _ => Some(self.menu_music),
        }
    }

    fn update_music(&mut self) {
        let want = self.desired_music();
        if self.music_sent != Some(want) {
            self.music_sent = Some(want);
            self.out.music = Some(want);
        }
    }

    // ------------------------------------------------------------------ helpers

    fn top(&self) -> Option<MenuId> {
        self.stack.last().map(|m| m.id)
    }

    fn content(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.lay.ox, y - self.lay.oy)
    }

    fn is_fire(&self, code: i64) -> bool {
        crate::input::triggers(&self.config.keys, 4, code)
    }

    fn menu_ctx(&self) -> MenuCtx<'_> {
        MenuCtx {
            strings: &self.strings,
            lang: self.lang(),
            config: &self.config,
            hx: self.lay.hx,
            cy: self.lay.cy,
            monitor: self.monitor,
            gameon: self.gameon,
            save_exists: self.save_exists,
            saved_flash: self.saved_flash > 0,
        }
    }

    /// Escape menu rows in content coordinates.
    fn pause_rows(&self) -> impl Iterator<Item = f32> {
        let cy = self.lay.cy as f32;
        messages::PAUSE_ROWS.into_iter().map(move |y| y + cy)
    }

    fn entries(&self, id: MenuId) -> Vec<Entry> {
        menus::entries(id, &self.menu_ctx())
    }

    fn menu_state(&self, id: MenuId) -> MenuState {
        MenuState { id, sel: menus::default_sel(id, &self.menu_ctx(), self.mentalzone) }
    }

    /// Back to the title options menu (`for (;;) { options_menu(0) ... }`).
    fn goto_title_menu(&mut self) {
        self.sim.texts.wipe();
        self.gameon = false;
        self.volume_music_ingame = false;
        self.stack.clear();
        let st = self.menu_state(MenuId::Options);
        self.stack.push(st);
        self.screen = Screen::Menu;
    }

    fn open(&mut self, id: MenuId) {
        if id == MenuId::Zone {
            self.refresh_save_exists();
        }
        // tunevolume starts the title music only with music on (soundtype 2)
        if id == MenuId::Volume && self.config.sound_mode == 2 {
            self.menu_music = MusicTrack::Title;
            if self.gameon {
                self.volume_music_ingame = true;
            }
        }
        let st = self.menu_state(id);
        self.stack.push(st);
        self.screen = Screen::Menu;
    }

    /// Generic menu input mapping (`e`: the entries of the menu the event goes to).
    fn nav_of(&self, ev: &InputEvent, e: &[Entry]) -> Nav {
        match *ev {
            InputEvent::Key { code, ch, echo } => {
                let up = crate::input::triggers(&self.config.keys, 2, code);
                let down = crate::input::triggers(&self.config.keys, 3, code);
                if echo {
                    return if up {
                        Nav::Up
                    } else if down {
                        Nav::Down
                    } else {
                        Nav::None
                    };
                }
                if code == key::ESCAPE {
                    Nav::Back
                } else if is_enter(code) {
                    Nav::Activate
                } else if let Some(d) = digit_of(code, ch) {
                    if !self.is_fire(code) {
                        Nav::Digit(d)
                    } else if (key::KP_0..=key::KP_0 + 9).contains(&code) {
                        // readopt: keypad keys never produce a character (update_keyboard
                        // fills unibuf for sym 33..127 only), so only the Fire check sees them
                        Nav::Fire
                    } else if d == 0 || menus::selectable(e).is_empty() || e.iter().any(|x| x.act.is_some() && x.digit == Some(d)) {
                        // a typed digit an option answers (info screens: fire_after_digit)
                        Nav::Digit(d)
                    } else {
                        // a typed digit above maxopt falls through to `osbyte_81(firekey)`
                        Nav::Fire
                    }
                } else if self.is_fire(code) {
                    Nav::Fire
                } else if up {
                    Nav::Up
                } else if down {
                    Nav::Down
                } else {
                    Nav::None
                }
            }
            InputEvent::Pad { button } => match button {
                pad::DPAD_UP => Nav::Up,
                pad::DPAD_DOWN => Nav::Down,
                pad::A => Nav::Activate,
                pad::B | pad::BACK => Nav::Back,
                pad::START => Nav::Fire,
                _ => Nav::None,
            },
            InputEvent::MouseMove { x, y } => {
                let (x, y) = self.content(x, y);
                Nav::Hover(x, y)
            }
            InputEvent::MouseDown { x, y, button } => match button {
                1 => {
                    let (x, y) = self.content(x, y);
                    Nav::Click(x, y)
                }
                2 => Nav::Back,
                4 => Nav::Up,
                5 => Nav::Down,
                _ => Nav::None,
            },
            InputEvent::CloseRequest | InputEvent::Monitor { .. } => Nav::None,
        }
    }

    // ------------------------------------------------------------------ events

    fn handle_event(&mut self, ev: InputEvent) {
        match ev {
            InputEvent::CloseRequest => return self.quit_now(),
            // F11, or Alt+Enter (sent as F11 by the host: `input::alt_enter_as_f11`)
            InputEvent::Key { code: crate::input::KEY_DISPLAY_MODE, echo: false, .. } => {
                return self.toggle_fullscreen();
            }
            InputEvent::Monitor { width, height } => {
                // the host applies the video settings for the new monitor itself
                self.monitor = (width, height);
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Loading { .. } => {}
            Screen::Starting { .. } => self.starting_event(&ev),
            Screen::Prelude { .. } => self.prelude_event(&ev),
            Screen::Menu => self.menu_event(&ev),
            Screen::DefineKeys { .. } => self.define_keys_event(&ev),
            Screen::Quit { .. } => self.quit_event(&ev),
            Screen::Playing => self.playing_event(&ev),
            Screen::Paused { .. } => self.paused_event(&ev),
            Screen::HighScore(_) => self.highscore_event(&ev),
        }
    }

    fn open_quit(&mut self) {
        let prev = std::mem::replace(&mut self.screen, Screen::Menu);
        self.screen = Screen::Quit { prev: Box::new(prev), sel: 0 };
    }

    /// prelude(): any key or click ends it (Alt alone does not: the C code excludes the SDL1
    /// Alt key numbers 307/308 so Alt + middle button can enable the cheat).
    fn prelude_event(&mut self, ev: &InputEvent) {
        match *ev {
            InputEvent::Key { code, echo, .. } => {
                if echo || code == key::ALT {
                    return;
                }
                if code == key::ESCAPE {
                    return self.open_quit();
                }
                self.goto_title_menu();
            }
            InputEvent::Pad { .. } => self.goto_title_menu(),
            InputEvent::MouseDown { button: 3, .. } => {
                // gocheat:
                if self.held & HELD_ALT == 0 {
                    return self.goto_title_menu();
                }
                self.cheatpermit = 1;
                if let Screen::Prelude { scroll } = &mut self.screen {
                    *scroll = messages::PRELUDE_CHEAT_SCROLL;
                }
            }
            InputEvent::MouseDown { button: 1 | 2, .. } => self.goto_title_menu(),
            _ => {}
        }
    }

    fn quit_event(&mut self, ev: &InputEvent) {
        let e = self.entries(MenuId::Quit);
        let nav = self.nav_of(ev, &e);
        let Screen::Quit { sel, .. } = &mut self.screen else { return };
        match menus::navigate(&e, sel, menus::fire_act(MenuId::Quit), nav, self.lay.cw, 9) {
            Some(Act::QuitYes) => self.quit_now(),
            Some(_) => {
                if let Screen::Quit { prev, .. } = std::mem::replace(&mut self.screen, Screen::Menu) {
                    self.screen = *prev;
                }
            }
            None => {}
        }
    }

    fn menu_event(&mut self, ev: &InputEvent) {
        let Some(top) = self.stack.last().copied() else {
            return self.goto_title_menu();
        };
        let e = self.entries(top.id);
        let maxopt = menus::info_maxopt(top.id);
        // readopt checks the digit range first, then the Fire key (a Fire key bound to a digit)
        let fire_key = matches!(*ev, InputEvent::Key { code, .. } if self.is_fire(code));
        let nav = menus::fire_after_digit(&e, self.nav_of(ev, &e), maxopt, fire_key);
        let mut sel = top.sel;
        let act = menus::navigate(&e, &mut sel, menus::fire_act(top.id), nav, self.lay.cw, maxopt);
        if let Some(m) = self.stack.last_mut() {
            m.sel = sel;
        }
        if let Some(a) = act {
            self.apply(a);
        }
    }

    /// Back out of the top menu.
    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
            return;
        }
        if self.gameon {
            // options_menu(1) returns, adjustopt → rejoin
            self.rejoin();
        } else {
            // options_menu(0) == 1 → abort_game; here: confirm first
            self.open_quit();
        }
    }

    fn apply(&mut self, a: Act) {
        match a {
            Act::Back => self.back(),
            Act::Play => {
                if self.gameon {
                    self.rejoin();
                } else {
                    self.play();
                }
            }
            Act::Open(id) => self.open(id),
            Act::DefineKeys => {
                self.screen = Screen::DefineKeys { step: 0, keys: self.config.keys };
            }
            Act::DefaultKeys => {
                self.config.keys = DEFAULT_KEYS;
                self.save_config();
            }
            Act::Sound(m) => {
                self.config.sound_mode = m;
                self.save_config();
                self.push_apply_audio();
                if m == 0 {
                    self.out.requests.push(HostRequest::StopSfx);
                }
            }
            Act::Volume(op) => self.volume(op),
            Act::Video(o) => self.set_video(|c, _| menus::video_toggle(c, o)),
            Act::DisplayMode => self.set_video(|c, _| c.display_mode = display::next_mode(c.display_mode)),
            // the wanted resolution stays; each aspect resolves it to its tallest fitting size
            Act::Aspect => self.set_video(|c, m| c.window_aspect = display::next_aspect(c.window_aspect, m)),
            Act::Resolution => self.set_video(|c, m| {
                let cur = display::pick_resolution(c.window_aspect, c.window_resolution, m);
                c.window_resolution = display::next_resolution(c.window_aspect, cur, m);
            }),
            Act::ScreenScale => self.set_video(|c, m| {
                let v = display::video(c, m);
                c.screen_scale = display::next_scale(v.scale, display::max_scale(v.resolution));
            }),
            Act::Lang(l) => {
                self.config.language = Some(l);
                self.save_config();
            }
            Act::Zone(z) => {
                if z == 3 && !self.config.id_permit {
                    // "To play the id ..." then back to the options menu
                    self.stack.pop();
                    self.open(MenuId::IdLock);
                } else {
                    self.mentalzone = z;
                    if z <= 3 {
                        self.config.mental_zone = z;
                        self.save_config();
                    }
                    self.stack.pop();
                }
            }
            Act::SavePosition => {
                let bytes = self.sim.savegame_bytes();
                if let Some(s) = &self.storage {
                    if s.write_savegame(&bytes).is_ok() {
                        self.saved_flash = SAVED_FLASH_TICKS;
                    }
                } else {
                    self.saved_flash = SAVED_FLASH_TICKS;
                }
                self.refresh_save_exists();
            }
            Act::QuitYes => self.quit_now(),
        }
    }

    fn volume(&mut self, op: VolOp) {
        let c = &mut self.config;
        match op {
            VolOp::FxUp => c.sfx_volume = (c.sfx_volume + 1).min(10),
            VolOp::FxDown => c.sfx_volume = c.sfx_volume.saturating_sub(1),
            VolOp::MusicUp => c.music_volume = (c.music_volume + 1).min(10),
            VolOp::MusicDown => c.music_volume = c.music_volume.saturating_sub(1),
        }
        self.save_config();
        self.push_apply_audio();
        // maketestsound(): voice 1 on channel 1 (the music is audible by itself)
        if matches!(op, VolOp::FxUp | VolOp::FxDown) && self.config.sound_mode >= 1 {
            self.out.sfx.push(SfxEvent { channel: 1, id: SfxId::JUMP, volume: 127, stereo: 0 });
        }
    }

    /// choosekeys(): the next key-down is stored; ESC aborts and keeps the old keys.
    fn define_keys_event(&mut self, ev: &InputEvent) {
        match *ev {
            InputEvent::Key { code, echo: false, .. } if code != key::ESCAPE => {
                let Screen::DefineKeys { step, keys } = &mut self.screen else { return };
                keys[*step] = code;
                *step += 1;
                if *step < 5 {
                    return;
                }
                self.config.keys = *keys;
                self.save_config();
            }
            InputEvent::Key { code: key::ESCAPE, echo: false, .. }
            | InputEvent::Pad { button: pad::B }
            | InputEvent::MouseDown { button: 2, .. } => {}
            _ => return,
        }
        self.screen = Screen::Menu;
    }

    fn play(&mut self) {
        if self.cheatpermit == 1 {
            self.screen = Screen::Starting { ticks: ZONE_CHEAT_TICKS };
        } else {
            self.begin_game(None);
        }
    }

    /// zonecheatread(): '0'..'8' typed before the game starts selects the starting neuron.
    fn starting_event(&mut self, ev: &InputEvent) {
        if let InputEvent::Key { code, ch, echo: false } = *ev {
            if let Some(d) = digit_of(code, ch).filter(|d| *d <= 8) {
                self.begin_game(Some(d as i32));
            }
        }
    }

    /// `currentzone = 0; getlevelfiles(); swi_bodgemusic_stop(); game()`.
    fn begin_game(&mut self, zone_cheat: Option<i32>) {
        self.sim.lay = self.lay;
        let savegame = if self.mentalzone == 4 { self.storage.as_ref().and_then(|s| s.read_savegame()) } else { None };
        let start = GameStart { mentalzone: self.mentalzone, cheatpermit: self.cheatpermit, zone_cheat, savegame };
        match self.sim.start_game(start) {
            Ok(()) => {
                // loadgame(): fread(&options.mentalzone, 1, 1, r0) replaced options.mentalzone
                // (1..=3, validated by check_savegame); C does not save the config here.
                if self.mentalzone == 4 && (1..=3).contains(&self.sim.mentalzone) {
                    self.mentalzone = self.sim.mentalzone;
                    self.config.mental_zone = self.sim.mentalzone;
                }
                self.stack.clear();
                self.gameon = false;
                self.volume_music_ingame = false;
                // the game over tune of a previous game does not survive an abandoned one
                self.menu_music = MusicTrack::Title;
                self.screen = Screen::Playing;
            }
            Err(_) => {
                // missing/corrupt save: back to the zone menu
                self.goto_title_menu();
                self.open(MenuId::Zone);
            }
        }
    }

    fn playing_event(&mut self, ev: &InputEvent) {
        match *ev {
            InputEvent::Key { code, echo, .. } => {
                if (key::F1..key::F1 + 3).contains(&code) {
                    // F1..F3 never pause; cheatread polls the held mask (HELD_F1..HELD_F3).
                } else if !echo && (code == key::ESCAPE || code == key::PAUSE) {
                    self.pause();
                }
            }
            InputEvent::Pad { button: pad::START | pad::BACK } => self.pause(),
            _ => {}
        }
    }

    /// The game loop saw ESC: `swi_bodgemusic_stop(); redraw_bonus(); escapehandler()`.
    fn pause(&mut self) {
        self.out.requests.push(HostRequest::StopSfx);
        // the host stopped every chunk, so no SDL_mixer channel is playing any more
        // the host stops every effect (StopSfx), so no channel is still playing
        self.sim.halt_all_channels();
        self.sim.redraw_bonus();
        self.sim.lay = self.lay;
        messages::pause(&mut self.sim);
        self.screen = Screen::Paused { anim: messages::PAUSE_ANIM, sel: PAUSE_SEL_RETURN };
    }

    /// rejoin(): wipetexttab; showgamescreen; then backprep + mainrestart.
    fn rejoin(&mut self) {
        self.sim.texts.wipe();
        self.sim.lay = self.lay;
        self.sim.resume();
        self.stack.clear();
        self.gameon = false;
        self.volume_music_ingame = false;
        self.screen = Screen::Playing;
    }

    /// escapehandler() returned 1: game() returns 1, back to options_menu(0).
    fn abandon(&mut self) {
        self.sim.flow = Flow::Idle;
        self.goto_title_menu();
    }

    fn pause_action(&mut self, row: usize) {
        match row {
            0 => self.abandon(),
            1 => {
                self.sim.loselife();
                self.rejoin();
            }
            2 => self.rejoin(),
            _ => {
                // adjustopt(): options_menu(1)
                self.sim.texts.wipe();
                self.gameon = true;
                self.volume_music_ingame = false;
                self.stack.clear();
                let st = self.menu_state(MenuId::Options);
                self.stack.push(st);
                self.screen = Screen::Menu;
            }
        }
    }

    fn paused_event(&mut self, ev: &InputEvent) {
        let Screen::Paused { sel, .. } = self.screen else { return };
        let mut row = None;
        let mut new_sel = sel;
        match *ev {
            InputEvent::Key { code, ch, echo } => {
                if code == key::UP {
                    new_sel = (sel + 3) % 4;
                } else if code == key::DOWN {
                    new_sel = (sel + 1) % 4;
                } else if !echo {
                    row = if code == key::ESCAPE {
                        Some(0)
                    } else if is_enter(code) {
                        Some(sel)
                    } else {
                        match letter_of(code, ch) {
                            Some('q') => Some(1),
                            Some('r') => Some(2),
                            Some('o') => Some(3),
                            _ => None,
                        }
                    };
                }
            }
            InputEvent::Pad { button } => match button {
                pad::DPAD_UP => new_sel = (sel + 3) % 4,
                pad::DPAD_DOWN => new_sel = (sel + 1) % 4,
                pad::A => row = Some(sel),
                pad::B | pad::START => row = Some(2),
                _ => {}
            },
            InputEvent::MouseMove { x, y } => {
                let (x, y) = self.content(x, y);
                if let Some(r) = menus::hit_row(self.pause_rows(), x, y, self.lay.cw) {
                    new_sel = r;
                }
            }
            InputEvent::MouseDown { x, y, button } => match button {
                1 => {
                    let (x, y) = self.content(x, y);
                    row = menus::hit_row(self.pause_rows(), x, y, self.lay.cw);
                }
                2 => row = Some(2),
                _ => {}
            },
            InputEvent::CloseRequest | InputEvent::Monitor { .. } => {}
        }
        if let Screen::Paused { sel, .. } = &mut self.screen {
            *sel = row.unwrap_or(new_sel);
        }
        if let Some(r) = row {
            self.pause_action(r);
        }
    }

    fn highscore_event(&mut self, ev: &InputEvent) {
        let fire = match *ev {
            InputEvent::Key { code, ch, echo: false } => {
                self.is_fire(code) || is_enter(code) || (code == key::ESCAPE) || digit_of(code, ch) == Some(0)
            }
            InputEvent::Pad { button } => matches!(button, pad::A | pad::B | pad::X | pad::START),
            InputEvent::MouseDown { button: 1, .. } => true,
            _ => false,
        };
        // only the fire key confirms a letter (osbyte_81(0) == -firekey), even when it is
        // bound to a digit; Enter is kept as a convenience
        let entry_fire = match *ev {
            InputEvent::Key { code, echo: false, .. } => self.is_fire(code) || is_enter(code),
            _ => fire,
        };
        let Screen::HighScore(hs) = &mut self.screen else { return };
        match hs.phase {
            HsPhase::Entry { .. } => {
                if entry_fire {
                    hs.fire = true;
                }
            }
            HsPhase::Finishing => {}
            // readopt(0)
            HsPhase::Done => {
                if fire {
                    self.goto_title_menu();
                }
            }
        }
    }

    // ------------------------------------------------------------------ per-tick logic

    fn step(&mut self) {
        if !matches!(self.screen, Screen::Playing) {
            // The sim animates its own table while playing; elsewhere texts only move when
            // animate() is called below.
            self.sim.texts.hold();
        }
        match &mut self.screen {
            Screen::Loading { ticks } => {
                *ticks += 1;
                if self.sfx_ready && *ticks >= LOADING_MIN_TICKS {
                    self.start_prelude();
                }
            }
            Screen::Prelude { scroll } => {
                if *scroll > 0 {
                    *scroll -= 1;
                    self.sim.texts.animate(1);
                }
            }
            Screen::Paused { anim, .. } => {
                if *anim > 0 {
                    *anim -= 1;
                    self.sim.texts.animate(1);
                }
            }
            Screen::Starting { ticks } => {
                *ticks -= 1;
                if *ticks <= 0 {
                    self.begin_game(None);
                }
            }
            Screen::Playing => self.play_tick(),
            Screen::HighScore(hs) => {
                if hs.tick(self.held, &mut self.config.initials) {
                    // scoreexit: savescores; dosaveconf
                    if let Some(s) = &self.storage {
                        let _ = s.save_highscores(hs.zone, &hs.table);
                    }
                    self.save_config();
                }
            }
            Screen::Menu | Screen::DefineKeys { .. } | Screen::Quit { .. } => {}
        }
    }

    fn start_prelude(&mut self) {
        self.sim.lay = self.lay;
        let lang = self.lang();
        messages::prelude(&mut self.sim, &self.strings, lang);
        self.screen = Screen::Prelude { scroll: messages::PRELUDE_SCROLL };
    }

    fn play_tick(&mut self) {
        self.sim.lay = self.lay;
        // bit0 F1, bit1 F2, bit2 F3 on every tick the key is down (keyboard.c cheatread)
        let cheat_keys = ((self.held >> HELD_CHEAT_SHIFT) & 7) as u8;
        let r = self.sim.tick(TickInput { held: self.held, cheat_keys });
        if self.config.sound_mode >= 1 {
            self.out.sfx.extend_from_slice(&self.sim.sfx_events);
        }
        if let TickResult::GameOver { completed } = r {
            self.game_over(completed);
        }
    }

    /// init(): after game() returned 0 — permitid / music, then showhighscore().
    fn game_over(&mut self, completed: bool) {
        self.out.requests.push(HostRequest::StopSfx);
        let mut id_unlocked = false;
        if completed {
            if !self.config.id_permit {
                self.config.id_permit = true;
                id_unlocked = true;
                self.save_config();
            }
            self.menu_music = MusicTrack::Title;
        } else {
            self.menu_music = MusicTrack::GameOver;
        }
        // loadgame() may have replaced options.mentalzone
        let zone = if (1..=3).contains(&self.sim.mentalzone) { self.sim.mentalzone } else { self.mentalzone.clamp(1, 3) };
        self.mentalzone = zone;
        self.config.mental_zone = zone;
        let table = self.storage.as_ref().map(|s| s.load_highscores(zone)).unwrap_or_else(default_highscores);
        let hs = HsState::new(table, &self.sim.plscore, self.config.initials, zone, id_unlocked);
        self.sim.texts.wipe();
        self.screen = Screen::HighScore(Box::new(hs));
    }

    // ------------------------------------------------------------------ scene

    /// Fills `lines` for the current screen and describes the rest of the scene.
    pub fn scene(&self, lines: &mut Vec<MenuLine>) -> SceneInfo {
        lines.clear();
        let (hx, cy) = (self.lay.hx, self.lay.cy);
        let line = |text: TextRef, x: i32, y: i32, align: Align| MenuLine { text, x: (x + hx) as f32, y: (y + cy) as f32, align, prefix: None };
        let chat = |highlight| SceneInfo { background: Background::Chat, sim_texts: false, highlight };
        match &self.screen {
            Screen::Loading { .. } => {
                lines.push(line(TextRef::Key(keys::LOAD_WAIT), 88, 64, Align::EnCentered));
                lines.push(line(TextRef::Key(keys::LOAD_FILES), 48, 128, Align::EnCentered));
                chat(None)
            }
            Screen::Prelude { .. } => SceneInfo { background: Background::Chat, sim_texts: true, highlight: None },
            Screen::Menu => match self.stack.last() {
                Some(top) => self.menu_scene(top.id, top.sel, lines),
                None => chat(None),
            },
            Screen::Quit { sel, .. } => self.menu_scene(MenuId::Quit, *sel, lines),
            Screen::DefineKeys { step, .. } => {
                let k = KEY_PROMPTS[(*step).min(4)];
                lines.push(line(TextRef::Key(k), 48, 128, Align::EnCentered));
                chat(None)
            }
            Screen::Starting { .. } => SceneInfo { background: Background::GameFrame, sim_texts: false, highlight: None },
            Screen::Playing => SceneInfo { background: Background::Game, sim_texts: true, highlight: None },
            Screen::Paused { anim, sel } => {
                let highlight = (*anim == 0).then(|| menus::highlight_rect(hx, messages::PAUSE_ROWS[*sel] + cy as f32));
                SceneInfo { background: Background::Chat, sim_texts: true, highlight }
            }
            Screen::HighScore(hs) => {
                // showhst()
                lines.push(line(TextRef::Key(keys::HS_TITLE), 64, 32, Align::EnCentered));
                for i in 0..5 {
                    lines.push(line(TextRef::Raw(entry_text(&hs.table, i)), 32, 64 + 32 * i as i32, Align::Orig));
                }
                if hs.hstindex < 5 {
                    lines.push(line(TextRef::Raw("←".into()), 280, (hs.hstindex as i32 + 2) << 5, Align::Orig));
                }
                let mut highlight = None;
                match hs.phase {
                    HsPhase::Done => {
                        if hs.id_unlocked {
                            let cx = (160 + hx) as f32;
                            lines.push(line(TextRef::Key(keys::MSG_ID_UNLOCKED), 160, 208, Align::Center(cx)));
                        }
                        lines.push(line(TextRef::Key(keys::HS_PRESS_FIRE), 96, 224, Align::EnCentered));
                    }
                    _ => highlight = Some(menus::highlight_rect(hx, (64 + 32 * hs.hstindex as i32 + cy) as f32)),
                }
                chat(highlight)
            }
        }
    }

    fn menu_scene(&self, id: MenuId, sel: usize, lines: &mut Vec<MenuLine>) -> SceneInfo {
        let e = self.entries(id);
        let sels = menus::selectable(&e);
        let highlight = sels.get(sel.min(sels.len().saturating_sub(1))).map(|&i| menus::highlight_rect(self.lay.hx, e[i].line.y));
        lines.extend(e.into_iter().map(|x| x.line));
        SceneInfo { background: Background::Chat, sim_texts: false, highlight }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

/// Regression tests: loadgame's mentalzone, a Fire key bound to a digit, in-game volume music.
#[cfg(test)]
mod regression_tests {
    use super::*;
    use crate::input::HELD_RIGHT;

    fn core_with(config: Config, storage: Option<Storage>) -> Core {
        Core::new(crate::assets::testdata::assets(), Arc::new(Strings::default()), storage, config, (320.0, 256.0), 7)
    }

    fn key_ev(c: &mut Core, code: i64, ch: Option<char>) {
        c.push_event(InputEvent::Key { code, ch, echo: false });
        c.tick(0);
    }

    fn press(c: &mut Core, code: i64) {
        key_ev(c, code, None);
    }

    /// A typed number key ('0'..'9' row, with its character).
    fn digit(c: &mut Core, d: u8) {
        key_ev(c, key::KEY_0 + d as i64, Some((b'0' + d) as char));
    }

    fn letter(c: &mut Core, l: char) {
        key_ev(c, key::A + (l.to_ascii_uppercase() as u8 - b'A') as i64, Some(l));
    }

    fn music(c: &mut Core) -> Option<Option<MusicTrack>> {
        let mut o = AppOutputs::default();
        c.drain(&mut o);
        o.music
    }

    fn at_menu(c: &mut Core) {
        c.set_sfx_ready();
        for _ in 0..LOADING_MIN_TICKS {
            c.tick(0);
        }
        press(c, key::SPACE);
        assert!(matches!(c.screen, Screen::Menu));
        assert_eq!(c.top(), Some(MenuId::Options));
    }

    fn fake_playing(c: &mut Core) {
        at_menu(c);
        c.stack.clear();
        c.screen = Screen::Playing;
    }

    #[test]
    fn loaded_game_replaces_mentalzone_and_abandon_keeps_it() {
        let dir = std::env::temp_dir().join(format!("asylum-frontend-loadzone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let st = Storage::new(dir.clone());
        let mut c = core_with(Config { mental_zone: 2, ..Config::default() }, Some(st.clone()));
        at_menu(&mut c);
        press(&mut c, key::ENTER);
        assert_eq!(c.sim.zone, 1, "Psyche");
        for _ in 0..50 {
            c.tick(HELD_RIGHT);
        }
        // save in Psyche, return, abandon
        press(&mut c, key::ESCAPE);
        letter(&mut c, 'o');
        digit(&mut c, 3);
        assert!(st.savegame_exists());
        press(&mut c, key::ENTER);
        press(&mut c, key::ESCAPE);
        press(&mut c, key::ESCAPE);
        // select Ego, then "4. Saved Game"
        digit(&mut c, 3);
        digit(&mut c, 1);
        assert_eq!((c.mentalzone, c.config.mental_zone), (1, 1));
        digit(&mut c, 3);
        digit(&mut c, 4);
        assert_eq!((c.mentalzone, c.config.mental_zone), (4, 1));
        press(&mut c, key::ENTER);
        assert!(matches!(c.screen, Screen::Playing));
        // loadgame(): fread(&options.mentalzone) replaced the global; the config is not saved
        assert_eq!((c.sim.mentalzone, c.mentalzone, c.config.mental_zone), (2, 2, 2));
        assert_eq!(st.load_config().mental_zone, 1);
        // a later dosaveconf (in-game Video toggle) persists the loaded zone
        press(&mut c, key::ESCAPE);
        letter(&mut c, 'o');
        digit(&mut c, 2);
        digit(&mut c, 3);
        assert_eq!(c.top(), Some(MenuId::Video));
        digit(&mut c, 5);
        assert_eq!(st.load_config().mental_zone, 2);
        for _ in 0..3 {
            press(&mut c, key::ESCAPE);
        }
        assert!(matches!(c.screen, Screen::Playing));
        // abandon (ESC, ESC): options.mentalzone keeps the loaded zone
        press(&mut c, key::ESCAPE);
        press(&mut c, key::ESCAPE);
        assert_eq!(c.top(), Some(MenuId::Options));
        assert_eq!(c.mentalzone, 2);
        digit(&mut c, 3);
        assert_eq!(c.stack.last().unwrap().sel, 1, "zone menu highlights Psyche, not the saved game");
        press(&mut c, key::ESCAPE);
        // Fire - Play starts Psyche fresh (restartplayer), without reading the save file
        std::fs::remove_file(st.savegame_path()).unwrap();
        press(&mut c, key::ENTER);
        assert!(matches!(c.screen, Screen::Playing), "a fresh game, not a reload of the save");
        assert_eq!((c.sim.zone, c.sim.mentalzone), (1, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fire_key_bound_to_a_digit_acts_as_fire_in_menus() {
        // a typed digit above maxopt falls through to the Fire check: Play
        let mut c = core_with(Config::default(), None);
        at_menu(&mut c);
        c.config.keys[4] = key::KEY_0 + 5;
        digit(&mut c, 5);
        assert!(matches!(c.screen, Screen::Playing));
        // a typed digit an option answers still selects that option
        let mut c = core_with(Config::default(), None);
        at_menu(&mut c);
        c.config.keys[4] = key::KEY_0 + 1;
        digit(&mut c, 1);
        assert_eq!(c.top(), Some(MenuId::Controls));
        // keypad keys never produce a character: a keypad Fire key always plays
        let mut c = core_with(Config::default(), None);
        at_menu(&mut c);
        c.config.keys[4] = key::KP_0 + 1;
        key_ev(&mut c, key::KP_0 + 1, Some('1'));
        assert!(matches!(c.screen, Screen::Playing), "KP_1 bound as Fire must not open Controls");
        // keypad digits that are not the Fire key still select options
        let mut c = core_with(Config::default(), None);
        at_menu(&mut c);
        c.config.keys[4] = key::KP_0 + 1;
        key_ev(&mut c, key::KP_0 + 2, Some('2'));
        assert_eq!(c.top(), Some(MenuId::Tune));
    }

    #[test]
    fn fire_key_bound_to_a_digit_confirms_initials() {
        let mut c = core_with(Config::default(), None);
        fake_playing(&mut c);
        c.config.keys[4] = key::KEY_0 + 5;
        c.sim.plscore = [0, 0, 0, 1, 2, 0, 0, 0];
        c.game_over(false);
        let phase = |c: &Core| match &c.screen {
            Screen::HighScore(hs) => hs.phase,
            _ => panic!("not on the high score screen"),
        };
        assert_eq!(phase(&c), HsPhase::Entry { letter: 0 });
        digit(&mut c, 5);
        for _ in 0..12 {
            c.tick(0);
        }
        assert_eq!(phase(&c), HsPhase::Entry { letter: 1 }, "the bound Fire key '5' confirms the letter");
        // other digits (and '0') still do not confirm a letter
        digit(&mut c, 3);
        digit(&mut c, 0);
        for _ in 0..12 {
            c.tick(0);
        }
        assert_eq!(phase(&c), HsPhase::Entry { letter: 1 });
    }

    #[test]
    fn in_game_volume_without_music_stays_silent_until_rejoin() {
        let mut c = core_with(Config { sound_mode: 1, ..Config::default() }, None);
        fake_playing(&mut c);
        c.tick(0);
        assert_eq!(music(&mut c), Some(None));
        press(&mut c, key::ESCAPE);
        letter(&mut c, 'o');
        digit(&mut c, 2);
        digit(&mut c, 2);
        assert_eq!(c.top(), Some(MenuId::Volume));
        assert_eq!(music(&mut c), None, "tunevolume starts no music with effects only");
        // music switched on afterwards: the in-game menus stay silent, as in C
        press(&mut c, key::ESCAPE);
        digit(&mut c, 1);
        digit(&mut c, 3);
        assert_eq!(c.config.sound_mode, 2);
        assert_eq!(music(&mut c), None);
        press(&mut c, key::ESCAPE);
        press(&mut c, key::ESCAPE);
        assert_eq!(c.top(), Some(MenuId::Options));
        assert_eq!(music(&mut c), None);
        // rejoin -> mainrestart starts the zone music
        press(&mut c, key::ENTER);
        assert!(matches!(c.screen, Screen::Playing));
        assert_eq!(music(&mut c), Some(Some(MusicTrack::Zone { zone: 0, part: 1 })));
    }

    #[test]
    fn in_game_volume_music_plays_until_rejoin() {
        let mut c = core_with(Config::default(), None);
        fake_playing(&mut c);
        c.tick(0);
        assert_eq!(music(&mut c), Some(Some(MusicTrack::Zone { zone: 0, part: 1 })));
        press(&mut c, key::ESCAPE);
        letter(&mut c, 'o');
        assert_eq!(music(&mut c), Some(None), "escapehandler / in-game options are silent");
        digit(&mut c, 2);
        digit(&mut c, 2);
        assert_eq!(music(&mut c), Some(Some(MusicTrack::Title)), "tunevolume starts the music");
        // nothing stops it in the remaining in-game menus (Tune, Sound, Options, define keys)
        press(&mut c, key::ESCAPE);
        assert_eq!(c.top(), Some(MenuId::Tune));
        assert_eq!(music(&mut c), None);
        digit(&mut c, 1);
        press(&mut c, key::ESCAPE);
        press(&mut c, key::ESCAPE);
        assert_eq!(c.top(), Some(MenuId::Options));
        digit(&mut c, 1);
        digit(&mut c, 1);
        assert!(matches!(c.screen, Screen::DefineKeys { .. }));
        press(&mut c, key::ESCAPE);
        press(&mut c, key::ESCAPE);
        assert_eq!(c.top(), Some(MenuId::Options));
        assert_eq!(music(&mut c), None, "still the title music");
        // rejoin -> mainrestart restarts the zone music
        press(&mut c, key::ENTER);
        assert!(matches!(c.screen, Screen::Playing));
        assert_eq!(music(&mut c), Some(Some(MusicTrack::Zone { zone: 0, part: 1 })));
        // the next visit to the in-game options is silent again
        press(&mut c, key::ESCAPE);
        letter(&mut c, 'o');
        digit(&mut c, 2);
        assert_eq!(music(&mut c), Some(None));
    }
}
