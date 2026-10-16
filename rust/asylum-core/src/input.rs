//! Input as seen by the core. The host (Godot) translates OS events into these.
//!
//! Key codes are Godot 4 `Key` enum values (physical keycodes) so the config file and key
//! rebinding work with any keyboard layout.

/// Godot 4.7 `Key` values (verified against the 4.7.2 binary).
pub mod key {
    pub const ESCAPE: i64 = 4194305;
    pub const TAB: i64 = 4194306;
    pub const BACKSPACE: i64 = 4194308;
    pub const ENTER: i64 = 4194309;
    pub const KP_ENTER: i64 = 4194310;
    pub const INSERT: i64 = 4194311;
    pub const DELETE: i64 = 4194312;
    pub const PAUSE: i64 = 4194313;
    pub const HOME: i64 = 4194317;
    pub const END: i64 = 4194318;
    pub const LEFT: i64 = 4194319;
    pub const UP: i64 = 4194320;
    pub const RIGHT: i64 = 4194321;
    pub const DOWN: i64 = 4194322;
    pub const PAGEUP: i64 = 4194323;
    pub const PAGEDOWN: i64 = 4194324;
    pub const SHIFT: i64 = 4194325;
    pub const CTRL: i64 = 4194326;
    pub const META: i64 = 4194327;
    pub const ALT: i64 = 4194328;
    pub const CAPSLOCK: i64 = 4194329;
    pub const F1: i64 = 4194332; // F2 = F1+1 ... F12 = F1+11
    pub const F12: i64 = 4194343;
    pub const KP_MULTIPLY: i64 = 4194433;
    pub const KP_DIVIDE: i64 = 4194434;
    pub const KP_SUBTRACT: i64 = 4194435;
    pub const KP_PERIOD: i64 = 4194436;
    pub const KP_ADD: i64 = 4194437;
    pub const KP_0: i64 = 4194438; // KP_9 = KP_0+9
    pub const MENU: i64 = 4194370;
    pub const SPACE: i64 = 32;
    pub const APOSTROPHE: i64 = 39;
    pub const COMMA: i64 = 44;
    pub const MINUS: i64 = 45;
    pub const PERIOD: i64 = 46;
    pub const SLASH: i64 = 47;
    pub const KEY_0: i64 = 48; // '0'..'9' = 48..57
    pub const SEMICOLON: i64 = 59;
    pub const EQUAL: i64 = 61;
    pub const A: i64 = 65; // 'A'..'Z' = 65..90
    pub const Z: i64 = 90;
    pub const X: i64 = 88;
    pub const BRACKETLEFT: i64 = 91;
    pub const BACKSLASH: i64 = 92;
    pub const BRACKETRIGHT: i64 = 93;
    pub const QUOTELEFT: i64 = 96;
}

/// Gamepad buttons (Godot `JoyButton` values).
pub mod pad {
    pub const A: i64 = 0;
    pub const B: i64 = 1;
    pub const X: i64 = 2;
    pub const Y: i64 = 3;
    pub const BACK: i64 = 4;
    pub const START: i64 = 6;
    pub const LEFT_SHOULDER: i64 = 9;
    pub const RIGHT_SHOULDER: i64 = 10;
    pub const DPAD_UP: i64 = 11;
    pub const DPAD_DOWN: i64 = 12;
    pub const DPAD_LEFT: i64 = 13;
    pub const DPAD_RIGHT: i64 = 14;
}

/// The five game actions (index into `Config::keys` and the held mask bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Left = 0,
    Right = 1,
    Up = 2,
    Down = 3,
    Fire = 4,
}

/// Bit masks for `held` (bit i = Action i).
pub const HELD_LEFT: u32 = 1 << 0;
pub const HELD_RIGHT: u32 = 1 << 1;
pub const HELD_UP: u32 = 1 << 2;
pub const HELD_DOWN: u32 = 1 << 3;
pub const HELD_FIRE: u32 = 1 << 4;
/// Alt held (prelude cheat activation together with the middle mouse button).
pub const HELD_ALT: u32 = 1 << 5;
/// Cheat keys F1, F2, F3 held (keyboard.c `cheatread` polls them every frame).
pub const HELD_F1: u32 = 1 << 6;
pub const HELD_F2: u32 = 1 << 7;
pub const HELD_F3: u32 = 1 << 8;

/// Discrete input events, delivered in order to the game thread.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// A keyboard key went down. `code` = Godot physical keycode, `ch` = Unicode character
    /// produced (lower-cased) if any, `echo` = auto-repeat.
    Key { code: i64, ch: Option<char>, echo: bool },
    /// A gamepad button went down.
    Pad { button: i64 },
    /// Mouse moved to (x, y) in logical viewport coordinates.
    MouseMove { x: f32, y: f32 },
    /// Mouse button pressed at (x, y): 1 left, 2 right, 3 middle, 4/5 wheel up/down.
    MouseDown { x: f32, y: f32, button: u8 },
    /// Window close request (handled like quitting from the menu).
    CloseRequest,
    /// Size in pixels of the monitor the window is on (window resolutions and scales are
    /// offered up to it, see `display`); sent at start-up and when the monitor changes.
    Monitor { width: i32, height: i32 },
}

