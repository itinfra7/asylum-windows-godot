//! Localisation: `game/locale/strings.csv` (UTF-8, header `key,en,ko,zh_CN`).
//!
//! English strings reproduce the original texts. Every key used by the code is declared in
//! [`keys`] and listed in [`keys::ALL`]; a unit test checks the CSV against that list.

use std::collections::{BTreeSet, HashMap};

use crate::font::Flavor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    En = 0,
    Ko = 1,
    ZhCn = 2,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Ko, Lang::ZhCn];

    /// Config / CSV code.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::ZhCn => "zh_CN",
        }
    }

    pub fn from_code(s: &str) -> Option<Lang> {
        let s = s.to_ascii_lowercase();
        if s.starts_with("ko") {
            Some(Lang::Ko)
        } else if s.starts_with("zh") {
            Some(Lang::ZhCn)
        } else if s.starts_with("en") {
            Some(Lang::En)
        } else {
            None
        }
    }

    /// CJK glyph shapes to use for characters the original font lacks.
    pub fn flavor(self) -> Flavor {
        match self {
            Lang::ZhCn => Flavor::ZhHans,
            _ => Flavor::Ko,
        }
    }
}

/// Parsed string table.
#[derive(Clone, Debug, Default)]
pub struct Strings {
    /// Per language: key -> text (`\n` in the CSV cell text stays a real newline).
    pub tables: [HashMap<String, String>; 3],
}

impl Strings {
    /// Parse the CSV (RFC 4180: quoted fields, `""` escapes, embedded newlines/commas).
    /// The literal two-character sequence `\n` inside a cell is converted to a newline.
    /// Columns are found by header name; blank lines, CRLF and a UTF-8 BOM are tolerated.
    /// Errors: missing header columns, unterminated quotes, text after a closing quote,
    /// rows without a key, duplicate keys.
    pub fn parse_csv(text: &str) -> Result<Strings, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut records = parse_records(text)?.into_iter();
        let (_, header) = records.next().ok_or_else(|| "strings.csv: empty file".to_string())?;
        let column = |name: &str| {
            header
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("strings.csv: header has no '{name}' column"))
        };
        let key_col = column("key")?;
        let mut lang_cols = [0usize; 3];
        for lang in Lang::ALL {
            lang_cols[lang as usize] = column(lang.code())?;
        }

        let mut s = Strings::default();
        for (line, rec) in records {
            if rec.iter().all(|f| f.is_empty()) {
                continue;
            }
            let key = rec.get(key_col).map(|k| k.trim()).unwrap_or("");
            if key.is_empty() {
                return Err(format!("strings.csv line {line}: row without a key"));
            }
            if s.tables[Lang::En as usize].contains_key(key) {
                return Err(format!("strings.csv line {line}: duplicate key {key}"));
            }
            for lang in Lang::ALL {
                let cell = rec.get(lang_cols[lang as usize]).map(|c| c.as_str()).unwrap_or("");
                s.tables[lang as usize].insert(key.to_string(), cell.replace("\\n", "\n"));
            }
        }
        Ok(s)
    }

    /// Text for `key` in `lang`; falls back to English, then to the key itself.
    pub fn get<'a>(&'a self, lang: Lang, key: &'a str) -> &'a str {
        if let Some(s) = self.tables[lang as usize].get(key) {
            if !s.is_empty() {
                return s;
            }
        }
        self.tables[Lang::En as usize].get(key).map(|s| s.as_str()).unwrap_or(key)
    }

    /// For every flavour, the set of characters (from all languages that use that flavour —
    /// EN and KO use `Ko`, ZH uses `ZhHans`; the language names in the language menu are
    /// added to both) that the original bitmap font does not cover. Used to build glyphs.
    /// Printable ASCII the original font lacks (`:` `/` `(` …) is added to both as well, so
    /// raw texts never fall back to '?'. Whitespace and control characters are never included.
    /// Both flavours are always present in the map.
    pub fn required_chars(&self) -> HashMap<Flavor, BTreeSet<char>> {
        let mut out: HashMap<Flavor, BTreeSet<char>> = HashMap::new();
        for lang in Lang::ALL {
            let set = out.entry(lang.flavor()).or_default();
            for text in self.tables[lang as usize].values() {
                set.extend(text.chars().filter(|&c| needs_extra_glyph(c)));
            }
        }
        let mut shared: BTreeSet<char> =
            (0x21u8..=0x7e).map(char::from).filter(|&c| needs_extra_glyph(c)).collect();
        for key in [keys::LANG_EN, keys::LANG_KO, keys::LANG_ZH] {
            for table in &self.tables {
                if let Some(name) = table.get(key) {
                    shared.extend(name.chars().filter(|&c| needs_extra_glyph(c)));
                }
            }
        }
        for flavor in [Flavor::Ko, Flavor::ZhHans] {
            out.entry(flavor).or_default().extend(shared.iter().copied());
        }
        out
    }
}

