//! maze.c — board helpers, bombs, bonuses, electricity, wake-up, teleport search, fuel-air.
//! (`mazeplot`, `draw_block`, `backdrop` are rendering → render/world.rs.)
#![allow(clippy::too_many_arguments)]

use crate::assets::BIdx;
use crate::audio::sfx::SfxId;
use crate::i18n::keys;
use crate::sim::*;

#[inline]
pub fn block_anim(b: u8) -> bool {
    (ANIMLOWLIM..=ANIMHIGHLIM).contains(&b)
}
#[inline]
pub fn block_solid(b: u8) -> bool {
    (SOLIDLOWLIM..=SOLIDHIGHLIM).contains(&b)
}
#[inline]
pub fn block_crumblestand(b: u8) -> bool {
    (CRUMBLESTANDLOWLIM..=CRUMBLESTANDHIGHLIM).contains(&b)
}
#[inline]
pub fn block_bomb(b: u8) -> bool {
    (BOMBLOWLIM..=BOMBHIGHLIM).contains(&b)
}
#[inline]
pub fn block_target(b: u8) -> bool {
    (TARGETLOWLIM..=TARGETHIGHLIM).contains(&b)
}
#[inline]
pub fn block_booby(b: u8) -> bool {
    (BOOBYLOWLIM..=BOOBYHIGHLIM).contains(&b)
}
/// Note: `b < _gashighlim` (only 54) exactly like the C code.
#[inline]
pub fn block_gas(b: u8) -> bool {
    (GASLOWLIM..GASHIGHLIM).contains(&b)
}
/// Weapon number 1..=16 or 0.
#[inline]
pub fn block_weapon(b: u8) -> i32 {
    if !(WEAPLOWLIM..=WEAPHIGHLIM).contains(&b) {
        0
    } else {
        (b - WEAPLOWLIM + 1) as i32
    }
}

/// maze.c `boobyvectab`.
const BOOBYVECTAB: [i32; 8] = [-0x400, -0x80, 0x400, -0x80, 0, -0x400, 0, 0x400];

/// Wake-up perimeter size in blocks (width, height) for a layout: original 20x16, grown to
/// the visible window + the original margin in larger views (PORTING_NOTES #12).
pub fn wake_box(lay: &crate::layout::Layout) -> (i32, i32) {
    // half the visible window in blocks (original 9 and 6)
    let vbx = (lay.gamew + 31) / 32;
    let vby = (lay.gameh + 31) / 32;
    (BOXWIDTH.max(2 * (vbx + 1)), BOXHEIGHT.max(2 * (vby + 2)))
}

/// Extra alien sleep range in pixels of a game window larger than the original 288x192
/// (half the extra size, so aliens on screen stay awake; PORTING_NOTES #12).
pub fn sleep_extra(lay: &crate::layout::Layout) -> (i32, i32) {
    ((lay.gamew - 288).max(0) / 2, (lay.gameh - 192).max(0) / 2)
}

/// screenwakeup box size in blocks (width, height): original 40x32. Larger views grow it
/// with the wake-up box, but never beyond the sleep range (as in the original, where the
/// box lies inside it): a woken alien is never put back to sleep on its first update.
pub fn screen_wake_box(lay: &crate::layout::Layout) -> (i32, i32) {
    let (bw, bh) = wake_box(lay);
    let (ex, ey) = sleep_extra(lay);
    // farthest woken alien centre: column centre col*16+8 with xpos up to a block away,
    // row position row*16+7
    let hx_max = ((SLEEPRANGE >> 8) + ex - 8) / 16;
    let hy_max = ((SLEEPRANGE >> 8) + ey - 9) / 16;
    ((BOXWIDTH * 2).max((2 * bw).min(2 * hx_max)), (BOXHEIGHT * 2).max((2 * bh).min(2 * hy_max)))
}

/// Callback for the wake-up scans (`dowakeup` in the game; replaceable in tests).
type WakeFn = fn(&mut Sim, BIdx);

impl Sim {
    pub fn fuelairproc(&mut self) {
        if self.framectr < self.fuelairlastframe + ((FUELAIRDURATION - self.fueltabctr) >> 1) {
            return;
        }
        self.fuelairlastframe = self.framectr;

        // ftr = fueltabread; fueltabread = ftw = fueltabwrite; fueltabwrite = ftr;
        let ftr = self.fuel_read;
        let ftw = self.fuel_write;
        self.fuel_read = ftw;
        self.fuel_write = ftr;
        let mut r = 0usize; // ftr cursor
        let mut w = 0usize; // ftw cursor

        self.fueltabctr -= 1;
        if self.fueltabctr < 0 {
            self.fueltab[ftw][w] = FUEL_NULL;
            return;
        }

        let bw = self.boardwidth;
        // `r7 = ftw + _fueltablim; r7 > ftw` is checked only before each entry is read and
        // one entry can add 4 cells, so `w` can reach _fueltablim+3 (NULL at +3). Each table
        // has 4 private guard entries: the overflow entries stay in their own table and are
        // read as normal front cells on the next pass; reads past the end give NULL. (C wrote
        // them into `fueltabofs[1][0..3]` when writing table 0, and past the array — UB —
        // when writing table 1.) PORTING_NOTES #35.
        while FUELTABLIM > w {
            let r8 = self.fueltab[ftr].get(r).copied().unwrap_or(FUEL_NULL);
            r += 1;
            if r8 == FUEL_NULL {
                self.fueltab[ftw][w] = FUEL_NULL;
                return;
            }
            let r0 = self.cell(r8) & !1;
            if r0 != GASLOWLIM {
                continue; // skipfuelair
            }
            for r1 in [r8.wrapping_sub(1), r8.wrapping_add(bw), r8.wrapping_add(1), r8.wrapping_sub(bw)] {
                if self.cell(r1) == 0 {
                    self.set_cell(r1, GASLOWLIM);
                    self.fueltab[ftw][w] = r1;
                    w += 1;
                }
            }
        }
        // fuelprocdone:
        self.fueltab[ftw][w] = FUEL_NULL;
    }

