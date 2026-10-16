//! sound.c `bidforsound`, `bidforsoundforce`, `soundclaimmaybe`, `soundclaimexplo`,
//! `soundclaim`, `wipesoundtab` → channel-resolved `SfxEvent`s.
//!
//! SDL_mixer emulation: a channel is "playing" while `tick_no < chan_busy_until[c]`
//! (`SfxId::ticks()`). `soundclaimmaybe`: claim only if not playing (the C volume
//! comparison is always false because chunk volume is 0xff). Explosion channel (3): first
//! idle channel of 3,4,5,6,7. `bidforsoundforce`: always claims `r0 & 7`. The C callers
//! pass (channel, samp, vol, pitch, volslide, pitchslide, frames, stereo, chunk); only
//! channel, vol, stereo and the chunk matter here. `stereo` is clamped to −127..=127.
//!
//! `options.soundtype`/`sound_available` and the `options.soundvol` cap are host concerns
//! (master volume, PORTING_NOTES #14); events are always emitted.

use crate::audio::sfx::SfxId;
use crate::sim::{SfxEvent, Sim, EXPLOCHANNEL};

impl Sim {
    /// `bidforsound(r0, …, r2 = vol, …, r7 = stereo, chunk)`.
    pub fn bidforsound(&mut self, channel: i32, vol: i32, stereo: i32, id: SfxId) {
        let c = channel & 7;
        if c == EXPLOCHANNEL {
            self.soundclaimexplo(vol, stereo, id);
        } else {
            self.soundclaimmaybe(c, vol, stereo, id);
        }
    }

    /// `bidforsoundforce(…)`.
    pub fn bidforsoundforce(&mut self, channel: i32, vol: i32, stereo: i32, id: SfxId) {
        self.soundclaim(channel & 7, vol, stereo, id);
    }

    /// `soundclaim(c, …)`: emit the event and mark the channel busy.
    pub fn soundclaim(&mut self, channel: i32, vol: i32, stereo: i32, id: SfxId) {
        let c = (channel & 7) as usize;
        self.sfx_events.push(SfxEvent {
            channel: c as u8,
            id,
            volume: vol.clamp(0, 127) as u8,
            stereo: stereo.clamp(-127, 127) as i8,
        });
        // Mix_HaltChannel + Mix_PlayChannelTimed: the new chunk replaces whatever played.
        self.chan_busy_until[c] = self.tick_no as i64 + id.ticks() as i64;
    }

    /// `wipesoundtab()`: in the SDL port it only calls the empty stub
    /// `swi_stasis_volslide(c, 0xfc00, 0)` for every channel. No channel is halted, so
    /// `Mix_Playing` (chan_busy_until) is unchanged.
    pub fn wipesoundtab(&mut self) {}

    /// Port bookkeeping, not in the C: every SDL_mixer channel is idle. Call it only after the
    /// host has stopped all chunks (HostRequest::StopSfx), or before a new game, because
    /// tick_no is frozen in the menus while the C chunks would have run out in real time.
    pub fn halt_all_channels(&mut self) {
        self.chan_busy_until = [0; 8];
    }

    /// `Mix_Playing(c)`.
    fn chan_playing(&self, c: i32) -> bool {
        (self.tick_no as i64) < self.chan_busy_until[(c & 7) as usize]
    }

    /// `soundclaimmaybe`: `!Mix_Playing(c) || chunk->volume(0xff) < vol` — the second term
    /// is never true (vol <= 0x7f).
    fn soundclaimmaybe(&mut self, c: i32, vol: i32, stereo: i32, id: SfxId) {
        if !self.chan_playing(c) {
            self.soundclaim(c, vol, stereo, id);
        }
    }

