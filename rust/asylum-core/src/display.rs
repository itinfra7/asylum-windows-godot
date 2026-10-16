//! Display modes, window aspect ratios, resolutions and the view scale
//! (docs/ARCHITECTURE.md §4).
//!
//! * Display mode: window, borderless window (covers the monitor), exclusive full screen.
//! * Window aspect 4:3 .. 21:9 and window resolution: every standard resolution of that
//!   aspect that is not wider or taller than the monitor (aspects without one are skipped).
//!   In the borderless window and full screen the resolution is the monitor's own.
//! * View scale 1.0x .. 3.0x (0.5 steps): the game draws `256 x scale` lines across that
//!   resolution, so a higher scale shows more of the game world (with smaller pixels). Only
//!   scales whose line count does not exceed the resolution's height are offered.
//!
//! The host turns the result into the logical viewport (`content_scale_size`, stretch
//! "expand"): the picture always fills the window or screen exactly. The core (menu) and the
//! host use these functions with the same monitor size, so the menu shows what is applied.

/// Display modes of `Config::display_mode`.
pub const MODE_WINDOW: u8 = 0;
/// A window without decorations covering the whole monitor.
pub const MODE_BORDERLESS: u8 = 1;
/// Exclusive full screen.
pub const MODE_FULLSCREEN: u8 = 2;
pub(crate) const MODE_NAMES: [&str; 3] = ["window", "borderless", "fullscreen"];

/// Window aspect ratios of `Config::window_aspect`.
pub const ASPECTS: [(i32, i32); 4] = [(4, 3), (16, 10), (16, 9), (21, 9)];
/// Default window aspect (16:9).
pub const ASPECT_DEFAULT: u8 = 2;
/// Default window resolution (if the monitor fits it).
pub const RESOLUTION_DEFAULT: (i32, i32) = (1280, 720);

/// Standard resolutions of every window aspect, ascending.
pub const RESOLUTIONS: [&[(i32, i32)]; 4] = [
    // 4:3
    &[
        (640, 480),
        (800, 600),
        (960, 720),
        (1024, 768),
        (1152, 864),
        (1280, 960),
        (1400, 1050),
        (1440, 1080),
        (1600, 1200),
        (1792, 1344),
        (1856, 1392),
        (1920, 1440),
        (2048, 1536),
        (2560, 1920),
        (2880, 2160),
        (3200, 2400),
        (4096, 3072),
    ],
    // 16:10
    &[
        (640, 400),
        (960, 600),
        (1152, 720),
        (1280, 800),
        (1440, 900),
        (1680, 1050),
        (1920, 1200),
        (2304, 1440),
        (2560, 1600),
        (2880, 1800),
        (3072, 1920),
        (3840, 2400),
        (5120, 3200),
    ],
    // 16:9
    &[
        (640, 360),
        (854, 480),
        (960, 540),
        (1024, 576),
        (1280, 720),
        (1366, 768),
        (1600, 900),
        (1920, 1080),
        (2048, 1152),
        (2560, 1440),
        (3200, 1800),
        (3840, 2160),
        (5120, 2880),
        (7680, 4320),
    ],
    // 21:9 (the 64:27, 43:18 and 12:5 panels sold as 21:9)
    &[(1280, 540), (1720, 720), (1920, 800), (2560, 1080), (3440, 1440), (3840, 1600), (5120, 2160)],
];

/// Smallest / largest `Config::screen_scale` in half steps (1.0x / 3.0x).
pub const SCALE_MIN: u8 = 2;
pub const SCALE_MAX: u8 = 6;

/// Original screen: 320x256 is the narrowest view shape (5:4).
const BASE_W: i32 = crate::layout::BASE_W;
const BASE_H: i32 = crate::layout::BASE_H;

/// The next display mode of the menu cycle window → borderless window → full screen.
pub fn next_mode(mode: u8) -> u8 {
    if mode < MODE_FULLSCREEN {
        mode + 1
    } else {
        MODE_WINDOW
    }
}

