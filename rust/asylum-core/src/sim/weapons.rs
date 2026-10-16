//! bullet.c (alien bullets) + projectile.c (player projectiles).
//!
//! `makebul`/`makeproj` return `true` on FAILURE (C returns 1). Ring allocation: like
//! makebul (`bulctr = (bulctr+1) % _bulno`); for makeproj implement the original assembly's
//! intent `projctr = (found_index + 1) % _projno` (the C port's off-by-one that let projctr
//! run past the 32-entry table is documented in PORTING_NOTES). Interpolation: increment
//! `bul_gen[k]`/`proj_gen[j]` when a slot is (re)filled; set `plot_key` to
//! `KEY_BUL + k` / `KEY_PROJ + j` before plotting an entry.
//!
//! The C tables `bulofs[0x1000]` / `projofs[0x1000]` only ever use their first 64 / 32
//! entries (`buladr`/`projadr` always point at the start): they are `Sim::bul` / `Sim::proj`.

use crate::atlas::Bank;
use crate::audio::sfx::SfxId;
use crate::sim::maze::block_target;
use crate::sim::*;

/// bullet.c `init_splittab`.
pub fn init_splittab(t: &mut [i32; 14]) {
    for i in 0..7 {
        let a = (0.5 + i as f64) / 3.5 * std::f64::consts::PI;
        t[i * 2] = (512.0 * a.sin()) as i32;
        t[i * 2 + 1] = (512.0 * a.cos()) as i32;
    }
}
/// projectile.c `init_projsplittab`.
pub fn init_projsplittab(t: &mut [i32; 10]) {
    for i in 0..5 {
        t[i * 2] = (64.0 * (0.5 - (i as f64 / 4.0 * std::f64::consts::PI).sin())) as i32;
        t[i * 2 + 1] = 0x100 * (i as i32 - 2);
    }
}
/// projectile.c `init_rocketbursttab`.
pub fn init_rocketbursttab(t: &mut [i32; 10]) {
    for i in 0..5 {
        let a = (0.5 + i as f64) / 2.5 * std::f64::consts::PI;
        t[i * 2] = (512.0 * a.sin()) as i32;
        t[i * 2 + 1] = (512.0 * a.cos()) as i32;
    }
}

#[inline]
fn block_trans(b: u8) -> bool {
    (TRANSLOWLIM..=TRANSHIGHLIM).contains(&b)
}

