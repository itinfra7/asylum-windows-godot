//! Rendering: the game thread builds a [`RenderFrame`] after every tick from a
//! [`SceneDesc`]; the main thread calls [`finalize`] each display frame to interpolate,
//! clip and pack instance data for Godot's 2D MultiMesh (one draw call).
//!
//! Draw order of a game frame (vdu.c / asylum.c game loop):
//! 1. GameScreen (wide composition, docs §4)
//! 2. backdrop tiles (parallax) — clip Game
//! 3. midground blocks (8..11, 15/16 parallax) then foreground blocks — clip Game
//! 4. `Sim::cmds` in order (player dead sprites, aliens, projectiles, bullets, live player…)
//! 5. HUD: bonus box (clip BonusBox: black fill + up to 3 block sprites), strength bar,
//!    lives glyph, score digits
//! 6. message texts (clip Game) — glyph quads like everything else
//!
//! Menus: ChatScreen (extended composition) + highlight bar + texts (clip Chat).
//!
//! Clip table of every frame: [`CLIP_NONE`] (whole viewport), [`CLIP_GAME`],
//! [`CLIP_PLAYER_FLOOR`], [`CLIP_BONUS`], [`CLIP_CHAT`], then per-command clips (dying
//! objects, flagged with [`CLIP_MOVES`]).
//!
//! Draw-time state: a frame is built after the whole `Sim::tick`, but the C drew the maze,
//! backdrop and every `relplot`/`plotdying` with the camera of the `mazeplot` point (before
//! the teleport in `playerplot(false)`) and presented the frame before `player_dead()` /
//! `zonerestart`. The renderer therefore reads the camera and the HUD values only from the
//! snapshot `Sim::draw` ([`crate::sim::DrawState`]), never from the live fields.
//!
//! Interpolation deltas (screen px, previous tick − current tick):
//! * camera `dc = draw.cam - cam_prev`; a jump > `SNAP_PX` zeroes it and every entity delta;
//! * world items: `dc` (backdrop and midground use their own parallax motion), keyed world
//!   items add the anchor motion of their key, keyed centre items use the anchor motion only;
//! * screen items 0; text slots `(prev - cur) / 256`;
//! * any per-quad delta above `SNAP_PX` is dropped; `no_interp` zeroes all of them.

pub mod frame;
pub mod text;
pub mod world;

#[cfg(test)]
pub(crate) mod testkit;
#[cfg(test)]
mod visual;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::atlas::{Atlas, AtlasRect, AtlasSprite, Bank, Img};
use crate::font::GlyphSet;
use crate::i18n::{Lang, Strings};
use crate::layout::Layout;
use crate::sim::{Align, Anchor, Clip, CmdKind, Pos, Sim, SprCmd, TextRef, TextTable, KEY_COUNT, KEY_NONE};

/// One textured quad. Positions in logical viewport pixels; UV in atlas texels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Texel rectangle; `uw`/`vh` may be negative (mirrored sampling).
    pub u: f32,
    pub v: f32,
    pub uw: f32,
    pub vh: f32,
    /// Screen-space offset to this quad's position in the previous tick (interpolation).
    pub dx: f32,
    pub dy: f32,
    /// RGBA8 modulate (0xRRGGBBAA). 0xFFFFFFFF = unmodified.
    pub color: u32,
    /// Index into `RenderFrame::clips` (0 = no clipping) in the low 15 bits.
    ///
    /// High bit [`CLIP_MOVES`] (0x8000): the clip rectangle travels with the quad — it is
    /// translated by the same interpolation offset as the quad (`(1-alpha)*delta`, after
    /// snapping) and then intersected with clip [`CLIP_GAME`]. Used by `cenplotdying`, whose
    /// four clip windows are derived from the object's position.
    pub clip: u16,
}

pub const WHITE: u32 = 0xFFFF_FFFF;
pub const BLACK: u32 = 0x0000_00FF;
/// Colour of the menu highlight bar (translucent white).
pub const HIGHLIGHT: u32 = 0xFFFF_FF38;

/// Fixed clip table entries.
pub const CLIP_NONE: u16 = 0;
pub const CLIP_GAME: u16 = 1;
pub const CLIP_PLAYER_FLOOR: u16 = 2;
pub const CLIP_BONUS: u16 = 3;
pub const CLIP_CHAT: u16 = 4;
/// Flag bit of [`Quad::clip`]: the clip rect moves with the quad (see there).
pub const CLIP_MOVES: u16 = 0x8000;
const CLIP_INDEX_MASK: u16 = 0x7FFF;

/// Everything the main thread needs to draw one tick.
#[derive(Clone, Debug, Default)]
pub struct RenderFrame {
    /// Monotonic sequence number (one per tick).
    pub seq: u64,
    pub quads: Vec<Quad>,
    /// Clip rectangles [x0, y0, x1, y1) in viewport pixels; index 0 = unclipped (whole view).
    pub clips: Vec<[f32; 4]>,
    /// Background clear colour (RGBA8).
    pub clear: u32,
    /// Round final positions to whole pixels (only the rasterizing render tests set it; the
    /// app always passes false).
    pub snap: bool,
    /// Disable interpolation (smooth motion off).
    pub no_interp: bool,
}

/// What the frontend wants on screen this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    Black,
    /// ChatScreen (menus, prelude, high scores) — texts clipped to the chat window.
    Chat,
    /// GameScreen + world + HUD (requires `sim`) — texts clipped to the game window.
    Game,
    /// GameScreen frame only (loading a level; shows the original "Digital Psychosis" art).
    GameFrame,
}

/// A static line of a menu (drawn after the animated message table).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuLine {
    pub text: TextRef,
    /// Original-semantics position (first glyph centre), content coordinates.
    pub x: f32,
    pub y: f32,
    pub align: Align,
    /// Optional prefix glyph drawn before the text (✓ / ✗ like the original menus).
    pub prefix: Option<char>,
}

