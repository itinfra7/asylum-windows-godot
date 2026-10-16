//! Tests of the frontend state machine (no rendering resources needed).

use super::*;
use crate::atlas::{AtlasRect, AtlasSprite, Bank, Img};
use crate::input::{HELD_LEFT, HELD_RIGHT};
use crate::render::RenderFrame;

fn core_with(config: Config) -> Core {
    Core::new(crate::assets::testdata::assets(), Arc::new(Strings::default()), None, config, (320.0, 256.0), 1)
}

fn core() -> Core {
    core_with(Config::default())
}

fn ev(c: &mut Core, e: InputEvent) {
    c.push_event(e);
    c.tick(0);
}

fn press(c: &mut Core, code: i64) {
    ev(c, InputEvent::Key { code, ch: None, echo: false });
}

/// Godot keycode of a letter key.
fn kc(l: char) -> i64 {
    key::A + (l.to_ascii_uppercase() as u8 - b'A') as i64
}

fn letter(c: &mut Core, l: char) {
    ev(c, InputEvent::Key { code: kc(l), ch: Some(l), echo: false });
}

fn digit(c: &mut Core, d: i64) {
    press(c, key::KEY_0 + d);
}

fn drain(c: &mut Core) -> AppOutputs {
    let mut o = AppOutputs::default();
    c.drain(&mut o);
    o
}

fn to_prelude(c: &mut Core) {
    c.set_sfx_ready();
    for _ in 0..LOADING_MIN_TICKS {
        c.tick(0);
    }
    assert!(matches!(c.screen, Screen::Prelude { .. }));
}

/// Loading → prelude → (key) → title options menu.
fn at_menu(c: &mut Core) {
    to_prelude(c);
    press(c, key::SPACE);
    assert!(matches!(c.screen, Screen::Menu));
    assert_eq!(c.top(), Some(MenuId::Options));
}

fn sel(c: &Core) -> usize {
    c.stack.last().unwrap().sel
}

fn lines(c: &Core) -> Vec<MenuLine> {
    let mut l = Vec::new();
    c.scene(&mut l);
    l
}

fn has_text(c: &Core, t: &TextRef) -> bool {
    lines(c).iter().any(|l| &l.text == t)
}

fn info(c: &Core) -> SceneInfo {
    c.scene(&mut Vec::new())
}

/// Playing without starting the sim (flow Idle: sim.tick is a no-op).
fn fake_playing(c: &mut Core) {
    at_menu(c);
    c.stack.clear();
    c.screen = Screen::Playing;
}

#[test]
fn startup_requests_and_title_music() {
    let mut c = core();
    let o = drain(&mut c);
    assert_eq!(o.requests[0], HostRequest::ApplyVideo(Config::default()));
    assert_eq!(o.requests[1], HostRequest::ApplyAudio { sound_mode: 2, sfx_volume: 8, music_volume: 7 });
    assert_eq!(o.music, Some(Some(MusicTrack::Title)));
    c.tick(0);
    let o = drain(&mut c);
    assert!(o.requests.is_empty() && o.music.is_none(), "music only on change");
    let mut q = core_with(Config { sound_mode: 1, ..Config::default() });
    assert_eq!(drain(&mut q).music, Some(None));
}

#[test]
fn loading_waits_for_sfx_bank_and_min_ticks() {
    let mut c = core();
    for _ in 0..30 {
        c.tick(0);
    }
    assert!(matches!(c.screen, Screen::Loading { .. }));
    assert_eq!(lines(&c)[0].text, TextRef::Key(keys::LOAD_WAIT));
    assert_eq!(info(&c).background, Background::Chat);
    c.set_sfx_ready();
    c.tick(0);
    assert!(matches!(c.screen, Screen::Prelude { scroll: messages::PRELUDE_SCROLL }));
    let mut d = core();
    d.set_sfx_ready();
    for _ in 0..LOADING_MIN_TICKS - 1 {
        d.tick(0);
    }
    assert!(matches!(d.screen, Screen::Loading { .. }));
    d.tick(0);
    assert!(matches!(d.screen, Screen::Prelude { .. }));
}

#[test]
fn prelude_scrolls_and_any_key_but_alt_continues() {
    let mut c = core();
    to_prelude(&mut c);
    let y0 = c.sim.texts.slots[0].y;
    c.tick(0);
    assert_eq!(c.sim.texts.slots[0].y, y0 - 256);
    assert!(info(&c).sim_texts);
    press(&mut c, key::ALT);
    assert!(matches!(c.screen, Screen::Prelude { .. }));
    ev(&mut c, InputEvent::MouseDown { x: 10.0, y: 10.0, button: 1 });
    assert!(matches!(c.screen, Screen::Menu));
    assert_eq!(c.cheatpermit, 0);
    // the options menu wipes the text table and starts on "Fire - Play"
    assert_eq!(c.sim.texts.visible().count(), 0);
    assert_eq!(sel(&c), 3);
}

