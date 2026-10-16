//! World rendering from the board (vdu.c `mazeplot`, `backdrop`, maze.c `draw_block`).
//!
//! Exact formulas (camx = xpos>>8, camy = ypos>>8 at mazeplot time = `Sim::draw.cam`,
//! gcx/gcy = game window centre):
//! * foreground block at cell (c, r): sprite hot spot at (gcx - 1 + 16c - camx,
//!   gcy + 7 + 16r - camy).
//! * midground (blocks 8..11, layer 1): with `half_w = gamew/2`, `half_h = gameh/2`,
//!   `r0 = camx - half_w`, `r1 = camy - half_h`, `c0 = r0 >> 4`, `rr0 = r1 >> 4`,
//!   r4 = column - c0, r5 = row - rr0, `kx = half_w/16 + 1`, `ky = half_h/16 + 1`:
//!   x = gamex - 1 + kx + 15*r4 - (r0 & 15) , y = gamey + 7 + ky + 15*r5 - (r1 & 15)
//!   (original: `(r4*15) + xblocks/2 + 1 + r8` with r8 = gamex-1-(r0&15); likewise y with
//!   yblocks/2+1 and r9 = gamey+7-(r1&15); kx/ky are those block counts of the actual window,
//!   10 and 7 for 288x192, so the midground keeps its place next to the walls at any size).
//! * draw_block: skip 0 and 8..11 in layer 0; neurons 40..47 animate with
//!   `40 + ((framectr & 28) >> 2)`; blocks <= 175: crumbles 160..175 use `b & !8`; gas
//!   54/55 flicker `b ^= r>>2`, electricity 56..63 `b ^= r>>1` with r = 3-bit hash of
//!   (cell index, framectr) (replaces `random()&7`); weapons 97..102 draw as 97.
//!   Sleeping-alien markers 240..=253 are never drawn (in the original they can never be
//!   inside the visible window; Psyche even has a sprite at 240 that must stay hidden).
//! * backdrop: tile 48x32 phase `bx = (r3 - 44) mod 48`, `by = (camy - (ypos >> 10)) & 31` with
//!   `r3 = camx - (xpos >> 10) + 2304`; tiles start at (gamex - bx, gamey - by), step
//!   48x32, clipped to the game window.
//!
//! With interpolation the camera is fractional: the frame builder emits blocks for the
//! integer camera of the tick with `dx,dy = cam_tick - cam_prev` deltas (see render/mod.rs).
//! The two parallax layers get their own exact motion for the same camera step (the
//! backdrop moves by `c - (c >> 2)`, the midground by `15*(r >> 4) + (r & 15)`).
//!
//! Cell ranges (all clipped to the game window): the midground uses the C start (r4, r5 >= 0,
//! i.e. columns >= c0 and rows >= rr0; row/column -1 would be partly visible at the top/left
//! edge, where the C shows only the backdrop) and gets a margin of 2 cells only on the right
//! and bottom. The foreground is drawn with a margin of 2 cells on every side (its rows and
//! columns before rr0/c0 are never visible at any position). The ranges cover the visible
//! cells of any window size, and the right/bottom margins keep interpolated positions from
//! opening gaps at those edges.

use crate::atlas::Img;
use crate::layout::Layout;
use crate::render::{draw_framectr, sprite_quad, Quad, RenderCtx, TexSource};
use crate::sim::{Sim, ANIMHIGHLIM, ANIMLOWLIM, CRUMBLELOWLIM, CRUMBLESTANDHIGHLIM, GASHIGHLIM, GASLOWLIM, NEURONLOWLIM, WEAPLOWLIM};

/// Append backdrop + midground + foreground block quads for the sim's current board.
pub fn emit_world(sim: &Sim, lay: &Layout, ctx: &RenderCtx, game_clip: u16, delta: (f32, f32), out: &mut Vec<Quad>) {
    world_quads(sim, lay, &*ctx.atlas, game_clip, (delta.0.round() as i32, delta.1.round() as i32), out);
}

