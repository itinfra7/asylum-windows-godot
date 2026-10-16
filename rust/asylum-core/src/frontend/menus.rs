//! Data-driven list menus (menus.c `options_menu`, `choosecontrol`, `tunegame`, `tunesound`,
//! `tunevolume`, `tunespeed`, `getzone`, `choosestick`) and their `readopt` style input.
//!
//! Every menu is a list of [`Entry`]s: static texts plus selectable items. Positions are the
//! original 320x256-space content coordinates (shifted by `lay.hx` horizontally and `lay.cy`
//! vertically, which `Builder::push` adds); numbered items are
//! `"<n>. <text>"` with `Align::Orig` exactly like the original single C strings, titles and
//! footers use `Align::EnCentered` (English pixel-identical, other languages centred on the
//! English text), new texts are centred on the 320 band.

use crate::config::Config;
use crate::display;
use crate::i18n::{keys, Lang, Strings};
use crate::render::MenuLine;
use crate::sim::{Align, TextRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    /// options_menu(gameon)
    Options,
    /// choosecontrol (+ current bindings)
    Controls,
    /// choosestick reminder screen
    PadInfo,
    /// tunegame
    Tune,
    /// tunesound
    Sound,
    /// tunevolume
    Volume,
    /// tunespeed
    Video,
    /// new: language choice
    Language,
    /// getzone
    Zone,
    /// getzone "To play the id ..." screen
    IdLock,
    /// new: quit confirmation
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoOpt {
    Smooth,
    Vsync,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolOp {
    FxUp,
    FxDown,
    MusicUp,
    MusicDown,
}

/// What activating an item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// "Fire - Play" (title) / "Fire - Return" (in game).
    Play,
    Open(MenuId),
    DefineKeys,
    DefaultKeys,
    Sound(u8),
    Volume(VolOp),
    Video(VideoOpt),
    /// Next display mode (window → borderless window → full screen).
    DisplayMode,
    /// Next window aspect (4:3 .. 21:9; window mode only).
    Aspect,
    /// Next window resolution of the aspect (window mode only), like tunespeed's size option.
    Resolution,
    /// Next view scale (1.0x .. 3.0x, as far as the resolution allows).
    ScreenScale,
    Lang(Lang),
    Zone(u8),
    SavePosition,
    Back,
    QuitYes,
}

/// One line of a menu; selectable when `act` is set.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub line: MenuLine,
    pub act: Option<Act>,
    /// Number key activating the item (readopt).
    pub digit: Option<u8>,
}

/// Abstract menu input (keys, gamepad and mouse are mapped to this by the core).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Nav {
    None,
    Up,
    Down,
    /// Enter / gamepad A: the selected item.
    Activate,
    /// Bound Fire key / gamepad Start (readopt returning 0).
    Fire,
    /// ESC / right click / gamepad B (readopt returning -1).
    Back,
    Digit(u8),
    /// Mouse position in content coordinates.
    Hover(f32, f32),
    Click(f32, f32),
}

/// State needed to build the entries.
pub struct MenuCtx<'a> {
    pub strings: &'a Strings,
    pub lang: Lang,
    pub config: &'a Config,
    pub hx: i32,
    /// Vertical offset of the original 256-line band (`Layout::cy`).
    pub cy: i32,
    /// Monitor size in pixels (resolution lists, see `display`).
    pub monitor: (i32, i32),
    pub gameon: bool,
    pub save_exists: bool,
    /// "Position Saved" is visible.
    pub saved_flash: bool,
}

const TICK: char = '✓';
const CROSS: char = '✗';
/// Advance of the tick/cross prefix (original glyphs 15/16).
const PREFIX_ADVANCE: i32 = 14;
/// Right end of setting values (320-space; the highlight bar ends at 298).
const VALUE_RIGHT: i32 = 296;

struct Builder<'a, 'b> {
    c: &'a MenuCtx<'b>,
    v: Vec<Entry>,
}

