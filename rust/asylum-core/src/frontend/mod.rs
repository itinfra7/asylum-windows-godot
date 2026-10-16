//! Frontend state machine (asylum.c `init()` loop + menus.c), running on the game thread
//! at 50 Hz. Replaces the original blocking loops with screens.
//!
//! Screens (original look: ChatScreen + message texts; English texts identical):
//! * Loading — "Please Wait / Loading Game files" until the SFX bank is ready (min 10 ticks)
//! * Prelude — vdu/menus.c `prelude()`: scrolling story (x>2000 = permanent messages moving
//!   up 1 px/tick for 264 ticks); any key → main menu; ESC → quit confirm; Alt + middle
//!   mouse button → cheat mode (scroll continues to 1024 to reveal the cheat lines).
//! * Main menu ("Options") — 1 Define Controls, 2 Tune Game Options, 3 Choose Mental Zone,
//!   "Fire - Play"; in-game variant (from the pause menu, "O"): 3 = Save Position,
//!   "Fire - Return". ESC on the title menu → quit confirmation (new).
//! * Controls: 1 Keyboard (define 5 keys in sequence, ESC aborts), 2 Gamepad (reminder
//!   screen), 3 Default keys; the current bindings are listed below the items.
//! * Tune Game: 1 Sound System (radio ✓/✗: No Sound / Effects Only / Effects and Music),
//!   2 Sound Volume (louder/quieter effects/music, live levels 0..10, test sound),
//!   3 Video System (cycling: display mode, window aspect, window resolution, view scale;
//!   toggles ✓/✗: Smooth Motion, Vsync), 4 Language (English / 한국어 / 简体中文).
//! * Zone: 1 Ego, 2 Psyche, 3 Id (locked → "To play the id / you need to complete / the ego
//!   first." until id_permit), 4 Saved Game (only when a save exists).
//! * Playing — Sim ticks; ESC → Paused.
//! * Paused — menus.c `escapehandler`: the six messages slide in for 64 ticks; ESC abandon
//!   (→ main menu, no high score), Q lose a life, R return, O options (in-game main menu).
//! * Game over → High score (menus.c showhighscore/updatehst/showhst): if the score enters
//!   the table, edit 3 initials with Left/Right (every 2 ticks while held), Fire confirms a
//!   letter; then save; "press fire" → main menu. Completing all 8 neurons unlocks the Id
//!   (permitid) and shows "WELL DONE!" texts (already emitted by the sim).
//!
//! Input: number keys exactly like `readopt` (direct activation; '0' = Fire), plus Up/Down,
//! Enter (selected line), ESC (back), mouse hover/click on menu lines, gamepad (D-pad,
//! A = accept, B = back, Start = Fire / pause). The bound Fire key does what Fire did in the
//! original menu ("Fire - Play"). Every list menu shows a highlight bar on the selected line
//! and starts on "Fire - Play", so Enter alone plays like the original. Sub-menus return to
//! the menu they were opened from. Key auto-repeat only moves the highlight.
//!
//! Layout: 320x256-space content coordinates + (`lay.hx`, `lay.cy`) (the centred band;
//! bottom-anchored texts use `lay.hy`, see messages.rs); numbered items keep the original x with
//! `Align::Orig` in every language, original single texts use `Align::EnCentered`, new texts
//! are centred on the band, so Korean / Chinese lines keep the original composition.
//! Module map: `state` (screens, events, ticks), `menus` (menu data + readopt navigation),
//! `messages` (prelude / escape menu texts), `highscore` (updatehst & initials).
//!
//! Music (when sound_mode == 2): Title on loading/prelude/menus, zone music while playing
//! (`Sim::music_part`), silence while paused, GameOver after a lost game, Title after a
//! completed game (the C code's (3,0) slot). Sound mode 0 mutes everything.

#[cfg(test)]
mod fit_tests;
mod highscore;
mod menus;
mod messages;
mod state;

use std::sync::Arc;

