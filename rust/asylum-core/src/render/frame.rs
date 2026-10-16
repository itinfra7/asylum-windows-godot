//! Screen compositions: GameScreen / ChatScreen for any content size (docs §4), and the
//! HUD (bonus box, strength bar, lives, score — vdu.c plotbonus/showstrength/showlives/
//! showscore with positions shifted by `lay.hx` and `lay.hy`).
//!
//! HUD constants (vdu.c vduread): scorex = 160-(16*7)/2 = 104, scorey = 220; livesx 44,
//! livesy 234; strength at (108, 239) size 108x6; bonus box centre (290, 232), bonush 20.
//!
//! Compositions for content larger than 320x256 (`hx = (cw-320)/2`, `hy = ch-256`); at
//! 320x256 every gap is empty and the pieces reassemble the original image 1:1.
//! * GameScreen frame (src rows 0..208): src x[0,64) → 0, src x[64,256) → hx+64, src
//!   x[256,320) → cw-64, the two gaps repeat src column 100; src rows [0,150) → 0 and
//!   [150,208) → 150+hy, the gap repeats src row 150 (a plain stretch of the side borders,
//!   below the logo and between the bolts). HUD panel (src rows 208..256, at 208+hy):
//!   src x[8,312) → hx+8, src x[0,8) → 0, src x[312,320) → cw-8; the gaps are filled with
//!   the mirrored tiling of src x[105,212) rows 208..235 (rows 236..255 mirror rows
//!   235..216), period 214, phase anchored at hx+8.
//! * ChatScreen: columns src x[0,160) → 0, src x[160,320) → cw-160, the gap filled with the
//!   mirrored tiling of src x[80,160) (period 160, anchored at x = 160); rows likewise: src
//!   y[0,128) → 0, src y[128,256) → ch-128, the gap the mirrored tiling of src y[64,128)
//!   anchored at y = 128.
//!
//! Mirrored tiling of a strip `[s0, s0+len)` anchored at `a` (period `2*len`): the strip is
//! reflected about `a` with its last column `s0+len-1` next to the anchor on both sides
//! (`t = (x - a) mod 2len`: t < len shows `s0+len-1-t`, else `s0+t-len`). For the chat
//! screen this continues the left half seamlessly (src 159 | 159); for the GameScreen band
//! the join with the original art at hx+8 is seamless too, while the other reflection
//! column is src 105 (src 104 carries a light vertical crack and is left out of the strip).

use crate::atlas::{AtlasRect, Bank, Img};
use crate::layout::Layout;
use crate::render::{draw_framectr, sprite_quad, Quad, RenderCtx, TexSource, BLACK, CLIP_NONE, WHITE};
use crate::sim::{Sim, BONUSLOW, STRENGTHMAX};

pub const SCOREX: i32 = 160 - (16 * 7) / 2;
pub const SCOREY: i32 = 220;
pub const LIVESX: i32 = 44;
pub const LIVESY: i32 = 234;
pub const STRENGTHX: i32 = 108;
pub const STRENGTHY: i32 = 239;
pub const STRENGTHW: i32 = STRENGTHMAX >> 8;
pub const STRENGTHH: i32 = 6;
pub const BONUSX: i32 = 290;
pub const BONUSY: i32 = 232;
pub const BONUSH: i32 = 20;
const CHARWIDTH: i32 = 16;
/// GameScreen frame row where the vertical stretch is inserted (and which it repeats).
pub const FRAME_SPLIT: i32 = 150;

/// Append the GameScreen composition quads (clip 0).
pub fn emit_game_screen(lay: &Layout, ctx: &RenderCtx, out: &mut Vec<Quad>) {
    game_screen_quads(lay, &*ctx.atlas, out);
}

/// Append the ChatScreen composition quads (clip 0).
pub fn emit_chat_screen(lay: &Layout, ctx: &RenderCtx, out: &mut Vec<Quad>) {
    chat_screen_quads(lay, &*ctx.atlas, out);
}

