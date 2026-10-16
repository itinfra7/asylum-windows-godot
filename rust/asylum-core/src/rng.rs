//! Reproduction of glibc's `random()` (TYPE_3, degree 31 additive feedback generator).
//!
//! The original SDL Asylum calls `random()` without seeding, i.e. with glibc's default
//! seed 1. We implement the identical algorithm so the game "feels" exactly like the C port;
//! the frontend may reseed (e.g. from the clock) for variety.

#[derive(Clone, Debug)]
pub struct Rng {
    /// Ring buffer holding the last 31 generated words.
    ring: [i32; 31],
    pos: usize,
}

impl Rng {
    /// Same as `srandom(seed)` in glibc.
    pub fn new(seed: u32) -> Rng {
        let seed = if seed == 0 { 1 } else { seed };
        let mut r = [0i32; 34];
        r[0] = seed as i32;
        for i in 1..31 {
            // r[i] = (16807 * r[i-1]) % 2147483647 computed without overflow (Schrage),
            // exactly as glibc's srandom_r does it.
            let hi = (r[i - 1] as i64) / 127773;
            let lo = (r[i - 1] as i64) % 127773;
            let mut word = 16807 * lo - 2836 * hi;
            if word < 0 {
                word += 2147483647;
            }
            r[i] = word as i32;
        }
        for i in 31..34 {
            r[i] = r[i - 31];
        }
        let mut rng = Rng { ring: [0; 31], pos: 0 };
        // Fill the ring with o[34..=64] and then discard 310 outputs (glibc discards 310).
        let mut hist: Vec<i32> = r.to_vec();
        for i in 34..(344 + 31) {
            let v = hist[i - 31].wrapping_add(hist[i - 3]);
            hist.push(v);
        }
        // The next outputs are hist[344..]; keep the last 31 values before index 344+31
        // so that subsequent values can be generated incrementally.
        rng.ring.copy_from_slice(&hist[344..344 + 31]);
        rng.pos = 0;
        rng
    }

    /// `random()`: returns a value in `0..=0x7fff_ffff`.
    #[inline]
    pub fn random(&mut self) -> i32 {
        // ring[pos] holds o[n] where n = 344 + produced; we emit it and replace it with o[n+31].
        let out = self.ring[self.pos];
        // r[n+31] = r[n] + r[n+28]
        let idx28 = (self.pos + 28) % 31;
        let next = self.ring[self.pos].wrapping_add(self.ring[idx28]);
        self.ring[self.pos] = next;
        self.pos = (self.pos + 1) % 31;
        ((out as u32) >> 1) as i32
    }
}

impl Default for Rng {
    fn default() -> Self {
        Rng::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_glibc_default_sequence() {
        // First outputs of glibc random() with the default seed (srandom(1)).
        let mut r = Rng::new(1);
        let expect = [1804289383, 846930886, 1681692777, 1714636915, 1957747793, 424238335, 719885386, 1649760492, 596516649, 1189641421];
        for e in expect {
            assert_eq!(r.random(), e);
        }
    }

    #[test]
    fn long_run_stays_in_range() {
        let mut r = Rng::new(12345);
        for _ in 0..100_000 {
            let v = r.random();
            assert!(v >= 0);
        }
    }
}
