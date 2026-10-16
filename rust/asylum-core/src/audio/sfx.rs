//! Sound-effect catalogue and synthesis (sound.c `make_sound`, init_chunk_* in
//! bullet.c / player.c / alien.c / maze.c).

use std::sync::atomic::{AtomicUsize, Ordering};

use super::kernel::{Kernel, VoiceTable, GAIN};
use super::{C_RATE, RATE};
use crate::assets::Voices;

/// Output samples per `frames` unit (make_sound renders frames*882 samples @ 22050 Hz,
/// i.e. 40 ms = 1920 samples @ 48 kHz).
pub const SAMPLES_PER_FRAME: usize = 1920;

/// `fullpitch` from asylum.h.
pub const FULLPITCH: i32 = 0x2155;

/// Identifier of a pre-synthesized chunk (index into `SFX_TABLE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SfxId(pub u16);

/// Parameters of `make_sound(samp, initpitch, volslide, pitchslide, frames)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SfxParams {
    pub samp: u8,
    pub initpitch: i32,
    pub volslide: i32,
    pub pitchslide: i32,
    pub frames: u8,
}

const fn p(samp: u8, initpitch: i32, volslide: i32, pitchslide: i32, frames: u8) -> SfxParams {
    SfxParams { samp, initpitch, volslide, pitchslide, frames }
}

const SHOOT_SLIDE: i32 = (FULLPITCH << 16) | 0xfe00;

impl SfxId {
    // bullet.c
    pub const BULLET_1: SfxId = SfxId(0);
    pub const BULLET_2: SfxId = SfxId(1);
    pub const BULLET_3: SfxId = SfxId(2);
    // player.c
    pub const ELEC_1: SfxId = SfxId(3);
    pub const ELEC_2: SfxId = SfxId(4);
    pub const ELEC_3: SfxId = SfxId(5);
    pub const JUMP: SfxId = SfxId(6);
    pub const SHUTDOWN_1: SfxId = SfxId(7);
    pub const SHUTDOWN_2: SfxId = SfxId(8);
    pub const SHUTDOWN_3: SfxId = SfxId(9);
    pub const TELEP_1: SfxId = SfxId(10);
    pub const TELEP_2: SfxId = SfxId(11);
    pub const TELEP_3: SfxId = SfxId(12);
    pub const BLAM: SfxId = SfxId(13);
    pub const FIRE: SfxId = SfxId(14);
    pub const BLAMFIRE: SfxId = SfxId(15);
    pub const ROCKET: SfxId = SfxId(16);
    pub const WEAPON_1: SfxId = SfxId(17);
    pub const WEAPON_2: SfxId = SfxId(18);
    pub const WEAPON_3: SfxId = SfxId(19);
    pub const WEAPON_4: SfxId = SfxId(20);
    // alien.c
    pub const EXPLO: SfxId = SfxId(21);
    pub const ATOM: SfxId = SfxId(22);
    pub const SPINFIRE: SfxId = SfxId(23);
    pub const SPINPOWERFIRE: SfxId = SfxId(24);
    pub const SHOOT: SfxId = SfxId(25);
    pub const SHOOTNUTTER: SfxId = SfxId(26);
    // maze.c
    pub const FUELAIR: SfxId = SfxId(27);
    pub const BONUS_1: SfxId = SfxId(28);
    pub const BONUS_2: SfxId = SfxId(29);
    /// CHUNK_STUNNED[i], i in 0..17 → ids 30..47
    pub const fn stunned(i: u8) -> SfxId {
        SfxId(30 + if i > 16 { 16 } else { i } as u16)
    }
    /// CHUNK_OBJGOT[i], i in 0..10 → ids 47..57
    pub const fn objgot(i: u8) -> SfxId {
        SfxId(47 + if i > 9 { 9 } else { i } as u16)
    }
    pub const COUNT: usize = 57;

    pub fn params(self) -> SfxParams {
        SFX_TABLE[self.0 as usize]
    }

    /// Chunk length in 50 Hz ticks (make_sound renders frames*882 samples @ 22050 Hz).
    pub fn ticks(self) -> i32 {
        self.params().frames as i32 * 2
    }
}

