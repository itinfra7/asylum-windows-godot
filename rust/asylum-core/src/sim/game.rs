//! asylum.c — `game()` main loop as a per-tick state machine, `setup()`, zone switching
//! (`loadzone`, `enterneuron`, `exitneuron`), level/neuron loading from the decoded assets,
//! `saveal/restoreal`, and the save game format (`savegame`/`loadgame`).
//!
//! C control flow → Rust:
//! ```text
//! game():  wipetexttab; setup; switchcolch; startmessage; startplayer; getarms;
//!          [cheat] zonecheatread                                   => start_game()
//!   zonerestart: mentalzone==4 ? loadgame : restartplayer          => zone_restart()
//!   mainrestart: showgamescreen; music; frameinc=1                 => main_restart()
//!   loop body (one frame)                                          => tick()
//!     plzone != currentzone → loadzone; goto zonerestart (zone_restart + main_restart),
//!         then the frame body runs in the same tick (C: next loop pass, right away)
//!     … plmove … plotscore; framectr += frameinc; player_dead():
//!         1 → zonerestart, 2 → return (game over)
//!   ESC → escapehandler (frontend); afterwards backprep + mainrestart => resume()
//! ```
//! Per tick bookkeeping (`begin_tick`): tick_no += 1, swap anchors, clear cmds/sfx_events,
//! remember the draw camera of the previous tick (`draw.cam`) in `cam_prev`.
//! `tick` fills `Sim::draw` at the C drawing points (camera at `mazeplot`, HUD after
//! `plotbonus`/`showstrength`), so a teleport or a restart later in the tick shows next frame.
//!
//! Save game layout (little endian, exactly the C fwrite sequence):
//! u8 mentalzone, u8 plzone, 25 bytes player state, STORE_LEN store_for_savegame,
//! STORE_LEN store_for_neuron, brain cells (w*h), neuron cells (w*h) if plzone != 0.
//!
//! Rendering-only C calls (`showgamescreen`, `mazeplot`, `backprep`, `scorewipe`,
//! `plotscore`, `switchbank`) and music calls are handled by the renderer/frontend.

use crate::sim::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// No game in progress.
    Idle,
    Running,
    /// player_dead() returned 2.
    Over,
}

/// Parameters of a new game.
#[derive(Clone, Debug, Default)]
pub struct GameStart {
    /// options.mentalzone: 1 Ego, 2 Psyche, 3 Id, 4 load saved game.
    pub mentalzone: u8,
    /// 1 = cheat mode (prelude Alt + middle mouse button).
    pub cheatpermit: u8,
    /// zonecheatread(): digit 0..=8 pressed at start in cheat mode → start in that neuron.
    pub zone_cheat: Option<i32>,
    /// Save game bytes when `mentalzone == 4`.
    pub savegame: Option<Vec<u8>>,
}

/// Bytes before the two stores: mentalzone, plzone, player state.
const SAVE_HEADER_LEN: usize = 2 + 25;

/// `currentpath` selection of getlevelfiles/getlevelsprites (4 → Psyche, "XXX" in the C).
fn zone_of_mentalzone(mentalzone: u8) -> u8 {
    match mentalzone {
        2 => 1,
        3 => 2,
        4 => 1,
        _ => 0,
    }
}

impl Sim {
    /// Starts a game (see module docs). Errors only for corrupt save games.
    pub fn start_game(&mut self, start: GameStart) -> Result<(), String> {
        self.flow = Flow::Idle;
        if start.mentalzone == 4 {
            // Reject a missing/corrupt save before anything is reset (C: silently ignored).
            match &start.savegame {
                Some(data) => {
                    self.check_savegame(data)?;
                }
                None => return Err("no saved game".into()),
            }
        }
        self.mentalzone = start.mentalzone;
        self.cheatpermit = start.cheatpermit;
        self.pending_save = start.savegame;
        // a new game starts with every SDL_mixer channel idle (the host stopped all effects)
        self.halt_all_channels();
        // init(): `currentzone = 0; getlevelfiles();` before every game.
        self.currentzone = 0;
        self.getlevelfiles();

        // game() prologue
        self.texts.wipe();
        self.setup();
        self.switchcolch();
        self.startmessage();
        self.startplayer();
        self.getarms();
        if self.cheatpermit == 1 {
            // zonecheatread(): only the digits '0'..='8' are accepted.
            if let Some(z) = start.zone_cheat.filter(|z| (0..=8).contains(z)) {
                self.plzone = z;
            }
        }
        self.zone_restart()?;
        self.main_restart();

        // Port bookkeeping: nothing of a previous game may leak into the first frame.
        self.cmds.clear();
        self.sfx_events.clear();
        self.plot_key = KEY_NONE;
        self.sync_draw();
        self.cam_prev = self.draw.cam;
        self.flow = Flow::Running;
        Ok(())
    }

