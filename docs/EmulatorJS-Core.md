# EmulatorJS Core (Browser)

DingooEmu can run in the browser as a standard
[EmulatorJS](https://emulatorjs.org) core. The build follows EmulatorJS's own
pipeline: the Rust libretro core is compiled for `wasm32-unknown-emscripten`,
linked against the `EmulatorJS/RetroArch` fork, and packaged as the
`<core>-wasm.data` bundles that an EmulatorJS page loads at runtime.

The project site ships a ready-to-use player at
<https://AloysHF.github.io/DingooEmu/play.html>: visitors pick a game file from
their own machine and it runs from a blob URL, so nothing is uploaded and the
site does not host any game data.

## What you get

Every nightly and tagged release publishes a single asset:

| File | Contents |
|------|----------|
| `dingoo-emu-emulatorjs.zip` | Both core variants, the build report `cores/reports/dingooemu.json`, `core.json`, `build.json`, `license.txt`, `THIRD_PARTY_LICENSES.md` and a README |

The two `.data` files stay separate inside the ZIP: EmulatorJS requests
`<core>-wasm.data` or `<core>-legacy-wasm.data` by name depending on WebGL2
support, so one bundle cannot serve both cases.

The bundle is laid out like this:

```
dingoo-emu-emulatorjs.zip
├── cores/
│   ├── dingooemu-wasm.data
│   ├── dingooemu-legacy-wasm.data
│   └── reports/dingooemu.json
├── core.json
├── build.json
├── license.txt
├── THIRD_PARTY_LICENSES.md
└── README.md
```

Each `.data` file is a 7z archive containing `<core>_libretro.js`,
`<core>_libretro.wasm`, `core.json`, `build.json`, `license.txt` and
`THIRD_PARTY_LICENSES.md`.

## Install into a site

EmulatorJS always fetches cores from `<EJS_pathtodata>cores/`, so host the
runtime yourself (or use the pinned CDN files) and drop the core beside it:

```
data/
├── loader.js
├── emulator.min.js
├── emulator.min.css
├── compression/
│   ├── extract7z.js       # extracts the core .data archive
│   └── extractzip.js      # extracts ZIP content, if you feed the core one
├── localization/          # optional, e.g. zh-CN.json
└── cores/
    ├── dingooemu-wasm.data
    ├── dingooemu-legacy-wasm.data
    └── reports/dingooemu.json
```

`emulator.min.zip` only contains `emulator.min.js` and `emulator.min.css`;
the `compression/` and `localization/` files are loaded on demand from the
same EmulatorJS data directory. Without `extract7z.js` the core download ends
in a "Network Error".

To upgrade the runtime, replace the runtime files with a newer EmulatorJS
release (≥ 4.2.3) and keep this `cores/` directory.

## Use

```html
<div id="game"></div>
<script>
  EJS_player = "#game";
  EJS_core = "dingooemu";
  EJS_gameUrl = "games/your-game.app";
  EJS_pathtodata = "data/";
  EJS_startOnLoaded = false;
</script>
<script src="data/loader.js"></script>
```

`site/play.html` in this repository is a complete example.

When the game comes from a blob URL, EmulatorJS writes it into the wasm
filesystem under `EJS_gameName`. Set that to the original file name, extension
included (for example `game.app`), otherwise the frontend cannot infer the
content type and the core receives an unknown path.

### Content

Dingoo A320 `.app` and Gemei A330 `.cc`, `.c2s` and `.c3s` games are
self-contained CCDL files, so the browser player accepts a single file — no
ZIP packaging is required. The core loads it through `retro_load_game` with
`need_fullpath = true`, which is why the file must be written into the wasm
filesystem rather than passed as a memory buffer. See
[Game-File-Formats.md](Game-File-Formats.md) for the container layouts and
[Game-Compatibility.md](Game-Compatibility.md) for tested titles.

Game data is copyrighted and is neither distributed with this repository nor
required by the site: the play page only loads files the visitor selects.

## Rendering and performance

The core renders through libretro's software framebuffer in `RGB565`, so the
browser player does not depend on WebGL. Both variants are still built because
EmulatorJS picks the file name by WebGL2 support.

The Cranelift JIT is limited to 64-bit native targets, so the wasm build always
uses the cached-block interpreter. The **CPU Execution Engine** option is
therefore not registered in the browser build.

## Controls

EmulatorJS maps the RetroPad to its default keyboard and gamepad bindings:

| Input | Dingoo Button |
|-------|---------------|
| Arrow keys | D-pad |
| X | A |
| Z | B |
| S | X |
| A | Y |
| Enter | START |
| Right Shift | SELECT |
| Q | L shoulder |
| W | R shoulder |

Touch controls and physical gamepads are provided by EmulatorJS. The
**Swap A/B Buttons** option exchanges the emulated A and B meanings, and the
**Screen Orientation** option rotates 240×320 portrait software.

## Saves

The core does not expose battery RAM through `retro_get_memory_data`, so
EmulatorJS's battery-save buttons are turned off by `"save": false` in
`core.json`. Save states are unaffected: they use `retro_serialize` and are
available from the EmulatorJS menu. They live in the browser's IndexedDB-backed
`/data/saves` mount, so they survive page reloads in the same browser profile.

## Core options

The core registers the same options as the desktop build through the legacy
`SET_VARIABLES` interface, which RetroArch/EmulatorJS upgrades to its normal
options menu:

| Option | Default | Effect |
|--------|---------|--------|
| Audio Volume (%) | 100 | Output volume |
| Key Auto-Repeat Delay (frames) | 24 | Delay before a held key repeats; `0` disables repeat |
| Key Auto-Repeat Period (frames) | 6 | Interval between repeat events |
| Swap A/B Buttons | disabled | Exchanges the A and B button meanings |
| Performance Diagnostic Log | disabled | Writes `dingooemu-diagnostic.txt` to the save directory |
| Unknown Guest Instruction Policy | skip | Skips or stops on unsupported MIPS/ARM instructions |
| Screen Orientation | landscape | Keeps 320×240 or rotates to 240×320 |

## Building the bundle

The nightly and release workflows run this automatically. To build locally you
need an activated [emsdk](https://emscripten.org/docs/getting_started/downloads.html)
(`emmake` in `PATH`), `p7zip`, `zip`, and the pinned nightly toolchain:

```bash
# Rust's prebuilt emscripten stdlib carries wasm exception handling, which
# Binaryen's Asyncify pass rejects. Rebuild std with panic=abort instead.
cargo +nightly build -p dingooemu-libretro \
  --target wasm32-unknown-emscripten --profile wasm-release \
  -Zbuild-std=std,panic_abort

scripts/build-emulatorjs-core.sh
```

The script clones the pinned `EmulatorJS/RetroArch` revision, links the staticlib
with `Makefile.emulatorjs`, builds the WebGL2 and legacy variants, and writes
`out/dingoo-emu-emulatorjs.zip`. `--work-dir` moves the RetroArch checkout out
of `target/`, which matters on CI because `rust-cache` restores `target/`
without the checkout's `.git`.

The Pages deployment fetches the pinned EmulatorJS runtime, downloads the
`dingoo-emu-emulatorjs.zip` asset from the rolling `nightly-build` release, and
redeploys after every successful Nightly Build.