    /// `soundclaimexplo` (bidforexplo): first idle channel of 3..=7, else nothing.
    fn soundclaimexplo(&mut self, vol: i32, stereo: i32, id: SfxId) {
        if let Some(c) = (3..=7).find(|&c| !self.chan_playing(c)) {
            self.soundclaim(c, vol, stereo, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{FIRECHANNEL, PLAYERCHANNEL};

    fn chans(s: &Sim) -> Vec<u8> {
        s.sfx_events.iter().map(|e| e.channel).collect()
    }

    #[test]
    fn claim_marks_busy_until_chunk_ends() {
        let mut s = crate::sim::testutil::sim();
        s.tick_no = 100;
        let id = SfxId::JUMP; // 4 frames → 8 ticks
        assert_eq!(id.ticks(), 8);
        s.bidforsound(PLAYERCHANNEL, 0x5f, 0, id);
        assert_eq!(s.chan_busy_until[0], 108);
        s.tick_no = 107;
        s.bidforsound(PLAYERCHANNEL, 0x7f, 0, SfxId::FIRE);
        assert_eq!(s.sfx_events.len(), 1, "busy channel: bid ignored");
        s.tick_no = 108;
        s.bidforsound(PLAYERCHANNEL | 8, 0x7f, 0, SfxId::FIRE); // r0 & 7
        assert_eq!(chans(&s), vec![0, 0]);
        assert_eq!(s.sfx_events[1].id, SfxId::FIRE);
        // other channels are independent
        s.bidforsound(FIRECHANNEL, 0x7f, 0, SfxId::FIRE);
        assert_eq!(chans(&s), vec![0, 0, 1]);
    }

    #[test]
    fn explo_channel_selection() {
        let mut s = crate::sim::testutil::sim();
        s.tick_no = 10;
        for _ in 0..6 {
            s.bidforsound(EXPLOCHANNEL, 0x7c, 0, SfxId::EXPLO);
        }
        assert_eq!(chans(&s), vec![3, 4, 5, 6, 7], "6th explosion finds no idle channel");
        // channel 5 finishes first → next explosion goes there
        s.chan_busy_until[5] = 0;
        s.bidforsound(EXPLOCHANNEL, 0x7c, 0, SfxId::EXPLO);
        assert_eq!(*chans(&s).last().unwrap(), 5);
        // non-explo channels are not used for explosions
        assert!(s.chan_busy_until[..3].iter().all(|&b| b == 0));
    }

    #[test]
    fn force_always_claims_and_clamps() {
        let mut s = crate::sim::testutil::sim();
        s.tick_no = 5;
        s.bidforsoundforce(11, 0x1f, 0, SfxId::SHUTDOWN_1);
        s.bidforsoundforce(3, 300, 900, SfxId::SHUTDOWN_2);
        s.bidforsoundforce(3, -4, -900, SfxId::SHUTDOWN_3);
        assert_eq!(chans(&s), vec![3, 3, 3]);
        let e = s.sfx_events[1];
        assert_eq!((e.volume, e.stereo), (127, 127));
        let e = s.sfx_events[2];
        assert_eq!((e.volume, e.stereo), (0, -127));
        assert_eq!(s.chan_busy_until[3], 5 + 200);
        s.halt_all_channels();
        s.bidforsound(EXPLOCHANNEL, 0x7f, 0, SfxId::ATOM);
        assert_eq!(*chans(&s).last().unwrap(), 3);
    }

    /// C wipesoundtab() only calls the empty `swi_stasis_volslide` stub, so the shutdown
    /// chord (channels 3/2/1, 200 ticks) still blocks those channels after exitneuron().
    #[test]
    fn exitneuron_keeps_shutdown_chord_channels_busy() {
        use crate::sim::game::GameStart;
        use crate::sim::{TickInput, SPARECHANNEL};
        let mut s = crate::sim::testutil::sim();
        s.start_game(GameStart { mentalzone: 1, ..Default::default() }).expect("start");
        s.tick(TickInput::default());
        s.change_zone(4);
        s.tick(TickInput::default());
        assert!(s.in_neuron);
        s.halt_all_channels();
        // player.c: touching the shutdown block forces channels 3, 2, 1 (100 frames each)
        let t = s.tick_no as i64;
        s.bidforsoundforce(3, 0x1f, (3 << 6) - 96, SfxId::SHUTDOWN_1);
        s.bidforsoundforce(2, 0x1f, (2 << 6) - 96, SfxId::SHUTDOWN_2);
        s.bidforsoundforce(1, 0x1f, (1 << 6) - 96, SfxId::SHUTDOWN_3);
        let busy = s.chan_busy_until;
        assert_eq!(&busy[1..4], &[t + 200; 3]);
        // shutdownctr reaches 150 → change_zone(0); the next tick runs loadzone → exitneuron
        s.tick_no = (t + 149) as u32;
        s.change_zone(0);
        s.loadzone();
        assert!(!s.in_neuron);
        assert_eq!(s.chan_busy_until, busy, "wipesoundtab must not halt any channel");
        s.tick_no = (t + 150) as u32;
        s.sfx_events.clear();
        s.bidforsound(FIRECHANNEL, 0x7f, 0, SfxId::FIRE);
        s.bidforsound(SPARECHANNEL, 0x7e, -127, SfxId::BONUS_2);
        assert!(s.sfx_events.is_empty(), "channels 1/2 still play the chord");
        s.bidforsound(EXPLOCHANNEL, 0x7c, 0, SfxId::EXPLO);
        assert_eq!(chans(&s), vec![4], "explosion skips busy channel 3");
        // once the chord has ended the channels are free again
        s.tick_no = (t + 200) as u32;
        s.bidforsound(FIRECHANNEL, 0x7f, 0, SfxId::FIRE);
        assert_eq!(chans(&s), vec![4, 1]);
    }

    #[test]
    fn halt_all_channels_frees_every_channel() {
        let mut s = crate::sim::testutil::sim();
        s.tick_no = 7;
        for c in 0..8 {
            s.bidforsoundforce(c, 0x7f, 0, SfxId::SHUTDOWN_1);
        }
        s.wipesoundtab();
        assert!(s.chan_busy_until.iter().all(|&b| b == 7 + 200), "wipesoundtab is a no-op");
        s.halt_all_channels();
        assert_eq!(s.chan_busy_until, [0; 8]);
        s.sfx_events.clear();
        s.bidforsound(PLAYERCHANNEL, 0x7f, 0, SfxId::JUMP);
        s.bidforsound(EXPLOCHANNEL, 0x7f, 0, SfxId::EXPLO);
        assert_eq!(chans(&s), vec![0, 3]);
    }
}