    /// `zonerestart:` label of game().
    fn zone_restart(&mut self) -> Result<(), String> {
        if self.mentalzone == 4 {
            match self.pending_save.take() {
                Some(data) => self.loadgame_bytes(&data),
                None => Err("no saved game".into()),
            }
        } else {
            self.restartplayer();
            Ok(())
        }
    }

    /// `mainrestart:` label of game() (screen and music are frontend/renderer work).
    fn main_restart(&mut self) {
        self.frameinc = 1;
    }

    /// One frame of the main loop (calls `begin_tick` first). No-op unless a game is running.
    pub fn tick(&mut self, input: TickInput) -> TickResult {
        if self.flow != Flow::Running {
            return TickResult::Continue;
        }
        self.begin_tick();

        if self.plzone != self.currentzone {
            // switchzone: loadzone(); goto zonerestart -> mainrestart -> the next loop pass
            // runs a full frame at once (only swi_blitz_wait(1) in between, nothing presented).
            self.loadzone();
            // C loadgame fails silently; with mentalzone != 4 this is always Ok.
            let _ = self.zone_restart();
            self.main_restart();
            // No return: fall through into the frame body, so no sprite-less frame is
            // published and this tick's input (incl. cheat keys) is not lost. One check is
            // enough: loadzone always leaves plzone == currentzone.
        }

        self.wake_grown_view();
        // rate50link: (frameinc = 1, rate50 = 1)
        self.plmove(input);
        self.bonuscheck();
        self.fuelairproc();
        // mazeplot(xpos, ypos) → renderer; maze and every relplot/plotdying of this tick use
        // this camera (playerplot(false) may teleport afterwards)
        self.draw.cam = self.camera();
        self.switchcolch();
        self.masterplotal = true;
        self.plot_key = KEY_NONE;
        self.playerplot(true);
        self.moval();
        self.project();
        self.bullets();
        // Player/HUD plots carry no interpolation key (the loops above leave theirs set).
        self.plot_key = KEY_NONE;
        self.alfire();
        self.playerplot(false);
        self.bonusplot();
        self.scoreadd();
        self.update_show_strength();
        // plotbonus/showstrength drew these; cheatread (F3) and player_dead change them only
        // after the drawing (shown next frame)
        self.snapshot_hud();
        self.texts.animate(self.frameinc);
        self.seeifdead();
        let (xpos, ypos) = (self.xpos, self.ypos);
        self.wakeupal(xpos, ypos);
        if self.cheatpermit == 1 {
            self.cheatread(input.cheat_keys);
        }
        // scorewipe(); plotscore(); → renderer
        self.framectr = self.framectr.wrapping_add(self.frameinc);
        // switchbank() → renderer
        match self.player_dead() {
            1 => {
                let _ = self.zone_restart();
                self.main_restart();
            }
            2 => {
                self.flow = Flow::Over;
                return TickResult::GameOver { completed: self.gotallneurons() };
            }
            _ => {}
        }
        TickResult::Continue
    }

    /// Back from the escape menu (R/Q/O): `redraw_bonus(); backprep; mainrestart`.
    pub fn resume(&mut self) {
        self.redraw_bonus();
        self.main_restart();
    }

    /// `setup()`.
    pub fn setup(&mut self) {
        self.framectr = 0;
        self.plzone = 0; // _firstzone
        self.wipesoundtab();
        self.initweapon();
        self.initprojtab();
        self.initbultab();
        self.initrockettab();
        self.getvars();
        self.prepstrength();
        self.scorezero();
        // backprep(backadr) → renderer
        self.boardreg();
        self.wipealtab();
        self.prepfueltab();
    }

    /// `loadzone()`.
    pub fn loadzone(&mut self) {
        let r1 = self.currentzone;
        self.currentzone = self.plzone;
        if self.currentzone == 0 {
            self.exitneuron(r1);
        } else {
            self.enterneuron(r1);
        }
    }

    /// `change_zone(zone)`.
    pub fn change_zone(&mut self, zone: i32) {
        self.plzone = zone;
    }

