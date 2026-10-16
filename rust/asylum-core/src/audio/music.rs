//! Streaming Bodge-tracker player (port of sound.c `initialize_music`, `swi_sound_qtempo`,
//! `sdl_music_hook`) rendering stereo f32 at 48 kHz. Runs on the music thread.
//!
//! Semantics: tempo `t` (0x1000 normal, 0x980 = slower game-over music) gives
//! `(2756*0x1000)/t` C-samples per beat (integer division), i.e. beat length in seconds
//! = that / 22050. A new beat reads event pairs until `(second & 0xc0) != 0`; voice v plays
//! instrument `inst[v]` at `4000*2^(pitch/12)` Hz from `start_time[v]`; voices 0/2 → left,
//! 1/3 → right (`s[v&1]`). Loop/gap handling per voice like the C code; after the end of a
//! non-looping sample the voice is silent. At the end of the section list the song restarts
//! from section 0 (seamless loop, voices keep ringing).
//!
//! Timing at 48 kHz: beat k starts at output sample `floor(k * beat_len * 48000 / 22050)`;
//! a voice's source position is `(n - start) * rate / 48000` (the C `off*rate/22050` in
//! seconds), interpolated with the shared kernel.

use std::sync::Arc;

use super::kernel::{Kernel, VoiceTable, GAIN};
use super::{C_RATE, RATE};
use crate::assets::tune::TUNE_BUF;
use crate::assets::{Tune, Voices};

/// Which piece is playing (the frontend decides, see frontend docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicTrack {
    /// Resources/Music1 — title / menus / zone completed.
    Title,
    /// Resources/Music2 at tempo 0x980 — game over.
    GameOver,
    /// Zone music: zone 0..3, part 1 = brain (Music1), 2 = neuron (Music2).
    Zone { zone: u8, part: u8 },
}

/// `swi_sound_qtempo(0x1000)`, the default of `initialize_music`.
pub const DEFAULT_TEMPO: i32 = 0x1000;

/// Event pairs read per beat before the pattern is treated as ended (a corrupt pattern
/// would otherwise loop forever / run off the C buffer).
const MAX_PAIRS_PER_BEAT: usize = TUNE_BUF / 2;

/// Output scale: C Sint16 units → ±1.
const OUT_SCALE: f32 = GAIN / 32768.0;

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    /// `music.inst[v]`; 0 = off.
    inst: usize,
    /// `music.pitch[v]`.
    pitch: i32,
    /// `music.start_time[v]` in output samples.
    start: u64,
    /// Source samples per output sample (`4000*2^(pitch/12) / 48000`).
    step: f64,
}

pub struct MusicSynth {
    tune: Tune,
    kernel: &'static Kernel,
    /// Per instrument slot (0..32) sample tables.
    tables: Vec<Option<VoiceTable>>,
    /// `music.tempo`: beat length in C samples.
    beat_len: u64,
    /// Output sample index (`music.time` at 48 kHz).
    n: u64,
    /// Index of the next beat and the output sample where it starts.
    beat: u64,
    next_beat_n: u64,
    /// `music.section` (byte offset into the tune) and `music.pointer`.
    section: usize,
    pointer: usize,
    ch: [Channel; 4],
}

impl MusicSynth {
    /// `tempo_t`: argument of swi_sound_qtempo (0x1000 default).
    pub fn new(voices: Arc<Voices>, tune: Tune, tempo_t: i32) -> MusicSynth {
        let tables = (0..32).map(|v| voices.get(v).map(VoiceTable::new)).collect();
        // swi_sound_qtempo; t <= 0 (a C division by zero / negative tempo) falls back to the default
        let t = if tempo_t > 0 { tempo_t } else { DEFAULT_TEMPO };
        let beat_len = ((2756 * 0x1000) / t).max(1) as u64;
        // initialize_music
        let section = 8;
        let pointer = tune.word((0x42 + tune.byte(section) as usize) * 4) as usize;
        MusicSynth {
            tune,
            kernel: Kernel::shared(),
            tables,
            beat_len,
            n: 0,
            beat: 0,
            next_beat_n: 0,
            section,
            pointer,
            ch: [Channel::default(); 4],
        }
    }

