//! player.c (+ keyboard.c `keyread`, `cheatread`; menus.c `completedzone`).
//!
//! Plotting: `blokeplot(sprites, n, x, y)` = `self.blokeplot_cen(bank, n, x, y + plotterofs,
//! Clip::PlayerFloor)` (playerplot sets the player clip; `seestars` plots in the same clip).
//! The HUD (`plotbonus`, `showstrength`, `showscore`, `showlives`) is drawn by the renderer
//! from state (`bonusctr`, `bonusreplot`, `plstrength`, `framectr`, `plscore`, `lives`);
//! `bonusplot`/`update_show_strength` here only run their state logic.
//! `keyread(&mut ks)` builds the press counters from `TickInput::held` (no joystick path).
//!
//! Where several `random()` calls appear in one C expression or argument list (whose
//! evaluation order C leaves unspecified) they are evaluated left to right.

use crate::assets::BIdx;
use crate::atlas::Bank;
use crate::audio::sfx::SfxId;
use crate::i18n::keys;
use crate::input::{HELD_DOWN, HELD_FIRE, HELD_LEFT, HELD_RIGHT, HELD_UP};
use crate::sim::maze::block_weapon;
use crate::sim::*;

/// C `pladr1 = 0` (NULL). Reads as 0 through `Board::get` (the C port would dereference
/// NULL in `windcheck` if wind were triggered before the next `plmove` recomputes it).
const PLADR_NULL: BIdx = i32::MIN;

/// player.c `init_rockettab` (index 192 is the guard entry = 0).
pub fn init_rockettab(t: &mut [i32; 193]) {
    for i in 0..184 {
        t[4 + i] = (256.0 * (i as f64 * 2.0 * std::f64::consts::PI / (192.0 - 8.0)).sin()) as i32;
    }
    for i in 0..4 {
        t[i] = 0;
        t[188 + i] = 0;
    }
    t[192] = 0;
}

/// file.c `read_littleendian` (missing bytes read as 0).
fn read_le(store: &[u8], off: usize) -> i32 {
    let b = |i: usize| store.get(off + i).copied().unwrap_or(0);
    i32::from_le_bytes([b(0), b(1), b(2), b(3)])
}

/// file.c `write_littleendian` (bytes beyond the store are dropped).
fn write_le(store: &mut [u8], off: usize, v: i32) {
    for (i, byte) in v.to_le_bytes().into_iter().enumerate() {
        if let Some(d) = store.get_mut(off + i) {
            *d = byte;
        }
    }
}

/// keyboard.c per-key counter update.
#[inline]
fn press(c: &mut u8, down: bool) {
    if down {
        *c = c.wrapping_add(1);
        if *c == 0 {
            *c = 0xff;
        }
    } else {
        *c = 0;
    }
}

impl Sim {
    /// player.c `blokeplot` inside the player clip (set_player_clip … writeclip).
    fn blokeplot(&mut self, bank: Bank, n: u8, x: i32, y: i32) {
        self.plot_key = KEY_NONE;
        let y = y + self.plotterofs as i32;
        self.blokeplot_cen(bank, n, x, y, Clip::PlayerFloor);
    }

    /// `rockettabadr[i]` (out-of-range reads 0; rocketctr always stays in 0..192).
    #[inline]
    fn rocktab(&self, i: i32) -> i32 {
        usize::try_from(i).ok().and_then(|i| self.rockettab.get(i)).copied().unwrap_or(0)
    }

    pub fn save_player_state(&self, store: &mut [u8]) {
        write_le(store, 0, self.lives);
        write_le(store, 4, self.plstrength);
        write_le(store, 8, self.neuronctr);
        write_le(store, 12, self.shutdownctr);
        for (i, &d) in self.plscore.iter().enumerate() {
            if let Some(s) = store.get_mut(16 + i) {
                *s = d;
            }
        }
    }

    pub fn restore_player_state(&mut self, store: &[u8]) {
        self.lives = read_le(store, 0);
        self.plstrength = read_le(store, 4);
        self.neuronctr = read_le(store, 8);
        self.shutdownctr = read_le(store, 12);
        for (i, d) in self.plscore.iter_mut().enumerate() {
            *d = store.get(16 + i).copied().unwrap_or(0);
        }
    }

    pub fn save_player(&self, store: &mut [u8]) {
        write_le(store, 0, SAVEVALID);
        write_le(store, 4, self.xpos);
        write_le(store, 8, self.ypos);
        write_le(store, 12, self.initplx);
        write_le(store, 16, self.initply);
        write_le(store, 20, self.hvec);
        write_le(store, 24, self.vvec);
        if store.len() >= 30 {
            store[28] = self.bonusctr;
            store[29] = self.plweapontype;
        }
    }

    /// Returns true if the store was not valid (C returns 1).
    pub fn restore_player(&mut self, store: &mut [u8]) -> bool {
        if store.len() < 30 || SAVEVALID != read_le(store, 0) {
            return true;
        }
        write_le(store, 0, 0);
        self.xpos = read_le(store, 4);
        self.ypos = read_le(store, 8);
        self.initplx = read_le(store, 12);
        self.initply = read_le(store, 16);
        self.hvec = read_le(store, 20);
        self.vvec = read_le(store, 24);
        self.bonusctr = store[28];
        self.plweapontype = store[29];
        false
    }

    pub fn electrocute(&mut self, r5: BIdx) {
        self.electrocuting = 1;
        let (x, y) = self.backtranslate(r5);
        let dx = (self.random() & (0xfe << 2)) - (0xfe << 1);
        let dy = (self.random() & (0xfe << 2)) - (0xfe << 1);
        self.explogonopyro(x, y + (16 << 8), dx, dy, 0, 0, None);
    }

    pub fn playerplot(&mut self, whendead: bool) {
        if (!whendead) ^ (self.snuffctr == 0) {
            return;
        }
        // set_player_clip(): every blokeplot below uses Clip::PlayerFloor.
        if self.snuffctr != 0 {
            //snuffhandler:
            self.snuffctr += self.frameinc;
            self.plotterofs = ((self.snuffctr - 96).max(0) >> 1) as u8;
            self.blokeplot(Bank::Bloke, 70, 0, 8);
            self.seestars();
            return; // writeclip()
        }

        if self.telepctr != 0 {
            let mut telepinc = self.frameinc;
            if telepinc > 2 {
                telepinc = 2;
            }
            if self.telepctr == 31 {
                telepinc = 1;
            }
            self.telepctr += telepinc;
            if self.telepctr > 64 {
                self.telepctr = 0;
            }
            if self.telepctr == 32 {
                //dotelep:
                self.xpos = self.telepxpos;
                self.ypos = self.telepypos;
                self.screenwakeup(self.xpos, self.ypos);
            }
        }

        //telepskip:
        self.plotterofs = (if self.telepctr > 32 { 64 - self.telepctr } else { self.telepctr }) as u8;
        self.plframe = ((self.xpos >> 10) & 3) as u8;
        if self.plframe == 2 {
            self.plframe = 0;
        } else if self.plframe == 3 {
            self.plframe = 2;
        }
        if self.plface == 1 {
            self.plframe += 3;
        }

        if self.electrocuting != 0 {
            //plotskel:
            self.blokeplot(Bank::Bloke, SKELSPRITENO + (self.plface == 1) as u8, 0, 8);
            self.seestars();
            return;
        }

        self.blokeplot(Bank::Bloke, if self.plface == 1 { 1 } else { 0 }, 0, 8);
        self.blokeplot(Bank::Bloke, self.plframe.wrapping_add(6), 0, 19);

        let r0 = if self.plstrength < self.laststrength {
            FUNNYFACESPRBASE + ((self.framectr >> 2) & 3) as u8
        } else if self.plface == 1 {
            //nofunnyface:
            15
        } else {
            12
        };
        //plotface:
        self.blokeplot(Bank::Bloke, r0, 0, -2); // head

        if self.plweapontype == 0 {
            self.plotarms();
        } else if self.plweapontype == MPMGBLAMNO {
            self.plotmpmgblam();
        } else if self.plweapontype == ROCKETBLAMNO {
            self.plotrocketblam();
        } else if self.plweapontype < ROCKBASE {
            self.plotmpmg();
        } else {
            self.plotrocket();
        }

        //playerplotins:
        self.seestars();
    }

    pub fn plotarms(&mut self) {
        let n = (self.plframe % 3) + 2;
        let x = (self.plframe > 2) as i32;
        self.blokeplot(Bank::Bloke, n, x, 9);
    }

    pub fn plotmpmg(&mut self) {
        let face = self.plface as i32;
        let x = -6 + (self.plface != 1) as i32 * 12 + if self.plfired != 0 { 0 } else { 2 * face - 1 };
        self.blokeplot(Bank::Bloke, 42 + (self.plface == 1) as u8, x, 10);
        self.blokeplot(Bank::Bloke, 44 + (self.plface == 1) as u8, 0, 8);
        self.plfired = 0;
    }

    /// Blam wobble: `r2` of plotmpmgblam/plotrocketblam.
    fn blamwobble(&self) -> i32 {
        let b = self.blamctr as i32;
        if b & 8 != 0 {
            (b & 15) - 15
        } else {
            b & 15
        }
    }

    pub fn plotmpmgblam(&mut self) {
        let r2 = self.blamwobble();
        let face = self.plface as i32;
        let x = -6 + (self.plface != 1) as i32 * 12 + if self.plfired != 0 { face * 2 - 1 } else { 0 };
        let blocks = self.blocks_bank();
        self.blokeplot(blocks, WEAPMPMGBLAM + (self.plface == 1) as u8, x, 11 + (r2 >> 1));
        self.blokeplot(Bank::Bloke, 44 + (self.plface == 1) as u8, 0, 8 + (r2 >> 1));
        self.plfired = 0;
    }

