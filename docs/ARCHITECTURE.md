# Architecture of Asylum for Windows (Godot port)

This document describes the design of the port and the rules every change follows.

The port is a function-by-function Rust translation of **SDL Asylum** (Hugh Robinson's C
port, in M-HT's SDL2 fork `M-HT/asylum`; GPL-3.0-or-later), running on **Godot 4.7.2**
through **gdext 0.5.5**. It uses the original game data unchanged (Andy Southgate, 1994;
public domain), shows English, Korean and Simplified Chinese text, and supports modern displays
(any window size and aspect ratio, HiDPI, high refresh rates).

Reference C source: `reference/sdl-asylum/*.c` (upstream commit in `UPSTREAM_COMMIT.txt`).
Original assets: `game/data/**` (byte-identical copies; never modify them).

---------------------------------------------------------------------------------------------

## 1. Repository layout

```
game/                      Godot project (project.godot, scenes, shaders, fonts, locale, data/)
game/data/                 ORIGINAL asset files (Resources/, Ego/, Psyche/, Id/, Voices/)
game/locale/strings.csv    All UI strings: columns  key,en,ko,zh_CN  (imported with "keep")
game/fonts/                CJK pixel font subsets + their OFL licenses
game/bin/                  Built GDExtension DLLs (copied by tools/build.ps1, not in git)
rust/asylum-core/          Engine-independent core (NO godot dependency, fully unit-testable)
rust/asylum-gdext/         Godot binding (cdylib) — thin host layer
reference/sdl-asylum/      Upstream C source (read-only reference)
third_party/               Fusion Pixel Font release the font subsets are generated from
tools/                     build / export / font-subset scripts, Godot patches
licenses/                  third-party license texts (shipped with the release)
tools/godot-template/      lean export template built by tools/build_template.ps1 (not in git)
docs/                      This file, PORTING_NOTES.md
dist/                      Portable build output and release zip (not in git)
build/                     Godot source checkout of tools/build_template.ps1 (not in git)
```

## 2. Thread model

```
 Godot main thread (gdext AsylumGame node)            Game thread (asylum-core runtime)
 ------------------------------------------            ----------------------------------
 gather input  --- Command channel (events) ----->     fixed 50 Hz loop (sleep_until)
 held-key mask --- AtomicU32 ------------------->       App::tick()  (menus + Sim)
                                                       build RenderFrame (quads, texts)
 take latest Arc<RenderFrame>  <-- Mutex slot ----      publish frame
 finalize(alpha): interpolate+clip+pack floats         push HostEvent (sfx, music, requests)
 upload MultiMesh buffer (1 draw call)          <--   (mpsc channel)
 play SFX on 8 AudioStreamPlayers (+pan buses)
 push music PCM to AudioStreamGenerator  <--- ring --- Music thread: MusicSynth (48 kHz)
                                                       commands from game thread (mpsc)
 Loader (startup): std::thread::scope workers decode assets / build atlas / synth SFX
 Godot renderer: separate render thread model (project setting), Compatibility renderer.
 Godot WorkerThreadPool capped at 4 (`threading/worker_pool/max_threads`): the game does its
 own threading, so one idle engine worker per logical core is pure overhead.
```

Rules:
* Rust worker threads never touch Godot objects. Only the gdext main thread does.
* The core `App` itself is single-threaded deterministic logic; threading lives in
  `asylum_core::runtime` only. Everything else is plain data + functions (unit-testable).
* asylum-core contains no `unsafe` code.

## 3. Timing, interpolation, frame semantics

* Logic runs at exactly 50 Hz (`TICK_HZ = 50`, 20 ms) like the original (`gearchange=1`,
  `frameinc = 1`, `options.explospeed = 1`). The 25 Hz "rate50" path is not ported.
* After every tick the game thread builds a `RenderFrame` describing tick N, where every quad
  and text run carries its screen position at tick N **and** the screen-space delta to where
  the same thing was at tick N-1 (`dx, dy`). The main thread draws
  `pos = cur + (1 - alpha) * delta`, `alpha = clamp((now - frame.tick_time)/20ms, 0, 1)`
  (one tick of latency, standard fixed-step interpolation). With *Smooth Motion* off,
  alpha = 1. Final positions are not rounded (`RenderFrame::snap` exists only for the
  rasterizing render tests).
* Deltas larger than `SNAP_PX = 48` px are zeroed (teleports, respawn, slot reuse). Text
  deltas are in the whole pixels a text is drawn at (`x >> 8`), so texts at fractional
  speeds (the 0.5 px/tick potion messages) never jump back at a tick boundary.
* Clipping is applied **after** interpolation (clip rects are fixed per frame).
* Instances carry the atlas rectangle in **texels** (`INSTANCE_CUSTOM`; the shader multiplies
  by `TEXTURE_PIXEL_SIZE`). The Compatibility renderer packs colour and custom data into
  truncated 16-bit floats, which hold every whole texel up to 2048 exactly; the atlas is
  therefore capped at 1024 x 2048 (the shipped fonts need about 1000 rows).

## 4. Coordinates & layout (modern monitor support)

* Video settings (`asylum_core::display`, *Options → 2. Tune Game Options → 3. Video System*):
  * Display mode: *Windowed* → *Borderless* (covers the monitor; Godot `FULLSCREEN`) →
    *Full Screen* (exclusive; Godot `EXCLUSIVE_FULLSCREEN`), cycling; F11 / Alt+Enter switch
    between *Windowed* and the last full-screen mode.
  * Window aspect 4:3 / 16:10 / 16:9 / 21:9 and window resolution: every standard resolution
    of the aspect (`display::RESOLUTIONS`) that is not wider or taller than the monitor; the
    window's client area is exactly that size (no DPI scaling: resolutions are pixels). In the
    full-screen modes the aspect and resolution are the monitor's own (rows inactive).
  * View scale (*Scale* in the menu, `ScreenScale` in `asylum.cfg`) 1.0x..3.0x (0.5 steps): the game draws `256 x scale` lines with the output's
    shape, so a higher scale shows more of the game world at smaller pixels; only scales whose
    line count does not exceed the resolution's height are offered.
* The host sets `content_scale_size` to that view size with `stretch/mode = canvas_items`,
  `stretch/aspect = expand`, fractional scale: the logical viewport always has the window's
  own shape (also when the player resizes or maximizes the window), so the picture fills the
  window or screen exactly — no black bars, and the core lays the screen out for the size
  instead of stretching a fixed picture. The host re-applies the settings when the window
  moves to another monitor and reports the monitor size to the core
  (`InputEvent::Monitor`); core and host compute the result with the same
  `display::video`.
* Host details (`asylum-gdext/src/video.rs`): window mode has a minimum client size of
  320x256 and a degenerate (0-px) window is never passed to the core; a window taller than
  the work area keeps its bottom (the HUD) on screen; a move to another monitor is applied
  once the window has stopped moving (not while minimized); a desktop resolution change
  re-enters the full-screen mode; the frame laid out for the previous view is scaled to the
  new view for the tick until the core catches up (no black area); the window mode and size
  are written to `override.cfg` beside the executable (portable install only), which Godot
  reads before it creates the window, so the game opens directly in the saved mode and size.
* Engine patches (`tools/godot-patches`, lean template): Godot's borderless full screen makes
  the window 2 px larger than the monitor; with the native WGL driver it now grows to the
  right instead of downwards (downwards shifted the bottom-left-anchored GL blit, leaving the
  top 2 rows unpainted). The process is per-monitor DPI aware (V1), so sizes are physical
  pixels on every monitor and Windows never bitmap-stretches the window.
* The host never overrides `on_notification`: Godot delivers some notifications (window
  focus) synchronously while `set_mode` runs, i.e. while the node is mutably borrowed; the
  close button uses the window's `close_requested` signal and teardown uses `exit_tree`.
* `Layout` (asylum_core::layout): the content is the whole viewport (`cw x ch`, at least
  320x256). Game window `gamex=16, gamey=8, gamew=cw-32, gameh=ch-64`; the original 320x256
  band is centred (`hx=(cw-320)/2`, `cy=(ch-256)/2`: menus, chat screens, pause screen) and
  the HUD panel is bottom-anchored (`hy=ch-256`: rows 208..256 at 208+hy, bonus messages,
  the intro scroll at the original speed, so its bottom 256 lines match the original). For 320x256 everything is
  pixel-identical to the original. All draw coordinates in a RenderFrame are **viewport**
  coordinates (content origin `ox,oy` already added).
* GameScreen composition for larger content (see render/frame.rs): frame rows 0..208:
  src x[0,64)->0, src x[256,320)->cw-64, src x[64,256)->hx+64, gaps filled by repeating src
  column 100; src rows [0,150)->0, [150,208)->150+hy, the gap repeats src row 150. HUD panel
  rows 208..256 at 208+hy: src x[8,312)->hx+8, src x[0,8)->0, src x[312,320)->cw-8, gaps
  filled with mirrored tiling of src x[105,212) rows 208..235 (rows 236..255 use row
  235-(y-236)), tile period 214, phase anchored at hx+8 (src column 104 has a light vertical
  crack and is excluded).
* ChatScreen composition: src x[0,160)->0, src x[160,320)->cw-160, gap filled by mirrored
  tiling of src x[80,160) (period 160, anchored at x=160); rows likewise src y[0,128)->0,
  [128,256)->ch-128, the gap mirrored tiling of src y[64,128) anchored at y=128.
* Larger-view gameplay adaptations (only when the window is larger than 288x192): alien plot
  range (`sprlx..sprhy`, rounded up to whole blocks), the wake-up box (`wakeupal`) and the
  sleep range (`al_sleepcheck`) grow with the visible game window, so aliens on screen stay
  awake; the midground parallax anchor uses the window's own block counts; messages that used
  fixed 320x256 positions are offset by `hx`/`cy` (or `hy`), game-window messages are placed
  from the window size without the 1000/2000 lifetime encoding and live long enough to leave
  a larger window; every text records its anchor (band, bottom, game window) so a layout
  change moves it along (`TextTable::relayout`); the pause screen slides in from outside the
  larger screen in the original 64 ticks. Sleeping-alien
  marker cells 240..=253 are never drawn by the renderer (Psyche has a sprite at 240 that the
  original never shows on screen). Board cells outside the maze are not drawn (only the
  backdrop shows there), which matters for the small 32x32-block neurons at large scales.

## 5. Original semantics that MUST be preserved (porting rules)

* Compile flags of the original: `-funsigned-char` ⇒ **every C `char` is `u8`**.
  `int` is `i32`. `int16_t` is `i16`. Use wrapping arithmetic semantics (the workspace
  disables overflow checks in all profiles; still prefer `wrapping_*` where intent matters).
  C `/` and `%` truncate toward zero like Rust; `>>` on negative `i32` is arithmetic in both.
  `abs()` → `i32::wrapping_abs`. Casting `double`→`int` truncates toward zero (`as i32`).
* `random()` is glibc's TYPE_3 additive generator (see `rng.rs`); returns 0..=2^31-1.
* Pointers into the board become `i32` cell offsets (`BIdx`) relative to the current board's
  contents (may be negative / out of range — `Board::get/set` handle that safely:
  [-32,0) reads/writes the 32-byte header, beyond that 0 / ignored).
* `alent*` pointers become `usize` indices into `Sim::al`; the player uses `Sim::fraudalent`.
  Functions that took `alent*` for "the object being collision-checked" take `&mut Alent`
  (copy-in/copy-out by the caller) — they never alias the platform objects they touch.
* Draw calls (`relplot`, `blokeplot`, `fspplot`, `mazescaleplot`, `plotdying`, ...) become
  `SprCmd` pushes (see `sim/mod.rs`) with an interpolation key. Board blocks and the backdrop
  are NOT emitted by the sim; the renderer draws them from the board at frame-build time
  (`draw_block` logic incl. animation lives in `render/world.rs`; its use of `random()` is
  replaced by a deterministic hash of (cell, framectr) so rendering never perturbs the game RNG).
* Sounds (`bidforsound*`) become `SfxEvent`s; channel-busy semantics of SDL_mixer
  (`Mix_Playing`) are emulated with per-channel busy-until tick counters
  (chunk length = `frames * 2` ticks, since make_sound renders frames*882 samples @22050 Hz).
  Stereo position: Archimedes convention −127 (left) .. 0 (centre) .. +127 (right)
  (fixes the SDL port bug where 0 was hard-left). Volume = initvol/128 × master.
* Known C-port bugs and how we treat them are listed in `docs/PORTING_NOTES.md`; when you find
  a new one, fix it according to the most plausible *original* intent and add a note there.

## 6. Module map (asylum-core)

| module | owns | C origin |
|---|---|---|
| `rng` | glibc random() | libc |
| `palette` | VIDC 256-colour palette | vdu.c init_palette |
| `assets::*` | Hmr decompression, FSP1 sprites, RLE screens, boards, backfiles, Sta1 voices, Bodge tunes, `GameAssets::load` (parallel) | file.c, vdu.c, sound.c |
| `atlas` | single RGBA8 atlas of every sprite/screen/backdrop/strength texture | vdu.c |
| `audio::sfx` | SFX catalogue (`SfxId`) + HQ synthesis @48 kHz (parallel) | sound.c make_sound, init_chunk_* |
| `audio::music` | streaming Bodge tracker synth @48 kHz | sound.c sdl_music_hook |
| `audio::kernel` | windowed-sinc interpolation tables | sound.c |
| `sim::*` | the game (state + per-tick logic) | asylum.c, player.c, alien.c, bullet.c, projectile.c, maze.c, keyboard.c (keyread) |
| `sim::text` | message table (texttab) | vdu.c message/texthandler |
| `frontend` | App state machine: loading, prelude, menus, play, pause, high scores | asylum.c init, menus.c |
| `render` | RenderFrame building (game thread) + finalize (main thread) | vdu.c, maze.c draw |
| `layout` | viewport → content layout | (new) |
| `display` | display modes, window aspects and resolution lists (up to the monitor), view scale, view size (`video`, shared with the host) | menus.c tunespeed (new) |
| `font` | Fusion Pixel BDF subsets restyled to the original glyph look (§7) | (new) |
| `i18n` | strings.csv parsing, lookup, glyph coverage | (new) |
| `input` | input events, Godot key codes, key names | keyboard.c |
| `config` | settings file | asylum.c loadconfig/saveconfig |
| `storage` | portable user-data dir, high score files, save game file | file.c |
| `runtime` | threads (game, music), channels, frame slot | (new) |

## 7. Text rendering

* Everything — including Korean and Chinese text — is drawn as atlas quads by the core; Godot
  does no text rendering at all.
* The original bitmap font (FSPChars, 48 glyphs) renders every character it covers, in every
  language (digits, A–Z with a–z drawn as A–Z, `. , ! ' ? - %`, `•` U+2022, `←` U+2190, `✓` U+2713, `✗` U+2717,
  ASYLUM logo U+E000, space). Advance rules (vdu.c texthandler): glyph index ≤ 9 → 16 px,
  glyph ≥ 43 or space → 8 px, otherwise 14 px. A text position `(x, y)` is the **centre of
  the first glyph** (original semantics); glyph *i* is drawn with its sprite centre at the pen.
* Every other character (Hangul, Hanzi, CJK punctuation, `:` `/` …) comes from the Fusion Pixel
  12 px BDF subsets in `game/fonts/` and is restyled at start-up exactly like the original
  glyphs (font.rs): 1-px strokes emboldened horizontally (keeping 1-px gaps), 1-px
  4-neighbour outline, core #33BB33 / outline #337733 (palette 67/35), 14-px cell centred on
  the line, advance = BDF dwidth + 3 (15 px for full-width glyphs). Korean text uses the
  Korean glyph forms, Chinese the Simplified Chinese forms; language names in the language
  menu always use their own forms.
* Alignment: `Align::Orig` (x is first-glyph centre, as original), `Align::Center(cx)`,
  `Align::EnCentered` (EN: identical to `Orig`; other languages: centred on the centre of the
  English text at the original position — keeps the original layout for every language).

## 8. Localization

* `game/locale/strings.csv` (UTF-8, header `key,en,ko,zh_CN`) is the single source of truth.
  Keys are declared in `asylum_core::i18n::keys` (one `pub const NAME: &str = "NAME";` per key).
  A unit test asserts every declared key exists in the CSV with all three languages filled.
* English strings reproduce the original texts exactly (case as in the C source; the bitmap
  font is upper-case only). Avoid characters outside the bitmap font in English strings.
* Default language: config value, else OS locale (`ko*` → Korean, `zh*` → Simplified Chinese,
  else English).

## 9. Persistence (portable)

* User data directory: `ASYLUM_USERDIR` if set; in the editor `user://`; otherwise
  `<exe_dir>/userdata/` when writable (portable mode), else `%APPDATA%\Asylum\`, else Godot
  `user://` (the portable engine patch maps `user://` to `<exe_dir>/userdata`, hence the
  explicit `%APPDATA%` step). Files: `asylum.cfg` (text key=value; video keys `DisplayMode`
  = window/borderless/fullscreen, `WindowAspect` = 4:3 … 21:9, `WindowResolution` = WxH,
  `ScreenScale` = 1.0..3.0, ...), `asylum_game.sav`
  (original binary layout of savegame()), `hiscores/{Ego,Psyche,Id,Extended}HighScores`
  (original 66-byte format, CRC byte 0xff; `ExtendedHighScores` is supported by the storage
  code but never written).
