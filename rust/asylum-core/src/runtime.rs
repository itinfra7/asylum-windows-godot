//! Threads and channels (docs §2). Nothing here touches Godot.
//!
//! * [`load`] — decodes assets, strings, fonts, builds the atlas, all in parallel
//!   (std::thread::scope). Called by the host once at start-up.
//! * [`Runtime::start`] spawns
//!   - the **game thread**: owns `App`, runs exactly 50 ticks/s with `sleep_until`
//!     (scheduled time `t0 + n*20ms`; if more than 5 ticks behind, resynchronise),
//!     consumes `InputEvent`s (mpsc) and the held mask (AtomicU32), builds a `RenderFrame`
//!     after each tick and publishes it with its scheduled tick time into a `Mutex` slot
//!     (swap, double buffered: the previous Arc is reused when uniquely owned), forwards SFX
//!     events/requests to the host channel and music commands to the music thread;
//!   - the **music thread**: owns `MusicSynth`, keeps ≈0.25 s of 48 kHz stereo PCM in a
//!     ring (`Mutex<VecDeque<[f32;2]>>`), handles Play/Stop commands (flushes the ring);
//!   - the **SFX synthesis thread**: runs `audio::sfx::synth_all` (itself parallel) and
//!     hands the bank to the host (`take_sfx_bank`), then tells the App it is ready.
//! * The host (main thread) calls `send`, `set_held`, `set_view`, `latest_frame`,
//!   `poll_events`, `pull_music`, `take_sfx_bank`, and `shutdown` (joins all threads,
//!   saves config).
//!
//! Panics in the worker threads are caught: a game-thread panic is reported by
//! [`Runtime::failure`] (the host shows it and quits), a music-thread panic only silences the
//! music, and an SFX-synthesis panic leaves the game without effects (but still running).

use std::any::Any;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::assets::{GameAssets, RawFiles, Tune};
use crate::atlas::Atlas;
use crate::audio::music::{MusicSynth, MusicTrack};
use crate::audio::sfx::Pcm;
use crate::config::Config;
use crate::font::GlyphSet;
use crate::frontend::{App, AppInit, AppOutputs, HostRequest};
use crate::i18n::{Lang, Strings};
use crate::input::InputEvent;
use crate::render::{RenderCtx, RenderFrame};
use crate::sim::SfxEvent;
use crate::storage::Storage;

/// Everything the host provides at start-up.
pub struct LoadInput {
    pub files: RawFiles,
    /// game/locale/strings.csv
    pub strings_csv: String,
    /// game/fonts/asylum-ko.bdf and asylum-zh_hans.bdf (subset BDFs)
    pub bdf_ko: String,
    pub bdf_zh: String,
    /// Portable user-data directory (absolute).
    pub userdir: PathBuf,
    /// OS locale, e.g. "ko_KR" (used when the config has no language).
    pub os_locale: String,
}

/// Decoded, immutable resources.
pub struct Loaded {
    pub assets: Arc<GameAssets>,
    pub atlas: Arc<Atlas>,
    pub strings: Arc<Strings>,
    pub glyphs: Arc<GlyphSet>,
    pub config: Config,
    pub userdir: PathBuf,
}

