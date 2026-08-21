# Standalone Emulator

This guide covers installing and running the standalone `dingooemu` binary,
loading `.app` game files, keyboard controls, screenshot mode, and every
command-line option.

## Installation

Download the latest standalone binary for your platform from the
[Releases](https://github.com/AloysHF/DingooEmu/releases) page.

You can also build it from source:

```bash
cargo build -p dingooemu --release
```

The binary is produced at `target/release/dingooemu` (`.exe` on Windows).

## Loading Games

The standalone emulator accepts `.app` files from the Dingoo A320 ecosystem.

```bash
dingooemu path/to/game.app
```

You can always print the built-in help with:

```bash
dingooemu --help
```

## Synopsis

```text
dingooemu [OPTIONS] <PATH>
```

## Options

| Option | Value | Default | Description |
|---|---|---|---|
| `<PATH>` | path | *required* | Path to the `.app` game file. |
| `-s, --scale <N>` | `1`–`16` | `2` | Validated integer scaling factor for the window. |
| `-f, --fullscreen` | flag | off | Open a borderless window at the desktop resolution. |
| `-v, --volume <N>` | `0`–`100` | `100` | Set the host master volume; `0` mutes output. |
| `--debug-logging` | flag | off | Enable debug-level emulator logging unless `RUST_LOG` overrides it. |
| `--remap <BUTTON:KEY>` | mapping | — | Replace a button's default keyboard mapping; may be repeated. |
| `--swap-ab` | flag | off | Exchange the emulated A and B button masks. |
| `--filter <MODE>` | `nearest`, `bilinear`, `bicubic`, `xbrz` | `nearest` | Select the window scaling filter. |
| `--show-gamepad` | flag | off | Overlay the current logical Dingoo button state. |
| `--repeat-delay <N>` | frames | `24` | Frames before a held button begins generating repeat presses. |
| `--repeat-period <N>` | frames, at least `1` | `6` | Frames between repeat presses after the delay. |
| `--cheat <RULE>` | rule | — | Freeze a memory location or MIPS register once per frame; may be repeated. |
| `--unknown-instruction-policy <MODE>` | `stop`, `skip` | `skip` | Stop on or log and skip an unimplemented MIPS instruction. |
| `--unknown-hle-policy <MODE>` | `report`, `stop` | `report` | Aggregate unknown SDK calls and continue with zero, or stop at the first non-allowlisted call. |
| `--allow-unknown-hle <NAME>` | exact function name | — | Preserve compatibility-stub behavior for one function in strict HLE mode; may be repeated. |
| `--hle-report <PATH>` | path | — | Write stable JSON diagnostics with an `unknown_hle` list, including counts and first-call context. |
| `--headless` | flag | off | Run in headless mode (no window). Runs for 300 frames and exits. |
| `--frames <N>` | integer | `300` | Number of frames to run in headless mode. |
| `-S, --screenshot <PATH>` | path | — | Render some frames, save a PNG screenshot, then exit. |
| `--screenshot-frames <N>` | integer | `30` | Number of frames to run before the screenshot is taken. |

`--screenshot-frames` only has an effect together with `--screenshot`.

## Default Key Mappings

| Key | Dingoo Button |
|-----|---------------|
| Arrow keys | D-pad |
| X | A |
| Z | B |
| S | X |
| A | Y |
| Enter | START |
| Right Shift | SELECT |
| Q | L shoulder |
| W | R shoulder |
| Esc | Exit |

These defaults match RetroArch's standard keyboard bindings for the equivalent
RetroPad buttons, so each physical key has the same in-game meaning in both
frontends.

Button names accepted by `--remap` are `up`, `down`, `left`, `right`, `a`,
`b`, `x`, `y`, `start`, `select`, `l`, and `r`. For example:

```bash
dingooemu --remap a:space --remap select:tab path/to/game.app
```

`escape` is reserved for exiting and cannot be assigned.

Use `--swap-ab` to keep the physical keys while exchanging their in-game A/B
meaning. It is applied after custom key remapping.

## Display Filters

`nearest` preserves hard pixel edges, `bilinear` smooths adjacent pixels,
`bicubic` uses sharper cubic interpolation, and `xbrz` applies edge-aware
pixel-art smoothing. All modes preserve the native 4:3 aspect ratio and add
black bars when the window or desktop has a different aspect ratio.

Use `--show-gamepad` when testing mappings or recording demonstrations. The
overlay is drawn at the native resolution before the selected display filter.

At 60 Hz, the default repeat delay is about 0.4 seconds and the default repeat
period is about 0.1 seconds. Set `--repeat-delay 0` to disable synthetic repeat.

## Cheat Rules

Cheat rules are applied before every emulated frame. Supported targets are
`mem8`, `mem16`, `mem32`, and MIPS registers `r0` through `r31`. Decimal and
`0x`-prefixed hexadecimal values are accepted:

```bash
dingooemu --cheat mem8:0x80100000=99 --cheat reg:r4=0x1234 game.app
```

Writes still pass through the emulator's normal memory validation. Invalid or
out-of-range rules stop startup with a clear error instead of being ignored.

For compatibility testing, `--unknown-instruction-policy stop` turns the first
unimplemented MIPS instruction into an execution error. The default `skip`
behavior logs the instruction and continues, matching previous releases.

Unknown SDK calls are always aggregated by exact function name. The default
`--unknown-hle-policy report` returns zero for compatibility and logs only the
first occurrence plus an end-of-run summary. Each entry records the total call
count, import address, first guest call site, and the first `a0` through `a3`
arguments. `--unknown-hle-policy stop` turns the first unknown call into an
execution error after recording it. Use `--allow-unknown-hle NAME` only for a
reviewed compatibility exception; matching is case-sensitive and exact.

The JSON report is written even when strict mode stops emulation:

```bash
dingooemu game.app --headless --frames 300 --unknown-hle-policy stop \
  --hle-report hle-report.json
```

## Audio Output

The standalone emulator sends Dingoo PCM audio to the default system output
device. It supports unsigned 8-bit and signed 16-bit little-endian PCM, mono
and stereo streams, guest volume controls, and automatic device resampling.
If the default output device cannot be opened, emulation continues without
audio and logs a warning. When the output queue is full, the guest audio task
waits and retries the same buffer so playback data is not skipped.

## Game Save Files

Files created through the emulated file API persist beside the loaded `.app`
file. Guest subdirectories are supported, while parent-directory traversal is
rejected. Modified files are flushed when the guest closes them and when the
emulator stops or resets.

## Performance

Use a release build for normal gameplay. The interpreter caches sequential
guest instruction blocks, uses multi-cycle CPU steps, treats guest frame
submissions as frontend frame boundaries, and advances any remaining clock
budget after a completed frame. This keeps timers, task delays, video, and
audio synchronized at 60 Hz without requiring the host to dispatch all 336
million hardware clock cycles individually.

## Screenshot Mode

Take a headless screenshot for automated testing or preview generation:

```bash
# Take screenshot after 30 frames (default) and save as PNG
dingooemu path/to/game.app --screenshot preview.png

# Take screenshot after a custom number of frames
dingooemu path/to/game.app --screenshot preview.png --screenshot-frames 60
```

## Batch Screenshot Mode

Build the standalone emulator and capture every `.app` file under
`tmp/dingoo_game` recursively:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1
```

Screenshots are written to `docs/images`, and per-game JSON diagnostics are
written to `tmp/hle-reports`. Every report contains an `unknown_hle` array,
including an empty array when no gaps were observed. The default capture point
is 60 frames, with per-game overrides for slow-starting or
performance-sensitive titles. Explicit parameters apply the requested values
to every game:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1 -Frames 60 -TimeoutSeconds 30
```

Run the same set in strict mode, optionally with reviewed exceptions:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1 `
  -UnknownHlePolicy stop `
  -AllowUnknownHle legacy_function `
  -ReportDirectory tmp/hle-reports-strict
```

## Examples

```bash
# Basic usage
dingooemu path/to/game.app

# 2x scaling (default)
dingooemu --scale 2 path/to/game.app

# 4x scaling
dingooemu --scale 4 path/to/game.app

# Borderless fullscreen mode
dingooemu --fullscreen path/to/game.app

# Run at 35% master volume
dingooemu --volume 35 path/to/game.app

# Enable detailed emulator diagnostics
dingooemu --debug-logging path/to/game.app

# Take screenshot after 60 frames
dingooemu --screenshot screenshot.png --screenshot-frames 60 game.app

# Headless mode (no window, runs for 300 frames)
dingooemu --headless path/to/game.app

# Run exactly 120 frames without opening a window
dingooemu --headless --frames 120 path/to/game.app
```