    pub fn prepfueltab(&mut self) {
        self.fuel_read = 0;
        self.fuel_write = 1;
        self.fueltab[0][0] = FUEL_NULL;
        self.fueltab[1][0] = FUEL_NULL;
        self.fueltabctr = 0;
        self.fuelairlastframe = 0;
    }

    /// Returns true if handled (C returns 1).
    pub fn embertrybomb(&mut self, r0: BIdx, al: usize) -> bool {
        if block_bomb(self.cell(r0)) {
            self.emberbomb(r0, al);
            true
        } else {
            false
        }
    }

    pub fn embertrybombtarget(&mut self, r0: BIdx, al: usize) -> bool {
        let b = self.cell(r0);
        if block_bomb(b) || block_target(b) {
            self.emberbomb(r0, al);
            true
        } else {
            false
        }
    }

    pub fn embertrybooby(&mut self, r0: BIdx, al: usize) -> bool {
        if block_booby(self.cell(r0)) {
            self.emberbomb(r0, al);
            true
        } else {
            false
        }
    }

    pub fn embertrygas(&mut self, r0: BIdx, al: usize) -> bool {
        if block_gas(self.cell(r0)) {
            self.emberbomb(r0, al);
            true
        } else {
            false
        }
    }

    pub fn emberbomb(&mut self, r0: BIdx, al: usize) {
        let b = self.cell(r0);
        if !(ATOMLOWLIM..=ATOMHIGHLIM).contains(&b) {
            self.normbomb(r0, al);
        } else {
            self.atombomb(r0);
        }
    }

    pub fn normbombsurvive(&mut self, r0: BIdx) {
        let b = self.cell(r0);
        if b == FUELAIRNO || b == FUELAIRNO + 1 {
            self.fuelbomb(r0);
            return;
        }
        // normbombins:
        self.destroy(r0);
    }

    pub fn fuelbomb(&mut self, r0: BIdx) {
        let mut r0 = r0;
        if self.cell(r0) != FUELAIRNO {
            r0 -= 1;
        }
        self.set_cell(r0, 0);
        self.set_cell(r0 + 1, 0);
        let (x, y) = self.backtranslate(r0);
        self.explogo(x + (8 << 8), y, 0, -(1 << 7), 0, 0, None);
    }

    pub fn seeifwind(&mut self, r1: BIdx, dx: &mut i32, dy: &mut i32, retval: bool) -> bool {
        let mut retval = retval;
        let b = self.cell(r1);
        if b == 2 {
            *dx -= WINDSPEED * 4;
            retval = true;
        }
        if b == 3 {
            *dx += WINDSPEED * 4;
            retval = true;
        }
        if b == 12 {
            *dy -= WINDSPEED;
            retval = true;
        }
        if b == 13 {
            *dy += WINDSPEED;
            retval = true;
        }
        retval
    }

    pub fn crumblecheck(&mut self, r1: BIdx) {
        let b = self.cell(r1);
        if !block_crumblestand(b) {
            return;
        }
        let b = b.wrapping_add(1);
        self.set_cell(r1, if b & 3 == 0 { 0 } else { b });
    }

    /// `translate(r0, r1)` = contents + (r0>>12) + (r1>>12)*width.
    pub fn translate(&self, r0: i32, r1: i32) -> BIdx {
        (r0 >> 12).wrapping_add((r1 >> 12).wrapping_mul(self.bd().width))
    }

    pub fn fntranslate(&self, r0: i32, r1: i32) -> BIdx {
        (r0.wrapping_add(8 << 8) >> 12).wrapping_add((r1.wrapping_sub(8 << 8) >> 12).wrapping_mul(self.bd().width))
    }

    /// Returns (x, y) in 8.8 world units (cell << 12). Uses C `%`/`/` (truncating).
    pub fn backtranslate(&self, r: BIdx) -> (i32, i32) {
        let r1 = self.boardwidth;
        if r1 == 0 {
            // C would trap (division by zero); boards always have a width.
            return (0, 0);
        }
        (r.wrapping_rem(r1) << 12, r.wrapping_div(r1) << 12)
    }

    pub fn plbombcheck(&mut self, r5: BIdx) -> i32 {
        let mut r1 = self.bombcheck(r5);
        if block_target(self.cell(r5)) {
            r1 = self.normalbomb(r5);
        }
        // nopltarg:
        r1
    }

    pub fn bombcheck(&mut self, r5: BIdx) -> i32 {
        let r0 = self.cell(r5);
        if !(BOMBLOWLIM..=BOMBHIGHLIM).contains(&r0) {
            return 0;
        }
        if r0 == FUELAIRNO || r0 == FUELAIRNO + 1 {
            return self.fuelairbomb(r5);
        }
        if r0 >= BOOBYLOWLIM {
            return self.boobybomb(r5);
        }
        if !(ATOMLOWLIM..=ATOMHIGHLIM).contains(&r0) {
            return self.normalbomb(r5);
        }
        self.atombomb(r5)
    }

