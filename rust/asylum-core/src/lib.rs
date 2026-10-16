//! asylum-core: engine-independent core of Asylum for Windows (Godot port).
//!
//! Copyright (C) 2026 itinfra7. A translation of SDL Asylum, Copyright (C) 2006-2009 Hugh
//! Robinson, licensed under GPL-3.0-or-later. Asylum was written by Andy Southgate in 1994;
//! the original game data is in the public domain. See docs/ARCHITECTURE.md for the rules
//! every module follows.

pub mod assets;
pub mod atlas;
pub mod audio;
pub mod config;
pub mod display;
pub mod font;
pub mod frontend;
pub mod i18n;
pub mod input;
pub mod layout;
pub mod palette;
pub mod render;
pub mod rng;
pub mod runtime;
pub mod sim;
pub mod storage;

/// Fixed logic rate (Hz) — the original runs at 50 Hz.
pub const TICK_HZ: u32 = 50;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