impl Sim {
    /// The bullet handler (aliens fire these).
    pub fn bullets(&mut self) {
        for k in 0..BULNO {
            // loop71:
            if self.bul[k].type_ == 0 {
                continue;
            }
            // foundbul:
            let mut b = self.bul[k];
            b.x = b.x.wrapping_add(b.dx);
            b.y = b.y.wrapping_add(b.dy);
            if b.x <= XLOWLIM || self.xposmax <= b.x || b.y <= YLOWLIM || self.yposmax <= b.y {
                // projoffscr
                b.type_ = 0;
                self.bul[k] = b;
                continue;
            }

            if b.dx <= (1 << 12) && b.dx.wrapping_neg() <= (1 << 12) && b.flags & BULL_ACCEL != 0 {
                b.dx += b.dx >> 4;
                b.dy += b.dy >> 4;
            }
            // bulnoacc:
            if b.flags & BULL_HOME != 0 && b.flags <= (3 << 6) * BULL_TTL {
                let mut r6 = 1 << 6;
                if b.flags & BULL_SLOWHOME == 0 {
                    r6 = 1 << 4;
                    b.dx -= b.dx >> 5;
                    b.dy -= b.dy >> 5;
                }
                b.dx += if b.x < self.xpos { r6 } else { -r6 };
                b.dy += if b.y < self.ypos { r6 } else { -r6 };
            }
            // bulnohome:
            b.dx = b.dx.clamp(-SPEEDLIM, SPEEDLIM);
            b.dy = b.dy.clamp(-SPEEDLIM, SPEEDLIM);

            b.flags = b.flags.wrapping_sub(BULL_TTL); // decrement the life counter
            self.bul[k] = b;

            let split = if b.flags < 0 {
                // out of time → buldestroy
                if b.flags & BULL_SPLIT == 0 {
                    self.bul[k].type_ = 0;
                    continue;
                }
                true // goto bulsplit
            } else if !(b.x <= self.pllx || self.plhx <= b.x || b.y <= self.plly || self.plhy <= b.y) {
                // bulletkill: hit the player
                self.bul[k].type_ = 0;
                let (dx4, dy4) = (b.dx >> 4, b.dy >> 4);
                let (ex, ey) = (b.x - b.dx, b.y - (8 << 8) - b.dy);
                self.explogonopyroquiet(ex, ey, dx4 + dy4, dy4 - dx4, 0, dx4 + dy4, None);
                self.explogonopyro(ex, ey, dx4 - dy4, dx4 + dy4, 0, dx4 - dy4, None);
                continue;
            } else {
                let r1 = self.cell(self.fntranslate(b.x, b.y));
                if !(r1 >= 16 && !block_trans(r1) && !block_target(r1)) {
                    // bulhitins: still flying
                    if !self.masterplotal {
                        continue;
                    }
                    let mut r0 = b.type_;
                    if r0 == BULSPRITEBASE + 8 || r0 == BULSPRITEBASE + 10 {
                        r0 ^= (self.framectr & 4) >> 2;
                    }
                    self.plot_key = KEY_BUL + k as u16;
                    self.relplot(Bank::Explo, r0 as u8, b.x, b.y);
                    self.plot_key = KEY_NONE;
                    continue;
                }
                // nobultarget: hit a block
                self.bul[k].type_ = 0;
                // bulnodestroy:
                if b.flags & BULL_EXPLO != 0 {
                    // causeexplobullet
                    let (dx4, dy4) = (b.dx >> 4, b.dy >> 4);
                    let (ex, ey) = (b.x - b.dx, b.y - (8 << 8) - b.dy);
                    self.explogonopyroquiet(ex, ey, dx4 + dy4, dy4 - dx4, dx4 + dy4, 0, None);
                    self.explogonopyro(ex, ey, dx4 - dy4, dx4 + dy4, dx4 - dy4, 0, None);
                    continue;
                }
                // buldestroy:
                if b.flags & BULL_SPLIT == 0 {
                    self.bul[k].type_ = 0;
                    continue;
                }
                true
            };
            if split {
                self.bulsplit(k);
            }
        }
        self.plot_key = KEY_NONE;
    }

    /// `bulsplit:` label of bullets(). Reads the slot afresh on every iteration like the C
    /// code (on the block-hit path the slot is already free and may be refilled).
    fn bulsplit(&mut self, k: usize) {
        let mut newtype = self.random() & 7;
        if newtype == 7 {
            newtype = 0;
        }
        let r7 = self.bul[k].type_;
        if r7 == BULSPRITEBASE + 13 {
            newtype = 7;
        }
        if r7 == BULSPRITEBASE + 14 {
            newtype = 10;
        }
        if r7 == BULSPRITEBASE + 15 {
            newtype = 12 + (newtype & 3);
        }
        if newtype == 15 {
            newtype = 0;
        }
        for i in (0..14).step_by(2) {
            // loop75:
            let b = self.bul[k];
            let _ = self.makebul(
                b.x,
                b.y,
                (b.dx >> 2) + self.splittab[i],
                (b.dy >> 2) + self.splittab[i + 1],
                newtype,
                (1 << 6) * BULL_TTL,
            );
        }
        self.bul[k].type_ = 0;
    }

