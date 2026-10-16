//! Long soak (manual): many seeds, every zone and neuron, several view sizes, random input
//! with and without cheats, savegame round trips mid-run. Run with
//! `CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true cargo test --release --test long_soak -- --ignored`.
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
    }).clone()
}
struct X(u64);
impl X { fn n(&mut self) -> u32 { let mut x = self.0; x ^= x << 13; x ^= x >> 7; x ^= x << 17; self.0 = x; (x >> 16) as u32 } }

#[test]
#[ignore]
fn long_soak() {
    let views = [(320.0, 256.0), (455.0, 256.0), (607.0, 256.0), (683.0, 512.0), (1820.0, 768.0), (2731.0, 768.0), (512.0, 1700.0)];
    let mut total = 0u64;
    for seed in 1..=12u64 {
        for z in 1..=3u8 {
            let (vw, vh) = views[(seed as usize + z as usize) % views.len()];
            let lay = Layout::new(vw, vh);
            let cheats = seed % 3 != 0;
            let zone_cheat = if seed % 4 == 0 { Some(((seed / 4) % 8 + 1) as i32) } else { None };
            let mut sim = Sim::new(assets(), lay, seed as u32 * 7 + z as u32);
            let start = GameStart { mentalzone: z, cheatpermit: if cheats { 1 } else { 0 }, zone_cheat, savegame: None };
            sim.start_game(start.clone()).unwrap();
            let mut r = X(seed * 1000 + z as u64);
            let (mut held, mut left) = (0u32, 0u32);
            for t in 0..120_000u32 {
                if left == 0 {
                    let v = r.n();
                    held = 0;
                    if v & 3 == 1 { held |= HELD_LEFT; }
                    if v & 3 == 2 { held |= HELD_RIGHT; }
                    if v & 4 != 0 { held |= HELD_FIRE; }
                    if v & 0x18 == 0x08 { held |= HELD_UP; }
                    if v & 0x60 == 0x20 { held |= HELD_DOWN; }
                    left = 1 + r.n() % 90;
                }
                left -= 1;
                let mut ck = 0u8;
                if cheats {
                    let v = r.n();
                    if v.is_multiple_of(500) {
                        ck |= 1;
                    } else if v % 500 == 1 {
                        ck |= 2;
                    }
                    if v.is_multiple_of(61) {
                        ck |= 4;
                    }
                }
                let res = sim.tick(TickInput { held, cheat_keys: ck });
                total += 1;
                assert!(sim.cmds.len() < 50_000, "runaway draw list");
                if t % 25_000 == 24_999 && sim.flow == Flow::Running {
                    // save and reload in place
                    let bytes = sim.savegame_bytes();
                    let mut other = Sim::new(assets(), lay, 99);
                    other.start_game(GameStart { mentalzone: 4, cheatpermit: start.cheatpermit, zone_cheat: None, savegame: Some(bytes) }).expect("reload");
                    sim = other;
                }
                if let TickResult::GameOver { .. } = res {
                    sim.start_game(start.clone()).unwrap();
                }
            }
        }
    }
    println!("ticks {total}");
}
