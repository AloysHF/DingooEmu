# Standalone Emulator

This guide covers installing and running the standalone `dingooemu` binary,
loading `.app` game files, keyboard controls, screenshot mode, and every
command-line option.

## Installation

Download the latest standalone binary for your platform from the
[Releases](https://github.com/jiangxincode/DingooEmu/releases) page.

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
| `--headless` | flag | off | Run in headless mode (no window). Runs for 300 frames and exits. |
| `--frames <N>` | integer | `300` | Number of frames to run in headless mode. |
| `-S, --screenshot <PATH>` | path | — | Render some frames, save a PNG screenshot, then exit. |
| `--screenshot-frames <N>` | integer | `30` | Number of frames to run before the screenshot is taken. |

`--screenshot-frames` only has an effect together with `--screenshot`.

## Default Key Mappings

| Key | Dingoo Button |
|-----|---------------|
| Arrow keys / WASD | D-pad |
| L | A |
| K | B |
| I | X |
| J | Y |
| 1 / Q | SELECT |
| 0 / O | START |
| Left Shift | L shoulder |
| Right Shift | R shoulder |
| Esc | Exit |

Button names accepted by `--remap` are `up`, `down`, `left`, `right`, `a`,
`b`, `x`, `y`, `start`, `select`, `l`, and `r`. For example:

```bash
dingooemu --remap a:space --remap select:tab path/to/game.app
```

`escape` is reserved for exiting and cannot be assigned.

Use `--swap-ab` to keep the physical keys while exchanging their in-game A/B
meaning. It is applied after custom key remapping.

## Audio Output

The standalone emulator sends Dingoo PCM audio to the default system output
device. It supports unsigned 8-bit and signed 16-bit little-endian PCM, mono
and stereo streams, guest volume controls, and automatic device resampling.
If the default output device cannot be opened, emulation continues without
audio and logs a warning. When the output queue is full, the guest audio task
waits and retries the same buffer so playback data is not skipped.

## Performance

Use a release build for normal gameplay. The interpreter uses multi-cycle CPU
steps, treats guest frame submissions as frontend frame boundaries, and
advances any remaining clock budget after a completed frame. This keeps
timers, task delays, video, and audio synchronized at 60 Hz without requiring
the host to dispatch all 336 million hardware clock cycles individually.

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

Screenshots are written to `docs/images`. The default capture point is 300
frames, with shorter per-game overrides for titles that exceed the default
timeout. Explicit parameters apply the requested values to every game:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1 -Frames 60 -TimeoutSeconds 30
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
