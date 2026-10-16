//! "Bodge 1 " tracker tunes (sound.c `swi_bodgemusic_load`, `initialize_music`).
//!
//! The C code reads the file into `char tuneload[a][30000]` (zero padded). Section list at
//! byte 8 (one byte per section, terminated by a byte with bit 7 set); pattern pointer for
//! section value s is the u32 LE at byte (0x42 + s) * 4. Pattern = pairs (first, second):
//! first != 0: set inst = first & 0x1f, pitch = second & 0x3f; first == 0: trigger channel
//! second & 3 with the pending inst/pitch (then reset inst=0,pitch=8). A beat ends when
//! (second & 0xc0) != 0; (second & 0x80) also ends the pattern (advance section).

pub const TUNE_BUF: usize = 30000;

#[derive(Clone, Debug)]
pub struct Tune {
    /// Whole file zero padded to `TUNE_BUF` bytes.
    pub raw: Vec<u8>,
}

impl Tune {
    #[inline]
    pub fn byte(&self, i: usize) -> u8 {
        *self.raw.get(i).unwrap_or(&0)
    }
    /// u32 LE at byte offset (wrapping safely to 0 outside the buffer).
    pub fn word(&self, byte_off: usize) -> u32 {
        u32::from_le_bytes([self.byte(byte_off), self.byte(byte_off + 1), self.byte(byte_off + 2), self.byte(byte_off + 3)])
    }
}

pub fn parse_tune(data: &[u8]) -> Result<Tune, String> {
    if data.len() < 8 || &data[0..5] != b"Bodge" {
        return Err("not a Bodge tune".into());
    }
    let mut raw = data.to_vec();
    if raw.len() < TUNE_BUF {
        raw.resize(TUNE_BUF, 0);
    }
    Ok(Tune { raw })
}