    pub fn enterneuron(&mut self, r1: i32) {
        if r1 == 0 {
            self.saveal(false);
        }
        let n = self.getneuronfiles(self.plzone);
        self.plzone = n;
        self.currentzone = n;
        if self.currentzone == 0 {
            self.exitneuron(r1);
            return;
        }
        self.wipealtab();
        self.getarms();
        self.initweapon();
        self.initprojtab();
        self.initbultab();
        // backprep(backadr) → renderer
        self.boardreg();
        self.prepfueltab();
        self.startplayer();
    }

    pub fn exitneuron(&mut self, r1: i32) {
        let _ = r1;
        self.in_neuron = false; // boardadr = brainadr
        self.restoreal(false);
        self.initweapon();
        self.initprojtab();
        self.initbultab();
        self.retrievebackdrop();
        // backprep(backadr) → renderer
        self.boardreg();
        self.prepfueltab();
        self.wipesoundtab();
    }

    /// `getlevelfiles()`: select zone from `mentalzone` (4 → Psyche like the C code until
    /// loadgame overwrites mentalzone), copy the brain board, select the backdrop.
    pub fn getlevelfiles(&mut self) {
        let z = zone_of_mentalzone(self.mentalzone);
        self.zone = z;
        self.brain = self.assets.zones[z as usize].brain.clone();
        self.in_neuron = false; // boardadr = brainadr
        self.backdrop = BackdropSel { zone: z, neuron: false };
    }

    /// `retrievebackdrop()`.
    pub fn retrievebackdrop(&mut self) {
        self.backdrop = BackdropSel { zone: self.zone, neuron: false };
    }

    /// `getneuronfiles(plzone)`: neuron backdrop; find the highest existing CellN with
    /// N <= plzone; load it into `neuron` (in_neuron = true). Returns the cell number or 0.
    pub fn getneuronfiles(&mut self, plzone: i32) -> i32 {
        self.backdrop = BackdropSel { zone: self.zone, neuron: true };
        let z = self.zone as usize;
        // Only Cell1..Cell8 can exist ('0'+plzone for plzone in 9..=255 names no file), so
        // start at 8. The C loop never ends for plzone <= 0 (unreachable there): return 0.
        let mut n = plzone.min(8);
        if n <= 0 {
            return 0;
        }
        let cell = loop {
            // neuronloadloop:
            if let Some(b) = &self.assets.zones[z].neurons[(n - 1) as usize] {
                break b.clone();
            }
            n -= 1;
            if n == 0 {
                // noneuronshere:
                return 0;
            }
        };
        self.neuron = cell;
        self.in_neuron = true; // boardadr = neuronadr
        n
    }

    /// `saveal(store_for_neuron)` (`savegame == false`) or `saveal(store_for_savegame)`.
    /// Aliens that did not fit (larger views only) go back to sleep: on neuron entry their
    /// markers are written into the live brain (the screen wake-up on the way out wakes them
    /// again); for a save game they are returned for the board copy in the file.
    pub fn saveal(&mut self, savegame: bool) -> Vec<(BIdx, u8)> {
        let mut store = std::mem::take(self.store_mut(savegame));
        self.save_player(&mut store[..30]);
        let rest = self.save_alents(&mut store[30..]);
        *self.store_mut(savegame) = store;
        let mut markers: Vec<(BIdx, u8)> = Vec::new();
        for i in rest {
            if let Some((c, m)) = self.sleep_marker(i) {
                if !markers.iter().any(|&(d, _)| d == c) {
                    markers.push((c, m));
                }
            }
        }
        if !savegame {
            for &(c, m) in &markers {
                self.set_cell(c, m);
            }
        }
        markers
    }

    pub fn restoreal(&mut self, savegame: bool) {
        let mut store = std::mem::take(self.store_mut(savegame));
        if !self.restore_player(&mut store[..30]) {
            self.wipealtab();
            self.restore_alents(&store[30..]);
        }
        *self.store_mut(savegame) = store;
    }

    fn store_mut(&mut self, savegame: bool) -> &mut Vec<u8> {
        if savegame {
            &mut self.store_for_savegame
        } else {
            &mut self.store_for_neuron
        }
    }