    pub fn plotrocket(&mut self) {
        let r0 = self.rocktab(self.rocketctr);
        let n = 50 + (r0 > 170) as i32 + (r0 > 60) as i32 - (r0 < -60) as i32 - (r0 < -170) as i32;
        let face = self.plface as i32;
        let x = if self.plfired != 0 { face * 2 - 1 } else { 0 };
        self.blokeplot(Bank::Bloke, n as u8, x, 1);
        self.blokeplot(Bank::Bloke, 53, face, 6);
        self.plfired = 0;
        if self.rocketflag == 0 {
            self.rocketctr = 0;
        }
    }

    pub fn plotrocketblam(&mut self) {
        let r2 = self.blamwobble();
        let face = self.plface as i32;
        let x = if self.plfired != 0 { 0 } else { 2 * face - 1 };
        let blocks = self.blocks_bank();
        self.blokeplot(blocks, WEAPROCKETBLAM, x, r2 >> 1);
        self.blokeplot(Bank::Bloke, 53, 0, (r2 >> 2) + 6);
        self.plfired = 0;
    }

    pub fn initrockettab(&mut self) {
        // rockettabadr = rockettab: the table is used directly.
        self.rocketctr = 0;
        self.rocketflag = 0;
    }

    pub fn seestars(&mut self) {
        if self.plstrength >= self.laststrength {
            self.laststrength = self.plstrength;
            return;
        }
        self.laststrength -= 1 << 8;
        let r1 = (self.framectr >> 1) & 7;
        let r0 = STARSSPRBASE + r1 as u8;
        let r2 = if (r1 & 3) == 0 {
            -6
        } else if r1 > 4 {
            -7
        } else {
            -5
        };
        self.blokeplot(Bank::Bloke, r0, 0, r2);

        let r4 = self.laststrength - self.plstrength;
        if r4 == 0 {
            return;
        }
        if self.electrocuting != 0 {
            //elecsound:
            self.bidforsound(PLAYERCHANNEL, 0x7f, 0, SfxId::ELEC_1);
            self.bidforsound(FIRECHANNEL, 0x7f, 127, SfxId::ELEC_2);
            self.bidforsound(SPARECHANNEL, 0x7f, -127, SfxId::ELEC_3);
        } else {
            let i = if r4 >= 0x1000 {
                16
            } else if r4 < 0 {
                0
            } else {
                r4 >> 8
            };
            self.bidforsound(PLAYERCHANNEL, 0x70, 0, SfxId::stunned(i as u8));
        }
    }

    pub fn seeifdead(&mut self) {
        if self.plstrength > 0 || self.snuffctr > 0 {
            return;
        }
        self.snuffctr = 1;

        self.makeobj(DECORATION, 1 << 8, (2 << 8) - (12 << 8), 0, -(2 << 8), DEADSPRBASE - 3 + (250 << 16), 24);
        self.makeobj(DECORATION, (1 << 8) - (5 << 8), 2 << 8, -(1 << 8), -(3 << 8), DEADSPRBASE + 1 + (250 << 16), 24);
        self.makeobj(DECORATION, (1 << 8) + (6 << 8), 2 << 8, 1 << 8, -(3 << 8), DEADSPRBASE + 2 + (250 << 16), 24);

        self.deathmessage();
        if self.lives == 0 {
            self.endgamemessage();
        }
    }

    pub fn bcheck(&mut self) {
        let mut fa = self.fraudalent;
        fa.uphit = 0;
        fa.downhit = 0;
        fa.x = self.xpos;
        let ytemp = self.ypos;
        fa.y = self.ypos;
        fa.dx = self.hvec;
        fa.dy = self.vvec;
        fa.falling = self.falling;
        let bw = self.boardwidth;
        let blocked = |s: &Sim, p: BIdx| s.cell(p) >= BLIM;

        let mut r11 = self.translate(fa.x, fa.y + fa.dy);
        // up
        if blocked(self, r11 - bw) || blocked(self, r11 - bw + 1) {
            self.noup(&mut fa);
        }
        // down
        if blocked(self, r11 + bw) || blocked(self, r11 + bw + 1) {
            self.nodown(&mut fa);
        }
        if blocked(self, r11 + bw * 2) || blocked(self, r11 + bw * 2 + 1) {
            self.nodownifplat(&mut fa);
        }
        let fg = self.fallinggap(&mut fa);
        if fg == 0 && (ytemp & (15 << 8)) != (15 << 8) {
            r11 = self.translate(fa.x + fa.dx, fa.y);
            let r10 = self.translate(fa.x + fa.dx, fa.y); // "+(4<<8)" was commented out
            // left
            if blocked(self, r10 - bw) || blocked(self, r11) || blocked(self, r11 + bw) {
                self.noleft(&mut fa);
            }
            // right
            if blocked(self, r10 - bw + 1) || blocked(self, r11 + 1) || blocked(self, r11 + bw + 1) {
                self.noright(&mut fa);
            }
        } else if fg == -1 {
            r11 = self.translate(fa.x + fa.dx, fa.y);
            if blocked(self, r11 - bw) || blocked(self, r11) {
                self.noleft(&mut fa);
            }
            if blocked(self, r11 - bw + 1) || blocked(self, r11 + 1) {
                self.noright(&mut fa);
            }
        } else {
            //shorT:
            r11 = self.translate(fa.x + fa.dx, fa.y);
            if blocked(self, r11) || blocked(self, r11 + bw) {
                self.noleft(&mut fa);
            }
            if blocked(self, r11 + 1) || blocked(self, r11 + 1 + bw) {
                self.noright(&mut fa);
            }
        }

        //bcheckins:
        r11 = self.translate(fa.x + fa.dx, fa.y + fa.dy);
        if blocked(self, r11 - bw) || blocked(self, r11 - bw + 1) {
            self.noup(&mut fa);
        }
        if blocked(self, r11 + bw) || blocked(self, r11 + bw + 1) {
            self.nodown(&mut fa);
        }
        self.falling = fa.falling;
        self.xpos = fa.x;
        self.ypos = fa.y;
        self.hvec = fa.dx;
        self.vvec = fa.dy;
        self.fraudalent = fa;
    }