* `asylum.cfg` is read as UTF-8 or as UTF-16 with a byte order mark (what Windows editors may
  save); a settings file that exists but cannot be read is copied to `asylum.cfg.bak` once
  before the first save replaces it.
* Godot itself is configured not to write logs or shader caches (file logging off, shader
  cache off); it still creates an empty `user://shader_cache` folder, which the portable
  patch puts in `<exe_dir>/userdata/`. The release zip never contains `userdata/` or the
  `override.cfg` a game run writes.

## 10. Build & export

* `tools/build.ps1`: `cargo build --release` (+ debug for editor) with the C runtime linked
  statically and local paths remapped out of the binary, copy DLLs to `game/bin`,
  `godot --headless --import`, `godot --headless --export-release "Windows Desktop"` into
  `dist/Asylum/` with the PCK embedded, then `README.md`, `LICENSE` (as
  `LICENSE.txt`), `THIRD_PARTY_NOTICES.md` and `licenses/`; `-Zip` writes the release zip from an explicit file
  list. The committed `export_presets.cfg` has empty `custom_template` paths (the official
  4.7.2 templates); when the lean release template built by `tools/build_template.ps1`
  (Godot 4.7.2 + `tools/godot-patches/*.patch`) exists, build.ps1 writes its relative path
  into the preset for the export only and restores the file afterwards. The official template
  lacks both engine patches (§4 and §9).
* Before the import, `tools/build.ps1` creates `game/.godot/extension_list.cfg` if it is
  missing: in a fresh checkout the editor would otherwise load the GDExtension during its first
  scan, and Godot 4.7.2 crashes on exit after such a late load.
* `tools/build_template.ps1` compiles through a temporary `subst` drive, so the template
  embeds no local paths either.

## 11. Quality bar

* `cargo test --release --workspace` green; `cargo clippy` clean of errors.
* Soak test: random-input simulation across all zones and neurons for ≥100k ticks without
  panics (index/overflow), deterministic for a fixed seed.
* No per-frame heap churn on the main thread hot path (reuse buffers).
