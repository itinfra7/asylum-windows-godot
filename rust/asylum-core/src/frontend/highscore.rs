//! menus.c `showhighscore` / `updatehst` / `comparescore` / `showhst` as a tick-driven state.
//!
//! Table layout (`highscorearea`): 5 entries of 13 bytes `"00000000 PSY\n"` + 1 CRC byte.
//! Initials are Archimedes bytes `'0'..='Z'+4`, displayed through `font::arc_char`.

use crate::font::arc_char;
use crate::input::{HELD_LEFT, HELD_RIGHT};
use crate::storage::HIGHSCORE_LEN;

const ENTRY: usize = 13;
const INITIAL_MIN: u8 = b'0';
const INITIAL_MAX: u8 = b'Z' + 4;
/// swi_blitz_wait(20) = 200 ms = 10 ticks (the check happens on the 10th).
const WAIT_LETTER: i32 = 9;
/// swi_blitz_wait(4) = 40 ms = 2 ticks between letter changes.
const WAIT_STEP: i32 = 1;

/// menus.c `comparescore(r10)`: true if the player's score is >= the entry at `entry`.
pub fn comparescore(entry: &[u8], plscore: &[u8; 8]) -> bool {
    for r3 in 0..8 {
        let e = entry[r3] as i32 - b'0' as i32;
        let p = plscore[r3] as i32;
        if e < p {
            return true; // scoregreater
        } else if e > p {
            return false; // scoreless
        }
    }
    true
}

/// menus.c `updatehst()` up to the initials loop: finds the rank, shifts the lower entries
/// down, writes the score digits and the current initials. Returns `hstindex` (5 = not on
/// the table, table untouched).
pub fn updatehst(table: &mut [u8; HIGHSCORE_LEN], plscore: &[u8; 8], initials: [u8; 3]) -> u8 {
    let mut hstindex: u8 = 5;
    let mut r10 = 4 * ENTRY;
    while comparescore(&table[r10..r10 + 8], plscore) {
        // loopd1:
        r10 = r10.wrapping_sub(ENTRY);
        hstindex -= 1;
        if hstindex == 0 {
            break;
        }
    }
    // lessthan:
    let r10 = r10.wrapping_add(ENTRY);
    if hstindex != 5 {
        let n = ENTRY * (4 - hstindex as usize);
        if n > 0 {
            // r9[13] = *r9 backwards from highscorearea + 4*13 - 1
            let start = 4 * ENTRY - n;
            table.copy_within(start..4 * ENTRY, start + ENTRY);
        }
        for (i, d) in plscore.iter().enumerate() {
            table[r10 + i] = d.wrapping_add(b'0');
        }
        table[r10 + 9..r10 + 12].copy_from_slice(&initials);
    }
    hstindex
}

