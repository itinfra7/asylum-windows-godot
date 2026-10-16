//! Windowed-sinc interpolation kernel and per-voice sample tables shared by SFX and music.
//!
//! The C code evaluates `w * sin(pi*(j-m)) / (1.3*pi*(j-m))` for taps j in [m-W, m+W]
//! (W = 32 for effects, 4 for music). We use a Lanczos windowed sinc (a = `TAPS`) normalised
//! to unity DC gain with `TAPS` taps per side and a phase table, then multiply by `1/1.3` to
//! keep the original loudness. Music and most effects play their source below 48 kHz; the
//! few pitch-sliding effects that exceed it alias, as they do in the C code (which applies no
//! low-pass scaling either).

use std::sync::OnceLock;

use crate::assets::voice::mulaw_table;
use crate::assets::Voice;

/// Taps on each side of the interpolation point.
pub const TAPS: usize = 8;
/// Phase resolution of the table.
pub const PHASES: usize = 1024;
const PHASE_BITS: u32 = PHASES.trailing_zeros();
/// Loudness factor of the original (`/1.3`).
pub const GAIN: f32 = 1.0 / 1.3;

const WIDTH: usize = 2 * TAPS;

pub struct Kernel {
    /// `PHASES` rows of `2*TAPS` coefficients; row p is for fractional offset p/PHASES,
    /// coefficient k multiplies sample `floor(pos) - TAPS + 1 + k`.
    pub table: Vec<f32>,
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

fn lanczos(d: f64) -> f64 {
    if d.abs() >= TAPS as f64 {
        0.0
    } else {
        sinc(d) * sinc(d / TAPS as f64)
    }
}

impl Kernel {
    pub fn new() -> Kernel {
        let mut table = Vec::with_capacity(PHASES * WIDTH);
        let mut row = [0f64; WIDTH];
        for p in 0..PHASES {
            let frac = p as f64 / PHASES as f64;
            for (k, c) in row.iter_mut().enumerate() {
                // distance from the interpolation point to sample floor(pos) - TAPS + 1 + k
                *c = lanczos(frac + (TAPS - 1) as f64 - k as f64);
            }
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|c| (c / sum) as f32));
        }
        Kernel { table }
    }

    /// Process-wide kernel (built once).
    pub fn shared() -> &'static Kernel {
        static K: OnceLock<Kernel> = OnceLock::new();
        K.get_or_init(Kernel::new)
    }

    /// Coefficients for fractional position `frac` in [0,1).
    #[inline]
    pub fn row(&self, frac: f64) -> &[f32] {
        let p = ((frac * PHASES as f64) as usize).min(PHASES - 1);
        &self.table[p * 2 * TAPS..(p + 1) * 2 * TAPS]
    }

    #[inline]
    fn row_at(&self, p: usize) -> &[f32; WIDTH] {
        self.table[p * WIDTH..(p + 1) * WIDTH].try_into().expect("kernel row")
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Kernel::new()
    }
}

/// Upper bound for a voice's `len` field (real voices are <= 16384 samples); protects
/// against absurd allocations for a malformed header.
const MAX_LEN: i64 = 1 << 20;

/// The C per-tap sample lookup `value(j)` of one voice (sound.c `make_sound` /
/// `sdl_music_hook`): loop region, gap crossfade, unsigned bytes, in C units (`mulaw/4`).
/// Differences from the C code (memory-safety / HQ): `len == 0` and the end of a
/// non-looping sample give silence; `gap == 0` never divides.
pub struct VoiceTable {
    len: i64,
    rep: i64,
    /// value(j) for j in [-TAPS, len + 2*TAPS), stored at index j + TAPS.
    ext: Vec<f32>,
}

impl VoiceTable {
    pub fn new(voice: &Voice) -> VoiceTable {
        let mulaw = mulaw_table();
        let m = |i: i64| mulaw[voice.byte(i) as usize];
        let len = (voice.len as i64).min(MAX_LEN);
        let gap = voice.gap as i64;
        let rep = voice.rep as i64;
        let mut ext = vec![0f32; len as usize + 3 * TAPS];
        for j in 0..len {
            let w = if gap > 0 && j >= len - gap {
                ((j - (len - gap)) as f64 * m(j + 44 - rep) as f64 + (len - j) as f64 * m(j + 44) as f64)
                    / gap as f64
                    / 4.0
            } else {
                // C: Sint16 / int → integer division
                (m(j + 44) as i32 / 4) as f64
            };
            ext[j as usize + TAPS] = w as f32;
        }
        let mut t = VoiceTable { len, rep, ext };
        for j in len..len + 2 * TAPS as i64 {
            // maps into [0, len), which is already filled
            t.ext[(j + TAPS as i64) as usize] = t.value(j);
        }
        t
    }

    /// Sample length (`len` header field).
    pub fn len(&self) -> i64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Sample value at integer source index `j` (0 for j < 0).
    #[inline]
    pub fn value(&self, j: i64) -> f32 {
        if j < 0 {
            return 0.0;
        }
        let j = if j >= self.len {
            if self.rep == 0 {
                return 0.0;
            }
            (self.len - self.rep) + (j - self.len) % self.rep
        } else {
            j
        };
        if j < 0 {
            // rep > len (malformed header)
            return 0.0;
        }
        self.ext[j as usize + TAPS]
    }

