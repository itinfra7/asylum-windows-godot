//! Full-screen RLE images GameScreen / ChatScreen (vdu.c `decomp`).
//!
//! Skip 68 header bytes; then repeatedly: byte r0; if r0 & 0x80: next byte is repeated
//! (r0 & 0x7f) + 2 times; else (r0 & 0x7f) + 1 literal bytes follow. Output: 320x256
//! palette indices.

pub const SCREEN_W: usize = 320;
pub const SCREEN_H: usize = 256;

#[derive(Clone)]
pub struct Screen {
    /// 320*256 palette indices, row-major.
    pub pix: Vec<u8>,
}

pub fn parse_screen(data: &[u8]) -> Result<Screen, String> {
    if data.len() < 68 {
        return Err("screen file too short".into());
    }
    let total = SCREEN_W * SCREEN_H;
    let mut pix = Vec::with_capacity(total);
    let mut p = 68usize;
    while pix.len() < total && p < data.len() {
        let r0 = data[p];
        p += 1;
        if r0 & 0x80 != 0 {
            let s = *data.get(p).unwrap_or(&0);
            p += 1;
            let n = (r0 & 0x7f) as usize + 2;
            for _ in 0..n {
                if pix.len() < total {
                    pix.push(s);
                }
            }
        } else {
            let n = (r0 & 0x7f) as usize + 1;
            for _ in 0..n {
                if pix.len() < total {
                    pix.push(*data.get(p).unwrap_or(&0));
                }
                p += 1;
            }
        }
    }
    pix.resize(total, 0);
    Ok(Screen { pix })
}