/// Append HUD quads. `bonus_clip` is the clip index of the bonus box.
pub fn emit_hud(sim: &Sim, lay: &Layout, ctx: &RenderCtx, bonus_clip: u16, out: &mut Vec<Quad>) {
    hud_quads(sim, lay, &*ctx.atlas, bonus_clip, out);
}

/// Quad showing texels `u, v` (+`uw`, `vh`, relative to image `r`, may be negative =
/// mirrored) at content rectangle (x, y, w, h).
#[allow(clippy::too_many_arguments)]
fn img_quad(lay: &Layout, r: AtlasRect, u: f32, v: f32, uw: f32, vh: f32, x: i32, y: i32, w: i32, h: i32) -> Quad {
    Quad {
        x: lay.vx(x as f32),
        y: lay.vy(y as f32),
        w: w as f32,
        h: h as f32,
        u: r.u as f32 + u,
        v: r.v as f32 + v,
        uw,
        vh,
        dx: 0.0,
        dy: 0.0,
        color: WHITE,
        clip: CLIP_NONE,
    }
}

/// 1:1 copy of src rect (sx, sy, w, h) to content (x, y).
#[allow(clippy::too_many_arguments)]
fn blit(lay: &Layout, r: AtlasRect, sx: i32, sy: i32, w: i32, h: i32, x: i32, y: i32, out: &mut Vec<Quad>) {
    if w > 0 && h > 0 {
        out.push(img_quad(lay, r, sx as f32, sy as f32, w as f32, h as f32, x, y, w, h));
    }
}

/// Runs of the mirrored tiling of strip `[s0, s0+len)` over dst `[g0, g1)`, reflected
/// about `anchor` with column `s0+len-1` next to it: calls `f(dst_x, width, u, uw)` (u/uw
/// relative to the image; a negative `uw` samples right to left).
fn mirror_tiling(g0: i32, g1: i32, anchor: i32, s0: i32, len: i32, mut f: impl FnMut(i32, i32, f32, f32)) {
    let period = 2 * len;
    let mut x = g0;
    while x < g1 {
        let t = (x - anchor).rem_euclid(period);
        if t < len {
            // walking away from the anchor: columns s0+len-1-t, decreasing
            let n = (len - t).min(g1 - x);
            f(x, n, (s0 + len - t) as f32, -(n as f32));
            x += n;
        } else {
            // walking back towards the next reflection: columns s0+t-len, increasing
            let n = (period - t).min(g1 - x);
            f(x, n, (s0 + t - len) as f32, n as f32);
            x += n;
        }
    }
}

/// [`emit_game_screen`] on any texture source.
pub(crate) fn game_screen_quads(lay: &Layout, tex: &dyn TexSource, out: &mut Vec<Quad>) {
    let r = tex.img(Img::GameScreen);
    if r.w == 0 || r.h == 0 {
        return;
    }
    let (cw, hx, hy) = (lay.cw, lay.hx, lay.hy);
    // frame: column pieces (src x, width, dst x); the gaps repeat src column 100 (its
    // centre half is sampled), the vertical gap repeats src row FRAME_SPLIT
    let cols = [(0, 64, 0), (64, 192, hx + 64), (256, 64, cw - 64)];
    let gaps = [(64, hx + 64), (hx + 256, cw - 64)];
    let split = FRAME_SPLIT;
    for (sy, h, dy) in [(0, split, 0), (split, 208 - split, split + hy)] {
        for (sx, w, dx) in cols {
            blit(lay, r, sx, sy, w, h, dx, dy, out);
        }
        for (g0, g1) in gaps {
            if g1 > g0 {
                out.push(img_quad(lay, r, 100.25, sy as f32, 0.5, h as f32, g0, dy, g1 - g0, h));
            }
        }
    }
    if hy > 0 {
        let v = split as f32 + 0.25;
        for (sx, w, dx) in cols {
            out.push(img_quad(lay, r, sx as f32, v, w as f32, 0.5, dx, split, w, hy));
        }
        for (g0, g1) in gaps {
            if g1 > g0 {
                out.push(img_quad(lay, r, 100.25, v, 0.5, 0.5, g0, split, g1 - g0, hy));
            }
        }
    }
    // HUD panel at the bottom
    let y0 = 208 + hy;
    blit(lay, r, 8, 208, 304, 48, hx + 8, y0, out);
    blit(lay, r, 0, 208, 8, 48, 0, y0, out);
    blit(lay, r, 312, 208, 8, 48, cw - 8, y0, out);
    for (g0, g1) in [(8, hx + 8), (hx + 312, cw - 8)] {
        mirror_tiling(g0, g1, hx + 8, 105, 107, |x, w, u, uw| {
            out.push(img_quad(lay, r, u, 208.0, uw, 28.0, x, y0, w, 28));
            // rows 236..255 show rows 235..216
            out.push(img_quad(lay, r, u, 236.0, uw, -20.0, x, y0 + 28, w, 20));
        });
    }
}

