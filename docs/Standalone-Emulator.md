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
| `-s, --scale <N>` | `1`–`16` | `2` | Integer scaling factor for the window. |
| `--headless` | flag | off | Run in headless mode (no window). Runs for 300 frames and exits. |
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

# Take screenshot after 60 frames
dingooemu --screenshot screenshot.png --screenshot-frames 60 game.app

# Headless mode (no window, runs for 300 frames)
dingooemu --headless path/to/game.app
```