/// True for a visible character the original bitmap font cannot draw.
fn needs_extra_glyph(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && crate::font::original_glyph(c).is_none()
}

/// RFC 4180 record splitter. Returns (first physical line of the record, fields).
/// Line breaks inside quoted fields are normalised to `\n`.
fn parse_records(text: &str) -> Result<Vec<(usize, Vec<String>)>, String> {
    let mut out = Vec::new();
    let mut rec: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut line = 1usize;
    let mut rec_line = 1usize;
    let mut in_quotes = false;
    // A closing quote was seen: only a separator or a line end may follow.
    let mut closed = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => {
                    in_quotes = false;
                    closed = true;
                }
                '\r' | '\n' => {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    field.push('\n');
                    line += 1;
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            ',' => {
                rec.push(std::mem::take(&mut field));
                closed = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                rec.push(std::mem::take(&mut field));
                out.push((rec_line, std::mem::take(&mut rec)));
                closed = false;
                line += 1;
                rec_line = line;
            }
            _ if closed => {
                return Err(format!("strings.csv line {line}: unexpected text after a closing quote"));
            }
            '"' if field.is_empty() => in_quotes = true,
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(format!("strings.csv line {rec_line}: unterminated quoted field"));
    }
    if closed || !field.is_empty() || !rec.is_empty() {
        rec.push(field);
        out.push((rec_line, rec));
    }
    Ok(out)
}