/// Pieces of one ChatScreen axis of length `len`: the first and last `half` source pixels
/// at both ends, the gap filled with the mirrored tiling of `[s0, s0+slen)` anchored at
/// `half`: (dst, size, src, src size; negative = mirrored).
fn chat_axis(len: i32, half: i32, s0: i32, slen: i32) -> Vec<(i32, i32, f32, f32)> {
    let mut v = vec![(0, half, 0.0, half as f32)];
    mirror_tiling(half, len - half, half, s0, slen, |x, w, u, uw| v.push((x, w, u, uw)));
    v.push((len - half, half, half as f32, half as f32));
    v
}

/// [`emit_chat_screen`] on any texture source.
pub(crate) fn chat_screen_quads(lay: &Layout, tex: &dyn TexSource, out: &mut Vec<Quad>) {
    let r = tex.img(Img::ChatScreen);
    if r.w == 0 || r.h == 0 {
        return;
    }
    let cols = chat_axis(lay.cw, 160, 80, 80);
    for (y, h, v, vh) in chat_axis(lay.ch, 128, 64, 64) {
        for &(x, w, u, uw) in &cols {
            out.push(img_quad(lay, r, u, v, uw, vh, x, y, w, h));
        }
    }
}

/// [`emit_hud`] on any texture source.
pub(crate) fn hud_quads(sim: &Sim, lay: &Layout, tex: &dyn TexSource, bonus_clip: u16, out: &mut Vec<Quad>) {
    let (hx, hy) = (lay.hx, lay.hy);
    let chars = |i: i32| tex.spr(Bank::Chars, i as usize);

    // plotbonus(bonusctr, bonusreplot): clear the box, then up to three block sprites
    let (bx, by) = (BONUSX + hx, BONUSY + hy);
    let white = tex.img(Img::White);
    if white.w > 0 {
        let mut q = img_quad(lay, white, 0.0, 0.0, white.w as f32, white.h as f32, bx - 16 / 2, by - BONUSH, 16, 2 * BONUSH);
        q.color = BLACK;
        q.clip = bonus_clip;
        out.push(q);
    }
    // HUD inputs as the C drew them (before player_dead/zonerestart of the tick): Sim::draw
    let hud = &sim.draw;
    let blocks = sim.blocks_bank();
    let bonusctr = hud.bonusctr as i32;
    let bonusreplot = hud.bonusreplot as i32;
    let r3 = (if bonusreplot > 7 { bonusreplot - 8 } else { 0 }) * BONUSH / 20;
    let lim = BONUSLOW as i32 + 12;
    let mut plot = |n: i32, y: i32| {
        let n = if n > lim { 0 } else { n };
        if let Some(s) = tex.spr(blocks, n as usize) {
            out.push(sprite_quad(lay, &s, bx, y, bonus_clip, (0.0, 0.0)));
        }
    };
    plot(bonusctr + 16, by + r3);
    if bonusctr != 0 {
        plot(bonusctr + 15, by - BONUSH + r3);
    }
    //nosecond:
    if bonusctr != 12 {
        plot(bonusctr + 17, by + BONUSH + r3);
    }

    // showstrength(plstrength): grey full width, then red proportional
    let strength = hud.plstrength.clamp(0, STRENGTHMAX);
    let row = (draw_framectr(sim) & 0x1f) as f32;
    let (sx, sy) = (STRENGTHX + hx, STRENGTHY + hy);
    let grey = tex.img(Img::StrengthGrey);
    if grey.w > 0 {
        out.push(img_quad(lay, grey, 0.0, row, STRENGTHW as f32, STRENGTHH as f32, sx, sy, STRENGTHW, STRENGTHH));
    }
    let red_w = (STRENGTHW * strength) / STRENGTHMAX;
    let red = tex.img(Img::StrengthRed);
    if red.w > 0 && red_w > 0 {
        out.push(img_quad(lay, red, 0.0, row, red_w as f32, STRENGTHH as f32, sx, sy, red_w, STRENGTHH));
    }

    // showlives(lives)
    if let Some(s) = chars(hud.lives & 7) {
        out.push(sprite_quad(lay, &s, LIVESX + hx, LIVESY + hy, CLIP_NONE, (0.0, 0.0)));
    }

    // showscore(plscore): leading zeros of the 10-million and million digits are skipped
    let p = &sim.plscore;
    let (mut x, y) = (SCOREX + hx, SCOREY + hy);
    let mut i = 0;
    if p[0] == 0 {
        x += CHARWIDTH / 2;
        i = 1;
        if p[1] == 0 {
            x += CHARWIDTH / 2;
            i = 2;
        }
    }
    //score10mil:
    for &d in &p[i..8] {
        if d <= 10 {
            if let Some(s) = chars(d as i32) {
                out.push(sprite_quad(lay, &s, x, y, CLIP_NONE, (0.0, 0.0)));
            }
        }
        x += CHARWIDTH;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::testkit::*;
    use crate::render::{finalize, RenderFrame, CLIP_BONUS};

    fn compose(lay: &Layout, chat: bool) -> Vec<[u8; 4]> {
        let tk = TestKit::get();
        let mut f = RenderFrame { clips: vec![[0.0, 0.0, lay.view_w, lay.view_h]], ..RenderFrame::default() };
        if chat {
            chat_screen_quads(lay, &tk.atlas, &mut f.quads);
        } else {
            game_screen_quads(lay, &tk.atlas, &mut f.quads);
        }
        // each pixel is covered exactly once
        let (w, h) = (lay.view_w as usize, lay.view_h as usize);
        let mut cover = vec![0u8; w * h];
        for q in &f.quads {
            for y in q.y as usize..(q.y + q.h) as usize {
                for x in q.x as usize..(q.x + q.w) as usize {
                    cover[y * w + x] += 1;
                }
            }
        }
        for y in 0..lay.ch as usize {
            for x in 0..lay.cw as usize {
                assert_eq!(cover[(y + lay.oy as usize) * w + x + lay.ox as usize], 1, "pixel ({x},{y}) cw={}", lay.cw);
            }
        }
        let mut buf = Vec::new();
        let n = finalize(&f, 1.0, &mut buf);
        assert_eq!(n, f.quads.len());
        tk.atlas.raster(&buf, w, h)
    }

    fn content_px(img: &[[u8; 4]], lay: &Layout, x: i32, y: i32) -> [u8; 4] {
        img[(y as usize + lay.oy as usize) * lay.view_w as usize + (x as usize + lay.ox as usize)]
    }

    #[test]
    fn original_width_is_pixel_identical() {
        let tk = TestKit::get();
        for (chat, src) in [(false, &tk.assets.game_screen), (true, &tk.assets.chat_screen)] {
            for lay in [Layout::original()] {
                let img = compose(&lay, chat);
                for y in 0..256 {
                    for x in 0..320 {
                        let c = crate::palette::rgba(src.pix[(y * 320 + x) as usize]);
                        assert_eq!(content_px(&img, &lay, x, y), c, "chat={chat} ({x},{y})");
                    }
                }
            }
        }
    }

    #[test]
    fn extended_game_screen_composition() {
        let tk = TestKit::get();
        let src = |x: i32, y: i32| crate::palette::rgba(tk.assets.game_screen.pix[(y * 320 + x) as usize]);
        let sizes = [(322.0, 256.0), (455.0, 256.0), (607.0, 256.0), (1000.0, 256.0), (320.0, 300.0), (683.0, 512.0), (1820.0, 768.0), (2731.0, 768.0)];
        for (vw, vh) in sizes {
            let lay = Layout::new(vw, vh);
            let (cw, hx, hy) = (lay.cw, lay.hx, lay.hy);
            let img = compose(&lay, false);
            let px = |x, y| content_px(&img, &lay, x, y);
            for y in (0..lay.ch).step_by(if hy > 100 { 7 } else { 1 }) {
                if y < 208 + hy {
                    // frame: source row of this content row
                    let sy = if y < FRAME_SPLIT { y } else if y < FRAME_SPLIT + hy { FRAME_SPLIT } else { y - hy };
                    for x in 0..64 {
                        assert_eq!(px(x, y), src(x, sy));
                        assert_eq!(px(cw - 64 + x, y), src(256 + x, sy));
                    }
                    for x in 64..256 {
                        assert_eq!(px(hx + x, y), src(x, sy));
                    }
                    for x in (64..hx + 64).chain(hx + 256..cw - 64) {
                        assert_eq!(px(x, y), src(100, sy), "column fill ({x},{y})");
                    }
                } else {
                    let y0 = y - hy;
                    for x in 0..8 {
                        assert_eq!(px(x, y), src(x, y0));
                        assert_eq!(px(cw - 8 + x, y), src(312 + x, y0));
                    }
                    for x in 8..312 {
                        assert_eq!(px(hx + x, y), src(x, y0));
                    }
                    let sy = if y0 < 236 { y0 } else { 235 - (y0 - 236) };
                    for x in (8..hx + 8).chain(hx + 312..cw - 8) {
                        let t = (x - (hx + 8)).rem_euclid(214);
                        let sx = if t < 107 { 211 - t } else { 105 + (t - 107) };
                        assert_eq!(px(x, y), src(sx, sy), "band ({x},{y}) cw={cw}");
                    }
                }
            }
        }
    }

    #[test]
    fn extended_chat_screen_composition() {
        let tk = TestKit::get();
        let src = |x: i32, y: i32| crate::palette::rgba(tk.assets.chat_screen.pix[(y * 320 + x) as usize]);
        // source pixel of a content pixel along one axis
        let map = |p: i32, len: i32, half: i32, s0: i32, slen: i32| {
            if p < half {
                p
            } else if p >= len - half {
                p - (len - 2 * half)
            } else {
                let t = (p - half).rem_euclid(2 * slen);
                if t < slen {
                    s0 + slen - 1 - t
                } else {
                    s0 + (t - slen)
                }
            }
        };
        for (vw, vh) in [(330.0, 256.0), (455.0, 256.0), (641.0, 256.0), (341.0, 300.0), (910.0, 512.0), (1820.0, 768.0)] {
            let lay = Layout::new(vw, vh);
            let (cw, ch) = (lay.cw, lay.ch);
            let img = compose(&lay, true);
            let px = |x, y| content_px(&img, &lay, x, y);
            for y in (0..ch).step_by(3) {
                for x in (0..cw).step_by(2) {
                    let (sx, sy) = (map(x, cw, 160, 80, 80), map(y, ch, 128, 64, 64));
                    assert_eq!(px(x, y), src(sx, sy), "({x},{y}) {cw}x{ch}");
                }
            }
            // seamless continuation of the left and top halves
            assert_eq!(px(160, 50), src(159, 50));
            assert_eq!(px(50, 128), src(50, 127));
        }
    }

    #[test]
    fn mirror_tiling_runs() {
        let mut runs = vec![];
        mirror_tiling(0, 30, 10, 100, 8, |x, w, u, uw| runs.push((x, w, u, uw)));
        // x=0: t = (0-10) mod 16 = 6 → columns 101, 100 (right to left from u=102)
        assert_eq!(runs[0], (0, 2, 102.0, -2.0));
        // then 100..107 forward, reflection at the anchor (107 | 107), back down to 100
        assert_eq!(runs[1], (2, 8, 100.0, 8.0));
        assert_eq!(runs[2], (10, 8, 108.0, -8.0));
        assert_eq!(runs[3], (18, 8, 100.0, 8.0));
        assert_eq!(runs.iter().map(|r| r.1).sum::<i32>(), 30);
        let mut runs = vec![];
        mirror_tiling(5, 5, 0, 0, 8, |x, w, u, uw| runs.push((x, w, u, uw)));
        assert!(runs.is_empty());
    }

    #[test]
    fn hud_matches_vdu_positions() {
        let tk = TestKit::get();
        let mut sim = crate::sim::testutil::sim();
        sim.plscore = [0, 0, 1, 2, 3, 4, 5, 6];
        sim.lives = 2;
        sim.plstrength = STRENGTHMAX / 2;
        sim.framectr = 41; // drawn with the pre-increment value 40 → row 8
        sim.bonusctr = 3;
        sim.bonusreplot = 18; // r3 = 10
        sim.sync_draw(); // the HUD is drawn from the draw snapshot
        for lay in [Layout::original(), Layout::new(454.0, 300.0), Layout::new(1365.0, 768.0)] {
            let (hx, hy) = (lay.hx, lay.hy);
            let mut q = Vec::new();
            hud_quads(&sim, &lay, &tk.atlas, CLIP_BONUS, &mut q);
            let at = |s: crate::atlas::AtlasSprite, x: i32, y: i32| (lay.vx((x - s.xc as i32) as f32), lay.vy((y - s.yc as i32) as f32));
            // bonus box: black fill, block 19 at centre+10, block 18 above, block 20 below
            assert_eq!((q[0].x, q[0].y, q[0].w, q[0].h, q[0].color, q[0].clip), (lay.vx((282 + hx) as f32), lay.vy((212 + hy) as f32), 16.0, 40.0, BLACK, CLIP_BONUS));
            let blk = |n| tk.atlas.spr(Bank::Blocks(0), n).unwrap();
            assert_eq!((q[1].x, q[1].y), at(blk(19), 290 + hx, 242 + hy));
            assert_eq!((q[2].x, q[2].y), at(blk(18), 290 + hx, 222 + hy));
            assert_eq!((q[3].x, q[3].y), at(blk(20), 290 + hx, 262 + hy));
            assert!(q[1..4].iter().all(|q| q.clip == CLIP_BONUS));
            // strength: grey 108 wide, red 54 wide, texture row 8
            let grey = tk.atlas.img(Img::StrengthGrey);
            assert_eq!((q[4].x, q[4].y, q[4].w, q[4].h), (lay.vx((108 + hx) as f32), lay.vy((239 + hy) as f32), 108.0, 6.0));
            assert_eq!((q[4].u, q[4].v, q[4].vh), (grey.u as f32, grey.v as f32 + 8.0, 6.0));
            assert_eq!((q[5].w, q[5].uw), (54.0, 54.0));
            // lives glyph '2'
            let c = |n| tk.atlas.spr(Bank::Chars, n).unwrap();
            assert_eq!((q[6].x, q[6].y, q[6].u), { let (x, y) = at(c(2), 44 + hx, 234 + hy); (x, y, c(2).u as f32) });
            // score 123456 with two leading zeros skipped: starts at 104 + 16
            assert_eq!(q.len(), 7 + 6);
            for (i, d) in (1..=6).enumerate() {
                let (x, y) = at(c(d), 104 + hx + 16 + 16 * i as i32, 220 + hy);
                assert_eq!((q[7 + i].x, q[7 + i].y, q[7 + i].u), (x, y, c(d).u as f32));
            }
        }
        // bonus box edge cases of plotbonus: counter 0 → no upper sprite; 12 → no lower;
        // indices past _bonuslow+12 draw block 0
        sim.bonusctr = 0;
        sim.bonusreplot = 4;
        sim.sync_draw();
        let mut q = Vec::new();
        hud_quads(&sim, &Layout::original(), &tk.atlas, CLIP_BONUS, &mut q);
        let n0 = q.iter().filter(|q| q.clip == CLIP_BONUS).count();
        assert_eq!(n0, 1 + 2);
        sim.bonusctr = 12;
        sim.sync_draw();
        q.clear();
        hud_quads(&sim, &Layout::original(), &tk.atlas, CLIP_BONUS, &mut q);
        let blk = |n| tk.atlas.spr(Bank::Blocks(0), n).unwrap().u as f32;
        let bonus: Vec<f32> = q.iter().filter(|q| q.clip == CLIP_BONUS).map(|q| q.u).collect();
        assert_eq!(bonus[1..], [blk(28), blk(27)]);
        sim.bonusctr = 13;
        sim.sync_draw();
        q.clear();
        hud_quads(&sim, &Layout::original(), &tk.atlas, CLIP_BONUS, &mut q);
        let bonus: Vec<f32> = q.iter().filter(|q| q.clip == CLIP_BONUS).map(|q| q.u).collect();
        assert_eq!(bonus[1..], [blk(0), blk(28), blk(0)]);
        // strength clamps and score with a leading digit
        sim.plstrength = -5;
        sim.plscore = [1, 0, 0, 0, 0, 0, 0, 0];
        sim.sync_draw();
        q.clear();
        hud_quads(&sim, &Layout::original(), &tk.atlas, CLIP_BONUS, &mut q);
        assert!(!q.iter().any(|q| q.w == 0.0));
        let score: Vec<&Quad> = q.iter().filter(|q| q.y == 220.0 - 7.0).collect();
        assert_eq!(score.len(), 8);
        assert_eq!(score[0].x, (104 - 7) as f32);
    }
}

#[cfg(test)]
mod dump {
    use super::*;
    use crate::render::testkit::*;
    use crate::render::{finalize, RenderFrame};

    /// `ASYLUM_RENDER_DUMP=<dir> cargo test -p asylum-core render::frame::dump -- --ignored`
    #[test]
    #[ignore]
    fn dump_compositions() {
        let Ok(dir) = std::env::var("ASYLUM_RENDER_DUMP") else { return };
        let tk = TestKit::get();
        for (vw, vh) in [(320.0, 256.0), (455.0, 256.0), (1820.0, 768.0)] {
            let lay = Layout::new(vw, vh);
            for chat in [false, true] {
                let mut f = RenderFrame { clips: vec![[0.0, 0.0, lay.view_w, lay.view_h]], ..RenderFrame::default() };
                if chat {
                    chat_screen_quads(&lay, &tk.atlas, &mut f.quads);
                } else {
                    game_screen_quads(&lay, &tk.atlas, &mut f.quads);
                }
                let mut buf = Vec::new();
                finalize(&f, 1.0, &mut buf);
                let img = tk.atlas.raster(&buf, lay.view_w as usize, lay.view_h as usize);
                let name = format!("{}_{}x{}.ppm", if chat { "chat" } else { "game" }, lay.cw, lay.ch);
                write_ppm(&std::path::Path::new(&dir).join(name), &img, lay.view_w as usize, lay.view_h as usize);
            }
        }
    }
}
