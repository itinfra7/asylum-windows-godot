# Asylum for Windows (Godot port)

A Windows port of *Asylum*, the 1994 Acorn Archimedes platform shooter by Andy Southgate. The
game code of [SDL Asylum](http://sdl-asylum.sourceforge.net/) was translated function by
function into Rust, and the port runs on [Godot Engine](https://godotengine.org/) 4.7.2. It
uses the original game data unchanged and adds modern display modes, gamepad and mouse support,
and Korean and Simplified Chinese translations.

Young Sigmund has a few problems. Enter his mind, shoot anything that moves, collect anything
that doesn't, and shut down the eight pulsating neurons hidden in each of the three mental
zones.

## Download and run

1. Download `asylum-windows-godot-<version>-windows-x86_64.zip` from
   [Releases](https://github.com/itinfra7/asylum-windows-godot/releases).
2. Extract it anywhere and start `Asylum\Asylum.exe`. Nothing is installed.

Requirements: 64-bit Windows 10 or later, and a graphics driver with OpenGL 3.3 support.

The game is portable: settings, high scores and the saved position go to `userdata\` next to
`Asylum.exe` (`%APPDATA%\Asylum\` if that folder is not writable).

## Features

- **The original game.** The three mental zones Ego, Psyche and Id with their original maps,
  graphics, music and sounds; the game logic follows SDL Asylum.
- **Display modes.** *Windowed*, *Borderless* (a window covering the whole monitor; the
  default) and *Full Screen* (exclusive). F11 or Alt+Enter switches between *Windowed* and the
  full-screen mode used last (*Borderless* if none was used since the game started).
- **Window size.** Aspect ratios 4:3, 16:10, 16:9 and 21:9; for each, every standard
  resolution that fits the monitor is offered.
- **Scale** from 1.0x to 3.0x in steps of 0.5, as far as the window or screen height allows.
  The game draws 256 × scale lines, so a higher scale raises the internal resolution and shows
  more of the game world.
- **No black bars, no plain stretching.** The picture always fills the window or screen: the
  layout is computed for the actual size, the play area grows with it, and the frame and the
  status panel are assembled from the original artwork.
- **Smooth Motion.** The game logic runs at the original 50 Hz; with *Smooth Motion* on, the
  picture is interpolated to the display's refresh rate. *Vsync* can be switched off.
- **Languages.** English, Korean and Simplified Chinese, chosen from the Windows language
  settings at first start and changeable in the menu. Korean and Chinese text uses pixel fonts
  restyled at run time to match the original font.
- **Input.** Rebindable keys with the original defaults, arrow keys and Space/Ctrl as
  alternatives, gamepads, and the mouse in menus.
- **Sound.** Music and sound effects are synthesized at 48 kHz from the original tracker data
  and samples. *Sound System* offers No Sound, Effects Only or Effects and Music; effect and
  music volumes are set separately.
- **Multithreaded.** Game logic, music synthesis and Godot's renderer run on separate threads,
  assets load in parallel, and each frame is one draw call.

## Controls

| Action | Keyboard (default) | Alternative keys | Gamepad |
|---|---|---|---|
| Left / Right | `Z` / `X` | `←` / `→` | D-pad or left stick |
| Jump | `;` | `↑` | A, D-pad up or stick up |
| Down (use a teleporter) | `.` | `↓` | D-pad down or stick down |
| Fire | `Enter` | `Space`, `Ctrl` | X or B |
| Pause | `Esc`, `Pause` | | Start or Back |
| Windowed ↔ full screen | `F11`, `Alt+Enter` | | |

- The default keys for Left, Right, Jump, Down and Fire are those of the original game;
  `Pause`, `F11` and `Alt+Enter` are additions of this port.
- An alternative key stops working while it is bound to another action.
- Keys are rebound under *Options → 1. Define Controls → 1. Keyboard*; *3. Reset Keys*
  restores the defaults. The gamepad layout is fixed.
- To teleport, stand in the center of a teleporter and press Down. In the Ego zone the
  teleporters look like candelabra.
- Pause screen: `R` returns to the game, `O` opens the options, `Q` loses this life, `Esc`
  abandons the game.
- Menus: number keys choose entries, as in the original game. The arrow keys and `Enter`, the
  mouse (hover to select, left click to choose, right click to go back, wheel to move) and the
  gamepad (D-pad to move, A to choose, B to go back) also work.

## Menus and settings

*Options → 2. Tune Game Options* holds the settings; during a game, `Esc` then `O` opens the
options.

| Entry | Settings |
|---|---|
| 1. Sound System | No Sound, Effects Only, Effects and Music |
| 2. Sound Volume | effect and music volume, 0–10 each |
| 3. Video System | Mode, Aspect and resolution (*Windowed* only), Scale, Smooth Motion, Vsync |
| 4. Language | English, 한국어, 简体中文 |

During a game, *Esc → O → 3. Save Position* saves the position. To continue from it, choose
*Options → 3. Choose Mental Zone → 4. Saved Game* (shown once a position has been saved), then
*Fire - Play*.

Files in the data folder (`userdata\` or `%APPDATA%\Asylum\`):

- `asylum.cfg`: the settings as plain `Key=Value` text.
- `asylum_game.sav`: the saved position.
- `hiscores\`: the high score tables of the three zones.

When the data folder is `userdata\`, the game also writes `override.cfg` next to `Asylum.exe`,
so that the next start opens the window directly in the saved mode and size. Deleting it is
harmless; it is written again at the next start.

## Differences from the original

The port keeps the original game logic, including its quirks.
[docs/PORTING_NOTES.md](https://github.com/itinfra7/asylum-windows-godot/blob/main/docs/PORTING_NOTES.md)
lists every deliberate change; in short:

- crashes and undefined behavior of the C code are fixed,
- larger views are fully supported: the ranges in which enemies are drawn, wake up and stay
  awake, and the placement and lifetime of on-screen messages follow the visible area instead
  of the original 288 × 192 game window,
- the optional 25 Hz mode is not ported, and the music (now synthesized at 48 kHz) no longer
  restarts after every death,
- the menus gained the video and language settings, mouse and gamepad support, and `Esc` goes
  back one level.

[docs/ARCHITECTURE.md](https://github.com/itinfra7/asylum-windows-godot/blob/main/docs/ARCHITECTURE.md)
describes the design: threads, timing and interpolation, layout, rendering, text and storage.

## Building from source

Requirements:

- Windows 10 or later (x64)
- [Rust](https://rustup.rs/) 1.94 or later with the MSVC toolchain
  (`x86_64-pc-windows-msvc`) and the Visual Studio Build Tools (C++ workload)
- [Godot 4.7.2](https://godotengine.org/download/archive/4.7.2-stable/) for Windows (64-bit,
  standard edition), extracted to the folder `Godot_v4.7.2-stable_win64.exe/` (the name of the
  extracted download) in the repository root, or passed to the build script with
  `-Godot <path to Godot_v4.7.2-stable_win64_console.exe>`
- an export template, either
  - the lean 2D/OpenGL template (about 36 MB instead of 109 MB) built by
    `tools/build_template.ps1`, which needs Git and Python 3 with
    [SCons](https://scons.org/); release builds use this template, or
  - the official Godot 4.7.2 export templates installed in the editor. Such a build lacks the
    two patches in `tools/godot-patches/`: in *Borderless* mode the top pixel rows stay
    unpainted, Windows may bitmap-scale the window on monitors with different scale factors,
    and Godot creates `%APPDATA%\Godot\app_userdata\Asylum\`.

Build:

```powershell
# optional: the lean export template, with the patches from tools\godot-patches
powershell -ExecutionPolicy Bypass -File tools\build_template.ps1
# the game: Rust library, Godot import and export, release zip
powershell -ExecutionPolicy Bypass -File tools\build.ps1 -Zip
```

The game is written to `dist/Asylum/`, the release zip to `dist/`. `-SkipDebug` leaves out the
debug library, which the Godot editor loads, including the editor runs of the import and
export steps; use it only when `game/bin/asylum_gdext.debug.dll` is up to date. Run
`tools/build.ps1` once before opening `game/project.godot` in the editor, so that the
GDExtension library exists.

Tests:

```powershell
cd rust
cargo test --release --workspace
# long soak test: every zone, several view sizes, random input, save game round trips
cargo test --release -p asylum-core --test long_soak -- --ignored
```

After changing `game/locale/strings.csv`, regenerate the font subsets from the repository root
with `python -I tools/subset_fonts.py`.

## Repository layout

| Path | Contents |
|---|---|
| `rust/asylum-core/` | engine-independent core: game logic, asset decoders, audio synthesis, menus, rendering lists, settings and storage |
| `rust/asylum-gdext/` | Godot GDExtension: window and display modes, input, drawing, audio output |
| `game/` | Godot project; `game/data/` holds the original game data, `game/locale/strings.csv` the menu and in-game texts in every language, `game/fonts/` the font subsets |
| `tools/` | build scripts, Godot patches, font subset generator |
| `docs/` | architecture and porting notes |
| `reference/sdl-asylum/` | the SDL Asylum C source the port is based on |
| `third_party/` | the Fusion Pixel Font release the font subsets are generated from |
| `licenses/` | license texts of the third-party components |

## Credits

- **Andy Southgate**: the original game *Asylum* (1994), later placed in the public domain
- **Hugh Robinson**: SDL Asylum, the C port this project is based on
- **Dmitry Marakasov, Ian Chapman, James Woodcock, Peter De Wachter**: ports, packages and
  patches of SDL Asylum
- **M-HT**: the SDL2 port and fixes of SDL Asylum ([M-HT/asylum](https://github.com/M-HT/asylum))
- **Jeffrey Lee and John Hoare**: obtained Andy Southgate's original assembly source and hosted
  it on their Asylum tribute site at acornarcade.com
- **[itinfra7](https://github.com/itinfra7)**: this port
- **TakWolf**: [Fusion Pixel Font](https://github.com/TakWolf/fusion-pixel-font) and
  [Ark Pixel Font](https://github.com/TakWolf/ark-pixel-font)
- **Lee Minseo (quiple)**: [Galmuri](https://github.com/quiple/galmuri), the Hangul glyphs

Built with:

- [Godot Engine](https://godotengine.org/) 4.7.2, by Juan Linietsky, Ariel Manzur and the Godot
  Engine contributors, with FreeType, SDL, zlib, Zstandard, libpng, Mbed TLS and the other
  libraries listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
- [godot-rust (gdext)](https://github.com/godot-rust/gdext), by Bromeon and the godot-rust
  contributors
- [Rust](https://www.rust-lang.org/) and [glam](https://github.com/bitshifter/glam-rs)
- [SCons](https://scons.org/), [Python](https://www.python.org/) and Microsoft Visual C++ for the
  export template

## License

Copyright (C) 2026 itinfra7. Based on SDL Asylum, Copyright (C) 2006-2009 Hugh Robinson.

This program is free software: you can redistribute it and/or modify it under the terms of the
GNU General Public License as published by the Free Software Foundation, either version 3 of
the License, or (at your option) any later version. It is distributed WITHOUT ANY WARRANTY; see
[LICENSE](LICENSE) (`LICENSE.txt` in the release package) for the full text. This project is a
modified version of SDL Asylum: its code was rewritten in Rust for Godot Engine in 2026.

Not covered by the GPL:

- The original game data in `game/data/` is in the public domain (Andy Southgate).
- The fonts in `game/fonts/` and `third_party/` are licensed under the SIL Open Font License 1.1.
- Third-party components are licensed under their own terms; see
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and `licenses/`.

Source code: each release is built from its tagged commit of
<https://github.com/itinfra7/asylum-windows-godot>. `Asylum.exe` is built by
`tools/build_template.ps1` from the Godot Engine 4.7.2-stable source
(<https://github.com/godotengine/godot>, commit `ed1daf0bf001b61586d9930840f2f1394092c079`)
with the patches in `tools/godot-patches/`; the Rust dependencies are the crates.io packages
pinned with checksums in `rust/Cargo.lock`.
