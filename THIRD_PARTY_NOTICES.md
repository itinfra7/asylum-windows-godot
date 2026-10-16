# Third-party notices

Asylum for Windows (Godot port), <https://github.com/itinfra7/asylum-windows-godot>, is
licensed under the GNU General Public License, version 3 or (at your option) any later version
(see `LICENSE`; `LICENSE.txt` in the release package). It contains, or is built from, the
components listed below. The license texts
referred to here are in the `licenses/` folder, both in the source repository and in the
release package.

## Original game data

*Asylum* was written by Andy Southgate in 1994 for the Acorn Archimedes. Its graphics and
music are © 1993 Andy Southgate and have been placed in the public domain by him (as stated in
the SDL Asylum README). The original data files in `game/data/` are used unchanged; the release
embeds them in `Asylum.exe`.

## SDL Asylum

SDL Asylum, Copyright (C) 2006-2009 Hugh Robinson, with the SDL2 port and bug fixes of the fork
by M-HT (<https://github.com/M-HT/asylum>). License: GPL-3.0-or-later. The Rust code of this
project is a translation of it. The reference copy in `reference/sdl-asylum/` is upstream
commit `5bacb53819dba3968c5c68421ee5cce7e33dd9f9`.

## Godot Engine (`Asylum.exe`)

This game uses Godot Engine, available under the following license:

```text
Copyright (c) 2014-present Godot Engine contributors (see AUTHORS.md).
Copyright (c) 2007-2014 Juan Linietsky, Ariel Manzur.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software
and associated documentation files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

`Asylum.exe` is the Windows release export template of Godot 4.7.2-stable (commit
`ed1daf0bf001b61586d9930840f2f1394092c079`, source
<https://github.com/godotengine/godot/tree/4.7.2-stable>), built by `tools/build_template.ps1`
with the two patches in `tools/godot-patches/`; the game's data pack is appended to it. The
original license file is `licenses/godot/LICENSE.txt`.

Godot Engine contains third-party components. The table lists the third-party code this build
of Godot uses. `licenses/godot/COPYRIGHT.txt` is Godot's complete copyright file with the full
texts of the licenses named below (libpng's license is in `licenses/godot/libpng-LICENSE.txt`,
the notices of the FreeType BDF and PCF drivers in `licenses/freetype/BDF-PCF.txt`); it also
covers parts that are compiled but unused here, such as the effects of the Vulkan and
Direct3D 12 renderers.

| Component | Version | License | Copyright |
|---|---|---|---|
| FreeType | 2.14.3 | FTL | 1996-2026 David Turner, Robert Wilhelm, and Werner Lemberg |
| FreeType BDF and PCF drivers | 2.14.3 | MIT; X11 | 2000 Computing Research Labs, New Mexico State University; 2000-2015 Francesco Zappa Nardelli; 1990, 1994, 1998 The Open Group |
| SDL | 3.2.28 | Zlib | 1997-2025 Sam Lantinga |
| hidapi (part of SDL) | 0.14.0 | BSD-3-Clause | 2010 Alan Ott, Signal 11 Software |
| fdlibm (part of SDL) | - | see below | 1993 Sun Microsystems, Inc. |
| zlib | 1.3.2 | Zlib | 1995-2026 Jean-loup Gailly and Mark Adler |
| MiniZip | 1.3.2 | Zlib | 1998-2026 Gilles Vollant; 2007-2008 Even Rouault; 2009-2010 Mathias Svensson |
| Zstandard | 1.5.7 | BSD-3-Clause | Meta Platforms, Inc. and affiliates |
| libpng | 1.6.58 | libpng-2.0 (`licenses/godot/libpng-LICENSE.txt`) | 1995-2026 The PNG Reference Library Authors; 2018-2026 Cosmin Truta; 2000-2002, 2004, 2006-2018 Glenn Randers-Pehrson; 1996-1997 Andreas Dilger; 1995-1996 Guy Eric Schalnat, Group 42, Inc. |
| Mbed TLS | 3.6.7 | Apache-2.0 | The Mbed TLS Contributors |
| Clipper2 | 1.5.4 | BSL-1.0 | 2010-2025 Angus Johnson |
| glad | 2.0.8 | CC0-1.0 and Apache-2.0 | 2013-2022 David Herberth; 2013-2020 The Khronos Group Inc. |
| Grisu2 | simdjson 667d0ed | MIT and Apache-2.0 | 2009 Florian Loitsch; 2018-2023 The simdjson authors |
| GamepadMotionHelpers | 39b578a | MIT | 2020-2023 Julian "Jibb" Smart |
| SPIRV-Reflect | vulkan-sdk-1.4.335.0 | Apache-2.0 | 2017-2022 Google Inc. |
| SPIRV-Headers | vulkan-sdk-1.4.335.0 | MIT | 2015-2024 The Khronos Group Inc. |
| FastLZ | - | MIT | 2005-2020 Ariya Hidayat |
| SMAZ | - | BSD-3-Clause | 2006-2009 Salvatore Sanfilippo |
| r128 | - | Unlicense | Alan Hickman |
| Minimal PCG32 | - | Apache-2.0 | 2014 M.E. O'Neill |
| PolyPartition | - | MIT | 2011-2021 Ivan Fratric and contributors |
| SMOL-V | - | Unlicense or MIT | 2016-2024 Aras Pranckevicius |
| Quite OK Audio (QOA) | - | MIT | 2023 Dominic Szablewski |
| mikktspace | - | Zlib | 2011 Morten S. Mikkelsen |
| OK Lab color space | - | MIT | 2021 Björn Ottosson |
| stb_rect_pack | - | Unlicense or MIT | Sean Barrett |
| NVIDIA NVAPI (minimal excerpt) | - | MIT | 2019-2022 NVIDIA Corporation |
| Bullet convex hull (`core/math/convex_hull`) | - | MIT and Zlib | 2011 Ole Kniemeyer, MAXON |
| Robert Penner's easing functions | - | MIT | 2001 Robert Penner |
| Pitch shift (`audio_effect_pitch_shift`) | - | MIT and WOL | 1999-2015 Stephan M. Bernsee |

Portions of this software are copyright © 2026 The FreeType Project
(<https://freetype.org>). All rights reserved.

The FreeType BDF and PCF font drivers and the hash functions they use carry their own notices,
reproduced in `licenses/freetype/BDF-PCF.txt`.

The SDL math library contains code from fdlibm, which carries this notice:

```text
Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.

Developed at SunPro, a Sun Microsystems, Inc. business.
Permission to use, copy, modify, and distribute this
software is freely granted, provided that this notice
is preserved.
```

## godot-rust (`asylum_gdext.dll`)

The crates `godot`, `godot-core`, `godot-ffi` and `godot-cell` 0.5.5 of godot-rust (gdext),
Copyright (c) godot-rust; Bromeon and contributors, are linked into `asylum_gdext.dll`
unmodified. License: Mozilla Public License 2.0 (`licenses/godot-rust/License.txt`). Their
source code is available at <https://github.com/godot-rust/gdext> (commit
`e7b3a4a7dc9678a5502dc8432c258c702ff89429`) and on crates.io (`godot` 0.5.5); the exact
versions and checksums are recorded in `rust/Cargo.lock`.

## glam (`asylum_gdext.dll`)

glam 0.32.1, Copyright 2020 Cameron Hart. License: MIT or Apache-2.0, at your option
(`licenses/glam/`). It contains code ported from DirectXMath, Realtime Math and GLM under the
MIT license (`licenses/glam/ATTRIBUTION.md`).

## Rust standard library (`asylum_gdext.dll`)

The Rust standard library (Rust 1.98.1), Copyright The Rust Project Developers. License: MIT or
Apache-2.0, at your option; parts under the Unicode License v3 (`licenses/rust/`).
`licenses/rust/COPYRIGHT-library.html` lists the notices of the standard library and its
dependencies.

## Fonts (embedded in `Asylum.exe`)

The Korean and Chinese text uses subsets of **Fusion Pixel Font**, 12px proportional, release
2026.09.25, Copyright (c) 2022, TakWolf (<https://github.com/TakWolf/fusion-pixel-font>).
Fusion Pixel Font is built from other fonts; the glyphs used here come from:

- **Ark Pixel Font**, Copyright (c) 2021, TakWolf (<https://github.com/TakWolf/ark-pixel-font>)
- **Galmuri** 2.40.4, Copyright (c) 2019-2025 Lee Minseo (<https://github.com/quiple/galmuri>),
  the Hangul syllables

All three are licensed under the SIL Open Font License, Version 1.1, without Reserved Font
Names; the subsets are Modified Versions under the same license (`licenses/fonts/`; in the
source repository also `game/fonts/`). The OFL does not extend to the rest of this project, and
the GPL does not apply to the fonts.

## Build-time only

Not part of the release files: the godot-rust crates `godot-macros`, `godot-bindings`,
`godot-codegen` and `gdextension-api` (MPL-2.0), and `proc-macro2`, `quote`, `unicode-ident`,
`venial`, `heck`, `nanoserde` and `nanoserde-derive` (MIT and/or Apache-2.0; `unicode-ident`
also Unicode-3.0).