use crate::assets::GameAssets;
use crate::audio::music::MusicTrack;
use crate::config::Config;
use crate::i18n::{Lang, Strings};
use crate::input::InputEvent;
use crate::layout::Layout;
use crate::render::{MenuLine, RenderCtx, RenderFrame, SceneDesc};
use crate::sim::SfxEvent;
use crate::storage::Storage;

/// Requests from the frontend to the host (Godot).
#[derive(Clone, Debug, PartialEq)]
pub enum HostRequest {
    /// Apply window/video settings from this config: display mode, window aspect and
    /// resolution, view scale (`display::video` gives the result on the current monitor) and
    /// vsync. Sent once at start-up and after video changes; the host also re-applies the
    /// latest one when the window moves to another monitor.
    ApplyVideo(Config),
    /// Apply audio settings (sound mode and volumes).
    ApplyAudio { sound_mode: u8, sfx_volume: u8, music_volume: u8 },
    /// Stop all currently playing effects (entering menus etc.).
    StopSfx,
    /// Quit the application (config already saved).
    Quit,
}

/// Outputs of one or more ticks, drained by the runtime.
#[derive(Default, Debug)]
pub struct AppOutputs {
    pub sfx: Vec<SfxEvent>,
    /// `Some(None)` = stop music, `Some(Some(t))` = play track t (restart), `None` = no change.
    pub music: Option<Option<MusicTrack>>,
    pub requests: Vec<HostRequest>,
}

/// Construction parameters.
pub struct AppInit {
    pub assets: Arc<GameAssets>,
    pub ctx: RenderCtx,
    pub storage: Storage,
    pub config: Config,
    /// Initial logical viewport size.
    pub view: (f32, f32),
    /// RNG seed for the game (e.g. derived from the clock).
    pub seed: u32,
}

pub struct App {
    core: state::Core,
    ctx: RenderCtx,
    /// Reused line buffer for `render`.
    lines: Vec<MenuLine>,
}

impl App {
    /// Emits `ApplyVideo` + `ApplyAudio` once; starts on the loading screen.
    pub fn new(init: AppInit) -> App {
        let core = state::Core::new(init.assets, init.ctx.strings.clone(), Some(init.storage), init.config, init.view, init.seed);
        App { core, ctx: init.ctx, lines: Vec::with_capacity(32) }
    }

    /// Queue an input event (applied in order at the next tick).
    pub fn push_event(&mut self, ev: InputEvent) {
        self.core.push_event(ev);
    }

    /// Logical viewport size changed.
    pub fn set_view(&mut self, w: f32, h: f32) {
        self.core.set_view(w, h);
    }

    /// The SFX bank finished synthesizing (ends the loading screen).
    pub fn set_sfx_ready(&mut self) {
        self.core.set_sfx_ready();
    }

    /// Advance one 50 Hz tick with the currently held keys (`input::HELD_*` bits).
    pub fn tick(&mut self, held: u32) {
        self.core.tick(held);
    }

    /// Build the render frame for the current state (call after `tick`).
    pub fn render(&mut self, out: &mut RenderFrame) {
        let c = &self.core;
        let info = c.scene(&mut self.lines);
        let scene = SceneDesc {
            lay: c.lay,
            lang: c.lang(),
            background: info.background,
            sim: (info.background == crate::render::Background::Game).then_some(&c.sim),
            texts: if info.sim_texts { &c.sim.texts } else { &c.empty_texts },
            lines: &self.lines,
            highlight: info.highlight,
            snap: false,
            no_interp: !c.config.smooth_motion,
        };
        crate::render::build_frame(&scene, &self.ctx, out);
    }

    /// Move accumulated outputs into `out`.
    pub fn drain(&mut self, out: &mut AppOutputs) {
        self.core.drain(out);
    }

    pub fn config(&self) -> &Config {
        &self.core.config
    }

    pub fn lang(&self) -> Lang {
        self.core.lang()
    }

    /// Current layout.
    pub fn layout(&self) -> Layout {
        self.core.lay
    }

    /// Strings in use (for tests).
    pub fn strings(&self) -> &Strings {
        &self.ctx.strings
    }

    /// True once the user confirmed quitting.
    pub fn wants_quit(&self) -> bool {
        self.core.wants_quit()
    }
}