#[test]
fn prelude_cheat_needs_alt_and_middle_button() {
    let mut c = core();
    to_prelude(&mut c);
    c.push_event(InputEvent::MouseDown { x: 0.0, y: 0.0, button: 3 });
    c.tick(HELD_ALT);
    assert_eq!(c.cheatpermit, 1);
    assert!(matches!(c.screen, Screen::Prelude { scroll } if scroll == messages::PRELUDE_CHEAT_SCROLL - 1));
    press(&mut c, key::Z);
    assert!(matches!(c.screen, Screen::Menu));
    assert_eq!(c.cheatpermit, 1);
    // Fire then waits for the zone cheat digit (silently, GameScreen frame)
    press(&mut c, key::SPACE);
    assert!(matches!(c.screen, Screen::Starting { .. }));
    assert_eq!(info(&c).background, Background::GameFrame);
    let mut d = core();
    to_prelude(&mut d);
    ev(&mut d, InputEvent::MouseDown { x: 0.0, y: 0.0, button: 3 });
    assert_eq!(d.cheatpermit, 0);
    assert!(matches!(d.screen, Screen::Menu));
}

#[test]
fn quit_confirmation_from_prelude_and_menu() {
    let mut c = core();
    to_prelude(&mut c);
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Quit { .. }));
    assert!(!info(&c).sim_texts, "prelude texts hidden but kept");
    assert_eq!(lines(&c)[0].text, TextRef::Key(keys::MENU_QUIT_TITLE));
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Prelude { .. }));
    assert!(c.sim.texts.visible().count() > 8);
    press(&mut c, key::SPACE);
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Quit { .. }));
    ev(&mut c, InputEvent::Pad { button: pad::B });
    assert!(matches!(c.screen, Screen::Menu));
    press(&mut c, key::ESCAPE);
    drain(&mut c);
    press(&mut c, key::ENTER);
    assert!(c.wants_quit());
    assert_eq!(drain(&mut c).requests, vec![HostRequest::Quit]);
    // nothing happens any more
    press(&mut c, key::ESCAPE);
    assert!(drain(&mut c).requests.is_empty());
    let mut d = core();
    d.push_event(InputEvent::CloseRequest);
    d.tick(0);
    assert!(d.wants_quit());
}

#[test]
fn menu_navigation_by_digits_arrows_and_back() {
    let mut c = core();
    at_menu(&mut c);
    digit(&mut c, 1);
    assert_eq!(c.top(), Some(MenuId::Controls));
    press(&mut c, key::ESCAPE);
    assert_eq!(c.top(), Some(MenuId::Options));
    digit(&mut c, 2);
    assert_eq!(c.top(), Some(MenuId::Tune));
    // tunegame ignores Fire
    press(&mut c, key::SPACE);
    assert_eq!(c.top(), Some(MenuId::Tune));
    digit(&mut c, 1);
    assert_eq!(c.top(), Some(MenuId::Sound));
    assert_eq!(sel(&c), 2, "cursor on the current sound mode");
    press(&mut c, key::ESCAPE);
    ev(&mut c, InputEvent::MouseDown { x: 0.0, y: 0.0, button: 2 });
    assert_eq!(c.top(), Some(MenuId::Options));
    assert_eq!(c.stack.len(), 1);
    // digits do not move the highlight; arrows and the bound up/down keys do
    assert_eq!(sel(&c), 3);
    press(&mut c, key::DOWN);
    assert_eq!(sel(&c), 0);
    c.push_event(InputEvent::Key { code: key::UP, ch: None, echo: true });
    c.tick(0);
    assert_eq!(sel(&c), 3);
    press(&mut c, key::SEMICOLON);
    assert_eq!(sel(&c), 2);
    assert_eq!(info(&c).highlight, Some([22.0, 151.0, 298.0, 169.0]));
    press(&mut c, key::ENTER);
    assert_eq!(c.top(), Some(MenuId::Zone));
    ev(&mut c, InputEvent::Pad { button: pad::B });
    // auto-repeat never activates
    c.push_event(InputEvent::Key { code: key::KEY_0 + 2, ch: Some('2'), echo: true });
    c.tick(0);
    assert_eq!(c.top(), Some(MenuId::Options));
    // ESC on the title menu asks before quitting
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Quit { .. }));
}

#[test]
fn mouse_hover_and_click_use_content_coordinates() {
    let mut c = core();
    c.set_view(400.0, 300.0);
    // the menu band is centred in the larger view
    assert_eq!((c.lay.hx, c.lay.cy), (40, 22));
    at_menu(&mut c);
    // the first report (cursor resting where the window opened) never moves the highlight
    let before = sel(&c);
    ev(&mut c, InputEvent::MouseMove { x: 40.0 + 100.0, y: 22.0 + 126.0 });
    assert_eq!(sel(&c), before);
    ev(&mut c, InputEvent::MouseMove { x: 40.0 + 100.0, y: 22.0 + 125.0 });
    assert_eq!(sel(&c), 1);
    ev(&mut c, InputEvent::MouseMove { x: 40.0 + 100.0, y: 22.0 + 200.0 });
    assert_eq!(sel(&c), 1, "no line there");
    // slow sub-pixel motion adds up from the last anchor instead of being dropped event by event
    let mut y = 22.0 + 200.0;
    while y > 22.0 + 150.0 {
        y -= 0.25;
        ev(&mut c, InputEvent::MouseMove { x: 40.0 + 100.0, y });
    }
    assert_eq!(sel(&c), 2, "the cursor crept onto the next line");
    ev(&mut c, InputEvent::MouseDown { x: 40.0 + 60.0, y: 22.0 + 162.0, button: 1 });
    assert_eq!(c.top(), Some(MenuId::Zone));
    ev(&mut c, InputEvent::MouseDown { x: 0.0, y: 0.0, button: 5 });
    assert_eq!(sel(&c), 1);
}