    pub fn makebul(&mut self, x: i32, y: i32, dx: i32, dy: i32, type_: i32, flags: i32) -> bool {
        let type_ = type_ + BULSPRITEBASE;
        let mut flags = flags;
        // bulctr is always in range; guard against foreign state anyway
        let start = if (0..BULNO as i32).contains(&self.bulctr) { self.bulctr as usize } else { 0 };
        // loop72 / loop69. The C port returned failure even when the wrap-around scan found
        // a slot (`bulctr = i; … if (i == bulctr) return 1`); the evident intent is used.
        let Some(i) = (start..BULNO).chain(0..start).find(|&i| self.bul[i].type_ == 0) else {
            return true; // failed
        };
        self.bulctr = i as i32;
        // foundmakebul:
        if flags < BULL_TTL {
            flags |= (1 << 6) * BULL_TTL;
        }
        if type_ >= BULSPRITEBASE + 8 {
            flags |= BULL_SLOWHOME | BULL_EXPLO;
        }
        if type_ >= BULSPRITEBASE + 12 {
            flags = ((1 << 5) * BULL_TTL) | BULL_SPLIT;
        }
        if type_ == BULSPRITEBASE + 7 || type_ == BULSPRITEBASE + 8 {
            flags |= BULL_HOME;
        }
        if type_ == BULSPRITEBASE + 10 {
            flags |= BULL_ACCEL;
        }
        self.bul[i] = Shot { type_, x, y, dx, dy, flags };
        self.bul_gen[i] = self.bul_gen[i].wrapping_add(1);

        self.bulctr = (self.bulctr + 1) % BULNO as i32; // loop68:

        let r7 = x.wrapping_sub(self.xpos) >> 9;
        let ax = x.wrapping_sub(self.xpos).wrapping_abs();
        let ay = y.wrapping_sub(self.ypos).wrapping_abs();
        let vol = 0x7f - (ax.wrapping_add(ay) >> 12);
        if vol >= 80 {
            let v = (vol & 0x7f).min(0x7c) - if type_ == BULSPRITEBASE + 8 { 0x10 } else { 0 };
            let id = if type_ > BULSPRITEBASE + 8 {
                SfxId::BULLET_3
            } else if type_ == BULSPRITEBASE + 8 {
                SfxId::BULLET_2
            } else {
                SfxId::BULLET_1
            };
            self.bidforsound(EXPLOCHANNEL, v, r7, id);
        }
        // nozapsound:
        false
    }

    pub fn initbultab(&mut self) {
        self.bulctr = 0;
        for b in self.bul.iter_mut() {
            // loop74: `*(int*)r10 = 0` clears the type only
            b.type_ = 0;
        }
    }

    /// The projectile handler.
    pub fn project(&mut self) {
        for j in 0..PROJNO {
            // loop41:
            if self.proj[j].type_ == 0 {
                continue;
            }
            // foundproj:
            let mut p = self.proj[j];
            p.x = p.x.wrapping_add(p.dx);
            p.y = p.y.wrapping_add(p.dy);
            if p.x <= XLOWLIM || self.xposmax <= p.x || p.y <= YLOWLIM || self.yposmax <= p.y {
                // projoffscr
                p.type_ = 0;
                self.proj[j] = p;
                continue;
            }
            if p.flags & PROJ_ROCKET != 0 {
                p.dx += p.dx >> 4;
                p.dy += p.dy >> 4;
            }
            // projnoacc:
            p.dx = p.dx.clamp(-SPEEDLIM, SPEEDLIM);
            p.dy = p.dy.clamp(-SPEEDLIM, SPEEDLIM);

            p.flags = p.flags.wrapping_sub(PROJ_TTL); // decrement the life counter
            self.proj[j] = p;
            if p.flags < 0 {
                // projdestroy: out of time
                if p.flags & PROJ_SPLIT != 0 {
                    self.projsplit(j);
                    continue;
                }
                // projoffscr:
                self.proj[j].type_ = 0;
                continue;
            }
            if let Some(rs) = self.bulcolcheck(p.x, p.y) {
                let loss = if p.flags & PROJ_ROCKET != 0 { BULLETLOSS << 2 } else { BULLETLOSS };
                let r6 = self.projhital(rs, loss);
                self.proj[j].type_ = 0;
                if r6 == 0 {
                    continue;
                }
                self.makeobj(FLYINGBONUS, p.x, p.y, p.dx >> 2, -(1 << 10), 0x200 << 16, r6);
                if p.flags & PROJ_ROCKET != 0 {
                    self.causeexplo_proj(j);
                } else {
                    self.causeexplonopyro_proj(j);
                }
                continue;
            }

            let r0 = self.fntranslate(p.x, p.y);
            let r1 = self.cell(r0);
            if r1 < 16 || (block_trans(r1) && p.flags & PROJ_ATOM == 0) {
                // projhitins: still flying
                if !self.masterplotal {
                    continue;
                }
                self.plot_key = KEY_PROJ + j as u16;
                self.relplot(Bank::Bloke, p.type_ as u8, p.x, p.y);
                self.plot_key = KEY_NONE;
                continue;
            }
            // projhitcont: hit a block
            if (SPCRUMBLELOWLIM..=SPCRUMBLEHIGHLIM).contains(&r1)
                && (p.type_ == PROJSMALLNO + 2 || p.flags & PROJ_WEIRDSPLIT != 0)
            {
                // hack, better than original && (plweapontype == 5)
                self.set_cell(r0, 0);
                self.explogo(p.x, p.y, 0, 0, 0, 0, None);
            }
            // nospcrumble:
            self.proj[j].type_ = 0;
            self.destroy(r0);
            if p.flags & PROJ_ATOM != 0 {
                self.atomrocket(j, r0);
                continue;
            }
            if p.flags & PROJ_EXPLO != 0 {
                self.causeexplonopyro_proj(j);
                continue;
            }
            if p.flags & PROJ_ROCKET != 0 {
                self.causeexplo_proj(j);
                continue;
            }
        }
        self.plot_key = KEY_NONE;
    }