/// All string keys. English text in comments = original C string.
pub mod keys {
    // ---- prelude / loading
    pub const PRELUDE_DP: &str = "PRELUDE_DP"; // "Digital Psychosis"
    pub const PRELUDE_PRESENTS: &str = "PRELUDE_PRESENTS"; // "Presents"
    /// Multi-line story; EN lines: "Young Sigmund has a" / "Few problems. Can" / "you help him find"
    /// / "the rogue brain cells" / "in his mind and shut" / "Them down?"
    pub const PRELUDE_STORY: &str = "PRELUDE_STORY";
    pub const PRELUDE_PRESS: &str = "PRELUDE_PRESS"; // "PRESS SPACE"
    pub const PRELUDE_CHEAT: &str = "PRELUDE_CHEAT"; // "Cheat Mode!!!"
    pub const PRELUDE_CHEAT_WEAPONS: &str = "PRELUDE_CHEAT_WEAPONS"; // "F1, F2 Get Weapons"
    pub const PRELUDE_CHEAT_STRENGTH: &str = "PRELUDE_CHEAT_STRENGTH"; // "F3 - Restore Strength"
    pub const PRELUDE_FUN: &str = "PRELUDE_FUN"; // "HAVE FUN !!!"
    pub const LOAD_WAIT: &str = "LOAD_WAIT"; // "Please Wait"
    pub const LOAD_FILES: &str = "LOAD_FILES"; // "Loading Game files"
    pub const ERR_LOAD_1: &str = "ERR_LOAD_1"; // "A game file could not"
    pub const ERR_LOAD_2: &str = "ERR_LOAD_2"; // "be loaded. Program"
    pub const ERR_LOAD_3: &str = "ERR_LOAD_3"; // "Must exit now."
    // ---- main options menu
    pub const MENU_OPTIONS: &str = "MENU_OPTIONS"; // "Options"
    pub const MENU_DEFINE_CONTROLS: &str = "MENU_DEFINE_CONTROLS"; // "Define Controls"
    pub const MENU_TUNE_GAME: &str = "MENU_TUNE_GAME"; // "Tune Game Options"
    pub const MENU_CHOOSE_ZONE: &str = "MENU_CHOOSE_ZONE"; // "Choose Mental Zone"
    pub const MENU_SAVE_POSITION: &str = "MENU_SAVE_POSITION"; // "Save Position"
    pub const MENU_POSITION_SAVED: &str = "MENU_POSITION_SAVED"; // "Position Saved" (new)
    pub const MENU_FIRE_PLAY: &str = "MENU_FIRE_PLAY"; // "Fire - Play"
    pub const MENU_FIRE_RETURN: &str = "MENU_FIRE_RETURN"; // "Fire - Return" (new, in-game options)
    pub const MENU_ESC_EXIT: &str = "MENU_ESC_EXIT"; // "ESC - Exit"
    pub const MENU_QUIT_TITLE: &str = "MENU_QUIT_TITLE"; // "Quit Asylum?" (new)
    pub const MENU_QUIT_YES: &str = "MENU_QUIT_YES"; // "Fire - Yes" (new)
    pub const MENU_QUIT_NO: &str = "MENU_QUIT_NO"; // "ESC - No" (new)
    // ---- zone choice
    pub const ZONE_TITLE_1: &str = "ZONE_TITLE_1"; // "Choose which"
    pub const ZONE_TITLE_2: &str = "ZONE_TITLE_2"; // "mental zone"
    pub const ZONE_EGO: &str = "ZONE_EGO"; // "Ego"
    pub const ZONE_PSYCHE: &str = "ZONE_PSYCHE"; // "Psyche"
    pub const ZONE_ID: &str = "ZONE_ID"; // "Id"
    pub const ZONE_SAVED: &str = "ZONE_SAVED"; // "Saved Game"
    pub const ZONE_ID_LOCK_1: &str = "ZONE_ID_LOCK_1"; // "To play the id"
    pub const ZONE_ID_LOCK_2: &str = "ZONE_ID_LOCK_2"; // "you need to complete"
    pub const ZONE_ID_LOCK_3: &str = "ZONE_ID_LOCK_3"; // "the ego first."
    // ---- controls
    pub const CTRL_TITLE: &str = "CTRL_TITLE"; // "Controls"
    pub const CTRL_KEYBOARD: &str = "CTRL_KEYBOARD"; // "Keyboard"
    pub const CTRL_GAMEPAD: &str = "CTRL_GAMEPAD"; // "Joystick" -> "Gamepad"
    pub const CTRL_DEFAULTS: &str = "CTRL_DEFAULTS"; // "Reset Keys" (new)
    pub const KEY_PRESS_LEFT: &str = "KEY_PRESS_LEFT"; // "Press Key For Left"
    pub const KEY_PRESS_RIGHT: &str = "KEY_PRESS_RIGHT"; // "Press Key For Right"
    pub const KEY_PRESS_UP: &str = "KEY_PRESS_UP"; // "Press Key For Jump" (C: "Press Key For Up"; up = jump)
    pub const KEY_PRESS_DOWN: &str = "KEY_PRESS_DOWN"; // "Press Key For Down"
    pub const KEY_PRESS_FIRE: &str = "KEY_PRESS_FIRE"; // "Press Key to Fire"
    pub const KEY_LEFT: &str = "KEY_LEFT"; // "Left"
    pub const KEY_RIGHT: &str = "KEY_RIGHT"; // "Right"
    pub const KEY_UP: &str = "KEY_UP"; // "Jump" (the up action jumps)
    pub const KEY_DOWN: &str = "KEY_DOWN"; // "Down"
    pub const KEY_FIRE: &str = "KEY_FIRE"; // "Fire"
    pub const PAD_TITLE: &str = "PAD_TITLE"; // "Reminder"
    pub const PAD_JUMP: &str = "PAD_JUMP"; // "To jump, you can"
    pub const PAD_JUMP_1: &str = "PAD_JUMP_1"; // "Use Stick Up"
    pub const PAD_JUMP_2: &str = "PAD_JUMP_2"; // "Use Button A"
    pub const PAD_FIRE: &str = "PAD_FIRE"; // "Button X or B - Fire"
    pub const PAD_MENU: &str = "PAD_MENU"; // "Start - Menu"
    // ---- tune game
    pub const TUNE_TITLE: &str = "TUNE_TITLE"; // "Tune Game"
    pub const TUNE_SOUND: &str = "TUNE_SOUND"; // "Sound System"
    pub const TUNE_VOLUME: &str = "TUNE_VOLUME"; // "Sound Volume"
    pub const TUNE_VIDEO: &str = "TUNE_VIDEO"; // "Video System"
    pub const TUNE_LANGUAGE: &str = "TUNE_LANGUAGE"; // "Language" (new)
    pub const SOUND_TITLE: &str = "SOUND_TITLE"; // "Tune Sound"
    pub const SOUND_NONE: &str = "SOUND_NONE"; // "No Sound"
    pub const SOUND_FX: &str = "SOUND_FX"; // "Effects Only"
    pub const SOUND_FX_MUSIC: &str = "SOUND_FX_MUSIC"; // "Effects and Music"
    pub const VOL_TITLE: &str = "VOL_TITLE"; // "Change volume"
    pub const VOL_FX_UP: &str = "VOL_FX_UP"; // "Louder effects"
    pub const VOL_FX_DOWN: &str = "VOL_FX_DOWN"; // "Quieter effects"
    pub const VOL_MUSIC_UP: &str = "VOL_MUSIC_UP"; // "Louder music"
    pub const VOL_MUSIC_DOWN: &str = "VOL_MUSIC_DOWN"; // "Quieter music"
    pub const VOL_FX_LEVEL: &str = "VOL_FX_LEVEL"; // "SFX"
    pub const VOL_MUSIC_LEVEL: &str = "VOL_MUSIC_LEVEL"; // "Music"
    pub const VIDEO_TITLE: &str = "VIDEO_TITLE"; // "Tune Video"
    pub const VIDEO_MODE: &str = "VIDEO_MODE"; // "Mode" (label; value right-aligned)
    pub const VIDEO_MODE_WINDOW: &str = "VIDEO_MODE_WINDOW"; // "Windowed"
    pub const VIDEO_MODE_BORDERLESS: &str = "VIDEO_MODE_BORDERLESS"; // "Borderless" (window covering the screen)
    pub const VIDEO_MODE_FULLSCREEN: &str = "VIDEO_MODE_FULLSCREEN"; // "Full Screen"
    pub const VIDEO_ASPECT: &str = "VIDEO_ASPECT"; // "Aspect" (label; value "16:9" etc.)
    pub const VIDEO_ASPECT_MONITOR: &str = "VIDEO_ASPECT_MONITOR"; // "Monitor" (aspect value outside window mode)
    pub const VIDEO_SCALE: &str = "VIDEO_SCALE"; // "Scale" (label; value e.g. "2.5" + VIDEO_SCALE_UNIT)
    pub const VIDEO_SCALE_UNIT: &str = "VIDEO_SCALE_UNIT"; // "x" (suffix of the scale value: "2.5x")
    pub const VIDEO_SMOOTH: &str = "VIDEO_SMOOTH"; // "Smooth Motion"
    pub const VIDEO_VSYNC: &str = "VIDEO_VSYNC"; // "Vsync"
    pub const LANG_TITLE: &str = "LANG_TITLE"; // "Language"
    pub const LANG_EN: &str = "LANG_EN"; // "English" (same in every column)
    pub const LANG_KO: &str = "LANG_KO"; // "한국어" (same in every column)
    pub const LANG_ZH: &str = "LANG_ZH"; // "简体中文" (same in every column)
    // ---- escape (pause) menu
    pub const ESC_INTERRUPTED: &str = "ESC_INTERRUPTED"; // "• Game Interrupted •"
    pub const ESC_SELECT: &str = "ESC_SELECT"; // "Please select action"
    pub const ESC_ABANDON: &str = "ESC_ABANDON"; // "ESC - Abandon game"
    pub const ESC_LOSE_LIFE: &str = "ESC_LOSE_LIFE"; // "Q    - lose this life"
    pub const ESC_RETURN: &str = "ESC_RETURN"; // "R    - Return to Game"
    pub const ESC_OPTIONS: &str = "ESC_OPTIONS"; // "O    - Alter Options"
    // ---- high scores
    pub const HS_TITLE: &str = "HS_TITLE"; // "Zone High Scores"
    pub const HS_PRESS_FIRE: &str = "HS_PRESS_FIRE"; // "press fire"
    // ---- in game messages
    pub const MSG_LETS_GO: &str = "MSG_LETS_GO"; // "Let's Go!"
    pub const MSG_SNUFFED: &str = "MSG_SNUFFED"; // "• Snuffed It! •"
    pub const MSG_GAME_OVER: &str = "MSG_GAME_OVER"; // "-  GAME OVER  -"
    pub const MSG_POTION: &str = "MSG_POTION"; // "A potion and Skull!"
    pub const MSG_RISE: &str = "MSG_RISE"; // "Rise from the dead!"
    pub const MSG_BONUS_10K: &str = "MSG_BONUS_10K"; // "Bonus 10000"
    pub const MSG_BONUS_20K: &str = "MSG_BONUS_20K"; // "Bonus 20000"
    pub const MSG_BONUS_30K: &str = "MSG_BONUS_30K"; // "Bonus 30000"
    pub const MSG_BONUS_50K: &str = "MSG_BONUS_50K"; // "Bonus 50000"
    pub const MSG_BONUS_100K: &str = "MSG_BONUS_100K"; // "Bonus 100000"
    pub const MSG_WELL_DONE: &str = "MSG_WELL_DONE"; // "WELL DONE!"
    pub const MSG_COMPLETED: &str = "MSG_COMPLETED"; // "You have completed"
    pub const MSG_THE_GAME: &str = "MSG_THE_GAME"; // "The game!!!"
    pub const MSG_THIS_ZONE: &str = "MSG_THIS_ZONE"; // "This zone!"
    pub const MSG_ID_UNLOCKED: &str = "MSG_ID_UNLOCKED"; // "The Id is now open!" (new)
    /// Weapon pickup messages, index = (plweapontype-1)&15 (player.c weaponcheck).
    pub const WEAPONS: [&str; 16] = [
        "WEAPON_0",  // "Standard Mini-Gun"
        "WEAPON_1",  // "Mini-Gun with spray"
        "WEAPON_2",  // "Five Stream Gun"
        "WEAPON_3",  // "Fast 5 way - zap those nasties!"
        "WEAPON_4",  // "A weird one!"
        "WEAPON_5",  // "Three Way Blitzer"
        "WEAPON_6",  // "Blitzing Five Way - Lets Party"
        "WEAPON_7",  // "MegaBlam 5000 Mini-Gun!!!!"
        "WEAPON_8",  // "Standard Rocket Launcher"
        "WEAPON_9",  // "Twin Rocket Launcher"
        "WEAPON_10", // "Quad Launcher - Open Fire!"
        "WEAPON_11", // "Launcher with starburst"
        "WEAPON_12", // "Twin rockets with starburst - let's see some fireworks!"
        "WEAPON_13", // "Quad Launcher with starburst! Arrgghhhh!!!!"
        "WEAPON_14", // "A Pot Pourri Rocket Launcher - mix and match!"
        "WEAPON_15", // "MegaBlam Rocket Launcher!!!!  Six Shots Only!"
    ];