fn panic_message(p: &(dyn Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn joined<T>(r: thread::Result<Result<T, String>>, what: &str) -> Result<T, String> {
    match r {
        Ok(v) => v,
        Err(p) => Err(format!("{what}: {}", panic_message(p.as_ref()))),
    }
}

/// Language used when the config has none: `ko*` → Korean, `zh*` → Simplified Chinese,
/// else English (docs §8).
pub fn locale_language(os_locale: &str) -> Lang {
    Lang::from_code(os_locale.trim()).unwrap_or(Lang::En)
}

/// Parallel start-up decoding (see module docs).
pub fn load(input: LoadInput) -> Result<Loaded, String> {
    let LoadInput { files, strings_csv, bdf_ko, bdf_zh, userdir, os_locale } = input;
    let (files, strings_csv, bdf_ko, bdf_zh, userdir) = (&files, &strings_csv, &bdf_ko, &bdf_zh, &userdir);
    let (assets, atlas, strings, glyphs, config) = thread::scope(|sc| {
        let assets_h = sc.spawn(move || GameAssets::load(files));
        let config_h = sc.spawn(move || Ok(Storage::new(userdir.clone()).load_config()));
        let ko_h = sc.spawn(move || crate::font::parse_bdf(bdf_ko).map_err(|e| format!("fonts/asylum-ko.bdf: {e}")));
        let zh_h = sc.spawn(move || crate::font::parse_bdf(bdf_zh).map_err(|e| format!("fonts/asylum-zh_hans.bdf: {e}")));
        // Strings + glyph styling on this thread while the assets decode.
        let glyph_side = (|| {
            let strings = catch_unwind(AssertUnwindSafe(|| {
                Strings::parse_csv(strings_csv).map_err(|e| format!("locale/strings.csv: {e}"))
            }));
            let ko = joined(ko_h.join(), "fonts/asylum-ko.bdf");
            let zh = joined(zh_h.join(), "fonts/asylum-zh_hans.bdf");
            let (strings, ko, zh) = (joined(strings, "locale/strings.csv")?, ko?, zh?);
            let glyphs = catch_unwind(AssertUnwindSafe(|| Ok(crate::font::build_glyph_set(&strings.required_chars(), &ko, &zh))));
            let glyphs = joined(glyphs, "glyphs")?;
            Ok::<_, String>((strings, glyphs))
        })();
        // Join every handle before evaluating results (an unjoined panicked thread would make
        // the scope itself panic).
        let assets = joined(assets_h.join(), "assets");
        let config = joined(config_h.join(), "config");
        let (strings, glyphs) = glyph_side?;
        let assets = assets?;
        let mut config: Config = config?;
        let atlas = catch_unwind(AssertUnwindSafe(|| Ok(Atlas::build(&assets, &glyphs))));
        let atlas = joined(atlas, "atlas")?;
        if config.language.is_none() {
            config.language = Some(locale_language(&os_locale));
        }
        Ok::<_, String>((assets, atlas, strings, glyphs, config))
    })?;
    Ok(Loaded {
        assets: Arc::new(assets),
        atlas: Arc::new(atlas),
        strings: Arc::new(strings),
        glyphs: Arc::new(glyphs),
        config,
        userdir: userdir.clone(),
    })
}

/// Events for the host.
#[derive(Clone, Debug)]
pub enum HostEvent {
    Sfx(SfxEvent),
    Request(HostRequest),
}

/// A published frame with its scheduled tick time.
#[derive(Clone)]
pub struct PublishedFrame {
    pub frame: Arc<RenderFrame>,
    pub tick_time: Instant,
}

/// Logic tick period (1 / TICK_HZ).
pub const TICK: Duration = Duration::from_millis(1000 / crate::TICK_HZ as u64);
/// More than this many ticks behind schedule → resynchronise instead of catching up.
pub const MAX_BEHIND: u32 = 5;
/// Music PCM kept ready by the music thread (≈0.25 s at 48 kHz).
pub const MUSIC_READY_FRAMES: usize = 12_000;
/// Granularity of music rendering.
pub const MUSIC_CHUNK: usize = 1024;
/// Idle sleep of the music thread.
const MUSIC_IDLE: Duration = Duration::from_millis(5);
/// RenderFrame buffers kept for reuse by the game thread.
const FRAME_POOL: usize = 3;

/// Tempo argument of swi_sound_qtempo for a track (0x980 = the slow game-over music).
pub fn music_tempo(track: MusicTrack) -> i32 {
    match track {
        MusicTrack::GameOver => 0x980,
        _ => 0x1000,
    }
}

/// The tune a track plays (asylum.c music slots; see frontend docs).
pub fn music_tune(assets: &GameAssets, track: MusicTrack) -> &Tune {
    match track {
        MusicTrack::Title => &assets.title_music,
        MusicTrack::GameOver => &assets.death_music,
        MusicTrack::Zone { zone, part } => {
            let z = &assets.zones[(zone as usize).min(assets.zones.len() - 1)];
            if part == 2 {
                &z.music2
            } else {
                &z.music1
            }
        }
    }
}

/// Sleep until `deadline`: OS sleep until 1 ms before it, then yield-spin.
pub fn sleep_until(deadline: Instant) {
    const MARGIN: Duration = Duration::from_millis(1);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > MARGIN {
            thread::sleep(left - MARGIN);
        } else {
            thread::yield_now();
        }
    }
}

/// Deadline of the tick after the one scheduled at `scheduled` (fixed schedule; when more
/// than `MAX_BEHIND` ticks late, the schedule restarts at `now`).
pub fn next_deadline(scheduled: Instant, now: Instant, period: Duration) -> Instant {
    let next = scheduled + period;
    if now.saturating_duration_since(next) > period * MAX_BEHIND {
        now
    } else {
        next
    }
}

fn pack_view(w: f32, h: f32) -> u64 {
    ((w.to_bits() as u64) << 32) | h.to_bits() as u64
}

fn unpack_view(v: u64) -> (f32, f32) {
    (f32::from_bits((v >> 32) as u32), f32::from_bits(v as u32))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding one of these locks cannot leave the data inconsistent.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// State shared between the host and the worker threads.
struct Shared {
    stop: AtomicBool,
    held: AtomicU32,
    view: AtomicU64,
    init_view: (f32, f32),
    frame: Mutex<Option<PublishedFrame>>,
    music: Mutex<VecDeque<[f32; 2]>>,
    music_epoch: AtomicU32,
    sfx_bank: Mutex<Option<Vec<Pcm>>>,
    sfx_ready: AtomicBool,
    config: Mutex<Config>,
    quit: AtomicBool,
    failure: Mutex<Option<String>>,
}

impl Shared {
    fn new(config: Config, view: (f32, f32)) -> Shared {
        Shared {
            stop: AtomicBool::new(false),
            held: AtomicU32::new(0),
            view: AtomicU64::new(pack_view(view.0, view.1)),
            init_view: view,
            frame: Mutex::new(None),
            music: Mutex::new(VecDeque::with_capacity(MUSIC_READY_FRAMES + MUSIC_CHUNK)),
            music_epoch: AtomicU32::new(0),
            sfx_bank: Mutex::new(None),
            sfx_ready: AtomicBool::new(false),
            config: Mutex::new(config),
            quit: AtomicBool::new(false),
            failure: Mutex::new(None),
        }
    }
}

/// What the game thread needs from the frontend (implemented by `App`; mocked in tests).
trait GameApp {
    fn push_event(&mut self, ev: InputEvent);
    fn set_view(&mut self, w: f32, h: f32);
    fn set_sfx_ready(&mut self);
    fn tick(&mut self, held: u32);
    fn render(&mut self, out: &mut RenderFrame);
    fn drain(&mut self, out: &mut AppOutputs);
    fn config(&self) -> &Config;
    fn wants_quit(&self) -> bool;
}

impl GameApp for App {
    fn push_event(&mut self, ev: InputEvent) {
        App::push_event(self, ev)
    }
    fn set_view(&mut self, w: f32, h: f32) {
        App::set_view(self, w, h)
    }
    fn set_sfx_ready(&mut self) {
        App::set_sfx_ready(self)
    }
    fn tick(&mut self, held: u32) {
        App::tick(self, held)
    }
    fn render(&mut self, out: &mut RenderFrame) {
        App::render(self, out)
    }
    fn drain(&mut self, out: &mut AppOutputs) {
        App::drain(self, out)
    }
    fn config(&self) -> &Config {
        App::config(self)
    }
    fn wants_quit(&self) -> bool {
        App::wants_quit(self)
    }
}

/// A streaming stereo PCM source (the music synth; mocked in tests).
trait PcmSource {
    fn fill(&mut self, out: &mut [[f32; 2]]);
}

impl PcmSource for MusicSynth {
    fn fill(&mut self, out: &mut [[f32; 2]]) {
        MusicSynth::fill(self, out)
    }
}

enum MusicCmd {
    Play(MusicTrack),
    Stop,
}

/// Takes a uniquely owned frame buffer from the pool (or allocates one when the host still
/// holds all of them).
fn take_spare(pool: &mut Vec<Arc<RenderFrame>>) -> Arc<RenderFrame> {
    for i in 0..pool.len() {
        if Arc::get_mut(&mut pool[i]).is_some() {
            return pool.swap_remove(i);
        }
    }
    Arc::new(RenderFrame::default())
}

/// Game thread body. Returns the App's final config.
fn game_loop<A: GameApp>(
    sh: &Shared,
    app: &mut A,
    ev_rx: &Receiver<InputEvent>,
    host_tx: &Sender<HostEvent>,
    music_tx: &Sender<MusicCmd>,
    period: Duration,
) -> Config {
    // The App was built with the start-up view; later changes are compared against it.
    let mut view = sh.init_view;
    let mut sfx_told = false;
    let mut outputs = AppOutputs::default();
    let mut pool: Vec<Arc<RenderFrame>> = (0..2).map(|_| Arc::new(RenderFrame::default())).collect();
    let mut cfg = app.config().clone();
    *lock(&sh.config) = cfg.clone();
    let mut scheduled = Instant::now();
    while !sh.stop.load(Ordering::Acquire) {
        sleep_until(scheduled);
        while let Ok(ev) = ev_rx.try_recv() {
            app.push_event(ev);
        }
        let v = unpack_view(sh.view.load(Ordering::Acquire));
        if v != view {
            view = v;
            app.set_view(v.0, v.1);
        }
        if !sfx_told && sh.sfx_ready.load(Ordering::Acquire) {
            sfx_told = true;
            app.set_sfx_ready();
        }
        app.tick(sh.held.load(Ordering::Acquire));

        let mut buf = take_spare(&mut pool);
        if let Some(f) = Arc::get_mut(&mut buf) {
            app.render(f);
        }
        let old = lock(&sh.frame).replace(PublishedFrame { frame: buf, tick_time: scheduled });
        if let Some(old) = old {
            if pool.len() < FRAME_POOL {
                pool.push(old.frame);
            }
        }

        app.drain(&mut outputs);
        for e in outputs.sfx.drain(..) {
            let _ = host_tx.send(HostEvent::Sfx(e));
        }
        if let Some(m) = outputs.music.take() {
            let _ = music_tx.send(match m {
                Some(t) => MusicCmd::Play(t),
                None => MusicCmd::Stop,
            });
        }
        for r in outputs.requests.drain(..) {
            if r == HostRequest::Quit {
                sh.quit.store(true, Ordering::Release);
            }
            let _ = host_tx.send(HostEvent::Request(r));
        }
        if app.wants_quit() {
            sh.quit.store(true, Ordering::Release);
        }
        if *app.config() != cfg {
            cfg = app.config().clone();
            *lock(&sh.config) = cfg.clone();
        }
        scheduled = next_deadline(scheduled, Instant::now(), period);
    }
    cfg
}

/// Game thread entry: builds the App on this thread and runs the loop; panics are caught
/// and reported through `Shared::failure`.
fn game_main<A: GameApp>(
    sh: Arc<Shared>,
    make_app: impl FnOnce() -> A,
    ev_rx: Receiver<InputEvent>,
    host_tx: Sender<HostEvent>,
    music_tx: Sender<MusicCmd>,
    period: Duration,
) -> Option<Config> {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let mut app = make_app();
        game_loop(&sh, &mut app, &ev_rx, &host_tx, &music_tx, period)
    }));
    match r {
        Ok(cfg) => Some(cfg),
        Err(p) => {
            *lock(&sh.failure) = Some(format!("game thread: {}", panic_message(p.as_ref())));
            sh.quit.store(true, Ordering::Release);
            None
        }
    }
}