    /// Band-limited value at fractional source position `pos` (nearest kernel phase).
    #[inline]
    pub fn interp(&self, kernel: &Kernel, pos: f64) -> f32 {
        let x = pos * PHASES as f64 + 0.5;
        let mut q = x as i64;
        if q as f64 > x {
            q -= 1; // floor for negative positions
        }
        let p = (q & (PHASES as i64 - 1)) as usize;
        let mut base = (q >> PHASE_BITS) - (TAPS as i64 - 1);
        if base > self.len {
            if self.rep == 0 {
                return 0.0;
            }
            if self.rep > self.len {
                return self.interp_slow(kernel, base, p);
            }
            // The sequence is periodic (period rep) from len-rep on: fold the window back.
            let p0 = self.len - self.rep;
            base = p0 + (base - p0) % self.rep;
        } else if base < -(TAPS as i64) {
            return self.interp_slow(kernel, base, p);
        }
        let s: &[f32; WIDTH] = self.ext[(base + TAPS as i64) as usize..][..WIDTH].try_into().expect("window");
        dot(kernel.row_at(p), s)
    }

    #[cold]
    fn interp_slow(&self, kernel: &Kernel, base: i64, p: usize) -> f32 {
        let c = kernel.row_at(p);
        (0..WIDTH).map(|k| c[k] * self.value(base + k as i64)).sum()
    }
}

#[inline]
fn dot(c: &[f32; WIDTH], s: &[f32; WIDTH]) -> f32 {
    let mut acc = [0f32; 4];
    for k in 0..WIDTH {
        acc[k & 3] += c[k] * s[k];
    }
    (acc[0] + acc[1]) + (acc[2] + acc[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::voice::VOICE_BUF;

    fn voice(len: u32, gap: u32, rep: u32, samples: &[u8]) -> Voice {
        let mut raw = vec![0u8; VOICE_BUF];
        raw[44..44 + samples.len()].copy_from_slice(samples);
        Voice { len, gap, rep, raw }
    }

    #[test]
    fn rows_sum_to_one() {
        let k = Kernel::new();
        assert_eq!(k.table.len(), PHASES * 2 * TAPS);
        for p in 0..PHASES {
            let s: f64 = k.table[p * WIDTH..(p + 1) * WIDTH].iter().map(|&c| c as f64).sum();
            assert!((s - 1.0).abs() < 1e-5, "row {p} sums to {s}");
        }
        // phase 0 is the identity at tap TAPS-1
        let r0 = k.row(0.0);
        for (i, &c) in r0.iter().enumerate() {
            let want = if i == TAPS - 1 { 1.0 } else { 0.0 };
            assert!((c - want).abs() < 1e-6, "tap {i}: {c}");
        }
        // half phase is symmetric
        let rh = k.row(0.5);
        for i in 0..WIDTH {
            assert!((rh[i] - rh[WIDTH - 1 - i]).abs() < 1e-6);
        }
    }

    #[test]
    fn value_semantics() {
        let mulaw = mulaw_table();
        // byte 0x42 → +(396+32)/4 = 107, byte 0x43 → -107 (unsigned bytes, odd = negative)
        let v = voice(4, 0, 0, &[0x42, 0x43, 0xff, 0x80]);
        let t = VoiceTable::new(&v);
        assert_eq!(mulaw[0x42], 428);
        assert_eq!(t.value(-1), 0.0);
        assert_eq!(t.value(0), 107.0);
        assert_eq!(t.value(1), -107.0);
        assert_eq!(t.value(2), (mulaw[0xff] / 4) as f32);
        assert_eq!(t.value(3), (mulaw[0x80] / 4) as f32);
        assert_eq!(t.value(4), 0.0, "silence after a non-looping sample");
        assert_eq!(t.value(1000), 0.0);

        // looping: len 4, rep 2 → 0 1 2 3 2 3 2 3 ...
        let v = voice(4, 0, 2, &[2, 4, 6, 8]);
        let t = VoiceTable::new(&v);
        let m = |b: usize| (mulaw[b] as i32 / 4) as f32;
        for j in 0..20 {
            let src = if j < 4 { j } else { 2 + (j - 4) % 2 };
            assert_eq!(t.value(j as i64), m([2, 4, 6, 8][src]), "j={j}");
        }

        // gap crossfade: len 4, gap 2, rep 2: j=2 → (0*m[raw[44]] + 2*m[raw[46]])/2/4
        let v = voice(4, 2, 2, &[2, 4, 6, 8]);
        let t = VoiceTable::new(&v);
        let f = |a: usize, b: usize, j: f64| ((j - 2.0) * mulaw[a] as f64 + (4.0 - j) * mulaw[b] as f64) / 2.0 / 4.0;
        assert_eq!(t.value(2), f(2, 6, 2.0) as f32);
        assert_eq!(t.value(3), f(4, 8, 3.0) as f32);

        // len == 0 → silence, also when "looping"
        let t = VoiceTable::new(&voice(0, 0, 2, &[2, 4]));
        assert_eq!(t.value(0), 0.0);
        assert_eq!(t.value(5), 0.0);
        assert_eq!(t.interp(Kernel::shared(), 3.3), 0.0);
    }

    #[test]
    fn interp_matches_direct_sum_and_folds_loops() {
        let k = Kernel::shared();
        let assets = crate::assets::testdata::assets();
        for slot in [7usize, 16, 19, 21, 24] {
            let v = assets.voices.get(slot).expect("voice");
            let t = VoiceTable::new(v);
            for &pos in &[0.0, 0.25, 3.75, 100.5, 4000.125, 9000.5, 123456.7, 1.0e6 + 0.3] {
                let q = (pos * PHASES as f64 + 0.5).floor() as i64;
                let (fl, p) = (q >> PHASE_BITS, (q & (PHASES as i64 - 1)) as usize);
                let row = k.row_at(p);
                let direct: f32 = (0..WIDTH).map(|i| row[i] * t.value(fl - TAPS as i64 + 1 + i as i64)).sum();
                let got = t.interp(k, pos);
                assert!((got - direct).abs() <= 1e-3 * (1.0 + direct.abs()), "voice {slot} pos {pos}: {got} vs {direct}");
            }
        }
    }
}