/// Margin (cells) drawn around the visible window.
const MARGIN: i32 = 2;

/// Backdrop tile size (vdu.c backw/backh at scale 1).
const BACKW: i32 = 48;
const BACKH: i32 = 32;

/// `c - (c >> 2)`: camera position of the backdrop layer (`(xpos>>8) - (xpos>>10)`).
fn back_par(c: i32) -> i32 {
    c.wrapping_sub(c >> 2)
}

/// Screen offset of the midground layer for `r = cam - half` (15 px per 16-px cell).
fn mid_par(r: i32) -> i32 {
    15i32.wrapping_mul(r >> 4).wrapping_add(r & 15)
}

/// 3-bit hash of (cell, framectr) replacing draw_block's `random()&7` (docs §5).
pub(crate) fn anim_hash(cell: i32, framectr: i32) -> i32 {
    let mut h = (cell as u32).wrapping_mul(0x9E37_79B1) ^ (framectr as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 29) as i32
}

/// maze.c `draw_block`: the sprite index to plot for board value `block`, or None.
pub(crate) fn draw_block(block: u8, layer: i32, framectr: i32, cell: i32) -> Option<u8> {
    if layer != 0 {
        return (8..12).contains(&block).then_some(block);
    }
    if block == 0 || (8..12).contains(&block) {
        return None;
    }
    // porting note #10: sleeping-alien markers are never drawn
    if (240..=253).contains(&block) {
        return None;
    }
    let mut b = block as i32;
    if (b & !7) == NEURONLOWLIM as i32 {
        b = NEURONLOWLIM as i32 + ((framectr & (7 << 2)) >> 2);
    }
    if b <= CRUMBLESTANDHIGHLIM as i32 {
        if b >= CRUMBLELOWLIM as i32 {
            b &= !8; //alter for crumbles
        }
        if (ANIMLOWLIM as i32..=ANIMHIGHLIM as i32).contains(&b) {
            let r1 = anim_hash(cell, framectr) & 7;
            if b == GASLOWLIM as i32 || b == GASHIGHLIM as i32 {
                b ^= r1 >> 2;
            } else {
                b ^= r1 >> 1;
            }
        }
    }
    //noanimate:
    if (WEAPLOWLIM as i32 + 1..=WEAPLOWLIM as i32 + 6).contains(&b) {
        b = WEAPLOWLIM as i32 + 1;
    }
    Some(b as u8)
}

