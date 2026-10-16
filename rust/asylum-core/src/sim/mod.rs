//! The game simulation: a faithful port of the SDL Asylum game logic.
//!
//! All C globals of asylum.c / player.c / alien.c / bullet.c / projectile.c / maze.c /
//! vdu.c (text table) live in [`Sim`]. Each C file maps to one Rust file that adds an
//! `impl Sim` block:
//!
//! | C file | Rust | notes |
//! |---|---|---|
//! | maze.c | `maze.rs` | board helpers, bombs, bonuses, wake-up, teleport search, fuel-air |
//! | player.c (+ keyboard.c keyread/cheats) | `player.rs` | player movement/weapons/plotting |
//! | alien.c | `alien.rs` | objects, collision tables, explosions, save/restore alents |
//! | bullet.c + projectile.c | `weapons.rs` | enemy bullets, player projectiles |
//! | vdu.c message/texthandler | `text.rs` | message table |
//! | sound.c bidforsound* | `sound.rs` | SFX channel claiming → `SfxEvent` |
//! | asylum.c game()/setup/zones/save | `game.rs` | per-tick main loop as a state machine |
//!
//! Porting rules: docs/ARCHITECTURE.md §5 (u8 chars, wrapping i32, board indices, etc.).

pub mod alien;
pub mod consts;
pub mod game;
pub mod maze;
pub mod player;
pub mod sound;
pub mod text;
pub mod weapons;

use std::sync::Arc;

use crate::assets::{BIdx, Board, GameAssets};
use crate::atlas::Bank;
use crate::audio::sfx::SfxId;
use crate::layout::Layout;
use crate::rng::Rng;

pub use consts::*;
pub use text::{Align, TextRef, TextSlot, TextTable};

/// C `struct alent` (alien/object entry).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Alent {
    pub type_: i32,
    pub x: i32,
    pub y: i32,
    pub dx: i32,
    pub dy: i32,
    pub r5: i32,
    pub r6: i32,
    // transient flags for colcheck (all `char` = u8)
    pub colchecktype: u8,
    pub downpress: u8,
    pub falling: u8,
    pub pluphit: u8,
    pub lefthit: u8,
    pub righthit: u8,
    pub uphit: u8,
    pub downhit: u8,
}

/// C `colchent` / `bulcolchent`: collision box of an object (`al` = index into `Sim::al`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Colch {
    pub al: usize,
    pub xmin: i32,
    pub ymin: i32,
    pub xmax: i32,
    pub ymax: i32,
}

/// C `projent` (player projectile) and `bulent` (alien bullet) have the same layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shot {
    pub type_: i32,
    pub x: i32,
    pub y: i32,
    pub dx: i32,
    pub dy: i32,
    pub flags: i32,
}

/// C `key_state`: per-key press counters (0 = up, counts up while held, saturates at 0xff).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyState {
    pub uppress: u8,
    pub downpress: u8,
    pub leftpress: u8,
    pub rightpress: u8,
    pub fire: u8,
    pub keypressed: u8,
}

/// Where a sprite command is positioned (resolved to screen coordinates by the renderer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pos {
    /// C world pixel coordinates `(x>>8, y>>8)` as passed to `relplot`/`plotdying`.
    /// Renderer: `sx = gcx - 1 + x - camx`, `sy = gcy - 8 + y - camy` (camx = xpos>>8).
    World { x: i32, y: i32 },
    /// Offset from the game-window centre: `sx = gcx + x`, `sy = gcy + y`
    /// (blokeplot_cen uses `Center{x, y-8}`; decoration's `mazescaleplot(_xofs+(x>>8),
    /// _yofs+8+(y>>8))` is `Center{(x>>8)-1, y>>8}` because _xofs = gcx-1 and _yofs+8 = gcy).
    Center { x: i32, y: i32 },
    /// Content coordinates (HUD band already offset by `hx` where appropriate).
    Screen { x: i32, y: i32 },
}