    pub fn projsplit(&mut self, j: usize) {
        let p = self.proj[j];
        let (x, y, mut dx, dy) = (p.x, p.y, p.dx >> 1, p.dy >> 1);
        let flags = p.flags;

        if flags & PROJ_ROCKET != 0 {
            self.rocketsplit(j);
            return;
        }

        let (mut r10, r9, mut r4);
        if flags & PROJ_FIVEWAY != 0 {
            r10 = 0;
            r9 = 5;
            r4 = PROJSMALLNO + 1;
        } else {
            r10 = 2;
            r9 = 3;
            r4 = PROJSMALLNO;
        }
        if flags & PROJ_SLOWSPLIT == 0 {
            dx >>= 1;
        }
        if flags & PROJ_WEIRDSPLIT != 0 {
            r4 = PROJSMALLNO + 2;
            dx <<= 2;
        }
        let r6 = if flags & PROJ_EXPLO != 0 {
            r4 = PROJSMALLNO + 3;
            PROJ_ROCKET
        } else {
            64 * PROJ_TTL
        };

        for _ in 0..r9 {
            // loop76:
            let (tx, ty) = (self.projsplittab[r10], self.projsplittab[r10 + 1]);
            self.makeproj(x, y, dx + tx, dy + ty, r4, r6);
            r10 += 2;
        }
        self.proj[j].type_ = 0;
    }

    pub fn rocketsplit(&mut self, j: usize) {
        let p = self.proj[j];
        let (x, y, dx, dy) = (p.x, p.y, p.dx >> 3, p.dy >> 3);
        let flags = p.flags;

        if flags & ROCK_DIVIDE != 0 {
            self.rocketpair(j);
            return;
        }
        // rocketburst:
        let r5 = (64 * PROJ_TTL) | PROJ_ROCKET | PROJ_EXPLO;
        for r10 in (0..10).step_by(2) {
            // loop77:
            let (tx, ty) = (self.rocketbursttab[r10], self.rocketbursttab[r10 + 1]);
            self.makeproj(x, y, dx + tx, dy + ty, PROJSMALLNO + 3, r5);
        }
        self.proj[j].type_ = 0;
        self.causeexplonopyro_proj(j);
    }

    pub fn rocketpair(&mut self, j: usize) {
        let p = self.proj[j];
        let (x, y, dx, dy) = (p.x, p.y, p.dx, p.dy);
        let flags = p.flags;
        let type_ = p.type_;
        let mut newflags = 64 * PROJ_TTL;

        if flags & ROCK_BURST != 0 {
            newflags = (16 * PROJ_TTL) | ROCK_BURST | PROJ_SPLIT;
        }
        if flags & ROCK_REDIVIDE != 0 {
            // continue splitting
            newflags &= !(64 * PROJ_TTL);
            newflags |= (16 * PROJ_TTL) | ROCK_DIVIDE | PROJ_SPLIT;
        }
        newflags |= PROJ_ROCKET;

        let mut newtype = type_ - 2;
        if newtype < ROCKETSPRITENO {
            newtype += 2;
        }
        if type_ & 1 != 0 {
            self.makeproj(x, y, dx - (dy >> 2), dy + (dx >> 2) - (dy >> 4), newtype, newflags);
        } else {
            self.makeproj(x, y, dx + (dy >> 2), dy - (dx >> 2) + (dy >> 4), newtype, newflags);
        }

        newtype = type_ + 2;
        if newtype > ROCKETSPRITELIM {
            newtype -= 2;
        }
        if type_ & 1 != 0 {
            self.makeproj(x, y, dx + (dy >> 2), dy - (dx >> 2) + (dy >> 4), newtype, newflags);
        } else {
            self.makeproj(x, y, dx - (dy >> 2), dy + (dx >> 2) - (dy >> 4), newtype, newflags);
        }
        self.proj[j].type_ = 0;
    }