/// Scene description handed from the frontend to `build_frame`.
pub struct SceneDesc<'a> {
    pub lay: Layout,
    pub lang: Lang,
    pub background: Background,
    pub sim: Option<&'a Sim>,
    pub texts: &'a TextTable,
    pub lines: &'a [MenuLine],
    /// Highlight bar for the selected menu line (content rect x0,y0,x1,y1).
    pub highlight: Option<[f32; 4]>,
    /// See `RenderFrame::snap` (tests only).
    pub snap: bool,
    pub no_interp: bool,
}

/// Immutable resources the renderer needs.
#[derive(Clone)]
pub struct RenderCtx {
    pub atlas: Arc<Atlas>,
    pub strings: Arc<Strings>,
    pub glyphs: Arc<GlyphSet>,
}

/// Texture lookups of the emitters. Implemented by [`Atlas`]; the unit tests use a fake
/// atlas built straight from the decoded assets.
pub(crate) trait TexSource {
    /// Non-empty sprite `idx` of `bank`.
    fn spr(&self, bank: Bank, idx: usize) -> Option<AtlasSprite>;
    /// Non-empty styled glyph `id`.
    fn glyph_spr(&self, id: u32) -> Option<AtlasSprite>;
    /// Full image rectangle (zero size if absent).
    fn img(&self, img: Img) -> AtlasRect;
}

impl TexSource for Atlas {
    fn spr(&self, bank: Bank, idx: usize) -> Option<AtlasSprite> {
        self.sprite(bank, idx).copied().filter(|s| s.w > 0 && s.h > 0)
    }
    fn glyph_spr(&self, id: u32) -> Option<AtlasSprite> {
        self.glyph(id).copied().filter(|s| s.w > 0 && s.h > 0)
    }
    fn img(&self, img: Img) -> AtlasRect {
        self.image(img)
    }
}

/// Process-wide frame counter (frames may be double buffered, so `out.seq` cannot be used).
static FRAME_SEQ: AtomicU64 = AtomicU64::new(0);

/// Build the frame for the current tick (game thread). Reuses `out`'s allocations.
pub fn build_frame(scene: &SceneDesc, ctx: &RenderCtx, out: &mut RenderFrame) {
    build_frame_with(scene, &*ctx.atlas, &ctx.strings, &ctx.glyphs, out);
}

pub(crate) fn build_frame_with(scene: &SceneDesc, tex: &dyn TexSource, strings: &Strings, glyphs: &GlyphSet, out: &mut RenderFrame) {
    let lay = &scene.lay;
    out.seq = FRAME_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    out.quads.clear();
    out.clips.clear();
    out.clear = BLACK;
    out.snap = scene.snap;
    out.no_interp = scene.no_interp;
    push_fixed_clips(lay, &mut out.clips);

    let sim = scene.sim;
    let background = match (scene.background, sim) {
        (Background::Game, None) => Background::GameFrame,
        (b, _) => b,
    };
    let text_clip = match background {
        Background::Game => CLIP_GAME,
        Background::Chat => CLIP_CHAT,
        _ => CLIP_NONE,
    };

    match background {
        Background::Black => {}
        Background::Chat => frame::chat_screen_quads(lay, tex, &mut out.quads),
        Background::GameFrame => frame::game_screen_quads(lay, tex, &mut out.quads),
        Background::Game => {
            let sim = sim.expect("checked above");
            frame::game_screen_quads(lay, tex, &mut out.quads);
            let motion = Motion::new(sim);
            world::world_quads(sim, lay, tex, CLIP_GAME, motion.dc, &mut out.quads);
            for cmd in &sim.cmds {
                emit_cmd(sim, lay, tex, &motion, cmd, out);
            }
            frame::hud_quads(sim, lay, tex, CLIP_BONUS, &mut out.quads);
        }
    }

    if let Some([x0, y0, x1, y1]) = scene.highlight {
        let r = tex.img(Img::White);
        if r.w > 0 && x1 > x0 && y1 > y0 {
            out.quads.push(Quad {
                x: lay.vx(x0),
                y: lay.vy(y0),
                w: x1 - x0,
                h: y1 - y0,
                u: r.u as f32,
                v: r.v as f32,
                uw: r.w as f32,
                vh: r.h as f32,
                color: HIGHLIGHT,
                clip: text_clip,
                ..Quad::default()
            });
        }
    }

    let tx = text::TextEmitter { lay, lang: scene.lang, tex, strings, glyphs, clip: text_clip };
    tx.emit_table(scene.texts, &mut out.quads);
    tx.emit_lines(scene.lines, &mut out.quads);

    for q in out.quads.iter_mut() {
        if scene.no_interp || q.dx.abs() > SNAP_PX || q.dy.abs() > SNAP_PX {
            q.dx = 0.0;
            q.dy = 0.0;
        }
    }
}

/// Clips 0..=4 (see module docs), viewport coordinates.
fn push_fixed_clips(lay: &Layout, clips: &mut Vec<[f32; 4]>) {
    let [gx0, gy0, gx1, _] = lay.game_rect();
    let (bx, by) = ((frame::BONUSX + lay.hx) as f32, frame::BONUSY + lay.hy);
    clips.push([0.0, 0.0, lay.view_w, lay.view_h]);
    clips.push(lay.game_rect());
    clips.push([gx0, gy0, gx1, lay.vy((lay.gcy() + 16) as f32)]);
    clips.push([
        lay.vx(bx - 8.0),
        lay.vy((by - frame::BONUSH) as f32),
        lay.vx(bx + 8.0),
        lay.vy((by + frame::BONUSH) as f32),
    ]);
    clips.push(lay.chat_rect());
}