    pub fn plmove(&mut self, input: TickInput) {
        if self.extending != 0 {
            self.extending -= 1;
        }
        if self.shutdownctr != 0 {
            self.shutdownctr += 1;
        }
        if self.shutdownctr == 150 {
            self.change_zone(0);
        } else if self.shutdownctr != 0 {
            self.deletepoint();
        }
        self.keyread(input.held);

        if self.snuffctr != 0 {
            self.ks.uppress = 0;
            self.ks.downpress = 0;
            self.ks.leftpress = 0;
            self.ks.rightpress = 0;
            self.ks.fire = 0;
        }
        self.vvec -= self.vvec >> 5;
        self.hvec -= self.hvec >> 5;
        let ks = self.ks;
        if ks.leftpress != ks.rightpress {
            self.plface = if ks.leftpress > ks.rightpress { 1 } else { 0 };
        }

        let mut r2 = if ks.leftpress < 4 && ks.rightpress < 4 { 1 << 8 } else { 2 << 8 };
        if ks.leftpress > ks.rightpress {
            r2 = -r2;
        }
        if ks.leftpress == ks.rightpress {
            r2 = 0;
        }

        if self.falling <= 4 {
            self.hvec = r2;
        } else {
            self.hvec += r2 >> 5;
        }

        //cheatins:
        if ks.downpress >= 32 {
            self.windctr = 1 << 10;
        }
        if self.windctr != 0 {
            self.windcheck();
        }
        self.hvec = self.hvec.clamp(-SPEEDLIM, SPEEDLIM);
        self.vvec = self.vvec.clamp(-SPEEDLIM, SPEEDLIM);
        let mut fa = self.fraudalent;
        fa.x = self.xpos;
        fa.y = self.ypos;
        fa.dx = self.hvec;
        fa.dy = self.vvec;
        fa.falling = self.falling;
        fa.pluphit = self.pluphit;
        fa.downpress = self.ks.downpress;
        let platypos = self.ypos;
        self.colcheck(&mut fa, 0, platypos);
        self.fraudalent = fa;
        self.falling = fa.falling;
        self.xpos = fa.x;
        self.ypos = fa.y;
        self.hvec = fa.dx;
        self.vvec = fa.dy;

        self.bcheck();
        self.pluphit = self.fraudalent.uphit;
        let dy = self.vvec.clamp(-(1 << 12), 1 << 12);
        self.xpos += self.hvec;
        self.ypos += dy;
        if self.xpos <= XLOWLIM || self.xposmax <= self.xpos || self.ypos <= YLOWLIM || self.yposmax <= self.ypos {
            self.restartplayer();
        }
        let x = self.xpos;
        let mut y = self.ypos;

        if self.snuffctr == 0 {
            y += 6 << 8;
            self.vvec = dy;
            self.pllx = x - (PLWIDTH / 2);
            self.plhx = x + (PLWIDTH / 2);
            self.plly = y - (PLHEIGHT / 2) + ((self.plotterofs as i32) << 8);
            self.plhy = y + (PLHEIGHT / 2);
            y = self.ypos - (6 << 8); // - adjustment
        } else {
            //fakecolch:
            self.plhx = 0;
            self.plhy = 0;
            self.pllx = 1;
            self.plly = 1;
        }
        //fakeins:
        let bw = self.boardwidth;
        self.pladr[1] = self.translate(x, y);
        self.pladr[2] = self.pladr[1] + 1;
        self.pladr[3] = self.pladr[1] + bw;
        self.pladr[4] = self.pladr[3] + 1;
        self.pladr[5] = self.pladr[1] + 2 * bw;
        self.pladr[6] = self.pladr[5] + 1;

        if (self.falling == 0 || self.falling == 0xff) && self.ks.uppress != 0 {
            if self.ks.downpress > self.ks.uppress {
                self.vvec = -((0x100 - (self.ks.downpress as i32 - self.ks.uppress as i32)) << 3);
            } else {
                self.vvec = -(19 << 7);
            }
            self.falling = 1;
            self.bidforsound(PLAYERCHANNEL, 0x5f, 0, SfxId::JUMP);
        }

        //jumpins:
        let r11 = self.translate(self.xpos, self.ypos + (1 << 8));
        let p7 = r11 + bw;
        let p8 = p7 + 1;
        self.pladr[7] = p7;
        self.pladr[8] = p8;
        if self.falling >= 8 && self.vvec >= (2 << 8) {
            self.crumblecheck(p7);
            self.crumblecheck(p8);
        }
        if self.cell(p7) < BLIM && self.cell(p8) < BLIM {
            self.falling = self.falling.wrapping_add(1);
            if self.falling == 0xff {
                self.falling = 0xfe;
            }
            // (falling == 0 here: standing on a platform)
            if self.falling > 1 && self.vvec < (12 << 8) {
                self.vvec += 8 << 4;
            }
        } else {
            //nofall:
            self.falling = 0;
        }
        //fallins:
        let c8 = self.cell(p8);
        if (PLATLOWLIM + 1..=PLATHIGHLIM + 1).contains(&c8) && (c8 & 1) != 0 {
            self.set_cell(p8 - 1, c8 - 1);
        }
        //noplatrebuild:
        let c7 = self.cell(p7);
        if (PLATLOWLIM..=PLATHIGHLIM).contains(&c7) {
            if c7 != PLATHIGHLIM {
                self.hiddenplatctr = 0;
                //nohidplat:
                if c7 & 1 == 0 {
                    self.plplattoobj(p7);
                    return;
                }
            } else {
                self.hiddenplatctr = self.hiddenplatctr.wrapping_add(self.frameinc as u8);
                if self.hiddenplatctr >= 50 && c7 & 1 == 0 {
                    self.hiddenplatctr = 0;
                    self.plplattoobj(p7);
                    return;
                }
            }
        }

        //fallinscont:
        let c7 = self.cell(p7);
        if c7 & 1 == 0 && (TELEPLOWLIM..=TELEPHIGHLIM).contains(&c7) {
            self.telep();
        }

        //notelep:
        // Deviation: the C port falls through to the final playerfire() after these two
        // plplattoobj calls (which already end in playerfire) — the original branches to
        // plplattoobj, whose tail is the shared `plattoins` exit, so it fires only once.
        let head = self.pladr[1] - bw;
        let visible_plat = |c: u8| (PLATLOWLIM..PLATHIGHLIM).contains(&c); // not hidden platforms
        if visible_plat(self.cell(head)) {
            self.plplattoobj(head);
            return;
        } else if visible_plat(self.cell(head + 1)) {
            //notelepcont1:
            self.plplattoobj(head + 1);
            return;
        } else if self.cell(p7) == EXTENDNO + 1 {
            //notelepcont2: doextendfromleft:
            if self.extending == 0 && self.cell(p7 + 1) == 0 {
                let (x, y) = self.backtranslate(p7);
                self.makeobj(EXTENDER, x - (8 << 8), y + (15 << 8), 1 << 8, 0, EXTENDNO as i32, 0);
                self.extending = 20;
            }
        } else if self.cell(p8) == EXTENDNO + 2 {
            //doextendfromright:
            if self.extending == 0 && self.cell(p8 - 1) == 0 {
                let (x, y) = self.backtranslate(p8);
                self.makeobj(EXTENDER, x + (8 << 8), y + (15 << 8), -(1 << 8), 0, EXTENDNO as i32, 0);
                self.extending = 20;
            }
        } else if self.cell(p7) == SHUTDOWNNO {
            self.set_cell(p7, 14);
            self.shutdownctr = 1;
            self.neuronctr += 1;
            self.bidforsoundforce(3, 0x1f, (3 << 6) - 96, SfxId::SHUTDOWN_1);
            self.bidforsoundforce(2, 0x1f, (2 << 6) - 96, SfxId::SHUTDOWN_2);
            self.bidforsoundforce(1, 0x1f, (1 << 6) - 96, SfxId::SHUTDOWN_3);
        }
        //plattoins:
        self.playerfire();
    }

    pub fn windcheck(&mut self) {
        if self.cell(self.pladr[1]) == 0 {
            return;
        }
        let (mut h, mut v) = (self.hvec, self.vvec);
        let mut r = self.seeifwind(self.pladr[1], &mut h, &mut v, false);
        r = self.seeifwind(self.pladr[2], &mut h, &mut v, r);
        r = self.seeifwind(self.pladr[3], &mut h, &mut v, r);
        r = self.seeifwind(self.pladr[4], &mut h, &mut v, r);
        self.hvec = h;
        self.vvec = v;
        if !r {
            self.windctr -= 64;
        }
        self.windctr -= 1;
        if self.windctr < 0 {
            self.windctr = 0;
        }
    }

    pub fn deletepoint(&mut self) {
        for r3 in (1..=16).rev() {
            //delloop: dodeletepoint:
            let a = self.random() & 15;
            let b = self.random() & 7;
            let mut x = self.xpos + ((a + b - 11) << 12);
            if x < (1 << 12) {
                return;
            }
            let mut y = self.ypos + (((self.random() & 15) - 7) << 12);
            if y < (1 << 12) {
                return;
            }
            if x > self.xposmax - (1 << 12) {
                return;
            }
            if y > self.yposmax - (1 << 12) {
                return;
            }
            x &= !0xfff;
            y &= !0xfff;
            let z = self.translate(x, y);
            if self.cell(z) == 0 {
                return;
            }
            self.set_cell(z, 0);
            self.makeobj(DYINGBONUS, x, y + (15 << 8), 0, 0, r3 | (1 << 20), 666);
        }
    }

    pub fn alfire(&mut self) {
        let a = self.random() & 15;
        let b = self.random() & 7;
        let mut x = self.xpos + ((a + b - 11) << 12);
        let mut y = self.ypos + (((self.random() & 15) - 7) << 12);

        if x < (1 << 12) {
            return;
        }
        if y < (1 << 12) {
            return;
        }
        if x > self.xposmax - (1 << 12) {
            return;
        }
        if y > self.yposmax - (1 << 12) {
            return;
        }
        x &= !0xfff;
        y &= !0xfff;
        let ady = ((self.ypos + (12 << 8)) - y - (16 << 8)) >> 6;
        if self.foundtarget(x, y, (self.xpos - x) >> 6, ady) {
            return;
        }
        //nofoundtarget:
        self.foundtarget(x + (1 << 12), y, (self.xpos - x - (1 << 12)) >> 6, ady);
    }

    pub fn makescoreobj(&mut self, x: i32, y: i32, type_: i32) {
        let dx = (self.random() & (0xfe << 1)) - 0xfe + (self.hvec >> 2);
        let dy = -(self.random() & (0xfe << 2));
        self.makeobj(SCOREOBJ, x, y, dx, dy, type_, 666);
    }

    pub fn foundmarker(&mut self, r11: BIdx) {
        self.bonusxpos = self.xpos;
        self.bonusypos = self.ypos;
        self.foundresetmarker(r11);
    }

    pub fn foundresetmarker(&mut self, r11: BIdx) {
        self.set_cell(r11, 0);
        let (x, y) = self.backtranslate(r11);
        self.telepxpos = x;
        self.telepypos = y - (1 << 8);
        self.telepctr = 1;
        self.bonustimer = 0x180;
    }

    pub fn normreset(&mut self) {
        self.telepctr = 1;
        self.telepxpos = self.bonusxpos;
        self.telepypos = self.bonusypos;
        self.bonustimer = 0;
    }

    pub fn dowakeup(&mut self, r7: BIdx) {
        let r0 = self.cell(r7);
        if !(240..=253).contains(&r0) {
            return;
        }
        self.set_cell(r7, 0);
        let (r1, r2) = self.backtranslate(r7);
        let r0 = r0.wrapping_sub((240 - ALIEN1) as u8) as i32;
        self.makeobj(r0, r1 + (8 << 8), r2 + (7 << 8), 0, 0, 0, (r0 << 9) - ((ALIEN1 - 1) << 9));
    }

    pub fn telep(&mut self) {
        self.initplx = self.xpos;
        self.initply = self.ypos;
        if self.ks.downpress < 16 {
            return;
        }
        if self.telepctr != 0 {
            return;
        }

        let p7 = self.pladr[7];
        let r4 = self.cell(p7);
        let mut r5 = if r4 & 2 != 0 { 1 } else { -1 };
        let ks = self.ks;
        if ks.leftpress >= 64 && ks.uppress == 0 && ks.fire != 0 && ks.fire <= ks.downpress && ks.rightpress >= 64 {
            r5 = -r5;
            self.bidforsound(EXPLOCHANNEL, 0x7f, 0, SfxId::TELEP_1);
        }
        self.ks.downpress = 0;
        let Some(r1) = self.normtelep(p7, r5, r4 ^ 2) else {
            return;
        };
        //telepfound:
        let (x, y) = self.backtranslate(r1);
        self.telepxpos = x + (8 << 8);
        self.telepypos = y - (17 << 8);
        self.telepctr = 1;
        self.bidforsound(PLAYERCHANNEL, 0x7f, 127, SfxId::TELEP_2);
        self.bidforsound(SPARECHANNEL, 0x7f, -127, SfxId::TELEP_3);
    }