/// [`emit_world`] on any texture source; `dc` = camera delta (prev - cur, 0 when snapped).
pub(crate) fn world_quads(sim: &Sim, lay: &Layout, tex: &dyn TexSource, clip: u16, dc: (i32, i32), out: &mut Vec<Quad>) {
    // mazeplot(xpos, ypos) ran before a teleport/restart later in the tick: draw camera
    let (camx, camy) = sim.draw.cam;
    backdrop(sim, lay, tex, clip, camx, camy, dc, out);

    let bd = sim.bd();
    let bank = sim.blocks_bank();
    let framectr = draw_framectr(sim);
    let (half_w, half_h) = (lay.gamew / 2, lay.gameh / 2);
    let r0 = camx.wrapping_sub(half_w);
    let r1 = camy.wrapping_sub(half_h);
    let (c0, rr0) = (r0 >> 4, r1 >> 4);
    let plot = |layer: i32, row: i32, col: i32, x: i32, y: i32, d: (f32, f32), out: &mut Vec<Quad>| {
        if row < 0 || row >= bd.height || col < 0 || col >= bd.width {
            return;
        }
        let cell = sim.boardwidth.wrapping_mul(row).wrapping_add(col);
        if let Some(n) = draw_block(bd.get(cell), layer, framectr, cell) {
            if let Some(s) = tex.spr(bank, n as usize) {
                out.push(sprite_quad(lay, &s, x, y, clip, d));
            }
        }
    };

    // Draw midground elements (15/16 parallax)
    let md = (
        (mid_par(r0) - mid_par(r0.wrapping_sub(dc.0))) as f32,
        (mid_par(r1) - mid_par(r1.wrapping_sub(dc.1))) as f32,
    );
    // C start (rows >= rr0, columns >= c0): row/column -1 would reach up to 7/2 px into the
    // top/left of the window, where the C shows only the backdrop. A larger window moves the
    // layer right/down by its larger block counts; the rows/columns before then cover the
    // top/left strip.
    let (kx, ky) = (half_w / 16 + 1, half_h / 16 + 1);
    let sx = if kx > 10 { -((kx - 10) / 15 + 2) } else { 0 };
    let sy = if ky > 7 { -((ky - 7) / 15 + 2) } else { 0 };
    let (mx, my) = (lay.gamew / 15 + 2 + MARGIN, lay.gameh / 15 + 2 + MARGIN);
    for r5 in sy..=my {
        for r4 in sx..=mx {
            let x = lay.gamex - 1 + kx + 15 * r4 - (r0 & 15);
            let y = lay.gamey + 7 + ky + 15 * r5 - (r1 & 15);
            plot(1, rr0 + r5, c0 + r4, x, y, md, out);
        }
    }
    // Now foreground ones
    let fd = (dc.0 as f32, dc.1 as f32);
    let (fx, fy) = (lay.gamew / 16 + 2 + MARGIN, lay.gameh / 16 + 2 + MARGIN);
    for r5 in -MARGIN..=fy {
        for r4 in -MARGIN..=fx {
            let x = lay.gamex - 1 + 16 * r4 - (r0 & 15);
            let y = lay.gamey + 7 + 16 * r5 - (r1 & 15);
            plot(0, rr0 + r5, c0 + r4, x, y, fd, out);
        }
    }
}