impl Builder<'_, '_> {
    fn s(&self, key: &str) -> String {
        self.c.strings.get(self.c.lang, key).to_string()
    }
    /// A static line at 320-space (x, y); the helpers below make it selectable.
    fn push(&mut self, text: TextRef, x: i32, y: i32, align: Align) -> &mut Entry {
        let line = MenuLine { text, x: (x + self.c.hx) as f32, y: (y + self.c.cy) as f32, align, prefix: None };
        let i = self.v.len();
        self.v.push(Entry { line, act: None, digit: None });
        &mut self.v[i]
    }
    fn numbered_text(&self, n: u8, key: &str) -> TextRef {
        TextRef::Raw(format!("{n}. {}", self.s(key)))
    }
    /// Static original text at its original position.
    fn title(&mut self, key: &'static str, x: i32, y: i32) {
        self.push(TextRef::Key(key), x, y, Align::EnCentered);
    }
    /// Static left-aligned text.
    fn left(&mut self, text: TextRef, x: i32, y: i32) {
        self.push(text, x, y, Align::Orig);
    }
    /// New text centred on the 320 band.
    fn center(&mut self, text: TextRef, y: i32, act: Option<Act>) {
        let cx = (160 + self.c.hx) as f32;
        self.push(text, 160, y, Align::Center(cx)).act = act;
    }
    /// `"<n>. <text>"`, selectable.
    fn item(&mut self, n: u8, key: &'static str, x: i32, y: i32, act: Act) {
        let t = self.numbered_text(n, key);
        let e = self.push(t, x, y, Align::Orig);
        e.act = Some(act);
        e.digit = Some(n);
    }
    /// `"<n>. <text>"`, not selectable (reminder screen).
    fn numbered(&mut self, n: u8, key: &'static str, x: i32, y: i32) {
        let t = self.numbered_text(n, key);
        self.push(t, x, y, Align::Orig);
    }
    /// Tick/cross item like the original `"-1. Full Screen"` whose first char is replaced.
    fn toggle(&mut self, n: u8, key: &'static str, x: i32, y: i32, on: bool, act: Act) {
        self.item(n, key, x, y, act);
        if let Some(e) = self.v.last_mut() {
            e.line.prefix = Some(if on { TICK } else { CROSS });
        }
    }
    /// `"<n>. <label>"` among toggles with its value right-aligned at the end of the row;
    /// the number lines up with the toggles' numbers. Not selectable without `act`.
    fn setting(&mut self, n: u8, key: &'static str, value: TextRef, x: i32, y: i32, act: Option<Act>) {
        let t = self.numbered_text(n, key);
        let e = self.push(t, x + PREFIX_ADVANCE, y, Align::Orig);
        e.act = act;
        e.digit = act.map(|_| n);
        let rx = (VALUE_RIGHT + self.c.hx) as f32;
        self.push(value, VALUE_RIGHT, y, Align::Right(rx));
    }
    /// `"<n>. <text>"` among toggles (the original `" 3.  640 x  512"` row).
    fn value_row(&mut self, n: u8, text: String, x: i32, y: i32, act: Option<Act>) {
        let e = self.push(TextRef::Raw(format!("{n}. {text}")), x + PREFIX_ADVANCE, y, Align::Orig);
        e.act = act;
        e.digit = act.map(|_| n);
    }
    /// Selectable footer ("Fire - Play", "ESC - Exit").
    fn footer(&mut self, key: &'static str, x: i32, y: i32, act: Act) {
        self.push(TextRef::Key(key), x, y, Align::EnCentered).act = Some(act);
    }
}

const BINDING_LABELS: [&str; 5] = [keys::KEY_LEFT, keys::KEY_RIGHT, keys::KEY_UP, keys::KEY_DOWN, keys::KEY_FIRE];
/// Display mode names (`display::MODE_*`).
const MODE_KEYS: [&str; 3] = [keys::VIDEO_MODE_WINDOW, keys::VIDEO_MODE_BORDERLESS, keys::VIDEO_MODE_FULLSCREEN];
/// Tune Video rows: y of the first, pitch (the original tunespeed spacing).
const VIDEO_Y: i32 = 72;
const VIDEO_PITCH: i32 = 24;
const LANG_ITEMS: [(&str, Lang); 3] = [(keys::LANG_EN, Lang::En), (keys::LANG_KO, Lang::Ko), (keys::LANG_ZH, Lang::ZhCn)];