    /// Every key (tests check the CSV contains all of them, in all languages).
    pub fn all() -> Vec<&'static str> {
        let mut v = vec![
            PRELUDE_DP, PRELUDE_PRESENTS, PRELUDE_STORY, PRELUDE_PRESS, PRELUDE_CHEAT, PRELUDE_CHEAT_WEAPONS,
            PRELUDE_CHEAT_STRENGTH, PRELUDE_FUN, LOAD_WAIT, LOAD_FILES, ERR_LOAD_1, ERR_LOAD_2, ERR_LOAD_3,
            MENU_OPTIONS, MENU_DEFINE_CONTROLS, MENU_TUNE_GAME, MENU_CHOOSE_ZONE, MENU_SAVE_POSITION,
            MENU_POSITION_SAVED, MENU_FIRE_PLAY, MENU_FIRE_RETURN, MENU_ESC_EXIT, MENU_QUIT_TITLE, MENU_QUIT_YES,
            MENU_QUIT_NO, ZONE_TITLE_1, ZONE_TITLE_2, ZONE_EGO, ZONE_PSYCHE, ZONE_ID, ZONE_SAVED, ZONE_ID_LOCK_1,
            ZONE_ID_LOCK_2, ZONE_ID_LOCK_3, CTRL_TITLE, CTRL_KEYBOARD, CTRL_GAMEPAD, CTRL_DEFAULTS, KEY_PRESS_LEFT,
            KEY_PRESS_RIGHT, KEY_PRESS_UP, KEY_PRESS_DOWN, KEY_PRESS_FIRE, KEY_LEFT, KEY_RIGHT, KEY_UP, KEY_DOWN,
            KEY_FIRE, PAD_TITLE, PAD_JUMP, PAD_JUMP_1, PAD_JUMP_2, PAD_FIRE, PAD_MENU, TUNE_TITLE, TUNE_SOUND,
            TUNE_VOLUME, TUNE_VIDEO, TUNE_LANGUAGE, SOUND_TITLE, SOUND_NONE, SOUND_FX, SOUND_FX_MUSIC, VOL_TITLE,
            VOL_FX_UP, VOL_FX_DOWN, VOL_MUSIC_UP, VOL_MUSIC_DOWN, VOL_FX_LEVEL, VOL_MUSIC_LEVEL, VIDEO_TITLE,
            VIDEO_MODE, VIDEO_MODE_WINDOW, VIDEO_MODE_BORDERLESS, VIDEO_MODE_FULLSCREEN, VIDEO_ASPECT,
            VIDEO_ASPECT_MONITOR, VIDEO_SCALE, VIDEO_SCALE_UNIT, VIDEO_SMOOTH, VIDEO_VSYNC, LANG_TITLE, LANG_EN, LANG_KO, LANG_ZH, ESC_INTERRUPTED, ESC_SELECT, ESC_ABANDON, ESC_LOSE_LIFE, ESC_RETURN,
            ESC_OPTIONS, HS_TITLE, HS_PRESS_FIRE, MSG_LETS_GO, MSG_SNUFFED, MSG_GAME_OVER, MSG_POTION, MSG_RISE,
            MSG_BONUS_10K, MSG_BONUS_20K, MSG_BONUS_30K, MSG_BONUS_50K, MSG_BONUS_100K, MSG_WELL_DONE,
            MSG_COMPLETED, MSG_THE_GAME, MSG_THIS_ZONE, MSG_ID_UNLOCKED,
        ];
        v.extend_from_slice(&WEAPONS);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{original_advance, original_glyph, SPACE_ADVANCE};

