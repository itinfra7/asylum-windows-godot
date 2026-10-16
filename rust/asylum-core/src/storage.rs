//! Files in the portable user-data directory (file.c replacements).
//!
//! * `asylum.cfg`            – [`crate::config::Config`]
//! * `asylum_game.sav`       – save game, original binary layout (asylum.c savegame/loadgame)
//! * `hiscores/<Zone>HighScores` – 5 entries × 13 bytes ("00000000 PSY\n") + 1 CRC byte (0xff),
//!   zones: Ego, Psyche, Id, Extended (mentalzone 1..4)
//!
//! All IO happens on the game thread. Writes go to a temp file then rename (atomic).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::Config;

pub const HIGHSCORE_LEN: usize = 13 * 5 + 1;
pub const DEFAULT_SCORE_LINE: &[u8; 13] = b"00000000 PSY\n";
/// What the SDL port's `swi_oscrc` stub always returns.
const HIGHSCORE_CRC: u8 = 0xff;

#[derive(Clone, Debug)]
pub struct Storage {
    pub dir: PathBuf,
    /// The settings file exists but could not be read: keep a copy before the first write
    /// replaces it (never destroy settings or progress silently).
    config_unreadable: Arc<AtomicBool>,
}

/// Settings text in any encoding Windows editors save: UTF-16 with a byte order mark, or
/// UTF-8 (with or without one); invalid bytes become U+FFFD instead of losing the file.
fn decode_text(b: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    };
    match b {
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

impl Storage {
    /// Creates the directory (and `hiscores/`) if needed.
    pub fn new(dir: PathBuf) -> Storage {
        let _ = std::fs::create_dir_all(dir.join("hiscores"));
        Storage { dir, config_unreadable: Arc::new(AtomicBool::new(false)) }
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("asylum.cfg")
    }
    pub fn savegame_path(&self) -> PathBuf {
        self.dir.join("asylum_game.sav")
    }
    pub fn highscore_path(&self, mental_zone: u8) -> PathBuf {
        let name = match mental_zone {
            2 => "PsycheHighScores",
            3 => "IdHighScores",
            4 => "ExtendedHighScores",
            _ => "EgoHighScores",
        };
        self.dir.join("hiscores").join(name)
    }

    pub fn load_config(&self) -> Config {
        match std::fs::read(self.config_path()) {
            Ok(b) => Config::parse(&decode_text(&b)),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    self.config_unreadable.store(true, Ordering::Relaxed);
                }
                Config::default()
            }
        }
    }

    pub fn save_config(&self, cfg: &Config) -> std::io::Result<()> {
        let path = self.config_path();
        if self.config_unreadable.swap(false, Ordering::Relaxed) {
            // the file was there but unreadable at start-up: keep it before replacing it
            let _ = std::fs::copy(&path, path.with_extension("cfg.bak"));
        }
        write_atomic(&path, cfg.serialize().as_bytes())
    }

    /// menus.c loadscores: returns the 66-byte table, or defaults when missing/corrupt
    /// (CRC byte must be 0xff like swi_oscrc in the SDL port).
    pub fn load_highscores(&self, mental_zone: u8) -> [u8; HIGHSCORE_LEN] {
        use std::io::Read;
        if (1..=4).contains(&mental_zone) {
            let mut t = [0u8; HIGHSCORE_LEN];
            // fread(..., 13*5+1) == 13*5+1: longer files are fine, only the first 66 bytes count.
            let read =
                std::fs::File::open(self.highscore_path(mental_zone)).and_then(|mut f| f.read_exact(&mut t));
            if read.is_ok() && t[13 * 5] == HIGHSCORE_CRC {
                return t;
            }
        }
        default_highscores()
    }

    /// file.c savescores: stores the CRC byte (always 0xff) then writes the table; zones
    /// outside 1..=4 have no file and are ignored.
    pub fn save_highscores(&self, mental_zone: u8, table: &[u8; HIGHSCORE_LEN]) -> std::io::Result<()> {
        if !(1..=4).contains(&mental_zone) {
            return Ok(());
        }
        let mut t = *table;
        t[13 * 5] = HIGHSCORE_CRC;
        write_atomic(&self.highscore_path(mental_zone), &t)
    }

    pub fn savegame_exists(&self) -> bool {
        self.savegame_path().is_file()
    }

    pub fn read_savegame(&self) -> Option<Vec<u8>> {
        std::fs::read(self.savegame_path()).ok()
    }

    pub fn write_savegame(&self, bytes: &[u8]) -> std::io::Result<()> {
        write_atomic(&self.savegame_path(), bytes)
    }
}

/// The default high score table.
pub fn default_highscores() -> [u8; HIGHSCORE_LEN] {
    let mut t = [0u8; HIGHSCORE_LEN];
    for i in 0..5 {
        t[i * 13..i * 13 + 13].copy_from_slice(DEFAULT_SCORE_LINE);
    }
    t[13 * 5] = 0xff;
    t
}