/// Display text of line `i` as menus.c `showhst` builds it: one running pointer, up to
/// 12 bytes > 0x0a per line, then one byte skipped. A line that ends early shifts all later ones.
///
/// No bounds check is needed: line `k` (k <= 4) starts at `p <= 13*k`, so the highest index
/// read is 4*13+11 = 63 and the CRC byte (65) is never read.
pub fn entry_text(table: &[u8; HIGHSCORE_LEN], i: usize) -> String {
    let mut p = 0usize;
    let mut s = String::new();
    for k in 0..=i {
        s.clear();
        let mut j = 0;
        while j < 12 && table[p] > 0x0a {
            if k == i {
                s.push(arc_char(table[p]));
            }
            j += 1;
            p += 1;
        }
        p += 1; // skip over newline
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HsPhase {
    /// Editing initial `letter` (0..3) of the new entry.
    Entry { letter: usize },
    /// The third letter was confirmed; saving after the final wait.
    Finishing,
    /// "press fire" (readopt(0)).
    Done,
}

#[derive(Clone, Debug)]
pub struct HsState {
    pub table: [u8; HIGHSCORE_LEN],
    pub hstindex: u8,
    /// options.mentalzone the table belongs to.
    pub zone: u8,
    pub phase: HsPhase,
    /// Ticks to skip before the next loop check (swi_blitz_wait).
    pub wait: i32,
    /// A fire key-down is pending (the C `keybuf == firekey` check).
    pub fire: bool,
    /// The Id was unlocked by this game (shows MSG_ID_UNLOCKED).
    pub id_unlocked: bool,
}

impl HsState {
    /// `showhighscore()`: `loadscores` has produced `table`; runs `updatehst`'s insertion.
    pub fn new(mut table: [u8; HIGHSCORE_LEN], plscore: &[u8; 8], initials: [u8; 3], zone: u8, id_unlocked: bool) -> HsState {
        let hstindex = updatehst(&mut table, plscore, initials);
        let phase = if hstindex != 5 { HsPhase::Entry { letter: 0 } } else { HsPhase::Done };
        HsState { table, hstindex, zone, phase, wait: WAIT_LETTER, fire: false, id_unlocked }
    }

    /// Byte index of the initial being edited.
    pub fn letter_pos(&self, letter: usize) -> usize {
        self.hstindex as usize * ENTRY + 9 + letter
    }

    /// One 50 Hz tick of the initials loop. Confirmed letters are copied to `initials`
    /// (options.initials). Returns true on the tick the entry finished (caller saves the
    /// table and the config).
    pub fn tick(&mut self, held: u32, initials: &mut [u8; 3]) -> bool {
        match self.phase {
            HsPhase::Done => false,
            HsPhase::Finishing => {
                if self.wait > 0 {
                    self.wait -= 1;
                    return false;
                }
                self.phase = HsPhase::Done;
                true
            }
            HsPhase::Entry { letter } => {
                if self.wait > 0 {
                    self.wait -= 1;
                    return false;
                }
                let pos = self.letter_pos(letter);
                if std::mem::take(&mut self.fire) {
                    initials[letter] = self.table[pos];
                    self.wait = WAIT_LETTER;
                    self.phase = if letter == 2 { HsPhase::Finishing } else { HsPhase::Entry { letter: letter + 1 } };
                    return false;
                }
                // keyread(&ks): leftpress/rightpress are 0 when the key is up
                let mut c = self.table[pos];
                if held & HELD_LEFT == 0 {
                    c = c.wrapping_add(1);
                }
                if held & HELD_RIGHT == 0 {
                    c = c.wrapping_sub(1);
                }
                c = c.clamp(INITIAL_MIN, INITIAL_MAX);
                self.table[pos] = c;
                self.wait = WAIT_STEP;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::default_highscores;

    fn table_with(scores: [&[u8; 8]; 5]) -> [u8; HIGHSCORE_LEN] {
        let mut t = default_highscores();
        for (i, s) in scores.iter().enumerate() {
            t[i * 13..i * 13 + 8].copy_from_slice(*s);
            t[i * 13 + 9..i * 13 + 12].copy_from_slice(b"AB0");
        }
        t
    }

    #[test]
    fn insertion_shifts_lower_entries() {
        let mut t = table_with([b"00050000", b"00040000", b"00030000", b"00020000", b"00010000"]);
        let score = [0, 0, 0, 3, 5, 0, 0, 0];
        let idx = updatehst(&mut t, &score, *b"XYZ");
        assert_eq!(idx, 2);
        assert_eq!(&t[0..13], b"00050000 AB0\n");
        assert_eq!(&t[13..26], b"00040000 AB0\n");
        assert_eq!(&t[26..39], b"00035000 XYZ\n");
        assert_eq!(&t[39..52], b"00030000 AB0\n");
        assert_eq!(&t[52..65], b"00020000 AB0\n");
        assert_eq!(t[65], 0xff);
    }

    #[test]
    fn top_and_bottom_and_off_table() {
        let base = table_with([b"00050000", b"00040000", b"00030000", b"00020000", b"00010000"]);
        let mut t = base;
        assert_eq!(updatehst(&mut t, &[0, 0, 1, 0, 0, 0, 0, 0], *b"TOP"), 0);
        assert_eq!(&t[0..13], b"00100000 TOP\n");
        assert_eq!(&t[52..65], b"00020000 AB0\n");
        let mut t = base;
        assert_eq!(updatehst(&mut t, &[0, 0, 0, 1, 5, 0, 0, 0], *b"LOW"), 4);
        assert_eq!(&t[52..65], b"00015000 LOW\n");
        assert_eq!(&t[39..52], b"00020000 AB0\n");
        let mut t = base;
        assert_eq!(updatehst(&mut t, &[0, 0, 0, 0, 9, 9, 9, 9], *b"NOP"), 5);
        assert_eq!(t, base);
        // ties rank above (comparescore returns 1 on equality)
        let mut t = base;
        assert_eq!(updatehst(&mut t, &[0, 0, 0, 4, 0, 0, 0, 0], *b"TIE"), 1);
        // the default table accepts even a zero score at the top, like the original
        let mut d = default_highscores();
        assert_eq!(updatehst(&mut d, &[0; 8], *b"PSY"), 0);
    }

    #[test]
    fn entry_text_maps_archimedes_bytes() {
        let mut t = default_highscores();
        t[9..12].copy_from_slice(b"=[^");
        assert_eq!(entry_text(&t, 0), "00000000 ←.'");
        assert_eq!(entry_text(&t, 4), "00000000 PSY");
    }

    #[test]
    fn entry_text_short_line_shifts_running_pointer() {
        // showhst walks one pointer: line 0 stops at byte 4 and skips only it, so line 1
        // starts at byte 5 and stops at the real newline (byte 12); line 2 is back at 13.
        let mut t = default_highscores();
        t[4] = b'\n';
        assert_eq!(entry_text(&t, 0), "0000");
        assert_eq!(entry_text(&t, 1), "000 PSY");
        assert_eq!(entry_text(&t, 2), "00000000 PSY");
        assert_eq!(entry_text(&t, 4), "00000000 PSY");
    }

    #[test]
    fn entry_text_shift_carries_over_missing_newlines() {
        // Without the newlines at 12 and 25 the shift from line 0 persists into lines 1 and 2
        // (12-byte lines, each followed by one skipped byte) until line 3 hits the newline
        // at 38. Line 4 then shows stored entry 3 (bytes 39..51); entry 4 is never shown.
        let mut t = default_highscores();
        t[4] = 0x0a;
        t[12] = b'X';
        t[25] = b'Y';
        t[48..51].copy_from_slice(b"ABC");
        t[61..64].copy_from_slice(b"DEF");
        let lines: Vec<String> = (0..5).map(|i| entry_text(&t, i)).collect();
        assert_eq!(lines, ["0000", "000 PSYX0000", "000 PSYY0000", "000 PSY", "00000000 ABC"]);
        // every byte <= 0x0a: each line is empty and advances the pointer by one byte
        let mut z = [0u8; HIGHSCORE_LEN];
        z[65] = 0xff;
        z[4] = b'A';
        z[5] = b'B';
        assert_eq!(entry_text(&z, 3), "");
        assert_eq!(entry_text(&z, 4), "AB");
    }

    #[test]
    fn initials_editing() {
        let t = default_highscores();
        let mut hs = HsState::new(t, &[0, 0, 0, 0, 1, 0, 0, 0], *b"PSY", 1, false);
        assert_eq!(hs.hstindex, 0);
        assert_eq!(hs.phase, HsPhase::Entry { letter: 0 });
        let mut ini = *b"PSY";
        // initial 200 ms wait
        for _ in 0..9 {
            hs.tick(HELD_RIGHT, &mut ini);
        }
        assert_eq!(hs.table[9], b'P');
        // holding right: +1 every 2 ticks
        for _ in 0..8 {
            hs.tick(HELD_RIGHT, &mut ini);
        }
        assert_eq!(hs.table[9], b'P' + 4);
        // holding left: -1 every 2 ticks, clamped at '0'
        for _ in 0..200 {
            hs.tick(HELD_LEFT, &mut ini);
        }
        assert_eq!(hs.table[9], b'0');
        // both / none: unchanged
        for _ in 0..10 {
            hs.tick(HELD_LEFT | HELD_RIGHT, &mut ini);
            hs.tick(0, &mut ini);
        }
        assert_eq!(hs.table[9], b'0');
        // fire confirms the letter, then 200 ms before the next one
        hs.fire = true;
        while hs.phase == (HsPhase::Entry { letter: 0 }) {
            hs.tick(0, &mut ini);
        }
        assert_eq!(ini, *b"0SY");
        for _ in 0..300 {
            hs.tick(HELD_RIGHT, &mut ini);
        }
        assert_eq!(hs.table[10], b'Z' + 4);
        hs.fire = true;
        let mut finished = false;
        for _ in 0..12 {
            finished |= hs.tick(0, &mut ini);
        }
        assert_eq!(ini, [b'0', b'Z' + 4, b'Y']);
        hs.fire = true;
        for _ in 0..40 {
            finished |= hs.tick(0, &mut ini);
        }
        assert!(finished);
        assert_eq!(hs.phase, HsPhase::Done);
        assert_eq!(&hs.table[0..13], b"00001000 0^Y\n");
        assert_eq!(entry_text(&hs.table, 0), "00001000 0'Y");
    }

    #[test]
    fn not_on_table_goes_straight_to_done() {
        let t = table_with([b"99999999", b"99999999", b"99999999", b"99999999", b"99999999"]);
        let hs = HsState::new(t, &[0, 0, 0, 0, 1, 0, 0, 0], *b"PSY", 2, false);
        assert_eq!(hs.hstindex, 5);
        assert_eq!(hs.phase, HsPhase::Done);
    }
}
