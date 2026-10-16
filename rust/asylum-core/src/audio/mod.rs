//! Audio synthesis from the original "Sta1" voices — high quality, 48 kHz.
//!
//! * [`sfx`] – catalogue of every sound-effect "chunk" the C code builds in `init_chunk_*`
//!   (`make_sound(samp, initpitch, volslide, pitchslide, frames)`) and their offline
//!   synthesis (parallel, std::thread::scope).
//! * [`music`] – streaming re-implementation of `sdl_music_hook` (Bodge tracker, 4 voices).
//! * [`kernel`] – shared band-limited interpolation kernel (Lanczos windowed sinc) and the
//!   per-voice sample tables (`VoiceTable`: loop, gap crossfade, guards).
//!
//! Quality notes: render directly at 48 kHz instead of
//! 22.05 kHz, double precision envelopes, unsigned sample bytes, `gap > 0`/`len == 0` guards,
//! and silence after a non-looping sample ends (the C music hook replayed byte 0 instead).
//! Timings are preserved in *seconds*: anything the C code counts in 22050 Hz samples is
//! scaled by 48000/22050. Loudness matches the C code (`w / 1.3` with a unity-DC kernel).

pub mod kernel;
pub mod music;
pub mod sfx;

/// Output sample rate of everything we synthesize (Godot mix rate is set to the same).
pub const RATE: u32 = 48_000;
/// The SDL port's mixing rate; C timings are expressed in samples at this rate.
pub const C_RATE: f64 = 22_050.0;
