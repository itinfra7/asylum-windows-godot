//! Maps the original 320x256 design onto the logical viewport (modern displays).
//!
//! The host sets the logical viewport to `display::video().view`: 256 x scale lines with the
//! output's shape (the window's client area, or the monitor in the full-screen modes; at
//! least 320 per 256 lines). Godot's "expand" stretch can make the reported size larger when
//! the player resizes or maximizes the window. The content fills the viewport, so a higher
//! view scale shows more of the game world with smaller pixels; the original 320x256 screen
//! is the 1.0x layout of a 5:4 output. See docs/ARCHITECTURE.md §4.

/// Narrowest content width (original screen).
pub const BASE_W: i32 = 320;
/// Lowest content height (original screen).
pub const BASE_H: i32 = 256;
/// Largest viewport side accepted (a degenerate window could report an infinite width).
pub const MAX_SIDE: f32 = 16384.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Logical viewport size reported by the host.
    pub view_w: f32,
    pub view_h: f32,
    /// Content origin inside the viewport (always 0: the content starts at the viewport's
    /// corner; a fraction of a pixel of a fractional viewport is left at the right/bottom).
    pub ox: f32,
    pub oy: f32,
    /// Content size: the whole viewport (cw >= 320, ch >= 256).
    pub cw: i32,
    pub ch: i32,
    /// Offset of the original 320-wide HUD band / chat text area inside the content.
    pub hx: i32,
    /// Vertical offset of the original 256-line band centred in the content: menus, chat
    /// screens and the pause screen keep their original layout around the centre.
    pub cy: i32,
    /// Vertical offset of the bottom-anchored parts (HUD panel rows 208..256, the bonus
    /// messages rising from it, the intro scroll starting at the bottom).
    pub hy: i32,
    /// Game window (content coordinates), original: 16, 8, 288, 192.
    pub gamex: i32,
    pub gamey: i32,
    pub gamew: i32,
    pub gameh: i32,
}

impl Layout {
    /// Compute the layout for a logical viewport of `view_w` x `view_h`.
    pub fn new(view_w: f32, view_h: f32) -> Layout {
        let side = |v: f32, base: i32| if v.is_finite() { v.clamp(base as f32, MAX_SIDE) } else { base as f32 };
        let vw = side(view_w, BASE_W);
        let vh = side(view_h, BASE_H);
        let cw = vw.floor() as i32;
        let ch = vh.floor() as i32;
        let ox = ((vw - cw as f32) * 0.5).floor();
        let oy = ((vh - ch as f32) * 0.5).floor();
        Layout {
            view_w: vw,
            view_h: vh,
            ox,
            oy,
            cw,
            ch,
            hx: (cw - BASE_W) / 2,
            cy: (ch - BASE_H) / 2,
            hy: ch - BASE_H,
            gamex: 16,
            gamey: 8,
            gamew: cw - 32,
            gameh: ch - 64,
        }
    }

    /// Original layout (320x256 viewport).
    pub fn original() -> Layout {
        Layout::new(BASE_W as f32, BASE_H as f32)
    }

    /// True when the content is larger than the original screen.
    pub fn is_extended(&self) -> bool {
        self.cw > BASE_W || self.ch > BASE_H
    }

    /// Centre of the game window (content coords), original (160, 104).
    pub fn gcx(&self) -> i32 {
        self.gamex + self.gamew / 2
    }
    pub fn gcy(&self) -> i32 {
        self.gamey + self.gameh / 2
    }

    /// Converts content x to viewport x.
    #[inline]
    pub fn vx(&self, x: f32) -> f32 {
        self.ox + x
    }
    #[inline]
    pub fn vy(&self, y: f32) -> f32 {
        self.oy + y
    }

    /// Game window rectangle in viewport coordinates: [x0, y0, x1, y1).
    pub fn game_rect(&self) -> [f32; 4] {
        [
            self.vx(self.gamex as f32),
            self.vy(self.gamey as f32),
            self.vx((self.gamex + self.gamew) as f32),
            self.vy((self.gamey + self.gameh) as f32),
        ]
    }

    /// Whole content rectangle in viewport coordinates.
    pub fn content_rect(&self) -> [f32; 4] {
        [self.ox, self.oy, self.ox + self.cw as f32, self.oy + self.ch as f32]
    }

    /// Chat-screen text window (vdu.c showchatscreen: clip 20,20 .. 299,235), viewport coords.
    pub fn chat_rect(&self) -> [f32; 4] {
        [
            self.vx(20.0),
            self.vy(20.0),
            self.vx((self.cw - 21) as f32),
            self.vy((self.ch - 21) as f32),
        ]
    }

    /// Half the visible game window in whole blocks, used to grow the alien plot range and
    /// the wake-up box with the window. Original: (18, 12) for `xblocks_disturb/yblocks_disturb`.
    pub fn disturb_blocks(&self) -> (i32, i32) {
        ((self.gamew + 15) / 16, (self.gameh + 15) / 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_is_identity() {
        let l = Layout::original();
        assert_eq!((l.cw, l.ch, l.hx, l.cy, l.hy), (320, 256, 0, 0, 0));
        assert_eq!((l.gamex, l.gamey, l.gamew, l.gameh, l.gcx(), l.gcy()), (16, 8, 288, 192, 160, 104));
        assert_eq!((l.ox, l.oy), (0.0, 0.0));
        assert!(!l.is_extended());
    }
    #[test]
    fn wide_monitor_at_1x() {
        // 2560x1080 at 1.0x: 607x256, the content is exactly the viewport
        let l = Layout::new(607.0, 256.0);
        assert_eq!((l.cw, l.ch, l.hx, l.cy, l.hy), (607, 256, 143, 0, 0));
        assert_eq!((l.gamew, l.gameh), (575, 192));
        assert_eq!(l.content_rect(), [0.0, 0.0, 607.0, 256.0]);
    }
    #[test]
    fn higher_scale_shows_more_world() {
        // 3.0x on 21:9: the game window grows in both directions, the HUD panel stays at
        // the bottom and the original band is centred
        let l = Layout::new(1820.0, 768.0);
        assert_eq!((l.cw, l.ch, l.hx, l.cy, l.hy), (1820, 768, 750, 256, 512));
        assert_eq!((l.gamex, l.gamey, l.gamew, l.gameh), (16, 8, 1788, 704));
        assert_eq!((l.gcx(), l.gcy()), (910, 360));
        // the HUD panel (original rows 208..256) ends at the bottom of the content
        assert_eq!(208 + l.hy + 48, l.ch);
        // the game window still ends 8 rows above the HUD panel, like the original
        assert_eq!(l.gamey + l.gameh + 8, 208 + l.hy);
        assert_eq!(l.chat_rect(), [20.0, 20.0, 1799.0, 747.0]);
        assert!(l.is_extended());
    }
    #[test]
    fn never_smaller_than_the_original() {
        let l = Layout::new(100.0, 50.0);
        assert_eq!((l.cw, l.ch, l.view_w, l.view_h), (320, 256, 320.0, 256.0));
    }
    #[test]
    fn degenerate_views_stay_bounded() {
        // a window dragged to a 0-px client height reports an infinite width
        for (w, h) in [(f32::INFINITY, 256.0), (f32::NAN, f32::NAN), (1e9, 1e9), (-5.0, f32::NEG_INFINITY)] {
            let l = Layout::new(w, h);
            assert!(l.cw >= 320 && l.cw <= 16384 && l.ch >= 256 && l.ch <= 16384, "{w}x{h}: {l:?}");
        }
    }
}