    /// Render the next `out.len()` stereo frames (overwrites `out`). Values roughly in ±1.
    pub fn fill(&mut self, out: &mut [[f32; 2]]) {
        let mut i = 0;
        while i < out.len() {
            if self.n == self.next_beat_n {
                self.new_beat();
            }
            let span = ((self.next_beat_n - self.n) as usize).min(out.len() - i);
            let buf = &mut out[i..i + span];
            buf.fill([0.0; 2]);
            for (v, ch) in self.ch.iter().enumerate() {
                if ch.inst == 0 {
                    continue;
                }
                let Some(table) = self.tables[ch.inst].as_ref() else { continue };
                let side = v & 1;
                let off0 = self.n - ch.start;
                for (k, o) in buf.iter_mut().enumerate() {
                    let off = (off0 + k as u64) as f64;
                    o[side] += table.interp(self.kernel, off * ch.step);
                }
            }
            for o in buf.iter_mut() {
                o[0] = (o[0] * OUT_SCALE).clamp(-1.0, 1.0);
                o[1] = (o[1] * OUT_SCALE).clamp(-1.0, 1.0);
            }
            self.n += span as u64;
            i += span;
        }
    }

    /// The "new beat" block of sdl_music_hook, at output sample `self.n`.
    fn new_beat(&mut self) {
        let mut inst = 0usize;
        let mut pitch = 8i32;
        let mut second;
        let mut pairs = 0;
        loop {
            let first = self.tune.byte(self.pointer);
            second = self.tune.byte(self.pointer + 1);
            self.pointer += 2;
            pairs += 1;
            if first != 0 {
                // pitch word
                inst = (first & 0x1f) as usize;
                pitch = (second & 0x3f) as i32;
            } else {
                let c = &mut self.ch[(second & 3) as usize];
                c.pitch = pitch;
                c.inst = inst;
                c.start = self.n;
                c.step = 4000.0 * (pitch as f64 / 12.0).exp2() / RATE as f64;
                inst = 0;
                pitch = 8;
            }
            if second & 0xc0 != 0 {
                break;
            }
            if pairs >= MAX_PAIRS_PER_BEAT || self.pointer >= TUNE_BUF {
                second = 0x80; // corrupt pattern: end it
                break;
            }
        }
        if second & 0x80 != 0 {
            self.section += 1;
            if self.tune.byte(self.section) & 0x80 != 0 {
                self.section = 8;
            }
            self.pointer = self.tune.word((0x42 + self.tune.byte(self.section) as usize) * 4) as usize;
        }
        self.beat += 1;
        self.next_beat_n = (self.beat as u128 * self.beat_len as u128 * RATE as u128 / C_RATE as u128) as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::voice::mulaw_table;
    use std::time::Instant;

    fn render(m: &mut MusicSynth, frames: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0f32; 2]; frames];
        m.fill(&mut out);
        out
    }

    fn rms(x: impl Iterator<Item = f32>) -> f64 {
        let (mut s, mut n) = (0f64, 0usize);
        for v in x {
            s += v as f64 * v as f64;
            n += 1;
        }
        (s / n.max(1) as f64).sqrt()
    }

    fn title() -> MusicSynth {
        let a = crate::assets::testdata::assets();
        MusicSynth::new(Arc::new(a.voices.clone()), a.title_music.clone(), DEFAULT_TEMPO)
    }

    /// Independent walk of the pattern data: (beat, channel, inst) of every trigger in the
    /// first `beats` beats.
    fn triggers(tune: &Tune, beats: usize) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        let mut sec = 8;
        let mut ptr = tune.word((0x42 + tune.byte(sec) as usize) * 4) as usize;
        for beat in 0..beats {
            let (mut inst, mut s) = (0, 0);
            while s & 0xc0 == 0 {
                let f = tune.byte(ptr);
                s = tune.byte(ptr + 1);
                ptr += 2;
                if f != 0 {
                    inst = (f & 0x1f) as usize;
                } else {
                    out.push((beat, (s & 3) as usize, inst));
                    inst = 0;
                }
            }
            if s & 0x80 != 0 {
                sec += 1;
                if tune.byte(sec) & 0x80 != 0 {
                    sec = 8;
                }
                ptr = tune.word((0x42 + tune.byte(sec) as usize) * 4) as usize;
            }
        }
        out
    }

    #[test]
    fn tempo_and_beat_grid() {
        let mut m = title();
        assert_eq!(m.beat_len, 2756);
        render(&mut m, 1);
        // beat 1 at floor(2756 * 48000 / 22050) = 5999
        assert_eq!(m.next_beat_n, 5999);
        render(&mut m, 5998);
        assert_eq!(m.beat, 1);
        render(&mut m, 1);
        // floor(2 * 2756 * 48000 / 22050) = 11998 (not 2 * 5999: no rounding drift)
        assert_eq!((m.beat, m.next_beat_n), (2, 11998));
        let a = crate::assets::testdata::assets();
        let mut g = MusicSynth::new(Arc::new(a.voices.clone()), a.death_music.clone(), 0x980);
        assert_eq!(g.beat_len, (2756 * 0x1000) / 0x980);
        render(&mut g, 1);
        assert_eq!(g.next_beat_n, 4641 * 48000 / 22050);
        // no drift: beat k starts at floor(k * len * 48000 / 22050)
        render(&mut g, 4641 * 48000 * 300 / 22050);
        assert_eq!(g.beat, 301);
        assert_eq!(g.next_beat_n, 4641 * 48000 * 301 / 22050);
    }

    #[test]
    fn song_loops_to_start() {
        let mut m = title();
        let start = (m.section, m.pointer);
        let mut beats = 0;
        loop {
            m.new_beat();
            beats += 1;
            if (m.section, m.pointer) == start {
                break;
            }
            assert!(beats < 100_000, "song never loops");
        }
        assert_eq!(beats, 2176, "title tune length in beats");
        // voices keep ringing across the loop
        assert!(m.ch.iter().any(|c| c.inst != 0));
    }

    #[test]
    fn title_tune_non_silent_stereo_and_onsets() {
        let mut m = title();
        let tune = m.tune.clone();
        let out = render(&mut m, 10 * 48_000);
        let (l, r) = (rms(out.iter().map(|o| o[0])), rms(out.iter().map(|o| o[1])));
        println!("title tune 10 s: rms L {l:.4}  R {r:.4}");
        assert!(l > 0.01 && r > 0.01, "both channels must carry music (L {l}, R {r})");
        assert!(out.iter().all(|o| o[0].abs() <= 1.0 && o[1].abs() <= 1.0));

        let beat_start = |k: usize| k * 2756 * 48000 / 22050;
        let tr = triggers(&tune, 80);
        for side in 0..2 {
            let &(k, _, _) = tr.iter().find(|&&(_, c, inst)| c & 1 == side && inst != 0).expect("a note on each side");
            let first_nz = out.iter().position(|o| o[side] != 0.0).expect("note sounds");
            println!("side {side}: first note at beat {k} (sample {}), first non-zero sample {first_nz}", beat_start(k));
            assert!(first_nz >= beat_start(k), "side {side} sounds before its first note");
            assert!(first_nz < beat_start(k) + 480, "side {side} onset later than 10 ms");
        }
    }

    /// One section, one pattern: beats 0..3 only silence channel 0, beat 3 starts Plink
    /// (17) on channel 1 (right), beat 4 ends the pattern.
    fn onset_tune() -> Tune {
        let mut raw = vec![0u8; TUNE_BUF];
        raw[..8].copy_from_slice(b"Bodge 1 ");
        raw[8] = 0; // section 0
        raw[9] = 0x80; // end of list
        raw[0x42 * 4..0x42 * 4 + 4].copy_from_slice(&400u32.to_le_bytes());
        let pat = [0, 0x40, 0, 0x40, 0, 0x40, 17, 12, 0, 0x41, 0, 0x80];
        raw[400..400 + pat.len()].copy_from_slice(&pat);
        Tune { raw }
    }

    #[test]
    fn first_note_onset_timing() {
        let a = crate::assets::testdata::assets();
        for tempo in [DEFAULT_TEMPO, 0x980] {
            let mut m = MusicSynth::new(Arc::new(a.voices.clone()), onset_tune(), tempo);
            let out = render(&mut m, 48_000);
            let beat_len = ((2756 * 0x1000) / tempo) as usize;
            let onset = 3 * beat_len * 48000 / 22050;
            assert!(out.iter().all(|o| o[0] == 0.0), "left channel stays silent");
            let first = out.iter().position(|o| o[1] != 0.0).expect("note sounds");
            println!("tempo {tempo:#x}: expected onset {onset}, first non-zero {first}");
            assert!(first >= onset && first < onset + 48, "tempo {tempo:#x}: onset {first}, want {onset}");
            // the note ends (Plink loops 2 samples at its end): still bounded and finite
            assert!(out.iter().all(|o| o[1].is_finite() && o[1].abs() <= 1.0));
        }
    }

    #[test]
    fn chunked_rendering_60s_is_finite_and_seamless() {
        let mut a = title();
        let mut b = title();
        let total = 60 * 48_000;
        let t0 = Instant::now();
        let whole = render(&mut a, total);
        let dt = t0.elapsed();
        println!("60 s of title music rendered in {dt:?} ({:.0}x real time)", 60.0 / dt.as_secs_f64());
        let mut chunked = Vec::with_capacity(total);
        let mut buf = vec![[0f32; 2]; 1500];
        let sizes = [1usize, 777, 1024, 1500, 333];
        let mut i = 0;
        while chunked.len() < total {
            let n = sizes[i % sizes.len()].min(total - chunked.len());
            b.fill(&mut buf[..n]);
            chunked.extend_from_slice(&buf[..n]);
            i += 1;
        }
        assert!(whole.iter().all(|o| o[0].is_finite() && o[1].is_finite()));
        assert!(whole == chunked, "chunk boundaries change the output");
        assert!(rms(whole[total - 48_000..].iter().map(|o| o[0] + o[1])) > 0.005, "still playing after 59 s");
    }

    /// Direct port of sdl_music_hook at 22050 Hz (window 4), with silence after a
    /// non-looping sample like ours, for a loudness comparison.
    fn c_music(voices: &Voices, tune: &Tune, samples: usize) -> Vec<[i16; 2]> {
        let mulaw = mulaw_table();
        let tempo = 2756i32;
        let (mut pitchv, mut instv, mut start) = ([0i32; 4], [0usize; 4], [0i32; 4]);
        let mut section = 8usize;
        let mut pointer = tune.word((0x42 + tune.byte(section) as usize) * 4) as usize;
        let mut out = Vec::with_capacity(samples);
        for time in 0..samples as i32 {
            if time % tempo == 0 {
                let (mut inst, mut pitch, mut second) = (0usize, 8i32, 0u8);
                while second & 0xc0 == 0 {
                    let first = tune.byte(pointer);
                    second = tune.byte(pointer + 1);
                    pointer += 2;
                    if first != 0 {
                        inst = (first & 0x1f) as usize;
                        pitch = (second & 0x3f) as i32;
                    } else {
                        let c = (second & 3) as usize;
                        pitchv[c] = pitch;
                        instv[c] = inst;
                        start[c] = time;
                        inst = 0;
                        pitch = 8;
                    }
                }
                if second & 0x80 != 0 {
                    section += 1;
                    if tune.byte(section) & 0x80 != 0 {
                        section = 8;
                    }
                    pointer = tune.word((0x42 + tune.byte(section) as usize) * 4) as usize;
                }
            }
            let mut s = [0i16; 2];
            for v in 0..4 {
                let Some(voice) = voices.get(instv[v]).filter(|_| instv[v] != 0) else { continue };
                let (len, gap, rep) = (voice.len as i64, voice.gap as i64, voice.rep as i64);
                let off = time - start[v];
                let rate = 4000.0 * 2f64.powf(pitchv[v] as f64 / 12.0);
                let matching = off as f64 * rate / 22050.0;
                for j in (matching as i32 - 4)..=(matching as i32 + 4) {
                    if j < 0 {
                        continue;
                    }
                    let mut jj = j as i64;
                    if jj >= len {
                        if rep == 0 {
                            continue;
                        }
                        jj = (len - rep) + (jj - len) % rep;
                    }
                    let w = if jj >= len - gap && gap > 0 {
                        ((jj - (len - gap)) as f64 * mulaw[voice.byte(jj + 44 - rep) as usize] as f64
                            + (len - jj) as f64 * mulaw[voice.byte(jj + 44) as usize] as f64)
                            / gap as f64
                            / 4.0
                    } else {
                        (mulaw[voice.byte(jj + 44) as usize] / 4) as f64
                    };
                    let x = std::f64::consts::PI * (j as f64 - matching);
                    let term = if x == 0.0 { w / 1.3 } else { w * ((x as f32).sin() as f64) / (1.3 * x) };
                    s[v & 1] = s[v & 1].wrapping_add(term as i32 as i16);
                }
            }
            out.push(s);
        }
        out
    }

    #[test]
    fn loudness_matches_c_hook() {
        let a = crate::assets::testdata::assets();
        let secs = 8;
        let c = c_music(&a.voices, &a.title_music, secs * 22050);
        let mut m = title();
        let hq = render(&mut m, secs * 48_000);
        for side in 0..2 {
            let rc = rms(c.iter().map(|s| s[side] as f32 / 32768.0));
            let rh = rms(hq.iter().map(|o| o[side]));
            println!("music side {side}: C rms {rc:.4}, HQ rms {rh:.4}, ratio {:.3}", rh / rc);
            assert!((0.75..=1.25).contains(&(rh / rc)), "side {side} loudness ratio {}", rh / rc);
        }
    }
}