    pub fn playerfire(&mut self) {
        if self.snuffctr != 0 {
            return;
        }
        if self.blamctr != 0 {
            self.blamctr = self.blamctr.wrapping_add(2);
        }
        if self.blamctr >= 224 {
            self.goblam();
            return;
        }
        if self.plweapontype == 0 {
            return;
        }
        if self.ks.fire != 0 {
            self.dofire();
        } else if self.rocketflag != 0 {
            // "I added this, can't find it in the source" (C porter)
            self.launchrocket();
        }
    }

    pub fn goblam(&mut self) {
        if self.plweapontype == ROCKETBLAMNO {
            self.rocketblam();
            return;
        }
        for r9 in (1..=32).rev() {
            //loopa0:
            let r3: i32 = r9 - 16;
            let r4 = r3.wrapping_abs();
            let (r0, r2) = if self.plface == 1 {
                (self.xpos - (20 << 8), (r4 << 5) - (4 << 8))
            } else {
                (self.xpos + (20 << 8), (4 << 8) - (r4 << 5))
            };
            self.makeproj(r0, self.ypos + (12 << 8), r2, r3 << 6, 57, PROJ_ROCKET);
        }
        let (r1, r3) = if self.plface == 1 {
            (self.xpos - (24 << 8), -(1 << 8))
        } else {
            (self.xpos + (24 << 8), 1 << 8)
        };
        self.explogonopyroquiet(r1, self.ypos + (4 << 8), r3, 0, 0, 0, None);
        self.plfired = 1;
        self.blamctr = 0;
        self.bidforsound(FIRECHANNEL, 0x7f, 0, SfxId::BLAM);
    }

    pub fn rocketblam(&mut self) {
        for r9 in (1..=32).rev() {
            //loopa1:
            let r1 = self.ypos + (2 << 8);
            let mut r3 = self.rocktab(r9 * 6); // r9 = 32 reads the guard entry 192
            let (r0, mut r4, r2);
            if self.plface == 1 {
                r0 = self.xpos - (12 << 8);
                r4 = 1;
                r2 = -(4 << 8);
                r3 = -r3;
            } else {
                r0 = self.xpos + (12 << 8);
                r4 = 0;
                r2 = 4 << 8;
            }
            r3 *= 2;

            if r3 > 170 {
                r4 += 2;
            }
            if r3 > 60 {
                r4 += 2;
            }
            if r3 < -60 {
                r4 -= 2;
            }
            if r3 < -170 {
                r4 -= 2;
            }
            r4 += 36;
            self.makeproj(r0 + r2 * 2, r1 + r3 * 2, r2, r3, r4, PROJ_WEIRDSPLIT | PROJ_ATOM);
        }

        let (r1, r3) = if self.plface == 1 {
            (self.xpos + (24 << 8), 1 << 9)
        } else {
            (self.xpos - (24 << 8), -(1 << 9))
        };
        self.explogonopyroquiet(r1, self.ypos - (8 << 8), r3, 0, 1, 0, None);

        self.plfired = 1;
        self.blamctr = 0;

        self.bidforsound(FIRECHANNEL, 0x7f, 0, SfxId::BLAM);
        self.rocketblamctr = self.rocketblamctr.wrapping_sub(1);
        if (self.rocketblamctr & (1 << 7)) == 0 {
            return;
        }
        self.plweapontype = ROCKBASE;
        self.explogoquiet(self.xpos, self.ypos + (4 << 8), 0, -(1 << 7), 0, 0, None);
    }

    pub fn dofire(&mut self) {
        if self.plweapontype >= ROCKBASE {
            self.firerocket();
            return;
        }
        //firempmg:
        if self.plweapontype == MPMGBLAMNO {
            self.blamfire();
            return;
        }

        if self.framectr - self.firelastframe < self.firerate as i32 {
            return;
        }
        self.firelastframe = self.framectr;
        let speed = (self.plweaponspeed as i32) << 8;
        let mut r2 = if self.plface == 1 { -(self.hvec >> 1) - speed } else { (self.hvec >> 1) + speed };
        let r5 = match self.plweapontype & 7 {
            2 => (4 * PROJ_TTL) | PROJ_SPLIT,
            3 => (4 * PROJ_TTL) | PROJ_SPLIT | PROJ_FIVEWAY,
            4 => (4 * PROJ_TTL) | PROJ_SPLIT | PROJ_FIVEWAY | PROJ_SLOWSPLIT,
            5 => {
                r2 >>= 2;
                (20 * PROJ_TTL) | PROJ_SPLIT | PROJ_SLOWSPLIT | PROJ_WEIRDSPLIT
            }
            6 => (4 * PROJ_TTL) | PROJ_SPLIT | PROJ_EXPLO,
            7 => (4 * PROJ_TTL) | PROJ_SPLIT | PROJ_FIVEWAY | PROJ_EXPLO,
            _ => 64 * PROJ_TTL,
        };
        let x = self.xpos + if self.plface == 1 { -(14 << 8) } else { 14 << 8 };
        let dy = (self.random() & 0xff) - 0x7f;
        let type_ = if self.plface == 1 { 31 } else { 30 };
        self.makeproj(x, self.ypos + (12 << 8), r2, dy, type_, r5);
        self.plfired = 1;
        self.bidforsound(FIRECHANNEL, 0x7e, 0, SfxId::FIRE);
    }

    pub fn blamfire(&mut self) {
        if self.blamctr != 0 {
            return;
        }
        self.blamctr = if self.plweapontype == 8 { 128 } else { 1 };
        self.bidforsound(FIRECHANNEL, 0x7e, 0, SfxId::BLAMFIRE);
    }

    pub fn firerocket(&mut self) {
        if self.plweapontype == ROCKETBLAMNO {
            self.blamfire();
            return;
        }
        self.rocketflag = 1;
        let mut r2 = 2;
        if self.plweapontype == ROCKBASE + 6 {
            r2 = 8;
        } else if self.plweapontype > ROCKBASE {
            // >= _rockbase+1
            r2 = 4;
        }
        if self.plface == 1 {
            self.rocketctr += r2;
        } else {
            self.rocketctr -= r2;
        }
        if self.rocketctr >= ROCKETTABLEN {
            self.rocketctr = 0;
        }
        if self.rocketctr < 0 {
            self.rocketctr = ROCKETTABLEN - 1;
        }
    }

    pub fn launchrocket(&mut self) {
        if self.framectr - self.firelastframe < self.firerate as i32 {
            return;
        }
        self.firelastframe = self.framectr;
        let speed = (self.plweaponspeed as i32) << 8;
        let (r0, r2, r3, mut r4) = if self.plface == 1 {
            (self.xpos - (12 << 8), -speed, -self.rocktab(self.rocketctr), 1)
        } else {
            (self.xpos + (12 << 8), speed, self.rocktab(self.rocketctr), 0)
        };
        let r1 = self.ypos + (2 << 8);
        if r3 > 170 {
            r4 += 2;
        }
        if r3 > 60 {
            r4 += 2;
        }
        if r3 < -60 {
            r4 -= 2;
        }
        if r3 < -170 {
            r4 -= 2;
        }
        r4 += 36;
        let mut r6 = self.plweapontype as i32 - ROCKBASE as i32;
        if r6 >= 6 {
            r6 = self.random() & 7;
        }
        let mut r5 = match r6 {
            1 => (8 * PROJ_TTL) | PROJ_SPLIT | ROCK_DIVIDE,                  // split
            2 => (8 * PROJ_TTL) | PROJ_SPLIT | ROCK_DIVIDE | ROCK_REDIVIDE, // split twice
            3 => (20 * PROJ_TTL) | PROJ_SPLIT | PROJ_ROCKET | ROCK_BURST,   // burst a safe distance away
            4 => (14 * PROJ_TTL) | PROJ_SPLIT | PROJ_ROCKET | ROCK_DIVIDE | ROCK_BURST, // dual burst
            5 => (8 * PROJ_TTL) | PROJ_SPLIT | PROJ_ROCKET | ROCK_DIVIDE | ROCK_REDIVIDE | ROCK_BURST, // quad burst
            _ => 64 * PROJ_TTL,
        };
        r5 |= PROJ_ROCKET;
        self.makeproj(r0 + r2 + r2, r1 + r3 + r3, r2, r3, r4, r5);
        self.plfired = 1;
        self.bidforsound(FIRECHANNEL, 0x7e, 0, SfxId::ROCKET);
        self.rocketflag = 0;
    }

    pub fn getarms(&mut self) {
        self.blamctr = 0;
        self.plweapontype = 0;
    }

    pub fn getrocket(&mut self) {
        self.plweapontype = ROCKBASE + (self.random() & 7) as u8;
        self.plweaponspeed = 2;
        self.firerate = 12;
        self.blamctr = 0;
    }

    pub fn getmpmg(&mut self) {
        self.plweapontype = MPMGBASE + (self.random() & 7) as u8;
        self.plweaponspeed = 8;
        self.firerate = 4;
        self.blamctr = 0;
    }

    pub fn prepstrength(&mut self) {
        self.plstrength = STRENGTHINIT;
        self.laststrength = STRENGTHINIT;
        self.snuffctr = 0;
        self.lagerctr = 0;
    }

