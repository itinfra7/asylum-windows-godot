//! Soak / robustness tests for the game simulation (docs/ARCHITECTURE.md §11).
//!
//! Random input across every zone and every neuron for many ticks: the port must never
//! panic (index out of bounds etc.), must stay deterministic for a fixed seed and must
//! round-trip save games.

use std::sync::{Arc, OnceLock};

use asylum_core::assets::{GameAssets, RawFiles};
use asylum_core::input::{HELD_DOWN, HELD_FIRE, HELD_LEFT, HELD_RIGHT, HELD_UP};
use asylum_core::layout::Layout;
use asylum_core::sim::game::{Flow, GameStart};
use asylum_core::sim::{Sim, TickInput, TickResult};

fn assets() -> Arc<GameAssets> {
    static A: OnceLock<Arc<GameAssets>> = OnceLock::new();
    A.get_or_init(|| {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/data");
        Arc::new(GameAssets::load(&RawFiles::from_dir(&dir).unwrap()).unwrap())
    })
    .clone()
}

/// Small deterministic PRNG for input scripting (independent of the game RNG).
struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 16) as u32
    }
}

/// Input "player": holds a random key combination for a random number of ticks.
struct Bot {
    rng: XorShift,
    held: u32,
    left: u32,
    cheats: bool,
}
impl Bot {
    fn new(seed: u64, cheats: bool) -> Bot {
        Bot { rng: XorShift(seed | 1), held: 0, left: 0, cheats }
    }
    fn input(&mut self) -> TickInput {
        if self.left == 0 {
            let r = self.rng.next();
            let mut h = 0;
            if r & 3 == 1 {
                h |= HELD_LEFT;
            }
            if r & 3 == 2 {
                h |= HELD_RIGHT;
            }
            if r & 4 != 0 {
                h |= HELD_FIRE;
            }
            if r & 0x18 == 0x08 {
                h |= HELD_UP;
            }
            if r & 0x60 == 0x20 {
                h |= HELD_DOWN;
            }
            self.held = h;
            self.left = 1 + (self.rng.next() % 60);
        }
        self.left -= 1;
        let mut cheat_keys = 0;
        if self.cheats {
            let r = self.rng.next();
            if r.is_multiple_of(400) {
                cheat_keys |= 1; // F1 mini-gun
            }
            if r % 400 == 1 {
                cheat_keys |= 2; // F2 rockets
            }
            if r.is_multiple_of(97) {
                cheat_keys |= 4; // F3 restore strength (keeps the run alive deeper)
            }
        }
        TickInput { held: self.held, cheat_keys }
    }
}

/// Runs up to `ticks` ticks, restarting after game over. Returns a state checksum.
fn run(mentalzone: u8, zone_cheat: Option<i32>, ticks: u32, seed: u32, wide: bool, cheats: bool) -> u64 {
    // "wide": a large view (3.0x on a 2560x1080 monitor); see soak_extreme_views for more
    let lay = if wide { Layout::new(1820.0, 768.0) } else { Layout::original() };
    run_with(lay, mentalzone, zone_cheat, ticks, seed, cheats)
}

fn run_with(lay: Layout, mentalzone: u8, zone_cheat: Option<i32>, ticks: u32, seed: u32, cheats: bool) -> u64 {
    let mut sim = Sim::new(assets(), lay, seed);
    let start = GameStart { mentalzone, cheatpermit: if cheats { 1 } else { 0 }, zone_cheat, savegame: None };
    sim.start_game(start.clone()).expect("start");
    let mut bot = Bot::new(seed as u64 * 7919 + mentalzone as u64, cheats);
    let mut sum: u64 = 1469598103934665603;
    let mut mix = |v: i64| {
        sum ^= v as u64;
        sum = sum.wrapping_mul(1099511628211);
    };
    for _ in 0..ticks {
        let r = sim.tick(bot.input());
        // The renderer reads these every tick; touch them to make sure they stay sane.
        assert!(sim.cmds.len() < 20_000, "runaway draw list");
        for c in &sim.cmds {
            let _ = c;
        }
        mix(sim.xpos as i64);
        mix(sim.ypos as i64);
        mix(sim.plstrength as i64);
        mix(sim.cmds.len() as i64);
        if let TickResult::GameOver { .. } = r {
            assert_eq!(sim.flow, Flow::Over);
            sim.start_game(start.clone()).expect("restart");
        }
    }
    for s in sim.plscore {
        mix(s as i64);
    }
    sum
}

