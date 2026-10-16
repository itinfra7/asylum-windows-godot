//! Window / video settings (HostRequest::ApplyVideo): display mode, window aspect and
//! resolution, view scale and vsync (see `asylum_core::display`).
//!
//! The logical viewport (`content_scale_size`) is the view size of the scale on the output
//! resolution, with stretch aspect "expand": it always has the window's own shape, so the
//! picture fills the window or screen exactly (no black bars) and the core lays the screen
//! out for that size rather than stretching a fixed picture.

use asylum_core::config::Config;
use asylum_core::display;
use godot::classes::display_server::VSyncMode;
use godot::classes::window::{ContentScaleAspect, ContentScaleMode, ContentScaleStretch, Mode};
use godot::classes::{DisplayServer, RenderingServer, Window};
use godot::global::MouseButtonMask;
use godot::obj::EngineBitfield;
use godot::prelude::*;

/// Window decorations assumed while the real ones are unknown (Windows 10/11: invisible
/// resize borders + title bar).
const FALLBACK_DECORATIONS: (i32, i32) = (16, 39);

/// Smallest window client area (the original screen).
const MIN_CLIENT: (i32, i32) = (320, 256);

/// Frames a window must stay in place on another monitor before the settings are re-applied
/// there (so a drag across monitors is never interrupted).
const SETTLE_FRAMES: u32 = 15;

/// The monitor a window is on: index and size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub screen: i32,
    pub size: (i32, i32),
}

fn current_monitor(win: &Gd<Window>) -> Monitor {
    let screen = win.get_current_screen();
    let s = DisplayServer::singleton().screen_get_size_ex().screen(screen).done();
    Monitor { screen, size: (s.x, s.y) }
}

fn is_full(m: Mode) -> bool {
    m == Mode::FULLSCREEN || m == Mode::EXCLUSIVE_FULLSCREEN
}

/// Where the engine should open the window next time (written to `override.cfg`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartWindow {
    /// Godot window mode: 0 window, 3 full screen (borderless), 4 exclusive full screen.
    pub mode: i32,
    pub size: (i32, i32),
    pub position: Option<(i32, i32)>,
}

pub struct Video {
    /// The editor embeds the game window: never move or resize it.
    placeable: bool,
    monitor: Monitor,
    /// Last applied settings (re-applied on another monitor).
    cfg: Option<Config>,
    /// Client size last given to the window.
    placed: Option<(i32, i32)>,
    /// A monitor change waiting for the window to settle: (position, frames unchanged).
    settle: Option<(Vector2i, u32)>,
    /// Vsync last requested.
    vsync: Option<bool>,
}

impl Video {
    pub fn new(placeable: bool, win: &Gd<Window>) -> Video {
        Video { placeable, monitor: current_monitor(win), cfg: None, placed: None, settle: None, vsync: None }
    }

    /// Size in pixels of the monitor the window is on.
    pub fn monitor(&self) -> (i32, i32) {
        self.monitor.size
    }

    pub fn apply(&mut self, win: &mut Gd<Window>, cfg: &Config) {
        self.cfg = Some(cfg.clone());
        let v = display::video(cfg, self.monitor.size);
        win.set_content_scale_mode(ContentScaleMode::CANVAS_ITEMS);
        win.set_content_scale_aspect(ContentScaleAspect::EXPAND);
        win.set_content_scale_stretch(ContentScaleStretch::FRACTIONAL);
        win.set_content_scale_size(Vector2i::new(v.view.0, v.view.1));
        let mode = win.get_mode();
        match cfg.display_mode {
            display::MODE_WINDOW => {
                win.set_min_size(Vector2i::new(MIN_CLIENT.0, MIN_CLIENT.1));
                // a new size, or leaving full screen: the window gets the chosen resolution
                // (a window the player maximized stays so until the size changes)
                if self.placeable && (self.placed != Some(v.resolution) || is_full(mode)) {
                    if mode != Mode::WINDOWED {
                        win.set_mode(Mode::WINDOWED);
                    }
                    if place_window(win, v.resolution) {
                        self.placed = Some(v.resolution);
                    }
                } else if is_full(mode) {
                    win.set_mode(Mode::WINDOWED);
                }
            }
            m => {
                let target = if m == display::MODE_BORDERLESS { Mode::FULLSCREEN } else { Mode::EXCLUSIVE_FULLSCREEN };
                if mode != target {
                    win.set_mode(target);
                } else if self.placeable && win.get_size() != Vector2i::new(self.monitor.size.0, self.monitor.size.1) {
                    // the desktop resolution changed: Windows leaves the full-screen window at
                    // its old size and Godot keeps its full-screen flag, so enter it again
                    win.set_mode(Mode::WINDOWED);
                    win.set_mode(target);
                }
            }
        }
        if self.vsync != Some(cfg.vsync) {
            self.vsync = Some(cfg.vsync);
            set_vsync(cfg.vsync);
        }
    }

