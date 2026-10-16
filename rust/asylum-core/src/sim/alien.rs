//! alien.c — objects (aliens, platforms, explosions, embers, bonuses, decorations…),
//! collision tables, explosion creation, save/restore of the object table.
//!
//! Conventions: object procs take the slot index `i`. Functions that take "the alent being
//! collision checked" use `&mut Alent` (callers copy in/out; for the player that is
//! `fraudalent`). Functions that took `alent* r10` meaning "existing object or NULL" take
//! `Option<usize>`. `makeobj/softmakeobj` return `true` on FAILURE (C returns 1) and
//! bump `al_gen[slot]`. `moval` sets `plot_key = KEY_ALIEN + i` before `procal(i)`.
//! `headonroof`: the C port wrote `dodgypointer->y = …` (teleporting the squashed alien to
//! the top of the map); we apply the random value to `al.dy` instead (PORTING_NOTES #1).
//!
//! Aliasing: the alien procs collision-check against the *previous* frame's platform table,
//! whose entries may point at a slot that has since been freed and refilled — possibly with
//! the very alien being checked. In C `al` and `r6` are then the same object; the private
//! `*_impl` variants keep the caller's copy and `self.al[r6]` in sync in that case.
#![allow(clippy::too_many_arguments)]

use crate::assets::BIdx;
use crate::sim::*;

/// `options.explospeed` (always 1 in this port: 50 Hz, docs §3).
const EXPLOSPEED: i32 = 1;

/// alien.c `init_alspintab`.
pub fn init_alspintab(t: &mut [i32; 40]) {
    for i in 0..20 {
        let a = i as f64 * 2.0 * std::f64::consts::PI / 16.0;
        t[i * 2] = (2560.0 * a.sin()) as i32;
        t[i * 2 + 1] = (2560.0 * a.cos()) as i32;
    }
}

/// file.c `write_littleendian` (out-of-range writes are ignored).
fn put_le(store: &mut [u8], at: usize, v: u32) {
    if let Some(b) = store.get_mut(at..at + 4) {
        b.copy_from_slice(&v.to_le_bytes());
    }
}

