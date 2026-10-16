//! Audio host: buses `Music` and `SFX` (→ Master), `SFX0..SFX7` (panner each, → SFX),
//! eight effect players (the SDL_mixer channels) and a streamed music player.

use asylum_core::audio::sfx::Pcm;
use asylum_core::audio::RATE;
use asylum_core::runtime::Runtime;
use asylum_core::sim::SfxEvent;
use godot::classes::audio_stream_wav::Format;
use godot::classes::{
    AudioEffectPanner, AudioServer, AudioStreamGenerator, DisplayServer, AudioStreamGeneratorPlayback, AudioStreamPlayer, AudioStreamWav,
};
use godot::global::linear_to_db;
use godot::prelude::*;

pub const CHANNELS: usize = 8;
/// Music frames pushed per `push_buffer` call (fixed so the packed buffer is reused).
const MUSIC_CHUNK: usize = 512;
const MUSIC_BUFFER_SECONDS: f32 = 0.5;

fn db(gain: f64) -> f32 {
    if gain <= 0.0 {
        -80.0
    } else {
        (linear_to_db(gain) as f32).max(-80.0)
    }
}

/// UI volume step 0..=10 → gain (v/10)^2.
pub fn volume_gain(v: u8) -> f64 {
    let x = v.min(10) as f64 / 10.0;
    x * x
}

fn ensure_bus(a: &mut Gd<AudioServer>, name: &str, send: &str) -> i32 {
    let mut idx = a.get_bus_index(name);
    if idx < 0 {
        let n = a.get_bus_count();
        a.add_bus();
        idx = n;
        a.set_bus_name(idx, name);
    }
    a.set_bus_send(idx, send);
    idx
}

pub struct Audio {
    sfx_bus: i32,
    music_bus: i32,
    players: Vec<Gd<AudioStreamPlayer>>,
    panners: Vec<Gd<AudioEffectPanner>>,
    streams: Vec<Option<Gd<AudioStreamWav>>>,
    music: Gd<AudioStreamPlayer>,
    /// False in headless runs (dummy audio driver: playbacks are never retired, so a
    /// generator playback would be reported as leaked at exit).
    music_enabled: bool,
    playback: Option<Gd<AudioStreamGeneratorPlayback>>,
    epoch: u32,
    /// Pulled music frames not yet pushed (always < MUSIC_CHUNK after a push round).
    carry: Vec<[f32; 2]>,
    chunk: PackedVector2Array,
}

impl Audio {
    /// Creates the buses and players; `add_child` attaches each player to the scene.
    pub fn new(mut add_child: impl FnMut(Gd<Node>)) -> Audio {
        let mut a = AudioServer::singleton();
        let music_bus = ensure_bus(&mut a, "Music", "Master");
        let sfx_bus = ensure_bus(&mut a, "SFX", "Master");
        let mut players = Vec::with_capacity(CHANNELS);
        let mut panners = Vec::with_capacity(CHANNELS);
        for i in 0..CHANNELS {
            let name = format!("SFX{i}");
            let b = ensure_bus(&mut a, &name, "SFX");
            let existing = (0..a.get_bus_effect_count(b))
                .find_map(|e| a.get_bus_effect(b, e).and_then(|fx| fx.try_cast::<AudioEffectPanner>().ok()));
            let panner = match existing {
                Some(p) => p,
                None => {
                    let p = AudioEffectPanner::new_gd();
                    a.add_bus_effect(b, &p);
                    p
                }
            };
            panners.push(panner);
            let mut p = AudioStreamPlayer::new_alloc();
            p.set_name(&format!("Sfx{i}"));
            p.set_bus(&name);
            add_child(p.clone().upcast());
            players.push(p);
        }

        let mut generator = AudioStreamGenerator::new_gd();
        generator.set_mix_rate(RATE as f32);
        generator.set_buffer_length(MUSIC_BUFFER_SECONDS);
        let mut music = AudioStreamPlayer::new_alloc();
        music.set_name("Music");
        music.set_bus("Music");
        music.set_stream(&generator);
        add_child(music.clone().upcast());

        let mut chunk = PackedVector2Array::new();
        chunk.resize(MUSIC_CHUNK);
        let mut audio = Audio {
            sfx_bus,
            music_bus,
            players,
            panners,
            streams: Vec::new(),
            music,
            music_enabled: DisplayServer::singleton().get_name() != "headless",
            playback: None,
            epoch: 0,
            carry: Vec::with_capacity(1 << 16),
            chunk,
        };
        audio.restart_music();
        audio
    }