    /// Call every frame: when the window is on another monitor (or the monitor changed its
    /// resolution) and has stopped moving, re-applies the settings for it and returns its
    /// size for the core. A minimized window is left alone until it is restored.
    pub fn poll(&mut self, win: &mut Gd<Window>) -> Option<(i32, i32)> {
        if win.get_mode() == Mode::MINIMIZED {
            return None;
        }
        let m = current_monitor(win);
        if m == self.monitor || m.size.0 <= 0 || m.size.1 <= 0 {
            self.settle = None;
            return None;
        }
        // a window still being dragged is not settled, however long it pauses: the frames
        // keep running inside the move loop (GetKeyState stays current there)
        if DisplayServer::singleton().mouse_get_button_state().is_set(MouseButtonMask::LEFT) {
            self.settle = None;
            return None;
        }
        let pos = win.get_position();
        match self.settle {
            Some((p, n)) if p == pos && n + 1 >= SETTLE_FRAMES => {}
            Some((p, n)) if p == pos => {
                self.settle = Some((p, n + 1));
                return None;
            }
            _ => {
                self.settle = Some((pos, 0));
                return None;
            }
        }
        self.settle = None;
        self.monitor = m;
        if let Some(cfg) = self.cfg.clone() {
            self.apply(win, &cfg);
        }
        Some(m.size)
    }

    /// How the engine should open the window next time, from the last applied settings.
    pub fn start_window(&self, win: &Gd<Window>) -> Option<StartWindow> {
        let cfg = self.cfg.as_ref()?;
        let v = display::video(cfg, self.monitor.size);
        Some(match cfg.display_mode {
            display::MODE_WINDOW => {
                let p = win.get_position();
                StartWindow { mode: 0, size: v.resolution, position: self.placed.map(|_| (p.x, p.y)) }
            }
            display::MODE_BORDERLESS => StartWindow { mode: 3, size: v.resolution, position: None },
            _ => StartWindow { mode: 4, size: v.resolution, position: None },
        })
    }
}

/// Sets vsync on the render thread. The OpenGL context lives there (separate render thread
/// model), so a swap-interval change made from the main thread silently fails ("changing
/// V-Sync mode is not supported"); Godot's own project setting path uses the render thread
/// too. Only called on change: some GL drivers warn on redundant requests.
fn set_vsync(on: bool) {
    let mode = if on { VSyncMode::ENABLED } else { VSyncMode::DISABLED };
    let ds = DisplayServer::singleton();
    let call = Callable::from_object_method(&ds, "window_set_vsync_mode").bindv(&varray![mode, 0]);
    RenderingServer::singleton().call_on_render_thread(&call);
}

/// `override.cfg` text that makes the engine open the window like `w` (Godot reads it from
/// the executable's folder before it creates the window, so the game does not start in a
/// default-sized window and then jump).
pub fn start_window_cfg(w: &StartWindow) -> String {
    let mut s = String::from("; Written by Asylum: how to open the window next time (settings: userdata/asylum.cfg).\n[display]\n");
    s += &format!("window/size/mode={}\n", w.mode);
    s += &format!("window/size/window_width_override={}\nwindow/size/window_height_override={}\n", w.size.0, w.size.1);
    match w.position {
        Some((x, y)) => s += &format!("window/size/initial_position_type=0\nwindow/size/initial_position=Vector2i({x}, {y})\n"),
        None => s += "window/size/initial_position_type=1\n",
    }
    s
}