/// Clip window for a sprite command (vdu.c `writeclip`, `set_player_clip`, bonus box, none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    /// Game window.
    Game,
    /// Game window, bottom edge at `gcy + 16` (set_player_clip).
    PlayerFloor,
    /// HUD bonus box: x in [bonusx-8, bonusx+8), y in [bonusy-20, bonusy+20).
    BonusBox,
    /// No clipping (releaseclip).
    None,
}

/// Special drawing modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmdKind {
    Sprite,
    /// vdu.c `cenplotdying`: four clipped copies spreading apart; `r5` = object r5.
    Dying { r5: i32 },
}

/// One recorded draw call of the tick (in the original plotting order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SprCmd {
    pub bank: Bank,
    pub idx: u8,
    pub pos: Pos,
    pub clip: Clip,
    pub kind: CmdKind,
    /// Interpolation key (0 = none). See `KEY_*`.
    pub key: u16,
}

/// Interpolation key ranges.
pub const KEY_NONE: u16 = 0;
/// Alien slot i → `KEY_ALIEN + i` (i in 0..=ALNO).
pub const KEY_ALIEN: u16 = 1;
/// Projectile slot j → `KEY_PROJ + j`.
pub const KEY_PROJ: u16 = 600;
/// Bullet slot k → `KEY_BUL + k`.
pub const KEY_BUL: u16 = 700;
pub const KEY_COUNT: usize = 800;

/// Anchor of an interpolation key for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anchor {
    pub x: i32,
    pub y: i32,
    /// Object generation (slot reuse detection).
    pub gen: u32,
    /// Tick number the anchor was recorded in (valid iff == the tick it is read for).
    pub tick: u32,
}

/// A sound request of this tick (already channel-resolved).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SfxEvent {
    /// SDL_mixer channel 0..8.
    pub channel: u8,
    pub id: SfxId,
    /// initvol 0..=127 (gain = volume/128).
    pub volume: u8,
    /// Archimedes stereo position −127..=127 (0 = centre).
    pub stereo: i8,
}

/// Held input sampled for one tick (bits = `input::HELD_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickInput {
    pub held: u32,
    /// Cheat keys currently down: bit0 F1 (get MPMG), bit1 F2 (get rocket), bit2 F3 (restore
    /// strength). Only honoured when `cheatpermit == 1`.
    pub cheat_keys: u8,
}

/// What happened in a tick (for the frontend).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickResult {
    Continue,
    /// `player_dead()` returned 2: the game is over. `completed` = gotallneurons().
    GameOver { completed: bool },
}

/// Which backdrop tile is current (`backadr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BackdropSel {
    pub zone: u8,
    pub neuron: bool,
}

/// Values the C drawing code of the current tick used, for the renderer (which builds the
/// frame after the whole tick). The camera of `mazeplot`/`relplot`/`plotdying` and the HUD
/// inputs can still change later in the same tick (teleport in `playerplot(false)`;
/// `player_dead()` → `prepstrength`/`zonerestart` after `switchbank()`); those changes first
/// show on the next frame, as in the C.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DrawState {
    /// (xpos>>8, ypos>>8) at mazeplot time.
    pub cam: (i32, i32),
    /// `lives` as shown at switchbank.
    pub lives: i32,
    /// `plstrength` as drawn by showstrength.
    pub plstrength: i32,
    /// `bonusctr` / `bonusreplot` as drawn by plotbonus.
    pub bonusctr: u8,
    pub bonusreplot: i16,
}

/// The complete game state (C globals).
pub struct Sim {
    pub assets: Arc<GameAssets>,
    pub rng: Rng,
    /// Current layout (set by the frontend before each tick).
    pub lay: Layout,

    // ---- asylum.c
    pub masterplotal: bool,
    /// Always 1 in this port (50 Hz, gearchange = 1).
    pub frameinc: i32,
    pub cheatpermit: u8,
    pub framectr: i32,
    pub currentzone: i32,
    pub plzone: i32,
    /// options.mentalzone (1 Ego, 2 Psyche, 3 Id, 4 saved game before loading).
    pub mentalzone: u8,
    /// Zone whose assets are in use (0 Ego, 1 Psyche, 2 Id) — C `currentpath`.
    pub zone: u8,
    pub xpos: i32,
    pub ypos: i32,
    pub plscore: [u8; 8],
    pub store_for_neuron: Vec<u8>,
    pub store_for_savegame: Vec<u8>,
    pub store_player_state: [u8; 25],
    /// Boards: `brain` is the zone maze, `neuron` the current neuron cell; `in_neuron`
    /// tells which one `boardadr` points to.
    pub brain: Board,
    pub neuron: Board,
    pub in_neuron: bool,
    pub backdrop: BackdropSel,