#[test]
fn soak_all_zones_original_layout() {
    for z in 1..=3u8 {
        run(z, None, 30_000, 1 + z as u32, false, true);
    }
}

#[test]
fn soak_all_zones_wide_layout() {
    for z in 1..=3u8 {
        run(z, None, 30_000, 100 + z as u32, true, true);
    }
}

#[test]
fn soak_extreme_views() {
    // 3.0x in a 32:9 borderless window, and a tall maximized 4:3 window
    let (w, h) = asylum_core::display::view_size(asylum_core::display::SCALE_MAX, (5120, 1440));
    assert_eq!((w, h), (2731, 768));
    for z in 1..=3u8 {
        run_with(Layout::new(w as f32, h as f32), z, None, 20_000, 300 + z as u32, true);
        run_with(Layout::new(512.0, 1700.0), z, None, 20_000, 400 + z as u32, true);
    }
}

#[test]
fn soak_every_neuron() {
    // zonecheatread: start directly inside neuron N of every zone.
    for z in 1..=3u8 {
        for n in 1..=8 {
            run(z, Some(n), 4_000, 1000 + z as u32 * 10 + n as u32, n % 2 == 0, true);
        }
    }
}

#[test]
fn soak_without_cheats_until_game_over() {
    for z in 1..=3u8 {
        run(z, None, 40_000, 77 + z as u32, z == 2, false);
    }
}

#[test]
fn deterministic_for_fixed_seed() {
    let a = run(1, None, 5_000, 4242, false, true);
    let b = run(1, None, 5_000, 4242, false, true);
    assert_eq!(a, b);
    let c = run(1, None, 5_000, 4243, false, true);
    assert_ne!(a, c, "different seeds should diverge");
}

#[test]
fn savegame_round_trip() {
    for z in 1..=3u8 {
        let mut sim = Sim::new(assets(), Layout::original(), 9);
        sim.start_game(GameStart { mentalzone: z, cheatpermit: 1, zone_cheat: None, savegame: None }).unwrap();
        let mut bot = Bot::new(99 + z as u64, true);
        for _ in 0..1500 {
            if let TickResult::GameOver { .. } = sim.tick(bot.input()) {
                break;
            }
        }
        if sim.flow != Flow::Running {
            continue;
        }
        let bytes = sim.savegame_bytes();
        assert!(bytes.len() >= 2 + 25 + 2 * asylum_core::sim::STORE_LEN + 256 * 256);
        assert_eq!(bytes[0], z);

        let mut other = Sim::new(assets(), Layout::original(), 10);
        other
            .start_game(GameStart { mentalzone: 4, cheatpermit: 0, zone_cheat: None, savegame: Some(bytes.clone()) })
            .expect("load saved game");
        assert_eq!(other.mentalzone, z, "mentalzone restored from the save");
        assert_eq!(other.plscore, sim.plscore);
        assert_eq!(other.lives, sim.lives);
        assert_eq!(other.neuronctr, sim.neuronctr);
        assert_eq!(other.brain.cells, sim.brain.cells);
        // and it keeps running
        let mut bot2 = Bot::new(5, true);
        for _ in 0..500 {
            other.tick(bot2.input());
        }
    }
}

#[test]
fn corrupt_savegame_is_rejected() {
    let mut sim = Sim::new(assets(), Layout::original(), 3);
    let r = sim.start_game(GameStart { mentalzone: 4, cheatpermit: 0, zone_cheat: None, savegame: Some(vec![1, 2, 3]) });
    assert!(r.is_err());
}