    /// Recursive (bounded by atombombctr < 0x1f).
    pub fn atombomb(&mut self, r5: BIdx) -> i32 {
        if self.atombombctr >= 0x1f {
            // atomabandon:
            return ATOMLOSS;
        }
        self.atombombctr = self.atombombctr.wrapping_add(1);
        let r0 = self.cell(r5).wrapping_sub(ATOMLOWLIM);
        let bw = self.boardwidth;
        let mut r5 = r5;
        if r0 & 1 != 0 {
            r5 -= 1;
        }
        if r0 & 2 != 0 {
            r5 -= bw;
        }

        // delete this a-bomb
        self.set_cell(r5, 0);
        self.set_cell(r5 + 1, 0);
        self.set_cell(r5 + bw, 0);
        self.set_cell(r5 + bw + 1, 0);
        self.procatom(r5 - bw);
        self.procatom(r5 - bw + 1);
        self.procatom(r5 - 1);
        self.procatom(r5 + 2);
        self.procatom(r5 + bw - 1);
        self.procatom(r5 + bw + 2);
        self.procatom(r5 + 2 * bw);
        self.procatom(r5 + 2 * bw + 1);
        let (x, y) = self.backtranslate(r5);
        self.atomexplogo(x + (8 << 8), y + (24 << 8), 0, 0, 0, 0, None);
        ATOMLOSS
    }

    pub fn procatom(&mut self, r5: BIdx) {
        let r0 = self.cell(r5);
        if block_target(r0) {
            self.shoottarget(r5);
        }
        // noatomtarget:
        self.set_cell(r5, 0);
        if !(ATOMLOWLIM..=ATOMHIGHLIM).contains(&r0) {
            return;
        }
        self.set_cell(r5, r0); // replace bomb segment
        self.atombomb(r5);
    }

    pub fn fuelairbomb(&mut self, r5: BIdx) -> i32 {
        self.deletetwin(r5);
        let r0 = r5 - self.boardwidth;
        self.set_cell(r0, GASLOWLIM);
        self.set_cell(r0 + 1, GASLOWLIM);
        let t = self.fuel_read;
        if self.fueltab[t][0] == FUEL_NULL || self.fueltab[t][1] == FUEL_NULL {
            self.fueltab[t][2] = FUEL_NULL; // replace end marker if overwritten
        }
        self.fueltab[t][0] = r0;
        self.fueltab[t][1] = r0 + 1;
        self.fueltabctr = FUELAIRDURATION;
        self.bidforsound(EXPLOCHANNEL, 0x6f, 0, SfxId::FUELAIR);
        0
    }

    pub fn normalbomb(&mut self, r5: BIdx) -> i32 {
        self.elecdestroy(r5);
        self.set_cell(r5, 0);
        let (x, y) = self.backtranslate(r5);
        self.explogo(x, y + (8 << 8), 0, 0, 0, 0, None);
        BOMBLOSS
    }

    pub fn boobybomb(&mut self, r5: BIdx) -> i32 {
        let k = (2 * ((self.cell(r5) as i32 - BOOBYLOWLIM as i32) & 3)) as usize;
        let (v0, v1) = (BOOBYVECTAB[k], BOOBYVECTAB[k + 1]);
        self.set_cell(r5, 0);
        let (x, y) = self.backtranslate(r5);
        self.explogonopyro(x, y + (8 << 8), -(v0 >> 2), -(v1 >> 2), 0, 0, None);
        self.makeobj(BOOBY, x, y + (8 << 8), v0, v1, 0, 0);
        BOMBLOSS
    }

    pub fn bonuslim(&mut self, r5: BIdx) {
        let b = self.cell(r5);
        if (NEURONLOWLIM..=NEURONHIGHLIM).contains(&b) {
            self.plsetneuronzone(b as i32 - (NEURONLOWLIM as i32 - 1));
            self.set_cell(r5, 0);
            return;
        }
        // noneuron:
        if (ELECLOWLIM..=ELECHIGHLIM).contains(&b) {
            self.electrocute(r5);
            return;
        }
        // bonuslimcont:
        let b = self.cell(r5);
        if !(BONUSLOW..=MEGABONUSLIM).contains(&b) {
            return;
        }
        if b > BONUSHIGH {
            self.megabonus(r5);
        } else {
            self.bonusgot(r5);
        }
    }

    pub fn deadbonuslim(&mut self, r5: BIdx) {
        let r0 = self.cell(r5);
        if !(BONUSHIGH - 2..=BONUSHIGH).contains(&r0) {
            return;
        }
        self.bonusgot(r5);
    }

    pub fn bonusgot(&mut self, r5: BIdx) {
        let r0 = self.cell(r5) as i32;
        self.set_cell(r5, 0);
        let (x, y) = self.backtranslate(r5);
        self.bonuscommon(r0 - BONUSLOW as i32, x, y);
    }

    pub fn megabonus(&mut self, r5: BIdx) {
        let r0 = self.cell(r5);
        self.set_cell(r5, 0);
        self.bidforsound(PLAYERCHANNEL, 0x7e, 127, SfxId::BONUS_1);
        self.bidforsound(SPARECHANNEL, 0x7e, -127, SfxId::BONUS_2);
        let (numb, key, score) = if r0 == BONUSHIGH + 1 {
            (10, keys::MSG_BONUS_10K, 10000)
        } else if r0 == BONUSHIGH + 2 {
            (20, keys::MSG_BONUS_20K, 20000)
        } else if r0 == BONUSHIGH + 3 {
            (30, keys::MSG_BONUS_30K, 30000)
        } else if r0 == BONUSHIGH + 4 {
            (50, keys::MSG_BONUS_50K, 50000)
        } else {
            (100, keys::MSG_BONUS_100K, 100000)
        };
        self.bonusnumb(numb);
        self.message_hud_bottom(96, 224, 0.0, -2.0, TextRef::Key(key), Align::EnCentered);
        self.addtoscore(score);
    }