/// Bindable, human readable key name in upper case ASCII (renders with the original font
/// where possible), e.g. "Z", "SEMICOLON", "ENTER", "LEFT", "F1", "KP 5".
pub fn key_name(code: i64) -> String {
    let named = match code {
        key::SPACE => "SPACE",
        key::ENTER => "ENTER",
        key::KP_ENTER => "KP ENTER",
        key::ESCAPE => "ESCAPE",
        key::TAB => "TAB",
        key::BACKSPACE => "BACKSPACE",
        key::INSERT => "INSERT",
        key::DELETE => "DELETE",
        key::PAUSE => "PAUSE",
        key::HOME => "HOME",
        key::END => "END",
        key::PAGEUP => "PAGE UP",
        key::PAGEDOWN => "PAGE DOWN",
        key::LEFT => "LEFT",
        key::RIGHT => "RIGHT",
        key::UP => "UP",
        key::DOWN => "DOWN",
        key::SHIFT => "SHIFT",
        key::CTRL => "CTRL",
        key::ALT => "ALT",
        key::META => "META",
        key::CAPSLOCK => "CAPS LOCK",
        key::MENU => "MENU",
        key::KP_MULTIPLY => "KP MULTIPLY",
        key::KP_DIVIDE => "KP DIVIDE",
        key::KP_SUBTRACT => "KP MINUS",
        key::KP_PERIOD => "KP PERIOD",
        key::KP_ADD => "KP PLUS",
        key::SEMICOLON => "SEMICOLON",
        key::PERIOD => "PERIOD",
        key::COMMA => "COMMA",
        key::SLASH => "SLASH",
        key::MINUS => "MINUS",
        key::EQUAL => "EQUAL",
        key::APOSTROPHE => "APOSTROPHE",
        key::BACKSLASH => "BACKSLASH",
        key::BRACKETLEFT => "BRACKET L",
        key::BRACKETRIGHT => "BRACKET R",
        key::QUOTELEFT => "QUOTE LEFT",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    match code {
        // Letters and digits as themselves (Godot has no lower-case key codes; accept them anyway).
        0x30..=0x39 | 0x41..=0x5a => char::from(code as u8).to_string(),
        0x61..=0x7a => char::from(code as u8).to_ascii_uppercase().to_string(),
        key::F1..=key::F12 => format!("F{}", code - key::F1 + 1),
        _ if (key::KP_0..=key::KP_0 + 9).contains(&code) => format!("KP {}", code - key::KP_0),
        _ => format!("KEY {code}"),
    }
}

/// The display-mode key (F11); hosts send Alt+Enter as this key ([`alt_enter_as_f11`]).
pub const KEY_DISPLAY_MODE: i64 = key::F1 + 10;

/// Key code a host sends for a pressed key: Alt+Enter becomes [`KEY_DISPLAY_MODE`]. The
/// modifier state must come with the key event itself; a polled held mask can miss an Alt
/// pressed or released within the same frame, and the Enter would then activate a menu row.
pub fn alt_enter_as_f11(code: i64, alt: bool) -> i64 {
    if alt && (code == key::ENTER || code == key::KP_ENTER) {
        KEY_DISPLAY_MODE
    } else {
        code
    }
}

/// Default bindings: the original Z, X, ;, ., Return (keyboard.c keydefs).
pub const DEFAULT_KEYS: [i64; 5] = [key::Z, key::X, key::SEMICOLON, key::PERIOD, key::ENTER];
/// Always-active alternative bindings (modern convenience): arrows + Space / Ctrl fire.
pub const ALT_KEYS: [&[i64]; 5] = [&[key::LEFT], &[key::RIGHT], &[key::UP], &[key::DOWN], &[key::SPACE, key::CTRL]];

/// Whether alternative key `alt` of `action` is active: not when the player bound it to
/// another action, whose binding then wins in the game and in the menus alike.
pub fn alt_active(keys: &[i64], action: usize, alt: i64) -> bool {
    !keys.iter().enumerate().any(|(j, &k)| j != action && k == alt)
}

/// Whether key `code` triggers `action` (0 left, 1 right, 2 up, 3 down, 4 fire): its bound
/// key or an active alternative key.
pub fn triggers(keys: &[i64], action: usize, code: i64) -> bool {
    keys.get(action) == Some(&code) || ALT_KEYS[action].iter().any(|&a| a == code && alt_active(keys, action, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_enter_is_the_display_mode_key() {
        assert_eq!(alt_enter_as_f11(key::ENTER, true), KEY_DISPLAY_MODE);
        assert_eq!(alt_enter_as_f11(key::KP_ENTER, true), KEY_DISPLAY_MODE);
        assert_eq!(alt_enter_as_f11(key::ENTER, false), key::ENTER);
        assert_eq!(alt_enter_as_f11(key::Z, true), key::Z);
    }

    #[test]
    fn alternative_keys_yield_to_bindings() {
        let keys = DEFAULT_KEYS;
        assert!(triggers(&keys, 4, key::ENTER) && triggers(&keys, 4, key::SPACE) && triggers(&keys, 4, key::CTRL));
        assert!(triggers(&keys, 2, key::UP) && triggers(&keys, 2, key::SEMICOLON) && !triggers(&keys, 2, key::SPACE));
        // Space bound to up: it jumps and moves the menu highlight up, it is no longer fire
        let keys = [key::Z, key::X, key::SPACE, key::PERIOD, key::ENTER];
        assert!(!triggers(&keys, 4, key::SPACE) && triggers(&keys, 4, key::CTRL));
        assert!(triggers(&keys, 2, key::SPACE) && triggers(&keys, 2, key::UP));
        // an arrow bound to fire stops being its direction
        let keys = [key::Z, key::X, key::SEMICOLON, key::PERIOD, key::UP];
        assert!(triggers(&keys, 4, key::UP) && !triggers(&keys, 2, key::UP));
        assert!(!alt_active(&keys, 2, key::UP) && alt_active(&keys, 4, key::UP));
    }

    #[test]
    fn letters_digits_and_function_keys() {
        assert_eq!(key_name(key::A), "A");
        assert_eq!(key_name(key::Z), "Z");
        assert_eq!(key_name(key::X), "X");
        assert_eq!(key_name(b'q' as i64), "Q");
        assert_eq!(key_name(key::KEY_0), "0");
        assert_eq!(key_name(key::KEY_0 + 9), "9");
        assert_eq!(key_name(key::F1), "F1");
        assert_eq!(key_name(key::F1 + 4), "F5");
        assert_eq!(key_name(key::F12), "F12");
        assert_eq!(key_name(key::KP_0), "KP 0");
        assert_eq!(key_name(key::KP_0 + 5), "KP 5");
        assert_eq!(key_name(key::KP_0 + 9), "KP 9");
    }

    #[test]
    fn named_keys() {
        let cases = [
            (key::SPACE, "SPACE"),
            (key::ENTER, "ENTER"),
            (key::KP_ENTER, "KP ENTER"),
            (key::ESCAPE, "ESCAPE"),
            (key::TAB, "TAB"),
            (key::BACKSPACE, "BACKSPACE"),
            (key::LEFT, "LEFT"),
            (key::RIGHT, "RIGHT"),
            (key::UP, "UP"),
            (key::DOWN, "DOWN"),
            (key::SHIFT, "SHIFT"),
            (key::CTRL, "CTRL"),
            (key::ALT, "ALT"),
            (key::META, "META"),
            (key::SEMICOLON, "SEMICOLON"),
            (key::PERIOD, "PERIOD"),
            (key::COMMA, "COMMA"),
            (key::SLASH, "SLASH"),
            (key::MINUS, "MINUS"),
            (key::EQUAL, "EQUAL"),
            (key::APOSTROPHE, "APOSTROPHE"),
            (key::BACKSLASH, "BACKSLASH"),
            (key::BRACKETLEFT, "BRACKET L"),
            (key::BRACKETRIGHT, "BRACKET R"),
            (key::QUOTELEFT, "QUOTE LEFT"),
            (key::PAGEUP, "PAGE UP"),
        ];
        for (code, name) in cases {
            assert_eq!(key_name(code), name);
        }
    }

    #[test]
    fn unknown_keys() {
        assert_eq!(key_name(0), "KEY 0");
        assert_eq!(key_name(33), "KEY 33");
        assert_eq!(key_name(key::F12 + 1), "KEY 4194344");
        assert_eq!(key_name(key::KP_0 + 10), "KEY 4194448");
        assert_eq!(key_name(-7), "KEY -7");
        assert_eq!(key_name(i64::MAX), format!("KEY {}", i64::MAX));
    }

    #[test]
    fn names_render_with_the_original_font() {
        let codes = (0..200).chain(key::ESCAPE..key::ESCAPE + 200).chain(DEFAULT_KEYS);
        for code in codes {
            for c in key_name(code).chars() {
                assert!(c == ' ' || c == '-' || c.is_ascii_digit() || c.is_ascii_uppercase(), "{code}: {c:?}");
                assert!(c == ' ' || crate::font::original_glyph(c).is_some());
            }
        }
        assert_eq!(DEFAULT_KEYS.map(key_name), ["Z", "X", "SEMICOLON", "PERIOD", "ENTER"]);
    }
}