/// Gives the window the client size `res` and centres it on its monitor's usable area.
/// Returns false when there is no screen to place it on (headless).
fn place_window(win: &mut Gd<Window>, res: (i32, i32)) -> bool {
    let ds = DisplayServer::singleton();
    let usable = ds.screen_get_usable_rect_ex().screen(win.get_current_screen()).done();
    if usable.size.x <= 0 || usable.size.y <= 0 {
        return false;
    }
    let d = win.get_size_with_decorations() - win.get_size();
    let deco = if d.x > 0 && d.y > 0 { (d.x, d.y) } else { FALLBACK_DECORATIONS };
    win.set_size(Vector2i::new(res.0, res.1));
    let (x, y) = window_position((usable.position.x, usable.position.y), (usable.size.x, usable.size.y), res, deco);
    win.set_position(Vector2i::new(x, y));
    true
}

/// Client-area origin that centres the whole window (client + decorations) on the usable
/// area. Above the client sit the title bar and the top frame, below it only the bottom
/// frame, which is as thick as each side frame (half the horizontal decoration). A window
/// taller than the area keeps its bottom (the HUD) on the area's bottom edge, and a window
/// wider than it is centred on the client alone (the side frames of Windows 10/11 are
/// invisible).
pub fn window_position(usable_pos: (i32, i32), usable: (i32, i32), size: (i32, i32), deco: (i32, i32)) -> (i32, i32) {
    let top = (deco.1 - deco.0 / 2).clamp(0, deco.1.max(0));
    let x = usable_pos.0 + (usable.0 - size.0) / 2;
    let y = if size.1 + deco.1 > usable.1 {
        usable_pos.1 + usable.1 - size.1
    } else {
        usable_pos.1 + (usable.1 - size.1 - deco.1) / 2 + top
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECO: (i32, i32) = FALLBACK_DECORATIONS;

    /// Outer window rect (left, top, right, bottom) for a client origin.
    fn outer((x, y): (i32, i32), (w, h): (i32, i32), deco: (i32, i32)) -> (i32, i32, i32, i32) {
        let top = deco.1 - deco.0 / 2;
        (x - deco.0 / 2, y - top, x + w + deco.0 / 2, y + h + deco.1 - top)
    }

    #[test]
    fn window_is_centred_with_its_decorations() {
        // 1920x980 window on a 2560x1019 work area: equal margins (±1 px) on all sides
        let size = (1920, 980);
        let p = window_position((0, 0), (2560, 1019), size, DECO);
        let (l, t, r, b) = outer(p, size, DECO);
        assert!((l - (2560 - r)).abs() <= 1 && (t - (1019 - b)).abs() <= 1, "{:?}", (l, t, r, b));
        // work areas that do not start at 0 (task bar at the top or on the left)
        let p0 = window_position((0, 0), (1920, 1040), (1280, 720), DECO);
        let p1 = window_position((0, 40), (1920, 1040), (1280, 720), DECO);
        assert_eq!(p1.1 - p0.1, 40);
        let p = window_position((60, 0), (1860, 1080), (1024, 768), DECO);
        let (l, _, r, _) = outer(p, (1024, 768), DECO);
        assert!((l - 60 - (1920 - r)).abs() <= 1);
        // the fallback decorations put a 31 px title bar + frame above the client
        assert_eq!(window_position((0, 0), (1000, 1000), (0, 0), DECO).1 - (1000 - 39) / 2, 31);
    }

    #[test]
    fn oversized_windows_keep_the_hud_on_screen() {
        // the monitor's own resolution as a window above a task bar: the client's bottom (the
        // HUD) stays on the work area, the title bar goes above the screen
        for (size, area) in [((2560, 1080), (2560, 1019)), ((1680, 1050), (2560, 1032)), ((1920, 1080), (1920, 1040))] {
            let (x, y) = window_position((0, 0), area, size, DECO);
            assert_eq!(y + size.1, area.1, "{size:?} on {area:?}");
            assert_eq!(x, (area.0 - size.0) / 2);
        }
    }

    #[test]
    fn start_window_override_text() {
        let s = start_window_cfg(&StartWindow { mode: 0, size: (1280, 720), position: Some((640, 150)) });
        assert!(s.contains("[display]\nwindow/size/mode=0\nwindow/size/window_width_override=1280\nwindow/size/window_height_override=720\n"));
        assert!(s.contains("window/size/initial_position_type=0\nwindow/size/initial_position=Vector2i(640, 150)\n"));
        let s = start_window_cfg(&StartWindow { mode: 3, size: (2560, 1080), position: None });
        assert!(s.contains("window/size/mode=3\n") && s.contains("initial_position_type=1\n"));
    }
}