    /// `savegame()` → file bytes.
    pub fn savegame_bytes(&mut self) -> Vec<u8> {
        // Deviation: a zone switch triggered in the last tick (neuron touched, or shutdown
        // finished) is completed first. The C code would save plzone with the old board and
        // object table (inconsistent file; it crashes if no neuron was loaded yet).
        if self.plzone != self.currentzone && self.flow == Flow::Running {
            self.loadzone();
            let _ = self.zone_restart();
            self.main_restart();
            // the next tick interpolates from the new place, not from the old zone
            self.sync_draw();
        }

        let mut out = Vec::with_capacity(
            SAVE_HEADER_LEN + 2 * STORE_LEN + self.brain.cells.len() + self.neuron.cells.len(),
        );
        out.push(self.mentalzone);
        out.push(self.plzone as u8); // fwrite(&plzone, 1, 1): low byte
        let mut st = self.store_player_state;
        self.save_player_state(&mut st);
        self.store_player_state = st;
        out.extend_from_slice(&st);
        let markers = self.saveal(true);
        out.extend_from_slice(&self.store_for_savegame);
        out.extend_from_slice(&self.store_for_neuron);
        let brain_at = out.len();
        out.extend_from_slice(&self.brain.cells);
        let neuron_at = out.len();
        if self.plzone != 0 {
            out.extend_from_slice(&self.neuron.cells);
        }
        // aliens that did not fit into the store sleep in the saved board (not the live one)
        let base = if self.in_neuron { neuron_at } else { brain_at };
        for (c, m) in markers {
            if let Some(b) = usize::try_from(c).ok().and_then(|c| out.get_mut(base + c)) {
                *b = m;
            }
        }
        out
    }

    /// Validates a save file against the assets; returns (brain cell count, neuron cell count).
    fn check_savegame(&self, data: &[u8]) -> Result<(usize, usize), String> {
        if data.len() < SAVE_HEADER_LEN {
            return Err(format!("saved game too short ({} bytes)", data.len()));
        }
        let mentalzone = data[0];
        // Deviation: savegame() only ever writes 1..=3 (mentalzone 4 is replaced while
        // loading); anything else would make every zonerestart reload or pick Ego silently.
        if !(1..=3).contains(&mentalzone) {
            return Err(format!("saved game has bad mental zone {mentalzone}"));
        }
        let plzone = data[1] as i32;
        let zone = &self.assets.zones[zone_of_mentalzone(mentalzone) as usize];
        let brain_len = zone.brain.cells.len();
        let neuron_len = if plzone != 0 {
            // the cell getneuronfiles(plzone) will pick
            match (1..=plzone.min(8)).rev().find_map(|i| zone.neurons[(i - 1) as usize].as_ref()) {
                Some(b) => b.cells.len(),
                None => return Err(format!("saved game refers to missing neuron {plzone}")),
            }
        } else {
            0
        };
        let need = SAVE_HEADER_LEN + 2 * STORE_LEN + brain_len + neuron_len;
        if data.len() < need {
            return Err(format!("saved game truncated ({} of {need} bytes)", data.len()));
        }
        Ok((brain_len, neuron_len))
    }

    /// `loadgame()` from file bytes (called from zone_restart when mentalzone == 4).
    /// The whole file is validated before any state changes.
    pub fn loadgame_bytes(&mut self, data: &[u8]) -> Result<(), String> {
        let (brain_len, neuron_len) = self.check_savegame(data)?;
        let mentalzone = data[0];
        // fread(&plzone, 1, 1): plzone was 0..=8 before, so only the low byte matters.
        let plzone = data[1] as i32;

        self.mentalzone = mentalzone;
        self.plzone = plzone;
        self.currentzone = plzone;
        self.store_player_state.copy_from_slice(&data[2..SAVE_HEADER_LEN]);
        let st = self.store_player_state;
        self.restore_player_state(&st);
        self.getlevelfiles();
        let mut o = SAVE_HEADER_LEN;
        self.store_for_savegame.clear();
        self.store_for_savegame.extend_from_slice(&data[o..o + STORE_LEN]);
        o += STORE_LEN;
        self.store_for_neuron.clear();
        self.store_for_neuron.extend_from_slice(&data[o..o + STORE_LEN]);
        o += STORE_LEN;
        self.restoreal(true);
        self.brain.cells.copy_from_slice(&data[o..o + brain_len]);
        o += brain_len;
        if self.plzone != 0 {
            // HACK of the C code: determines dimensions and loads the neuron
            self.getneuronfiles(self.plzone);
            self.neuron.cells.copy_from_slice(&data[o..o + neuron_len]);
        }
        // backprep(backadr) → renderer
        self.boardreg();
        self.reinitplayer();
        // the file does not record the view it was saved in: a larger view wakes its box on
        // the next tick (wake_grown_view)
        self.woken_box = (BOXWIDTH, BOXHEIGHT);
        Ok(())
    }

