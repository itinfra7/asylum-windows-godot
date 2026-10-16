//! Decoding of the ORIGINAL Asylum data files (never modified, see game/data).
//!
//! * `hammer`  – "Hmr" LZ decompression (file.c `swi_blitz_hammerop`)
//! * `fsp`     – "FSP1" sprite banks (vdu.c `initialize_sprites`)
//! * `screen`  – RLE full screens GameScreen/ChatScreen (vdu.c `decomp`)
//! * `board`   – maze boards Brain / Neurons/CellN (asylum.c, maze.c)
//! * `voice`   – "Sta1" mu-law instrument/effect samples (sound.c `load_voice`)
//! * `tune`    – "Bodge 1 " tracker tunes (sound.c)
//!
//! `GameAssets::load` decodes everything, using worker threads (std::thread::scope) so the
//! three zones, the common banks and the voices decode in parallel.

pub mod board;
pub mod fsp;
pub mod hammer;
pub mod screen;
pub mod tune;
pub mod voice;

use std::collections::HashMap;

pub use board::{BIdx, Board};
pub use fsp::{Sprite, SpriteBank};
pub use screen::Screen;
pub use tune::Tune;
pub use voice::{Voice, Voices};

/// Raw file contents keyed by their path relative to `game/data/`, using forward slashes,
/// e.g. `"Resources/GameScreen"`, `"Ego/Neurons/Cell3"`, `"Voices/Jump"`.
#[derive(Default, Clone)]
pub struct RawFiles {
    pub files: HashMap<String, Vec<u8>>,
}

impl RawFiles {
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|v| v.as_slice())
    }

    /// Reads every file below `data_dir` (used by tests and tools; the Godot host reads the
    /// files from the PCK itself and fills `files`).
    pub fn from_dir(data_dir: &std::path::Path) -> std::io::Result<RawFiles> {
        fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut HashMap<String, Vec<u8>>) -> std::io::Result<()> {
            for e in std::fs::read_dir(dir)? {
                let e = e?;
                let p = e.path();
                if p.is_dir() {
                    walk(base, &p, out)?;
                } else {
                    let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                    out.insert(rel, std::fs::read(&p)?);
                }
            }
            Ok(())
        }
        let mut files = HashMap::new();
        walk(data_dir, data_dir, &mut files)?;
        Ok(RawFiles { files })
    }

    /// The list of files the game needs (so the host knows what to read).
    pub fn required_paths() -> Vec<String> {
        let mut v: Vec<String> = [
            "Resources/GameScreen",
            "Resources/ChatScreen",
            "Resources/FSPBloke",
            "Resources/FSPChars",
            "Resources/FSPExplo",
            "Resources/Music1",
            "Resources/Music2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for z in ZONE_DIRS {
            for f in ["Brain", "Backfile", "FSPBlocks", "FSPAliens", "Music1", "Music2", "Neurons/Backfile"] {
                v.push(format!("{z}/{f}"));
            }
            for c in 1..=8 {
                v.push(format!("{z}/Neurons/Cell{c}"));
            }
        }
        for (_, name) in voice::VOICE_FILES {
            v.push(format!("Voices/{name}"));
        }
        v
    }
}

/// Directory names of the three mental zones, index = zone id (0 Ego, 1 Psyche, 2 Id).
pub const ZONE_DIRS: [&str; 3] = ["Ego", "Psyche", "Id"];

/// A 48x32 backdrop tile ("Backfile"), palette indices, row-major.
#[derive(Clone)]
pub struct Backdrop {
    pub pix: Vec<u8>,
}

/// Everything belonging to one mental zone.
pub struct ZoneAssets {
    /// FSPBlocks (256 slots).
    pub blocks: SpriteBank,
    /// FSPAliens (256 slots).
    pub aliens: SpriteBank,
    /// Main maze ("Brain").
    pub brain: Board,
    /// Backdrop tile of the main maze.
    pub backdrop: Backdrop,
    /// Backdrop tile used inside neurons ("Neurons/Backfile").
    pub neuron_backdrop: Backdrop,
    /// Neurons/Cell1..Cell8 (index 0 = Cell1). `None` if the file does not exist.
    pub neurons: [Option<Board>; 8],
    /// In-game music (brain) and neuron music.
    pub music1: Tune,
    pub music2: Tune,
}