fn aspect_index(i: u8) -> usize {
    (i as usize).min(ASPECTS.len() - 1)
}

/// The aspect a window uses: the chosen one if the monitor can show one of its resolutions,
/// else the next one in cycle order that it can (None on a monitor smaller than every
/// resolution; the window then takes the monitor's size).
pub fn effective_aspect(i: u8, monitor: (i32, i32)) -> Option<u8> {
    let n = ASPECTS.len();
    let i = aspect_index(i);
    (0..n).map(|k| ((i + k) % n) as u8).find(|&a| !window_resolutions(a, monitor).is_empty())
}

/// The next window aspect of the menu cycle 4:3 → 16:10 → 16:9 → 21:9, skipping aspects the
/// monitor cannot show.
pub fn next_aspect(i: u8, monitor: (i32, i32)) -> u8 {
    let n = ASPECTS.len();
    let cur = effective_aspect(i, monitor).map_or(aspect_index(i), |a| a as usize);
    (1..=n).map(|k| ((cur + k) % n) as u8).find(|&a| !window_resolutions(a, monitor).is_empty()).unwrap_or(cur as u8)
}

/// `"4:3"`, `"16:10"`, … (menu and config file).
pub fn aspect_text(i: u8) -> String {
    let (a, b) = ASPECTS[aspect_index(i)];
    format!("{a}:{b}")
}

/// `"16:9"` → index in [`ASPECTS`].
pub fn parse_aspect(v: &str) -> Option<u8> {
    let (a, b) = v.split_once(':')?;
    let r = (a.trim().parse::<i32>().ok()?, b.trim().parse::<i32>().ok()?);
    ASPECTS.iter().position(|x| *x == r).map(|i| i as u8)
}

/// `"1920x1080"` → (1920, 1080).
pub fn parse_resolution(v: &str) -> Option<(i32, i32)> {
    let (w, h) = v.split_once(['x', 'X'])?;
    let r = (w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?);
    (r.0 > 0 && r.1 > 0 && r.0 <= 16384 && r.1 <= 16384).then_some(r)
}

/// `"1920 x 1080"` (menu).
pub fn resolution_text((w, h): (i32, i32)) -> String {
    format!("{w} x {h}")
}

fn fits(r: (i32, i32), monitor: (i32, i32)) -> bool {
    r.0 <= monitor.0 && r.1 <= monitor.1
}

/// The window resolutions offered for an aspect on a monitor: every one not wider or taller
/// than the monitor (possibly none).
pub fn window_resolutions(aspect: u8, monitor: (i32, i32)) -> Vec<(i32, i32)> {
    RESOLUTIONS[aspect_index(aspect)].iter().copied().filter(|r| fits(*r, monitor)).collect()
}

/// The window resolution used for a wanted one with the chosen aspect: itself when offered,
/// else the tallest offered one not taller than it (else the smallest offered); the
/// monitor's size when no aspect has a resolution that fits.
pub fn pick_resolution(aspect: u8, want: (i32, i32), monitor: (i32, i32)) -> (i32, i32) {
    let Some(a) = effective_aspect(aspect, monitor) else {
        return (monitor.0.max(1), monitor.1.max(1));
    };
    let list = window_resolutions(a, monitor);
    if list.contains(&want) {
        return want;
    }
    list.iter().rev().find(|r| r.1 <= want.1).copied().unwrap_or(list[0])
}

/// The next window resolution of the menu cycle (ascending, then the smallest again).
pub fn next_resolution(aspect: u8, cur: (i32, i32), monitor: (i32, i32)) -> (i32, i32) {
    let cur = pick_resolution(aspect, cur, monitor);
    let Some(a) = effective_aspect(aspect, monitor) else { return cur };
    let list = window_resolutions(a, monitor);
    let i = list.iter().position(|r| *r == cur).unwrap_or(0);
    list[(i + 1) % list.len()]
}