    // ---- maze.c
    pub boardwidth: i32,
    pub xposmax: i32,
    pub yposmax: i32,
    /// `fueltabofs[2][0x3c0]` of board pointers plus 4 guard entries per table (PORTING_NOTES
    /// #35); `FUEL_NULL` = C NULL (end marker).
    pub fueltab: [[BIdx; FUELTABLIM + 4]; 2],
    /// Which table `fueltabread` / `fueltabwrite` point at.
    pub fuel_read: usize,
    pub fuel_write: usize,
    pub fueltabctr: i32,
    pub fuelairlastframe: i32,

    // ---- player.c
    pub falling: u8,
    pub plframe: u8,
    pub plface: u8,
    pub plfired: u8,
    pub plweapontype: u8,
    pub plweaponspeed: u8,
    pub pluphit: u8,
    pub firerate: u8,
    pub rocketflag: u8,
    pub electrocuting: u8,
    pub platuphit: u8,
    pub blamctr: u8,
    pub rocketblamctr: u8,
    pub plotterofs: u8,
    pub hiddenplatctr: u8,
    pub extending: u8,
    pub bonusctr: u8,
    pub bonusreplot: i16,
    pub windctr: i32,
    pub firelastframe: i32,
    pub rocketctr: i32,
    pub shutdownctr: i32,
    pub atombombctr: u8,
    pub laststrength: i32,
    pub telepctr: i32,
    pub telepxpos: i32,
    pub telepypos: i32,
    pub bonusxpos: i32,
    pub bonusypos: i32,
    pub bonustimer: i32,
    pub lagerctr: i32,
    pub plstrength: i32,
    pub neuronctr: i32,
    pub snuffctr: i32,
    pub lives: i32,
    pub plscoreadd: i32,
    pub ks: KeyState,
    pub initplx: i32,
    pub initply: i32,
    pub hvec: i32,
    pub vvec: i32,
    /// pladr1..pladr8 at indices 1..=8 (index 0 unused).
    pub pladr: [BIdx; 9],
    pub pllx: i32,
    pub plly: i32,
    pub plhx: i32,
    pub plhy: i32,
    /// The player as an `alent` for the shared collision code.
    pub fraudalent: Alent,
    /// sin table, 192 entries + 1 guard (rocketblam reads index 192).
    pub rockettab: [i32; 193],

    // ---- alien.c
    pub plotal: bool,
    pub alonobj: u8,
    pub alonplat: u8,
    pub colchplace: bool,
    pub sprlx: i32,
    pub sprly: i32,
    pub sprhx: i32,
    pub sprhy: i32,
    pub xtemp: i32,
    pub ytemp: i32,
    /// `colchofs[2*_colchlim + 16]`; the current/old/use tables are (start, end) index pairs.
    pub colch: Vec<Colch>,
    pub colch_tab: usize,
    pub colch_ptr: usize,
    pub oldcolch_tab: usize,
    pub oldcolch_ptr: usize,
    pub colch_use_tab: usize,
    pub colch_use_ptr: usize,
    pub bulcolch: Vec<Colch>,
    pub bulcolch_ptr: usize,
    /// `aladr[_alno]` (+1 because wipealtab clears one entry past the end).
    pub al: Vec<Alent>,
    pub alctr: i32,
    /// `platsandstr`: index of the alien whose `dy` it points at (None = NULL).
    pub platsandstr: Option<usize>,
    pub alspintab: [i32; 40],

    // ---- bullet.c / projectile.c
    pub bul: [Shot; BULNO],
    pub bulctr: i32,
    pub splittab: [i32; 14],
    pub proj: [Shot; PROJNO],
    pub projctr: i32,
    pub projsplittab: [i32; 10],
    pub rocketbursttab: [i32; 10],