    pub fn makeproj(&mut self, x: i32, y: i32, dx: i32, dy: i32, type_: i32, flags: i32) -> bool {
        // projctr is always in range after the PORTING_NOTES #2 fix; guard anyway
        let start = if (0..PROJNO as i32).contains(&self.projctr) { self.projctr as usize } else { 0 };
        // loop42 (projctr.._projno) then loop43 (0..projctr)
        match (start..PROJNO).chain(0..start).find(|&i| self.proj[i].type_ == 0) {
            Some(r8) => self.foundmakeproj(r8, x, y, dx, dy, type_, flags),
            None => true,
        }
    }

    /// projectile.c `foundmakeproj` with `projctr = (index + 1) % _projno` (PORTING_NOTES #2).
    #[allow(clippy::too_many_arguments)]
    fn foundmakeproj(&mut self, r8: usize, x: i32, y: i32, dx: i32, dy: i32, type_: i32, flags: i32) -> bool {
        let mut flags = flags;
        if flags < PROJ_TTL {
            flags |= 64 * PROJ_TTL;
        }
        self.proj[r8] = Shot { type_, x, y, dx, dy, flags };
        self.proj_gen[r8] = self.proj_gen[r8].wrapping_add(1);
        self.projctr = ((r8 + 1) % PROJNO) as i32;
        false
    }

    pub fn initprojtab(&mut self) {
        self.projctr = 0;
        for p in self.proj.iter_mut() {
            // loop44: clears the type only
            p.type_ = 0;
        }
    }

    /// projectile.c `causeexplo(projent*)`.
    pub fn causeexplo_proj(&mut self, j: usize) {
        let p = self.proj[j];
        self.explogomaybe(p.x - p.dx, p.y - p.dy - (8 << 8), 0, 0, 1, -1, None);
    }