/// Music thread body: keeps `MUSIC_READY_FRAMES` rendered, handles Play/Stop.
fn music_loop<S: PcmSource>(sh: &Shared, rx: &Receiver<MusicCmd>, make: &mut dyn FnMut(MusicTrack) -> S) {
    let mut synth: Option<S> = None;
    let mut chunk = vec![[0.0f32; 2]; MUSIC_CHUNK];
    let mut handle = |cmd: MusicCmd, synth: &mut Option<S>| {
        {
            let mut ring = lock(&sh.music);
            ring.clear();
            sh.music_epoch.fetch_add(1, Ordering::AcqRel);
        }
        *synth = match cmd {
            MusicCmd::Play(t) => Some(make(t)),
            MusicCmd::Stop => None,
        };
    };
    while !sh.stop.load(Ordering::Acquire) {
        while let Ok(cmd) = rx.try_recv() {
            handle(cmd, &mut synth);
        }
        let need = match &synth {
            Some(_) => lock(&sh.music).len() < MUSIC_READY_FRAMES,
            None => false,
        };
        if need {
            if let Some(s) = synth.as_mut() {
                s.fill(&mut chunk);
            }
            lock(&sh.music).extend(chunk.iter().copied());
            continue;
        }
        match rx.recv_timeout(MUSIC_IDLE) {
            Ok(cmd) => handle(cmd, &mut synth),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // The game thread is gone: keep serving what is playing until stopped.
                thread::sleep(MUSIC_IDLE);
            }
        }
    }
}