    /// Escape menu "Q": lagerctr = 0; plstrength = 0 (rejoin is frontend work).
    pub fn loselife(&mut self) {
        self.lagerctr = 0;
        self.plstrength = 0; // no return; rejoin() is done by the frontend
    }

    pub fn update_show_strength(&mut self) {
        if self.lagerctr != 0 {
            self.lagerctr -= self.frameinc;
            if self.lagerctr < 0 {
                self.lagerctr = 0;
            }
            self.plstrength += self.frameinc << 6;
            if self.plstrength > STRENGTHINIT {
                self.plstrength = STRENGTHINIT;
            }
            self.laststrength += self.frameinc << 6;
            if self.laststrength > STRENGTHINIT {
                self.laststrength = STRENGTHINIT;
            }
        }
        // showstrength(plstrength): drawn by the renderer.
    }

    pub fn bonuscheck(&mut self) {
        if self.snuffctr != 0 {
            //deadbonuscheck:
            self.atombombctr = 0;
            self.electrocuting = 0;
            for i in 1..=4 {
                self.deadbonuslim(self.pladr[i]);
            }
            return;
        }

        self.atombombctr = 0;
        self.electrocuting = 0;
        for i in 1..=4 {
            let p = self.pladr[i];
            let loss = self.plbombcheck(p);
            self.plstrength -= loss;
            self.bonuslim(p);
            self.weaponcheck(p);
        }
    }

    pub fn bonuscommon(&mut self, bonus: i32, x: i32, y: i32) {
        self.sortbonus(bonus as u8);
        let mut score = (bonus + 2) >> 1;
        if score > 10 {
            score = 10;
        }
        self.addtoscore(score * 100);
        self.makescoreobj(x, y, 0xf60 + (score << 8));
        // CHUNK_OBJGOT[score-1]; bonus is never negative (index clamped for safety)
        self.bidforsound(PLAYERCHANNEL, 0x6e, 0, SfxId::objgot((score - 1).max(0) as u8));
    }

    pub fn sortbonus(&mut self, r0: u8) {
        if r0 == 13 || r0 == 14 {
            //bonusstrength:
            self.lagerctr += if r0 == 14 { 150 } else { 50 };
            self.laststrength = self.plstrength;
        }

        if r0 == self.bonusctr {
            //advancebonus:
            self.bonusctr = self.bonusctr.wrapping_add(1);
            //sortreplot:
            self.bonusreplot = 28;
            return;
        }

        if r0 == 15 {
            self.bonusctr = 0;
            if self.snuffctr != 0 && self.plstrength > 0 {
                self.prepstrength();
                // they cross the centred band; on taller screens they live long enough to
                // travel the extra height above and below it (2 ticks per pixel)
                let time = 400 + 2 * self.lay.cy;
                self.message_band(time, 48, 24, 0.0, 0.5, TextRef::Key(keys::MSG_POTION), Align::EnCentered);
                self.message_band(time, 48, 176, 0.0, -0.5, TextRef::Key(keys::MSG_RISE), Align::EnCentered);
            }
            //sortreplot:
            self.bonusreplot = 28;
        }
    }

    pub fn bonusnumb(&mut self, r9: i32) {
        let mut r6 = -1;
        let mut r9 = r9 & 0xff;
        while r9 > 0 {
            //loopc3:
            r6 = (r6 + 1) & 3; // was random()&3
            self.makescoreobj(self.xpos, self.ypos, 0xf60 + (10 << 8) + (r6 << 8));
            r9 -= r6 + 1;
        }
    }

    pub fn initweapon(&mut self) {
        self.firelastframe = 0;
        if self.plweapontype >= ROCKBASE {
            self.plweaponspeed = 2;
            self.firerate = 12;
        } else {
            self.plweaponspeed = 8;
            self.firerate = 4;
        }
        self.rocketblamctr = 5;
    }

    /// Returns true when (x, y) ± (dx, dy)/2 overlaps the player box (C returns 0 for EQ).
    pub fn plcolcheck(&self, x: i32, y: i32, dx: i32, dy: i32) -> bool {
        (x + (dx >> 1) > self.pllx)
            && (self.plhx > x - (dx >> 1))
            && (y + (dy >> 1) > self.plly)
            && (self.plhy > y - (dy >> 1))
    }

    pub fn weaponcheck(&mut self, r5: BIdx) {
        let weap = block_weapon(self.cell(r5));
        if weap == 0 {
            return;
        }
        self.plweapontype = weap as u8;
        self.set_cell(r5, 0);
        if self.plweapontype >= ROCKBASE {
            self.plweaponspeed = 2;
            self.firerate = 12;
        } else {
            self.plweaponspeed = 8;
            self.firerate = 4;
        }
        self.rocketblamctr = 5;

        let r2 = self.plweapontype as i32 - 1;
        let (r7, r0) = if (r2 & 7) == 7 {
            (SCORESPRBASE + 12, 4000)
        } else if r2 & 4 != 0 {
            (SCORESPRBASE + 10, 2000)
        } else {
            (SCORESPRBASE + 9, 1000)
        };
        self.addtoscore(r0);

        self.makescoreobj(self.xpos, self.ypos, 60 + (r7 << 8));
        let mega = r7 == SCORESPRBASE + 12;
        self.bidforsound(PLAYERCHANNEL, 0x7f, 127, if mega { SfxId::WEAPON_3 } else { SfxId::WEAPON_1 });
        self.bidforsound(SPARECHANNEL, 0x7f, -127, if mega { SfxId::WEAPON_4 } else { SfxId::WEAPON_2 });
        // XXX Original code also kills message so the same one is never redisplayed
        let idx = (self.plweapontype.wrapping_sub(1) & 15) as usize;
        self.message_scroll(TextRef::Key(keys::WEAPONS[idx]));
    }

    pub fn scoreadd(&mut self) {
        if self.plscoreadd == 0 {
            return;
        }
        let mut placeval: i32 = 100_000_000;
        for r10 in 0..8 {
            placeval /= 10;
            //loop37:
            if self.plscoreadd >= placeval {
                let mut digit: u8 = 0;
                for bit in (0..4).rev() {
                    if self.plscoreadd >= (placeval << bit) {
                        digit += 1 << bit;
                        self.plscoreadd -= placeval << bit;
                    }
                }
                self.plscore[r10] = self.plscore[r10].wrapping_add(digit);
            }
            //scoreaddskip:
        }
        let mut carry: u8 = 0;
        for r10 in (0..8).rev() {
            // now handle carries
            //loop38:
            self.plscore[r10] = self.plscore[r10].wrapping_add(carry);
            if self.plscore[r10] >= 10 {
                self.plscore[r10] -= 10;
                carry = 1;
            } else {
                carry = 0;
            }
        }
    }

    pub fn plplattoobj(&mut self, r0: BIdx) {
        self.plattoobjins(r0, self.vvec >> 3);
        self.vvec = 0;
        //plattoins:
        self.playerfire();
    }

    pub fn addtoscore(&mut self, sc: i32) { self.plscoreadd = self.plscoreadd.wrapping_add(sc); }
    pub fn pllosestrength(&mut self, s: i32) { self.plstrength = self.plstrength.wrapping_sub(s); }

    pub fn plsetneuronzone(&mut self, zone: i32) {
        self.initplx = self.xpos;
        self.initply = self.ypos;
        self.change_zone(zone);
    }

    pub fn bonusplot(&mut self) {
        let r2 = if self.frameinc > 2 { 2 } else { self.frameinc };

        if self.bonustimer != 0 {
            self.bonustimer -= r2;
        }
        if self.bonustimer == 1 || self.bonustimer == 2 {
            self.bonusreset();
        }

        if self.bonusreplot != 0 {
            self.bonusreplot -= r2 as i16; // from above
            if self.bonusreplot < 0 {
                self.bonusreplot = 0;
            }
        } else if self.bonusctr >= 13 {
            self.bonusbonus();
        }
        // plotbonus(bonusctr, bonusreplot): drawn by the renderer.
    }

    pub fn bonusbonus(&mut self) {
        self.bonusctr = 8;
        self.bonusreplot = 28;
        self.ks.keypressed = 0;
        match self.bonusfind() {
            Some(r11) => self.foundmarker(r11),
            None => self.bonus1(),
        }
    }

    pub fn bonusreset(&mut self) {
        if self.ks.keypressed != 0 {
            self.normreset();
            return;
        }
        self.bonusctr = 0;
        self.bonusreplot = 28;
        self.ks.keypressed = 1;
        match self.bonusfind() {
            Some(r11) => self.foundresetmarker(r11),
            None => {
                self.bonus1();
                self.normreset();
            }
        }
    }

    /// asylum.c `bonus1()`.
    pub fn bonus1(&mut self) {
        self.bonusnumb(10);
        self.message_hud_bottom(96, 224, 0.0, -2.0, TextRef::Key(keys::MSG_BONUS_10K), Align::EnCentered);
        self.addtoscore(10000);
    }

    pub fn restartplayer(&mut self) {
        self.xpos = self.initplx;
        self.ypos = self.initply;
        self.screenwakeup(self.xpos, self.ypos);
        self.reinitplayer();
    }

    pub fn reinitplayer(&mut self) {
        self.telepctr = 33;
        //bonusctr=0; (commented out in original)
        self.bonusreplot = 28;
        self.pladr[1] = PLADR_NULL;
        self.windctr = 0;
        self.bonustimer = 0;
        self.rocketblamctr = 0;
        self.shutdownctr = 0;
        if self.gotallneurons() {
            self.completedzone();
            self.lives = 0;
            self.snuffctr = 1;
        }
    }