    /// projectile.c `causeexplonopyro(projent*)`.
    pub fn causeexplonopyro_proj(&mut self, j: usize) {
        let p = self.proj[j];
        self.explogonopyro(p.x - p.dx, p.y - p.dy - (8 << 8), 0, 0, 1, -1, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::maze::tests::small_board;
    use crate::sim::testutil::sim;

    /// Sim on a small empty board with the player far away.
    fn setup() -> Sim {
        let mut s = sim();
        small_board(&mut s, 32, 32, 0);
        s.xpos = 2 << 12;
        s.ypos = 2 << 12;
        s.pllx = 0;
        s.plhx = 1 << 12;
        s.plly = 0;
        s.plhy = 1 << 12;
        s.initbultab();
        s.initprojtab();
        s
    }

    #[test]
    fn makebul_ring_allocation() {
        let mut s = setup();
        let far = 30 << 12; // no zap sound
        assert!(!s.makebul(far, far, 0, 0, 0, 0));
        assert_eq!((s.bul[0].type_, s.bulctr, s.bul_gen[0]), (BULSPRITEBASE, 1, 1));
        assert_eq!(s.bul[0].flags, (1 << 6) * BULL_TTL);
        for _ in 1..BULNO {
            assert!(!s.makebul(far, far, 0, 0, 1, 0));
        }
        assert_eq!(s.bulctr, 0);
        assert!(s.makebul(far, far, 0, 0, 1, 0), "table full");
        // free slots 10 and 3 while bulctr points at 40: scan forward, wrap to 3, then 10
        s.bul[10].type_ = 0;
        s.bul[3].type_ = 0;
        s.bulctr = 40;
        assert!(!s.makebul(far, far, 0, 0, 2, 0));
        assert_eq!((s.bul[3].type_, s.bulctr, s.bul_gen[3]), (BULSPRITEBASE + 2, 4, 2));
        assert!(!s.makebul(far, far, 0, 0, 2, 0));
        assert_eq!((s.bul[10].type_, s.bulctr), (BULSPRITEBASE + 2, 11));
        assert!(s.sfx_events.is_empty());
    }

    #[test]
    fn makebul_flags_and_sound() {
        let mut s = setup();
        s.tick_no = 1;
        // type 8 → slow homing explosive + homing; close to the player → zap sound
        assert!(!s.makebul(s.xpos + (8 << 9), s.ypos, 0, 0, 8, 1 << 24));
        assert_eq!(s.bul[0].flags, (1 << 24) | BULL_SLOWHOME | BULL_EXPLO | BULL_HOME);
        let e = s.sfx_events[0];
        assert_eq!((e.channel, e.id, e.volume, e.stereo), (3, SfxId::BULLET_2, 0x7c - 0x10, 8));
        // type >= 12 → fixed short life + split
        assert!(!s.makebul(30 << 12, 30 << 12, 0, 0, 13, 1 << 24));
        assert_eq!(s.bul[1].flags, ((1 << 5) * BULL_TTL) | BULL_SPLIT);
    }

    #[test]
    fn makeproj_ring_allocation_fix() {
        let mut s = setup();
        for i in 0..PROJNO {
            assert!(!s.makeproj(100, 100, 0, 0, 1, 0));
            assert_eq!(s.projctr, ((i + 1) % PROJNO) as i32);
        }
        assert_eq!(s.proj[5].flags, 64 * PROJ_TTL);
        assert!(s.makeproj(100, 100, 0, 0, 1, 0), "table full");
        s.proj[7].type_ = 0;
        s.projctr = 20;
        assert!(!s.makeproj(100, 100, 0, 0, 2, PROJ_TTL));
        assert_eq!((s.proj[7].type_, s.proj[7].flags, s.projctr, s.proj_gen[7]), (2, PROJ_TTL, 8, 2));
        // C port would have set projctr = 32 + 7 here (past the table)
        assert!((0..PROJNO as i32).contains(&s.projctr));
    }

    #[test]
    fn bullet_dies_on_solid_block() {
        let mut s = setup();
        s.masterplotal = true;
        s.tick_no = 3;
        let (bx, by) = (10, 10);
        s.set_cell(bx + by * 32, SOLIDLOWLIM);
        // fntranslate(x+dx, y+dy) lands in the solid block after one move
        let x = (bx << 12) - (8 << 8) - (1 << 8);
        let y = (by << 12) + (8 << 8);
        s.bul[0] = Shot { type_: BULSPRITEBASE + 1, x, y, dx: 1 << 8, dy: 0, flags: 10 * BULL_TTL };
        // and a bullet in free space is plotted with its interpolation key
        s.bul[5] = Shot { type_: BULSPRITEBASE + 2, x: 20 << 12, y: 20 << 12, dx: 1 << 8, dy: 0, flags: 10 * BULL_TTL };
        s.bullets();
        assert_eq!(s.bul[0].type_, 0);
        assert_eq!(s.bul[0].x, x + (1 << 8));
        assert_eq!(s.cell(bx + by * 32), SOLIDLOWLIM, "bullets never destroy blocks");
        assert_eq!(s.bul[5].type_, BULSPRITEBASE + 2);
        assert_eq!(s.bul[5].flags, 9 * BULL_TTL);
        assert_eq!(s.cmds.len(), 1);
        let c = s.cmds[0];
        assert_eq!((c.bank, c.idx, c.key), (Bank::Explo, (BULSPRITEBASE + 2) as u8, KEY_BUL + 5));
        assert_eq!(c.pos, Pos::World { x: ((20 << 12) + (1 << 8)) >> 8, y: 20 << 4 });
        assert_eq!(s.anchors[(KEY_BUL + 5) as usize].tick, 3);
        assert_eq!(s.plot_key, KEY_NONE);
    }

    #[test]
    fn bullet_lifetime_offscreen_and_speed_limit() {
        let mut s = setup();
        s.masterplotal = false;
        s.bul[0] = Shot { type_: 17, x: 20 << 12, y: 20 << 12, dx: 0, dy: 0, flags: 0 };
        s.bul[1] = Shot { type_: 17, x: 1 << 8, y: 20 << 12, dx: -(2 << 8), dy: 0, flags: 10 * BULL_TTL };
        s.bul[2] = Shot { type_: 17, x: 20 << 12, y: 20 << 12, dx: 40 << 8, dy: -(40 << 8), flags: 10 * BULL_TTL };
        s.bul[3] = Shot { type_: 17, x: 20 << 12, y: 20 << 12, dx: 1 << 8, dy: 0, flags: (10 * BULL_TTL) | BULL_ACCEL };
        s.bullets();
        assert_eq!(s.bul[0].type_, 0, "out of time");
        assert_eq!(s.bul[1].type_, 0, "left the map");
        assert_eq!((s.bul[2].dx, s.bul[2].dy), (SPEEDLIM, -SPEEDLIM));
        assert_eq!(s.bul[3].dx, (1 << 8) + (1 << 4));
        assert!(s.cmds.is_empty(), "masterplotal off: nothing plotted");
    }

    #[test]
    fn projectile_flies_and_plots() {
        let mut s = setup();
        s.masterplotal = true;
        s.proj[4] = Shot { type_: 3, x: 20 << 12, y: 20 << 12, dx: 2 << 8, dy: 0, flags: (5 * PROJ_TTL) | PROJ_ROCKET };
        s.proj[6] = Shot { type_: 3, x: 1 << 8, y: 20 << 12, dx: -(2 << 8), dy: 0, flags: 5 * PROJ_TTL };
        s.project();
        assert_eq!(s.proj[4].x, (20 << 12) + (2 << 8));
        assert_eq!(s.proj[4].dx, (2 << 8) + (2 << 4), "rockets accelerate");
        assert_eq!(s.proj[4].flags, (4 * PROJ_TTL) | PROJ_ROCKET);
        assert_eq!(s.proj[6].type_, 0);
        assert_eq!(s.cmds.len(), 1);
        assert_eq!((s.cmds[0].bank, s.cmds[0].idx, s.cmds[0].key), (Bank::Bloke, 3, KEY_PROJ + 4));
    }

    #[test]
    fn projsplit_spawns_three_way() {
        let mut s = setup();
        s.proj[0] = Shot { type_: 5, x: 20 << 12, y: 20 << 12, dx: 4 << 8, dy: 0, flags: PROJ_SPLIT };
        s.projctr = 1;
        s.projsplit(0);
        assert_eq!(s.proj[0].type_, 0);
        let spawned: Vec<(i32, i32, i32)> = s.proj[1..4].iter().map(|p| (p.type_, p.dx, p.dy)).collect();
        let dx = (4 << 8) >> 2;
        assert_eq!(
            spawned,
            vec![
                (PROJSMALLNO, dx + s.projsplittab[2], s.projsplittab[3]),
                (PROJSMALLNO, dx + s.projsplittab[4], s.projsplittab[5]),
                (PROJSMALLNO, dx + s.projsplittab[6], s.projsplittab[7]),
            ]
        );
        assert!(s.proj[1..4].iter().all(|p| p.flags == 64 * PROJ_TTL));
        assert_eq!(s.projctr, 4);
    }

    #[test]
    fn rocketpair_divides() {
        let mut s = setup();
        s.proj[0] = Shot { type_: 36, x: 20 << 12, y: 20 << 12, dx: 8 << 8, dy: 0, flags: ROCK_DIVIDE | PROJ_ROCKET };
        s.projctr = 1;
        s.rocketpair(0);
        assert_eq!(s.proj[0].type_, 0);
        let (a, b) = (s.proj[1], s.proj[2]);
        assert_eq!((a.type_, b.type_), (34, 38));
        assert_eq!((a.dx, a.dy), (8 << 8, -((8 << 8) >> 2)));
        assert_eq!((b.dx, b.dy), (8 << 8, (8 << 8) >> 2));
        assert_eq!(a.flags, (64 * PROJ_TTL) | PROJ_ROCKET);
    }
}