/// All decoded game data. Immutable after loading; shared via `Arc`.
pub struct GameAssets {
    pub chars: SpriteBank,
    pub bloke: SpriteBank,
    pub explo: SpriteBank,
    pub game_screen: Screen,
    pub chat_screen: Screen,
    pub zones: [ZoneAssets; 3],
    pub voices: Voices,
    /// Resources/Music1 (title) and Resources/Music2 (game over, played slower).
    pub title_music: Tune,
    pub death_music: Tune,
}

impl GameAssets {
    /// Decode all files. Uses `std::thread::scope` to decode zones/banks/voices in parallel.
    /// Returns a human readable error naming the offending file on failure.
    pub fn load(files: &RawFiles) -> Result<GameAssets, String> {
        fn get<'a>(files: &'a RawFiles, path: &str) -> Result<&'a [u8], String> {
            files.get(path).ok_or_else(|| format!("missing data file: {path}"))
        }
        fn bank(files: &RawFiles, path: &str, max: usize) -> Result<SpriteBank, String> {
            let raw = hammer::unhammer(get(files, path)?).map_err(|e| format!("{path}: {e}"))?;
            fsp::parse_fsp(&raw, max).map_err(|e| format!("{path}: {e}"))
        }
        fn board(files: &RawFiles, path: &str) -> Result<Board, String> {
            let raw = hammer::unhammer(get(files, path)?).map_err(|e| format!("{path}: {e}"))?;
            board::parse_board(&raw).map_err(|e| format!("{path}: {e}"))
        }
        fn backdrop(files: &RawFiles, path: &str) -> Result<Backdrop, String> {
            let mut pix = hammer::unhammer(get(files, path)?).map_err(|e| format!("{path}: {e}"))?;
            pix.resize(48 * 32, 0);
            Ok(Backdrop { pix })
        }
        fn tune(files: &RawFiles, path: &str) -> Result<Tune, String> {
            tune::parse_tune(get(files, path)?).map_err(|e| format!("{path}: {e}"))
        }
        fn zone(files: &RawFiles, z: usize) -> Result<ZoneAssets, String> {
            let d = ZONE_DIRS[z];
            let mut neurons: [Option<Board>; 8] = Default::default();
            for (c, slot) in neurons.iter_mut().enumerate() {
                let path = format!("{d}/Neurons/Cell{}", c + 1);
                if files.get(&path).is_some() {
                    *slot = Some(board(files, &path)?);
                }
            }
            Ok(ZoneAssets {
                blocks: bank(files, &format!("{d}/FSPBlocks"), 256)?,
                aliens: bank(files, &format!("{d}/FSPAliens"), 256)?,
                brain: board(files, &format!("{d}/Brain"))?,
                backdrop: backdrop(files, &format!("{d}/Backfile"))?,
                neuron_backdrop: backdrop(files, &format!("{d}/Neurons/Backfile"))?,
                neurons,
                music1: tune(files, &format!("{d}/Music1"))?,
                music2: tune(files, &format!("{d}/Music2"))?,
            })
        }
        fn voices(files: &RawFiles) -> Result<Voices, String> {
            let mut slots: Vec<Option<Voice>> = vec![None; 32];
            for (v, name) in voice::VOICE_FILES {
                let path = format!("Voices/{name}");
                slots[v] = Some(voice::parse_voice(get(files, &path)?).map_err(|e| format!("{path}: {e}"))?);
            }
            Ok(Voices { slots })
        }
        fn screen(files: &RawFiles, p: &str) -> Result<Screen, String> {
            let raw = hammer::unhammer(get(files, p)?).map_err(|e| format!("{p}: {e}"))?;
            screen::parse_screen(&raw).map_err(|e| format!("{p}: {e}"))
        }
        type Common = (SpriteBank, SpriteBank, SpriteBank, Screen, Screen, Tune, Tune);
        fn common(files: &RawFiles) -> Result<Common, String> {
            Ok((
                bank(files, "Resources/FSPChars", 48)?,
                bank(files, "Resources/FSPBloke", 77)?,
                bank(files, "Resources/FSPExplo", 32)?,
                screen(files, "Resources/GameScreen")?,
                screen(files, "Resources/ChatScreen")?,
                tune(files, "Resources/Music1")?,
                tune(files, "Resources/Music2")?,
            ))
        }

        // Decode the three zones, the common banks/screens and the voices in parallel.
        std::thread::scope(|sc| {
            let zh: Vec<_> = (0..3).map(|z| sc.spawn(move || zone(files, z))).collect();
            let vh = sc.spawn(|| voices(files));
            let ch = sc.spawn(|| common(files));
            let mut zones = Vec::with_capacity(3);
            for h in zh {
                zones.push(h.join().map_err(|_| "zone decoder panicked".to_string())??);
            }
            let voices = vh.join().map_err(|_| "voice decoder panicked".to_string())??;
            let (chars, bloke, explo, game_screen, chat_screen, title_music, death_music) =
                ch.join().map_err(|_| "decoder panicked".to_string())??;
            let zones: [ZoneAssets; 3] = match zones.try_into() {
                Ok(z) => z,
                Err(_) => return Err("zone count".to_string()),
            };
            Ok(GameAssets { chars, bloke, explo, game_screen, chat_screen, zones, voices, title_music, death_music })
        })
    }

}