/// Largest scale whose line count fits a resolution's height (1.0x always counts).
pub fn max_scale(resolution: (i32, i32)) -> u8 {
    (SCALE_MIN..=SCALE_MAX).rev().find(|&s| 128 * s as i32 <= resolution.1).unwrap_or(SCALE_MIN)
}

/// The scale actually used: the chosen one, limited to `max`.
pub fn effective_scale(halves: u8, max: u8) -> u8 {
    halves.clamp(SCALE_MIN, max.clamp(SCALE_MIN, SCALE_MAX))
}

/// The next scale of the menu cycle 1.0x, 1.5x, … up to `max`, then 1.0x again.
pub fn next_scale(halves: u8, max: u8) -> u8 {
    let max = max.clamp(SCALE_MIN, SCALE_MAX);
    let cur = effective_scale(halves, max);
    if cur < max {
        cur + 1
    } else {
        SCALE_MIN
    }
}

/// `"1.0"`, `"1.5"`, … for a scale in half steps (config file and menu).
pub fn scale_text(halves: u8) -> String {
    format!("{}.{}", halves / 2, if halves % 2 == 1 { 5 } else { 0 })
}

/// `"1"`, `"1.5"`, `"2.0"`, `"2x"` … → half steps; anything that is not a valid scale → None.
pub fn parse_scale(v: &str) -> Option<u8> {
    let f: f64 = v.trim_end_matches(['x', 'X']).parse().ok()?;
    let halves = f * 2.0;
    let h = halves.round();
    let valid = (halves - h).abs() < 1e-6 && (SCALE_MIN as f64..=SCALE_MAX as f64).contains(&h);
    valid.then_some(h as u8)
}

/// Logical view size for a scale on an output resolution: `128 * halves` lines with the
/// output's shape (never narrower than the original 320 per 256 lines).
pub fn view_size(halves: u8, resolution: (i32, i32)) -> (i32, i32) {
    let h = 128 * halves.clamp(SCALE_MIN, SCALE_MAX) as i32;
    let w = if resolution.1 > 0 { (h as f64 * resolution.0 as f64 / resolution.1 as f64).round() as i32 } else { 0 };
    (w.max(h * BASE_W / BASE_H), h)
}

/// What a config gives on a monitor (physical pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Video {
    /// Window aspect used (None: the full-screen modes, or a monitor too small for any).
    pub aspect: Option<u8>,
    /// Output resolution: the window's client size, or the monitor in the full modes.
    pub resolution: (i32, i32),
    /// Scale used (half steps).
    pub scale: u8,
    /// Logical view size (`content_scale_size`).
    pub view: (i32, i32),
}