#[test]
fn define_keys_flow_and_abort() {
    let mut c = core();
    at_menu(&mut c);
    digit(&mut c, 1);
    digit(&mut c, 1);
    assert!(matches!(c.screen, Screen::DefineKeys { step: 0, .. }));
    assert_eq!(lines(&c)[0].text, TextRef::Key(keys::KEY_PRESS_LEFT));
    let new = [kc('a'), kc('d'), kc('w'), kc('s'), key::SPACE];
    for (i, k) in new.iter().enumerate() {
        press(&mut c, *k);
        if i < 4 {
            assert_eq!(lines(&c)[0].text, TextRef::Key(KEY_PROMPTS[i + 1]));
        }
    }
    assert_eq!(c.config.keys, new);
    assert!(matches!(c.screen, Screen::Menu));
    assert_eq!(c.top(), Some(MenuId::Controls));
    assert!(has_text(&c, &TextRef::Raw("SPACE".into())));
    // ESC aborts and keeps the old keys; F11 is never bound
    digit(&mut c, 1);
    press(&mut c, kc('q'));
    press(&mut c, key::F1 + 10);
    assert_eq!(c.config.display_mode, display::MODE_WINDOW, "F11 toggled the display mode");
    press(&mut c, key::ESCAPE);
    assert_eq!(c.config.keys, new);
    assert_eq!(c.top(), Some(MenuId::Controls));
    // 3. Reset Keys
    digit(&mut c, 3);
    assert_eq!(c.config.keys, DEFAULT_KEYS);
    // the bound fire key leaves choosecontrol like readopt returning 0
    c.config.keys[4] = key::X;
    press(&mut c, key::X);
    assert_eq!(c.top(), Some(MenuId::Options));
}

#[test]
fn sound_and_volume_emit_audio_requests() {
    let mut c = core();
    at_menu(&mut c);
    digit(&mut c, 2);
    digit(&mut c, 1);
    drain(&mut c);
    digit(&mut c, 1);
    let o = drain(&mut c);
    assert_eq!(c.config.sound_mode, 0);
    assert!(o.requests.contains(&HostRequest::ApplyAudio { sound_mode: 0, sfx_volume: 8, music_volume: 7 }));
    assert!(o.requests.contains(&HostRequest::StopSfx));
    assert_eq!(o.music, Some(None));
    assert_eq!(lines(&c).iter().filter_map(|l| l.prefix).collect::<String>(), "✓✗✗");
    digit(&mut c, 3);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::Title)));
    press(&mut c, key::ESCAPE);
    digit(&mut c, 2);
    assert_eq!(c.top(), Some(MenuId::Volume));
    drain(&mut c);
    digit(&mut c, 1);
    let o = drain(&mut c);
    assert_eq!(c.config.sfx_volume, 9);
    assert!(o.requests.contains(&HostRequest::ApplyAudio { sound_mode: 2, sfx_volume: 9, music_volume: 7 }));
    assert_eq!(o.sfx.len(), 1, "test sound");
    for _ in 0..5 {
        digit(&mut c, 1);
    }
    assert_eq!(c.config.sfx_volume, 10);
    digit(&mut c, 4);
    assert_eq!(c.config.music_volume, 6);
    assert_eq!(drain(&mut c).sfx.len(), 5, "no test sound for music changes");
    assert!(has_text(&c, &TextRef::Raw("VOL_FX_LEVEL 10    VOL_MUSIC_LEVEL 6".into())));
    // no effects at all in sound mode 0
    c.config.sound_mode = 0;
    digit(&mut c, 2);
    assert!(drain(&mut c).sfx.is_empty());
}

/// Opens Tune Video from the title menu on a 2560x1080 monitor.
fn at_video_menu(c: &mut Core) {
    ev(c, InputEvent::Monitor { width: 2560, height: 1080 });
    at_menu(c);
    digit(c, 2);
    digit(c, 3);
    assert_eq!(c.top(), Some(MenuId::Video));
    drain(c);
}

#[test]
fn display_mode_rotation_and_fullscreen_keys() {
    let mut c = core();
    at_video_menu(&mut c);
    assert_eq!(c.config.display_mode, display::MODE_BORDERLESS);
    let mut seen = Vec::new();
    for _ in 0..3 {
        digit(&mut c, 1);
        seen.push(c.config.display_mode);
        assert_eq!(drain(&mut c).requests, vec![HostRequest::ApplyVideo(c.config.clone())]);
    }
    assert_eq!(seen, [display::MODE_FULLSCREEN, display::MODE_WINDOW, display::MODE_BORDERLESS]);
    // F11 / Alt+Enter: back to a window, then to the last full-screen mode used
    press(&mut c, key::F1 + 10);
    assert_eq!(c.config.display_mode, display::MODE_WINDOW);
    press(&mut c, key::F1 + 10);
    assert_eq!(c.config.display_mode, display::MODE_BORDERLESS);
    digit(&mut c, 1);
    assert_eq!(c.config.display_mode, display::MODE_FULLSCREEN);
    // Alt+Enter as the host sends it, with Alt released before the tick polled the keys
    press(&mut c, crate::input::alt_enter_as_f11(key::ENTER, true));
    assert_eq!(c.config.display_mode, display::MODE_WINDOW);
    press(&mut c, crate::input::alt_enter_as_f11(key::KP_ENTER, true));
    assert_eq!(c.config.display_mode, display::MODE_FULLSCREEN, "the last full-screen mode");
    assert_eq!(c.top(), Some(MenuId::Video), "the keys never activate menu rows");
}