/// Helpers for tests in every module (loads the real game/data).
#[cfg(test)]
pub mod testdata {
    use super::*;

    /// Path of the original data directory (game/data) relative to this crate.
    pub fn data_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game/data")
    }

    pub fn raw_files() -> RawFiles {
        RawFiles::from_dir(&data_dir()).expect("game/data readable")
    }

    /// Loads the real assets (decoded once per test process).
    pub fn assets() -> std::sync::Arc<GameAssets> {
        static CELL: std::sync::OnceLock<std::sync::Arc<GameAssets>> = std::sync::OnceLock::new();
        CELL.get_or_init(|| std::sync::Arc::new(GameAssets::load(&raw_files()).expect("assets decode"))).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_required_files_exist() {
        let files = testdata::raw_files();
        for p in RawFiles::required_paths() {
            assert!(files.get(&p).is_some(), "missing {p}");
        }
    }

    #[test]
    fn decodes_everything_with_expected_shapes() {
        let a = testdata::assets();
        assert_eq!(a.chars.sprites.len(), 48);
        assert_eq!(a.chars.get(17).map(|s| (s.w, s.h, s.xc, s.yc)), Some((12, 14, 6, 7)));
        assert_eq!(a.chars.get(0).map(|s| (s.w, s.h)), Some((14, 14)));
        assert_eq!(a.bloke.sprites.len(), 77);
        assert!(a.bloke.get(76).is_none());
        assert_eq!(a.explo.get(0).map(|s| (s.w, s.h)), Some((32, 32)));
        for z in 0..3 {
            let zn = &a.zones[z];
            assert_eq!((zn.brain.width, zn.brain.height), (256, 256));
            assert_eq!(zn.brain.cells.len(), 65536);
            assert_eq!(zn.backdrop.pix.len(), 48 * 32);
            assert_eq!(zn.blocks.get(4).map(|s| (s.w, s.h)), Some((16, 16)));
            assert!(zn.neurons.iter().all(|n| n.is_some()));
            assert!(zn.aliens.get(0).is_some());
        }
        let c2 = a.zones[0].neurons[1].as_ref().unwrap();
        assert_eq!((c2.width, c2.height), (96, 32));
        // GameScreen: top border row 4 at x=160 is not black, window interior is black
        assert_ne!(a.game_screen.pix[4 * 320 + 160], 0);
        assert_eq!(a.game_screen.pix[50 * 320 + 40], 0);
        for (v, _) in voice::VOICE_FILES {
            let vo = a.voices.get(v).unwrap();
            assert!(vo.len > 0 && (vo.len as usize) < voice::VOICE_BUF);
        }
        assert_eq!(&a.title_music.raw[0..5], b"Bodge");
    }

    #[test]
    fn start_marker_present_in_every_brain() {
        let a = testdata::assets();
        for z in 0..3 {
            assert!(a.zones[z].brain.cells.contains(&255), "zone {z} has a start marker");
        }
    }
}
