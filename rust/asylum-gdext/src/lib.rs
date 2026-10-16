//! Godot 4 host for Asylum (thin layer over `asylum_core`, see docs/ARCHITECTURE.md §2).
//!
//! * [`game::AsylumGame`] — the main scene root (Node2D): start-up loading, per-frame frame
//!   upload (one MultiMesh draw), input, window/video settings, quitting.
//! * [`audio`] — buses, 8 SFX players with panners, streamed music.
//! * [`gfx`] — atlas texture, sprite shader material, MultiMesh on a child canvas item.
//!
//! Debug environment variables (optional, for automated checks):
//! * `ASYLUM_USERDIR`  — use this user-data directory instead of the portable/user:// one.
//! * `ASYLUM_LOCALE`   — pretend the OS locale is this (e.g. `ko_KR`) when the config has
//!   no language yet.
//! * `ASYLUM_KEYS`     — comma separated `time:keycode` list of key presses to inject.
//! * `ASYLUM_SCREENSHOT=<png path>` (+ `ASYLUM_SCREENSHOT_FRAME`, default 180) — save the
//!   viewport at that time, then quit.
//! * `ASYLUM_BOT=<seed>` — a random player: random held keys and occasional key presses, for
//!   long unattended runs.
//!
//! Times count 1/60 s from the end of loading, independent of the display refresh rate.

// MSVC prints localized "Creating library ..." lines that rustc reports as warnings.
#![allow(linker_messages)]

mod audio;
mod game;
mod gfx;
mod video;

use godot::prelude::*;

struct AsylumExtension;

#[gdextension]
unsafe impl ExtensionLibrary for AsylumExtension {}