/// file.c `read_littleendian` (reads past the end yield the end marker).
fn get_le(store: &[u8], at: usize) -> u32 {
    store.get(at..at + 4).map_or(0xffff_ffff, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl Sim {
    // ------------------------------------------------------------------ main loop

    pub fn moval(&mut self) {
        // Larger views: cover the visible game window, rounded up to whole blocks (identical to
        // C for the 288x192 window).
        let (wx, wy) = self.lay.disturb_blocks();
        let xd = XBLOCKS_DISTURB.max(wx);
        let yd = YBLOCKS_DISTURB.max(wy);
        self.sprlx = self.xpos - (((xd + 2) * 8) << 8);
        self.sprhx = self.xpos + (((xd + 2) * 8) << 8);
        self.sprly = self.ypos - (((yd + 2) * 8) << 8);
        self.sprhy = self.ypos + (((yd + 2) * 8) << 8);

        for i in 0..ALNO {
            if self.al[i].type_ != 0 {
                self.plot_key = KEY_ALIEN + i as u16;
                self.procal(i);
            }
        }
        self.plot_key = KEY_NONE;
    }

    pub fn procal(&mut self, i: usize) {
        let a = self.al[i];
        if a.x <= XLOWLIM || self.xposmax <= a.x || a.y <= YLOWLIM || self.yposmax <= a.y {
            // aloffscr:
            if a.type_ != DECORATION {
                self.al[i].type_ = 0;
                return;
            }
        }
        // noaloffscr:
        self.plotal = if a.x >= self.sprlx && self.sprhx >= a.x && a.y >= self.sprly && self.sprhy >= a.y {
            self.masterplotal
        } else {
            false
        };
        match a.type_ & 0x1f {
            1 => self.explo(i),
            2 => self.ember(i),
            3 => self.plat_n(1, i),
            4 => self.plat_n(2, i),
            5 | 6 => self.plat_n(3, i),
            7..=10 => self.plat_n(4, i),
            11 => self.plat_n(5, i),
            12 => self.scoreobj(i),
            13 => self.dyingbonus(i),
            14 => self.flyingbonus(i),
            15 => self.booby(i),
            16 => self.decoration(i),
            17 => self.extender(i),
            t @ 18..=31 => self.alien_n(t - 17, i),
            _ => {} // aljerr
        }
    }

    // ------------------------------------------------------------------ wandering

    pub fn alienwander(&mut self, i: usize, r5: BIdx) {
        let a = self.al[i];
        if a.righthit == 1 || a.lefthit == 1 {
            self.almightjump(i);
        } else if self.cell(r5) == 0 || self.cell(r5 + 1) == 0 {
            self.alpossjump(i);
        } else if (self.random() & 0x3f) == 0 {
            self.almightjump(i);
        } else {
            self.almightjumpins(i);
        }
    }

    pub fn almightjumpins(&mut self, i: usize) {
        let a = self.al[i];
        if a.dx > (1 << 5) || a.dx < -(1 << 5) || a.dy > (1 << 7) || a.dy < -(1 << 7) {
            return;
        }
        self.alienstopped(i);
    }

    pub fn jumpyalwander(&mut self, i: usize) {
        let a = self.al[i];
        if a.righthit == 1 || a.lefthit == 1 {
            self.almightjump(i);
        } else if !(a.dx > (1 << 5) || a.dx < -(1 << 5) || a.dy > (1 << 7) || a.dy < -(1 << 7)) {
            self.alienstopped(i);
        }
    }

    pub fn alienwanderfly(&mut self, i: usize) {
        if (self.random() & 0xff) == 0 {
            self.alienstoppedfly(i);
        }
        let a = self.al[i];
        if a.dx > (1 << 5) || a.dx < -(1 << 5) || a.dy > (1 << 5) || a.dy < -(1 << 5) {
            return;
        }
        self.alienstoppedfly(i);
    }

    pub fn alienwandernojump(&mut self, i: usize) {
        let a = self.al[i];
        if a.dx > (1 << 5) || a.dx < -(1 << 5) || a.dy > (1 << 8) || a.dy < -(1 << 8) {
            return;
        }
        self.alienstopped(i);
    }

    pub fn alienstopped(&mut self, i: usize) {
        if self.al[i].downhit == 1 || self.alonobj == 1 {
            self.al[i].dx = (self.random() & (1 << 9)) - (1 << 8);
        }
    }

    pub fn alienstoppedfly(&mut self, i: usize) {
        let r2 = self.random();
        let r11 = &mut self.al[i];
        if (r2 & 6) == 0 {
            // flyup:
            r11.dx = 0;
            r11.dy = -(1 << 7);
        } else if (r2 & 1) == 0 {
            // flyupdown:
            r11.dx = 0;
            r11.dy = (r2 & (1 << 9)) - (1 << 8);
        } else {
            r11.dx = (r2 & (1 << 10)) - (1 << 9);
            r11.dy = 0;
        }
    }

    pub fn almightjump(&mut self, i: usize) {
        if self.al[i].downhit != 1 {
            self.al[i].dx = 0;
        } else if (self.random() & 3) == 0 {
            self.al[i].dy = -(8 << 8);
        }
        self.almightjumpins(i);
    }

    pub fn alpossjump(&mut self, i: usize) {
        if self.al[i].downhit != 1 {
            self.al[i].dx = 0;
        } else if (self.random() & 0x1f) == 0 {
            self.al[i].dy = -(8 << 8);
        }
        self.almightjumpins(i);
    }

    pub fn almightwelljump(&mut self, i: usize) {
        if self.al[i].downhit != 1 {
            self.al[i].dx = 0;
        } else if (self.random() & 3) != 0 {
            self.al[i].dy = -(8 << 8);
        }
        self.almightjumpins(i);
    }

    pub fn alientestplat(&mut self, i: usize, r5: BIdx) {
        let _ = i;
        if self.alonplat == 0 {
            return;
        }
        self.plattoobj(r5);
    }

    // ------------------------------------------------------------------ misc objects

    pub fn decoration(&mut self, i: usize) {
        let r11 = &mut self.al[i];
        r11.r6 -= 1;
        if r11.r6 < 0 {
            r11.x += r11.dx;
            r11.y += r11.dy;
            r11.dy += 1 << 4;
        }
        r11.r5 -= 1 << 16;
        if r11.r5 < 0 {
            r11.type_ = 0;
        }
        if !self.masterplotal {
            return;
        }
        // mazescaleplot(blokeadr, r5&0xff, _xofs+(x>>8), _yofs+8+(y>>8))
        let a = self.al[i];
        self.plot_center(Bank::Bloke, (a.r5 & 0xff) as u8, (a.x >> 8) - 1, a.y >> 8, Clip::Game);
    }

    pub fn extender(&mut self, i: usize) {
        let a = self.al[i];
        let tmpx = a.x + a.dx;
        let tmpy = a.y + a.dy;
        // `if (!plcolcheck(...)) ;` is a no-op (plcolcheck has no side effects): not ported.
        self.al[i].x += a.dx;
        self.al[i].y += a.dy;

        let r0 = self.fntranslate(tmpx, tmpy);
        let f = a.r5 & 0xff;
        if a.r5 & (1 << 8) != 0 {
            // contracting:
            let c = self.cell(r0) as i32;
            if c == f || c == 0 {
                self.set_cell(r0, 0);
            } else if c != f + 1 && c != f + 2 && !maze::block_gas(c as u8) {
                self.al[i].type_ = 0;
                return;
            }
        } else {
            if self.cell(r0) == 0 {
                self.set_cell(r0, f as u8);
            }
            let c = self.cell(r0) as i32;
            if c != f && c != f + 1 && c != f + 2 && !maze::block_gas(c as u8) {
                self.al[i].dx = -self.al[i].dx;
                self.al[i].r5 |= 1 << 8;
            }
        }

        // nostopextend:
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        let r0 = self.fntranslate(a.x + a.dx * 15, a.y);
        let f = a.r5 & 0xff;
        let c = self.cell(r0) as i32;
        if c != 0 && c != f && c != f + 1 && c != f + 2 {
            return;
        }
        let b = self.blocks_bank();
        self.relplot(b, (a.r5 & 0xff) as u8, a.x + a.dx * 7, a.y);
    }

    // ------------------------------------------------------------------ aliens

    /// alien1 .. alien14 (`n` = 1..=14), dispatched from procal.
    pub fn alien_n(&mut self, n: i32, i: usize) {
        match n {
            1 => self.alien1(i),
            2 => self.alien2(i),
            3 => self.alien3(i),
            4 => self.alien4(i),
            5 => self.alien5(i),
            6 => self.alien6(i),
            7 => self.alien7(i),
            8 => self.alien8(i),
            9 => self.alien9(i),
            10 => self.alien10(i),
            11 => self.alien11(i),
            12 => self.alien12(i),
            13 => self.alien13(i),
            14 => self.alien14(i),
            _ => {}
        }
    }

    /// `colcheck(r11, colchecktype, r11->y-(8<<8))` on slot `i`.
    fn al_colcheck(&mut self, i: usize, colchecktype: i32) {
        let mut a = self.al[i];
        let platypos = a.y - (8 << 8);
        self.colcheck_impl(&mut a, Some(i), colchecktype, platypos);
        self.al[i] = a;
    }

    /// `albcheck(r11)` on slot `i`.
    fn al_albcheck(&mut self, i: usize) -> BIdx {
        let mut a = self.al[i];
        let r5 = self.albcheck(&mut a);
        self.al[i] = a;
        r5
    }

    fn al_move(&mut self, i: usize) {
        let r11 = &mut self.al[i];
        r11.x += r11.dx;
        r11.y += r11.dy;
    }

    fn al_gravity(&mut self, i: usize) {
        let r11 = &mut self.al[i];
        r11.dy += 1 << 7;
        if r11.dy > (1 << 11) {
            r11.dy = 1 << 11;
        }
    }

    /// Contact damage: player and alien both lose strength.
    fn al_touch(&mut self, i: usize, height: i32) {
        let a = self.al[i];
        if self.plcolcheck(a.x, a.y, ALWIDTH, height) {
            self.pllosestrength(EXPLOCONTLOSS);
            self.al[i].r6 -= EXPLOCONTLOSS;
        }
    }

    /// The two `if (abs(..) > range) alsleep(s, r11);` checks.
    fn al_sleepcheck(&mut self, i: usize, s: i32, range: i32) {
        // Larger views: the range grows with the visible window like the plot range and the
        // wake-up box, so aliens on screen stay awake (identical to C for 288x192).
        let (ex, ey) = self.sleep_extra();
        if self.al[i].x.wrapping_sub(self.xpos).wrapping_abs() > range + ex {
            self.alsleep(s, i);
        }
        if self.al[i].y.wrapping_sub(self.ypos).wrapping_abs() > range + ey {
            self.alsleep(s, i);
        }
    }

    /// Extra sleep range (8.8) of a game window larger than the original 288x192.
    fn sleep_extra(&self) -> (i32, i32) {
        let (x, y) = crate::sim::maze::sleep_extra(&self.lay);
        (x << 8, y << 8)
    }

    /// Common body of alien2..alien10 (walkers). Returns false if the alien was killed.
    fn alwalk(&mut self, i: usize, s: i32, jumpy: bool) -> bool {
        self.al_colcheck(i, 1);
        let r5 = self.al_albcheck(i);
        self.al_move(i);
        self.al_gravity(i);
        if jumpy {
            self.jumpyalwander(i);
        } else {
            self.alienwander(i, r5);
        }
        self.alientestplat(i, r5);

        self.bulcolchadd(i);
        self.al_touch(i, ALHEIGHT);
        if self.al[i].r6 <= 0 {
            self.alkill(i);
            return false;
        }
        self.al_sleepcheck(i, s, SLEEPRANGE);
        true
    }

    fn al_plot(&mut self, i: usize, n: i32) {
        let a = self.al[i];
        let b = self.aliens_bank();
        self.relplot(b, n as u8, a.x, a.y);
    }

    /// Plotting of the spinning-shot aliens (alien4/5/6): body, head, four orbiting shots.
    fn alspinplot(&mut self, i: usize, body: i32, shotbase: i32, shotmask: i32) {
        let a = self.al[i];
        let b = self.aliens_bank();
        self.relplot(b, body as u8, a.x, a.y);
        self.relplot(b, (4 + ((a.r5 >> 24) & 3)) as u8, a.x, a.y - (7 << 8));
        if a.r5 & (1 << 31) != 0 {
            return;
        }
        let f = (a.r5 >> 26) & 3;
        let r0 = (a.r5 >> 24) & 3;
        let mut r3 = (r0 * 2) as usize;
        for r4 in (1..=4).rev() {
            let n = BULSPRITEBASE + shotbase + ((r4 + f) & shotmask);
            let (ox, oy) = (self.alspintab[r3], self.alspintab[r3 + 1]);
            self.relplot(Bank::Explo, n as u8, a.x + ox, a.y - (7 << 8) + oy);
            r3 += 8; // 8 ints (32 bytes)
        }
    }

    fn alien1(&mut self, i: usize) {
        self.al_colcheck(i, 2);
        self.al_albcheck(i);
        self.al_move(i);
        self.al_gravity(i);
        self.alienwandernojump(i);

        self.bulcolchadd(i);
        self.al_touch(i, ALHEIGHT / 2);
        if self.al[i].r6 <= 0 {
            self.alkill(i);
            return;
        }
        self.al_sleepcheck(i, 1, SLEEPRANGE);

        if !self.plotal {
            return;
        }
        let a = self.al[i];
        let n = 8 + (a.dx > 0) as i32 * 2 + ((self.framectr & 8) >> 3);
        let b = self.aliens_bank();
        self.relplot(b, n as u8, a.x, a.y + (8 << 8));
    }

    fn alien2(&mut self, i: usize) {
        if !self.alwalk(i, 2, false) {
            return;
        }
        if !self.plotal {
            return;
        }
        self.al_plot(i, 0);
    }

    fn alien3(&mut self, i: usize) {
        if !self.alwalk(i, 3, false) {
            return;
        }
        if (self.random() & 0x3f) == 0 {
            self.alshoot(i);
        }
        if !self.plotal {
            return;
        }
        self.al_plot(i, 1);
    }

    fn alien4(&mut self, i: usize) {
        if !self.alwalk(i, 4, true) {
            return;
        }
        let r5 = self.al[i].r5;
        self.al[i].r5 = r5 + (r5 << 16);
        if self.al[i].r5 & (1 << 29) != 0 {
            self.al[i].r5 = self.alspinfire(i);
        }
        if self.al[i].r5 & (1 << 31) != 0 {
            self.al[i].r5 -= 8;
        } else {
            self.al[i].r5 += 4;
        }
        if (self.al[i].r5 & 0xffff) == 0 {
            self.al[i].r5 = 0;
        }
        if !self.plotal {
            return;
        }
        self.alspinplot(i, 2, 0, 3);
    }

    fn alien5(&mut self, i: usize) {
        if !self.alwalk(i, 5, false) {
            return;
        }
        let r5 = self.al[i].r5;
        self.al[i].r5 = r5 + (r5 << 16);
        if self.al[i].r5 & (1 << 29) != 0 {
            self.al[i].r5 = self.alspinpowerfire(i, 0);
        }
        if self.al[i].r5 & (1 << 31) != 0 {
            self.al[i].r5 -= 2;
        } else {
            self.al[i].r5 += 2;
        }
        if (self.al[i].r5 & 0xffff) == 0 {
            self.al[i].r5 = 0;
        }
        if !self.plotal {
            return;
        }
        self.alspinplot(i, 2, 8, 1);
    }

    fn alien6(&mut self, i: usize) {
        if !self.alwalk(i, 6, false) {
            return;
        }
        let r5 = self.al[i].r5;
        self.al[i].r5 = r5 + (r5 << 16);
        if self.al[i].r5 & (1 << 29) != 0 {
            self.al[i].r5 = self.alspinpowerfire(i, 1);
        }
        if self.al[i].r5 & (1 << 31) != 0 {
            self.al[i].r5 -= 2;
        } else {
            self.al[i].r5 += 8;
        }
        if (self.al[i].r5 & 0xffff) == 0 {
            self.al[i].r5 = 0;
        }
        if !self.plotal {
            return;
        }
        self.alspinplot(i, 3, 10, 1);
    }

    pub fn alspinfire(&mut self, i: usize) -> i32 {
        let a = self.al[i];
        let r0 = self.random() & 3;
        let mut r6 = (r0 << 1) as usize;
        for r4 in (1..=4).rev() {
            let t = self.alspintab;
            self.makebul(t[r6] + a.x, t[r6 + 1] + a.y - (7 << 8), t[r6 + 8] >> 2, t[r6 + 9] >> 2, (r4 + r0) & 3, (1 << 8) * BULL_TTL);
            r6 += 8;
        }
        r6 -= 8;
        let t = self.alspintab;
        // C: "XXX scaling?" — spin-table offsets compared with the player position, kept as is.
        let r2 = 0x7f - (t[r6] - self.xpos).wrapping_abs() - (t[r6 + 1] - (7 << 8) - self.ypos).wrapping_abs();
        if r2 > 0x40 {
            self.bidforsound(EXPLOCHANNEL, r2, (t[r6 + 1] - (7 << 8)) >> 9, SfxId::SPINFIRE);
        }
        // nospinzapsound:
        0x8000_0100_u32 as i32
    }

    pub fn alspinpowerfire(&mut self, i: usize, r7: i32) -> i32 {
        let a = self.al[i];
        let r0 = self.random() & 3;
        let mut r6 = (r0 << 1) as usize;
        for _r4 in (1..=4).rev() {
            let t = self.alspintab;
            self.makebul(t[r6] + a.x, t[r6 + 1] + a.y - (7 << 8), t[r6 + 8] >> 2, t[r6 + 9] >> 2, 8 + (r7 << 1), (1 << 8) * BULL_TTL);
            r6 += 8;
        }
        r6 -= 8;
        let t = self.alspintab;
        let r2 = 0x7f - (t[r6] - self.xpos).wrapping_abs() - (t[r6 + 1] - (7 << 8) - self.ypos).wrapping_abs();
        if r2 > 0x40 {
            self.bidforsound(EXPLOCHANNEL, r2, (t[r6 + 1] - (7 << 8)) >> 9, SfxId::SPINPOWERFIRE);
        }
        // nopowerzapsound:
        0x8000_0040_u32 as i32
    }

    fn alien7(&mut self, i: usize) {
        if !self.alwalk(i, 7, false) {
            return;
        }
        let rng = self.random();
        if (rng & 0x3f) == 0 {
            self.alshootnutter(i);
        } else if (rng & 0xf) == 0 {
            self.alshootfast(i);
        }
        if !self.plotal {
            return;
        }
        self.al_plot(i, 16);
    }

    fn alien8(&mut self, i: usize) {
        if !self.alwalk(i, 8, false) {
            return;
        }
        let rng = self.random();
        if (rng & 0x3f) == 0 {
            self.alshootnutter(i);
        } else if (rng & 0xf) == 0 {
            self.alshootmental(i);
        }
        if !self.plotal {
            return;
        }
        self.al_plot(i, 16);
    }

    fn alien9(&mut self, i: usize) {
        if !self.alwalk(i, 9, false) {
            return;
        }
        let rng = self.random();
        if (rng & 0x1f) == 0 {
            self.alshootnutterplus(i);
        }
        if !self.plotal {
            return;
        }
        self.al_plot(i, 17);
    }

    fn alien10(&mut self, i: usize) {
        if !self.alwalk(i, 10, false) {
            return;
        }
        let rng = self.random();
        if (rng & 0x1f) == 0 {
            self.alshootnuttermental(i);
        }
        if !self.plotal {
            return;
        }
        self.al_plot(i, 17);
    }

    fn alien11(&mut self, i: usize) {
        self.al_colcheck(i, 1);
        self.al_albcheck(i);
        self.al_move(i); // no gravity

        self.alienwanderfly(i);

        self.bulcolchadd(i);
        self.al_touch(i, ALHEIGHT);
        if self.al[i].r6 <= 0 {
            self.alkill(i);
            return;
        }
        self.al_sleepcheck(i, 11, 2 * SLEEPRANGE);
        let rng = self.random();
        if (rng & 0x1f) == 0 {
            self.alshootfast(i);
        }

        if !self.plotal {
            return;
        }
        let n = 12 + (self.random() & 3);
        self.al_plot(i, n);
    }

    fn alien12(&mut self, i: usize) {
        self.al_colcheck(i, 1);
        self.al_albcheck(i);
        self.al_move(i); // no gravity
        {
            let r11 = &mut self.al[i];
            r11.dx += r11.dx >> 6;
            r11.dy += r11.dy >> 6;
            r11.dx = r11.dx.clamp(-SPEEDLIM, SPEEDLIM);
            r11.dy = r11.dy.clamp(-SPEEDLIM, SPEEDLIM);
        }

        self.alienwanderfly(i);

        self.bulcolchadd(i);
        self.al_touch(i, ALHEIGHT);
        if self.al[i].r6 <= 0 {
            self.alkill(i);
            return;
        }
        self.al_sleepcheck(i, 12, 4 * SLEEPRANGE);
        let rng = self.random();
        if (rng & 0x1f) == 0 {
            self.alshootnuttermental(i);
        }

        if !self.plotal {
            return;
        }
        let n = 12 + (self.random() & 3);
        self.al_plot(i, n);
    }

    fn alien13(&mut self, i: usize) {
        self.al_colcheck(i, 2);
        self.al_albcheck(i);
        self.al_move(i);

        if self.al[i].dy > (1 << 11) {
            self.al[i].dy = 1 << 11;
        }
        self.alienwanderfly(i);

        self.bulcolchaddshort(i);
        self.al_touch(i, ALHEIGHT / 2);
        if self.al[i].r6 <= 0 {
            self.alkill(i);
            return;
        }
        self.al_sleepcheck(i, 13, SLEEPRANGE);

        if !self.plotal {
            return;
        }
        let a = self.al[i];
        let n = 8 + 2 * (a.dx > 0) as i32 + ((self.framectr & 8) == 8) as i32;
        self.al_plot(i, n);
    }

    fn alien14(&mut self, i: usize) {
        self.al_colcheck(i, 1);
        self.al_albcheck(i);
        self.al_move(i);

        self.al[i].dy += 1;
        if self.al[i].dy > (1 << 9) {
            self.al[i].dy = 1 << 8;
        }

        self.bulcolchadd(i);
        if self.al[i].downhit == 1 {
            self.hamsterspecial(i);
        }
        self.al_touch(i, ALHEIGHT);
        if self.al[i].r6 <= 0 {
            self.alkill(i);
            return;
        }
        self.al_sleepcheck(i, 14, SLEEPRANGE);

        if !self.plotal {
            return;
        }
        self.al_plot(i, 18);
    }

    pub fn hamsterspecial(&mut self, i: usize) {
        let r11 = &mut self.al[i];
        r11.type_ = ALIEN1 + 11;
        r11.r6 = 12 << 8;
        r11.dx = 0;
        r11.dy = 0;
    }

    pub fn alkill(&mut self, i: usize) {
        let a = self.al[i];
        self.addtoscore(((a.type_ - (ALIEN1 - 1)) & 0x1f) * 100);
        self.explogo(a.x, a.y, 0, 0, 0, 0, Some(i));
        self.exploins(i);
    }

    // ------------------------------------------------------------------ alien shots

    /// The fire-range test shared by the alshoot* family.
    fn al_inrange(r2: i32, r3: i32) -> bool {
        !(r2 > FIRERANGE || r3 > FIRERANGE || r2 < -FIRERANGE || r3 < -FIRERANGE)
    }

    pub fn alshoot(&mut self, i: usize) {
        let a = self.al[i];
        let r2 = self.xpos - a.x;
        let r3 = (self.ypos + (8 << 8)) - (a.y - (6 << 8)); // aim at body
        if !Self::al_inrange(r2, r3) {
            return;
        }
        let mut r4 = self.random() & 7;
        if r4 == 7 {
            r4 = 6;
        }
        self.makebul(a.x, a.y, r2 >> 6, r3 >> 6, r4, (1 << 8) * BULL_TTL);
        self.bidforsound(EXPLOCHANNEL, 0x78, 0, SfxId::SHOOT);
    }

    pub fn alshootfast(&mut self, i: usize) {
        let a = self.al[i];
        let r2 = self.xpos - a.x;
        let r3 = (self.ypos + (8 << 8)) - (a.y - (6 << 8));
        if !Self::al_inrange(r2, r3) {
            return;
        }
        let mut r4 = self.random() & 7;
        if r4 == 7 {
            r4 = 6;
        }
        self.makebul(a.x, a.y, r2 >> 5, r3 >> 5, r4, (1 << 8) * BULL_TTL);
        self.bidforsound(EXPLOCHANNEL, 0x6f, 0, SfxId::SHOOT);
    }

    pub fn alshootnutter(&mut self, i: usize) {
        let a = self.al[i];
        let r2 = self.xpos - a.x;
        let r3 = (self.ypos + (12 << 8)) - (a.y - (8 << 8));
        if !Self::al_inrange(r2, r3) {
            return;
        }
        self.makebul(a.x, a.y, 0, -(1 << 10), 12, (1 << 6) * BULL_TTL);
        self.bidforsound(EXPLOCHANNEL, 0x6f, 0, SfxId::SHOOTNUTTER);
    }

    pub fn alshootnutterplus(&mut self, i: usize) {
        let a = self.al[i];
        let r2 = self.xpos - a.x;
        let r3 = (self.ypos + (12 << 8)) - (a.y - (8 << 8));
        if !Self::al_inrange(r2, r3) {
            return;
        }
        self.makebul(a.x, a.y, 0, -(1 << 10), 13, (1 << 5) * BULL_TTL);
        self.bidforsound(EXPLOCHANNEL, 0x6f, 0, SfxId::SHOOTNUTTER);
    }

    pub fn alshootnuttermental(&mut self, i: usize) {
        let a = self.al[i];
        let mut r2 = self.xpos - a.x;
        let mut r3 = (self.ypos + (12 << 8)) - (a.y - (8 << 8));
        if !Self::al_inrange(r2, r3) {
            return;
        }
        let r = self.random();
        if r & (1 << 11) != 0 {
            r2 = 0;
            r3 = -(1 << 10);
        } else {
            r2 = if r & (1 << 10) != 0 { 1 << 10 } else { -(1 << 10) };
            r3 = 0;
        }
        self.makebul(a.x, a.y, r2, r3, 14, (1 << 5) * BULL_TTL);
        self.bidforsound(EXPLOCHANNEL, 0x6f, 0, SfxId::SHOOTNUTTER);
    }

    pub fn alshootmental(&mut self, i: usize) {
        let a = self.al[i];
        let r2 = self.xpos - a.x;
        let r3 = (self.ypos + (8 << 8)) - (a.y - (6 << 8));
        if !Self::al_inrange(r2, r3) {
            return;
        }
        let r4 = 8 + (self.random() & 2);
        self.makebul(a.x, a.y, r2 >> 6, r3 >> 6, r4, (1 << 8) * BULL_TTL);
        self.bidforsound(EXPLOCHANNEL, 0x6f, 0, SfxId::SHOOT);
    }

    pub fn alsleep(&mut self, s: i32, i: usize) {
        let a = self.al[i];
        let r0 = self.translate(a.x - (4 << 8), a.y - (8 << 8));
        if self.cell(r0) != 0 {
            return;
        }
        self.set_cell(r0, (240 + (s - 1)) as u8);
        self.al[i].type_ = 0;
    }

    // ------------------------------------------------------------------ explosions, embers, bonuses

    pub fn explo(&mut self, i: usize) {
        {
            let r11 = &mut self.al[i];
            r11.x += r11.dx;
            r11.y += r11.dy;
        }
        let a = self.al[i];
        let r4 = a.r5;
        let r5 = EXPLOSPEED << 2;
        if r4 <= r5 && self.plcolcheck(a.x, a.y, EXPWIDTH, EXPHEIGHT) {
            self.pllosestrength(EXPLOCONTLOSS);
        }
        self.exploins(i);
    }

    pub fn exploins(&mut self, i: usize) {
        let r11 = &mut self.al[i];
        r11.r5 += EXPLOSPEED;
        if r11.r5 < 0 {
            r11.r5 = 50 << 8; // trap negatives
        }
        let r0 = (r11.r5 & 0xff) as u8;
        if r0 >= (8 << 2) {
            // explo frame length
            r11.type_ = 0;
            return;
        }
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        self.relplot(Bank::Explo, ((a.r5 >> 2) & 0x3f) as u8, a.x, a.y);
    }

    pub fn booby(&mut self, i: usize) {
        let a = self.al[i];
        let r0 = self.fntranslate(a.x + a.dx, a.y);
        if self.embertrybombtarget(r0, i) {
            return;
        }
        // noboobybomb:
        if self.cell(r0) >= BLIM {
            self.al[i].dx = -(self.al[i].dx >> 1);
        }
        let a = self.al[i];
        let r0 = self.fntranslate(a.x + a.dx, a.y + a.dy);
        if self.embertrybooby(r0, i) {
            return;
        }
        // noboobybooby:
        if self.cell(r0) >= BLIM {
            self.al[i].dy = -(self.al[i].dy >> 1);
        }
        let r11 = &mut self.al[i];
        r11.x += r11.dx;
        r11.y += r11.dy;
        r11.dy += 1 << 4; // gravity
        r11.r5 += 1;
        if r11.r5 < 0 {
            r11.r5 = 1 << 8; // trap negatives
        }
        if r11.r5 >= 140 {
            // booby lifetime
            r11.type_ = 0;
            return;
        }
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        self.relplot(Bank::Explo, (12 + ((a.r5 >> 2) & 3)) as u8, a.x, a.y);
    }

    pub fn ember(&mut self, i: usize) {
        let a = self.al[i];
        let r0 = self.fntranslate(a.x + a.dx, a.y);
        if self.embertrybomb(r0, i) {
            return;
        }
        // noemberbomb:
        if self.cell(r0) >= BLIM {
            self.al[i].dx = -(self.al[i].dx >> 1);
        }
        let a = self.al[i];
        let r0 = self.fntranslate(a.x + a.dx, a.y + a.dy);
        if self.embertrygas(r0, i) {
            return;
        }
        // noembergas:
        if self.cell(r0) >= BLIM {
            self.al[i].dy = -(self.al[i].dy >> 1);
        }
        let r11 = &mut self.al[i];
        r11.x += r11.dx;
        r11.y += r11.dy;
        r11.dy += 1 << 4; // gravity

        r11.r5 += 1;
        if r11.r5 < 0 {
            r11.r5 = 1 << 8; // trap negatives
        }
        if r11.r5 >= 70 {
            // ember lifetime
            r11.type_ = 0;
            return;
        }
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        self.relplot(Bank::Explo, (8 + ((a.r5 >> 2) & 3)) as u8, a.x, a.y);
    }

    pub fn normbomb(&mut self, r0: BIdx, i: usize) {
        if (self.al[i].type_ & 0x1f) != BOOBY {
            self.al[i].type_ = 0;
        }
        self.normbombsurvive(r0);
    }

    pub fn bonusobjgot(&mut self, i: usize) {
        let a = self.al[i];
        self.bonuscommon(a.r6 - BONUSLOW as i32, a.x, a.y);
        self.al[i].type_ = 0;
    }

    /// One `flyingbonus` wall test: reverses `d` (and marks the hit) when the cells ahead
    /// are blocks and the bonus moves towards them.
    fn flyingbounce(&self, c0: BIdx, c1: BIdx, towards_neg: bool, d: &mut i32, r4: &mut i32) {
        if self.cell(c0) >= BONUSLOW || self.cell(c1) >= BONUSLOW {
            let moving = if towards_neg { *d <= 0 } else { *d >= 0 };
            if moving {
                *d = -*d;
                *r4 = 1;
            }
        }
    }

    pub fn flyingbonus(&mut self, i: usize) {
        let mut r4 = 0; // hit wall marker
        let bw = self.boardwidth;
        let mut a = self.al[i];

        let r0 = self.fntranslate(a.x + a.dx - (8 << 8), a.y - (8 << 8));
        self.flyingbounce(r0, r0 + bw, true, &mut a.dx, &mut r4);
        self.flyingbounce(r0 + 1, r0 + bw + 1, false, &mut a.dx, &mut r4);
        // flyingskip:
        let r0 = self.fntranslate(a.x + a.dx - (8 << 8), a.y + a.dy - (8 << 8));
        self.flyingbounce(r0, r0 + 1, true, &mut a.dy, &mut r4);
        self.flyingbounce(r0 + bw, r0 + bw + 1, false, &mut a.dy, &mut r4);
        // flyingvertskip:
        let r0 = self.fntranslate(a.x + a.dx - (8 << 8), a.y + a.dy - (8 << 8));
        self.flyingbounce(r0, r0 + bw, true, &mut a.dx, &mut r4);
        self.flyingbounce(r0 + 1, r0 + bw + 1, false, &mut a.dx, &mut r4);
        let r0 = self.fntranslate(a.x + a.dx - (8 << 8), a.y + a.dy - (8 << 8));
        self.flyingbounce(r0, r0 + 1, true, &mut a.dy, &mut r4);
        self.flyingbounce(r0 + bw, r0 + bw + 1, false, &mut a.dy, &mut r4);

        a.x += a.dx;
        a.y += a.dy;
        a.dy += 1 << 5; // gravity
        if r4 != 0 {
            a.dx -= a.dx >> 2;
            a.dy -= a.dy >> 2;
        }
        // flyingnoslow:
        a.r5 -= 1 << 16;
        if a.r5 < 0 {
            // bonusdowngrade:
            a.type_ = DYINGBONUS;
            a.r5 = a.r6 | (1 << 20);
        }
        self.al[i] = a;

        if self.plcolcheck(a.x, a.y, 16 << 8, 0) {
            self.bonusobjgot(i);
        }

        if !self.plotal {
            return;
        }
        let a = self.al[i];
        let b = self.blocks_bank();
        self.relplot(b, (a.r6 & 0xff) as u8, a.x, a.y);
    }

    pub fn dyingbonus(&mut self, i: usize) {
        self.al[i].r5 -= 1 << 16;
        if self.al[i].r5 < 0 {
            self.deleteobj(i);
            return;
        }
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        let b = self.blocks_bank();
        self.plotdying(b, (a.r5 & 0xff) as u8, a.x, a.y, a.r5);
    }

    pub fn scoreobj(&mut self, i: usize) {
        let r11 = &mut self.al[i];
        r11.x += r11.dx;
        r11.y += r11.dy;
        r11.dy += 1 << 5;
        r11.r5 -= 1;
        if (r11.r5 & 0xff) == 0 {
            r11.type_ = 0;
            return;
        }
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        self.relplot(Bank::Bloke, (a.r5 >> 8) as u8, a.x, a.y);
    }

    // ------------------------------------------------------------------ platforms

    /// plat1 .. plat5 (`n` = 1..=5).
    pub fn plat_n(&mut self, n: i32, i: usize) {
        match n {
            1 => self.plat1(i),
            2 => self.plat2(i),
            3 => self.plat3(i),
            4 => self.plat4(i),
            5 => self.plat5(i),
            _ => {}
        }
    }

    /// Shared start of plat1..plat5: `dx = 0; dvcheck; y += dy; dy += acc; cap at lim`.
    fn plat_fall(&mut self, i: usize, acc: i32, lim: i32) {
        self.al[i].dx = 0;
        let mut a = self.al[i];
        self.dvcheck(&mut a);
        a.y += a.dy;
        a.dy += acc;
        if a.dy > lim {
            a.dy = lim;
        }
        self.al[i] = a;
    }

    fn plat1(&mut self, i: usize) {
        self.plat_fall(i, 1 << 4, 1 << 11);
        if self.al[i].downhit != 0 {
            self.platland(i, PLATNO);
        }
        self.colchadd(i);
        self.platins(i, PLATNO);
    }

    fn plat2(&mut self, i: usize) {
        self.plat_fall(i, 1 << 4, 1 << 11);
        if self.al[i].downhit != 0 || self.al[i].uphit != 0 {
            self.plattoexplo(i);
        }
        self.colchadd(i);
        self.platins(i, PLATNO + 2);
    }

    fn plat3(&mut self, i: usize) {
        self.plat_fall(i, 1 << 4, 1 << 11);
        if self.al[i].downhit != 0 {
            self.platland(i, PLATNO + 4);
        }
        self.colchadd(i);
        self.platins(i, PLATNO + 4);
    }

    fn plat4(&mut self, i: usize) {
        self.plat_fall(i, 1 << 4, 1 << 11);
        // The block number is re-evaluated from the (possibly cleared) type, as in C.
        if self.al[i].downhit != 0 {
            let r9 = (PLATNO as i32 - 8 + (self.al[i].type_ << 1)) as u8;
            self.platland(i, r9);
        }
        self.colchadd(i);
        let r9 = (PLATNO as i32 - 8 + (self.al[i].type_ << 1)) as u8;
        self.platins(i, r9);
    }

    fn plat5(&mut self, i: usize) {
        self.plat_fall(i, 1 << 2, 1 << 9);
        if self.al[i].downhit != 0 {
            self.platland(i, PLATNO + 14);
        }
        self.colchadd(i);
        self.platins(i, PLATNO + 14);
    }

    pub fn plattoobj(&mut self, r0: BIdx) {
        self.plattoobjins(r0, 0);
    }

    pub fn plattoobjins(&mut self, r0: BIdx, r4: i32) {
        let mut r0 = r0;
        let mut r4 = r4;
        let r1 = self.cell(r0);
        if 1 & r1 != 0 {
            r0 -= 1;
        }
        let (x, y) = self.backtranslate(r0);

        // C `char r`: computed in int, truncated to unsigned char.
        let mut r = ((self.cell(r0) as i32 - PLATNO as i32 + 2 * PLATBASE) >> 1) as u8;
        if r as i32 == EXPLOPLAT {
            r4 = -1; // C: -1-(1>>5); stop instant explosion
        }
        if r as i32 >= DOWNPLAT {
            r = r.wrapping_add(1);
        }
        let blk = self.cell(r0) as i32;
        let v = self.makeobj(r as i32, x + (8 << 8), y + (15 << 8), 0, r4, (32 << 16) + 16, blk);

        if !v {
            self.set_cell(r0, 0);
            self.set_cell(r0 + 1, 0);
        }
    }

    pub fn platins(&mut self, i: usize, r9: u8) {
        if !self.plotal {
            return;
        }
        let a = self.al[i];
        let b = self.blocks_bank();
        self.relplot(b, r9, a.x - (8 << 8), a.y);
        self.relplot(b, r9.wrapping_add(1), a.x + (8 << 8), a.y);
    }

    // ------------------------------------------------------------------ collision tables

    pub fn colchadd(&mut self, i: usize) {
        if self.colch_ptr.wrapping_sub(self.colch_tab) >= COLCHLIM {
            return;
        }
        let a = self.al[i];
        let xoff = a.r5 >> 16;
        let yoff = a.r5 & 0xffff;
        if let Some(e) = self.colch.get_mut(self.colch_ptr) {
            *e = Colch {
                al: i,
                xmin: a.x - (xoff << 7),
                ymin: a.y - (yoff << 7),
                xmax: a.x + (xoff << 7),
                ymax: a.y + (yoff << 7),
            };
        }
        self.colch_ptr += 1;
    }

    pub fn bulcolchadd(&mut self, i: usize) {
        if self.bulcolch_ptr >= BULCOLCHLIM {
            return;
        }
        let a = self.al[i];
        if let Some(e) = self.bulcolch.get_mut(self.bulcolch_ptr) {
            *e = Colch { al: i, xmin: a.x - (32 << 7), ymin: a.y - (32 << 7), xmax: a.x + (32 << 7), ymax: a.y + (32 << 7) };
        }
        self.bulcolch_ptr += 1;
    }

    pub fn bulcolchaddshort(&mut self, i: usize) {
        if self.bulcolch_ptr >= BULCOLCHLIM {
            return;
        }
        let a = self.al[i];
        if let Some(e) = self.bulcolch.get_mut(self.bulcolch_ptr) {
            *e = Colch {
                al: i,
                xmin: a.x - (32 << 7),
                ymin: a.y + (8 << 8) - (16 << 7),
                xmax: a.x + (32 << 7),
                ymax: a.y + (8 << 8) + (16 << 7),
            };
        }
        self.bulcolch_ptr += 1;
    }

    pub fn platland(&mut self, i: usize, r9: u8) {
        let a = self.al[i];
        let bw = self.boardwidth;
        let mut r0 = self.translate(a.x, a.y);
        if self.cell(r0) >= PLATBLIM {
            r0 -= bw;
        }
        if self.cell(r0 + bw) != EXTENDNO && self.cell(r0 + bw + 1) != EXTENDNO {
            self.set_cell(r0, r9);
            self.set_cell(r0 + 1, r9.wrapping_add(1));
            self.al[i].type_ = 0;
        }
    }

    pub fn plattoexplo(&mut self, i: usize) {
        self.blowup(i);
    }

    pub fn deleteobj(&mut self, i: usize) {
        self.al[i].type_ = 0;
    }

    pub fn blowup(&mut self, i: usize) {
        let a = self.al[i];
        self.explogo(a.x, a.y, 0, 1 << 6, 0, 0, Some(i));
    }

    pub fn switchcolch(&mut self) {
        self.oldcolch_tab = self.colch_tab;
        self.oldcolch_ptr = self.colch_ptr;
        self.colchplace = !self.colchplace;
        let start = if self.colchplace { 0 } else { COLCHLIM + 16 }; // +16 for safety
        self.colch_tab = start;
        self.colch_ptr = start;

        self.bulcolch_ptr = 0; // reset bullet checking table
    }

    /// Returns the hit object's index.
    pub fn bulcolcheck(&self, x: i32, y: i32) -> Option<usize> {
        self.bulcolch[..self.bulcolch_ptr.min(self.bulcolch.len())]
            .iter()
            .find(|e| e.xmax >= x && e.ymax >= y && x >= e.xmin && y >= e.ymin)
            .map(|e| e.al)
    }

    pub fn projhital(&mut self, al: usize, loss: i32) -> i32 {
        self.al[al].r6 -= loss;
        if self.al[al].r6 > 0 {
            return 0;
        }
        let r0 = self.al[al].type_;
        let alofs = if r0 >= ALIEN1 { r0 - ALIEN1 } else { 0 };
        let mut r6 = (self.random() & 7) + alofs + BONUSLOW as i32;
        if r6 > BONUSHIGH as i32 {
            r6 = BONUSHIGH as i32 - 3;
        }
        if r6 < BONUSLOW as i32 {
            r6 = BONUSLOW as i32;
        }
        r6
    }

    pub fn colcheck(&mut self, al: &mut Alent, colchecktype: i32, platypos: i32) {
        self.colcheck_impl(al, None, colchecktype, platypos);
    }

    /// `slot`: the `self.al` index `al` was copied from (None for the player).
    fn colcheck_impl(&mut self, al: &mut Alent, slot: Option<usize>, colchecktype: i32, platypos: i32) {
        // dodgypointer = al
        let (x, y) = if colchecktype != 0 { (al.x, al.y) } else { (self.xpos, self.ypos) };
        self.alonobj = colchecktype as u8;
        if colchecktype == 0 {
            // player check
            self.platsandstr = None;
            self.colch_use_tab = self.colch_tab;
            self.colch_use_ptr = self.colch_ptr;
        } else {
            self.colch_use_tab = self.oldcolch_tab;
            self.colch_use_ptr = self.oldcolch_ptr;
        }

        self.xtemp = x;
        self.ytemp = y;
        let r4 = 24 << 8;
        let r5 = 32 << 8;
        let xlo = x - (r4 >> 1);
        let mut ylo = y - (r5 >> 1);
        ylo += 8 << 8;
        let xhi = xlo + r4;
        let mut yhi = ylo + r5;
        yhi += 2 << 8; // so always in contact with platforms

        let mut r11 = self.colch_use_tab;
        loop {
            // colchins: find the next overlapping box
            let e = loop {
                if r11 >= self.colch_use_ptr || r11 >= self.colch.len() {
                    return; // colched
                }
                let e = self.colch[r11];
                r11 += 1;
                if e.xmax >= xlo && e.ymax >= ylo && xhi >= e.xmin && yhi >= e.ymin {
                    break e;
                }
            };
            if (e.ymax - ylo >= (1 << 8)) // lim for plat on head -> can move horiz.
                && (yhi - e.ymin >= (3 << 8))
            // same for standing on plat
            {
                // limr2:
                if e.xmax > xhi && al.dx > 0 {
                    al.dx = 0;
                }
                if e.xmin < xlo && al.dx < 0 {
                    al.dx = 0;
                }
            }
            // nolimr2:
            if ((e.xmax - xlo) >= (6 << 8)) // margin for right on edge
                && ((xhi - e.xmin) >= (6 << 8))
            {
                let alias = slot == Some(e.al);
                if (yhi - e.ymin) <= (16 << 8) {
                    // safety margin for platform under pl
                    self.rise_impl(e.al, al, alias, colchecktype, platypos);
                }
                if (e.ymax - ylo) <= (16 << 8) {
                    // ditto for plat on head
                    self.platonhead_impl(e.al, al, alias, colchecktype);
                }
            }
        }
    }

    /// Aliased `al`/`r6`: copy the platform object into the caller's alent.
    #[inline]
    fn alias_pull(&self, alias: bool, r6: usize, al: &mut Alent) {
        if alias {
            *al = self.al[r6];
        }
    }

    /// Aliased `al`/`r6`: copy the caller's alent into the platform object.
    #[inline]
    fn alias_push(&mut self, alias: bool, r6: usize, al: &Alent) {
        if alias {
            self.al[r6] = *al;
        }
    }

    pub fn rise(&mut self, r6: usize, al: &mut Alent, colchecktype: i32, platypos: i32) {
        self.rise_impl(r6, al, false, colchecktype, platypos);
    }

    fn rise_impl(&mut self, r6: usize, al: &mut Alent, alias: bool, colchecktype: i32, platypos: i32) {
        self.alias_push(alias, r6, al);
        if colchecktype == 2 {
            // dontrise:
            al.dy = 0;
            self.alonobj = 1;
            self.alias_push(alias, r6, al);
            return;
        }
        let mut r0 = self.al[r6].type_;
        if r0 == FASTPLATFIRE {
            self.platfire(r6);
        }
        if r0 == DOWNPLAT || r0 == FALLPLAT {
            // platfall: plat type 5 falls
            r0 = if r0 == FALLPLAT { self.al[r6].dy } else { (1 << 4) + self.al[r6].dy };
            if r0 < -(1 << 5) {
                r0 += 1 << 5; // plat is moving up - more speed
            }
            if r0 > (1 << 11) {
                r0 = 1 << 11;
            }
            self.al[r6].dy = r0;
            self.alias_pull(alias, r6, al);
            if al.dy > r0 {
                al.dy = r0;
            }
            if colchecktype == 0 {
                al.falling = 0xff;
                self.platsandstr = Some(r6);
            }
            self.alias_push(alias, r6, al);
            al.dy += self.al[r6].y - (32 << 8) - platypos;
            self.alias_push(alias, r6, al);
            return;
        }
        if r0 > FASTPLAT {
            // plat 6+ moves quickly
            self.al[r6].dy -= 1 << 7;
            self.alias_pull(alias, r6, al);
        }
        r0 = if colchecktype == 0 { al.pluphit as i32 } else { self.alheadcheck() as i32 };

        r0 = if r0 == 0 {
            self.al[r6].dy - (1 << 5) // hit
        } else {
            self.headonroof_impl(r6, al, alias, colchecktype, platypos)
        };

        if r0 > (1 << 5) {
            r0 -= 1 << 5; // plat is moving down - more speed
        }

        if r0 < -(3 << 10) {
            r0 = -(3 << 10);
            // platmaxspeed:
            let t = self.al[r6].type_;
            if t == FASTPLATSTOP {
                // platstop:
                self.platsurefire(r6);
                r0 = -(1 << 9);
            } else if t == FASTPLATEXPLO || t == FASTPLATFIRE {
                // platexplo:
                self.platdestroy(r6);
                self.alias_pull(alias, r6, al);
                return;
            }
        }
        // platmaxspeedins:
        self.al[r6].dy = r0;
        self.alias_pull(alias, r6, al);
        if al.dy >= r0 {
            al.dy = r0;
        }

        if colchecktype == 0 {
            al.falling = 0xff;
            self.platsandstr = Some(r6);
        }
        self.alias_push(alias, r6, al);

        al.dy += self.al[r6].y - (32 << 8) - platypos;
        if colchecktype != 0 {
            self.alonobj = 1;
            al.dx >>= 2;
        }
        self.alias_push(alias, r6, al);
    }

    pub fn platonhead(&mut self, r6: usize, al: &mut Alent, colchecktype: i32) {
        self.platonhead_impl(r6, al, false, colchecktype);
    }

    fn platonhead_impl(&mut self, r6: usize, al: &mut Alent, alias: bool, colchecktype: i32) {
        self.alias_push(alias, r6, al);
        if self.al[r6].type_ == EXPLOPLAT {
            self.platdestroy(r6);
            self.alias_pull(alias, r6, al);
            return;
        }
        if self.al[r6].dy > 0 {
            self.al[r6].dy = -self.al[r6].dy;
        }
        self.alias_pull(alias, r6, al);
        if self.al[r6].dy > al.dy {
            self.al[r6].dy += al.dy >> 1;
        }
        self.alias_pull(alias, r6, al);
        // platuphit = 1; seems useless
        if al.dy < 0 {
            al.dy = 0;
        }
        self.alias_push(alias, r6, al);
        if al.falling == 0xff {
            // only meaningful when colchecktype == 0
            // platsandwich:
            if colchecktype == 0 {
                if let Some(p) = self.platsandstr {
                    self.al[p].dy = 1 << 8;
                    self.alias_pull(alias, r6, al);
                }
            }
        }
    }

    pub fn platdestroy(&mut self, r6: usize) {
        let a = self.al[r6];
        self.explogo(a.x, a.y, 0, -(1 << 6), 0, 0, Some(r6));
    }

    pub fn platfire(&mut self, r6: usize) {
        let a = self.al[r6];
        let r4 = self.random();
        if (r4 & (3 << 24)) == 0 {
            // firing rate
            self.makebul(a.x, a.y + (16 << 8), (r4 & (0xfe << 2)) - (0xfe << 1), -((r4 & (0xfe << 12)) >> 11), 8, (1 << 8) * BULL_TTL);
        }
        // platfireskip:
    }

    pub fn platsurefire(&mut self, r6: usize) {
        let a = self.al[r6];
        let r4 = self.random();
        self.makebul(a.x, a.y + (16 << 8), (r4 & (0xfe << 2)) - (0xfe << 1), -((r4 & (0xfe << 12)) >> 11), 14, (1 << 8) * BULL_TTL);
    }

    pub fn headonroof(&mut self, r6: usize, al: &mut Alent, colchecktype: i32, platypos: i32) -> i32 {
        self.headonroof_impl(r6, al, false, colchecktype, platypos)
    }

    fn headonroof_impl(&mut self, r6: usize, al: &mut Alent, alias: bool, colchecktype: i32, platypos: i32) -> i32 {
        let mut r0;
        let r1;
        if colchecktype != 0 {
            if (self.random() & 7) == 0 {
                // C: dodgypointer->y = …  (PORTING_NOTES #1: applied to dy)
                al.dy = (self.random() & (1 << 9)) - (1 << 8);
                self.alias_push(alias, r6, al);
            }
            // skipalrandload:
            r0 = 1;
            r1 = 0;
        } else {
            r0 = 1 << 8;
            r1 = al.downpress as i32;
        }
        // skipalrand:
        self.al[r6].y = platypos + (32 << 8); // force plat to player pos
        if r1 > 64 {
            r0 = 1 << 11;
        }
        if self.al[r6].type_ == UPDOWNPLAT {
            self.al[r6].type_ = DOWNPLAT;
        }
        self.alias_pull(alias, r6, al);
        r0
    }

    pub fn alheadcheck(&self) -> bool {
        let r0 = self.translate(self.xtemp, self.ytemp);
        let bw = self.boardwidth;
        self.cell(r0 - bw) >= BLIM || self.cell(r0 - bw + 1) >= BLIM
    }

    // ------------------------------------------------------------------ board collision

    pub fn albcheck(&mut self, al: &mut Alent) -> BIdx {
        al.lefthit = 0;
        al.righthit = 0;
        al.uphit = 0;
        al.downhit = 0; // wipe all four
        self.alonplat = 0;
        self.xtemp = al.x - (1 << 8);
        self.ytemp = al.y - (8 << 8);
        let bw = self.boardwidth;
        let r6 = self.ytemp & (15 << 8);
        let z = self.translate(self.xtemp, self.ytemp + al.dy);
        // up
        if self.cell(z - bw) >= BLIM || self.cell(z - bw + 1) >= BLIM {
            self.noup(al);
        }
        // down
        if self.cell(z + bw) >= BLIM || self.cell(z + bw + 1) >= BLIM {
            self.nodown(al);
        }
        if r6 != (15 << 8) {
            let z = self.translate(self.xtemp + al.dx, self.ytemp);
            // left
            if self.cell(z - bw) >= BLIM || self.cell(z) >= BLIM || self.cell(z + bw) >= BLIM {
                self.noleft(al);
            }
            // right
            if self.cell(z - bw + 1) >= BLIM || self.cell(z + 1) >= BLIM || self.cell(z + bw + 1) >= BLIM {
                self.noright(al);
            }
        } else {
            // alshort:
            let z = self.translate(self.xtemp + al.dx, self.ytemp);
            if self.cell(z) >= BLIM || self.cell(z + bw) >= BLIM {
                self.noleft(al);
            }
            // right
            if self.cell(z + 1) >= BLIM || self.cell(z + bw + 1) >= BLIM {
                self.noright(al);
            }
        }

        // albcheckins:
        let z = self.translate(self.xtemp + al.dx, self.ytemp + al.dy);
        // up
        if self.cell(z - bw) >= BLIM || self.cell(z - bw + 1) >= BLIM {
            self.noup(al);
        }
        // down
        if self.cell(z + bw) >= BLIM || self.cell(z + bw + 1) >= BLIM {
            self.nodown(al);
        }
        let r5 = z + (bw << 1); // signal position to caller
        if (PLATLOWLIM..=ALPLATHIGHLIM).contains(&self.cell(r5)) {
            self.alonplat = 1;
        }
        r5
    }

    pub fn noleft(&mut self, al: &mut Alent) {
        if al.dx < 0 {
            al.dx = 0;
            al.lefthit = 1;
        }
    }

    pub fn noright(&mut self, al: &mut Alent) {
        if al.dx > 0 {
            al.dx = 0;
            al.righthit = 1;
        }
    }

    pub fn fallinggap(&mut self, al: &mut Alent) -> i32 {
        let re = al;
        let bw = self.boardwidth;
        let r11 = self.translate(re.x + re.dx, re.y /*+dy*/ - if re.dy < 0 { 15 << 8 } else { 0 });
        let r1 = if re.dx < 0 { 0 } else { 1 };

        if self.cell(r11 + r1) < BLIM && self.cell(r11 + bw + r1) < BLIM {
            // stopfall:
            if re.dy > 0 {
                // stopfalldown:
                if self.cell(r11 - bw + r1) < BLIM {
                    return 0;
                }
                if self.cell(r11 + bw * 2 + r1) < BLIM {
                    return 0;
                }
                if self.cell(r11 + bw * 2 + (1 - r1)) < BLIM {
                    self.nodown(re);
                    return 1;
                }
            } else if re.dy < 0 {
                // stopfallup:
                if (self.ytemp & (15 << 8)) > (8 << 8) {
                    return 0;
                }
                if self.cell(r11 - bw + r1) < BLIM {
                    return 0;
                }
                if self.cell(r11 + bw * 2 + r1) < BLIM {
                    return 0;
                }
                if self.cell(r11 + bw * 2 + (1 - r1)) < BLIM {
                    self.noup(re);
                    return -1;
                }
            }
        }
        0
    }

    pub fn nodown(&mut self, al: &mut Alent) {
        if al.dy > 0 {
            al.dy = ((15 << 8) & ((16 << 8) - self.ytemp)) - 1;
            if al.dy < 0 {
                al.dy = 0;
            }
        }
        // nodownrel:
        al.downhit = 1;
    }

    pub fn nodownifplat(&mut self, al: &mut Alent) {
        if al.falling != 0xff {
            return;
        }
        if al.dy > 0 {
            al.dy = ((15 << 8) & ((16 << 8) - self.ytemp)) - 1;
            if al.dy < 0 {
                al.dy = 0;
            }
        }
        // nodownrel:
        al.downhit = 1;
    }

    pub fn noup(&mut self, al: &mut Alent) {
        if al.dy >= 0 {
            return;
        }
        al.dy = -((self.ytemp + (1 << 8)) & (15 << 8));
        al.uphit = 1;
    }

    pub fn dvcheck(&mut self, al: &mut Alent) {
        self.xtemp = al.x;
        self.ytemp = al.y;
        al.uphit = 0;
        al.downhit = 0;
        let bw = self.boardwidth;
        let r6 = self.translate(al.x, al.y + al.dy);
        // up
        if self.cell(r6 - bw) >= PLATBLIM || self.cell(r6 + 1 - bw) >= PLATBLIM {
            self.noup(al);
        }
        // down
        if self.cell(r6) >= PLATBLIM || self.cell(r6 + 1) >= PLATBLIM {
            self.nodown(al);
        }
    }

    pub fn wipealtab(&mut self) {
        self.alctr = 0;
        // _alno+1 entries, like C (the table has a spare entry, PORTING_NOTES #3)
        for a in self.al.iter_mut() {
            a.type_ = 0;
        }
        self.boardreg();
    }

    // ------------------------------------------------------------------ explosion creation

    /// alien.c `causeexplo(alent*)` / `causeexplonopyro(alent*)`.
    pub fn causeexplo_al(&mut self, i: usize) {
        let a = self.al[i];
        self.explogomaybe(a.x - a.dx, a.y - a.dy - (8 << 8), 0, 0, 1, -1, None);
    }

    pub fn causeexplonopyro_al(&mut self, i: usize) {
        let a = self.al[i];
        self.explogonopyro(a.x - a.dx, a.y - a.dy - (8 << 8), 0, 0, 1, -1, None);
    }

    pub fn explogonopyro(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        self.explocreate(r1, r2, r3, r4, r5, r6, r10);
    }

    pub fn explogonopyroquiet(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        self.explocreatequiet(r1, r2, r3, r4, r5, r6, r10);
    }

    pub fn explogoquiet(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        self.explocreatequiet(r1, r2, r3, r4, r5, r6, r10);
        self.embercreate(r1, r2, r6);
    }

    pub fn explogomaybe(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        self.explocreate(r1, r2, r3, r4, r5, r6, r10);
        if (self.random() & 3) == 0 {
            self.embercreate(r1, r2, r6);
        }
    }

    pub fn explogo(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        self.explocreate(r1, r2, r3, r4, r5, r6, r10);
        self.embercreate(r1, r2, r6);
    }

    pub fn explocreate(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        let (mut r1, mut r2) = (r1, r2);
        if let Some(k) = r10 {
            r1 = self.al[k].x;
            r2 = self.al[k].y;
        }
        let vol = 0x7f - ((r1.wrapping_sub(self.xpos).wrapping_abs() + r2.wrapping_sub(self.ypos).wrapping_abs()) >> 12);
        if vol >= 80 {
            let v = if (vol & 0x7f) > 0x7c { 0x7c } else { vol & 0x7f };
            self.bidforsound(EXPLOCHANNEL, v, (r1 - self.xpos) >> 9, SfxId::EXPLO);
        }
        // noexplosound:
        self.explocreatequiet(r1, r2, r3, r4, r5, r6, r10);
    }

    pub fn explocreatequiet(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        match r10 {
            None => {
                // noobject:
                self.softmakeobj(EXPLO, r1, r2 + (8 << 8), r3, r4, if r5 != 0 { 1 << 15 } else { 0 }, r6);
            }
            Some(k) => {
                let a = &mut self.al[k];
                a.type_ = EXPLO;
                a.dx = r3;
                a.dy = r4;
                a.r5 = 0;
            }
        }
        // noobjectins:
    }

    pub fn embercreate(&mut self, r1: i32, r2: i32, r6: i32) {
        // The original achieves the +8 by modifying r2 inside explocreatequiet.
        let r2 = r2 + (8 << 8);
        self.softmakeobj(EMBER, r1, r2, 1 << 8, -(1 << 8), 0, r6);
        self.softmakeobj(EMBER, r1, r2, -(1 << 8), -(1 << 8), 0, r6);
    }

    pub fn atomexplogo(&mut self, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32, r10: Option<usize>) {
        self.explogoquiet(r1, r2, r3, r4, r5, r6, r10);
        self.explogoquiet(r1, r2 - (16 << 8), r3, -(1 << 6), r5, r6, r10);
        self.explogoquiet(r1, r2 - (32 << 8), r3, -(1 << 7), r5, r6, r10);
        self.explogoquiet(r1 + (12 << 8), r2 - (24 << 8), 1 << 6, -(3 << 5), r5, r6, r10);
        self.explogoquiet(r1 - (12 << 8), r2 - (24 << 8), -(1 << 6), -(3 << 5), r5, r6, r10);
        let r7 = (r1 - self.xpos) >> 9;
        let mut v = 0x8f - ((r1.wrapping_sub(self.xpos).wrapping_abs() + r2.wrapping_sub(self.ypos).wrapping_abs()) >> 12);
        if v > 0x7f {
            v = 0x7f;
        }
        if v >= 60 {
            self.bidforsound(EXPLOCHANNEL, 0x7f, r7, SfxId::ATOM);
        }
        // noatomsound:
    }

    // ------------------------------------------------------------------ save / restore

    /// Writes `SAVEAREALEN*28` bytes (end marker 0xffffffff). Returns the alien slots that did
    /// not fit: only larger views can wake more objects than the store holds; then everything
    /// that cannot go back to sleep is kept, plus the aliens nearest to the player, and the
    /// caller puts the rest back to sleep ([`Sim::sleep_marker`]). When everything fits the
    /// order is the original slot order.
    pub fn save_alents(&self, store: &mut [u8]) -> Vec<usize> {
        let cap = SAVEAREALEN - 1;
        let savable: Vec<usize> =
            (0..ALNO).filter(|&i| !matches!(self.al[i].type_, EXPLO | SCOREOBJ | FLYINGBONUS | DYINGBONUS | 0)).collect();
        let (order, rest) = if savable.len() <= cap {
            (savable, Vec::new())
        } else {
            let sleeps = |i: &usize| (ALIEN1..ALIEN1 + 14).contains(&self.al[*i].type_);
            let (mut aliens, mut keep): (Vec<usize>, Vec<usize>) = savable.into_iter().partition(sleeps);
            let dist = |i: &usize| {
                let a = &self.al[*i];
                (a.x.wrapping_sub(self.xpos) as i64).abs() + (a.y.wrapping_sub(self.ypos) as i64).abs()
            };
            aliens.sort_by_key(dist);
            let room = cap.saturating_sub(keep.len()).min(aliens.len());
            let rest = aliens.split_off(room);
            keep.extend(aliens);
            keep.sort_unstable();
            keep.truncate(cap);
            (keep, rest)
        };
        let mut st = 0usize;
        for &i in &order {
            // procsaveal:
            let al = &self.al[i];
            put_le(store, st, al.type_ as u32);
            put_le(store, st + 4, al.x as u32);
            put_le(store, st + 8, al.y as u32);
            put_le(store, st + 12, al.dx as u32);
            put_le(store, st + 16, al.dy as u32);
            put_le(store, st + 20, al.r5 as u32);
            put_le(store, st + 24, al.r6 as u32);
            st += 28;
        }
        put_le(store, st, 0xffff_ffff); // end marker
        rest
    }

    /// Where and as which marker alien `i` goes back to sleep (`alsleep`: the cell above its
    /// position if empty, else its own cell if empty), or None.
    pub fn sleep_marker(&self, i: usize) -> Option<(BIdx, u8)> {
        let a = self.al[i];
        let t = a.type_ - ALIEN1;
        if !(0..14).contains(&t) {
            return None;
        }
        [self.translate(a.x - (4 << 8), a.y - (8 << 8)), self.translate(a.x, a.y)]
            .into_iter()
            .find(|&c| self.cell(c) == 0)
            .map(|c| (c, (240 + t) as u8))
    }

    /// Restores into slots 0.. (the caller wipes the table first, as in C). Only the seven
    /// saved fields are written; restored slots get a new interpolation generation.
    pub fn restore_alents(&mut self, store: &[u8]) {
        let st_end = (SAVEAREALEN - 1) * 28;
        let mut st = 0usize;
        let mut k = 0usize;
        while get_le(store, st) != 0xffff_ffff && st < st_end {
            // sl1:
            let a = &mut self.al[k];
            a.type_ = get_le(store, st) as i32;
            a.x = get_le(store, st + 4) as i32;
            a.y = get_le(store, st + 8) as i32;
            a.dx = get_le(store, st + 12) as i32;
            a.dy = get_le(store, st + 16) as i32;
            a.r5 = get_le(store, st + 20) as i32;
            a.r6 = get_le(store, st + 24) as i32;
            self.al_gen[k] = self.al_gen[k].wrapping_add(1);
            k += 1;
            st += 28;
        }
    }

    // ------------------------------------------------------------------ allocation

    pub fn softmakeobj(&mut self, r0: i32, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32) -> bool {
        let mut r10 = 0usize;
        let mut r9 = ALNO as i32;

        // (alctr < 0 cannot happen; guarded only to keep the index in range.)
        if self.alctr >= r9 || self.alctr < 0 {
            self.alctr = 0;
        }
        r9 -= self.alctr;
        if r9 < 10 {
            // check 10 object spaces
            self.alctr = 0;
            r10 = 0;
        } else {
            r10 += self.alctr as usize;
        }
        loop {
            // softloop:
            if self.al[r10].type_ == 0 {
                return self.foundmakeal(r10, self.alctr, r0, r1, r2, r3, r4, r5, r6);
            }
            r10 += 1;
            self.alctr += 1;
            r9 -= 1;
            if r9 <= 0 {
                break;
            }
        }
        true
    }

    pub fn makeobj(&mut self, r0: i32, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32) -> bool {
        let mut tmpalctr = self.alctr;
        let mut r9 = ALNO as i32;

        if tmpalctr >= r9 || tmpalctr < 0 {
            tmpalctr = 0;
        }
        r9 -= tmpalctr;
        let mut r10 = tmpalctr as usize;
        loop {
            // loop52:
            if self.al[r10].type_ == 0 {
                return self.foundmakeal(r10, tmpalctr, r0, r1, r2, r3, r4, r5, r6);
            }
            r10 += 1;
            tmpalctr += 1;
            r9 -= 1;
            if r9 <= 0 {
                break;
            }
        }
        // Wrap around: tmpalctr keeps counting past _alno, so a find here resets alctr to 0.
        r10 = 0;
        r9 = self.alctr;
        loop {
            // loop53:
            if r10 >= ALNO {
                break;
            }
            if self.al[r10].type_ == 0 {
                return self.foundmakeal(r10, tmpalctr, r0, r1, r2, r3, r4, r5, r6);
            }
            r10 += 1;
            tmpalctr += 1;
            r9 -= 1;
            if r9 <= 0 {
                break;
            }
        }
        true
    }

    pub fn foundmakeal(&mut self, i: usize, newalctr: i32, r0: i32, r1: i32, r2: i32, r3: i32, r4: i32, r5: i32, r6: i32) -> bool {
        let a = &mut self.al[i];
        a.type_ = r0;
        a.x = r1;
        a.y = r2;
        a.dx = r3;
        a.dy = r4;
        a.r5 = r5;
        a.r6 = r6;
        self.al_gen[i] = self.al_gen[i].wrapping_add(1);

        self.alctr = newalctr + 1;
        if self.alctr >= ALNO as i32 {
            self.alctr = 0;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Board;
    use crate::layout::Layout;
    use crate::sim::testutil::sim;

    /// Replace the current board by an empty `w`x`h` board (boardreg done by hand).
    fn small_board(s: &mut Sim, w: i32, h: i32) {
        s.brain = Board { width: w, height: h, header: [0; 32], cells: vec![0; (w * h) as usize] };
        s.in_neuron = false;
        s.boardwidth = w;
        s.xposmax = w << 12;
        s.yposmax = h << 12;
    }

    fn obj(type_: i32, x: i32, y: i32) -> Alent {
        Alent { type_, x, y, ..Default::default() }
    }

    #[test]
    fn makeobj_allocates_in_ring_order() {
        let mut s = sim();
        assert_eq!(s.alctr, 0);
        assert!(!s.makeobj(ALIEN1, 1, 2, 3, 4, 5, 6));
        assert_eq!(s.al[0], Alent { type_: ALIEN1, x: 1, y: 2, dx: 3, dy: 4, r5: 5, r6: 6, ..Default::default() });
        assert_eq!((s.alctr, s.al_gen[0]), (1, 1));
        assert!(!s.softmakeobj(EXPLO, 0, 0, 0, 0, 0, 0));
        assert_eq!((s.al[1].type_, s.alctr), (EXPLO, 2));
        // occupied slots are skipped
        s.al[2].type_ = EMBER;
        assert!(!s.makeobj(BOOBY, 0, 0, 0, 0, 0, 0));
        assert_eq!((s.al[3].type_, s.alctr), (BOOBY, 4));
        // a freed slot below alctr is not reused while there is room above
        s.al[0].type_ = 0;
        assert!(!s.makeobj(BOOBY, 0, 0, 0, 0, 0, 0));
        assert_eq!((s.al[4].type_, s.alctr), (BOOBY, 5));
    }

    #[test]
    fn softmakeobj_restarts_near_the_end() {
        let mut s = sim();
        s.alctr = 505; // fewer than 10 spaces left: restart at 0
        assert!(!s.softmakeobj(EMBER, 0, 0, 0, 0, 0, 0));
        assert_eq!((s.al[0].type_, s.al[505].type_, s.alctr), (EMBER, 0, 1));

        // only r9 (= _alno - alctr) slots are scanned after the restart
        let mut s = sim();
        for a in s.al.iter_mut().take(ALNO) {
            a.type_ = EMBER;
        }
        s.al[7].type_ = 0;
        s.alctr = 506; // r9 = 6: slots 0..6 only
        assert!(s.softmakeobj(EXPLO, 0, 0, 0, 0, 0, 0));
        assert_eq!(s.alctr, 6);
        assert_eq!(s.al[7].type_, 0);
        s.alctr = 505; // r9 = 7: slot 6 is the last one checked... still not 7
        assert!(s.softmakeobj(EXPLO, 0, 0, 0, 0, 0, 0));
        s.alctr = 504; // r9 = 8 reaches slot 7
        assert!(!s.softmakeobj(EXPLO, 0, 0, 0, 0, 0, 0));
        assert_eq!((s.al[7].type_, s.alctr), (EXPLO, 8));
    }

    #[test]
    fn makeobj_wraps_and_full_table_fails() {
        let mut s = sim();
        for a in s.al.iter_mut().take(ALNO) {
            a.type_ = EMBER;
        }
        s.al[3].type_ = 0;
        s.alctr = 510;
        // first loop scans 510, 511; second loop finds slot 3 with tmpalctr = 512 + 3
        assert!(!s.makeobj(ALIEN1 + 1, 0, 0, 0, 0, 0, 0));
        assert_eq!((s.al[3].type_, s.alctr), (ALIEN1 + 1, 0));
        // full table
        assert!(s.makeobj(ALIEN1, 0, 0, 0, 0, 0, 0));
        assert_eq!(s.alctr, 0);
        assert!(s.softmakeobj(EXPLO, 0, 0, 0, 0, 0, 0));
        assert_eq!(s.alctr, ALNO as i32); // the C scan leaves alctr at _alno
        assert!(s.al_gen.iter().take(ALNO).enumerate().all(|(k, &g)| g == (k == 3) as u32));
    }

    #[test]
    fn switchcolch_toggles_tables() {
        let mut s = sim();
        s.switchcolch();
        assert!(s.colchplace);
        assert_eq!((s.colch_tab, s.colch_ptr, s.oldcolch_tab, s.oldcolch_ptr), (0, 0, 0, 0));
        s.al[9] = Alent { type_: RISEPLAT, x: 100 << 8, y: 50 << 8, r5: (32 << 16) + 16, ..Default::default() };
        s.colchadd(9);
        s.bulcolchadd(9);
        assert_eq!(s.colch[0], Colch { al: 9, xmin: 84 << 8, ymin: 42 << 8, xmax: 116 << 8, ymax: 58 << 8 });
        s.switchcolch();
        assert!(!s.colchplace);
        assert_eq!((s.colch_tab, s.colch_ptr), (COLCHLIM + 16, COLCHLIM + 16));
        assert_eq!((s.oldcolch_tab, s.oldcolch_ptr, s.bulcolch_ptr), (0, 1, 0));
        // the second table holds a full _colchlim entries inside the vector
        for _ in 0..COLCHLIM + 5 {
            s.colchadd(9);
        }
        assert_eq!(s.colch_ptr, 2 * COLCHLIM + 16);
    }

    #[test]
    fn bulcolcheck_finds_boxes() {
        let mut s = sim();
        s.al[4] = obj(ALIEN1 + 1, 100 << 8, 100 << 8);
        s.al[5] = obj(ALIEN1 + 12, 200 << 8, 100 << 8);
        s.bulcolchadd(4);
        s.bulcolchaddshort(5);
        // box: +-16 px around the object
        assert_eq!(s.bulcolcheck(108 << 8, 92 << 8), Some(4));
        assert_eq!(s.bulcolcheck(116 << 8, 116 << 8), Some(4));
        assert_eq!(s.bulcolcheck(117 << 8, 100 << 8), None);
        // short box: y in [y+8-8, y+8+8]
        assert_eq!(s.bulcolcheck(200 << 8, 101 << 8), Some(5));
        assert_eq!(s.bulcolcheck(200 << 8, 99 << 8), None);
    }

    #[test]
    fn colcheck_player_rises_with_platform_under_feet() {
        let mut s = sim();
        s.xpos = 1000 << 8;
        s.ypos = 800 << 8;
        // platform centred 32 px below the player's y
        s.al[5] = Alent { type_: RISEPLAT, x: s.xpos, y: s.ypos + (32 << 8), r5: (32 << 16) + 16, ..Default::default() };
        s.colch_tab = 0;
        s.colch_ptr = 0;
        s.colchadd(5);
        let mut fa = Alent { x: s.xpos, y: s.ypos, dx: 0x100, dy: 0, ..Default::default() };
        let py = s.ypos;
        s.colcheck(&mut fa, 0, py);
        assert_eq!(fa.dy, -32); // platform dy (0) - (1<<5)
        assert_eq!(fa.dx, 0x100); // not limited horizontally when standing on it
        assert_eq!(fa.falling, 0xff);
        assert_eq!(s.platsandstr, Some(5));
        assert_eq!(s.al[5].dy, -32);
        assert_eq!(s.alonobj, 0);
    }

    #[test]
    fn colcheck_player_sandwiched_between_platforms() {
        let mut s = sim();
        s.xpos = 1000 << 8;
        s.ypos = 800 << 8;
        s.al[5] = Alent { type_: RISEPLAT, x: s.xpos, y: s.ypos + (32 << 8), r5: (32 << 16) + 16, ..Default::default() };
        s.al[6] = Alent { type_: RISEPLAT, x: s.xpos, y: s.ypos - (12 << 8), dy: 100, r5: (32 << 16) + 16, ..Default::default() };
        s.colchadd(5);
        s.colchadd(6);
        let mut fa = Alent { x: s.xpos, y: s.ypos, dx: 0x100, ..Default::default() };
        let py = s.ypos;
        s.colcheck(&mut fa, 0, py);
        // head platform: limits dx, reverses its dy, stops the player rising
        assert_eq!(fa.dx, 0);
        assert_eq!(fa.dy, 0);
        assert_eq!(s.al[6].dy, -100);
        // *platsandstr = 1<<8 on the platform under the player
        assert_eq!(s.al[5].dy, 1 << 8);
    }

    #[test]
    fn colcheck_alien_on_downplat_and_norise() {
        let mut s = sim();
        let (ax, ay) = (500 << 8, 600 << 8);
        s.al[8] = Alent { type_: DOWNPLAT, x: ax, y: ay + (24 << 8), r5: (32 << 16) + 16, ..Default::default() };
        s.colchadd(8);
        s.switchcolch(); // aliens check against the previous table
        s.al[3] = Alent { type_: ALIEN1 + 1, x: ax, y: ay, dx: 300, dy: 512, ..Default::default() };
        s.al_colcheck(3, 1);
        assert_eq!(s.al[3].dx, 0); // horizontal limit (limr2)
        assert_eq!(s.al[3].dy, 16); // capped to the falling platform speed
        assert_eq!(s.al[8].dy, 16);
        assert_eq!(s.alonobj, 1);
        assert_eq!(s.platsandstr, None);

        // colchecktype 2: dontrise
        s.al[3].dy = 400;
        s.al_colcheck(3, 2);
        assert_eq!(s.al[3].dy, 0);
        assert_eq!(s.al[8].dy, 16);
    }

    #[test]
    fn colcheck_alias_with_stale_entry_of_own_slot() {
        // Slot 3 held a platform last frame and now holds the alien being checked. In C
        // `al` and `r6` are then the same object: headonroof's `r6->y = platypos+(32<<8)`
        // moves the alien itself and `r6->dy = r0` overwrites its dy.
        let mut s = sim();
        small_board(&mut s, 64, 64);
        s.al[3] = Alent { type_: DOWNPLAT, x: 500 << 8, y: 600 << 8, r5: (32 << 16) + 16, ..Default::default() };
        s.colchadd(3);
        s.switchcolch();
        s.al[3] = Alent { type_: ALIEN1 + 1, x: 500 << 8, y: 576 << 8, dx: 0, dy: 64, ..Default::default() };
        let roof = s.translate(500 << 8, 576 << 8) - 64;
        s.set_cell(roof, 128);
        s.al_colcheck(3, 1);
        assert_eq!(s.al[3].type_, ALIEN1 + 1);
        assert_eq!(s.al[3].y, (576 - 8 + 32) << 8);
        assert_eq!(s.al[3].dy, 1);
        assert_eq!(s.alonobj, 1);
    }

    #[test]
    fn albcheck_on_small_board() {
        let mut s = sim();
        small_board(&mut s, 8, 8);
        // alien whose check point (x-1px, y-8px) is at column 2, 4 px into row 2
        let mut a = Alent { type_: ALIEN1 + 1, x: (2 << 12) + (1 << 8), y: (2 << 12) + (12 << 8), dx: 0, dy: 1 << 8, ..Default::default() };
        s.set_cell(26, 128); // solid block below (row 3, col 2)
        s.set_cell(34, PLATLOWLIM); // platform two rows below the check row
        let r5 = s.albcheck(&mut a);
        assert_eq!(r5, 34);
        assert_eq!(s.alonplat, 1);
        assert_eq!(a.downhit, 1);
        assert_eq!(a.dy, 0xbff); // up to the cell boundary minus 1
        assert_eq!((a.lefthit, a.righthit, a.uphit), (0, 0, 0));
        assert_eq!((s.xtemp, s.ytemp), (2 << 12, (2 << 12) + (4 << 8)));

        // walking left into a wall, jumping into a ceiling
        let mut b = Alent { type_: ALIEN1 + 1, x: (2 << 12) + (1 << 8), y: (2 << 12) + (12 << 8), dx: -(1 << 8), dy: -(1 << 8), ..Default::default() };
        s.set_cell(26, 0);
        s.set_cell(17, 130); // left of the check cell (row 2, col 1)
        s.set_cell(10, 129); // above (row 1, col 2)
        s.albcheck(&mut b);
        assert_eq!((b.lefthit, b.dx), (1, 0));
        assert_eq!(b.uphit, 1);
        assert_eq!(b.dy, -0x500); // -((ytemp + 1px) & (15<<8))
        assert_eq!(s.alonplat, 0);
    }

    #[test]
    fn exploins_lifetime_and_plotting() {
        let mut s = sim();
        s.plotal = false;
        s.al[3] = obj(EXPLO, 100 << 8, 100 << 8);
        let mut n = 0;
        while s.al[3].type_ != 0 {
            s.exploins(3);
            n += 1;
            assert!(n < 100);
        }
        assert_eq!(n, 32); // explo frame length 8<<2 at explospeed 1
        assert!(s.cmds.is_empty());

        // explo moves; from r5 = 5 on the player contact test is skipped
        s.al[3] = Alent { type_: EXPLO, x: 100 << 8, y: 100 << 8, dx: 3, dy: -2, r5: 5, ..Default::default() };
        s.plotal = true;
        s.explo(3);
        assert_eq!((s.al[3].x, s.al[3].y, s.al[3].r5), ((100 << 8) + 3, (100 << 8) - 2, 6));
        let c = *s.cmds.last().unwrap();
        assert_eq!((c.bank, c.idx, c.pos), (Bank::Explo, 1, Pos::World { x: 100, y: 99 }));
        let mut n = 1;
        while s.al[3].type_ != 0 {
            s.explo(3);
            n += 1;
        }
        assert_eq!(n, 27);
    }

    #[test]
    fn explosion_creation_quiet() {
        let mut s = sim();
        s.explogoquiet(10 << 8, 20 << 8, 1, 2, 1, 7, None);
        assert_eq!(s.al[0], Alent { type_: EXPLO, x: 10 << 8, y: 28 << 8, dx: 1, dy: 2, r5: 1 << 15, r6: 7, ..Default::default() });
        assert_eq!(s.al[1], Alent { type_: EMBER, x: 10 << 8, y: 28 << 8, dx: 1 << 8, dy: -(1 << 8), r5: 0, r6: 7, ..Default::default() });
        assert_eq!((s.al[2].type_, s.al[2].dx), (EMBER, -(1 << 8)));
        // existing object becomes an explosion in place
        s.al[2].r5 = 99;
        s.explocreatequiet(0, 0, 5, 6, 0, 0, Some(2));
        assert_eq!((s.al[2].type_, s.al[2].dx, s.al[2].dy, s.al[2].r5), (EXPLO, 5, 6, 0));
        assert_eq!(s.alctr, 3);
    }

    #[test]
    fn save_restore_round_trip() {
        let mut s = sim();
        s.al[0] = Alent { type_: ALIEN1 + 1, x: 1, y: 2, dx: -3, dy: 4, r5: -5, r6: 6, ..Default::default() };
        s.al[1] = obj(EXPLO, 9, 9);
        s.al[2] = Alent { type_: RISEPLAT, x: 0x1234_5678, y: -1, dx: 0, dy: 7, r5: (32 << 16) + 16, r6: 176, ..Default::default() };
        s.al[3] = obj(SCOREOBJ, 9, 9);
        s.al[4] = obj(FLYINGBONUS, 9, 9);
        s.al[5] = obj(DYINGBONUS, 9, 9);
        s.al[6] = obj(DECORATION, 3, 4);
        let mut store = vec![0u8; SAVEAREALEN * 28];
        s.save_alents(&mut store);
        assert_eq!(store[0..4], (ALIEN1 + 1).to_le_bytes());
        assert_eq!(store[20..24], (-5i32).to_le_bytes());
        assert_eq!(store[28..32], RISEPLAT.to_le_bytes());
        assert_eq!(store[32..36], 0x1234_5678i32.to_le_bytes());
        assert_eq!(store[56..60], DECORATION.to_le_bytes());
        assert_eq!(store[84..88], [0xff; 4]);

        let mut t = sim();
        t.restore_alents(&store);
        let core = |a: &Alent| (a.type_, a.x, a.y, a.dx, a.dy, a.r5, a.r6);
        assert_eq!(core(&t.al[0]), core(&s.al[0]));
        assert_eq!(core(&t.al[1]), core(&s.al[2]));
        assert_eq!(core(&t.al[2]), core(&s.al[6]));
        assert_eq!(t.al[3].type_, 0);
        assert_eq!(&t.al_gen[..4], &[1, 1, 1, 0]);
    }

    #[test]
    fn full_store_keeps_the_nearest_aliens_and_reports_the_rest() {
        let mut s = sim();
        s.xpos = 0;
        s.ypos = 0;
        // 100 aliens, slot k at x = k, plus a moving platform in a high slot
        for k in 0..100 {
            s.al[k] = obj(ALIEN1 + 2, k as i32, 0);
        }
        s.al[110] = Alent { type_: RISEPLAT, x: 5000, y: 0, r5: (32 << 16) + 16, r6: 176, ..Default::default() };
        let mut store = vec![0u8; SAVEAREALEN * 28];
        let rest = s.save_alents(&mut store);
        let end = (SAVEAREALEN - 1) * 28;
        assert_eq!(store[end..end + 4], [0xff; 4], "the store is full");
        let mut t = sim();
        t.restore_alents(&store);
        // the platform (cannot sleep) is kept, then the 76 nearest aliens, in slot order
        let kept: Vec<i32> = t.al[..SAVEAREALEN - 1].iter().map(|a| a.x).collect();
        assert_eq!(kept[..76], (0..76).collect::<Vec<i32>>()[..]);
        assert_eq!((t.al[76].type_, t.al[76].x), (RISEPLAT, 5000));
        assert_eq!(rest, (76..100).collect::<Vec<usize>>());
        // fewer objects than the store holds: the original slot order, nothing left over
        let mut u = sim();
        u.al[3] = obj(ALIEN1 + 2, 9, 0);
        u.al[1] = obj(ALIEN1 + 2, 7, 0);
        assert!(u.save_alents(&mut store).is_empty());
        let mut v = sim();
        v.restore_alents(&store);
        assert_eq!((v.al[0].x, v.al[1].x), (7, 9));
    }

    #[test]
    fn leftover_aliens_go_back_to_sleep_on_neuron_entry() {
        let mut s = sim();
        s.xpos = 2000 << 8;
        s.ypos = 2000 << 8;
        for k in 0..90 {
            let (x, y) = ((2000 + 40 * k as i32) << 8, (2000 + 8) << 8);
            s.al[k] = Alent { type_: ALIEN1 + 3, x, y, r6: 50, ..Default::default() };
            let c = s.translate(x - (4 << 8), y - (8 << 8));
            s.set_cell(c, 0);
        }
        s.saveal(false);
        // the 13 farthest are marked in the brain as sleeping ALIEN1+3 (marker 243)
        let woken: Vec<usize> = (77..90).collect();
        let _ = woken;
        let markers = (0..90)
            .filter(|&k| {
                let a = s.al[k];
                s.cell(s.translate(a.x - (4 << 8), a.y - (8 << 8))) == 243
            })
            .count();
        assert_eq!(markers, 90 - (SAVEAREALEN - 1));
    }

    #[test]
    fn procal_deletes_offscreen_objects_except_decorations() {
        let mut s = sim();
        s.al[4] = obj(ALIEN1 + 1, 0, 100 << 12); // x <= xlowlim
        s.procal(4);
        assert_eq!(s.al[4].type_, 0);
        s.al[4] = obj(EMBER, 10 << 12, s.yposmax); // y >= yposmax
        s.procal(4);
        assert_eq!(s.al[4].type_, 0);
        s.al[4] = obj(SCOREOBJ, 10 << 12, YLOWLIM); // y <= ylowlim
        s.procal(4);
        assert_eq!(s.al[4].type_, 0);
        s.al[4] = obj(SCOREOBJ, s.xposmax, 10 << 12); // x >= xposmax
        s.procal(4);
        assert_eq!(s.al[4].type_, 0);

        // decorations use centre-relative coordinates and survive
        s.masterplotal = true;
        s.al[6] = Alent { type_: DECORATION, x: 1 << 8, y: -(10 << 8), dx: 1 << 8, dy: -(2 << 8), r5: (5 << 16) | 70, r6: 1, ..Default::default() };
        s.procal(6);
        assert_eq!((s.al[6].type_, s.al[6].r6, s.al[6].r5), (DECORATION, 0, (4 << 16) | 70));
        let c = *s.cmds.last().unwrap();
        assert_eq!((c.bank, c.idx, c.pos, c.clip), (Bank::Bloke, 70, Pos::Center { x: 0, y: -10 }, Clip::Game));
        s.procal(6); // r6 < 0: starts moving with gravity
        assert_eq!((s.al[6].x, s.al[6].y, s.al[6].dy), (2 << 8, -(12 << 8), -(2 << 8) + 16));
    }

    #[test]
    fn moval_plot_range_original_and_wide() {
        let mut s = sim();
        s.xpos = 1000 << 8;
        s.ypos = 800 << 8;
        s.moval();
        assert_eq!((s.sprlx, s.sprhx), ((1000 - 160) << 8, (1000 + 160) << 8));
        assert_eq!((s.sprly, s.sprhy), ((800 - 112) << 8, (800 + 112) << 8));

        // a score object 200 px right of the player: plotted only in a larger view
        s.masterplotal = true;
        s.al[7] = Alent { type_: SCOREOBJ, x: (1200 << 8), y: 800 << 8, r5: 0x1060, ..Default::default() };
        s.moval();
        assert!(s.cmds.is_empty());
        assert_eq!(s.plot_key, KEY_NONE);

        s.lay = Layout::new(640.0, 256.0); // gamew 608 -> 38 blocks
        s.moval();
        assert_eq!((s.sprlx, s.sprhx), ((1000 - 320) << 8, (1000 + 320) << 8));
        assert_eq!((s.sprly, s.sprhy), ((800 - 112) << 8, (800 + 112) << 8));
        let c = *s.cmds.last().unwrap();
        assert_eq!((c.bank, c.idx, c.key), (Bank::Bloke, 0x10, KEY_ALIEN + 7));
        assert_eq!(s.anchors[(KEY_ALIEN + 7) as usize].gen, s.al_gen[7]);
        assert_eq!(s.plot_key, KEY_NONE);

        // an odd window width rounds up to whole blocks (2560x1080 at 1.0x: gamew 575 → 36)
        s.lay = Layout::new(607.0, 256.0);
        s.moval();
        assert_eq!(s.sprhx - s.xpos, ((36 + 2) * 8) << 8);
        // the plot range always reaches past both window edges
        for w in [423.0, 575.0, 591.0, 1365.0, 1820.0] {
            s.lay = Layout::new(w + 32.0, 256.0);
            s.moval();
            let l = s.lay;
            assert!((s.sprhx - s.xpos) >> 8 >= (l.gamex + l.gamew) - (l.gcx() - 1) + 15, "right edge at {w}");
            assert!((s.xpos - s.sprlx) >> 8 >= (l.gcx() - 1) - l.gamex + 15, "left edge at {w}");
        }
    }

    #[test]
    fn sleep_range_grows_with_the_window() {
        let mut s = sim();
        s.xpos = 3000 << 8;
        s.ypos = 2000 << 8;
        let walker = |s: &mut Sim, x: i32, y: i32| {
            // alsleep only parks an alien in an empty cell
            let c = s.translate((x - 4) << 8, (y - 8) << 8);
            s.set_cell(c, 0);
            Alent { type_: ALIEN1 + 1, x: x << 8, y: y << 8, r6: 100, ..Default::default() }
        };
        // original window: 520 px away falls asleep
        s.al[3] = walker(&mut s, 3000 + 520, 2000);
        s.al_sleepcheck(3, 2, SLEEPRANGE);
        assert_eq!(s.al[3].type_, 0, "asleep in the original window");
        // 3.0x on 21:9 (game window 1788x704): still awake 700 px to the side and 500 px up,
        // asleep beyond the window plus the original margin
        s.lay = Layout::new(1820.0, 768.0);
        s.al[3] = walker(&mut s, 3000 + 700, 2000 - 500);
        s.al_sleepcheck(3, 2, SLEEPRANGE);
        assert_ne!(s.al[3].type_, 0, "awake on screen");
        s.al[3] = walker(&mut s, 3000 + 512 + 750 + 1, 2000);
        s.al_sleepcheck(3, 2, SLEEPRANGE);
        assert_eq!(s.al[3].type_, 0);
        // every alien the wake-up perimeter can wake stays awake on the next check
        let (bw, bh) = crate::sim::maze::wake_box(&s.lay);
        s.al[3] = walker(&mut s, 3000 + 16 * (bw / 2) + 8, 2000 + 16 * (bh / 2) + 8);
        s.al_sleepcheck(3, 2, SLEEPRANGE);
        assert_ne!(s.al[3].type_, 0);
    }

    #[test]
    fn projhital_bonus_numbers() {
        let mut s = sim();
        s.al[2] = Alent { type_: ALIEN1 + 13, r6: 100, ..Default::default() };
        assert_eq!(s.projhital(2, 50), 0);
        assert_eq!(s.al[2].r6, 50);
        let b = s.projhital(2, 50);
        // (random&7) + 13 + 16 = 29..36; above _bonushigh -> _bonushigh-3
        assert!((28..=BONUSHIGH as i32).contains(&b));
        s.al[2] = Alent { type_: RISEPLAT, r6: 1, ..Default::default() };
        let b = s.projhital(2, 1);
        assert!((BONUSLOW as i32..BONUSLOW as i32 + 8).contains(&b));
    }

    #[test]
    fn hamster_and_scoreobj() {
        let mut s = sim();
        s.al[1] = Alent { type_: ALIEN1 + 13, dx: 5, dy: 6, r6: 1, ..Default::default() };
        s.hamsterspecial(1);
        assert_eq!((s.al[1].type_, s.al[1].r6, s.al[1].dx, s.al[1].dy), (ALIEN1 + 11, 12 << 8, 0, 0));
        s.plotal = false;
        s.al[2] = Alent { type_: SCOREOBJ, x: 0, y: 0, dx: 2, dy: 0, r5: 0x1002, ..Default::default() };
        s.scoreobj(2);
        assert_eq!((s.al[2].x, s.al[2].dy, s.al[2].r5, s.al[2].type_), (2, 32, 0x1001, SCOREOBJ));
        s.scoreobj(2);
        assert_eq!(s.al[2].type_, 0);
    }
}