    fn project_file(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn game_strings() -> Strings {
        Strings::parse_csv(&project_file("game/locale/strings.csv")).expect("strings.csv parses")
    }

    #[test]
    fn parse_rfc4180() {
        let csv = "\u{feff}key,en,ko,zh_CN\r\n\
                   A,\"x, y\",\"say \"\"hi\"\"\",z\r\n\
                   \r\n\
                   B,\"two\r\nlines\",one\\ntwo,\r\n\
                   \n\
                   C,c\n\
                   D,\"\",d,\"dd\"";
        let s = Strings::parse_csv(csv).unwrap();
        assert_eq!(s.get(Lang::En, "A"), "x, y");
        assert_eq!(s.get(Lang::Ko, "A"), "say \"hi\"");
        assert_eq!(s.get(Lang::ZhCn, "A"), "z");
        assert_eq!(s.get(Lang::En, "B"), "two\nlines");
        assert_eq!(s.get(Lang::Ko, "B"), "one\ntwo");
        // Empty / missing cells fall back to English, unknown keys to the key.
        assert_eq!(s.get(Lang::ZhCn, "B"), "two\nlines");
        assert_eq!(s.get(Lang::Ko, "C"), "c");
        assert_eq!(s.get(Lang::Ko, "D"), "d");
        assert_eq!(s.get(Lang::ZhCn, "D"), "dd");
        assert_eq!(s.get(Lang::En, "NOPE"), "NOPE");
        assert_eq!(s.tables[Lang::En as usize].len(), 4);
    }

    #[test]
    fn parse_finds_columns_by_name() {
        let s = Strings::parse_csv("zh_CN,key,ko,en\nzh,K,ko,en\n").unwrap();
        assert_eq!(s.get(Lang::En, "K"), "en");
        assert_eq!(s.get(Lang::Ko, "K"), "ko");
        assert_eq!(s.get(Lang::ZhCn, "K"), "zh");
    }

    #[test]
    fn parse_errors() {
        assert!(Strings::parse_csv("").is_err());
        assert!(Strings::parse_csv("key,en,ko\nA,a,b\n").is_err());
        assert!(Strings::parse_csv("key,en,ko,zh_CN\nA,\"open,b,c\n").is_err());
        assert!(Strings::parse_csv("key,en,ko,zh_CN\nA,\"a\"x,b,c\n").is_err());
        assert!(Strings::parse_csv("key,en,ko,zh_CN\nA,a,b,c\nA,a,b,c\n").is_err());
        assert!(Strings::parse_csv("key,en,ko,zh_CN\n,a,b,c\n").is_err());
    }

    #[test]
    fn csv_has_every_key_in_every_language() {
        let s = game_strings();
        let all = keys::all();
        for key in &all {
            for lang in Lang::ALL {
                let t = s.tables[lang as usize].get(*key).unwrap_or_else(|| panic!("{key} missing"));
                assert!(!t.trim().is_empty(), "{key} empty in {}", lang.code());
                assert_eq!(t.trim(), t, "{key}/{}: stray outer whitespace", lang.code());
            }
        }
        for key in s.tables[Lang::En as usize].keys() {
            assert!(all.contains(&key.as_str()), "{key} is not declared in i18n::keys");
        }
    }

    #[test]
    fn english_is_the_original_c_text() {
        let s = game_strings();
        let en = |k| s.get(Lang::En, k).to_string();
        assert_eq!(
            en(keys::PRELUDE_STORY),
            "Young Sigmund has a\nFew problems. Can\nyou help him find\n\
             the rogue brain cells\nin his mind and shut\nThem down?"
        );
        assert_eq!(en(keys::PRELUDE_CHEAT_WEAPONS), "F1, F2 Get Weapons");
        assert_eq!(en(keys::LOAD_FILES), "Loading Game files");
        assert_eq!(en(keys::MENU_CHOOSE_ZONE), "Choose Mental Zone");
        assert_eq!(en(keys::ZONE_ID_LOCK_3), "the ego first.");
        assert_eq!(en(keys::PAD_JUMP), "To jump, you can");
        assert_eq!(en(keys::ESC_INTERRUPTED), "• Game Interrupted •");
        assert_eq!(en(keys::ESC_LOSE_LIFE), "Q    - lose this life");
        assert_eq!(en(keys::ESC_RETURN), "R    - Return to Game");
        assert_eq!(en(keys::ESC_OPTIONS), "O    - Alter Options");
        assert_eq!(en(keys::HS_PRESS_FIRE), "press fire");
        assert_eq!(en(keys::MSG_LETS_GO), "Let's Go!");
        assert_eq!(en(keys::MSG_SNUFFED), "• Snuffed It! •");
        assert_eq!(en(keys::MSG_GAME_OVER), "-  GAME OVER  -");
        assert_eq!(en(keys::MSG_BONUS_100K), "Bonus 100000");
        assert_eq!(en(keys::WEAPONS[3]), "Fast 5 way - zap those nasties!");
        assert_eq!(en(keys::WEAPONS[12]), "Twin rockets with starburst - let's see some fireworks!");
        assert_eq!(en(keys::WEAPONS[15]), "MegaBlam Rocket Launcher!!!!  Six Shots Only!");
    }

    #[test]
    fn english_uses_only_the_original_font() {
        let s = game_strings();
        for (key, text) in &s.tables[Lang::En as usize] {
            if [keys::LANG_KO, keys::LANG_ZH].contains(&key.as_str()) {
                continue;
            }
            for c in text.chars() {
                assert!(c == ' ' || c == '\n' || original_glyph(c).is_some(), "{key}: {c:?}");
            }
        }
    }

    #[test]
    fn language_names_and_brands_are_untranslated() {
        let s = game_strings();
        for (key, name) in [(keys::LANG_EN, "English"), (keys::LANG_KO, "한국어"), (keys::LANG_ZH, "简体中文")] {
            for lang in Lang::ALL {
                assert_eq!(s.get(lang, key), name);
            }
        }
        for lang in [Lang::Ko, Lang::ZhCn] {
            assert!(s.get(lang, keys::WEAPONS[7]).starts_with("MegaBlam 5000 "));
            assert!(s.get(lang, keys::WEAPONS[15]).starts_with("MegaBlam "));
        }
    }

    #[test]
    fn key_prefixes_match_english() {
        let s = game_strings();
        let prefixed = [
            (keys::ESC_ABANDON, "ESC - "),
            // padded like the original so the dashes line up with "ESC - "
            (keys::ESC_LOSE_LIFE, "Q    - "),
            (keys::ESC_RETURN, "R    - "),
            (keys::ESC_OPTIONS, "O    - "),
            (keys::MENU_ESC_EXIT, "ESC - "),
            (keys::MENU_QUIT_NO, "ESC - "),
            (keys::PRELUDE_CHEAT_WEAPONS, "F1, F2 "),
            (keys::PRELUDE_CHEAT_STRENGTH, "F3 - "),
            (keys::PAD_MENU, "Start - "),
        ];
        for lang in [Lang::Ko, Lang::ZhCn] {
            for (key, prefix) in prefixed {
                assert!(s.get(lang, key).starts_with(prefix), "{key}/{}", lang.code());
            }
            // "Fire - ..." lines keep the "<fire> - " form with the localised name of Fire.
            let fire = format!("{} - ", s.get(lang, keys::KEY_FIRE));
            for key in [keys::MENU_FIRE_PLAY, keys::MENU_FIRE_RETURN, keys::MENU_QUIT_YES] {
                assert!(s.get(lang, key).starts_with(&fire), "{key}/{}", lang.code());
            }
        }
    }

    /// Width with the original advances; other glyphs (CJK) counted at a pessimistic 16 px.
    fn width_estimate(line: &str) -> i32 {
        line.chars()
            .map(|c| match original_glyph(c) {
                Some(g) => original_advance(g),
                None if c == ' ' => SPACE_ADVANCE,
                None => 16,
            })
            .sum()
    }

    #[test]
    fn translations_fit_the_screen() {
        let s = game_strings();
        for lang in [Lang::Ko, Lang::ZhCn] {
            for key in keys::all() {
                // Weapon names scroll through the game window (message_scroll).
                if keys::WEAPONS.contains(&key) {
                    continue;
                }
                for line in s.get(lang, key).lines() {
                    assert!(width_estimate(line) <= 280, "{key}/{}: {line:?} too wide", lang.code());
                }
            }
            let story = s.get(lang, keys::PRELUDE_STORY).lines().count();
            assert!((4..=6).contains(&story), "{} story has {story} lines", lang.code());
        }
        assert_eq!(s.get(Lang::En, keys::PRELUDE_STORY).lines().count(), 6);
    }

    #[test]
    fn cjk_text_uses_the_original_punctuation_glyphs() {
        // Korean uses half-width punctuation (drawn with the original glyphs). Simplified
        // Chinese uses mainland full-width punctuation inside Chinese sentences; full-width
        // letters/digits and the ideographic space are never used.
        let s = game_strings();
        for (key, text) in &s.tables[Lang::Ko as usize] {
            for c in text.chars() {
                assert!(!('\u{ff01}'..='\u{ff5e}').contains(&c) && c != '\u{3000}', "{key}: {c:?}");
            }
        }
        let zh_punct = "！？，：；（）";
        for (key, text) in &s.tables[Lang::ZhCn as usize] {
            for c in text.chars() {
                let fullwidth = ('\u{ff01}'..='\u{ff5e}').contains(&c);
                assert!((!fullwidth || zh_punct.contains(c)) && c != '\u{3000}', "{key}: {c:?}");
            }
            // no half-width ! or ? directly after a Chinese character
            let chars: Vec<char> = text.chars().collect();
            for w in chars.windows(2) {
                let cjk = ('\u{4e00}'..='\u{9fff}').contains(&w[0]);
                assert!(!(cjk && (w[1] == '!' || w[1] == '?')), "{key}: half-width {:?} after {:?}", w[1], w[0]);
            }
        }
    }

    #[test]
    fn required_chars_per_flavour() {
        let s = game_strings();
        let req = s.required_chars();
        let ko = &req[&Flavor::Ko];
        let zh = &req[&Flavor::ZhHans];
        for c in "골칫거리한국어简体中文".chars() {
            assert!(ko.contains(&c), "ko lacks {c}");
        }
        for c in "西格蒙德简体中文한국어".chars() {
            assert!(zh.contains(&c), "zh lacks {c}");
        }
        assert!(!ko.contains(&'西') && !zh.contains(&'골'));
        for set in [ko, zh] {
            assert!(set.contains(&':') && set.contains(&'/'));
            assert!(set.iter().all(|&c| original_glyph(c).is_none() && !c.is_whitespace()));
        }
    }

    /// Guard against editing strings.csv without re-running tools/subset_fonts.py.
    #[test]
    fn font_subsets_cover_required_chars() {
        let encodings = |rel: &str| -> std::collections::HashSet<u32> {
            project_file(rel)
                .lines()
                .filter_map(|l| l.strip_prefix("ENCODING "))
                .filter_map(|v| v.split_whitespace().next()?.parse().ok())
                .collect()
        };
        let ko = encodings("game/fonts/asylum-ko.bdf");
        let zh = encodings("game/fonts/asylum-zh_hans.bdf");
        let req = game_strings().required_chars();
        for (flavor, own, other) in [(Flavor::Ko, &ko, &zh), (Flavor::ZhHans, &zh, &ko)] {
            for &c in &req[&flavor] {
                let u = c as u32;
                assert!(own.contains(&u) || other.contains(&u), "{c:?} ({flavor:?}) missing from game/fonts");
            }
        }
    }
}
