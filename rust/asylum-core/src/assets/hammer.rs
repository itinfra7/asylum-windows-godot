//! "Hmr" decompression (file.c `swi_blitz_hammerop`).
//!
//! Format: "Hmr", fmt byte, u32 little-endian unpacked length, then a byte-code stream
//! (0x00..0x0f run, 0x10..0x1f literal, 0x20..0x3f backward copy, 0x40..0x7f strided copy,
//! 0x80..0xfe forward copy, 0xff end). If fmt != 0 the output is de-interleaved in 4 planes.
//! Files without the "Hmr" magic are returned unchanged (loadhammered falls back to loadfile).

/// Returns true if `data` starts with the "Hmr" magic.
pub fn is_hammered(data: &[u8]) -> bool {
    data.len() >= 8 && &data[0..3] == b"Hmr"
}

/// Decompress (or pass through) a possibly hammered file.
pub fn unhammer(data: &[u8]) -> Result<Vec<u8>, String> {
    if !is_hammered(data) {
        return Ok(data.to_vec());
    }
    let fmt = data[3];
    let len = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let mut a: Vec<u8> = Vec::with_capacity(len.max(16));
    let mut pos = 8usize;
    let next = |pos: &mut usize| -> Option<u8> {
        let b = data.get(*pos).copied();
        *pos += 1;
        b
    };
    while let Some(c) = next(&mut pos) {
        if c == 0xff {
            break;
        } else if c < 0x10 {
            let n = if c == 15 { 256 } else { c as usize + 2 };
            let d = next(&mut pos).ok_or("truncated run")?;
            a.extend(std::iter::repeat_n(d, n));
        } else if c < 0x20 {
            let n = (c & 0xf) as usize + 1;
            for _ in 0..n {
                a.push(next(&mut pos).ok_or("truncated literal")?);
            }
        } else if c < 0x40 {
            let d = next(&mut pos).ok_or("truncated copy")? as isize;
            let mut p = a.len() as isize - 1 - (((c & 3) as isize) << 8) - d;
            if p < 0 {
                return Err("bad back reference (type 3)".into());
            }
            let n = ((c & 0x1c) >> 2) as usize + 2;
            for _ in 0..n {
                let v = a[p.max(0) as usize];
                a.push(v);
                p -= 1;
            }
        } else if c < 0x80 {
            let d = next(&mut pos).ok_or("truncated copy")? as isize;
            let sh = if c & 4 != 0 { (c & 6) + 2 } else { c & 6 };
            let step = 1isize << (sh >> 1);
            let p = a.len() as isize - 1 - (((c & 1) as isize) << 8) - d;
            if p < 0 {
                return Err("bad back reference (type 4)".into());
            }
            let n = ((c & 0x38) >> 3) as isize + 2;
            for i in 0..n {
                let idx = (p + (i * step) / 4) as usize;
                let v = *a.get(idx).ok_or("strided copy out of range")?;
                a.push(v);
            }
        } else {
            let d = next(&mut pos).ok_or("truncated copy")? as isize;
            let p = a.len() as isize - 1 - (((c & 7) as isize) << 8) - d;
            if p < 0 {
                return Err("bad back reference (type 5)".into());
            }
            let n = ((c & 0x78) >> 3) as usize + 2;
            // forward copy; the source may overlap the bytes being appended (LZ run)
            for k in 0..n {
                let v = a[p as usize + k];
                a.push(v);
            }
        }
    }
    if fmt != 0 {
        let wlen = (len + 3) >> 2;
        let p = a.len();
        let mut out = vec![0u8; p];
        for (i, o) in out.iter_mut().enumerate() {
            *o = *a.get((i % 4) * wlen + (i >> 2)).unwrap_or(&0);
        }
        Ok(out)
    } else {
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passthrough() {
        assert_eq!(unhammer(b"FSP1abcd").unwrap(), b"FSP1abcd".to_vec());
    }
    #[test]
    fn run_and_literal() {
        // run of 3 x 'A' (c=1 -> n=3), literal 2 bytes "BC" (c=0x11), end
        let data = [b'H', b'm', b'r', 0, 5, 0, 0, 0, 1, b'A', 0x11, b'B', b'C', 0xff];
        assert_eq!(unhammer(&data).unwrap(), b"AAABC".to_vec());
    }
}