/// Parameters of every chunk, index = SfxId.
pub const SFX_TABLE: [SfxParams; SfxId::COUNT] = {
    let mut t = [p(0, 0, 0, 0, 0); SfxId::COUNT];
    t[0] = p(9, FULLPITCH + 0x1000, 0, SHOOT_SLIDE, 2);
    t[1] = p(10, FULLPITCH + 0x1800, 0, SHOOT_SLIDE, 2);
    t[2] = p(10, FULLPITCH + 0x1000, 0, SHOOT_SLIDE, 2);
    t[3] = p(19, 0x2000, 0xff00, 0, 10);
    t[4] = p(19, 0x2400, 0xff00, 0, 10);
    t[5] = p(19, 0x2800, 0xff00, 0, 10);
    t[6] = p(1, 0x4600, 0, 0, 4);
    t[7] = p(19, 0x3300, (0x7f << 25) | 0x1000, 0x100ff00, 100);
    t[8] = p(19, 0x3200, (0x7f << 25) | 0x1000, 0x100ff00, 100);
    t[9] = p(19, 0x3100, (0x7f << 25) | 0x1000, 0x100ff00, 100);
    t[10] = p(1, 0x1000, 0, 0, 50);
    t[11] = p(16, 0x1000, 0xff00, 0x50000a00, 25);
    t[12] = p(16, 0x4000, 0xff00, 0x1000f400, 25);
    t[13] = p(4, FULLPITCH, 0, 0, 8);
    t[14] = p(5, FULLPITCH, 0, 0, 2);
    t[15] = p(19, FULLPITCH, 0, (FULLPITCH << 17) + 0x200, 40);
    t[16] = p(6, FULLPITCH, 0, 0, 2);
    t[17] = p(16, 0x3000, 0xff00, 0x60000200, 10);
    t[18] = p(16, 0x3800, 0xff00, 0x60000200, 10);
    t[19] = p(19, 0x3000, 0xff00, 0x60000800, 10);
    t[20] = p(19, 0x3800, 0xff00, 0x60000800, 10);
    t[21] = p(3, 0x3800, 0, 0, 10);
    t[22] = p(4, 0x2800, 0, 0, 50);
    t[23] = p(9, FULLPITCH + 0x1000, 0, 0, 2);
    t[24] = p(10, FULLPITCH, 0, 0, 2);
    t[25] = p(9, FULLPITCH + 0x1000, 0, SHOOT_SLIDE, 2);
    t[26] = p(10, FULLPITCH, 0, SHOOT_SLIDE, 2);
    t[27] = p(7, 0x6000, 0xff80, 0x2000ff00, 50);
    t[28] = p(2, 0x2200, 0, 0, 5);
    t[29] = p(2, 0x2000, 0, 0, 5);
    let mut i = 0;
    while i < 17 {
        t[30 + i] = p(8, 0x4000 - (i as i32) * 0x100, 0, 0, 50);
        i += 1;
    }
    let mut j = 0;
    while j < 10 {
        t[47 + j] = p(2, (((j as i32) + 1) << 6) + 0x3000, 0, 0, 3);
        j += 1;
    }
    t
};

/// A synthesized mono chunk, 16-bit PCM at `audio::RATE`.
#[derive(Clone, Debug, Default)]
pub struct Pcm {
    pub samples: Vec<i16>,
}

fn silent(params: SfxParams) -> Pcm {
    Pcm { samples: vec![0; params.frames as usize * SAMPLES_PER_FRAME] }
}

/// Synthesize one chunk (port of make_sound at 48 kHz, see audio/mod.rs notes).
pub fn synth(voices: &Voices, kernel: &super::kernel::Kernel, params: SfxParams) -> Pcm {
    match voices.get(params.samp as usize) {
        Some(v) => synth_table(&VoiceTable::new(v), kernel, params),
        None => silent(params),
    }
}

/// make_sound's sample loop, with every C-sample-time quantity evaluated at
/// `time = n * 22050/48000` and the source position advanced by `rate/48000` per sample.
fn synth_table(table: &VoiceTable, kernel: &Kernel, params: SfxParams) -> Pcm {
    let numsamples = params.frames as usize * SAMPLES_PER_FRAME;
    let ps = (params.pitchslide as u16 as i16) as f64 * 22.0 / C_RATE;
    let vs = (params.volslide as u16 as i16) as f64 / C_RATE;
    let tscale = C_RATE / RATE as f64;
    let gain = GAIN as f64;
    let mut matching = 0.0f64;
    let mut samples = Vec::with_capacity(numsamples);
    for n in 0..numsamples {
        let time = n as f64 * tscale;
        let pitch = params.initpitch as f64 + ps * time;
        let old_sample_rate = 500.0 * (pitch / 4096.0).exp2();
        matching += old_sample_rate / RATE as f64;
        let vscale = (0x7f as f64 + vs * time).min(255.0);
        let mono = if vscale > 0.0 { table.interp(kernel, matching) as f64 * gain * (vscale / 127.0) } else { 0.0 };
        samples.push(mono.round() as i16); // `as` saturates to the i16 range
    }
    Pcm { samples }
}

