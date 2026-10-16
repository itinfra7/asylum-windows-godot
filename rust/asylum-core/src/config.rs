//! Persistent settings (`userdata/asylum.cfg`, text `Key=Value` lines, unknown keys ignored).
//! Replaces asylum.c loadconfig/saveconfig (the original `.asylum` file).

use crate::display;
use crate::i18n::Lang;
use crate::input::DEFAULT_KEYS;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// UI language; `None` until chosen (host picks from OS locale on first run).
    pub language: Option<Lang>,
    /// Physical keycodes for Left, Right, Up(jump), Down, Fire.
    pub keys: [i64; 5],
    /// 0 = no sound, 1 = effects only, 2 = effects + music (options.soundtype).
    pub sound_mode: u8,
    /// 0..=10 linear steps (UI), applied as gain (v/10)^2 in the host.
    pub sfx_volume: u8,
    pub music_volume: u8,
    /// `display::MODE_WINDOW`, `MODE_BORDERLESS` or `MODE_FULLSCREEN`.
    pub display_mode: u8,
    /// Window aspect ratio (index into `display::ASPECTS`).
    pub window_aspect: u8,
    /// Window client size in pixels (one of `display::RESOLUTIONS` of the aspect; a value the
    /// monitor cannot show is replaced by `display::pick_resolution`).
    pub window_resolution: (i32, i32),
    /// View scale in half steps: 2 = 1.0x (256 lines, the original view) … 6 = 3.0x; a
    /// higher scale shows more of the game world (see `display`).
    pub screen_scale: u8,
    pub smooth_motion: bool,
    pub vsync: bool,
    /// Last chosen zone 1..=3 (options.mentalzone); 4 (saved game) is never persisted.
    pub mental_zone: u8,
    /// Id unlocked (options.idpermit).
    pub id_permit: bool,
    /// High-score initials (options.initials), Archimedes bytes '0'..'Z'+4.
    pub initials: [u8; 3],
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: None,
            keys: DEFAULT_KEYS,
            sound_mode: 2,
            sfx_volume: 8,
            music_volume: 7,
            display_mode: display::MODE_BORDERLESS,
            window_aspect: display::ASPECT_DEFAULT,
            window_resolution: display::RESOLUTION_DEFAULT,
            screen_scale: display::SCALE_MIN,
            smooth_motion: true,
            vsync: true,
            mental_zone: 1,
            id_permit: false,
            initials: *b"PSY",
        }
    }
}

/// Largest valid Godot keycode (`KEY_CODE_MASK`; modifier bits are never stored).
const KEY_CODE_MAX: i64 = (1 << 23) - 1;
/// Highest initials byte (menus.c updatehst clamps to '0'..='Z'+4).
const INITIAL_MAX: u8 = b'Z' + 4;
const KEY_NAMES: [&str; 5] = ["KeyLeft", "KeyRight", "KeyUp", "KeyDown", "KeyFire"];