    pub fn destroy(&mut self, r5: BIdx) {
        self.bombcheck(r5);

        let r0 = self.cell(r5);
        if r0 == GASLOWLIM || r0 == GASHIGHLIM {
            self.normalbomb(r5);
            return;
        }
        if (BONUSLOW..=BONUSHIGH).contains(&r0) {
            let (x, mut y) = self.backtranslate(r5);
            y += 8 << 8;
            let r5v = self.cell(r5) as i32 | (1 << 20);
            self.makeobj(DYINGBONUS, x, y + (7 << 8), 0, 0, r5v, 666);
            self.set_cell(r5, 0);
        }
        // noshootbonus:
        if block_target(r0) {
            self.shoottarget(r5);
            return;
        }
        // noshoottarget:
        if (CRUMBLELOWLIM..=CRUMBLEHIGHLIM).contains(&r0) {
            let b = self.cell(r5).wrapping_add(1);
            self.set_cell(r5, if b & 3 == 0 { 0 } else { b });
        }
        // nocrumble:
    }

    /// `atomrocket(projent* r11, char* r0)`; `proj` = projectile slot.
    pub fn atomrocket(&mut self, proj: usize, r0: BIdx) {
        let r1 = self.cell(r0);
        if block_solid(r1) {
            self.causeexplo_proj(proj);
            return;
        }
        // atomdest:
        if (ELECLOWLIM..=ELECHIGHLIM).contains(&r1) || block_target(r1) {
            self.elecdestroy(r0);
        }
        // noelecatom2:
        if (NEURONLOWLIM..=NEURONHIGHLIM).contains(&r1) {
            self.causeexplo_proj(proj);
            return;
        }
        // notreasatom:
        self.set_cell(r0, 0);
        self.causeexplo_proj(proj);
    }