/// Write via temp file + rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("asylum-storage-{tag}-{}-{nanos}-{n}", std::process::id()));
            assert!(!dir.exists());
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample_table() -> [u8; HIGHSCORE_LEN] {
        let mut t = default_highscores();
        t[..13].copy_from_slice(b"00123450 ABC\n");
        t[13..26].copy_from_slice(b"00004200 Z^0\n");
        t
    }

    #[test]
    fn missing_files_give_defaults() {
        let tmp = TempDir::new("missing");
        let st = Storage::new(tmp.0.clone());
        assert!(tmp.0.join("hiscores").is_dir());
        for zone in 0..=5 {
            assert_eq!(st.load_highscores(zone), default_highscores());
        }
        let d = default_highscores();
        assert_eq!(&d[..13], DEFAULT_SCORE_LINE);
        assert_eq!(&d[52..65], DEFAULT_SCORE_LINE);
        assert_eq!(d[65], 0xff);
        assert!(!st.savegame_exists());
        assert_eq!(st.load_config(), Config::default());
    }

    #[test]
    fn settings_in_other_encodings_survive() {
        let tmp = TempDir::new("encodings");
        let st = Storage::new(tmp.0.clone());
        // a comment saved in CP949 (not UTF-8) must not throw the whole file away
        std::fs::write(st.config_path(), b"# \xC7\xD1\xB1\xB9\xBE\xEE\nIdPermit=1\nMentalZone=3\nKeyFire=65\n").unwrap();
        let c = st.load_config();
        assert!(c.id_permit && c.mental_zone == 3 && c.keys[4] == 65);
        // Notepad's UTF-16 (little and big endian) with a byte order mark
        for be in [false, true] {
            let text = "IdPermit=1\r\nMentalZone=3\r\n# \u{d55c}\u{ad6d}\u{c5b4}\r\n";
            let mut b = if be { vec![0xFE, 0xFF] } else { vec![0xFF, 0xFE] };
            for u in text.encode_utf16() {
                b.extend_from_slice(&if be { u.to_be_bytes() } else { u.to_le_bytes() });
            }
            std::fs::write(st.config_path(), b).unwrap();
            let c = st.load_config();
            assert!(c.id_permit && c.mental_zone == 3, "utf16 be={be}");
        }
    }

    #[test]
    fn an_unreadable_settings_file_is_kept_before_it_is_replaced() {
        let tmp = TempDir::new("unreadable");
        let st = Storage::new(tmp.0.clone());
        // a directory where the file should be: reading fails with something other than
        // "not found"
        std::fs::create_dir_all(st.config_path()).unwrap();
        assert_eq!(st.load_config(), Config::default());
        assert!(st.config_unreadable.load(Ordering::Relaxed));
        std::fs::remove_dir_all(st.config_path()).unwrap();
        std::fs::write(st.config_path(), b"IdPermit=1\n").unwrap();
        st.save_config(&Config::default()).unwrap();
        assert_eq!(std::fs::read(st.config_path().with_extension("cfg.bak")).unwrap(), b"IdPermit=1\n");
        assert!(!st.config_unreadable.load(Ordering::Relaxed), "only the first write keeps a copy");
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new("roundtrip");
        let st = Storage::new(tmp.0.clone());
        let t = sample_table();
        for zone in 1..=4 {
            st.save_highscores(zone, &t).unwrap();
            assert_eq!(st.load_highscores(zone), t);
        }
        assert!(tmp.0.join("hiscores/EgoHighScores").is_file());
        assert!(tmp.0.join("hiscores/PsycheHighScores").is_file());
        assert!(tmp.0.join("hiscores/IdHighScores").is_file());
        assert!(tmp.0.join("hiscores/ExtendedHighScores").is_file());
        assert_eq!(std::fs::read(st.highscore_path(1)).unwrap().len(), HIGHSCORE_LEN);
        // Zones are independent.
        let mut t2 = t;
        t2[0] = b'9';
        st.save_highscores(2, &t2).unwrap();
        assert_eq!(st.load_highscores(1), t);
        assert_eq!(st.load_highscores(2), t2);
    }

    #[test]
    fn save_sets_the_crc_byte() {
        let tmp = TempDir::new("crc");
        let st = Storage::new(tmp.0.clone());
        let mut t = sample_table();
        t[65] = 0x12;
        st.save_highscores(3, &t).unwrap();
        let bytes = std::fs::read(st.highscore_path(3)).unwrap();
        assert_eq!(bytes[65], 0xff);
        assert_eq!(&bytes[..65], &t[..65]);
    }

    #[test]
    fn corrupt_or_short_files_give_defaults() {
        let tmp = TempDir::new("corrupt");
        let st = Storage::new(tmp.0.clone());
        let mut bad_crc = sample_table();
        bad_crc[65] = 0;
        std::fs::write(st.highscore_path(1), bad_crc).unwrap();
        assert_eq!(st.load_highscores(1), default_highscores());
        std::fs::write(st.highscore_path(2), &sample_table()[..65]).unwrap();
        assert_eq!(st.load_highscores(2), default_highscores());
        std::fs::write(st.highscore_path(3), b"").unwrap();
        assert_eq!(st.load_highscores(3), default_highscores());
        // Like fread, trailing bytes after the 66-byte table are ignored.
        let mut long = sample_table().to_vec();
        long.extend_from_slice(b"trailing junk");
        std::fs::write(st.highscore_path(4), &long).unwrap();
        assert_eq!(st.load_highscores(4), sample_table());
    }

    #[test]
    fn invalid_zones_are_not_written() {
        let tmp = TempDir::new("zones");
        let st = Storage::new(tmp.0.clone());
        st.save_highscores(0, &sample_table()).unwrap();
        st.save_highscores(5, &sample_table()).unwrap();
        assert_eq!(std::fs::read_dir(tmp.0.join("hiscores")).unwrap().count(), 0);
    }

    #[test]
    fn config_and_savegame_files() {
        let tmp = TempDir::new("cfg");
        let st = Storage::new(tmp.0.clone());
        let cfg = Config { language: Some(crate::i18n::Lang::Ko), sfx_volume: 3, ..Config::default() };
        st.save_config(&cfg).unwrap();
        assert_eq!(st.load_config(), cfg);
        st.write_savegame(&[1, 2, 3]).unwrap();
        assert!(st.savegame_exists());
        assert_eq!(st.read_savegame().unwrap(), vec![1, 2, 3]);
        assert!(!tmp.0.join("asylum.tmp").exists() && !tmp.0.join("asylum_game.tmp").exists());
    }
}