    // ---- vdu.c message table
    pub texts: TextTable,

    // ---- per-tick outputs (cleared at the start of every tick by game.rs)
    pub cmds: Vec<SprCmd>,
    pub sfx_events: Vec<SfxEvent>,
    /// SDL_mixer emulation: tick until which each channel is busy.
    pub chan_busy_until: [i64; 8],
    /// Monotonic tick counter (never reset; used for sound busy times and anchors).
    pub tick_no: u32,
    /// Key used by `relplot` & co. for the object currently being processed.
    pub plot_key: u16,
    pub anchors: Vec<Anchor>,
    pub anchors_prev: Vec<Anchor>,
    /// Generation counters for slot reuse detection.
    pub al_gen: Vec<u32>,
    pub proj_gen: [u32; PROJNO],
    pub bul_gen: [u32; BULNO],
    /// Draw camera (`draw.cam`) of the previous tick.
    pub cam_prev: (i32, i32),
    /// What this tick's C drawing code saw (camera, HUD inputs); see [`DrawState`].
    pub draw: DrawState,
    /// Game flow state (see game.rs).
    pub flow: game::Flow,
    /// Save game bytes to load at the next `zonerestart` (mentalzone == 4).
    pub pending_save: Option<Vec<u8>>,
    /// Wake-up box (blocks) the last screen wake-up covered; a larger view wakes again
    /// (`wake_grown_view`).
    pub woken_box: (i32, i32),
}

/// C NULL in the fuel tables.
pub const FUEL_NULL: BIdx = i32::MIN;

impl Sim {
    /// Fresh state; tables initialised like `c_array_initializers()`.
    pub fn new(assets: Arc<GameAssets>, lay: Layout, seed: u32) -> Sim {
        let brain = assets.zones[0].brain.clone();
        let neuron = brain.clone();
        let mut s = Sim {
            assets,
            rng: Rng::new(seed),
            lay,
            masterplotal: false,
            frameinc: 1,
            cheatpermit: 0,
            framectr: 0,
            currentzone: 0,
            plzone: 0,
            mentalzone: 1,
            zone: 0,
            xpos: 64 << 8,
            ypos: 48 << 8,
            plscore: [0; 8],
            store_for_neuron: vec![0; STORE_LEN],
            store_for_savegame: vec![0; STORE_LEN],
            store_player_state: [0; 25],
            brain,
            neuron,
            in_neuron: false,
            backdrop: BackdropSel::default(),
            boardwidth: 256,
            xposmax: 256 << 12,
            yposmax: 256 << 12,
            fueltab: [[FUEL_NULL; FUELTABLIM + 4]; 2],
            fuel_read: 0,
            fuel_write: 1,
            fueltabctr: 0,
            fuelairlastframe: 0,
            falling: 0,
            plframe: 0,
            plface: 0,
            plfired: 0,
            plweapontype: 0,
            plweaponspeed: 0,
            pluphit: 0,
            firerate: 0,
            rocketflag: 0,
            electrocuting: 0,
            platuphit: 0,
            blamctr: 0,
            rocketblamctr: 0,
            plotterofs: 0,
            hiddenplatctr: 0,
            extending: 0,
            bonusctr: 0,
            bonusreplot: 0,
            windctr: 0,
            firelastframe: 0,
            rocketctr: 0,
            shutdownctr: 0,
            atombombctr: 0,
            laststrength: 0,
            telepctr: 0,
            telepxpos: 0,
            telepypos: 0,
            bonusxpos: 0,
            bonusypos: 0,
            bonustimer: 0,
            lagerctr: 0,
            plstrength: 0,
            neuronctr: 0,
            snuffctr: 0,
            lives: 0,
            plscoreadd: 0,
            ks: KeyState::default(),
            initplx: 0,
            initply: 0,
            hvec: 0,
            vvec: 0,
            pladr: [0; 9],
            pllx: 0,
            plly: 0,
            plhx: 0,
            plhy: 0,
            fraudalent: Alent::default(),
            rockettab: [0; 193],
            plotal: false,
            alonobj: 0,
            alonplat: 0,
            colchplace: false,
            sprlx: 0,
            sprly: 0,
            sprhx: 0,
            sprhy: 0,
            xtemp: 0,
            ytemp: 0,
            colch: vec![Colch::default(); 2 * COLCHLIM + 16],
            colch_tab: 0,
            colch_ptr: 0,
            oldcolch_tab: 0,
            oldcolch_ptr: 0,
            colch_use_tab: 0,
            colch_use_ptr: 0,
            bulcolch: vec![Colch::default(); BULCOLCHLIM],
            bulcolch_ptr: 0,
            al: vec![Alent::default(); ALNO + 1],
            alctr: 0,
            platsandstr: None,
            alspintab: [0; 40],
            bul: [Shot::default(); BULNO],
            bulctr: 0,
            splittab: [0; 14],
            proj: [Shot::default(); PROJNO],
            projctr: 0,
            projsplittab: [0; 10],
            rocketbursttab: [0; 10],
            texts: TextTable::default(),
            cmds: Vec::with_capacity(1024),
            sfx_events: Vec::with_capacity(16),
            chan_busy_until: [0; 8],
            tick_no: 0,
            plot_key: KEY_NONE,
            anchors: vec![Anchor::default(); KEY_COUNT],
            anchors_prev: vec![Anchor::default(); KEY_COUNT],
            al_gen: vec![0; ALNO + 1],
            proj_gen: [0; PROJNO],
            bul_gen: [0; BULNO],
            cam_prev: (0, 0),
            draw: DrawState::default(),
            woken_box: (BOXWIDTH, BOXHEIGHT),
            flow: game::Flow::Idle,
            pending_save: None,
        };
        s.init_tables();
        s
    }