#[test]
fn window_aspect_and_resolutions() {
    let mut c = core();
    at_video_menu(&mut c);
    // borderless: aspect and resolution follow the monitor and their rows are inactive
    assert!(has_text(&c, &TextRef::Raw("3. 2560 x 1080".into())));
    digit(&mut c, 2);
    digit(&mut c, 3);
    assert_eq!((c.config.window_aspect, c.config.window_resolution), (display::ASPECT_DEFAULT, (1280, 720)));
    assert!(drain(&mut c).requests.is_empty());
    // window mode
    digit(&mut c, 1);
    digit(&mut c, 1);
    assert_eq!(c.config.display_mode, display::MODE_WINDOW);
    drain(&mut c);
    assert!(has_text(&c, &TextRef::Raw("3. 1280 x 720".into())));
    // 16:9 → 21:9 keeps the window about as tall (the wanted 1280x720 resolves to 1720x720)
    let res = |c: &Core| display::video(&c.config, c.monitor).resolution;
    digit(&mut c, 2);
    assert_eq!((c.config.window_aspect, res(&c)), (3, (1720, 720)));
    assert_eq!(drain(&mut c).requests, vec![HostRequest::ApplyVideo(c.config.clone())]);
    // every 21:9 resolution up to the monitor, then the smallest again
    let mut seen = Vec::new();
    for _ in 0..4 {
        digit(&mut c, 3);
        seen.push(c.config.window_resolution);
    }
    assert_eq!(seen, [(1920, 800), (2560, 1080), (1280, 540), (1720, 720)]);
    assert!(has_text(&c, &TextRef::Raw("3. 1720 x 720".into())));
    // 21:9 → 4:3 → 16:10 → 16:9 → 21:9: the chosen size is never lost
    let mut seen = Vec::new();
    for _ in 0..4 {
        digit(&mut c, 2);
        seen.push((c.config.window_aspect, res(&c)));
    }
    assert_eq!(seen, [(0, (960, 720)), (1, (1152, 720)), (2, (1280, 720)), (3, (1720, 720))]);
    // a full-height choice comes back after a whole cycle
    c.config.window_aspect = 2;
    c.config.window_resolution = (1920, 1080);
    let mut seen = Vec::new();
    for _ in 0..4 {
        digit(&mut c, 2);
        seen.push(res(&c));
    }
    assert_eq!(seen, [(2560, 1080), (1440, 1080), (1680, 1050), (1920, 1080)]);
}

#[test]
fn highlight_stays_on_the_same_option_when_rows_change() {
    let mut c = core();
    at_video_menu(&mut c);
    let act = |c: &Core| {
        let e = c.entries(MenuId::Video);
        let s = menus::selectable(&e);
        e[s[c.stack.last().unwrap().sel]].act
    };
    // window mode, highlight on Scale (row 4), then '1' → borderless: rows 2/3 disappear
    c.config.display_mode = display::MODE_WINDOW;
    for _ in 0..3 {
        press(&mut c, key::DOWN);
    }
    assert_eq!(act(&c), Some(Act::ScreenScale));
    digit(&mut c, 1);
    assert_eq!(c.config.display_mode, display::MODE_BORDERLESS);
    assert_eq!(act(&c), Some(Act::ScreenScale));
    // F11 back to a window: rows 2/3 come back, the highlight stays on Scale
    press(&mut c, key::F1 + 10);
    assert_eq!(c.config.display_mode, display::MODE_WINDOW);
    assert_eq!(act(&c), Some(Act::ScreenScale));
    let before = (c.config.window_aspect, c.config.vsync, c.config.screen_scale);
    press(&mut c, key::ENTER);
    assert_eq!((c.config.window_aspect, c.config.vsync), (before.0, before.1));
    assert_ne!(c.config.screen_scale, before.2);
}

#[test]
fn scale_cycles_up_to_the_resolution_height() {
    let mut c = core();
    at_video_menu(&mut c);
    // borderless on 1080 lines: 1.0x .. 3.0x
    let mut seen = Vec::new();
    for _ in 0..5 {
        digit(&mut c, 4);
        seen.push(c.config.screen_scale);
    }
    assert_eq!(seen, [3, 4, 5, 6, 2]);
    assert!(has_text(&c, &TextRef::Raw("1.0VIDEO_SCALE_UNIT".into())));
    // a 1280x540 window allows up to 2.0x (512 lines)
    c.config.display_mode = display::MODE_WINDOW;
    c.config.window_aspect = 3;
    c.config.window_resolution = (1280, 540);
    c.config.screen_scale = 6;
    assert!(has_text(&c, &TextRef::Raw("2.0VIDEO_SCALE_UNIT".into())), "shown as the scale used");
    digit(&mut c, 4);
    assert_eq!(c.config.screen_scale, 2, "wraps from the largest usable scale");
    digit(&mut c, 4);
    digit(&mut c, 4);
    assert_eq!(c.config.screen_scale, 4);
}