/// Synthesize every chunk in `SFX_TABLE` using `threads` worker threads
/// (`std::thread::scope`; 0 = `available_parallelism`). Result index = SfxId.
pub fn synth_all(voices: &Voices, threads: usize) -> Vec<Pcm> {
    let kernel = Kernel::shared();

    // Identical parameter sets (e.g. BULLET_1 / SHOOT) are rendered once.
    let mut jobs: Vec<SfxParams> = Vec::new();
    let mut job_of = [0usize; SfxId::COUNT];
    for (id, p) in SFX_TABLE.iter().enumerate() {
        job_of[id] = match jobs.iter().position(|q| q == p) {
            Some(j) => j,
            None => {
                jobs.push(*p);
                jobs.len() - 1
            }
        };
    }
    let mut tables: Vec<Option<VoiceTable>> = (0..256).map(|_| None).collect();
    for p in &jobs {
        let s = p.samp as usize;
        if tables[s].is_none() {
            tables[s] = voices.get(s).map(VoiceTable::new);
        }
    }
    // Longest chunks first, handed out dynamically.
    let mut order: Vec<usize> = (0..jobs.len()).collect();
    order.sort_by_key(|&j| std::cmp::Reverse(jobs[j].frames));

    let threads = if threads == 0 { std::thread::available_parallelism().map_or(1, |n| n.get()) } else { threads };
    let threads = threads.clamp(1, order.len().max(1));
    let next = AtomicUsize::new(0);
    let rendered: Vec<(usize, Pcm)> = std::thread::scope(|sc| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                sc.spawn(|| {
                    let mut out = Vec::new();
                    while let Some(&j) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let p = jobs[j];
                        let pcm = match &tables[p.samp as usize] {
                            Some(t) => synth_table(t, kernel, p),
                            None => silent(p),
                        };
                        out.push((j, pcm));
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("sfx synthesis worker panicked")).collect()
    });

    let mut by_job: Vec<Option<Pcm>> = (0..jobs.len()).map(|_| None).collect();
    for (j, pcm) in rendered {
        by_job[j] = Some(pcm);
    }
    let mut last_use = vec![0usize; jobs.len()];
    for (id, &j) in job_of.iter().enumerate() {
        last_use[j] = id;
    }
    (0..SfxId::COUNT)
        .map(|id| {
            let j = job_of[id];
            let pcm = if last_use[j] == id { by_job[j].take() } else { by_job[j].clone() };
            pcm.expect("every job rendered")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::voice::mulaw_table;
    use crate::assets::Voice;
    use std::time::Instant;

    fn rms(s: &[i16]) -> f64 {
        if s.is_empty() {
            return 0.0;
        }
        (s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / s.len() as f64).sqrt()
    }

    /// Direct port of the C `make_sound` sample loop at 22050 Hz (mono).
    fn c_make_sound(voice: &Voice, initpitch: i32, volslide: i32, pitchslide: i32, frames: u8) -> Vec<i16> {
        let mulaw = mulaw_table();
        let numsamples = (frames as f64 * 22050.0 / 50.0 * 2.0) as i32;
        let ps = ((pitchslide & 0xffff) as i16) as f64 * 22.0 / 22050.0;
        let vs = ((volslide & 0xffff) as i16) as f64 / 22050.0;
        let (len, gap, rep) = (voice.len, voice.gap, voice.rep);
        let mut matching = 0.0f64;
        let mut out = Vec::with_capacity(numsamples as usize);
        for time in 0..numsamples {
            let mut mono: u16 = 0;
            let pitch = initpitch as f64 + ps * time as f64;
            let old_sample_rate = 500.0 * 2f64.powf(pitch / 4096.0);
            matching += old_sample_rate / 22050.0;
            for j in (matching as i32 - 32)..=(matching as i32 + 32) {
                if j < 0 {
                    continue;
                }
                let mut jj = j;
                if jj as u32 >= len {
                    if rep == 0 {
                        continue;
                    }
                    jj = (len.wrapping_sub(rep)).wrapping_add((jj as u32).wrapping_sub(len) % rep) as i32;
                }
                let mut w = if jj >= len.wrapping_sub(gap) as i32 {
                    ((jj as u32).wrapping_sub(len.wrapping_sub(gap)) as f64
                        * mulaw[voice.byte(jj as i64 + 44 - rep as i64) as usize] as f64
                        + len.wrapping_sub(jj as u32) as f64 * mulaw[voice.byte(jj as i64 + 44) as usize] as f64)
                        / gap as f64
                        / 4.0
                } else {
                    (mulaw[voice.byte(jj as i64 + 44) as usize] as i32 / 4) as f64
                };
                let mut vscale = 0x7f as f64 + vs * time as f64;
                if vscale > 255.0 {
                    vscale = 255.0;
                }
                if vscale > 0.0 {
                    w *= vscale / 127.0;
                } else {
                    w = 0.0;
                }
                let x = std::f64::consts::PI * (j as f64 - matching);
                let term = if x == 0.0 { w / 1.3 } else { w * ((x as f32).sin() as f64) / (1.3 * x) };
                mono = mono.wrapping_add(term as i32 as i16 as u16);
            }
            out.push(mono as i16);
        }
        out
    }

    #[test]
    fn ids_and_lengths() {
        assert_eq!(SfxId::stunned(0), SfxId(30));
        assert_eq!(SfxId::stunned(16), SfxId(46));
        assert_eq!(SfxId::stunned(99), SfxId(46));
        assert_eq!(SfxId::objgot(0), SfxId(47));
        assert_eq!(SfxId::objgot(9), SfxId(56));
        assert_eq!(SfxId::objgot(9).0 as usize + 1, SfxId::COUNT);
        assert_eq!(SfxId::JUMP.ticks(), 8);
        assert_eq!(SfxId::stunned(3).params(), p(8, 0x4000 - 3 * 0x100, 0, 0, 50));
        assert_eq!(SfxId::objgot(2).params(), p(2, (3 << 6) + 0x3000, 0, 0, 3));
        // make_sound's C length (frames*882 @ 22050 Hz) in seconds, at 48 kHz
        for f in [1u8, 2, 4, 50, 100] {
            assert_eq!(f as usize * 882 * RATE as usize / 22050, f as usize * SAMPLES_PER_FRAME);
        }
        let a = crate::assets::testdata::assets();
        let k = Kernel::shared();
        assert_eq!(synth(&a.voices, k, SfxId::JUMP.params()).samples.len(), 4 * 1920);
        // missing voice → silent chunk of the right length
        let none = synth(&a.voices, k, p(31, 0x1000, 0, 0, 3));
        assert_eq!(none.samples.len(), 3 * 1920);
        assert!(none.samples.iter().all(|&s| s == 0));
    }

    #[test]
    fn synth_all_every_id_non_silent_and_timed() {
        let a = crate::assets::testdata::assets();
        let t0 = Instant::now();
        let bank = synth_all(&a.voices, 0);
        let par = t0.elapsed();
        let t1 = Instant::now();
        let bank1 = synth_all(&a.voices, 1);
        let seq = t1.elapsed();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let total: usize = bank.iter().map(|p| p.samples.len()).sum();
        println!(
            "synth_all: {} chunks, {:.2} s of audio: {:?} with {threads} threads, {:?} with 1 thread",
            bank.len(),
            total as f64 / RATE as f64,
            par,
            seq
        );
        assert_eq!(bank.len(), SfxId::COUNT);
        for (id, pcm) in bank.iter().enumerate() {
            let params = SFX_TABLE[id];
            assert_eq!(pcm.samples.len(), params.frames as usize * SAMPLES_PER_FRAME, "id {id}");
            assert!(a.voices.get(params.samp as usize).is_some(), "id {id} uses a real voice");
            let peak = pcm.samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
            // ROCKET is the quietest chunk (C: rms 151, peak ~360)
            assert!(peak > 200 && rms(&pcm.samples) > 50.0, "id {id} is (nearly) silent: peak {peak}");
            assert_eq!(pcm.samples, bank1[id].samples, "id {id}: thread count changes the output");
        }
        // shared jobs are the same audio, and synth() agrees with synth_all()
        assert_eq!(bank[SfxId::BULLET_1.0 as usize].samples, bank[SfxId::SHOOT.0 as usize].samples);
        for id in [SfxId::JUMP, SfxId::FUELAIR, SfxId::stunned(5)] {
            assert_eq!(synth(&a.voices, Kernel::shared(), id.params()).samples, bank[id.0 as usize].samples);
        }
    }

    #[test]
    fn loudness_matches_c_formula() {
        let a = crate::assets::testdata::assets();
        let k = Kernel::shared();
        // every chunk, including the pitch-sliding ones that alias in both versions
        for id in (0..SfxId::COUNT as u16).map(SfxId) {
            let pr = id.params();
            let v = a.voices.get(pr.samp as usize).unwrap();
            let c = c_make_sound(v, pr.initpitch, pr.volslide, pr.pitchslide, pr.frames);
            assert_eq!(c.len(), pr.frames as usize * 882);
            let hq = synth(&a.voices, k, pr);
            let (rc, rh) = (rms(&c), rms(&hq.samples));
            let ratio = rh / rc;
            println!("{id:?}: C rms {rc:.1}, HQ rms {rh:.1}, ratio {ratio:.3}");
            assert!((0.75..=1.25).contains(&ratio), "{id:?}: loudness ratio {ratio}");
        }
    }
}