    pub fn redraw_bonus(&mut self) { self.bonusreplot = 4; }
    pub fn gotallneurons(&self) -> bool { self.neuronctr >= NEURONSTOGET }

    /// 0 alive, 1 lost a life (zonerestart), 2 game over.
    pub fn player_dead(&mut self) -> i32 {
        if self.snuffctr >= 300 {
            //playerdead: (showgamescreen() is the renderer's job)
            if self.lives == 0 {
                return 2;
            }
            self.lives -= 1;
            if self.lives > 9 {
                self.lives = 9;
            }
            self.prepstrength();
            1
        } else {
            0
        }
    }

    pub fn scorezero(&mut self) {
        self.plscoreadd = 0;
        self.plscore = [0; 8];
        self.lives = 2;
        self.neuronctr = 0;
    }

    pub fn startplayer(&mut self) {
        self.bonusctr = 0;
        let (x, y) = self.findplayer();
        self.initplx = x;
        self.initply = y;
    }

    pub fn getvars(&mut self) {
        //getstrengthtab();
        self.xpos = 64 << 8;
        self.ypos = 48 << 8;
        self.initplx = 0;
        self.initply = 0;
        self.hvec = 0;
        self.vvec = 0;
        self.pladr[1] = PLADR_NULL;
        self.pladr[2] = PLADR_NULL;
        self.plface = 0;
    }

    /// keyboard.c `keyread(&ks)` from held bits.
    pub fn keyread(&mut self, held: u32) {
        let ks = &mut self.ks;
        press(&mut ks.leftpress, held & HELD_LEFT != 0);
        press(&mut ks.rightpress, held & HELD_RIGHT != 0);
        press(&mut ks.uppress, held & HELD_UP != 0);
        press(&mut ks.downpress, held & HELD_DOWN != 0);
        press(&mut ks.fire, held & HELD_FIRE != 0);
        if ks.leftpress != 0 || ks.rightpress != 0 || ks.uppress != 0 || ks.downpress != 0 || ks.fire != 0 {
            ks.keypressed = 1;
        }
    }

    /// keyboard.c `cheatread()` (F1 getmpmg, F2 getrocket, F3 prepstrength). The caller
    /// checks `cheatpermit == 1` like asylum.c; F4 (screensave) is not ported.
    pub fn cheatread(&mut self, cheat_keys: u8) {
        if cheat_keys & 1 != 0 {
            self.getmpmg();
        }
        if cheat_keys & 2 != 0 {
            self.getrocket();
        }
        if cheat_keys & 4 != 0 {
            self.prepstrength();
        }
    }