#[test]
fn smaller_monitor_limits_the_lists() {
    let mut c = core();
    ev(&mut c, InputEvent::Monitor { width: 1280, height: 720 });
    at_menu(&mut c);
    digit(&mut c, 2);
    digit(&mut c, 3);
    c.config.display_mode = display::MODE_WINDOW;
    digit(&mut c, 2);
    // 21:9 on a 1280x720 monitor: only 1280x540 fits
    let res = |c: &Core| display::video(&c.config, c.monitor).resolution;
    assert_eq!((c.config.window_aspect, res(&c)), (3, (1280, 540)));
    digit(&mut c, 3);
    assert_eq!(res(&c), (1280, 540));
    // a 1024x768 monitor cannot show 21:9 at all: the aspect cycle skips it
    ev(&mut c, InputEvent::Monitor { width: 1024, height: 768 });
    c.config.window_aspect = 2;
    digit(&mut c, 2);
    assert_eq!(c.config.window_aspect, 0, "16:9 → 4:3, 21:9 skipped");
    let r = res(&c);
    assert!(r.0 <= 1024 && r.1 <= 768, "{r:?}");
}

#[test]
fn video_toggles_apply() {
    let mut c = core();
    at_video_menu(&mut c);
    digit(&mut c, 5);
    assert!(!c.config.smooth_motion);
    digit(&mut c, 6);
    assert!(!c.config.vsync);
    assert_eq!(drain(&mut c).requests.len(), 2);
}

#[test]
fn larger_view_centres_menus_and_keeps_texts_in_place() {
    let mut c = core();
    at_menu(&mut c);
    let y0 = lines(&c)[1].y;
    c.set_view(1365.0, 768.0);
    assert_eq!((c.lay.cw, c.lay.ch, c.lay.hx, c.lay.cy, c.lay.hy), (1365, 768, 522, 256, 512));
    assert_eq!(lines(&c)[1].y, y0 + 256.0);
    // texts already on screen move with the centred band
    c.sim.texts.message(100, 50, 0.0, 0.0, TextRef::Raw("A".into()), Align::Orig);
    let before = c.sim.texts.visible().next().map(|s| (s.x >> 8, s.y >> 8));
    c.set_view(607.0, 256.0);
    let after = c.sim.texts.visible().next().map(|s| (s.x >> 8, s.y >> 8));
    assert_eq!(after, before.map(|(x, y)| (x + 143 - 522, y - 256)));
}

#[test]
fn language_change_is_instant() {
    let mut c = core();
    at_menu(&mut c);
    digit(&mut c, 2);
    digit(&mut c, 4);
    assert_eq!(c.top(), Some(MenuId::Language));
    digit(&mut c, 3);
    assert_eq!(c.lang(), Lang::ZhCn);
    assert_eq!(c.config.language, Some(Lang::ZhCn));
    assert_eq!(lines(&c).iter().filter_map(|l| l.prefix).collect::<String>(), "✗✗✓");
    ev(&mut c, InputEvent::Pad { button: pad::A });
    assert_eq!(c.lang(), Lang::En, "A activates the highlighted English item");
}

#[test]
fn zone_menu_and_id_lock() {
    let mut c = core();
    at_menu(&mut c);
    digit(&mut c, 3);
    assert_eq!(c.top(), Some(MenuId::Zone));
    assert!(!has_text(&c, &TextRef::Raw("4. ZONE_SAVED".into())));
    digit(&mut c, 4);
    assert_eq!(c.top(), Some(MenuId::Zone), "no saved game");
    digit(&mut c, 3);
    assert_eq!(c.top(), Some(MenuId::IdLock));
    assert_eq!(lines(&c)[0].text, TextRef::Key(keys::ZONE_ID_LOCK_1));
    assert!(info(&c).highlight.is_none());
    digit(&mut c, 0);
    assert_eq!(c.top(), Some(MenuId::Options));
    assert_eq!(c.mentalzone, 1);
    digit(&mut c, 3);
    digit(&mut c, 2);
    assert_eq!((c.mentalzone, c.config.mental_zone, c.top()), (2, 2, Some(MenuId::Options)));
    c.config.id_permit = true;
    digit(&mut c, 3);
    assert_eq!(sel(&c), 1, "cursor on the current zone");
    digit(&mut c, 3);
    assert_eq!((c.mentalzone, c.config.mental_zone), (3, 3));
    // Fire leaves the zone menu (the C code set mentalzone = 0)
    digit(&mut c, 3);
    press(&mut c, key::SPACE);
    assert_eq!((c.top(), c.mentalzone), (Some(MenuId::Options), 3));
}