pub fn video_value(c: &Config, o: VideoOpt) -> bool {
    match o {
        VideoOpt::Smooth => c.smooth_motion,
        VideoOpt::Vsync => c.vsync,
    }
}

pub fn video_toggle(c: &mut Config, o: VideoOpt) {
    let v = match o {
        VideoOpt::Smooth => &mut c.smooth_motion,
        VideoOpt::Vsync => &mut c.vsync,
    };
    *v = !*v;
}

/// The lines of menu `id`.
pub fn entries(id: MenuId, c: &MenuCtx) -> Vec<Entry> {
    let mut b = Builder { c, v: Vec::with_capacity(16) };
    let cfg = c.config;
    match id {
        MenuId::Options => {
            b.title(keys::MENU_OPTIONS, 128, 48);
            b.item(1, keys::MENU_DEFINE_CONTROLS, 32, 96, Act::Open(MenuId::Controls));
            b.item(2, keys::MENU_TUNE_GAME, 32, 128, Act::Open(MenuId::Tune));
            if c.gameon {
                b.item(3, keys::MENU_SAVE_POSITION, 32, 160, Act::SavePosition);
                if c.saved_flash {
                    b.center(TextRef::Key(keys::MENU_POSITION_SAVED), 192, None);
                }
                b.footer(keys::MENU_FIRE_RETURN, 88, 224, Act::Play);
            } else {
                b.item(3, keys::MENU_CHOOSE_ZONE, 32, 160, Act::Open(MenuId::Zone));
                b.footer(keys::MENU_FIRE_PLAY, 88, 224, Act::Play);
            }
        }
        MenuId::Controls => {
            b.title(keys::CTRL_TITLE, 96, 48);
            b.item(1, keys::CTRL_KEYBOARD, 64, 72, Act::DefineKeys);
            b.item(2, keys::CTRL_GAMEPAD, 64, 96, Act::Open(MenuId::PadInfo));
            b.item(3, keys::CTRL_DEFAULTS, 64, 120, Act::DefaultKeys);
            for (i, label) in BINDING_LABELS.iter().enumerate() {
                let y = 148 + 18 * i as i32;
                b.left(TextRef::Key(label), 48, y);
                b.left(TextRef::Raw(crate::input::key_name(cfg.keys[i])), 152, y);
            }
        }
        MenuId::PadInfo => {
            b.title(keys::PAD_TITLE, 96, 48);
            b.title(keys::PAD_JUMP, 48, 96);
            b.numbered(1, keys::PAD_JUMP_1, 32, 128);
            b.numbered(2, keys::PAD_JUMP_2, 32, 160);
            b.left(TextRef::Key(keys::PAD_FIRE), 32, 192);
            b.left(TextRef::Key(keys::PAD_MENU), 32, 212);
        }
        MenuId::Tune => {
            b.title(keys::TUNE_TITLE, 96, 48);
            b.item(1, keys::TUNE_SOUND, 64, 96, Act::Open(MenuId::Sound));
            b.item(2, keys::TUNE_VOLUME, 64, 128, Act::Open(MenuId::Volume));
            b.item(3, keys::TUNE_VIDEO, 64, 160, Act::Open(MenuId::Video));
            b.item(4, keys::TUNE_LANGUAGE, 64, 192, Act::Open(MenuId::Language));
        }
        MenuId::Sound => {
            b.title(keys::SOUND_TITLE, 96, 48);
            let items = [keys::SOUND_NONE, keys::SOUND_FX, keys::SOUND_FX_MUSIC];
            for (i, k) in items.iter().enumerate() {
                let m = i as u8;
                b.toggle(m + 1, k, 32, 96 + 32 * i as i32, cfg.sound_mode == m, Act::Sound(m));
            }
            b.footer(keys::MENU_ESC_EXIT, 96, 220, Act::Back);
        }
        MenuId::Volume => {
            b.title(keys::VOL_TITLE, 80, 32);
            b.item(1, keys::VOL_FX_UP, 48, 96, Act::Volume(VolOp::FxUp));
            b.item(2, keys::VOL_FX_DOWN, 48, 116, Act::Volume(VolOp::FxDown));
            b.item(3, keys::VOL_MUSIC_UP, 48, 136, Act::Volume(VolOp::MusicUp));
            b.item(4, keys::VOL_MUSIC_DOWN, 48, 156, Act::Volume(VolOp::MusicDown));
            let levels = format!(
                "{} {}    {} {}",
                b.s(keys::VOL_FX_LEVEL),
                cfg.sfx_volume,
                b.s(keys::VOL_MUSIC_LEVEL),
                cfg.music_volume
            );
            b.center(TextRef::Raw(levels), 188, None);
            b.footer(keys::MENU_ESC_EXIT, 96, 220, Act::Back);
        }
        MenuId::Video => {
            // mode, aspect (window only), resolution (window only; the monitor's otherwise),
            // view scale, then the toggles
            b.title(keys::VIDEO_TITLE, 96, 48);
            let v = display::video(cfg, c.monitor);
            let window = cfg.display_mode == display::MODE_WINDOW;
            let y = |i: i32| VIDEO_Y + VIDEO_PITCH * i;
            let mode = TextRef::Key(MODE_KEYS[cfg.display_mode.min(display::MODE_FULLSCREEN) as usize]);
            b.setting(1, keys::VIDEO_MODE, mode, 32, y(0), Some(Act::DisplayMode));
            let aspect = match v.aspect {
                Some(a) => TextRef::Raw(display::aspect_text(a)),
                None => TextRef::Key(keys::VIDEO_ASPECT_MONITOR),
            };
            b.setting(2, keys::VIDEO_ASPECT, aspect, 32, y(1), window.then_some(Act::Aspect));
            b.value_row(3, display::resolution_text(v.resolution), 32, y(2), window.then_some(Act::Resolution));
            let scale = TextRef::Raw(format!("{}{}", display::scale_text(v.scale), b.s(keys::VIDEO_SCALE_UNIT)));
            b.setting(4, keys::VIDEO_SCALE, scale, 32, y(3), Some(Act::ScreenScale));
            for (n, k, o) in [(5, keys::VIDEO_SMOOTH, VideoOpt::Smooth), (6, keys::VIDEO_VSYNC, VideoOpt::Vsync)] {
                b.toggle(n, k, 32, y(n as i32 - 1), video_value(cfg, o), Act::Video(o));
            }
            b.footer(keys::MENU_ESC_EXIT, 96, 220, Act::Back);
        }
        MenuId::Language => {
            b.title(keys::LANG_TITLE, 96, 48);
            for (i, (k, l)) in LANG_ITEMS.iter().enumerate() {
                b.toggle(i as u8 + 1, k, 32, 96 + 32 * i as i32, c.lang == *l, Act::Lang(*l));
            }
            b.footer(keys::MENU_ESC_EXIT, 96, 220, Act::Back);
        }
        MenuId::Zone => {
            b.title(keys::ZONE_TITLE_1, 88, 48);
            b.title(keys::ZONE_TITLE_2, 96, 68);
            b.item(1, keys::ZONE_EGO, 64, 96, Act::Zone(1));
            b.item(2, keys::ZONE_PSYCHE, 64, 128, Act::Zone(2));
            b.item(3, keys::ZONE_ID, 64, 160, Act::Zone(3));
            if c.save_exists {
                b.item(4, keys::ZONE_SAVED, 64, 192, Act::Zone(4));
            }
        }
        MenuId::IdLock => {
            b.title(keys::ZONE_ID_LOCK_1, 80, 100);
            b.title(keys::ZONE_ID_LOCK_2, 32, 128);
            b.title(keys::ZONE_ID_LOCK_3, 80, 156);
        }
        MenuId::Quit => {
            b.center(TextRef::Key(keys::MENU_QUIT_TITLE), 96, None);
            b.center(TextRef::Key(keys::MENU_QUIT_YES), 144, Some(Act::QuitYes));
            b.center(TextRef::Key(keys::MENU_QUIT_NO), 176, Some(Act::Back));
        }
    }
    b.v
}

