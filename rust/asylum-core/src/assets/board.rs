//! Maze boards (C `struct board { int first_int, width, height, ...; char contents[65536]; }`).
//!
//! Files are hammered; after unhammering: 32-byte header (8 little-endian i32, width at
//! byte 4, height at byte 8) followed by width*height cell bytes.
//!
//! The C code freely forms pointers outside `contents` (e.g. `r11 - boardwidth` on row 0).
//! `Board::get/set` reproduce that safely: offsets in [-32, 0) address the header bytes,
//! offsets in [0, len) the cells, everything else reads 0 and ignores writes.

/// Cell offset relative to the start of `contents` (C `char*` into the board).
pub type BIdx = i32;

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub width: i32,
    pub height: i32,
    pub header: [u8; 32],
    pub cells: Vec<u8>,
}

impl Board {
    pub fn len(&self) -> i32 {
        self.cells.len() as i32
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    #[inline]
    pub fn get(&self, i: BIdx) -> u8 {
        if i >= 0 {
            *self.cells.get(i as usize).unwrap_or(&0)
        } else if i >= -32 {
            self.header[(32 + i) as usize]
        } else {
            0
        }
    }

    #[inline]
    pub fn set(&mut self, i: BIdx, v: u8) {
        if i >= 0 {
            if let Some(c) = self.cells.get_mut(i as usize) {
                *c = v;
            }
        } else if i >= -32 {
            self.header[(32 + i) as usize] = v;
        }
    }
}

/// Parse an (already unhammered) board.
pub fn parse_board(data: &[u8]) -> Result<Board, String> {
    if data.len() < 32 {
        return Err("board too short".into());
    }
    let w = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let h = i32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
        return Err(format!("bad board size {w}x{h}"));
    }
    let n = (w * h) as usize;
    let mut cells = data[32..].to_vec();
    cells.resize(n, 0);
    let mut header = [0u8; 32];
    header.copy_from_slice(&data[0..32]);
    Ok(Board { width: w, height: h, header, cells })
}