#[test]
fn pause_menu_transitions() {
    let mut c = core();
    fake_playing(&mut c);
    c.tick(0);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::Zone { zone: 0, part: 1 })));
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Paused { anim, sel: 2 } if anim == messages::PAUSE_ANIM - 1));
    let o = drain(&mut c);
    assert!(o.requests.contains(&HostRequest::StopSfx));
    assert_eq!(o.music, Some(None));
    assert_eq!(c.sim.texts.visible().count(), 6);
    assert!(info(&c).highlight.is_none(), "no bar while sliding in");
    // auto-repeat of ESC must not abandon
    c.push_event(InputEvent::Key { code: key::ESCAPE, ch: None, echo: true });
    for _ in 0..70 {
        c.tick(0);
    }
    assert!(matches!(c.screen, Screen::Paused { anim: 0, .. }));
    assert_eq!(info(&c).highlight, Some(menus::highlight_rect(0, 168.0)));
    letter(&mut c, 'r');
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!(c.sim.texts.visible().count(), 0);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::Zone { zone: 0, part: 1 })));
    // Q: lose this life (the physical key when no character is produced)
    c.sim.plstrength = 1000;
    press(&mut c, key::ESCAPE);
    ev(&mut c, InputEvent::Key { code: kc('q'), ch: Some('ㅂ'), echo: false });
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!(c.sim.plstrength, 0);
    // O: in-game options, Fire returns
    press(&mut c, key::ESCAPE);
    drain(&mut c);
    letter(&mut c, 'o');
    assert!(c.gameon && c.top() == Some(MenuId::Options));
    assert!(has_text(&c, &TextRef::Raw("3. MENU_SAVE_POSITION".into())));
    assert!(has_text(&c, &TextRef::Key(keys::MENU_FIRE_RETURN)));
    assert_eq!(drain(&mut c).music, None, "still silent");
    digit(&mut c, 2);
    digit(&mut c, 2);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::Title)), "tunevolume plays the title music");
    press(&mut c, key::ESCAPE);
    // C: the music tunevolume started keeps playing in the in-game menus until rejoin
    assert_eq!(drain(&mut c).music, None);
    press(&mut c, key::ESCAPE);
    press(&mut c, key::ENTER);
    assert!(matches!(c.screen, Screen::Playing));
    assert!(!c.gameon);
    // O then ESC also returns to the game
    press(&mut c, key::ESCAPE);
    letter(&mut c, 'o');
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Playing));
    // ESC ESC: abandon → title menu, no high score
    press(&mut c, key::ESCAPE);
    press(&mut c, key::ESCAPE);
    assert!(matches!(c.screen, Screen::Menu));
    assert!(!c.gameon && c.top() == Some(MenuId::Options));
    assert_eq!(c.sim.flow, Flow::Idle);
    assert!(has_text(&c, &TextRef::Key(keys::MENU_FIRE_PLAY)));
}

#[test]
fn pause_menu_mouse_and_pad() {
    let mut c = core();
    fake_playing(&mut c);
    ev(&mut c, InputEvent::Pad { button: pad::START });
    assert!(matches!(c.screen, Screen::Paused { .. }));
    ev(&mut c, InputEvent::MouseMove { x: 100.0, y: 139.0 });
    ev(&mut c, InputEvent::MouseMove { x: 100.0, y: 138.0 });
    assert!(matches!(c.screen, Screen::Paused { sel: 1, .. }));
    press(&mut c, key::DOWN);
    assert!(matches!(c.screen, Screen::Paused { sel: 2, .. }));
    ev(&mut c, InputEvent::Pad { button: pad::B });
    assert!(matches!(c.screen, Screen::Playing));
    press(&mut c, key::PAUSE);
    ev(&mut c, InputEvent::MouseDown { x: 100.0, y: 199.0, button: 1 });
    assert!(c.gameon && c.top() == Some(MenuId::Options));
}

#[test]
fn cheat_keys_apply_on_every_tick_they_are_held() {
    use crate::sim::consts::STRENGTHINIT;
    // input::HELD_F1 / HELD_F3 (bits 6 and 8 of the held mask)
    const F1_HELD: u32 = 1 << 6;
    const F3_HELD: u32 = 1 << 8;
    let mut c = core();
    at_menu(&mut c);
    c.cheatpermit = 1;
    press(&mut c, key::ENTER);
    digit(&mut c, 0);
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!(c.sim.cheatpermit, 1);
    // keyboard.c cheatread polls osbyte_81(-F1/-F3) every frame: getmpmg + prepstrength on
    // every tick the keys stay down (not once per key-down / auto-repeat event)
    for i in 0..8 {
        c.sim.plweaponspeed = 0;
        c.sim.blamctr = 99;
        c.sim.plstrength = STRENGTHINIT / 2;
        c.sim.lagerctr = 7;
        c.tick(F1_HELD | F3_HELD);
        assert!(matches!(c.screen, Screen::Playing), "tick {i}");
        assert_eq!((c.sim.plweaponspeed, c.sim.firerate, c.sim.blamctr), (8, 4, 0), "getmpmg on tick {i}");
        assert_eq!((c.sim.plstrength, c.sim.lagerctr), (STRENGTHINIT, 0), "prepstrength on tick {i}");
    }
    // released: no cheat
    c.sim.plweaponspeed = 0;
    c.sim.plstrength = STRENGTHINIT / 2;
    c.tick(0);
    assert_eq!(c.sim.plweaponspeed, 0);
    assert_ne!(c.sim.plstrength, STRENGTHINIT);
    // a key-down event alone neither applies the cheat nor pauses (only the held mask counts)
    c.push_event(InputEvent::Key { code: key::F1, ch: None, echo: false });
    c.push_event(InputEvent::Key { code: key::F1 + 2, ch: None, echo: true });
    c.tick(0);
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!(c.sim.plweaponspeed, 0);
    // without cheat permission the held keys do nothing
    let mut d = core();
    at_menu(&mut d);
    press(&mut d, key::ENTER);
    assert_eq!(d.sim.cheatpermit, 0);
    d.sim.plweaponspeed = 0;
    d.tick(F1_HELD | F3_HELD);
    assert_eq!(d.sim.plweaponspeed, 0);
}