/// Per-frame interpolation state of a game frame.
struct Motion<'a> {
    /// Camera delta (prev - cur screen position of world-fixed things), 0 after a snap.
    dc: (i32, i32),
    /// Camera jumped: all entity deltas are dropped.
    snapped: bool,
    tick: u32,
    anchors: &'a [Anchor],
    anchors_prev: &'a [Anchor],
}

impl<'a> Motion<'a> {
    fn new(sim: &'a Sim) -> Motion<'a> {
        // Camera the C drew this tick with (mazeplot point), not the post-tick camera.
        let (cx, cy) = sim.draw.cam;
        let dc = (cx.wrapping_sub(sim.cam_prev.0), cy.wrapping_sub(sim.cam_prev.1));
        let snapped = dc.0.unsigned_abs() as f32 > SNAP_PX || dc.1.unsigned_abs() as f32 > SNAP_PX;
        Motion {
            dc: if snapped { (0, 0) } else { dc },
            snapped,
            tick: sim.tick_no,
            anchors: &sim.anchors,
            anchors_prev: &sim.anchors_prev,
        }
    }

    /// Motion of the object behind `key` between the previous and this tick (prev - cur).
    fn anchor_delta(&self, key: u16) -> Option<(i32, i32)> {
        let k = key as usize;
        if key == KEY_NONE || k >= KEY_COUNT || self.snapped {
            return None;
        }
        let (a, p) = (self.anchors.get(k)?, self.anchors_prev.get(k)?);
        (a.tick == self.tick && p.tick == self.tick.wrapping_sub(1) && a.gen == p.gen)
            .then(|| (p.x.wrapping_sub(a.x), p.y.wrapping_sub(a.y)))
    }

    /// Screen delta of a command.
    fn cmd_delta(&self, cmd: &SprCmd) -> (f32, f32) {
        let (x, y) = match cmd.pos {
            Pos::World { .. } => {
                let (ax, ay) = self.anchor_delta(cmd.key).unwrap_or((0, 0));
                (self.dc.0.wrapping_add(ax), self.dc.1.wrapping_add(ay))
            }
            Pos::Center { .. } => self.anchor_delta(cmd.key).unwrap_or((0, 0)),
            Pos::Screen { .. } => (0, 0),
        };
        (x as f32, y as f32)
    }
}

/// `framectr` as the C drawing code of this tick saw it: mazeplot/showstrength run before
/// the loop's `framectr += frameinc`, frames are built after the tick.
pub(crate) fn draw_framectr(sim: &Sim) -> i32 {
    sim.framectr.wrapping_sub(sim.frameinc)
}

fn clip_index(c: Clip) -> u16 {
    match c {
        Clip::Game => CLIP_GAME,
        Clip::PlayerFloor => CLIP_PLAYER_FLOOR,
        Clip::BonusBox => CLIP_BONUS,
        Clip::None => CLIP_NONE,
    }
}

/// Quad of sprite `s` with its hot spot at content position (x, y).
pub(crate) fn sprite_quad(lay: &Layout, s: &AtlasSprite, x: i32, y: i32, clip: u16, delta: (f32, f32)) -> Quad {
    Quad {
        x: lay.vx((x - s.xc as i32) as f32),
        y: lay.vy((y - s.yc as i32) as f32),
        w: s.w as f32,
        h: s.h as f32,
        u: s.u as f32,
        v: s.v as f32,
        uw: s.w as f32,
        vh: s.h as f32,
        dx: delta.0,
        dy: delta.1,
        color: WHITE,
        clip,
    }
}

/// One recorded draw call → quads (vdu.c cenplot / blokeplot_cen / fspplot / cenplotdying).
fn emit_cmd(sim: &Sim, lay: &Layout, tex: &dyn TexSource, motion: &Motion, cmd: &SprCmd, out: &mut RenderFrame) {
    let Some(s) = tex.spr(cmd.bank, cmd.idx as usize) else { return };
    let delta = motion.cmd_delta(cmd);
    // relplot/plotdying used the live xpos/ypos of the drawing point (before a teleport or
    // restart later in the same tick).
    let (camx, camy) = sim.draw.cam;
    match cmd.kind {
        CmdKind::Sprite => {
            let (x, y) = match cmd.pos {
                Pos::World { x, y } => (lay.gcx() - 1 + x - camx, lay.gcy() - 8 + y - camy),
                Pos::Center { x, y } => (lay.gcx() + x, lay.gcy() + y),
                Pos::Screen { x, y } => (x, y),
            };
            out.quads.push(sprite_quad(lay, &s, x, y, clip_index(cmd.clip), delta));
        }
        CmdKind::Dying { r5 } => {
            // Only `plotdying` (world coordinates) records Dying; other positions are
            // resolved like world coordinates relative to the centre.
            let (x, y) = match cmd.pos {
                Pos::World { x, y } => (x - camx, y - camy),
                Pos::Center { x, y } => (x, y + 8),
                Pos::Screen { x, y } => (x - lay.gcx(), y - lay.gcy() + 8),
            };
            for (px, py, rect) in cenplotdying(lay, x, y, r5).into_iter().flatten() {
                if out.clips.len() > CLIP_INDEX_MASK as usize {
                    break;
                }
                let ci = out.clips.len() as u16 | CLIP_MOVES;
                out.clips.push([
                    lay.vx(rect[0] as f32),
                    lay.vy(rect[1] as f32),
                    lay.vx(rect[2] as f32),
                    lay.vy(rect[3] as f32),
                ]);
                out.quads.push(sprite_quad(lay, &s, px, py, ci, delta));
            }
        }
    }
}

/// vdu.c `cenplotdying(sprites, n, x, y, r5)` with `x, y` relative to the camera
/// (`(x>>8)-(xpos>>8)`): up to four copies `(hot spot x, y, clip [x0, y0, x1, y1))` in content
/// coordinates, in the C plotting order.
pub(crate) fn cenplotdying(lay: &Layout, x: i32, y: i32, r5: i32) -> [Option<(i32, i32, [i32; 4])>; 4] {
    let (gamex, gamey, gamew, gameh) = (lay.gamex, lay.gamey, lay.gamew, lay.gameh);
    let mut out = [None; 4];
    let r8 = 8 - (r5 >> 17);
    let r1 = x + gamex + gamew / 2;
    let r2 = r8 + y + gamey + gameh / 2 - 8; // sprh/2 = 8
    let mut fsply = gamey;
    let mut fsplx = gamex;
    let mut fsphy = r2;
    let mut fsphx;
    if fsphy <= gamey + gameh && fsply < fsphy {
        fsphx = r1;
        if fsphx <= gamex + gamew && fsplx < fsphx {
            out[0] = Some((fsphx + r8, fsphy + r8, [fsplx, fsply, fsphx, fsphy]));
        }
        fsphx = gamex + gamew;
        fsplx = r1;
        if fsplx >= gamex && fsphx > gamex {
            out[1] = Some((fsplx - r8, fsphy + r8, [fsplx, fsply, fsphx, fsphy]));
        }
    }
    //nodyingtop:
    fsphy = gamey + gameh;
    fsplx = gamex;
    fsply = r2;
    if fsply >= gamey && fsply < fsphy {
        fsphx = r1;
        if fsphx <= gamex + gamew && fsplx < fsphx {
            out[2] = Some((fsphx + r8, fsply - r8, [fsplx, fsply, fsphx, fsphy]));
        }
        fsphx = gamex + gamew;
        fsplx = r1;
        if fsplx >= gamex && fsphx > fsplx {
            out[3] = Some((fsplx - r8, fsply - r8, [fsplx, fsply, fsphx, fsphy]));
        }
    }
    out
}

/// Floats per MultiMesh instance (Transform2D 8 + colour 4 + custom data 4).
pub const FLOATS_PER_INSTANCE: usize = 16;

/// Main thread: interpolate (`pos = cur + (1-alpha)*delta`, deltas > SNAP_PX ignored at
/// build time), clip against `frame.clips`, cull, and pack instances for a 2D MultiMesh:
/// `[w, 0, 0, x,  0, h, 0, y,  r, g, b, a,  u, v, uw, vh]` (unit quad mesh 0..1).
/// The atlas rectangle is in texels: the GL Compatibility renderer packs colour and custom
/// data as truncated 16-bit floats, which hold every whole texel up to 2048 exactly (the
/// shader normalises with `TEXTURE_PIXEL_SIZE`); normalised coordinates would not be exact.
/// Returns the number of instances written to `out` (which is resized as needed).
pub fn finalize(frame: &RenderFrame, alpha: f32, out: &mut Vec<f32>) -> usize {
    out.clear();
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
    let t = if frame.no_interp { 0.0 } else { 1.0 - alpha };
    let game = frame.clips.get(CLIP_GAME as usize).copied();
    let mut n = 0;
    for q in &frame.quads {
        if !(q.w > 0.0 && q.h > 0.0) || q.color & 0xFF == 0 {
            continue;
        }
        let mut x = q.x + q.dx * t;
        let mut y = q.y + q.dy * t;
        if frame.snap {
            x = x.round();
            y = y.round();
        }
        let (mut x0, mut y0, mut x1, mut y1) = (x, y, x + q.w, y + q.h);
        let (mut u, mut v, mut uw, mut vh) = (q.u, q.v, q.uw, q.vh);
        let ci = (q.clip & CLIP_INDEX_MASK) as usize;
        if ci != 0 {
            if let Some(&r) = frame.clips.get(ci) {
                let mut r = r;
                if q.clip & CLIP_MOVES != 0 {
                    let (ox, oy) = (x - q.x, y - q.y);
                    r = [r[0] + ox, r[1] + oy, r[2] + ox, r[3] + oy];
                    if let Some(g) = game {
                        r = [r[0].max(g[0]), r[1].max(g[1]), r[2].min(g[2]), r[3].min(g[3])];
                    }
                }
                let (nx0, nx1) = (x0.max(r[0]), x1.min(r[2]));
                let (ny0, ny1) = (y0.max(r[1]), y1.min(r[3]));
                if !(nx1 > nx0 && ny1 > ny0) {
                    continue;
                }
                let (fx, fy) = (uw / q.w, vh / q.h);
                u += (nx0 - x0) * fx;
                uw = (nx1 - nx0) * fx;
                v += (ny0 - y0) * fy;
                vh = (ny1 - ny0) * fy;
                (x0, x1, y0, y1) = (nx0, nx1, ny0, ny1);
            }
        }
        let c = q.color;
        out.extend_from_slice(&[
            x1 - x0,
            0.0,
            0.0,
            x0,
            0.0,
            y1 - y0,
            0.0,
            y0,
            (c >> 24) as f32 / 255.0,
            ((c >> 16) & 0xFF) as f32 / 255.0,
            ((c >> 8) & 0xFF) as f32 / 255.0,
            (c & 0xFF) as f32 / 255.0,
            u,
            v,
            uw,
            vh,
        ]);
        n += 1;
    }
    n
}

/// What the GL Compatibility renderer makes of an instance colour / custom value
/// (`Math::make_half_float`: mantissa truncated to 10 bits, denormals flushed to zero).
#[cfg(test)]
pub(crate) fn half(x: f32) -> f32 {
    if x.abs() < 1.0 / 16384.0 {
        0.0
    } else {
        f32::from_bits(x.to_bits() & !0x1FFF)
    }
}

/// Interpolation deltas above this many pixels are dropped (teleports, slot reuse).
pub const SNAP_PX: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::testkit::*;
    use crate::sim::{Anchor, KEY_ALIEN};