pub fn video(cfg: &crate::config::Config, monitor: (i32, i32)) -> Video {
    let (aspect, resolution) = if cfg.display_mode == MODE_WINDOW || monitor.0 <= 0 || monitor.1 <= 0 {
        (effective_aspect(cfg.window_aspect, monitor), pick_resolution(cfg.window_aspect, cfg.window_resolution, monitor))
    } else {
        (None, monitor)
    };
    let scale = effective_scale(cfg.screen_scale, max_scale(resolution));
    Video { aspect, resolution, scale, view: view_size(scale, resolution) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UW: (i32, i32) = (2560, 1080);

    #[test]
    fn lists_are_true_to_their_aspect() {
        for (i, list) in RESOLUTIONS.iter().enumerate() {
            let (a, b) = ASPECTS[i];
            let target = a as f64 / b as f64;
            for w in list.windows(2) {
                assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1, "ascending {w:?}");
            }
            for &(w, h) in list.iter() {
                let r = w as f64 / h as f64;
                // 21:9 covers the 2.33..2.4 panels; the others are exact (854x480 rounds)
                let tol = if i == 3 { 0.03 } else { 0.004 };
                assert!((r / target - 1.0).abs() < tol, "{w}x{h} for {a}:{b}");
            }
        }
    }

    #[test]
    fn every_resolution_up_to_the_monitor() {
        let w = |a| window_resolutions(a, UW);
        assert_eq!(*w(0).last().unwrap(), (1440, 1080));
        assert_eq!(w(0).len(), 8);
        assert_eq!(*w(1).last().unwrap(), (1680, 1050));
        assert_eq!(w(2), [(640, 360), (854, 480), (960, 540), (1024, 576), (1280, 720), (1366, 768), (1600, 900), (1920, 1080)]);
        assert_eq!(w(3), [(1280, 540), (1720, 720), (1920, 800), (2560, 1080)]);
        // nothing wider or taller than the monitor; tiny monitors keep the smallest
        assert_eq!(window_resolutions(3, (1920, 1080)), [(1280, 540), (1720, 720), (1920, 800)]);
        assert_eq!(window_resolutions(2, (800, 600)), [(640, 360)]);
        // large monitors get the high end of the lists
        assert!(window_resolutions(2, (2560, 1440)).ends_with(&[(1920, 1080), (2048, 1152), (2560, 1440)]));
        assert!(window_resolutions(1, (3840, 2400)).contains(&(3072, 1920)));
        assert_eq!(*window_resolutions(2, (7680, 4320)).last().unwrap(), (7680, 4320));
    }

    #[test]
    fn aspects_without_a_fitting_resolution_are_skipped() {
        // 21:9 starts at 1280x540: a 1024x768 or portrait monitor cannot show it
        for m in [(1024, 768), (1080, 1920)] {
            assert!(window_resolutions(3, m).is_empty());
            assert_eq!(effective_aspect(3, m), Some(0), "a saved 21:9 falls back to the next aspect");
            assert_eq!(next_aspect(2, m), 0, "the cycle skips 21:9");
            let r = pick_resolution(3, (2560, 1080), m);
            assert!(r.0 <= m.0 && r.1 <= m.1, "{r:?} on {m:?}");
        }
        assert_eq!(next_aspect(3, (2560, 1080)), 0);
        assert_eq!(next_aspect(0, (2560, 1080)), 1);
        // a monitor smaller than every resolution: the window takes the monitor's size
        assert_eq!(effective_aspect(2, (600, 340)), None);
        assert_eq!(pick_resolution(2, (1280, 720), (600, 340)), (600, 340));
        assert_eq!(next_resolution(2, (1280, 720), (600, 340)), (600, 340));
        assert_eq!(next_aspect(2, (600, 340)), 2);
    }

    #[test]
    fn switching_aspects_keeps_the_wanted_size() {
        // the wanted 1920x1080 is resolved per aspect; nothing is lost on a full cycle
        let m = (1920, 1080);
        let res: Vec<(i32, i32)> = (0..4).map(|a| pick_resolution(a, (1920, 1080), m)).collect();
        assert_eq!(res, [(1440, 1080), (1680, 1050), (1920, 1080), (1920, 800)]);
    }

    #[test]
    fn resolution_choice_and_cycle() {
        assert_eq!(pick_resolution(2, (1920, 1080), UW), (1920, 1080));
        // not offered (other aspect or too big): the tallest offered one not taller
        assert_eq!(pick_resolution(3, (1920, 1080), UW), (2560, 1080));
        assert_eq!(pick_resolution(0, (1366, 768), UW), (1024, 768));
        assert_eq!(pick_resolution(2, (3840, 2160), UW), (1920, 1080));
        assert_eq!(pick_resolution(3, (640, 360), UW), (1280, 540));
        let mut r = (640, 360);
        let mut seen = vec![r];
        for _ in 0..8 {
            r = next_resolution(2, r, UW);
            seen.push(r);
        }
        assert_eq!(seen.first(), seen.last(), "wraps after the monitor's size");
        assert_eq!(seen[7], (1920, 1080));
    }

    #[test]
    fn scale_limited_by_the_resolution_height() {
        assert_eq!(max_scale((1920, 1080)), 6);
        assert_eq!(max_scale((1280, 720)), 5);
        assert_eq!(max_scale((640, 480)), 3);
        assert_eq!(max_scale((640, 360)), 2);
        assert_eq!((effective_scale(6, 4), next_scale(6, 4), next_scale(3, 4), next_scale(4, 4)), (4, 2, 4, 2));
        assert_eq!((effective_scale(0, 6), next_scale(200, 6)), (2, 2));
    }

    #[test]
    fn view_has_the_output_shape() {
        assert_eq!(view_size(2, UW), (607, 256));
        assert_eq!(view_size(6, UW), (1820, 768));
        assert_eq!(view_size(2, (1920, 1080)), (455, 256));
        assert_eq!(view_size(4, (1024, 768)), (683, 512));
        // portrait monitors: at least 5:4 (the host's "expand" stretch grows the height)
        assert_eq!(view_size(2, (1080, 1920)), (320, 256));
    }

    #[test]
    fn config_to_video() {
        let mut c = crate::config::Config {
            display_mode: MODE_WINDOW,
            window_aspect: 0,
            window_resolution: (1440, 1080),
            screen_scale: 6,
            ..Default::default()
        };
        assert_eq!(video(&c, UW), Video { aspect: Some(0), resolution: (1440, 1080), scale: 6, view: (1024, 768) });
        // a smaller monitor: the tallest 4:3 that fits, the scale limited by its height
        assert_eq!(video(&c, (1280, 720)), Video { aspect: Some(0), resolution: (960, 720), scale: 5, view: (853, 640) });
        c.display_mode = MODE_BORDERLESS;
        assert_eq!(video(&c, UW), Video { aspect: None, resolution: UW, scale: 6, view: (1820, 768) });
        c.display_mode = MODE_FULLSCREEN;
        c.screen_scale = 2;
        assert_eq!(video(&c, (1920, 1080)), Video { aspect: None, resolution: (1920, 1080), scale: 2, view: (455, 256) });
    }

    #[test]
    fn texts() {
        assert_eq!((next_mode(0), next_mode(1), next_mode(2), next_mode(9)), (1, 2, 0, 0));
        let a: Vec<String> = (0..4).map(aspect_text).collect();
        assert_eq!(a, ["4:3", "16:10", "16:9", "21:9"]);
        let big = (16384, 16384);
        assert_eq!((next_aspect(0, big), next_aspect(3, big), next_aspect(200, big)), (1, 0, 0));
        for (i, t) in a.iter().enumerate() {
            assert_eq!(parse_aspect(t), Some(i as u8));
        }
        assert_eq!((parse_aspect("16 : 9"), parse_aspect("5:4"), parse_aspect("16/9"), parse_aspect("")), (Some(2), None, None, None));
        assert_eq!(parse_resolution("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_resolution(" 1366 X 768 "), Some((1366, 768)));
        for bad in ["", "1920", "x", "0x0", "-1x5", "99999x5", "axb"] {
            assert_eq!(parse_resolution(bad), None, "{bad}");
        }
        assert_eq!(resolution_text((1920, 1080)), "1920 x 1080");
        let s: Vec<String> = (SCALE_MIN..=SCALE_MAX).map(scale_text).collect();
        assert_eq!(s, ["1.0", "1.5", "2.0", "2.5", "3.0"]);
        for (text, halves) in [("1", 2), ("1.0", 2), ("1.5", 3), ("2", 4), ("2.50", 5), ("3.0", 6), ("2x", 4), ("1.5X", 3)] {
            assert_eq!(parse_scale(text), Some(halves), "{text}");
        }
        for bad in ["1.25", "0.5", "3.5", "NaN", "inf", "", "x", "-2"] {
            assert_eq!(parse_scale(bad), None, "{bad}");
        }
    }
}