#[test]
fn high_score_entry_after_a_lost_game() {
    let mut c = core();
    fake_playing(&mut c);
    drain(&mut c);
    c.sim.plscore = [0, 0, 0, 1, 2, 0, 0, 0];
    c.game_over(false);
    assert!(drain(&mut c).requests.contains(&HostRequest::StopSfx));
    c.tick(0);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::GameOver)));
    let Screen::HighScore(hs) = &c.screen else { panic!() };
    assert_eq!(hs.hstindex, 0);
    let l = lines(&c);
    assert_eq!(l[1].text, TextRef::Raw("00012000 PSY".into()));
    assert_eq!((l[6].text.clone(), l[6].x, l[6].y), (TextRef::Raw("←".into()), 280.0, 64.0));
    assert_eq!(info(&c).highlight, Some(menus::highlight_rect(0, 64.0)));
    assert!(!has_text(&c, &TextRef::Key(keys::HS_PRESS_FIRE)));
    // right held: P → R; Fire (the bound key, Space or Enter) confirms each letter
    for _ in 0..12 {
        c.tick(HELD_RIGHT);
    }
    press(&mut c, key::ENTER);
    for _ in 0..11 {
        c.tick(HELD_LEFT);
    }
    press(&mut c, key::SPACE);
    for _ in 0..12 {
        c.tick(0);
    }
    press(&mut c, key::ESCAPE); // ESC does not confirm a letter
    press(&mut c, key::ENTER);
    for _ in 0..20 {
        c.tick(0);
    }
    assert_eq!(c.config.initials, *b"RRY");
    let Screen::HighScore(hs) = &c.screen else { panic!() };
    assert_eq!(hs.phase, HsPhase::Done);
    assert_eq!(&hs.table[0..13], b"00012000 RRY\n");
    assert!(has_text(&c, &TextRef::Key(keys::HS_PRESS_FIRE)));
    press(&mut c, key::ESCAPE);
    assert_eq!(c.top(), Some(MenuId::Options));
    assert_eq!(drain(&mut c).music, None, "the game over tune continues in the menus");
}

#[test]
fn completed_game_unlocks_the_id() {
    let mut c = core();
    fake_playing(&mut c);
    c.sim.plscore = [9, 9, 9, 9, 9, 9, 9, 9];
    c.sim.mentalzone = 2;
    c.game_over(true);
    assert!(c.config.id_permit);
    assert_eq!((c.mentalzone, c.config.mental_zone), (2, 2));
    c.tick(0);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::Title)));
    let Screen::HighScore(hs) = &mut c.screen else { panic!() };
    assert_eq!(hs.zone, 2);
    hs.phase = HsPhase::Done;
    assert!(has_text(&c, &TextRef::Key(keys::MSG_ID_UNLOCKED)));
}

#[test]
fn layout_change_moves_menu_texts() {
    let mut c = core();
    to_prelude(&mut c);
    let x0 = c.sim.texts.slots[0].x;
    c.set_view(454.0, 256.0);
    assert_eq!(c.sim.texts.slots[0].x, x0 + (67 << 8));
    assert_eq!(c.sim.lay.hx, 67);
}