/// vdu.c `backdrop(xpos, ypos)`: 48x32 tiles covering the game window.
#[allow(clippy::too_many_arguments)]
fn backdrop(sim: &Sim, lay: &Layout, tex: &dyn TexSource, clip: u16, camx: i32, camy: i32, dc: (i32, i32), out: &mut Vec<Quad>) {
    let r = tex.img(Img::Backdrop { zone: sim.backdrop.zone, neuron: sim.backdrop.neuron });
    if r.w == 0 || r.h == 0 {
        return;
    }
    let r3 = back_par(camx).wrapping_add(3072 - 768); // parallax
    let bx = (r3 - 44).rem_euclid(BACKW);
    let by = back_par(camy) & 0x1f; // parallax
    let d = (back_par(camx) - back_par(camx.wrapping_sub(dc.0)), back_par(camy) - back_par(camy.wrapping_sub(dc.1)));
    // extra tiles on the side an interpolation offset uncovers
    let x0 = lay.gamex - bx - if d.0 > 0 { BACKW } else { 0 };
    let y0 = lay.gamey - by - if d.1 > 0 { BACKH } else { 0 };
    let x1 = lay.gamex + lay.gamew + (-d.0).max(0);
    let y1 = lay.gamey + lay.gameh + (-d.1).max(0);
    let mut ty = y0;
    while ty < y1 {
        let mut tx = x0;
        while tx < x1 {
            out.push(Quad {
                x: lay.vx(tx as f32),
                y: lay.vy(ty as f32),
                w: BACKW as f32,
                h: BACKH as f32,
                u: r.u as f32,
                v: r.v as f32,
                uw: BACKW as f32,
                vh: BACKH as f32,
                dx: d.0 as f32,
                dy: d.1 as f32,
                color: crate::render::WHITE,
                clip,
            });
            tx += BACKW;
        }
        ty += BACKH;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::Bank;
    use crate::render::testkit::*;
    use crate::render::CLIP_GAME;

    /// Direct port of vdu.c mazeplot's loops (scale 1, 288x192 window): (layer, block, x, y).
    fn c_mazeplot(sim: &Sim, framectr: i32) -> Vec<(i32, u8, f64, f64)> {
        let (xblocks, yblocks, sprw, sprh, gamex, gamey) = (18, 12, 16, 16, 16, 8);
        let (xpos, ypos) = (sim.xpos, sim.ypos);
        let bd = sim.bd();
        let mut v = vec![];
        let r0 = (xpos - ((xblocks * 8) << 8)) >> 8;
        let r1 = (ypos - ((yblocks * 8) << 8)) >> 8;
        let r8 = ((15 - (r0 & 15) - 16) * sprw) / 16 + gamex;
        let r9 = ((15 - (r1 & 15) - 8) * sprh) / 16 + gamey;
        for layer in [1, 0] {
            let (mut r5, mut r7) = (0, r1 >> 4);
            if r7 < 0 {
                r5 = -r7;
                r7 = 0;
            }
            while r5 < yblocks + 2 && r7 < bd.height {
                let mut r4 = 0;
                let mut r6 = ((xpos >> 8) - (xblocks * 8)) >> 4;
                if r6 < 0 {
                    r4 = -r6;
                    r6 = 0;
                }
                while r4 < xblocks + 3 && r6 < bd.width {
                    let cell = sim.boardwidth * r7 + r6;
                    let b = bd.get(cell);
                    let (x, y) = if layer == 1 {
                        ((r4 * sprw) as f64 * 15.0 / 16.0 + (xblocks / 2 + 1 + r8) as f64, (r5 * sprh) as f64 * 15.0 / 16.0 + (yblocks / 2 + 1 + r9) as f64)
                    } else {
                        ((r4 * sprw + r8) as f64, (r5 * sprh + r9) as f64)
                    };
                    if let Some(n) = draw_block(b, layer, framectr, cell) {
                        v.push((layer, n, x, y));
                    }
                    r4 += 1;
                    r6 += 1;
                }
                r5 += 1;
                r7 += 1;
            }
        }
        v
    }

    fn test_sim(xpos: i32, ypos: i32) -> Sim {
        let mut sim = crate::sim::testutil::sim();
        sim.boardwidth = sim.bd().width;
        sim.xpos = xpos;
        sim.ypos = ypos;
        sim.sync_draw();
        sim.cam_prev = sim.draw.cam;
        sim
    }

    #[test]
    fn block_positions_match_c_mazeplot() {
        let tk = TestKit::get();
        // (xpos, ypos, extra midground block 9 at (row - rr0, column - c0))
        type Cam<'a> = (i32, i32, &'a [(i32, i32)]);
        let cams: [Cam; 6] = [
            (1000 << 8, 600 << 8, &[]),
            ((1237 << 8) + 77, (523 << 8) + 200, &[]),
            (150 << 8, 100 << 8, &[]),
            (60 << 8, 40 << 8, &[]),
            // (r1 & 15) = 0: row rr0-1 would reach 7 px into the top of the window; the C
            // midground loop starts at rr0 and never draws it
            (1000 << 8, 592 << 8, &[(-1, 8), (-1, 0)]),
            // (r0 & 15) = 0: column c0-1 would reach 2 px into the left edge; the C starts at c0
            (1008 << 8, 600 << 8, &[(5, -1), (0, -1)]),
        ];
        for (xpos, ypos, extra) in cams {
            let mut sim = test_sim(xpos, ypos);
            // sprinkle known blocks (fg and midground) around the camera
            let (cx, cy) = sim.camera();
            for (i, (dx, dy)) in [(0, 0), (-150, -100), (140, 90), (-140, 95), (147, -97), (3, 50)].iter().enumerate() {
                let cell = sim.boardwidth * ((cy + dy) >> 4) + ((cx + dx) >> 4);
                sim.bd_mut().set(cell, if i % 2 == 0 { 128 } else { 9 });
            }
            let lay = Layout::original();
            let (r0, r1) = (cx - lay.gamew / 2, cy - lay.gameh / 2);
            let s9 = tk.atlas.spr(Bank::Blocks(0), 9).unwrap();
            for &(dr, dcol) in extra {
                let cell = sim.boardwidth * ((r1 >> 4) + dr) + (r0 >> 4) + dcol;
                sim.bd_mut().set(cell, 9);
                // the cell would be visible if it were drawn with the midground formula
                let x = lay.gamex + 9 + 15 * dcol - (r0 & 15) - s9.xc as i32;
                let y = lay.gamey + 14 + 15 * dr - (r1 & 15) - s9.yc as i32;
                assert!(x < lay.gamex + lay.gamew && x + s9.w as i32 > lay.gamex && y < lay.gamey + lay.gameh && y + s9.h as i32 > lay.gamey);
            }
            let mut q = Vec::new();
            world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (0, 0), &mut q);
            let fc = draw_framectr(&sim);
            let c = c_mazeplot(&sim, fc);
            assert!(!c.is_empty());
            let blocks: Vec<&Quad> = q.iter().filter(|q| q.w != 48.0).collect();
            let [gx0, gy0, gx1, gy1] = lay.game_rect();
            // empty sprites are a no-op in the C as well
            let c: Vec<_> = c.into_iter().filter(|e| tk.atlas.spr(Bank::Blocks(0), e.1 as usize).is_some()).collect();
            for &(layer, n, x, y) in &c {
                let s = tk.atlas.spr(Bank::Blocks(0), n as usize).unwrap();
                // SDL path: top-left = (int)(x - xc)
                let (tx, ty) = ((x - s.xc as f64) as i32 as f32, (y - s.yc as f64) as i32 as f32);
                assert!(
                    blocks.iter().any(|q| q.x == tx && q.y == ty && q.u == s.u as f32 && q.v == s.v as f32),
                    "layer {layer} block {n} at ({x}, {y}) cam {:?}",
                    sim.camera()
                );
            }
            // every extra (margin) block lies outside the window
            let visible = |q: &&&Quad| q.x < gx1 && q.x + q.w > gx0 && q.y < gy1 && q.y + q.h > gy0;
            assert_eq!(blocks.iter().filter(visible).count(), c.iter().filter(|&&(_, n, x, y)| {
                let s = tk.atlas.spr(Bank::Blocks(0), n as usize).unwrap();
                let (tx, ty) = ((x - s.xc as f64) as i32 as f32, (y - s.yc as f64) as i32 as f32);
                tx < gx1 && tx + s.w as f32 > gx0 && ty < gy1 && ty + s.h as f32 > gy0
            }).count(), "visible blocks, cam {:?}", sim.camera());
        }
    }

    #[test]
    fn foreground_formula_and_larger_views() {
        let tk = TestKit::get();
        let mut sim = test_sim(2000 << 8, 1000 << 8);
        let (cx, cy) = sim.camera();
        let (col, row) = (cx / 16 + 5, cy / 16 - 3);
        let cell = sim.boardwidth * row + col;
        sim.bd_mut().set(cell, 128);
        let s = tk.atlas.spr(Bank::Blocks(0), 128).unwrap();
        for lay in [Layout::original(), Layout::new(454.0, 256.0), Layout::new(640.0, 300.0)] {
            let mut q = Vec::new();
            world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (0, 0), &mut q);
            let x = lay.gcx() - 1 + 16 * col - cx - s.xc as i32;
            let y = lay.gcy() + 7 + 16 * row - cy - s.yc as i32;
            assert!(q.iter().any(|q| q.x == lay.vx(x as f32) && q.y == lay.vy(y as f32) && q.u == s.u as f32), "{lay:?}");
        }
        // a block at the far left edge of a 640-wide window is drawn too
        let lay = Layout::new(640.0, 256.0);
        let col = (cx - lay.gamew / 2) / 16 + 1;
        let cell = sim.boardwidth * (cy / 16) + col;
        sim.bd_mut().set(cell, 129);
        let s = tk.atlas.spr(Bank::Blocks(0), 129).unwrap();
        let mut q = Vec::new();
        world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (0, 0), &mut q);
        assert!(q.iter().any(|q| q.u == s.u as f32 && q.v == s.v as f32 && q.x < lay.vx(lay.gamex as f32 + 16.0)));
    }

    #[test]
    fn midground_stays_next_to_its_cell_at_any_size() {
        let tk = TestKit::get();
        let s9 = tk.atlas.spr(Bank::Blocks(0), 9).unwrap();
        let s128 = tk.atlas.spr(Bank::Blocks(0), 128).unwrap();
        for lay in [Layout::original(), Layout::new(455.0, 256.0), Layout::new(607.0, 256.0), Layout::new(1365.0, 768.0), Layout::new(1820.0, 768.0)] {
            for (xpos, ypos) in [(2000 << 8, 1000 << 8), ((1237 << 8) + 77, (523 << 8) + 200)] {
                let mut sim = test_sim(xpos, ypos);
                let (cx, cy) = sim.camera();
                // a midground block in the camera's cell and a foreground block one row below
                let (col, row) = (cx >> 4, cy >> 4);
                let bw = sim.boardwidth;
                for r in row - 3..=row + 3 {
                    for c in col - 3..=col + 3 {
                        sim.bd_mut().set(bw * r + c, 0);
                    }
                }
                sim.bd_mut().set(bw * row + col, 9);
                sim.bd_mut().set(bw * (row + 1) + col, 128);
                let mut q = Vec::new();
                world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (0, 0), &mut q);
                let (gx, gy) = (lay.gcx() as f32, lay.gcy() as f32);
                let near = |s: crate::atlas::AtlasSprite| {
                    q.iter()
                        .filter(|q| q.u == s.u as f32 && q.v == s.v as f32)
                        .map(|q| (q.x + s.xc as f32, q.y + s.yc as f32))
                        .min_by(|a, b| ((a.0 - gx).abs() + (a.1 - gy).abs()).total_cmp(&((b.0 - gx).abs() + (b.1 - gy).abs())))
                        .expect("drawn")
                };
                // hot spots: the midground cell sits within 2 px of the foreground column and
                // 16 +- 2 px above the next foreground row, as in the original window
                let ((mx, my), (fx, fy)) = (near(s9), near(s128));
                assert!((mx - fx).abs() <= 2.0 && (fy - my - 16.0).abs() <= 2.0, "{lay:?} mid ({mx},{my}) fg ({fx},{fy})");
            }
        }
        // a large window draws midground blocks in its top-left strip (the layer sits further
        // right/down there, so cells before c0/rr0 cover the corner)
        let lay = Layout::new(1820.0, 768.0);
        let mut sim = test_sim(4000 << 8, 2000 << 8);
        let (cx, cy) = sim.camera();
        let (c, r) = ((cx - lay.gamew / 2) >> 4, (cy - lay.gameh / 2) >> 4);
        let bw = sim.boardwidth;
        for dr in -6..3 {
            for dc in -6..3 {
                sim.bd_mut().set(bw * (r + dr) + c + dc, 9);
            }
        }
        let mut q = Vec::new();
        world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (0, 0), &mut q);
        let n = q.iter().filter(|q| q.u == s9.u as f32 && q.x < (lay.gamex + 24) as f32 && q.y < (lay.gamey + 24) as f32).count();
        assert!(n > 0, "top-left midground strip");
    }

    #[test]
    fn backdrop_tiles_cover_window_with_c_phase() {
        let tk = TestKit::get();
        for (xpos, ypos) in [(1000 << 8, 600 << 8), (1237 << 8, 523 << 8), (5 << 8, 3 << 8)] {
            let sim = test_sim(xpos, ypos);
            let lay = Layout::original();
            let mut q = Vec::new();
            world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (0, 0), &mut q);
            let tiles: Vec<&Quad> = q.iter().filter(|q| q.w == 48.0).collect();
            // C: back_to_blit.x = 48 - r2 where r2 = (44+48-(r3%48))%48: screen gamex shows tile column 48-r2
            let r3 = (xpos >> 8) - (xpos >> 10) + 3072 - 768;
            let r2 = (44 + 48 - (r3 % 48)) % 48;
            let r4 = 0x1f & ((ypos >> 8) - (ypos >> 10));
            let first = tiles[0];
            assert_eq!((16.0 - first.x) as i32 % 48, (48 - r2) % 48);
            assert_eq!((8.0 - first.y) as i32, r4);
            // 288x192 window fully covered
            for &(px, py) in &[(16.0, 8.0), (303.0, 199.0), (160.0, 100.0)] {
                assert!(tiles.iter().any(|t| px >= t.x && px < t.x + 48.0 && py >= t.y && py < t.y + 32.0));
            }
            assert!(tiles.iter().all(|t| t.clip == CLIP_GAME && t.dx == 0.0));
        }
    }

    #[test]
    fn parallax_deltas() {
        let tk = TestKit::get();
        let mut sim = test_sim(1000 << 8, 600 << 8);
        let cell = sim.boardwidth * (600 >> 4) + (1000 >> 4);
        sim.bd_mut().set(cell, 10);
        let lay = Layout::original();
        let mut q = Vec::new();
        world_quads(&sim, &lay, &tk.atlas, CLIP_GAME, (8, -4), &mut q);
        let s = tk.atlas.spr(Bank::Blocks(0), 10).unwrap();
        // midground: r0 = 856 (prev 848), r1 = 504 (prev 508)
        let mid = q.iter().find(|q| q.u == s.u as f32 && q.v == s.v as f32).unwrap();
        assert_eq!((mid.dx, mid.dy), (8.0, -4.0 + 0.0));
        assert_eq!(mid_par(504) - mid_par(508), -4);
        let tile = q.iter().find(|q| q.w == 48.0).unwrap();
        // backdrop moves by c - (c>>2): 1000-250 - (992-248) = 6, 600-150 - (604-151) = -3
        assert_eq!((tile.dx, tile.dy), (6.0, -3.0));
        // extra tile column on the left for a positive delta
        assert!(tile.x < 16.0 - 48.0 + 1.0);
        assert_eq!(mid_par(856) - mid_par(848), 8);
        assert_eq!(mid_par(16) - mid_par(15), 0, "one step per cell does not move");
        assert_eq!(mid_par(17) - mid_par(16), 1);
    }

    #[test]
    fn draw_block_rules() {
        assert_eq!(draw_block(0, 0, 0, 0), None);
        assert_eq!(draw_block(9, 0, 0, 0), None);
        assert_eq!(draw_block(9, 1, 0, 0), Some(9));
        assert_eq!(draw_block(128, 1, 0, 0), None);
        for fc in 0..64 {
            assert_eq!(draw_block(43, 0, fc, 7), Some(40 + ((fc & 28) >> 2) as u8));
        }
        assert_eq!(draw_block(170, 0, 0, 0), Some(162));
        assert_eq!(draw_block(163, 0, 0, 0), Some(163));
        assert_eq!(draw_block(176, 0, 0, 0), Some(176), "platforms are not crumbles");
        for b in 97..=102 {
            assert_eq!(draw_block(b, 0, 0, 0), Some(97));
        }
        assert_eq!(draw_block(96, 0, 0, 0), Some(96));
        assert_eq!(draw_block(103, 0, 0, 0), Some(103));
        for b in 240..=253 {
            assert_eq!(draw_block(b, 0, 0, 0), None);
        }
        assert_eq!(draw_block(254, 0, 0, 0), Some(254));
        // gas flickers between 54/55, electricity within its group of four
        let mut seen = std::collections::BTreeSet::new();
        for fc in 0..200 {
            let g = draw_block(54, 0, fc, 1234).unwrap();
            assert!(g == 54 || g == 55);
            seen.insert(g);
            let e = draw_block(57, 0, fc, 99).unwrap();
            assert!((56..60).contains(&e));
            let e = draw_block(61, 0, fc, 99).unwrap();
            assert!((60..64).contains(&e));
        }
        assert_eq!(seen.len(), 2);
        assert_eq!(draw_block(57, 0, 5, 99), draw_block(57, 0, 5, 99), "deterministic");
    }
}