    fn frame_with(quads: Vec<Quad>, clips: Vec<[f32; 4]>) -> RenderFrame {
        RenderFrame { seq: 1, quads, clips, clear: BLACK, snap: false, no_interp: false }
    }

    fn quad(x: f32, y: f32, w: f32, h: f32) -> Quad {
        Quad { x, y, w, h, u: 100.0, v: 200.0, uw: w, vh: h, color: WHITE, ..Quad::default() }
    }

    #[test]
    fn finalize_packs_instances() {
        let mut q = quad(10.0, 20.0, 16.0, 8.0);
        q.color = 0xFF80_40C0;
        let f = frame_with(vec![q], vec![[0.0, 0.0, 320.0, 256.0]]);
        let mut out = Vec::new();
        assert_eq!(finalize(&f, 1.0, &mut out), 1);
        let want = [16.0, 0.0, 0.0, 10.0, 0.0, 8.0, 0.0, 20.0, 1.0, 128.0 / 255.0, 64.0 / 255.0, 192.0 / 255.0, 100.0, 200.0, 16.0, 8.0];
        assert_eq!(out.len(), FLOATS_PER_INSTANCE);
        for (a, b) in out.iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{out:?}");
        }
    }

    #[test]
    fn finalize_interpolates_and_snaps() {
        let mut q = quad(10.0, 20.0, 4.0, 4.0);
        (q.dx, q.dy) = (3.0, -5.0);
        let mut f = frame_with(vec![q], vec![[0.0, 0.0, 320.0, 256.0]]);
        let mut out = Vec::new();
        finalize(&f, 0.0, &mut out);
        assert_eq!((out[3], out[7]), (13.0, 15.0));
        finalize(&f, 0.5, &mut out);
        assert_eq!((out[3], out[7]), (11.5, 17.5));
        finalize(&f, 7.0, &mut out); // alpha clamped
        assert_eq!((out[3], out[7]), (10.0, 20.0));
        finalize(&f, f32::NAN, &mut out);
        assert_eq!((out[3], out[7]), (10.0, 20.0));
        f.snap = true;
        finalize(&f, 0.4, &mut out); // 10 + 1.8, 20 - 3
        assert_eq!((out[3], out[7]), (12.0, 17.0));
        f.snap = false;
        f.no_interp = true;
        finalize(&f, 0.0, &mut out);
        assert_eq!((out[3], out[7]), (10.0, 20.0));
    }

    #[test]
    fn finalize_clips_and_adjusts_uvs() {
        // 16x8 quad at (10,20) with texels (100,200)..(116,208), clip [14,22)-(20,40)
        let q = Quad { clip: 1, ..quad(10.0, 20.0, 16.0, 8.0) };
        let f = frame_with(vec![q], vec![[0.0, 0.0, 320.0, 256.0], [14.0, 22.0, 20.0, 40.0]]);
        let mut out = Vec::new();
        assert_eq!(finalize(&f, 1.0, &mut out), 1);
        assert_eq!(&out[..8], &[6.0, 0.0, 0.0, 14.0, 0.0, 6.0, 0.0, 22.0]);
        assert_eq!(&out[12..], &[104.0, 202.0, 6.0, 6.0]);
        // mirrored UVs are clipped proportionally too
        let q = Quad { clip: 1, u: 116.0, uw: -16.0, ..quad(10.0, 20.0, 16.0, 8.0) };
        let f = frame_with(vec![q], f.clips.clone());
        finalize(&f, 1.0, &mut out);
        assert_eq!(&out[12..], &[112.0, 202.0, -6.0, 6.0]);
        // scaled quad (2 px per texel)
        let q = Quad { clip: 1, uw: 8.0, ..quad(10.0, 20.0, 16.0, 8.0) };
        let f = frame_with(vec![q], f.clips.clone());
        finalize(&f, 1.0, &mut out);
        assert_eq!(out[12], 102.0);
        assert_eq!(out[14], 3.0);
    }

    #[test]
    fn finalize_culls_and_reuses_buffer() {
        let outside = Quad { clip: 1, ..quad(100.0, 100.0, 8.0, 8.0) };
        let empty = quad(0.0, 0.0, 0.0, 8.0);
        let invisible = Quad { color: 0xFFFF_FF00, ..quad(0.0, 0.0, 8.0, 8.0) };
        let touching = Quad { clip: 1, ..quad(20.0, 20.0, 8.0, 8.0) }; // x1 == clip x0
        let ok = quad(1.0, 1.0, 2.0, 2.0);
        let f = frame_with(vec![outside, empty, invisible, touching, ok], vec![[0.0; 4], [28.0, 0.0, 50.0, 50.0]]);
        let mut out = Vec::with_capacity(1024);
        let cap = out.capacity();
        out.extend_from_slice(&[9.0; 40]);
        assert_eq!(finalize(&f, 1.0, &mut out), 1);
        assert_eq!(out.len(), FLOATS_PER_INSTANCE);
        assert_eq!(out.capacity(), cap);
        assert_eq!(out[3], 1.0);
    }

    #[test]
    fn finalize_moving_clip_follows_quad_then_game_window() {
        // clip [10,10)-(20,20) moving with the quad; game window clip [0,0)-(16,100)
        let mut q = Quad { clip: 2 | CLIP_MOVES, ..quad(10.0, 10.0, 10.0, 10.0) };
        (q.dx, q.dy) = (4.0, 0.0);
        let f = frame_with(vec![q], vec![[0.0, 0.0, 320.0, 256.0], [0.0, 0.0, 16.0, 100.0], [10.0, 10.0, 20.0, 20.0]]);
        let mut out = Vec::new();
        // alpha 0: quad and clip at x = 14..24, game window cuts at 16
        finalize(&f, 0.0, &mut out);
        assert_eq!((out[3], out[0]), (14.0, 2.0));
        assert_eq!((out[12], out[14]), (100.0, 2.0));
        // a fixed clip would cut the moved quad at its old place instead
        let q2 = Quad { clip: 2, ..q };
        let f2 = frame_with(vec![q2], f.clips.clone());
        finalize(&f2, 0.0, &mut out);
        assert_eq!((out[3], out[0]), (14.0, 6.0));
    }

    fn scene<'a>(lay: Layout, bg: Background, sim: Option<&'a Sim>, texts: &'a TextTable, lines: &'a [MenuLine]) -> SceneDesc<'a> {
        SceneDesc { lay, lang: Lang::En, background: bg, sim, texts, lines, highlight: None, snap: false, no_interp: false }
    }

    #[test]
    fn clip_table_matches_original_windows() {
        let tk = TestKit::get();
        let texts = TextTable::default();
        let mut f = RenderFrame::default();
        let lay = Layout::new(400.0, 300.0); // cw 400, ch 300, hx 40, hy 44, gameh 236
        build_frame_with(&scene(lay, Background::Black, None, &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert_eq!(f.clips.len(), 5);
        assert_eq!(f.clips[0], [0.0, 0.0, 400.0, 300.0]);
        assert_eq!(f.clips[1], [16.0, 8.0, 384.0, 244.0]);
        assert_eq!(f.clips[2], [16.0, 8.0, 384.0, 126.0 + 16.0]);
        assert_eq!(f.clips[3], [40.0 + 282.0, 44.0 + 212.0, 40.0 + 298.0, 44.0 + 252.0]);
        assert_eq!(f.clips[4], [20.0, 20.0, 379.0, 279.0]);
        assert_eq!(f.clear, BLACK);
        let seq = f.seq;
        build_frame_with(&scene(lay, Background::Black, None, &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert!(f.seq > seq);
    }

    #[test]
    fn cenplotdying_matches_c_conditionals() {
        let lay = Layout::original();
        // r5 = 0 → r8 = 8; object at the centre: r1 = 160, r2 = 8 + 96 = 104
        let c = cenplotdying(&lay, 0, 0, 0);
        assert_eq!(c[0], Some((168, 112, [16, 8, 160, 104])));
        assert_eq!(c[1], Some((152, 112, [160, 8, 304, 104])));
        assert_eq!(c[2], Some((168, 96, [16, 104, 160, 200])));
        assert_eq!(c[3], Some((152, 96, [160, 104, 304, 200])));
        // r5 = 5<<17 → r8 = 3; far left: r1 = 10 < gamex → only the right-hand copies,
        // but "fsplx >= gamex" fails for them, and the left copies need fsplx < fsphx
        let c = cenplotdying(&lay, -150, 0, 5 << 17);
        assert_eq!(c, [None; 4]);
        // below the window: r2 > gamey+gameh → no top copies; bottom needs r2 < 200
        let c = cenplotdying(&lay, 0, 120, 0);
        assert_eq!(c, [None; 4]);
        // r2 exactly at the bottom edge (8 + 96 + 96 = 200): top copies only
        let c = cenplotdying(&lay, 0, 96, 0);
        assert_eq!(c[0].map(|v| v.2), Some([16, 8, 160, 200]));
        assert!(c[2].is_none() && c[3].is_none());
        // bottom-right asymmetry of the C: "fsphx > fsplx" instead of "> gamex"
        let c = cenplotdying(&lay, 144, 0, 0); // r1 = 304 = gamex+gamew
        assert!(c[1].is_some() && c[3].is_none());
    }

    #[test]
    fn game_frame_deltas_follow_camera_and_anchors() {
        let tk = TestKit::get();
        let mut sim = crate::sim::testutil::sim();
        sim.boardwidth = sim.bd().width;
        sim.xpos = 1000 << 8;
        sim.ypos = 600 << 8;
        sim.sync_draw();
        sim.cam_prev = (997, 602); // camera moved +3, -2
        sim.tick_no = 10;
        let k = KEY_ALIEN + 5;
        sim.anchors[k as usize] = Anchor { x: 1050, y: 640, gen: 7, tick: 10 };
        sim.anchors_prev[k as usize] = Anchor { x: 1046, y: 641, gen: 7, tick: 9 };
        let bank = sim.aliens_bank();
        sim.cmds.push(SprCmd { bank, idx: 0, pos: Pos::World { x: 1050, y: 640 }, clip: Clip::Game, kind: CmdKind::Sprite, key: k });
        sim.cmds.push(SprCmd { bank, idx: 0, pos: Pos::World { x: 1050, y: 640 }, clip: Clip::Game, kind: CmdKind::Sprite, key: 0 });
        sim.cmds.push(SprCmd { bank, idx: 0, pos: Pos::Center { x: 3, y: 4 }, clip: Clip::PlayerFloor, kind: CmdKind::Sprite, key: k });
        sim.cmds.push(SprCmd { bank, idx: 0, pos: Pos::Screen { x: 30, y: 40 }, clip: Clip::None, kind: CmdKind::Sprite, key: k });
        let texts = TextTable::default();
        let lay = Layout::original();
        let mut f = RenderFrame::default();
        let n_world = {
            let mut v = Vec::new();
            frame::game_screen_quads(&lay, &tk.atlas, &mut v);
            let screen = v.len();
            world::world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (3, -2), &mut v);
            (screen, v.len())
        };
        build_frame_with(&scene(lay, Background::Game, Some(&sim), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        let s = tk.atlas.spr(bank, 0).unwrap();
        let cmds = &f.quads[n_world.1..n_world.1 + 4];
        // world, keyed: dc + anchor motion
        assert_eq!((cmds[0].x, cmds[0].y), ((159 + 50 - s.xc as i32) as f32, (96 + 40 - s.yc as i32) as f32));
        assert_eq!((cmds[0].dx, cmds[0].dy, cmds[0].clip), (3.0 - 4.0, -2.0 + 1.0, CLIP_GAME));
        // world, no key: camera only
        assert_eq!((cmds[1].dx, cmds[1].dy), (3.0, -2.0));
        // centre, keyed: anchor motion only
        assert_eq!((cmds[2].x, cmds[2].y), ((163 - s.xc as i32) as f32, (108 - s.yc as i32) as f32));
        assert_eq!((cmds[2].dx, cmds[2].dy, cmds[2].clip), (-4.0, 1.0, CLIP_PLAYER_FLOOR));
        // screen: none
        assert_eq!((cmds[3].dx, cmds[3].dy, cmds[3].clip), (0.0, 0.0, CLIP_NONE));
        // the GameScreen is static, blocks move with the camera
        assert!(f.quads[..n_world.0].iter().all(|q| q.dx == 0.0 && q.dy == 0.0));

        // generation change → no anchor motion; camera jump → nothing moves
        sim.anchors_prev[k as usize].gen = 6;
        build_frame_with(&scene(lay, Background::Game, Some(&sim), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert_eq!((f.quads[n_world.1].dx, f.quads[n_world.1].dy), (3.0, -2.0));
        sim.anchors_prev[k as usize].gen = 7;
        sim.cam_prev = (900, 602);
        build_frame_with(&scene(lay, Background::Game, Some(&sim), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert!(f.quads.iter().all(|q| q.dx == 0.0 && q.dy == 0.0));
        // smooth motion off
        sim.cam_prev = (997, 602);
        let mut sc = scene(lay, Background::Game, Some(&sim), &texts, &[]);
        sc.no_interp = true;
        build_frame_with(&sc, &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert!(f.quads.iter().all(|q| q.dx == 0.0 && q.dy == 0.0));
        assert!(f.no_interp);
    }

    #[test]
    fn dying_commands_get_moving_clips() {
        let tk = TestKit::get();
        let mut sim = crate::sim::testutil::sim();
        sim.xpos = 1000 << 8;
        sim.ypos = 600 << 8;
        sim.sync_draw();
        sim.cam_prev = (1000, 600);
        let bank = sim.blocks_bank();
        sim.cmds.push(SprCmd { bank, idx: 16, pos: Pos::World { x: 1000, y: 600 }, clip: Clip::Game, kind: CmdKind::Dying { r5: 3 << 17 }, key: 0 });
        let texts = TextTable::default();
        let lay = Layout::original();
        let mut f = RenderFrame::default();
        build_frame_with(&scene(lay, Background::Game, Some(&sim), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert_eq!(f.clips.len(), 5 + 4);
        let dying: Vec<&Quad> = f.quads.iter().filter(|q| q.clip & CLIP_MOVES != 0).collect();
        assert_eq!(dying.len(), 4);
        let expect = cenplotdying(&lay, 0, 0, 3 << 17);
        let s = tk.atlas.spr(bank, 16).unwrap();
        for (q, e) in dying.iter().zip(expect) {
            let (px, py, r) = e.unwrap();
            assert_eq!((q.x, q.y), ((px - s.xc as i32) as f32, (py - s.yc as i32) as f32));
            let c = f.clips[(q.clip & CLIP_INDEX_MASK) as usize];
            assert_eq!(c, [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]);
        }
    }

    #[test]
    fn commands_and_camera_motion_use_draw_time_camera() {
        // Teleport tick (player.c playerplot, telepctr == 32): xpos/ypos change after mazeplot
        // and every relplot/plotdying of the tick. Those draws used the departure camera.
        let tk = TestKit::get();
        let mut sim = crate::sim::testutil::sim();
        sim.xpos = 1000 << 8;
        sim.ypos = 600 << 8;
        sim.sync_draw();
        sim.cam_prev = (997, 602); // camera moved +3, -2 up to the drawing point
        let bank = sim.aliens_bank();
        let alien = SprCmd { bank, idx: 0, pos: Pos::World { x: 1050, y: 640 }, clip: Clip::Game, kind: CmdKind::Sprite, key: 0 };
        let dbank = sim.blocks_bank();
        let dying = SprCmd { bank: dbank, idx: 16, pos: Pos::World { x: 1000, y: 600 }, clip: Clip::Game, kind: CmdKind::Dying { r5: 3 << 17 }, key: 0 };
        // xpos = telepxpos; ypos = telepypos (after the drawing point)
        sim.xpos = 3007 << 8;
        sim.ypos = 205 << 8;

        let motion = Motion::new(&sim);
        assert_eq!((motion.dc, motion.snapped), ((3, -2), false));
        let lay = Layout::original();
        let mut f = RenderFrame::default();
        push_fixed_clips(&lay, &mut f.clips);
        emit_cmd(&sim, &lay, &tk.atlas, &motion, &alien, &mut f);
        let s = tk.atlas.spr(bank, 0).unwrap();
        assert_eq!(f.quads.len(), 1);
        let q = f.quads[0];
        assert_eq!((q.x, q.y), ((159 + 50 - s.xc as i32) as f32, (96 + 40 - s.yc as i32) as f32));
        assert_eq!((q.dx, q.dy), (3.0, -2.0));

        f.quads.clear();
        emit_cmd(&sim, &lay, &tk.atlas, &motion, &dying, &mut f);
        let s = tk.atlas.spr(dbank, 16).unwrap();
        let want: Vec<(f32, f32)> = cenplotdying(&lay, 0, 0, 3 << 17)
            .into_iter()
            .flatten()
            .map(|(px, py, _)| ((px - s.xc as i32) as f32, (py - s.yc as i32) as f32))
            .collect();
        assert_eq!(want.len(), 4);
        assert_eq!(f.quads.iter().map(|q| (q.x, q.y)).collect::<Vec<_>>(), want);
    }

    #[test]
    fn game_frame_ignores_state_changed_after_drawing() {
        // Death tick: switchbank() presents the frame before player_dead() (lives--,
        // prepstrength) and zonerestart (restartplayer moves xpos/ypos, reinitplayer sets
        // bonusreplot = 28). A teleport in playerplot(false) also comes after every
        // world-relative draw. The frame must stay the one of the drawing points.
        let tk = TestKit::get();
        let mut sim = crate::sim::testutil::sim();
        sim.boardwidth = sim.bd().width;
        sim.xpos = 1000 << 8;
        sim.ypos = 600 << 8;
        sim.lives = 3;
        sim.plstrength = crate::sim::STRENGTHMAX / 4;
        sim.bonusctr = 3;
        sim.bonusreplot = 0;
        sim.sync_draw();
        sim.cam_prev = (997, 602);
        let bank = sim.aliens_bank();
        sim.cmds.push(SprCmd { bank, idx: 0, pos: Pos::World { x: 1050, y: 640 }, clip: Clip::Game, kind: CmdKind::Sprite, key: 0 });
        let dbank = sim.blocks_bank();
        sim.cmds.push(SprCmd { bank: dbank, idx: 16, pos: Pos::World { x: 1010, y: 590 }, clip: Clip::Game, kind: CmdKind::Dying { r5: 3 << 17 }, key: 0 });
        let texts = TextTable::default();
        let lay = Layout::original();
        let mut want = RenderFrame::default();
        build_frame_with(&scene(lay, Background::Game, Some(&sim), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut want);
        assert!(want.quads.iter().any(|q| q.dx == 3.0 && q.dy == -2.0), "camera motion of the drawing point");

        sim.lives -= 1;
        sim.plstrength = crate::sim::STRENGTHMAX;
        sim.xpos = 3007 << 8;
        sim.ypos = 205 << 8;
        sim.bonusreplot = 28;
        let mut f = RenderFrame::default();
        build_frame_with(&scene(lay, Background::Game, Some(&sim), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        assert_eq!(f.quads, want.quads);
        assert_eq!(f.clips, want.clips);
    }

    #[test]
    fn teleport_tick_frame_is_the_frame_of_the_departure_place() {
        // End to end with real ticks: the frame after the tick in which telepctr reaches 32
        // shows the maze and every world object (relplot/plotdying) exactly like the same
        // tick of a twin game without the teleport; only the sunk player sprite differs.
        use crate::sim::game::GameStart;
        use crate::sim::TickInput;
        let tk = TestKit::get();
        let game = || {
            let mut s = crate::sim::testutil::sim();
            s.start_game(GameStart { mentalzone: 1, ..GameStart::default() }).unwrap();
            for t in 0..40 {
                let held = if t % 20 < 12 { crate::input::HELD_RIGHT | crate::input::HELD_FIRE } else { 0 };
                s.tick(TickInput { held, cheat_keys: 0 });
            }
            s
        };
        let (mut a, mut b) = (game(), game());
        assert_eq!((a.telepctr, a.xpos, a.ypos), (b.telepctr, b.xpos, b.ypos));
        a.telepctr = 31;
        a.telepxpos = a.xpos + (300 << 8);
        a.telepypos = a.ypos - (100 << 8);
        a.tick(TickInput::default());
        b.tick(TickInput::default());
        assert_eq!(a.telepctr, 32);
        assert_ne!(a.camera(), b.camera());
        let texts = TextTable::default();
        let lay = Layout::original();
        let frame = |s: &Sim| {
            let mut f = RenderFrame::default();
            build_frame_with(&scene(lay, Background::Game, Some(s), &texts, &[]), &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
            f
        };
        let (fa, fb) = (frame(&a), frame(&b));
        let not_player = |f: &RenderFrame| f.quads.iter().filter(|q| q.clip != CLIP_PLAYER_FLOOR).copied().collect::<Vec<_>>();
        let world = not_player(&fa);
        assert!(world.iter().filter(|q| q.clip == CLIP_GAME).count() > 50);
        assert_eq!(world, not_player(&fb));
        assert_eq!(fa.clips, fb.clips);
    }

    #[test]
    fn menu_frame_has_highlight_under_texts() {
        let tk = TestKit::get();
        let texts = TextTable::default();
        let lines = [MenuLine { text: TextRef::Raw("1. Ego".into()), x: 64.0, y: 96.0, align: Align::Orig, prefix: None }];
        let lay = Layout::new(320.0, 256.0);
        let mut sc = scene(lay, Background::Chat, None, &texts, &lines);
        sc.highlight = Some([40.0, 88.0, 280.0, 104.0]);
        let mut f = RenderFrame::default();
        build_frame_with(&sc, &tk.atlas, &tk.strings, &tk.glyphs, &mut f);
        let hi = f.quads.iter().position(|q| q.color == HIGHLIGHT).unwrap();
        let first_glyph = f.quads.iter().position(|q| q.clip == CLIP_CHAT && q.color == WHITE).unwrap();
        assert!(hi < first_glyph);
        let h = f.quads[hi];
        assert_eq!((h.x, h.y, h.w, h.h, h.clip), (40.0, 88.0, 240.0, 16.0, CLIP_CHAT));
        // "1. Ego": 6 glyphs ('1' '.' 'E' 'G' 'O' with a space)
        assert_eq!(f.quads.len() - first_glyph, 5);
    }
}