/// What Fire (and digit 0, like readopt returning 0) does; `None` = ignored (tunegame,
/// tunespeed loop on 0).
pub fn fire_act(id: MenuId) -> Option<Act> {
    match id {
        MenuId::Options => Some(Act::Play),
        MenuId::Tune | MenuId::Video => None,
        MenuId::Quit => Some(Act::QuitYes),
        // choosecontrol / tunesound / tunevolume return on 0; getzone's
        // `mentalzone = 0` on Fire is a C bug and becomes "back".
        _ => Some(Act::Back),
    }
}

/// readopt(maxopt) of the info screens: getzone id lock = readopt(0), choosestick reminder = readopt(9).
pub fn info_maxopt(id: MenuId) -> u8 {
    if id == MenuId::IdLock {
        0
    } else {
        9
    }
}

/// readopt tests the digit range before the Fire key (`osbyte_81(firekey)`): on a screen
/// without selectable entries, a digit above `maxopt` that is also the bound Fire key
/// (`is_fire_key`) answers as Fire.
pub fn fire_after_digit(e: &[Entry], nav: Nav, maxopt: u8, is_fire_key: bool) -> Nav {
    match nav {
        Nav::Digit(d) if d > maxopt && is_fire_key && selectable(e).is_empty() => Nav::Fire,
        _ => nav,
    }
}