#[test]
fn storage_persists_settings_and_scores() {
    let dir = std::env::temp_dir().join(format!("asylum-frontend-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let st = Storage::new(dir.clone());
    let assets = crate::assets::testdata::assets();
    let mut c = Core::new(assets, Arc::new(Strings::default()), Some(st.clone()), Config::default(), (320.0, 256.0), 1);
    at_menu(&mut c);
    digit(&mut c, 2);
    digit(&mut c, 4);
    digit(&mut c, 2);
    assert_eq!(st.load_config().language, Some(Lang::Ko));
    c.sim.plscore = [0, 0, 5, 0, 0, 0, 0, 0];
    c.game_over(false);
    for _ in 0..3 {
        press(&mut c, key::ENTER);
        for _ in 0..12 {
            c.tick(0);
        }
    }
    for _ in 0..12 {
        c.tick(0);
    }
    assert_eq!(&st.load_highscores(1)[0..13], b"00500000 PSY\n");
    let _ = std::fs::remove_dir_all(&dir);
}

struct FakeTex;

impl crate::render::TexSource for FakeTex {
    fn spr(&self, _: Bank, _: usize) -> Option<AtlasSprite> {
        Some(AtlasSprite { u: 0, v: 0, w: 12, h: 14, xc: 6, yc: 7 })
    }
    fn glyph_spr(&self, _: u32) -> Option<AtlasSprite> {
        None
    }
    fn img(&self, _: Img) -> AtlasRect {
        AtlasRect { u: 0, v: 0, w: 320, h: 256 }
    }
}

fn render(c: &Core) -> RenderFrame {
    let mut lines = Vec::new();
    let info = c.scene(&mut lines);
    let scene = crate::render::SceneDesc {
        lay: c.lay,
        lang: c.lang(),
        background: info.background,
        sim: (info.background == Background::Game).then_some(&c.sim),
        texts: if info.sim_texts { &c.sim.texts } else { &c.empty_texts },
        lines: &lines,
        highlight: info.highlight,
        snap: false,
        no_interp: !c.config.smooth_motion,
    };
    let mut f = RenderFrame::default();
    crate::render::build_frame_with(&scene, &FakeTex, &c.strings, &crate::font::GlyphSet::default(), &mut f);
    f
}

#[test]
fn every_screen_builds_a_frame() {
    let mut c = core();
    assert!(!render(&c).quads.is_empty());
    at_menu(&mut c);
    assert!(render(&c).quads.len() > 20);
    for d in [1, 2, 3] {
        digit(&mut c, d);
        assert!(!render(&c).quads.is_empty());
        press(&mut c, key::ESCAPE);
    }
    press(&mut c, key::ESCAPE);
    assert!(!render(&c).quads.is_empty());
    press(&mut c, key::ESCAPE);
    c.stack.clear();
    c.screen = Screen::Playing;
    press(&mut c, key::ESCAPE);
    for _ in 0..64 {
        c.tick(0);
    }
    assert!(!render(&c).quads.is_empty());
    c.game_over(false);
    assert!(!render(&c).quads.is_empty());
}

#[test]
fn real_game_start_play_and_game_over() {
    let mut c = core();
    at_menu(&mut c);
    drain(&mut c);
    press(&mut c, key::ENTER);
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!(c.sim.flow, Flow::Running);
    assert_eq!(drain(&mut c).music, Some(Some(MusicTrack::Zone { zone: 0, part: 1 })));
    let mut sfx = 0;
    for i in 0..400 {
        c.tick(if i % 60 < 30 { HELD_RIGHT | crate::input::HELD_FIRE } else { HELD_LEFT | crate::input::HELD_UP });
        sfx += drain(&mut c).sfx.len();
    }
    assert!(c.sim.framectr >= 400);
    assert!(sfx > 0, "sim sound events are forwarded");
    // lives == 0 and the snuff counter elapsed: player_dead() == 2
    c.sim.lives = 0;
    c.sim.snuffctr = 300;
    c.tick(0);
    assert!(matches!(c.screen, Screen::HighScore(_)), "{:?}", std::mem::discriminant(&c.screen));
    assert!(drain(&mut c).requests.contains(&HostRequest::StopSfx));
}

#[test]
fn save_position_then_play_the_saved_game() {
    let dir = std::env::temp_dir().join(format!("asylum-frontend-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let st = Storage::new(dir.clone());
    let assets = crate::assets::testdata::assets();
    let cfg = Config { mental_zone: 2, ..Config::default() };
    let mut c = Core::new(assets, Arc::new(Strings::default()), Some(st.clone()), cfg, (320.0, 256.0), 7);
    at_menu(&mut c);
    press(&mut c, key::ENTER);
    assert_eq!(c.sim.zone, 1, "Psyche");
    for _ in 0..50 {
        c.tick(HELD_RIGHT);
    }
    press(&mut c, key::ESCAPE);
    letter(&mut c, 'o');
    digit(&mut c, 3);
    assert!(st.savegame_exists());
    assert!(has_text(&c, &TextRef::Key(keys::MENU_POSITION_SAVED)));
    for _ in 0..SAVED_FLASH_TICKS {
        c.tick(0);
    }
    assert!(!has_text(&c, &TextRef::Key(keys::MENU_POSITION_SAVED)));
    // abandon, choose "4. Saved Game", play it
    press(&mut c, key::ENTER);
    press(&mut c, key::ESCAPE);
    press(&mut c, key::ESCAPE);
    digit(&mut c, 3);
    assert!(has_text(&c, &TextRef::Raw("4. ZONE_SAVED".into())));
    digit(&mut c, 4);
    assert_eq!((c.mentalzone, c.config.mental_zone), (4, 2), "4 is never persisted");
    press(&mut c, key::ENTER);
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!(c.sim.mentalzone, 2, "loadgame restored options.mentalzone");
    // a corrupt save returns to the zone menu
    std::fs::write(st.savegame_path(), b"xx").unwrap();
    press(&mut c, key::ESCAPE);
    press(&mut c, key::ESCAPE);
    c.mentalzone = 4;
    press(&mut c, key::ENTER);
    assert_eq!(c.top(), Some(MenuId::Zone));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cheat_mode_zone_cheat_digit() {
    let mut c = core();
    at_menu(&mut c);
    c.cheatpermit = 1;
    press(&mut c, key::ENTER);
    assert!(matches!(c.screen, Screen::Starting { .. }));
    assert_eq!(drain(&mut c).music, Some(None));
    digit(&mut c, 5);
    assert!(matches!(c.screen, Screen::Playing));
    assert_eq!((c.sim.cheatpermit, c.sim.plzone), (1, 5));
    c.tick(0);
    assert_eq!(c.sim.currentzone, 5);
    // without a digit the game starts after the type-ahead window
    let mut d = core();
    at_menu(&mut d);
    d.cheatpermit = 1;
    press(&mut d, key::ENTER);
    for _ in 0..ZONE_CHEAT_TICKS {
        d.tick(0);
    }
    assert!(matches!(d.screen, Screen::Playing));
    assert_eq!(d.sim.plzone, 0);
}