fn parse_int<T: std::str::FromStr + PartialOrd>(v: &str, lo: T, hi: T) -> Option<T> {
    v.parse::<T>().ok().filter(|x| *x >= lo && *x <= hi)
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_initials(v: &str) -> Option<[u8; 3]> {
    let b = v.as_bytes();
    if b.len() != 3 {
        return None;
    }
    let mut out = [0u8; 3];
    for (o, &c) in out.iter_mut().zip(b) {
        let c = c.to_ascii_uppercase();
        if !(b'0'..=INITIAL_MAX).contains(&c) {
            return None;
        }
        *o = c;
    }
    Some(out)
}

fn parse_display_mode(v: &str) -> Option<u8> {
    let v = v.to_ascii_lowercase();
    display::MODE_NAMES.iter().position(|n| *n == v).map(|i| i as u8).or_else(|| parse_int(&v, 0, 2))
}

impl Config {
    /// Parse; never fails (bad values fall back to defaults). Enforces the original rule
    /// "if (!idpermit && mentalzone > 2) mentalzone = 2".
    ///
    /// Format: one `Key=Value` per line; keys are case-insensitive, surrounding whitespace,
    /// blank lines, `#`/`;` comments, unknown keys and invalid values are ignored; for
    /// repeated keys the last valid value wins.
    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        // configs written before DisplayMode had `Fullscreen` (a borderless full screen)
        let mut legacy_fullscreen = None;
        let mut display_seen = false;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if let Some(i) = KEY_NAMES.iter().position(|n| n.eq_ignore_ascii_case(&k)) {
                if let Some(code) = parse_int(v, 1, KEY_CODE_MAX) {
                    c.keys[i] = code;
                }
                continue;
            }
            let set_bool = |dst: &mut bool| {
                if let Some(b) = parse_bool(v) {
                    *dst = b;
                }
            };
            match k.as_str() {
                "language" => c.language = Lang::from_code(v).or(c.language),
                "soundmode" => c.sound_mode = parse_int(v, 0, 2).unwrap_or(c.sound_mode),
                "sfxvolume" => c.sfx_volume = parse_int(v, 0, 10).unwrap_or(c.sfx_volume),
                "musicvolume" => c.music_volume = parse_int(v, 0, 10).unwrap_or(c.music_volume),
                "displaymode" => {
                    if let Some(m) = parse_display_mode(v) {
                        c.display_mode = m;
                        display_seen = true;
                    }
                }
                "fullscreen" => legacy_fullscreen = parse_bool(v).or(legacy_fullscreen),
                "windowaspect" => c.window_aspect = display::parse_aspect(v).unwrap_or(c.window_aspect),
                "windowresolution" => c.window_resolution = display::parse_resolution(v).unwrap_or(c.window_resolution),
                "screenscale" => c.screen_scale = display::parse_scale(v).unwrap_or(c.screen_scale),
                "smoothmotion" => set_bool(&mut c.smooth_motion),
                "vsync" => set_bool(&mut c.vsync),
                "mentalzone" => c.mental_zone = parse_int(v, 1, 3).unwrap_or(c.mental_zone),
                "idpermit" => set_bool(&mut c.id_permit),
                "initials" => c.initials = parse_initials(v).unwrap_or(c.initials),
                _ => {}
            }
        }
        if let (false, Some(f)) = (display_seen, legacy_fullscreen) {
            c.display_mode = if f { display::MODE_BORDERLESS } else { display::MODE_WINDOW };
        }
        // asylum.c loadconfig, applied after the whole file like the original.
        if !c.id_permit && c.mental_zone > 2 {
            c.mental_zone = 2;
        }
        c
    }

    pub fn serialize(&self) -> String {
        use std::fmt::Write;
        let b = |v: bool| if v { 1 } else { 0 };
        let mut s = String::new();
        if let Some(lang) = self.language {
            let _ = writeln!(s, "Language={}", lang.code());
        }
        for (name, code) in KEY_NAMES.iter().zip(self.keys) {
            let _ = writeln!(s, "{name}={code}");
        }
        // Zone 4 (saved game) is not a startup zone.
        let zone = if (1..=3).contains(&self.mental_zone) { self.mental_zone } else { 1 };
        let initials = match parse_initials(std::str::from_utf8(&self.initials).unwrap_or("")) {
            Some(i) => i,
            None => Config::default().initials,
        };
        let d = Config::default();
        let scale = if (display::SCALE_MIN..=display::SCALE_MAX).contains(&self.screen_scale) { self.screen_scale } else { d.screen_scale };
        let res = display::parse_resolution(&format!("{}x{}", self.window_resolution.0, self.window_resolution.1)).unwrap_or(d.window_resolution);
        let _ = writeln!(s, "SoundMode={}", self.sound_mode);
        let _ = writeln!(s, "SfxVolume={}", self.sfx_volume);
        let _ = writeln!(s, "MusicVolume={}", self.music_volume);
        let _ = writeln!(s, "DisplayMode={}", display::MODE_NAMES[self.display_mode.min(display::MODE_FULLSCREEN) as usize]);
        let _ = writeln!(s, "WindowAspect={}", display::aspect_text(self.window_aspect));
        let _ = writeln!(s, "WindowResolution={}x{}", res.0, res.1);
        let _ = writeln!(s, "ScreenScale={}", display::scale_text(scale));
        let _ = writeln!(s, "SmoothMotion={}", b(self.smooth_motion));
        let _ = writeln!(s, "Vsync={}", b(self.vsync));
        let _ = writeln!(s, "MentalZone={zone}");
        let _ = writeln!(s, "IdPermit={}", b(self.id_permit));
        let _ = writeln!(s, "Initials={}", initials.iter().map(|&c| c as char).collect::<String>());
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::key;

    #[test]
    fn round_trip() {
        let c = Config {
            language: Some(Lang::ZhCn),
            keys: [key::A, key::A + 3, key::SPACE, key::DOWN, key::KP_0 + 5],
            sound_mode: 1,
            sfx_volume: 0,
            music_volume: 10,
            display_mode: display::MODE_FULLSCREEN,
            window_aspect: 3,
            window_resolution: (2560, 1080),
            screen_scale: 3,
            smooth_motion: false,
            vsync: false,
            mental_zone: 3,
            id_permit: true,
            initials: *b"A^0",
        };
        assert_eq!(Config::parse(&c.serialize()), c);
        let d = Config::default();
        assert_eq!(Config::parse(&d.serialize()), d);
        for lang in Lang::ALL {
            let c = Config { language: Some(lang), ..Config::default() };
            assert_eq!(Config::parse(&c.serialize()), c);
        }
    }

    #[test]
    fn serialized_format() {
        let s = Config { language: Some(Lang::Ko), ..Config::default() }.serialize();
        assert!(s.starts_with("Language=ko\nKeyLeft=90\nKeyRight=88\nKeyUp=59\nKeyDown=46\nKeyFire=4194309\n"));
        assert!(s.contains("\nDisplayMode=borderless\nWindowAspect=16:9\nWindowResolution=1280x720\nScreenScale=1.0\n"));
        assert!(s.contains("\nInitials=PSY\n"));
        assert!(!Config::default().serialize().contains("Language"));
    }

    #[test]
    fn mental_zone_rules() {
        assert_eq!(Config::parse("MentalZone=3").mental_zone, 2);
        assert_eq!(Config::parse("MentalZone=3\nIdPermit=1").mental_zone, 3);
        // Order independent, like the check after the C loop.
        assert_eq!(Config::parse("IdPermit=1\nMentalZone=3").mental_zone, 3);
        assert_eq!(Config::parse("MentalZone=2").mental_zone, 2);
        assert_eq!(Config::parse("MentalZone=4\nIdPermit=1").mental_zone, 1);
        assert_eq!(Config::parse("MentalZone=0").mental_zone, 1);
        let c = Config { mental_zone: 4, id_permit: true, ..Config::default() };
        assert_eq!(Config::parse(&c.serialize()).mental_zone, 1);
    }

    #[test]
    fn robust_to_garbage() {
        let d = Config::default();
        assert_eq!(Config::parse(""), d);
        assert_eq!(Config::parse("\u{0}\u{1}\u{ffff}garbage\n===\n=\n=5\nNoSuchKey=1\n"), d);
        let bad = "Language=fr\nKeyLeft=abc\nKeyRight=-5\nKeyUp=0\nKeyDown=99999999999\nKeyFire=\n\
                   SoundMode=7\nSfxVolume=-1\nMusicVolume=11\nDisplayMode=3\nDisplayMode=max\nVsync=maybe\n\
                   Initials=toolong\nScreenScale=1.25\nScreenScale=3.5\nScreenScale=NaN\nWindowAspect=5:4\n\
                   WindowAspect=wide\nWindowResolution=0x0\nWindowResolution=abc\nMentalZone=x\n";
        assert_eq!(Config::parse(bad), d);
        assert_eq!(Config::parse("Initials=a b").initials, d.initials);
        assert_eq!(Config::parse("Initials=___").initials, d.initials);
    }

    #[test]
    fn lenient_syntax() {
        let c = Config::parse(
            "\u{feff}  language = KO \r\n# comment\r\n;KeyLeft=65\r\nkeyleft=65\r\nDISPLAYMODE = Window\r\n\
             Initials=abc\r\nSfxVolume=3\r\nSfxVolume=oops\r\nwindowresolution = 1440X1080\r\n",
        );
        assert_eq!(c.language, Some(Lang::Ko));
        assert_eq!(c.keys[0], key::A);
        assert_eq!(c.display_mode, display::MODE_WINDOW);
        assert_eq!(c.window_resolution, (1440, 1080));
        assert_eq!(&c.initials, b"ABC");
        assert_eq!(c.sfx_volume, 3);
        assert_eq!(Config::parse("Language=zh_CN").language, Some(Lang::ZhCn));
        assert_eq!(Config::parse("Language=en").language, Some(Lang::En));
    }

    #[test]
    fn display_settings() {
        for (text, m) in [("window", 0), ("Borderless", 1), ("FULLSCREEN", 2), ("0", 0), ("2", 2)] {
            assert_eq!(Config::parse(&format!("DisplayMode={text}")).display_mode, m, "{text}");
        }
        assert_eq!(Config::parse("WindowAspect=21:9").window_aspect, 3);
        assert_eq!(Config::parse("ScreenScale=2.5").screen_scale, 5);
        // older configs: Fullscreen=1 was a borderless full screen, 0 a window; DisplayMode wins
        assert_eq!(Config::parse("Fullscreen=1").display_mode, display::MODE_BORDERLESS);
        assert_eq!(Config::parse("Fullscreen=0").display_mode, display::MODE_WINDOW);
        assert_eq!(Config::parse("Fullscreen=1\nDisplayMode=window").display_mode, display::MODE_WINDOW);
        assert_eq!(Config::parse("DisplayMode=fullscreen\nFullscreen=0").display_mode, display::MODE_FULLSCREEN);
        // settings of older versions are ignored
        assert_eq!(Config::parse("WideView=0\nMaximized=1\nIntegerScaling=1\nPixelPerfect=1\nWindowW=1365"), Config::default());
        // invalid in-memory values are never written
        let s = Config { display_mode: 7, window_aspect: 9, window_resolution: (-1, 0), screen_scale: 9, ..Config::default() }.serialize();
        assert!(s.contains("\nDisplayMode=fullscreen\nWindowAspect=21:9\nWindowResolution=1280x720\nScreenScale=1.0\n"), "{s}");
    }
}