    /// `c_array_initializers()`: projsplittab, rocketbursttab, alspintab, rockettab, splittab.
    pub fn init_tables(&mut self) {
        weapons::init_projsplittab(&mut self.projsplittab);
        weapons::init_rocketbursttab(&mut self.rocketbursttab);
        weapons::init_splittab(&mut self.splittab);
        alien::init_alspintab(&mut self.alspintab);
        player::init_rockettab(&mut self.rockettab);
    }

    /// The board `boardadr` points at.
    #[inline]
    pub fn bd(&self) -> &Board {
        if self.in_neuron {
            &self.neuron
        } else {
            &self.brain
        }
    }
    #[inline]
    pub fn bd_mut(&mut self) -> &mut Board {
        if self.in_neuron {
            &mut self.neuron
        } else {
            &mut self.brain
        }
    }
    /// `*p` for a board pointer.
    #[inline]
    pub fn cell(&self, p: BIdx) -> u8 {
        self.bd().get(p)
    }
    /// `*p = v` for a board pointer.
    #[inline]
    pub fn set_cell(&mut self, p: BIdx, v: u8) {
        self.bd_mut().set(p, v)
    }

    /// C `random()`.
    #[inline]
    pub fn random(&mut self) -> i32 {
        self.rng.random()
    }

    // ------------------------------------------------------------------ plotting helpers
    // They record `SprCmd`s instead of drawing (see docs §5). The interpolation key is
    // `self.plot_key`, set by the loops in moval/project/bullets (0 for the player/HUD).

    fn push_cmd(&mut self, bank: Bank, idx: u8, pos: Pos, clip: Clip, kind: CmdKind) {
        let key = self.plot_key;
        if key != KEY_NONE {
            let k = key as usize;
            if k < KEY_COUNT && self.anchors[k].tick != self.tick_no {
                let (ax, ay) = match pos {
                    Pos::World { x, y } | Pos::Center { x, y } | Pos::Screen { x, y } => (x, y),
                };
                let gen = self.key_gen(key);
                self.anchors[k] = Anchor { x: ax, y: ay, gen, tick: self.tick_no };
            }
        }
        self.cmds.push(SprCmd { bank, idx, pos, clip, kind, key });
    }