pub struct Runtime {
    shared: Arc<Shared>,
    ev_tx: Sender<InputEvent>,
    host_rx: Receiver<HostEvent>,
    game: Option<JoinHandle<Option<Config>>>,
    music: Option<JoinHandle<()>>,
    sfx: Option<JoinHandle<()>>,
    storage: Option<Storage>,
}

impl Runtime {
    pub fn start(loaded: &Loaded, view: (f32, f32), seed: u32) -> Runtime {
        let shared = Arc::new(Shared::new(loaded.config.clone(), view));
        let (ev_tx, ev_rx) = mpsc::channel::<InputEvent>();
        let (host_tx, host_rx) = mpsc::channel::<HostEvent>();
        let (music_tx, music_rx) = mpsc::channel::<MusicCmd>();

        let init = AppInit {
            assets: loaded.assets.clone(),
            ctx: RenderCtx { atlas: loaded.atlas.clone(), strings: loaded.strings.clone(), glyphs: loaded.glyphs.clone() },
            storage: Storage::new(loaded.userdir.clone()),
            config: loaded.config.clone(),
            view,
            seed,
        };
        let sh = shared.clone();
        let game = thread::Builder::new()
            .name("asylum-game".into())
            .spawn(move || game_main(sh, move || App::new(init), ev_rx, host_tx, music_tx, TICK))
            .expect("spawn game thread");

        let sh = shared.clone();
        let assets = loaded.assets.clone();
        let music = thread::Builder::new()
            .name("asylum-music".into())
            .spawn(move || {
                let voices = Arc::new(assets.voices.clone());
                let mut make = |t: MusicTrack| MusicSynth::new(voices.clone(), music_tune(&assets, t).clone(), music_tempo(t));
                let r = catch_unwind(AssertUnwindSafe(|| music_loop(&sh, &music_rx, &mut make)));
                if let Err(p) = r {
                    lock(&sh.music).clear();
                    eprintln!("asylum: music thread stopped: {}", panic_message(p.as_ref()));
                }
            })
            .expect("spawn music thread");

        let sh = shared.clone();
        let assets = loaded.assets.clone();
        let sfx = thread::Builder::new()
            .name("asylum-sfx".into())
            .spawn(move || {
                match catch_unwind(AssertUnwindSafe(|| crate::audio::sfx::synth_all(&assets.voices, 0))) {
                    Ok(bank) => *lock(&sh.sfx_bank) = Some(bank),
                    Err(p) => eprintln!("asylum: sound effects unavailable: {}", panic_message(p.as_ref())),
                }
                // Never keep the game on the loading screen, even without effects.
                sh.sfx_ready.store(true, Ordering::Release);
            })
            .expect("spawn sfx thread");

        Runtime {
            shared,
            ev_tx,
            host_rx,
            game: Some(game),
            music: Some(music),
            sfx: Some(sfx),
            storage: Some(Storage::new(loaded.userdir.clone())),
        }
    }
    pub fn send(&self, ev: InputEvent) {
        let _ = self.ev_tx.send(ev);
    }
    pub fn set_held(&self, mask: u32) {
        self.shared.held.store(mask, Ordering::Release);
    }
    pub fn set_view(&self, w: f32, h: f32) {
        self.shared.view.store(pack_view(w, h), Ordering::Release);
    }
    /// Latest published frame (cheap Arc clone).
    pub fn latest_frame(&self) -> Option<PublishedFrame> {
        lock(&self.shared.frame).clone()
    }
    pub fn poll_events(&self, out: &mut Vec<HostEvent>) {
        while let Ok(e) = self.host_rx.try_recv() {
            out.push(e);
        }
    }
    /// Pop up to `max` frames of music PCM into `out` (appends), only while the music is
    /// still at `epoch` ([`Runtime::music_epoch`]): the ring is flushed and the epoch bumped
    /// under the same lock, so a track change between reading the epoch and pulling cannot
    /// slip the new track's first frames into the old track's queue. False on a mismatch
    /// (nothing pulled; read the epoch again).
    pub fn pull_music(&self, epoch: u32, out: &mut Vec<[f32; 2]>, max: usize) -> bool {
        let mut ring = lock(&self.shared.music);
        if self.shared.music_epoch.load(Ordering::Acquire) != epoch {
            return false;
        }
        let n = max.min(ring.len());
        out.extend(ring.drain(..n));
        true
    }
    /// Incremented whenever the music restarts or stops (the ring was flushed): the host
    /// should then discard PCM it has already queued so the change is heard at once.
    pub fn music_epoch(&self) -> u32 {
        self.shared.music_epoch.load(Ordering::Acquire)
    }
    /// The synthesized SFX bank, once (index = SfxId).
    pub fn take_sfx_bank(&self) -> Option<Vec<Pcm>> {
        lock(&self.shared.sfx_bank).take()
    }
    /// Snapshot of the current config (for key bindings).
    pub fn config(&self) -> Config {
        lock(&self.shared.config).clone()
    }
    /// True after the App asked to quit (or the game thread failed, see [`Runtime::failure`]).
    pub fn quit_requested(&self) -> bool {
        self.shared.quit.load(Ordering::Acquire)
    }
    /// Message of a fatal game-thread panic, if any.
    pub fn failure(&self) -> Option<String> {
        lock(&self.shared.failure).clone()
    }
    /// Stop and join all threads; persists config.
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let final_cfg = self.game.take().and_then(|h| h.join().ok().flatten());
        if let Some(h) = self.music.take() {
            let _ = h.join();
        }
        if let Some(h) = self.sfx.take() {
            // synth_all cannot be interrupted; never block quitting on it.
            if h.is_finished() {
                let _ = h.join();
            }
        }
        if let Some(storage) = self.storage.take() {
            let cfg = final_cfg.unwrap_or_else(|| lock(&self.shared.config).clone());
            if let Err(e) = storage.save_config(&cfg) {
                eprintln!("asylum: could not save {}: {e}", storage.config_path().display());
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::sfx::SfxId;

    /// What the mock app observed.
    #[derive(Default)]
    struct Rec {
        ticks: u64,
        events: Vec<InputEvent>,
        views: Vec<(f32, f32)>,
        held_last: u32,
        sfx_ready_at: Option<u64>,
    }

    /// Scriptable stand-in for `App`.
    #[derive(Default)]
    struct MockApp {
        cfg: Config,
        /// tick → outputs to emit at that tick
        script: Vec<(u64, AppOutputs)>,
        pending: AppOutputs,
        rec: Arc<Mutex<Rec>>,
    }

    impl GameApp for MockApp {
        fn push_event(&mut self, ev: InputEvent) {
            self.rec.lock().unwrap().events.push(ev);
        }
        fn set_view(&mut self, w: f32, h: f32) {
            self.rec.lock().unwrap().views.push((w, h));
        }
        fn set_sfx_ready(&mut self) {
            let mut r = self.rec.lock().unwrap();
            r.sfx_ready_at = Some(r.ticks);
        }
        fn tick(&mut self, held: u32) {
            let ticks = {
                let mut r = self.rec.lock().unwrap();
                r.ticks += 1;
                r.held_last = held;
                r.ticks
            };
            if let Some(i) = self.script.iter().position(|(t, _)| *t == ticks) {
                let (_, o) = self.script.remove(i);
                self.pending.sfx.extend(o.sfx);
                if o.music.is_some() {
                    self.pending.music = o.music;
                }
                self.pending.requests.extend(o.requests);
            }
            if ticks == 3 {
                self.cfg.sfx_volume = 3;
            }
        }
        fn render(&mut self, out: &mut RenderFrame) {
            out.seq = self.rec.lock().unwrap().ticks;
            out.quads.clear();
            out.quads.resize(4, Default::default());
        }
        fn drain(&mut self, out: &mut AppOutputs) {
            out.sfx.append(&mut self.pending.sfx);
            if self.pending.music.is_some() {
                out.music = self.pending.music.take();
            }
            out.requests.append(&mut self.pending.requests);
        }
        fn config(&self) -> &Config {
            &self.cfg
        }
        fn wants_quit(&self) -> bool {
            false
        }
    }

    struct Harness {
        sh: Arc<Shared>,
        ev_tx: Sender<InputEvent>,
        host_rx: Receiver<HostEvent>,
        music_rx: Receiver<MusicCmd>,
        handle: JoinHandle<Option<Config>>,
    }

    fn spawn_mock(app: MockApp, period: Duration) -> Harness {
        let sh = Arc::new(Shared::new(Config::default(), (320.0, 256.0)));
        let (ev_tx, ev_rx) = mpsc::channel();
        let (host_tx, host_rx) = mpsc::channel();
        let (music_tx, music_rx) = mpsc::channel();
        let sh2 = sh.clone();
        let handle = thread::spawn(move || game_main(sh2, move || app, ev_rx, host_tx, music_tx, period));
        Harness { sh, ev_tx, host_rx, music_rx, handle }
    }

    fn wait_for(cond: impl Fn() -> bool, max: Duration) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < max {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        cond()
    }

    #[test]
    fn deadline_schedule_and_resync() {
        let t0 = Instant::now();
        let p = Duration::from_millis(20);
        // on time / slightly late: fixed schedule
        assert_eq!(next_deadline(t0, t0, p), t0 + p);
        assert_eq!(next_deadline(t0, t0 + p * 3, p), t0 + p);
        assert_eq!(next_deadline(t0, t0 + p * 6, p), t0 + p);
        // more than 5 ticks behind: restart at now
        let now = t0 + p * 7;
        assert_eq!(next_deadline(t0, now, p), now);
    }

    #[test]
    fn sleep_until_is_accurate() {
        for ms in [0u64, 3, 12] {
            let t0 = Instant::now();
            let d = t0 + Duration::from_millis(ms);
            sleep_until(d);
            let e = Instant::now();
            assert!(e >= d);
            assert!(e - d < Duration::from_millis(8), "overslept {:?}", e - d);
        }
    }

    #[test]
    fn view_packing_round_trips() {
        for v in [(320.0f32, 256.0f32), (455.25, 256.0), (0.0, -1.5)] {
            assert_eq!(unpack_view(pack_view(v.0, v.1)), v);
        }
    }

    #[test]
    fn frame_pool_reuses_unique_buffers() {
        let mut pool = vec![Arc::new(RenderFrame { seq: 7, ..Default::default() })];
        let held = pool[0].clone();
        // shared → a new buffer is allocated, the pool keeps the shared one
        let fresh = take_spare(&mut pool);
        assert_eq!(fresh.seq, 0);
        assert_eq!(pool.len(), 1);
        drop(held);
        let reused = take_spare(&mut pool);
        assert_eq!(reused.seq, 7);
        assert!(pool.is_empty());
    }

    #[test]
    fn game_loop_runs_at_fixed_rate_and_forwards_everything() {
        let rec = Arc::new(Mutex::new(Rec::default()));
        let sfx = SfxEvent { channel: 2, id: SfxId::JUMP, volume: 100, stereo: -20 };
        let app = MockApp {
            rec: rec.clone(),
            script: vec![
                (2, AppOutputs { sfx: vec![sfx], music: Some(Some(MusicTrack::Title)), requests: vec![HostRequest::StopSfx] }),
                (4, AppOutputs { music: Some(None), ..Default::default() }),
                (6, AppOutputs { requests: vec![HostRequest::Quit], ..Default::default() }),
            ],
            ..Default::default()
        };
        let h = spawn_mock(app, TICK);
        h.sh.held.store(0b10101, Ordering::Release);
        h.ev_tx.send(InputEvent::Pad { button: 1 }).unwrap();
        h.ev_tx.send(InputEvent::MouseMove { x: 1.0, y: 2.0 }).unwrap();
        h.sh.view.store(pack_view(455.0, 256.0), Ordering::Release);
        h.sh.sfx_ready.store(true, Ordering::Release);

        // published frames: consecutive scheduled tick times are exactly one period apart
        assert!(wait_for(|| lock(&h.sh.frame).is_some(), Duration::from_secs(2)));
        let mut last: Option<PublishedFrame> = None;
        let mut gaps = 0;
        let t_start = Instant::now();
        while gaps < 5 && t_start.elapsed() < Duration::from_secs(3) {
            let f = lock(&h.sh.frame).clone().unwrap();
            if let Some(l) = &last {
                if f.frame.seq == l.frame.seq + 1 {
                    assert_eq!(f.tick_time - l.tick_time, TICK);
                    gaps += 1;
                }
            }
            if last.as_ref().is_none_or(|l| l.frame.seq != f.frame.seq) {
                assert_eq!(f.frame.quads.len(), 4);
                last = Some(f);
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(gaps, 5, "saw consecutive frames");
        drop(last);

        // rate: ~50 ticks per second
        let s0 = lock(&h.sh.frame).as_ref().unwrap().frame.seq;
        let t0 = Instant::now();
        thread::sleep(Duration::from_millis(500));
        let s1 = lock(&h.sh.frame).as_ref().unwrap().frame.seq;
        let dt = t0.elapsed().as_secs_f64();
        let rate = (s1 - s0) as f64 / dt;
        assert!((35.0..=60.0).contains(&rate), "tick rate {rate}");

        assert!(h.sh.quit.load(Ordering::Acquire), "Quit request sets quit flag");
        assert_eq!(lock(&h.sh.config).sfx_volume, 3, "config snapshot follows the app");

        let mut events = Vec::new();
        while let Ok(e) = h.host_rx.try_recv() {
            events.push(format!("{e:?}"));
        }
        assert_eq!(events.len(), 3, "{events:?}");
        assert!(events[0].contains("Sfx") && events[0].contains("stereo: -20"));
        assert!(events[1].contains("StopSfx"));
        assert!(events[2].contains("Quit"));
        let cmds: Vec<_> = h.music_rx.try_iter().collect();
        assert!(matches!(cmds.as_slice(), [MusicCmd::Play(MusicTrack::Title), MusicCmd::Stop]));

        h.sh.stop.store(true, Ordering::Release);
        let cfg = h.handle.join().unwrap().expect("no panic");
        assert_eq!(cfg.sfx_volume, 3);
        let r = rec.lock().unwrap();
        assert_eq!(r.events, vec![InputEvent::Pad { button: 1 }, InputEvent::MouseMove { x: 1.0, y: 2.0 }]);
        assert_eq!(r.views, vec![(455.0, 256.0)]);
        assert!(r.sfx_ready_at.is_some());
        assert_eq!(r.held_last, 0b10101);
    }

    #[test]
    fn game_thread_panic_is_reported() {
        struct Boom;
        impl GameApp for Boom {
            fn push_event(&mut self, _: InputEvent) {}
            fn set_view(&mut self, _: f32, _: f32) {}
            fn set_sfx_ready(&mut self) {}
            fn tick(&mut self, _: u32) {
                panic!("boom at tick");
            }
            fn render(&mut self, _: &mut RenderFrame) {}
            fn drain(&mut self, _: &mut AppOutputs) {}
            fn config(&self) -> &Config {
                unreachable!()
            }
            fn wants_quit(&self) -> bool {
                false
            }
        }
        let sh = Arc::new(Shared::new(Config::default(), (320.0, 256.0)));
        let (_ev_tx, ev_rx) = mpsc::channel();
        let (host_tx, _host_rx) = mpsc::channel();
        let (music_tx, _music_rx) = mpsc::channel();
        let r = game_main(sh.clone(), || Boom, ev_rx, host_tx, music_tx, TICK);
        assert!(r.is_none());
        assert!(sh.quit.load(Ordering::Acquire));
        let msg = lock(&sh.failure).clone().unwrap();
        assert!(msg.contains("game thread"), "{msg}");
    }

    struct Ramp {
        v: f32,
        calls: Arc<AtomicU32>,
    }
    impl PcmSource for Ramp {
        fn fill(&mut self, out: &mut [[f32; 2]]) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            for f in out.iter_mut() {
                *f = [self.v, -self.v];
            }
        }
    }

    #[test]
    fn music_thread_keeps_the_ring_filled_and_flushes_on_commands() {
        let sh = Arc::new(Shared::new(Config::default(), (320.0, 256.0)));
        let (tx, rx) = mpsc::channel();
        let calls = Arc::new(AtomicU32::new(0));
        let sh2 = sh.clone();
        let calls2 = calls.clone();
        let h = thread::spawn(move || {
            let mut make = |t: MusicTrack| Ramp { v: if t == MusicTrack::Title { 0.25 } else { 0.5 }, calls: calls2.clone() };
            music_loop(&sh2, &rx, &mut make)
        });
        // nothing playing: nothing rendered
        thread::sleep(Duration::from_millis(20));
        assert_eq!(lock(&sh.music).len(), 0);
        let e0 = sh.music_epoch.load(Ordering::Acquire);

        tx.send(MusicCmd::Play(MusicTrack::Title)).unwrap();
        assert!(wait_for(|| lock(&sh.music).len() >= MUSIC_READY_FRAMES, Duration::from_secs(2)));
        let len = lock(&sh.music).len();
        assert!(len < MUSIC_READY_FRAMES + MUSIC_CHUNK, "keeps ~0.25 s, not more ({len})");
        assert_eq!(len % MUSIC_CHUNK, 0);
        assert_eq!(sh.music_epoch.load(Ordering::Acquire), e0 + 1);
        assert_eq!(lock(&sh.music).front(), Some(&[0.25, -0.25]));

        // consuming re-triggers rendering
        let before = calls.load(Ordering::Relaxed);
        lock(&sh.music).drain(..4000);
        assert!(wait_for(|| lock(&sh.music).len() >= MUSIC_READY_FRAMES, Duration::from_secs(2)));
        assert!(calls.load(Ordering::Relaxed) > before);

        // a new track flushes the old PCM
        tx.send(MusicCmd::Play(MusicTrack::GameOver)).unwrap();
        assert!(wait_for(|| sh.music_epoch.load(Ordering::Acquire) == e0 + 2, Duration::from_secs(2)));
        assert!(wait_for(|| lock(&sh.music).len() >= MUSIC_READY_FRAMES, Duration::from_secs(2)));
        assert!(lock(&sh.music).iter().all(|f| *f == [0.5, -0.5]));

        tx.send(MusicCmd::Stop).unwrap();
        assert!(wait_for(|| sh.music_epoch.load(Ordering::Acquire) == e0 + 3, Duration::from_secs(2)));
        thread::sleep(Duration::from_millis(20));
        assert_eq!(lock(&sh.music).len(), 0);

        sh.stop.store(true, Ordering::Release);
        h.join().unwrap();
    }

    #[test]
    fn music_track_selection_matches_the_c_slots() {
        let a = crate::assets::testdata::assets();
        assert!(std::ptr::eq(music_tune(&a, MusicTrack::Title), &a.title_music));
        assert!(std::ptr::eq(music_tune(&a, MusicTrack::GameOver), &a.death_music));
        assert!(std::ptr::eq(music_tune(&a, MusicTrack::Zone { zone: 1, part: 1 }), &a.zones[1].music1));
        assert!(std::ptr::eq(music_tune(&a, MusicTrack::Zone { zone: 2, part: 2 }), &a.zones[2].music2));
        assert_eq!(music_tempo(MusicTrack::GameOver), 0x980);
        assert_eq!(music_tempo(MusicTrack::Title), 0x1000);
        assert_eq!(music_tempo(MusicTrack::Zone { zone: 0, part: 1 }), 0x1000);
    }

    #[test]
    fn os_locale_picks_the_language() {
        assert_eq!(locale_language("ko_KR"), Lang::Ko);
        assert_eq!(locale_language("zh_CN"), Lang::ZhCn);
        assert_eq!(locale_language("zh"), Lang::ZhCn);
        assert_eq!(locale_language("en_GB"), Lang::En);
        assert_eq!(locale_language("de_DE"), Lang::En);
        assert_eq!(locale_language(""), Lang::En);
    }

    /// Whole start-up path with the real data and the real App (as the Godot host runs it).
    #[test]
    fn real_load_start_tick_and_shutdown() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
        let dir = std::env::temp_dir().join(format!("asylum-rt-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let input = LoadInput {
            files: RawFiles::from_dir(&root.join("game/data")).unwrap(),
            strings_csv: read("game/locale/strings.csv"),
            bdf_ko: read("game/fonts/asylum-ko.bdf"),
            bdf_zh: read("game/fonts/asylum-zh_hans.bdf"),
            userdir: dir.clone(),
            os_locale: "zh_CN".into(),
        };
        let loaded = load(input).expect("load");
        assert_eq!(loaded.config.language, Some(Lang::ZhCn), "OS locale applied to a fresh config");
        assert!(!loaded.glyphs.glyphs.is_empty());

        let rt = Runtime::start(&loaded, (455.0, 256.0), 7);
        assert!(wait_for(|| rt.latest_frame().is_some_and(|f| !f.frame.quads.is_empty()), Duration::from_secs(10)));
        let t0 = rt.latest_frame().unwrap().tick_time;
        rt.set_view(600.0, 256.0);
        rt.send(InputEvent::Key { code: crate::input::key::SPACE, ch: Some(' '), echo: false });
        thread::sleep(Duration::from_millis(300));
        let f = rt.latest_frame().unwrap();
        let ticks = (f.tick_time - t0).as_millis() / 20;
        assert!(ticks >= 8, "game thread keeps ticking ({ticks} ticks in 300 ms)");
        assert!(rt.failure().is_none(), "{:?}", rt.failure());
        let mut evs = Vec::new();
        rt.poll_events(&mut evs);
        assert!(
            evs.iter().any(|e| matches!(e, HostEvent::Request(HostRequest::ApplyVideo(_)))),
            "start-up ApplyVideo request: {evs:?}"
        );
        assert!(!rt.quit_requested());
        rt.shutdown();
        let saved = std::fs::read_to_string(dir.join("asylum.cfg")).expect("config saved at shutdown");
        assert!(!saved.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_host_side_api() {
        // Exercise the host-facing methods without spawning the real App.
        let sh = Arc::new(Shared::new(Config::default(), (320.0, 256.0)));
        let (ev_tx, ev_rx) = mpsc::channel();
        let (host_tx, host_rx) = mpsc::channel();
        let rt = Runtime {
            shared: sh.clone(),
            ev_tx,
            host_rx,
            game: None,
            music: None,
            sfx: None,
            // Saving needs Config::serialize (frontend); covered by the e2e test instead.
            storage: None,
        };
        rt.send(InputEvent::CloseRequest);
        assert_eq!(ev_rx.try_recv().unwrap(), InputEvent::CloseRequest);
        rt.set_held(5);
        assert_eq!(sh.held.load(Ordering::Acquire), 5);
        rt.set_view(500.0, 256.0);
        assert_eq!(unpack_view(sh.view.load(Ordering::Acquire)), (500.0, 256.0));
        assert!(rt.latest_frame().is_none());
        host_tx.send(HostEvent::Request(HostRequest::StopSfx)).unwrap();
        let mut evs = Vec::new();
        rt.poll_events(&mut evs);
        assert_eq!(evs.len(), 1);
        lock(&sh.music).extend([[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]]);
        let mut pcm = vec![[9.0, 9.0]];
        let e = rt.music_epoch();
        assert!(rt.pull_music(e, &mut pcm, 2));
        assert_eq!(pcm, vec![[9.0, 9.0], [0.1, 0.2], [0.3, 0.4]]);
        // a track change since the epoch was read: nothing is pulled
        sh.music_epoch.fetch_add(1, Ordering::AcqRel);
        assert!(!rt.pull_music(e, &mut pcm, 10));
        assert_eq!(pcm.len(), 3);
        assert!(rt.pull_music(e + 1, &mut pcm, 10));
        assert_eq!(pcm.len(), 4);
        *lock(&sh.sfx_bank) = Some(vec![Pcm::default(); 2]);
        assert_eq!(rt.take_sfx_bank().map(|b| b.len()), Some(2));
        assert!(rt.take_sfx_bank().is_none());
        assert!(!rt.quit_requested());
        assert!(rt.failure().is_none());
        lock(&sh.config).music_volume = 4;
        assert_eq!(rt.config().music_volume, 4);
        rt.shutdown();
        assert!(sh.stop.load(Ordering::Acquire));
    }
}