    pub fn foundtarget(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> bool {
        let mut target = self.cell(self.translate(x, y));
        if !block_target(target) {
            return false;
        }
        let y = y.wrapping_add(16 << 8); // aim at body
        let r0 = self.translate(x + (8 << 8) + (dx >> 4), y - (8 << 8) + (dy >> 4));
        if self.cell(r0) > TARGETHIGHLIM {
            return true;
        }
        target &= !3;
        let mut r4 = self.random() & 7;
        if r4 == 7 {
            r4 = 6;
        }
        if target >= POWERTARGET {
            r4 -= 6;
        }
        if target == POWERTARGET {
            r4 = 8 + (r4 & 2);
        }
        if target == NUTTERTARGET {
            r4 = 12 + (r4 & 3);
        }
        self.makebul(x, y, dx, dy, r4, (1 << 8) * BULL_TTL);
        true
    }

    pub fn shoottarget(&mut self, r5: BIdx) {
        let r3 = self.cell(r5);
        self.set_cell(r5, 0);
        self.addtoscore(TARGETSCORE);
        if r3 >= POWERTARGET {
            self.elecdestroy(r5);
        }
        let (x, y) = self.backtranslate(r5);
        self.explogo(x, y + (8 << 8), 0, 0, 0, 0, None);
        self.makescoreobj(x, y, 0xf60 + ((TARGETSCORE / 100) << 8));
    }

    pub fn elecdestroy(&mut self, r5: BIdx) {
        let bw = self.boardwidth;
        self.eleccheck(r5 - bw);
        self.eleccheck(r5 - 1);
        self.eleccheck(r5 + 1);
        self.eleccheck(r5 + bw);
    }

    pub fn eleccheck(&mut self, r10: BIdx) {
        let r0 = self.cell(r10);
        if !(ELECLOWLIM..=ELECHIGHLIM).contains(&r0) {
            return;
        }
        let r4 = if r0 >= ELECVERTLOWLIM { self.boardwidth } else { 1 };
        self.elecdelete(r4, r10);
        self.elecdelete(-r4, r10);
    }

    pub fn elecdelete(&mut self, r4: i32, r10: BIdx) {
        let mut r10 = r10;
        let mut r2 = ELECLOWLIM;
        // Terminates: off-board reads return 0 (Board::get).
        while (ELECLOWLIM..=ELECHIGHLIM).contains(&r2) {
            self.set_cell(r10, 0);
            r10 = r10.wrapping_add(r4);
            r2 = self.cell(r10);
        }
    }

    pub fn deletetwin(&mut self, r5: BIdx) {
        if self.cell(r5) & 1 != 0 {
            self.set_cell(r5 - 1, 0);
        } else {
            self.set_cell(r5 + 1, 0);
        }
        self.set_cell(r5, 0);
    }

    /// After teleport/restart: wakes everything in a 40x32 box (original). In larger views the
    /// box must at least cover the visible game window plus the original margin.
    pub fn screenwakeup(&mut self, xpos: i32, ypos: i32) {
        self.screenwakeup_with(xpos, ypos, Sim::dowakeup);
    }

    fn screenwakeup_with(&mut self, xpos: i32, ypos: i32, wake: WakeFn) {
        self.woken_box = wake_box(&self.lay);
        let (sw, sh) = screen_wake_box(&self.lay);
        let bw = self.boardwidth;
        let mut r7 = self.translate(xpos, ypos).wrapping_sub(sw / 2).wrapping_sub(bw.wrapping_mul(sh / 2));
        // outer limits: start, end
        let r8: BIdx = 0;
        let r9: BIdx = self.bd().width.wrapping_mul(self.bd().height);
        for _ in 0..sh {
            // loopa4:
            self.linecheck(r7, r8, r9, sw, wake);
            r7 = r7.wrapping_add(bw);
        }
    }

    fn linecheck(&mut self, r7: BIdx, r8: BIdx, r9: BIdx, width: i32, wake: WakeFn) {
        let mut r7 = r7;
        for _ in 0..width {
            // loopa3:
            if r7 > r8 && r9 > r7 && self.cell(r7) > TRIGGERLIM {
                wake(self, r7);
            }
            r7 = r7.wrapping_add(1);
        }
    }

    /// A view that grew since the last screen wake-up (window resized, display mode or scale
    /// changed mid-game, a save loaded in a larger view): the perimeter scan would never reach
    /// the sleeping aliens now inside the larger box, so wake the whole screen box once.
    /// Never happens at 320x256, where the box cannot grow.
    pub fn wake_grown_view(&mut self) {
        let cur = wake_box(&self.lay);
        if cur.0 > self.woken_box.0 || cur.1 > self.woken_box.1 {
            self.screenwakeup(self.xpos, self.ypos);
        } else {
            self.woken_box = cur;
        }
    }

    /// Perimeter wake-up box (original 20x16 blocks; grown in larger views, see docs §4).
    pub fn wakeupal(&mut self, xpos: i32, ypos: i32) {
        self.wakeupal_with(xpos, ypos, Sim::dowakeup);
    }

    fn wakeupal_with(&mut self, xpos: i32, ypos: i32, wake: WakeFn) {
        let (bw, bh) = wake_box(&self.lay);
        let w = self.boardwidth;
        let mut r7 = self.translate(xpos, ypos).wrapping_sub(bw / 2).wrapping_sub(w.wrapping_mul(bh / 2));
        let r8: BIdx = 0;
        let r9: BIdx = self.bd().width.wrapping_mul(self.bd().height);
        self.boxcheck(bw, 1, &mut r7, r8, r9, wake);
        self.boxcheck(bh, w, &mut r7, r8, r9, wake);
        self.boxcheck(bw, -1, &mut r7, r8, r9, wake);
        self.boxcheck(bh, -w, &mut r7, r8, r9, wake);
    }

    fn boxcheck(&mut self, r4: i32, r5: i32, r7: &mut BIdx, r8: BIdx, r9: BIdx, wake: WakeFn) {
        for _ in 0..r4 {
            // loop80:
            if *r7 > r8 && r9 > *r7 && self.cell(*r7) > TRIGGERLIM {
                wake(self, *r7);
            }
            *r7 = r7.wrapping_add(r5);
        }
    }

    /// Finds the start marker (255), clears it; returns (initplx, initply).
    pub fn findplayer(&mut self) -> (i32, i32) {
        let n = self.bd().width.wrapping_mul(self.bd().height);
        let mut r5: Option<BIdx> = None;
        let mut r3 = n;
        while r3 > 0 {
            // loop60:
            if self.cell(r3 - 1) == MARKERNO {
                self.set_cell(r3 - 1, 0);
                r5 = Some(r3 - 1);
            }
            r3 -= 1;
        }
        let (x, y) = match r5 {
            Some(p) => self.backtranslate(p),
            None => (64 << 8, 64 << 8),
        };
        (x, y - (1 << 8))
    }

    pub fn normtelep(&self, start: BIdx, dir: i32, find: u8) -> Option<BIdx> {
        let r2: BIdx = 0;
        let r3: BIdx = self.bd().width.wrapping_mul(self.bd().height);
        let mut r1 = start;
        loop {
            // loop62:
            if r1 <= r2 {
                return None; // telepoffleft
            }
            if r1 >= r3 {
                return None; // telepoffright
            }
            if self.cell(r1) == find {
                return Some(r1);
            }
            if dir == 0 {
                return None; // C would loop forever; telep() always passes ±1
            }
            r1 = r1.wrapping_add(dir);
        }
    }

    pub fn bonusfind(&self) -> Option<BIdx> {
        let n = self.bd().width.wrapping_mul(self.bd().height);
        (0..n).find(|&p| self.cell(p) == STARSNO)
    }

    /// `boardreg()`: boardwidth, xposmax, yposmax from the current board.
    /// (`boardlowlim/boardhighlim` are unused by the C code and not kept.)
    pub fn boardreg(&mut self) {
        let (w, h) = (self.bd().width, self.bd().height);
        self.boardwidth = w;
        self.xposmax = w << 12;
        self.yposmax = h << 12;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::assets::Board;
    use crate::layout::Layout;
    use crate::sim::testutil::sim;

    /// Replaces the brain board with an empty `w`x`h` board filled with `fill`.
    pub(crate) fn small_board(s: &mut Sim, w: i32, h: i32, fill: u8) {
        s.brain = Board { width: w, height: h, header: [0; 32], cells: vec![fill; (w * h) as usize] };
        s.in_neuron = false;
        s.boardreg();
    }

    #[test]
    fn translate_backtranslate_round_trip() {
        let mut s = sim();
        small_board(&mut s, 37, 23, 0);
        assert_eq!((s.boardwidth, s.xposmax, s.yposmax), (37, 37 << 12, 23 << 12));
        for cy in 0..23 {
            for cx in 0..37 {
                let p = s.translate((cx << 12) + 0x0fff, (cy << 12) + 0x0123);
                assert_eq!(p, cx + cy * 37);
                assert_eq!(s.backtranslate(p), (cx << 12, cy << 12));
            }
        }
        // fntranslate: +8 px in x, -8 px in y
        assert_eq!(s.fntranslate((3 << 12) + (8 << 8), (5 << 12) + (8 << 8)), 4 + 5 * 37);
        assert_eq!(s.fntranslate(3 << 12, 5 << 12), 3 + 4 * 37);
        // negative offsets use C truncation (r0 % w keeps the sign of r0)
        assert_eq!(s.backtranslate(-1), (-1 << 12, 0));
        assert_eq!(s.backtranslate(-38), (-1 << 12, -1 << 12));
    }

    #[test]
    fn wake_box_geometry_original_and_larger_views() {
        let orig = Layout::original();
        assert_eq!(wake_box(&orig), (BOXWIDTH, BOXHEIGHT));
        assert_eq!(screen_wake_box(&orig), (2 * BOXWIDTH, 2 * BOXHEIGHT));
        // 16:9 at 1.0x: cw = 455 → gamew = 423 → 14 blocks half-window → box 30 wide
        let wide = Layout::new(455.0, 256.0);
        assert_eq!(wake_box(&wide), (30, 16));
        assert_eq!(screen_wake_box(&wide), (60, 32));
        // 21:9 at 3.0x: 1788x704 window → 56 / 22 blocks half-window
        let big = Layout::new(1820.0, 768.0);
        assert_eq!((big.gamew, big.gameh), (1788, 704));
        assert_eq!(wake_box(&big), (114, 48));
        // the screen box stops at the sleep range (512 + 750 / 512 + 256 px) and still covers
        // more than the visible window
        assert_eq!(screen_wake_box(&big), (156, 94));
        for lay in [Layout::original(), wide, big, Layout::new(910.0, 512.0), Layout::new(2731.0, 768.0), Layout::new(512.0, 1700.0)] {
            let (sw, sh) = screen_wake_box(&lay);
            assert!(sw / 2 * 16 >= lay.gamew / 2 && sh / 2 * 16 >= lay.gameh / 2, "{lay:?}");
            // every woken alien centre stays inside the sleep range
            let (ex, ey) = sleep_extra(&lay);
            if sw > BOXWIDTH * 2 {
                assert!(sw / 2 * 16 + 8 <= (SLEEPRANGE >> 8) + ex, "{lay:?}");
            }
            if sh > BOXHEIGHT * 2 {
                assert!(sh / 2 * 16 + 9 <= (SLEEPRANGE >> 8) + ey, "{lay:?}");
            }
        }
    }

    #[test]
    fn a_grown_view_wakes_the_sleepers_now_on_screen() {
        let mut s = sim();
        s.lay = Layout::original();
        s.xpos = 2000 << 8;
        s.ypos = 2000 << 8;
        s.screenwakeup(s.xpos, s.ypos);
        assert_eq!(s.woken_box, (BOXWIDTH, BOXHEIGHT));
        // a sleeping alien 60 blocks to the right: outside the original boxes, on screen at 3.0x
        let c = s.translate((2000 + 60 * 16) << 8, 2000 << 8);
        s.set_cell(c, 241);
        s.wake_grown_view();
        assert_eq!(s.cell(c), 241, "nothing changes at the original size");
        s.lay = Layout::new(1820.0, 768.0);
        s.wake_grown_view();
        assert_eq!(s.cell(c), 0, "woken when the view grew");
        assert_eq!(s.woken_box, wake_box(&s.lay));
        // shrinking only lowers the mark, so growing again wakes again
        s.lay = Layout::original();
        s.wake_grown_view();
        assert_eq!(s.woken_box, (BOXWIDTH, BOXHEIGHT));
    }

    fn clear_cell(s: &mut Sim, p: BIdx) {
        s.set_cell(p, 0);
    }

    /// Cells cleared by a wake scan on a board full of trigger values.
    fn woken(s: &Sim) -> Vec<(i32, i32)> {
        let w = s.boardwidth;
        let mut v: Vec<(i32, i32)> =
            (0..s.bd().len()).filter(|&p| s.cell(p) == 0).map(|p| (p % w, p / w)).collect();
        v.sort();
        v
    }

    #[test]
    fn wakeupal_perimeter_matches_c_box() {
        let mut s = sim();
        small_board(&mut s, 64, 64, 240);
        let (cx, cy) = (30, 30);
        s.wakeupal_with(cx << 12, cy << 12, clear_cell);
        // C: start = centre - 10 - 8*width, perimeter 20 x 16
        let mut expect = Vec::new();
        let (x0, y0) = (cx - 10, cy - 8);
        for i in 0..20 {
            expect.push((x0 + i, y0)); // right
            expect.push((x0 + 20 - i, y0 + 16)); // left (bottom edge)
        }
        for j in 0..16 {
            expect.push((x0 + 20, y0 + j)); // down
            expect.push((x0, y0 + 16 - j)); // up
        }
        expect.sort();
        expect.dedup();
        assert_eq!(woken(&s), expect);
        assert_eq!(expect.len(), 2 * (20 + 16));
    }

    #[test]
    fn wakeupal_wide_grows_and_skips_off_board() {
        let mut s = sim();
        s.lay = Layout::new(455.1, 256.0);
        small_board(&mut s, 64, 64, 240);
        s.wakeupal_with(32 << 12, 32 << 12, clear_cell);
        let v = woken(&s);
        assert_eq!(v.len(), 2 * (30 + 16));
        assert_eq!(v.iter().map(|c| c.0).min(), Some(32 - 15));
        assert_eq!(v.iter().map(|c| c.0).max(), Some(32 + 15));
    }

    #[test]
    fn wakeupal_keeps_c_bounds_checks() {
        // perimeter starting exactly at offset 0: the C `r7 > r8` check skips it
        let mut s = sim();
        small_board(&mut s, 64, 64, 240);
        s.wakeupal_with(10 << 12, 8 << 12, clear_cell);
        assert_eq!(s.cell(0), 240);
        assert_eq!(woken(&s).len(), 2 * (20 + 16) - 1);
        // top edge on row -1 (board header bytes): skipped, header untouched
        let mut s = sim();
        small_board(&mut s, 64, 64, 240);
        s.brain.header = [240; 32];
        s.wakeupal_with(45 << 12, 7 << 12, clear_cell);
        assert_eq!(s.brain.header, [240; 32]);
        // far outside the board: nothing happens, no panic
        let mut s = sim();
        small_board(&mut s, 64, 64, 240);
        s.wakeupal_with(-(500 << 12), 900 << 12, clear_cell);
        s.screenwakeup_with(i32::MAX, i32::MIN, clear_cell);
        assert!(woken(&s).is_empty());
    }

    #[test]
    fn screenwakeup_box() {
        let mut s = sim();
        small_board(&mut s, 100, 80, 200);
        s.screenwakeup_with(50 << 12, 40 << 12, clear_cell);
        let v = woken(&s);
        assert_eq!(v.len(), 40 * 32);
        assert_eq!(v.first(), Some(&(30, 24))); // translate - 20 - 16*width
        assert_eq!(v.last(), Some(&(69, 55)));
        // wide: 60 x 32
        let mut s = sim();
        s.lay = Layout::new(455.1, 256.0);
        small_board(&mut s, 100, 80, 200);
        s.screenwakeup_with(50 << 12, 40 << 12, clear_cell);
        assert_eq!(woken(&s).len(), 60 * 32);
        // values <= _triggerlim are ignored
        let mut s = sim();
        small_board(&mut s, 100, 80, TRIGGERLIM);
        s.screenwakeup_with(50 << 12, 40 << 12, clear_cell);
        assert!(woken(&s).is_empty());
    }

    #[test]
    fn fuelair_spreads_on_empty_board() {
        let mut s = sim();
        small_board(&mut s, 16, 16, 0);
        s.prepfueltab();
        s.framectr = 0;
        let bomb = 8 + 8 * 16;
        s.set_cell(bomb, FUELAIRNO);
        s.set_cell(bomb + 1, FUELAIRNO + 1);
        assert_eq!(s.bombcheck(bomb), 0);
        assert_eq!(s.cell(bomb), 0);
        assert_eq!(s.cell(bomb + 1), 0);
        let gas0 = bomb - 16;
        assert_eq!((s.cell(gas0), s.cell(gas0 + 1)), (GASLOWLIM, GASLOWLIM));
        assert_eq!(s.fueltabctr, FUELAIRDURATION);
        assert_eq!(s.sfx_events.len(), 1);
        assert_eq!(s.sfx_events[0].id, SfxId::FUELAIR);

        s.fuelairproc();
        assert_eq!(s.fueltabctr, FUELAIRDURATION - 1);
        // the read table is now the one just written: 6 new cells + NULL
        let t = s.fuel_read;
        let new: Vec<BIdx> = s.fueltab[t].iter().copied().take_while(|&p| p != FUEL_NULL).collect();
        assert_eq!(new, vec![gas0 - 1, gas0 + 16, gas0 - 16, gas0 + 1 + 16, gas0 + 2, gas0 + 1 - 16]);
        let gas: usize = s.brain.cells.iter().filter(|&&c| c == GASLOWLIM).count();
        assert_eq!(gas, 8);

        // keeps spreading until the duration runs out, then the table is terminated
        for f in 1..200 {
            s.framectr = f;
            s.fuelairproc();
        }
        assert!(s.fueltabctr < 0);
        assert_eq!(s.fueltab[s.fuel_write][0], FUEL_NULL);
        let gas: usize = s.brain.cells.iter().filter(|&&c| c == GASLOWLIM).count();
        assert_eq!(gas, 256, "an empty 16x16 board fills completely");
    }

    #[test]
    fn fuelair_duration_throttles() {
        let mut s = sim();
        small_board(&mut s, 16, 16, 0);
        s.prepfueltab();
        s.fueltabctr = 0x10;
        s.fuelairlastframe = 100;
        s.framectr = 103; // needs 100 + (0x18-0x10)/2 = 104
        s.fuelairproc();
        assert_eq!(s.fueltabctr, 0x10);
        s.framectr = 104;
        s.fuelairproc();
        assert_eq!(s.fueltabctr, 0x0f);
        assert_eq!(s.fuelairlastframe, 104);
    }

    /// PORTING_NOTES #35: a pass that stops on the `_fueltablim` bound has written 3 entries
    /// plus the closing NULL past index 0x3c0. They stay in the written table (4 guard
    /// entries), never touch the other table (C: `fueltabofs[1][0..3]`, or past the array),
    /// and are spread exactly one generation on the next pass.
    #[test]
    fn fuelair_table_overflow_stays_in_own_table() {
        const W: i32 = 52;
        let nb = |p: BIdx| [p - 1, p + W, p + 1, p - W];
        let on_board = |p: BIdx| (0..W * W).contains(&p);
        for wt in [0usize, 1] {
            let rt = 1 - wt;
            let mut s = sim();
            small_board(&mut s, W, W, 0);
            s.prepfueltab();
            assert_eq!(s.fueltab[wt].len(), FUELTABLIM + 4);
            // 241 gas cells 3 apart (no shared neighbours, 2 cells from every edge); the first
            // has one neighbour blocked, so the pass adds 3 + 240 * 4 = 963 cells: the last
            // entry read takes `w` from 959 to 963.
            let front: Vec<BIdx> = (0..241).map(|k| (2 + 3 * (k % 16)) + (2 + 3 * (k / 16)) * W).collect();
            for &p in &front {
                s.set_cell(p, GASLOWLIM);
            }
            s.set_cell(front[0] - W, 1);
            s.fuel_read = rt;
            s.fuel_write = wt;
            s.fueltab[rt][..front.len()].copy_from_slice(&front);
            s.fueltab[rt][front.len()] = FUEL_NULL;
            let read_before = s.fueltab[rt];
            s.fueltabctr = FUELAIRDURATION;
            s.fuelairlastframe = 0;
            s.framectr = 0;

            s.fuelairproc();
            assert_eq!((s.fuel_read, s.fuel_write), (wt, rt));
            let t = s.fueltab[wt];
            assert!(t[..FUELTABLIM + 3].iter().all(|&p| p != FUEL_NULL));
            assert_eq!(t[FUELTABLIM + 3], FUEL_NULL, "closing NULL in the last guard entry");
            assert_eq!(t[FUELTABLIM - 1..FUELTABLIM + 3], nb(front[240]));
            assert_eq!(s.fueltab[rt], read_before, "the other table is never written");

            // Next pass: leave free only the cells that just the overflow entries can reach
            // (`e1`) and their outer neighbours (`e2`); every other free cell is blocked.
            let entries = &t[..FUELTABLIM + 3];
            assert!(entries.iter().all(|&q| nb(q).into_iter().all(on_board)));
            let (inner, over) = entries.split_at(FUELTABLIM);
            let near = |set: &[BIdx], n: BIdx| set.iter().any(|&q| nb(q).contains(&n));
            let mut e1: Vec<BIdx> = Vec::new();
            for &e in over {
                for n in nb(e) {
                    if on_board(n) && s.cell(n) == 0 && !near(inner, n) && !e1.contains(&n) {
                        e1.push(n);
                    }
                }
            }
            let mut e2: Vec<BIdx> = Vec::new();
            for &n in &e1 {
                for m in nb(n) {
                    if on_board(m) && s.cell(m) == 0 && !e1.contains(&m) && !e2.contains(&m) && !near(entries, m)
                    {
                        e2.push(m);
                    }
                }
            }
            assert!(!e1.is_empty() && !e2.is_empty());
            for p in 0..W * W {
                if s.cell(p) == 0 && !e1.contains(&p) && !e2.contains(&p) {
                    s.set_cell(p, 1);
                }
            }
            let gas = |s: &Sim| s.brain.cells.iter().filter(|&&c| c == GASLOWLIM).count();
            let gas_before = gas(&s);

            s.fuelairproc();
            assert_eq!(s.fuel_read, rt);
            let new: Vec<BIdx> = s.fueltab[rt].iter().copied().take_while(|&p| p != FUEL_NULL).collect();
            assert_eq!(new, e1, "the overflow entries are read as normal front cells");
            assert_eq!(gas(&s), gas_before + e1.len());
            assert!(e2.iter().all(|&p| s.cell(p) == 0), "gas spreads one generation per pass");
        }
    }

    #[test]
    fn board_search_helpers() {
        let mut s = sim();
        small_board(&mut s, 10, 10, 0);
        s.set_cell(57, MARKERNO);
        s.set_cell(23, MARKERNO);
        let (x, y) = s.findplayer();
        assert_eq!((x, y), (3 << 12, (2 << 12) - (1 << 8)), "lowest marker wins");
        assert_eq!((s.cell(23), s.cell(57)), (0, 0));
        assert_eq!(s.findplayer(), (64 << 8, (64 << 8) - (1 << 8)));

        s.set_cell(40, 194);
        assert_eq!(s.normtelep(45, -1, 194), Some(40));
        assert_eq!(s.normtelep(45, 1, 194), None);
        assert_eq!(s.normtelep(0, 1, 0), None, "start <= contents is off-left");

        assert_eq!(s.bonusfind(), None);
        s.set_cell(77, STARSNO);
        s.set_cell(99, STARSNO);
        assert_eq!(s.bonusfind(), Some(77));
    }

    #[test]
    fn small_block_rules() {
        let mut s = sim();
        small_board(&mut s, 10, 10, 0);
        // crumble-stand advances and disappears every 4th step
        s.set_cell(5, CRUMBLESTANDLOWLIM);
        s.crumblecheck(5);
        assert_eq!(s.cell(5), CRUMBLESTANDLOWLIM + 1);
        s.set_cell(5, CRUMBLESTANDLOWLIM + 3);
        s.crumblecheck(5);
        assert_eq!(s.cell(5), 0);
        // electricity strips are deleted along their direction
        for p in 31..36 {
            s.set_cell(p, ELECLOWLIM);
        }
        s.set_cell(36, 1);
        s.eleccheck(33);
        assert!((31..36).all(|p| s.cell(p) == 0));
        assert_eq!(s.cell(36), 1);
        // twins
        s.set_cell(50, 69);
        s.set_cell(49, 68);
        s.deletetwin(50);
        assert_eq!((s.cell(49), s.cell(50)), (0, 0));
        // wind
        let (mut dx, mut dy) = (0, 0);
        s.set_cell(60, 3);
        assert!(s.seeifwind(60, &mut dx, &mut dy, false));
        assert_eq!((dx, dy), (WINDSPEED * 4, 0));
        assert!(!s.seeifwind(61, &mut dx, &mut dy, false));
    }
}