    /// Generation counter belonging to an interpolation key.
    pub fn key_gen(&self, key: u16) -> u32 {
        let k = key as usize;
        if key >= KEY_BUL {
            self.bul_gen.get(k - KEY_BUL as usize).copied().unwrap_or(0)
        } else if key >= KEY_PROJ {
            self.proj_gen.get(k - KEY_PROJ as usize).copied().unwrap_or(0)
        } else if key >= KEY_ALIEN {
            self.al_gen.get(k - KEY_ALIEN as usize).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// Records the HUD inputs as the C drew them this tick (see [`DrawState`]).
    pub fn snapshot_hud(&mut self) {
        self.draw.lives = self.lives;
        self.draw.plstrength = self.plstrength;
        self.draw.bonusctr = self.bonusctr;
        self.draw.bonusreplot = self.bonusreplot;
    }

    /// Takes the whole draw snapshot from the current state (game start, forced zone switch,
    /// tests).
    pub fn sync_draw(&mut self) {
        self.draw.cam = self.camera();
        self.snapshot_hud();
    }

    /// player.c `relplot(sprites, n, x, y)` — world coordinates (8.8 fixed point). The C
    /// subtracts the live `xpos>>8, ypos>>8`; the renderer subtracts `draw.cam`, the camera of
    /// this tick's drawing point (nothing calls relplot after the teleport in playerplot).
    pub fn relplot(&mut self, bank: Bank, n: u8, x: i32, y: i32) {
        self.push_cmd(bank, n, Pos::World { x: x >> 8, y: y >> 8 }, Clip::Game, CmdKind::Sprite);
    }

    /// player.c `plotdying(sprites, n, x, y, r5)` (vdu.c cenplotdying); camera as `relplot`.
    pub fn plotdying(&mut self, bank: Bank, n: u8, x: i32, y: i32, r5: i32) {
        self.push_cmd(bank, n, Pos::World { x: x >> 8, y: y >> 8 }, Clip::Game, CmdKind::Dying { r5 });
    }

    /// vdu.c `blokeplot_cen(sprites, n, x, y)` with the current clip (player plotting uses
    /// `Clip::PlayerFloor`).
    pub fn blokeplot_cen(&mut self, bank: Bank, n: u8, x: i32, y: i32, clip: Clip) {
        self.push_cmd(bank, n, Pos::Center { x, y: y - 8 }, clip, CmdKind::Sprite);
    }

    /// Generic plot at a centre-relative position.
    pub fn plot_center(&mut self, bank: Bank, n: u8, x: i32, y: i32, clip: Clip) {
        self.push_cmd(bank, n, Pos::Center { x, y }, clip, CmdKind::Sprite);
    }

    /// Generic plot at content coordinates.
    pub fn plot_screen(&mut self, bank: Bank, n: u8, x: i32, y: i32, clip: Clip) {
        self.push_cmd(bank, n, Pos::Screen { x, y }, clip, CmdKind::Sprite);
    }

    /// Current zone's block bank.
    #[inline]
    pub fn blocks_bank(&self) -> Bank {
        Bank::Blocks(self.zone)
    }
    /// Current zone's alien bank.
    #[inline]
    pub fn aliens_bank(&self) -> Bank {
        Bank::Aliens(self.zone)
    }
}

/// Test helpers shared by the sim modules.
#[cfg(test)]
pub mod testutil {
    use super::*;
    /// A fresh Sim on the real assets with the original 320x256 layout.
    pub fn sim() -> Sim {
        Sim::new(crate::assets::testdata::assets(), Layout::original(), 1)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tables_match_c_values() {
        let s = super::testutil::sim();
        // spot values computed with the C expressions
        assert_eq!(s.rockettab[4], 0);
        assert_eq!(s.rockettab[50], (256.0 * (46.0f64 * 2.0 * std::f64::consts::PI / 184.0).sin()) as i32);
        assert_eq!(s.alspintab[1], 2560);
        assert_eq!(s.splittab[0], (512.0 * (0.5f64 / 3.5 * std::f64::consts::PI).sin()) as i32);
        assert_eq!(s.projsplittab[1], -0x200);
    }
}