    /// Builds one 16-bit mono 48 kHz stream per SfxId.
    pub fn set_bank(&mut self, bank: &[Pcm]) {
        self.streams = bank
            .iter()
            .map(|pcm| {
                if pcm.samples.is_empty() {
                    return None;
                }
                let mut bytes = PackedByteArray::new();
                bytes.resize(pcm.samples.len() * 2);
                for (d, s) in bytes.as_mut_slice().as_chunks_mut::<2>().0.iter_mut().zip(&pcm.samples) {
                    *d = s.to_le_bytes();
                }
                let mut w = AudioStreamWav::new_gd();
                w.set_format(Format::FORMAT_16_BITS);
                w.set_mix_rate(RATE as i32);
                w.set_stereo(false);
                w.set_data(&bytes);
                Some(w)
            })
            .collect();
    }

    /// Plays an effect on its channel (replacing what that channel was playing).
    pub fn play(&mut self, e: &SfxEvent) {
        let ch = e.channel as usize;
        if ch >= CHANNELS {
            return;
        }
        let Some(Some(stream)) = self.streams.get(e.id.0 as usize) else {
            return;
        };
        let p = &mut self.players[ch];
        p.stop();
        p.set_stream(stream);
        p.set_volume_db(db(e.volume as f64 / 128.0));
        self.panners[ch].set_pan((e.stereo as f32 / 127.0).clamp(-1.0, 1.0));
        p.play();
    }

    pub fn stop_all(&mut self) {
        for p in &mut self.players {
            p.stop();
        }
    }

    /// Stops every player (before quitting, so no playback outlives the scene).
    pub fn shutdown(&mut self) {
        self.stop_all();
        self.playback = None;
        self.music.stop();
    }

    pub fn apply(&mut self, sound_mode: u8, sfx_volume: u8, music_volume: u8) {
        let mut a = AudioServer::singleton();
        let sg = volume_gain(sfx_volume);
        let mg = volume_gain(music_volume);
        a.set_bus_volume_db(self.sfx_bus, db(sg));
        a.set_bus_mute(self.sfx_bus, sound_mode == 0 || sg <= 0.0);
        a.set_bus_volume_db(self.music_bus, db(mg));
        a.set_bus_mute(self.music_bus, sound_mode < 2 || mg <= 0.0);
    }

    fn restart_music(&mut self) {
        self.carry.clear();
        if !self.music_enabled {
            return;
        }
        self.music.stop();
        self.music.play();
        self.playback = self.music.get_stream_playback().and_then(|p| p.try_cast::<AudioStreamGeneratorPlayback>().ok());
    }

    /// Streams music PCM from the runtime into the generator.
    pub fn pump_music(&mut self, rt: &Runtime) {
        let epoch = rt.music_epoch();
        if epoch != self.epoch {
            // New track or stop: drop what the generator still holds so the change is immediate.
            self.epoch = epoch;
            self.restart_music();
        }
        let Some(pb) = self.playback.as_mut() else {
            return;
        };
        let avail = pb.get_frames_available().max(0) as usize;
        if avail < MUSIC_CHUNK {
            return;
        }
        let want = (avail / MUSIC_CHUNK * MUSIC_CHUNK).saturating_sub(self.carry.len());
        if !rt.pull_music(self.epoch, &mut self.carry, want) {
            // the track changed meanwhile: restart with it on the next frame
            return;
        }
        let mut off = 0;
        while self.carry.len() - off >= MUSIC_CHUNK {
            for (d, s) in self.chunk.as_mut_slice().iter_mut().zip(&self.carry[off..off + MUSIC_CHUNK]) {
                *d = Vector2::new(s[0], s[1]);
            }
            pb.push_buffer(&self.chunk);
            off += MUSIC_CHUNK;
        }
        self.carry.drain(..off);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_curve() {
        assert_eq!(volume_gain(0), 0.0);
        assert_eq!(volume_gain(10), 1.0);
        assert!((volume_gain(5) - 0.25).abs() < 1e-12);
        assert_eq!(volume_gain(200), 1.0);
    }
}