    /// Per-tick bookkeeping (see module docs).
    pub fn begin_tick(&mut self) {
        self.tick_no = self.tick_no.wrapping_add(1);
        std::mem::swap(&mut self.anchors, &mut self.anchors_prev);
        self.cmds.clear();
        self.sfx_events.clear();
        self.plot_key = KEY_NONE;
        self.cam_prev = self.draw.cam;
    }

    /// Camera in world pixels (`xpos>>8, ypos>>8`).
    pub fn camera(&self) -> (i32, i32) {
        (self.xpos >> 8, self.ypos >> 8)
    }

    /// Which zone music should play: 1 = brain (Music1), 2 = neuron (Music2).
    pub fn music_part(&self) -> u8 {
        if self.currentzone == 0 {
            1
        } else {
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{HELD_DOWN, HELD_FIRE, HELD_LEFT, HELD_RIGHT, HELD_UP};
    use crate::sim::testutil::sim;

    fn start(s: &mut Sim, mentalzone: u8) {
        s.start_game(GameStart { mentalzone, ..Default::default() }).expect("start");
    }

    /// Walk right/left, jump, fire, duck — a deterministic pattern over `t`.
    fn scripted(t: u32) -> TickInput {
        let held = match (t / 50) % 8 {
            0 | 1 => HELD_RIGHT,
            2 => HELD_RIGHT | HELD_UP,
            3 => HELD_RIGHT | HELD_FIRE,
            4 | 5 => HELD_LEFT,
            6 => HELD_LEFT | HELD_UP | HELD_FIRE,
            _ if t.is_multiple_of(3) => HELD_FIRE | HELD_DOWN,
            _ => 0,
        };
        TickInput { held, cheat_keys: 0 }
    }

    #[test]
    fn getlevelfiles_selects_zone_like_currentpath() {
        let mut s = sim();
        for (mz, z) in [(0u8, 0u8), (1, 0), (2, 1), (3, 2), (4, 1), (9, 0)] {
            s.mentalzone = mz;
            s.in_neuron = true;
            s.getlevelfiles();
            assert_eq!(s.zone, z, "mentalzone {mz}");
            assert!(!s.in_neuron);
            assert_eq!(s.backdrop, BackdropSel { zone: z, neuron: false });
            assert_eq!(s.brain, s.assets.zones[z as usize].brain);
        }
    }

    #[test]
    fn getneuronfiles_loads_cell_and_backdrop() {
        let mut s = sim();
        s.mentalzone = 3;
        s.getlevelfiles();
        assert_eq!(s.getneuronfiles(3), 3);
        assert!(s.in_neuron);
        assert_eq!(s.backdrop, BackdropSel { zone: 2, neuron: true });
        assert_eq!(Some(&s.neuron), s.assets.zones[2].neurons[2].as_ref());
        // Only Cell1..Cell8 exist: larger numbers fall back to Cell8.
        assert_eq!(s.getneuronfiles(200), 8);
        assert_eq!(Some(&s.neuron), s.assets.zones[2].neurons[7].as_ref());
        s.in_neuron = false;
        assert_eq!(s.getneuronfiles(0), 0);
        assert!(!s.in_neuron);
        s.retrievebackdrop();
        assert_eq!(s.backdrop, BackdropSel { zone: 2, neuron: false });
    }

    #[test]
    fn begin_tick_bookkeeping() {
        let mut s = sim();
        s.tick_no = 41;
        s.anchors[5] = Anchor { x: 1, y: 2, gen: 3, tick: 41 };
        s.cmds.push(SprCmd {
            bank: s.blocks_bank(),
            idx: 1,
            pos: Pos::Screen { x: 0, y: 0 },
            clip: Clip::None,
            kind: CmdKind::Sprite,
            key: KEY_NONE,
        });
        s.sfx_events.push(SfxEvent { channel: 0, id: crate::audio::sfx::SfxId::JUMP, volume: 1, stereo: 0 });
        s.plot_key = KEY_PROJ;
        // last tick drew at (100, 70) and teleported afterwards
        s.draw.cam = (100, 70);
        s.xpos = 3000 << 8;
        s.ypos = 900 << 8;
        s.begin_tick();
        assert_eq!(s.tick_no, 42);
        assert_eq!(s.anchors_prev[5].tick, 41);
        assert_ne!(s.anchors[5].tick, s.tick_no);
        assert!(s.cmds.is_empty() && s.sfx_events.is_empty());
        assert_eq!(s.plot_key, KEY_NONE);
        assert_eq!(s.cam_prev, (100, 70));
    }

    /// Two identical running games (same seed and input) after `ticks` ticks.
    fn twin_games(ticks: u32) -> (Sim, Sim) {
        let (mut a, mut b) = (sim(), sim());
        start(&mut a, 1);
        start(&mut b, 1);
        for t in 0..ticks {
            a.tick(scripted(t));
            b.tick(scripted(t));
        }
        assert_eq!((a.xpos, a.ypos, a.draw), (b.xpos, b.ypos, b.draw));
        (a, b)
    }

    #[test]
    fn teleport_tick_is_drawn_at_the_departure_camera() {
        // player.c playerplot(false) dotelep: xpos = telepxpos after mazeplot and after every
        // relplot/plotdying of the tick; switchbank shows that frame at the old camera.
        let (mut a, mut b) = twin_games(40);
        assert_eq!((a.telepctr, a.snuffctr), (0, 0));
        a.telepctr = 31;
        a.telepxpos = a.xpos + (300 << 8);
        a.telepypos = a.ypos - (100 << 8);
        let dest = (a.telepxpos >> 8, a.telepypos >> 8);
        a.tick(TickInput::default());
        b.tick(TickInput::default()); // twin without teleport: plmove is the same
        assert_eq!(a.telepctr, 32);
        assert_eq!(a.camera(), dest);
        assert_eq!(a.draw.cam, b.camera());
        assert_eq!(a.draw, b.draw);
        assert_ne!(a.draw.cam, a.camera());
        // the next frame is drawn at the destination; its camera step starts at the old place
        let departure = a.draw.cam;
        a.tick(TickInput::default());
        assert_eq!(a.cam_prev, departure);
        assert_eq!(a.draw.cam, a.camera());
        assert!((a.draw.cam.0 - dest.0).abs() < 16 && (a.draw.cam.1 - dest.1).abs() < 16);
    }

    #[test]
    fn respawn_tick_is_drawn_with_the_old_camera_and_hud() {
        // asylum.c: switchbank() presents the frame, then player_dead() (lives--,
        // prepstrength) and zonerestart (restartplayer moves xpos/ypos, reinitplayer sets
        // bonusreplot = 28, telepctr = 33).
        let (mut a, mut b) = twin_games(40);
        for s in [&mut a, &mut b] {
            s.lives = 2;
            s.plstrength = 0;
            s.lagerctr = 0;
            s.bonusreplot = 0;
            s.bonusctr = 3;
            s.initplx = s.xpos + (600 << 8);
            s.initply = s.ypos;
        }
        a.snuffctr = 300; // dead long enough: this tick respawns
        b.snuffctr = 200; // still dying: same movement, no respawn
        a.tick(TickInput::default());
        b.tick(TickInput::default());
        assert_eq!((a.lives, a.plstrength, a.bonusreplot, a.telepctr), (1, STRENGTHINIT, 28, 33));
        assert_eq!(a.camera(), (a.initplx >> 8, a.initply >> 8));
        assert_eq!(b.lives, 2);
        assert_eq!(a.draw.cam, b.camera());
        assert_ne!(a.draw.cam, a.camera());
        assert_eq!(a.draw, DrawState { cam: a.draw.cam, lives: 2, plstrength: 0, bonusctr: 3, bonusreplot: 0 });
        // the next frame shows the restart place and the reset HUD
        let old = a.draw.cam;
        a.tick(TickInput::default());
        assert_eq!(a.cam_prev, old);
        assert_eq!(a.draw.cam, a.camera());
        assert_eq!((a.draw.lives, a.draw.plstrength, a.draw.bonusreplot), (1, STRENGTHINIT, 27));
    }

    #[test]
    fn start_and_forced_zone_switch_sync_the_draw_state() {
        let mut s = sim();
        start(&mut s, 1);
        assert_eq!(s.draw.cam, s.camera());
        assert_eq!(s.cam_prev, s.draw.cam);
        assert_eq!((s.draw.lives, s.draw.plstrength, s.draw.bonusreplot), (s.lives, s.plstrength, s.bonusreplot));
        for t in 0..20 {
            s.tick(scripted(t));
        }
        // a neuron touched in the last tick: savegame completes the switch first
        s.change_zone(1);
        let _ = s.savegame_bytes();
        assert_eq!(s.currentzone, 1);
        let cam = s.camera();
        assert_eq!(s.draw.cam, cam);
        // the first tick after the save starts its camera step at the neuron start
        s.tick(TickInput::default());
        assert_eq!(s.cam_prev, cam);
    }

    #[test]
    fn tick_without_game_does_nothing() {
        let mut s = sim();
        assert_eq!(s.tick(TickInput::default()), TickResult::Continue);
        assert_eq!(s.tick_no, 0);
        s.flow = Flow::Over;
        assert_eq!(s.tick(TickInput::default()), TickResult::Continue);
        assert_eq!(s.tick_no, 0);
    }

    #[test]
    fn loadgame_rejects_bad_data_without_side_effects() {
        let mut s = sim();
        let before = (s.mentalzone, s.plzone, s.xpos, s.brain.clone());
        assert!(s.loadgame_bytes(&[]).is_err());
        assert!(s.loadgame_bytes(&[1, 0]).is_err());
        let brain_len = s.assets.zones[0].brain.cells.len();
        let mut d = vec![0u8; SAVE_HEADER_LEN + 2 * STORE_LEN + brain_len];
        d[0] = 0;
        assert!(s.loadgame_bytes(&d).is_err(), "mental zone 0");
        d[0] = 4;
        assert!(s.loadgame_bytes(&d).is_err(), "mental zone 4");
        d[0] = 1;
        d[1] = 2; // neuron cells missing
        assert!(s.loadgame_bytes(&d).is_err());
        d.pop();
        d[1] = 0; // one brain byte missing
        assert!(s.loadgame_bytes(&d).is_err());
        assert_eq!(before, (s.mentalzone, s.plzone, s.xpos, s.brain.clone()));
    }

    #[test]
    fn missing_save_is_an_error() {
        let mut s = sim();
        let r = s.start_game(GameStart { mentalzone: 4, ..Default::default() });
        assert!(r.is_err());
        assert_eq!(s.flow, Flow::Idle);
        let r = s.start_game(GameStart { mentalzone: 4, savegame: Some(vec![1, 0, 0]), ..Default::default() });
        assert!(r.is_err());
        assert_eq!(s.flow, Flow::Idle);
        // rejected before anything was reset
        assert_eq!((s.mentalzone, s.pending_save.is_none()), (1, true));
    }

    #[test]
    fn savegame_layout_length() {
        let mut s = sim();
        start(&mut s, 2);
        let brain_len = (s.brain.width * s.brain.height) as usize;
        let b = s.savegame_bytes();
        assert_eq!(b.len(), SAVE_HEADER_LEN + 2 * STORE_LEN + brain_len);
        assert_eq!((b[0], b[1]), (2, 0));
        // valid marker of save_player at the start of store_for_savegame
        assert_eq!(&b[SAVE_HEADER_LEN..SAVE_HEADER_LEN + 4], &SAVEVALID.to_le_bytes());
        assert_eq!(&b[b.len() - brain_len..], &s.brain.cells[..]);

        // inside a neuron the neuron cells follow
        s.change_zone(1);
        s.tick(TickInput::default());
        assert_eq!((s.currentzone, s.in_neuron), (1, true));
        let neuron_len = (s.neuron.width * s.neuron.height) as usize;
        let b = s.savegame_bytes();
        assert_eq!(b.len(), SAVE_HEADER_LEN + 2 * STORE_LEN + brain_len + neuron_len);
        assert_eq!(b[1], 1);
        assert_eq!(&b[b.len() - neuron_len..], &s.neuron.cells[..]);
    }

    #[test]
    fn smoke_zones_scripted_input() {
        for mz in 1..=3u8 {
            let mut s = sim();
            start(&mut s, mz);
            assert_eq!(s.flow, Flow::Running);
            assert_eq!(s.zone, zone_of_mentalzone(mz));
            let mut ticks = 0;
            let (mut moved, mut drawn) = (0, 0);
            for t in 0..2000u32 {
                ticks += 1;
                let x = s.xpos;
                let r = s.tick(scripted(t));
                moved += (s.xpos != x) as i32;
                drawn += !s.cmds.is_empty() as i32;
                if let TickResult::GameOver { .. } = r {
                    break;
                }
            }
            assert_eq!(s.tick_no, ticks);
            assert!(moved > 100 && drawn > 100, "zone {mz}: moved {moved}, drawn {drawn}");
            // framectr counts every frame (a zone switch tick runs a frame too)
            assert!(s.framectr > 0 && s.framectr <= ticks as i32, "zone {mz}: {}", s.framectr);
        }
    }

    #[test]
    fn zone_switch_enters_and_leaves_neuron() {
        let mut s = sim();
        start(&mut s, 1);
        for t in 0..20 {
            s.tick(scripted(t));
        }
        let brain_pos = (s.xpos, s.ypos);
        s.initplx = s.xpos; // what plsetneuronzone does
        s.initply = s.ypos;
        s.change_zone(4);
        assert_eq!(s.tick(TickInput::default()), TickResult::Continue);
        assert_eq!((s.currentzone, s.plzone, s.in_neuron), (4, 4, true));
        assert!(s.backdrop.neuron);
        assert_eq!(s.music_part(), 2);
        s.change_zone(0);
        s.tick(TickInput::default());
        assert_eq!((s.currentzone, s.in_neuron), (0, false));
        assert!(!s.backdrop.neuron);
        assert_eq!((s.xpos, s.ypos), brain_pos);
        assert_eq!(s.music_part(), 1);
    }

    /// C game(): `loadzone(); goto zonerestart` is followed at once by a full frame, so the
    /// switch tick must draw sprites and advance framectr like any other tick.
    #[test]
    fn zone_switch_tick_runs_a_frame() {
        let mut s = sim();
        start(&mut s, 1);
        for t in 0..20 {
            s.tick(scripted(t));
        }
        s.initplx = s.xpos; // what plsetneuronzone does
        s.initply = s.ypos;
        for zone in [4, 0] {
            s.change_zone(zone);
            let (tick_no, framectr) = (s.tick_no, s.framectr);
            assert_eq!(s.tick(TickInput::default()), TickResult::Continue);
            assert_eq!((s.currentzone, s.plzone), (zone, zone));
            assert_eq!(s.tick_no, tick_no + 1, "zone {zone}: one tick");
            assert_eq!(s.framectr, framectr + 1, "zone {zone}: the frame body ran");
            assert!(!s.cmds.is_empty(), "zone {zone}: sprite-less frame after a zone switch");
        }
    }

    /// Cheat keys handled on a zone switch tick must not be thrown away (C cheatread()
    /// polls the held key in the first frame of the new zone).
    #[test]
    fn zone_switch_tick_keeps_cheat_keys() {
        let mut s = sim();
        s.start_game(GameStart { mentalzone: 1, cheatpermit: 1, ..Default::default() }).expect("start");
        s.tick(TickInput::default());
        s.change_zone(2);
        // enterneuron() -> getarms() resets the weapon; F2 in the same tick gives a rocket.
        s.tick(TickInput { held: 0, cheat_keys: 2 });
        assert_eq!((s.currentzone, s.in_neuron), (2, true));
        assert!(s.plweapontype >= ROCKBASE, "F2 lost on the switch tick: {}", s.plweapontype);
    }

    #[test]
    fn zone_cheat_starts_in_neuron() {
        let mut s = sim();
        s.start_game(GameStart { mentalzone: 1, cheatpermit: 1, zone_cheat: Some(5), savegame: None })
            .expect("start");
        assert_eq!((s.plzone, s.currentzone), (5, 0));
        s.tick(TickInput::default());
        assert_eq!((s.currentzone, s.in_neuron), (5, true));
        // ignored without cheat mode or for non-digits
        for (cheat, z) in [(0, Some(5)), (1, Some(9)), (1, Some(-1))] {
            s.start_game(GameStart { mentalzone: 1, cheatpermit: cheat, zone_cheat: z, savegame: None })
                .expect("start");
            assert_eq!(s.plzone, 0);
        }
    }

    #[test]
    fn save_load_round_trip() {
        for neuron in [0, 3] {
            let mut s = sim();
            start(&mut s, 2);
            if neuron != 0 {
                s.change_zone(neuron);
            }
            for t in 0..150 {
                s.tick(scripted(t));
            }
            let saved = s.savegame_bytes();
            let (xpos, ypos, zone) = (s.xpos, s.ypos, s.currentzone);
            let (lives, score) = (s.lives, s.plscore);

            let mut l = sim();
            l.start_game(GameStart { mentalzone: 4, savegame: Some(saved.clone()), ..Default::default() })
                .expect("load");
            assert_eq!(l.mentalzone, 2);
            assert_eq!((l.zone, l.currentzone, l.plzone), (1, zone, zone));
            assert_eq!(l.in_neuron, zone != 0);
            assert_eq!((l.xpos, l.ypos), (xpos, ypos));
            assert_eq!((l.lives, l.plscore), (lives, score));
            assert_eq!(l.brain.cells, s.brain.cells);
            if zone != 0 {
                assert_eq!(l.neuron.cells, s.neuron.cells);
            }
            // the loaded game keeps running
            for t in 0..100 {
                l.tick(scripted(t));
            }
        }
    }
}