/// Initially selected item (index among the selectable entries).
pub fn default_sel(id: MenuId, c: &MenuCtx, mentalzone: u8) -> usize {
    let n = selectable(&entries(id, c)).len();
    let s = match id {
        // the "Fire - Play" footer, so Enter behaves like the original Fire
        MenuId::Options => n.saturating_sub(1),
        MenuId::Sound => c.config.sound_mode as usize,
        MenuId::Language => c.lang as usize,
        MenuId::Zone => (mentalzone.max(1) - 1) as usize,
        _ => 0,
    };
    s.min(n.saturating_sub(1))
}

/// Indices of the selectable entries.
pub fn selectable(e: &[Entry]) -> Vec<usize> {
    e.iter().enumerate().filter(|(_, x)| x.act.is_some()).map(|(i, _)| i).collect()
}

/// Mouse row hit test (content coordinates): the nearest row within 10 px (rows closer than
/// 20 px have overlapping bands; the upper one wins an exact tie).
pub fn hit_row(y_rows: impl Iterator<Item = f32>, x: f32, y: f32, cw: i32) -> Option<usize> {
    if x < 20.0 || x >= (cw - 20) as f32 {
        return None;
    }
    y_rows
        .enumerate()
        .map(|(i, ry)| (i, (y - ry).abs()))
        .filter(|&(_, d)| d <= 10.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Highlight bar (content rect) over a line at `y`.
pub fn highlight_rect(hx: i32, y: f32) -> [f32; 4] {
    [(hx + 22) as f32, y - 9.0, (hx + 298) as f32, y + 9.0]
}

/// Applies `nav` to a menu: moves `sel` and returns the activated action, if any.
/// `maxopt` is the readopt range of a screen without selectable entries ([`info_maxopt`]);
/// menus with selectable entries ignore it.
pub fn navigate(e: &[Entry], sel: &mut usize, fire: Option<Act>, nav: Nav, cw: i32, maxopt: u8) -> Option<Act> {
    let sels = selectable(e);
    if sels.is_empty() {
        // readopt(maxopt) info screens: '0'..='0'+maxopt, Fire or ESC return (plus Enter and
        // click); the id lock (readopt(0)) ignores '1'..'9'
        return match nav {
            Nav::Activate | Nav::Fire | Nav::Back | Nav::Click(..) => Some(Act::Back),
            Nav::Digit(d) if d <= maxopt => Some(Act::Back),
            _ => None,
        };
    }
    let n = sels.len();
    if *sel >= n {
        *sel = n - 1;
    }
    let rows = || sels.iter().map(|&i| e[i].line.y);
    match nav {
        Nav::None => None,
        Nav::Up => {
            *sel = (*sel + n - 1) % n;
            None
        }
        Nav::Down => {
            *sel = (*sel + 1) % n;
            None
        }
        Nav::Activate => e[sels[*sel]].act,
        Nav::Fire | Nav::Digit(0) => fire,
        Nav::Back => Some(Act::Back),
        // readopt: direct activation, the highlight stays where it is
        Nav::Digit(d) => sels.iter().find(|&&i| e[i].digit == Some(d)).and_then(|&i| e[i].act),
        Nav::Hover(x, y) => {
            if let Some(k) = hit_row(rows(), x, y, cw) {
                *sel = k;
            }
            None
        }
        Nav::Click(x, y) => {
            let k = hit_row(rows(), x, y, cw)?;
            *sel = k;
            e[sels[k]].act
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(strings: &'a Strings, config: &'a Config, gameon: bool) -> MenuCtx<'a> {
        MenuCtx { strings, lang: Lang::En, config, hx: 0, cy: 0, monitor: (2560, 1080), gameon, save_exists: false, saved_flash: false }
    }

    #[test]
    fn options_layout_matches_original() {
        let s = Strings::default();
        let cfg = Config::default();
        let e = entries(MenuId::Options, &ctx(&s, &cfg, false));
        let pos: Vec<(f32, f32)> = e.iter().map(|x| (x.line.x, x.line.y)).collect();
        assert_eq!(pos, vec![(128.0, 48.0), (32.0, 96.0), (32.0, 128.0), (32.0, 160.0), (88.0, 224.0)]);
        assert_eq!(e[0].line.align, Align::EnCentered);
        assert_eq!(e[1].line.align, Align::Orig);
        assert_eq!(e[1].line.text, TextRef::Raw("1. MENU_DEFINE_CONTROLS".into()));
        assert_eq!(e[4].line.text, TextRef::Key(keys::MENU_FIRE_PLAY));
        let g = entries(MenuId::Options, &ctx(&s, &cfg, true));
        assert_eq!(g[3].act, Some(Act::SavePosition));
        assert_eq!(g[4].line.text, TextRef::Key(keys::MENU_FIRE_RETURN));
    }

    #[test]
    fn every_menu_fits_the_chat_window() {
        let s = Strings::default();
        let cfg = Config { id_permit: true, ..Config::default() };
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
        for gameon in [false, true] {
            for id in ids {
                let mut c = ctx(&s, &cfg, gameon);
                c.save_exists = true;
                c.saved_flash = true;
                let e = entries(id, &c);
                let sels = selectable(&e);
                for w in sels.windows(2) {
                    assert!(e[w[1]].line.y - e[w[0]].line.y >= 18.0, "{id:?}: highlight bars overlap");
                }
                for x in &e {
                    assert!(x.line.y - 7.0 >= 20.0 && x.line.y + 7.0 <= 235.0, "{id:?} line at {}", x.line.y);
                }
            }
        }
    }

    #[test]
    fn toggles_show_tick_and_cross() {
        let s = Strings::default();
        let cfg = Config { sound_mode: 1, vsync: false, ..Config::default() };
        let e = entries(MenuId::Sound, &ctx(&s, &cfg, false));
        let p: Vec<_> = e.iter().filter_map(|x| x.line.prefix).collect();
        assert_eq!(p, vec!['\u{2717}', '\u{2713}', '\u{2717}']);
        let v = entries(MenuId::Video, &ctx(&s, &cfg, false));
        let p: Vec<_> = v.iter().filter_map(|x| x.line.prefix).collect();
        assert_eq!(p, vec!['\u{2713}', '\u{2717}'], "smooth motion on, vsync off");
    }

    #[test]
    fn video_menu_rows() {
        let s = Strings::default();
        // borderless: aspect and resolution follow the monitor and are not selectable
        let cfg = Config::default();
        let v = entries(MenuId::Video, &ctx(&s, &cfg, false));
        type Row = (f32, f32, Option<Act>, Option<u8>, TextRef);
        let rows: Vec<Row> = v.iter().map(|e| (e.line.x, e.line.y, e.act, e.digit, e.line.text.clone())).collect();
        let key = TextRef::Key;
        let raw = |t: &str| TextRef::Raw(t.into());
        let t = |o| Some(Act::Video(o));
        assert_eq!(
            rows,
            vec![
                (96.0, 48.0, None, None, key(keys::VIDEO_TITLE)),
                (46.0, 72.0, Some(Act::DisplayMode), Some(1), raw("1. VIDEO_MODE")),
                (296.0, 72.0, None, None, key(keys::VIDEO_MODE_BORDERLESS)),
                (46.0, 96.0, None, None, raw("2. VIDEO_ASPECT")),
                (296.0, 96.0, None, None, key(keys::VIDEO_ASPECT_MONITOR)),
                (46.0, 120.0, None, None, raw("3. 2560 x 1080")),
                (46.0, 144.0, Some(Act::ScreenScale), Some(4), raw("4. VIDEO_SCALE")),
                (296.0, 144.0, None, None, raw("1.0VIDEO_SCALE_UNIT")),
                (32.0, 168.0, t(VideoOpt::Smooth), Some(5), raw("5. VIDEO_SMOOTH")),
                (32.0, 192.0, t(VideoOpt::Vsync), Some(6), raw("6. VIDEO_VSYNC")),
                (96.0, 220.0, Some(Act::Back), None, key(keys::MENU_ESC_EXIT)),
            ]
        );
        assert_eq!(v[2].line.align, Align::Right(296.0));
        // the numbers of the setting rows start where the toggles' numbers start
        assert_eq!(PREFIX_ADVANCE, crate::font::original_advance(crate::font::original_glyph(TICK).unwrap()));
        assert_eq!(PREFIX_ADVANCE, crate::font::original_advance(crate::font::original_glyph(CROSS).unwrap()));
        // window mode: the chosen aspect and resolution, both selectable
        let cfg = Config { display_mode: display::MODE_WINDOW, window_aspect: 3, window_resolution: (1920, 800), screen_scale: 6, ..Config::default() };
        let v = entries(MenuId::Video, &ctx(&s, &cfg, false));
        assert_eq!((v[3].act, v[3].digit, &v[4].line.text), (Some(Act::Aspect), Some(2), &raw("21:9")));
        assert_eq!((v[5].act, v[5].digit, &v[5].line.text), (Some(Act::Resolution), Some(3), &raw("3. 1920 x 800")));
        assert_eq!(v[7].line.text, raw("3.0VIDEO_SCALE_UNIT"));
        // digits select their rows
        let c = ctx(&s, &cfg, false);
        let mut sel = default_sel(MenuId::Video, &c, 7);
        for (d, act) in [(1, Act::DisplayMode), (2, Act::Aspect), (3, Act::Resolution), (4, Act::ScreenScale), (6, Act::Video(VideoOpt::Vsync))] {
            assert_eq!(navigate(&v, &mut sel, None, Nav::Digit(d), 320, 7), Some(act), "digit {d}");
        }
        assert_eq!(navigate(&v, &mut sel, None, Nav::Digit(7), 320, 7), None);
        // a larger screen centres the menu (cy) and moves the values with the band (hx)
        let mut c = ctx(&s, &cfg, false);
        c.hx = 522;
        c.cy = 256;
        let v = entries(MenuId::Video, &c);
        assert_eq!((v[1].line.x, v[1].line.y, v[2].line.align), (46.0 + 522.0, 72.0 + 256.0, Align::Right(296.0 + 522.0)));
    }

    #[test]
    fn larger_view_shifts_by_hx() {
        let s = Strings::default();
        let cfg = Config::default();
        let mut c = ctx(&s, &cfg, false);
        c.hx = 67;
        let e = entries(MenuId::Quit, &c);
        assert_eq!(e[0].line.align, Align::Center(227.0));
        let o = entries(MenuId::Options, &c);
        assert_eq!(o[1].line.x, 99.0);
    }

    #[test]
    fn navigation_wraps_and_digits_select() {
        let s = Strings::default();
        let cfg = Config::default();
        let c = ctx(&s, &cfg, false);
        let e = entries(MenuId::Options, &c);
        let mut sel = default_sel(MenuId::Options, &c, 1);
        assert_eq!(sel, 3);
        assert_eq!(navigate(&e, &mut sel, Some(Act::Play), Nav::Activate, 320, 9), Some(Act::Play));
        navigate(&e, &mut sel, None, Nav::Down, 320, 9);
        assert_eq!(sel, 0);
        navigate(&e, &mut sel, None, Nav::Up, 320, 9);
        assert_eq!(sel, 3);
        assert_eq!(navigate(&e, &mut sel, Some(Act::Play), Nav::Digit(2), 320, 9), Some(Act::Open(MenuId::Tune)));
        assert_eq!(sel, 3);
        assert_eq!(navigate(&e, &mut sel, Some(Act::Play), Nav::Digit(0), 320, 9), Some(Act::Play));
        assert_eq!(navigate(&e, &mut sel, Some(Act::Play), Nav::Digit(7), 320, 9), None);
        // mouse: hover row 128 then click row 160
        navigate(&e, &mut sel, None, Nav::Hover(100.0, 131.0), 320, 9);
        assert_eq!(sel, 1);
        assert_eq!(navigate(&e, &mut sel, None, Nav::Click(60.0, 158.0), 320, 9), Some(Act::Open(MenuId::Zone)));
        assert_eq!(navigate(&e, &mut sel, None, Nav::Click(60.0, 190.0), 320, 9), None);
        assert_eq!(navigate(&e, &mut sel, None, Nav::Click(5.0, 160.0), 320, 9), None);
        // id lock screen (readopt(0)): only '0', Fire or ESC return (plus Enter and click)
        let info = entries(MenuId::IdLock, &c);
        assert_eq!(navigate(&info, &mut sel, Some(Act::Back), Nav::Digit(0), 320, 0), Some(Act::Back));
        assert_eq!(navigate(&info, &mut sel, Some(Act::Back), Nav::Digit(1), 320, 0), None);
        assert_eq!(navigate(&info, &mut sel, Some(Act::Back), Nav::Down, 320, 0), None);
        // gamepad reminder (readopt(9)): every digit returns
        let pad = entries(MenuId::PadInfo, &c);
        assert_eq!(navigate(&pad, &mut sel, Some(Act::Back), Nav::Digit(9), 320, 9), Some(Act::Back));
    }

    #[test]
    fn id_lock_accepts_only_zero_fire_or_escape() {
        let s = Strings::default();
        let cfg = Config::default();
        let c = ctx(&s, &cfg, false);
        let info = entries(MenuId::IdLock, &c);
        let m = info_maxopt(MenuId::IdLock);
        assert_eq!(m, 0);
        assert_eq!(info_maxopt(MenuId::PadInfo), 9);
        let mut sel = 0;
        for d in 1..=9 {
            assert_eq!(navigate(&info, &mut sel, Some(Act::Back), Nav::Digit(d), 320, m), None, "digit {d}");
        }
        for nav in [Nav::Digit(0), Nav::Fire, Nav::Back, Nav::Activate, Nav::Click(60.0, 128.0)] {
            assert_eq!(navigate(&info, &mut sel, Some(Act::Back), nav, 320, m), Some(Act::Back), "{nav:?}");
        }
        assert_eq!(navigate(&info, &mut sel, Some(Act::Back), Nav::Hover(60.0, 128.0), 320, m), None);
        let pad = entries(MenuId::PadInfo, &c);
        for d in 0..=9 {
            assert_eq!(navigate(&pad, &mut sel, Some(Act::Back), Nav::Digit(d), 320, 9), Some(Act::Back), "pad digit {d}");
        }
    }

    #[test]
    fn fire_bound_to_a_digit_still_closes_the_id_lock() {
        let s = Strings::default();
        let cfg = Config::default();
        let c = ctx(&s, &cfg, false);
        let info = entries(MenuId::IdLock, &c);
        let m = info_maxopt(MenuId::IdLock);
        // readopt: the digit test fails ('5' > '0' + 0), then osbyte_81(firekey) sees Fire
        let nav = fire_after_digit(&info, Nav::Digit(5), m, true);
        assert_eq!(nav, Nav::Fire);
        assert_eq!(navigate(&info, &mut 0, Some(Act::Back), nav, 320, m), Some(Act::Back));
        // a digit that is not the Fire key stays a (rejected) digit
        assert_eq!(fire_after_digit(&info, Nav::Digit(5), m, false), Nav::Digit(5));
        // in range, the digit test wins
        assert_eq!(fire_after_digit(&info, Nav::Digit(0), m, true), Nav::Digit(0));
        // menus with selectable entries are unchanged
        let opts = entries(MenuId::Options, &c);
        assert_eq!(fire_after_digit(&opts, Nav::Digit(5), 0, true), Nav::Digit(5));
        assert_eq!(fire_after_digit(&info, Nav::Down, m, true), Nav::Down);
    }
}
