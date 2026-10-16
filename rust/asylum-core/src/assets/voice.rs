//! "Sta1" voice samples (sound.c `load_voice`, `make_sound`, `sdl_music_hook`).
//!
//! The C code loads the whole file into `char voice[v][30000]` (zero padded) and reads:
//! word 6 (byte 24) = len, word 7 (byte 28) = gap, word 8 (byte 32) = rep, samples from byte 44.
//! Samples are 8-bit VIDC logarithmic ("mu-law"), decoded with `MULAW`.

/// (voice slot, file name) exactly as `load_voices()` in sound.c.
pub const VOICE_FILES: [(usize, &str); 21] = [
    (1, "Jump"),
    (2, "Bonus"),
    (3, "Explo"),
    (4, "AtomExplo"),
    (5, "Cannon"),
    (6, "Rocket"),
    (7, "Hiss"),
    (8, "Stunned"),
    (9, "SmallZap"),
    (10, "BigZap"),
    (16, "Organ"),
    (17, "Plink"),
    (18, "PlinkHard"),
    (19, "Raver"),
    (20, "Dome"),
    (21, "NasalStr"),
    (22, "BassHollow"),
    (23, "NasalBass"),
    (24, "BassDrum"),
    (25, "Snare"),
    (26, "Cymbal"),
];

/// Size of the C `voice[]` row; files are zero padded to this length.
pub const VOICE_BUF: usize = 30000;

#[derive(Clone, Debug)]
pub struct Voice {
    pub len: u32,
    pub gap: u32,
    pub rep: u32,
    /// Whole file, zero padded to `VOICE_BUF` bytes (index like the C array).
    pub raw: Vec<u8>,
}

impl Voice {
    /// Byte of the C array (`voice[v][i]`), 0 outside the buffer.
    #[inline]
    pub fn byte(&self, i: i64) -> u8 {
        if i < 0 { 0 } else { *self.raw.get(i as usize).unwrap_or(&0) }
    }
}

/// 32 voice slots (index = C voice number); missing ones are `None`.
#[derive(Clone, Debug, Default)]
pub struct Voices {
    pub slots: Vec<Option<Voice>>,
}

impl Voices {
    pub fn get(&self, v: usize) -> Option<&Voice> {
        self.slots.get(v).and_then(|s| s.as_ref())
    }
}

/// `init_mulaw()` table.
pub fn mulaw_table() -> [i16; 256] {
    let exp: [i32; 8] = [0, 132, 396, 924, 1980, 4092, 8316, 16764];
    let mut t = [0i16; 256];
    for i in 0..256usize {
        let sign = if i & 1 != 0 { -1 } else { 1 };
        let v = exp[i >> 5] + (((i as i32) & 0x1e) << (2 + (i >> 5)));
        t[i] = (sign * v) as i16;
    }
    t
}

pub fn parse_voice(data: &[u8]) -> Result<Voice, String> {
    if data.len() < 44 || &data[0..4] != b"Sta1" {
        return Err("not a Sta1 voice".into());
    }
    let w = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let mut raw = data.to_vec();
    if raw.len() < VOICE_BUF {
        raw.resize(VOICE_BUF, 0);
    }
    Ok(Voice { len: w(24), gap: w(28), rep: w(32), raw })
}