    /// menus.c `completedzone()` messages.
    pub fn completedzone(&mut self) {
        //completedzone:
        self.message_hud(114, 64, 0.0, 0.0, TextRef::Key(keys::MSG_WELL_DONE), Align::EnCentered);
        self.message_hud(44, 96, 0.0, 0.0, TextRef::Key(keys::MSG_COMPLETED), Align::EnCentered);
        let last = if self.mentalzone == 3 { keys::MSG_THE_GAME } else { keys::MSG_THIS_ZONE };
        self.message_hud(114, 128, 0.0, 0.0, TextRef::Key(last), Align::EnCentered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::testutil::sim;

    /// A sim with the board registers set up and an empty board.
    fn empty_sim() -> Sim {
        let mut s = sim();
        s.boardreg();
        for c in s.brain.cells.iter_mut() {
            *c = 0;
        }
        s
    }

    fn count_type(s: &Sim, t: i32) -> usize {
        s.al.iter().filter(|a| a.type_ == t).count()
    }

    #[test]
    fn keyread_counts_and_saturates() {
        let mut s = sim();
        assert_eq!(s.ks.keypressed, 0);
        s.keyread(0);
        assert_eq!(s.ks, KeyState::default());
        for i in 1..=254u32 {
            s.keyread(HELD_LEFT | HELD_FIRE);
            assert_eq!(s.ks.leftpress as u32, i);
            assert_eq!(s.ks.fire as u32, i);
        }
        s.keyread(HELD_LEFT);
        assert_eq!(s.ks.leftpress, 0xff);
        assert_eq!(s.ks.fire, 0);
        // ++0xff wraps to 0 and is forced back to 0xff
        for _ in 0..10 {
            s.keyread(HELD_LEFT | HELD_RIGHT | HELD_UP | HELD_DOWN);
            assert_eq!(s.ks.leftpress, 0xff);
        }
        assert_eq!((s.ks.rightpress, s.ks.uppress, s.ks.downpress), (10, 10, 10));
        assert_eq!(s.ks.keypressed, 1);
        // releasing clears the counters but never keypressed
        s.keyread(0);
        assert_eq!((s.ks.leftpress, s.ks.rightpress, s.ks.uppress, s.ks.downpress, s.ks.fire), (0, 0, 0, 0, 0));
        assert_eq!(s.ks.keypressed, 1);
        s.ks.keypressed = 0;
        s.keyread(0);
        assert_eq!(s.ks.keypressed, 0);
        s.keyread(HELD_DOWN);
        assert_eq!(s.ks.keypressed, 1);
    }

    #[test]
    fn scoreadd_digits_and_carries() {
        let mut s = sim();
        s.plscoreadd = 0;
        s.scoreadd();
        assert_eq!(s.plscore, [0; 8]);

        s.plscoreadd = 12_345_678;
        s.scoreadd();
        assert_eq!(s.plscore, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(s.plscoreadd, 0);

        s.plscore = [0, 0, 0, 9, 9, 9, 9, 9];
        s.plscoreadd = 1;
        s.scoreadd();
        assert_eq!(s.plscore, [0, 0, 1, 0, 0, 0, 0, 0]);

        s.plscore = [0, 0, 0, 0, 0, 7, 5, 0];
        s.plscoreadd = 950; // 7+9 = 16 -> carry, 5+5 = 10 -> carry
        s.scoreadd();
        assert_eq!(s.plscore, [0, 0, 0, 0, 1, 7, 0, 0]);

        // 99,999,999 + 1: the carry falls off the top digit
        s.plscore = [9; 8];
        s.plscoreadd = 1;
        s.scoreadd();
        assert_eq!(s.plscore, [0; 8]);

        // the top place can collect up to 15 (8+4+2+1) before the carry pass
        s.plscore = [0; 8];
        s.plscoreadd = 150_000_000;
        s.scoreadd();
        assert_eq!(s.plscore, [5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(s.plscoreadd, 0);
    }

    #[test]
    fn sortbonus_sequence_strength_and_reset() {
        let mut s = sim();
        s.bonusctr = 3;
        s.bonusreplot = 0;
        s.sortbonus(3);
        assert_eq!((s.bonusctr, s.bonusreplot), (4, 28));

        // wrong bonus: nothing changes
        s.bonusreplot = 0;
        s.sortbonus(7);
        assert_eq!((s.bonusctr, s.bonusreplot), (4, 0));

        // strength bonuses (13 = +50, 14 = +150) even when out of sequence
        s.plstrength = 1000;
        s.laststrength = 5000;
        s.sortbonus(13);
        assert_eq!((s.lagerctr, s.laststrength, s.bonusctr), (50, 1000, 4));
        s.sortbonus(14);
        assert_eq!(s.lagerctr, 200);

        // an in-sequence strength bonus also advances
        s.bonusctr = 13;
        s.sortbonus(13);
        assert_eq!((s.lagerctr, s.bonusctr), (250, 14));

        // skull (15) out of sequence while alive: reset to 0
        s.bonusctr = 5;
        s.bonusreplot = 0;
        s.snuffctr = 0;
        s.sortbonus(15);
        assert_eq!((s.bonusctr, s.bonusreplot), (0, 28));
        assert_eq!(s.plstrength, 1000);

        // dead with no strength: no resurrection
        s.bonusctr = 5;
        s.snuffctr = 40;
        s.plstrength = 0;
        s.sortbonus(15);
        assert_eq!((s.bonusctr, s.snuffctr), (0, 40));
    }

    #[test]
    fn sortbonus_potion_and_skull_resurrects() {
        let mut s = sim();
        s.bonusctr = 2;
        s.snuffctr = 40;
        s.plstrength = 1;
        s.lagerctr = 99;
        s.sortbonus(15);
        assert_eq!((s.bonusctr, s.bonusreplot), (0, 28));
        assert_eq!((s.snuffctr, s.lagerctr, s.plstrength, s.laststrength), (0, 0, STRENGTHINIT, STRENGTHINIT));
        let texts: Vec<_> = s.texts.visible().map(|t| t.text.clone()).collect();
        assert!(texts.contains(&TextRef::Key(keys::MSG_POTION)));
        assert!(texts.contains(&TextRef::Key(keys::MSG_RISE)));
    }

    #[test]
    fn player_state_round_trip() {
        let mut a = sim();
        a.lives = 3;
        a.plstrength = -5;
        a.neuronctr = 7;
        a.shutdownctr = 0x1234_5678;
        a.plscore = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut store = [0u8; 25];
        a.save_player_state(&mut store);
        assert_eq!(&store[0..4], &[3, 0, 0, 0]);
        assert_eq!(&store[4..8], &(-5i32).to_le_bytes());
        assert_eq!(&store[12..16], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&store[16..24], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(store[24], 0);

        let mut b = sim();
        b.restore_player_state(&store);
        assert_eq!((b.lives, b.plstrength, b.neuronctr, b.shutdownctr), (3, -5, 7, 0x1234_5678));
        assert_eq!(b.plscore, a.plscore);
    }

    #[test]
    fn player_round_trip_and_invalidation() {
        let mut a = sim();
        a.xpos = 0x12345;
        a.ypos = -77;
        a.initplx = 1 << 20;
        a.initply = 3 << 12;
        a.hvec = -256;
        a.vvec = 1024;
        a.bonusctr = 9;
        a.plweapontype = 14;
        let mut store = vec![0u8; STORE_LEN];
        a.save_player(&mut store);
        assert_eq!(&store[0..4], &SAVEVALID.to_le_bytes());
        assert_eq!(&store[0..4], b"IcOK");
        assert_eq!((store[28], store[29]), (9, 14));

        let mut b = sim();
        assert!(!b.restore_player(&mut store));
        assert_eq!((b.xpos, b.ypos, b.initplx, b.initply, b.hvec, b.vvec), (0x12345, -77, 1 << 20, 3 << 12, -256, 1024));
        assert_eq!((b.bonusctr, b.plweapontype), (9, 14));
        // the C code invalidates the store after restoring it
        assert_eq!(&store[0..4], &[0, 0, 0, 0]);
        let mut c = sim();
        assert!(c.restore_player(&mut store));
        assert_eq!(c.xpos, 64 << 8);
    }

    #[test]
    fn plcolcheck_box() {
        let mut s = sim();
        s.pllx = 100 << 8;
        s.plhx = 116 << 8;
        s.plly = 50 << 8;
        s.plhy = 82 << 8;
        assert!(s.plcolcheck(108 << 8, 60 << 8, 0, 0));
        // edges are exclusive
        assert!(!s.plcolcheck(100 << 8, 60 << 8, 0, 0));
        assert!(!s.plcolcheck(116 << 8, 60 << 8, 0, 0));
        assert!(!s.plcolcheck(108 << 8, 82 << 8, 0, 0));
        // half the extent counts on each side
        assert!(s.plcolcheck(98 << 8, 60 << 8, 6 << 8, 0));
        assert!(!s.plcolcheck(98 << 8, 60 << 8, 4 << 8, 0));
        assert!(s.plcolcheck(108 << 8, 84 << 8, 0, 6 << 8));
        // the dead player's fake box never collides
        s.plhx = 0;
        s.plhy = 0;
        s.pllx = 1;
        s.plly = 1;
        assert!(!s.plcolcheck(0, 0, 0, 0));
        assert!(!s.plcolcheck(0, 0, 2, 2));
    }

    #[test]
    fn bonusnumb_makes_score_objects() {
        // 10 -> r6 = 0,1,2,3 (10-1-2-3-4 = 0): four objects
        let mut s = sim();
        s.bonusnumb(10);
        assert_eq!(count_type(&s, SCOREOBJ), 4);
        let mut s = sim();
        s.bonusnumb(1);
        assert_eq!(count_type(&s, SCOREOBJ), 1);
        // only the low byte counts: 0x100 -> nothing
        let mut s = sim();
        s.bonusnumb(0x100);
        assert_eq!(count_type(&s, SCOREOBJ), 0);
        // 11-1-2-3-4 = 1 > 0 -> a fifth object
        let mut s = sim();
        s.bonusnumb(11);
        assert_eq!(count_type(&s, SCOREOBJ), 5);
    }

    #[test]
    fn playerplot_alive_uses_player_clip() {
        let mut s = sim();
        s.plot_key = KEY_ALIEN + 5;
        s.snuffctr = 0;
        s.telepctr = 0;
        s.plstrength = STRENGTHINIT;
        s.laststrength = STRENGTHINIT;
        s.plface = 0;
        s.xpos = 0; // plframe 0
        s.plweapontype = 0;

        // only plotted by the call matching the dead/alive state
        s.playerplot(true);
        assert!(s.cmds.is_empty());
        s.playerplot(false);
        let idx: Vec<u8> = s.cmds.iter().map(|c| c.idx).collect();
        assert_eq!(idx, vec![0, 6, 12, 2]); // body, legs, head, arms
        for c in &s.cmds {
            assert_eq!(c.clip, Clip::PlayerFloor);
            assert_eq!(c.bank, Bank::Bloke);
            assert_eq!(c.key, KEY_NONE);
            assert_eq!(c.kind, CmdKind::Sprite);
        }
        // blokeplot(…, 0, 8) -> Center{0, 8 - 8}; legs at 19 -> 11; head at -2 -> -10
        assert_eq!(s.cmds[0].pos, Pos::Center { x: 0, y: 0 });
        assert_eq!(s.cmds[1].pos, Pos::Center { x: 0, y: 11 });
        assert_eq!(s.cmds[2].pos, Pos::Center { x: 0, y: -10 });
        assert_eq!(s.cmds[3].pos, Pos::Center { x: 0, y: 1 });

        // facing left, running frame 3 -> 2+3 = 5, mini-gun
        s.cmds.clear();
        s.plface = 1;
        s.xpos = 3 << 10;
        s.plweapontype = 2;
        s.plfired = 1;
        s.playerplot(false);
        let idx: Vec<u8> = s.cmds.iter().map(|c| c.idx).collect();
        assert_eq!(idx, vec![1, 11, 15, 43, 45]);
        assert_eq!(s.cmds[3].pos, Pos::Center { x: -6, y: 2 });
        assert_eq!(s.plfired, 0);

        // the blam weapon comes from the zone's block bank
        s.cmds.clear();
        s.plweapontype = MPMGBLAMNO;
        s.playerplot(false);
        assert_eq!(s.cmds[3].bank, s.blocks_bank());
        assert_eq!(s.cmds[3].idx, WEAPMPMGBLAM + 1);
        assert!(s.cmds.iter().all(|c| c.clip == Clip::PlayerFloor));
    }

    #[test]
    fn playerplot_dead_and_teleport_offsets() {
        let mut s = sim();
        s.snuffctr = 150;
        s.plstrength = 0;
        s.laststrength = 0;
        s.playerplot(false);
        assert!(s.cmds.is_empty());
        s.playerplot(true);
        assert_eq!(s.snuffctr, 151);
        assert_eq!(s.plotterofs, (151 - 96) >> 1);
        assert_eq!(s.cmds.len(), 1);
        assert_eq!(s.cmds[0].idx, 70);
        assert_eq!(s.cmds[0].clip, Clip::PlayerFloor);
        assert_eq!(s.cmds[0].pos, Pos::Center { x: 0, y: 8 + 27 - 8 });

        // teleport: the sprite sinks until plotterofs 31
        let mut s = sim();
        s.plstrength = STRENGTHINIT;
        s.laststrength = STRENGTHINIT;
        s.telepctr = 30;
        s.playerplot(false);
        assert_eq!((s.telepctr, s.plotterofs), (31, 31));
        assert_eq!(s.cmds[0].pos, Pos::Center { x: 0, y: 31 });
        // past the midpoint it rises again
        s.telepctr = 40;
        s.playerplot(false);
        assert_eq!((s.telepctr, s.plotterofs), (41, 23));
        s.telepctr = 64;
        s.playerplot(false);
        assert_eq!((s.telepctr, s.plotterofs), (0, 0));
    }

    #[test]
    fn seestars_tracks_strength_loss() {
        let mut s = sim();
        s.plstrength = 1000;
        s.laststrength = 500;
        s.seestars();
        assert_eq!(s.laststrength, 1000);
        assert!(s.cmds.is_empty());
        // loss of exactly one step: stars but no sound (r4 == 0 afterwards)
        s.plstrength = 1000 - 256;
        s.framectr = 0;
        s.seestars();
        assert_eq!(s.laststrength, 1000 - 256);
        assert_eq!(s.cmds.len(), 1);
        assert_eq!(s.cmds[0].idx, STARSSPRBASE);
        assert_eq!(s.cmds[0].clip, Clip::PlayerFloor);
        assert_eq!(s.cmds[0].pos, Pos::Center { x: 0, y: -6 - 8 });
        assert!(s.sfx_events.is_empty());
    }

    #[test]
    fn seestars_stunned_sound() {
        let mut s = sim();
        s.plstrength = 0;
        s.laststrength = 0x500;
        s.seestars();
        // r4 = 0x400 -> CHUNK_STUNNED[4] on the player channel
        assert_eq!(s.sfx_events.len(), 1);
        assert_eq!(s.sfx_events[0].id, SfxId::stunned(4));
        assert_eq!(s.sfx_events[0].channel, PLAYERCHANNEL as u8);
        assert_eq!(s.sfx_events[0].volume, 0x70);
    }

    #[test]
    fn seeifdead_creates_three_decorations() {
        let mut s = sim();
        s.plstrength = 1;
        s.seeifdead();
        assert_eq!(s.snuffctr, 0);
        s.plstrength = 0;
        s.lives = 0;
        s.seeifdead();
        assert_eq!(s.snuffctr, 1);
        assert_eq!(count_type(&s, DECORATION), 3);
        let r5: Vec<i32> = s.al.iter().filter(|a| a.type_ == DECORATION).map(|a| a.r5 & 0xffff).collect();
        assert!(r5.contains(&(DEADSPRBASE - 3)));
        assert!(r5.contains(&(DEADSPRBASE + 1)));
        assert!(r5.contains(&(DEADSPRBASE + 2)));
        // already dead: nothing more
        s.seeifdead();
        assert_eq!(count_type(&s, DECORATION), 3);
    }

    #[test]
    fn rocket_aim_wraps() {
        let mut s = sim();
        s.plweapontype = ROCKBASE + 6;
        s.plface = 1;
        s.rocketctr = 188;
        s.firerocket();
        assert_eq!((s.rocketctr, s.rocketflag), (0, 1));
        s.plface = 0;
        s.plweapontype = ROCKBASE;
        s.firerocket();
        assert_eq!(s.rocketctr, ROCKETTABLEN - 1);
        s.plweapontype = ROCKBASE + 1;
        s.firerocket();
        assert_eq!(s.rocketctr, ROCKETTABLEN - 5);
        s.initrockettab();
        assert_eq!((s.rocketctr, s.rocketflag), (0, 0));
    }

    #[test]
    fn strength_lives_and_death() {
        let mut s = sim();
        s.plstrength = STRENGTHINIT - 10;
        s.laststrength = STRENGTHINIT - 100;
        s.lagerctr = 1;
        s.update_show_strength();
        assert_eq!((s.lagerctr, s.plstrength, s.laststrength), (0, STRENGTHINIT, STRENGTHINIT - 36));
        s.update_show_strength(); // lagerctr 0: unchanged
        assert_eq!(s.laststrength, STRENGTHINIT - 36);

        s.snuffctr = 299;
        assert_eq!(s.player_dead(), 0);
        s.snuffctr = 300;
        s.lives = 12;
        assert_eq!(s.player_dead(), 1);
        assert_eq!((s.lives, s.snuffctr, s.plstrength), (9, 0, STRENGTHINIT));
        s.snuffctr = 300;
        s.lives = 0;
        assert_eq!(s.player_dead(), 2);

        s.loselife();
        assert_eq!((s.lagerctr, s.plstrength), (0, 0));
        s.plscore = [1; 8];
        s.plscoreadd = 5;
        s.scorezero();
        assert_eq!((s.plscore, s.plscoreadd, s.lives, s.neuronctr), ([0; 8], 0, 2, 0));
    }

    #[test]
    fn cheats_and_weapons() {
        let mut s = sim();
        s.plstrength = 3;
        s.snuffctr = 7;
        s.cheatread(4);
        assert_eq!((s.plstrength, s.snuffctr), (STRENGTHINIT, 0));
        s.cheatread(1);
        assert!((MPMGBASE..MPMGBASE + 8).contains(&s.plweapontype));
        assert_eq!((s.plweaponspeed, s.firerate), (8, 4));
        s.cheatread(2);
        assert!((ROCKBASE..ROCKBASE + 8).contains(&s.plweapontype));
        assert_eq!((s.plweaponspeed, s.firerate), (2, 12));
        s.getarms();
        assert_eq!(s.plweapontype, 0);
        s.plweapontype = ROCKBASE + 2;
        s.initweapon();
        assert_eq!((s.plweaponspeed, s.firerate, s.rocketblamctr), (2, 12, 5));
    }

    #[test]
    fn bonusplot_counts_down_replot() {
        let mut s = sim();
        s.bonusreplot = 1;
        s.bonusctr = 13;
        s.bonusplot();
        assert_eq!(s.bonusreplot, 0);
        assert_eq!(s.bonusctr, 13); // bonusbonus only after the replot is done
        s.bonustimer = 10;
        s.bonusreplot = 5;
        s.bonusplot();
        assert_eq!((s.bonustimer, s.bonusreplot), (9, 4));
    }

    #[test]
    fn bonusreset_with_star_marker() {
        let mut s = empty_sim();
        let bw = s.boardwidth;
        let star = 10 + 20 * bw;
        s.set_cell(star, STARSNO);
        s.ks.keypressed = 0;
        s.bonustimer = 3; // -> 2 -> bonusreset
        s.bonusplot();
        assert_eq!(s.cell(star), 0);
        assert_eq!((s.bonusctr, s.ks.keypressed, s.telepctr, s.bonustimer), (0, 1, 1, 0x180));
        assert_eq!((s.telepxpos, s.telepypos), (10 << 12, (20 << 12) - (1 << 8)));
        // a key was pressed meanwhile: back to the bonus start position
        s.bonusxpos = 5 << 12;
        s.bonusypos = 6 << 12;
        s.bonustimer = 3;
        s.bonusplot();
        assert_eq!((s.telepxpos, s.telepypos, s.bonustimer, s.telepctr), (5 << 12, 6 << 12, 0, 1));
    }

    #[test]
    fn windcheck_null_and_wind() {
        let mut s = empty_sim();
        s.getvars();
        s.windctr = 100;
        s.hvec = 7;
        s.windcheck(); // pladr1 NULL reads 0 -> early return
        assert_eq!((s.windctr, s.hvec), (100, 7));

        let bw = s.boardwidth;
        let p1 = 30 + 30 * bw;
        s.pladr[1] = p1;
        s.pladr[2] = p1 + 1;
        s.pladr[3] = p1 + bw;
        s.pladr[4] = p1 + bw + 1;
        s.set_cell(p1, 1); // non-empty, not wind
        s.set_cell(p1 + 1, 3); // blows right
        s.set_cell(p1 + bw, 12); // blows up
        s.hvec = 0;
        s.vvec = 0;
        s.windcheck();
        assert_eq!((s.hvec, s.vvec, s.windctr), (WINDSPEED * 4, -WINDSPEED, 99));
        // no wind cells: the counter drops by 65
        s.set_cell(p1 + 1, 1);
        s.set_cell(p1 + bw, 1);
        s.windcheck();
        assert_eq!(s.windctr, 99 - 65);
        s.windctr = 10;
        s.windcheck();
        assert_eq!(s.windctr, 0);
    }

    #[test]
    fn reinit_and_vars() {
        let mut s = sim();
        s.neuronctr = 3;
        s.windctr = 5;
        s.shutdownctr = 9;
        s.rocketblamctr = 4;
        s.reinitplayer();
        assert_eq!((s.telepctr, s.bonusreplot, s.windctr, s.shutdownctr, s.rocketblamctr), (33, 28, 0, 0, 0));
        assert_eq!(s.pladr[1], PLADR_NULL);
        assert_eq!(s.snuffctr, 0);
        s.xpos = 1;
        s.plface = 1;
        s.getvars();
        assert_eq!((s.xpos, s.ypos, s.plface, s.pladr[2]), (64 << 8, 48 << 8, 0, PLADR_NULL));
        s.xpos = 77;
        s.ypos = 88;
        s.plsetneuronzone(4);
        assert_eq!((s.initplx, s.initply, s.plzone), (77, 88, 4));
    }

    #[test]
    fn rockettab_guard_entry() {
        let s = sim();
        assert_eq!(s.rocktab(192), 0);
        assert_eq!(s.rocktab(-1), 0);
        assert_eq!(s.rocktab(50), s.rockettab[50]);
    }

    #[test]
    fn plmove_lands_walks_jumps_and_shuts_down() {
        let mut s = empty_sim();
        let bw = s.boardwidth;
        for x in 0..bw {
            s.set_cell(x + 50 * bw, SOLIDLOWLIM);
        }
        s.xpos = 20 << 12;
        s.ypos = 40 << 12;
        s.initplx = s.xpos;
        s.initply = s.ypos;
        s.prepstrength();
        for _ in 0..200 {
            s.plmove(TickInput::default());
        }
        assert_eq!(s.falling, 0);
        assert_eq!(s.pladr[7] / bw, 50, "standing on the floor row");
        let y = s.ypos;
        s.plmove(TickInput::default());
        assert_eq!(s.ypos, y);

        let x = s.xpos;
        for _ in 0..10 {
            s.plmove(TickInput { held: HELD_LEFT, cheat_keys: 0 });
        }
        assert!(s.xpos < x);
        assert_eq!((s.plface, s.ks.leftpress), (1, 10));
        assert_eq!(s.ypos, y);

        s.sfx_events.clear();
        s.plmove(TickInput { held: HELD_UP, cheat_keys: 0 });
        assert_eq!(s.vvec, -(19 << 7));
        assert!(s.sfx_events.iter().any(|e| e.id == SfxId::JUMP && e.volume == 0x5f));
        for _ in 0..200 {
            s.plmove(TickInput::default());
        }
        assert_eq!((s.falling, s.ypos >> 8, s.pladr[7] / bw), (0, y >> 8, 50), "landed again");

        // standing on the shutdown block of a neuron
        let p7 = s.pladr[7];
        s.set_cell(p7, SHUTDOWNNO);
        s.sfx_events.clear();
        s.plmove(TickInput::default());
        assert_eq!((s.cell(p7), s.shutdownctr, s.neuronctr), (14, 1, 1));
        let chans: Vec<u8> = s.sfx_events.iter().map(|e| e.channel).collect();
        assert_eq!(chans, vec![3, 2, 1]);
        assert_eq!(s.sfx_events[0].stereo, 96);
    }

    #[test]
    fn telep_needs_down_and_finds_partner() {
        let mut s = empty_sim();
        let bw = s.boardwidth;
        let p7 = 20 + 40 * bw;
        let dest = p7 + 9;
        s.pladr[7] = p7;
        s.set_cell(p7, TELEPLOWLIM);
        s.xpos = 3 << 12;
        s.ypos = 4 << 12;
        s.ks.downpress = 15;
        s.telep();
        assert_eq!((s.initplx, s.initply, s.telepctr), (3 << 12, 4 << 12, 0));
        // r4 = 192 (bit 1 clear) -> dir -1, looks for 192 ^ 2 = 194 to the left
        s.set_cell(p7 - 9, TELEPLOWLIM + 2);
        s.set_cell(dest, TELEPLOWLIM + 2);
        s.ks.downpress = 16;
        s.telep();
        assert_eq!(s.ks.downpress, 0);
        assert_eq!(s.telepctr, 1);
        let (x, y) = s.backtranslate(p7 - 9);
        assert_eq!((s.telepxpos, s.telepypos), (x + (8 << 8), y - (17 << 8)));
    }
}
