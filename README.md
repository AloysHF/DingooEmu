# Dingoo A320 Emulator — A Dingoo A320 emulator written in Rust

<p align="center">
  <img src="res/logo-banner.png" alt="Dingoo A320 Emulator" width="600">
</p>

<p align="center">
  <a href="https://jiangxincode.github.io/DingooEmu/"><img src="https://img.shields.io/badge/Website-DingooEmu-E8553A?logo=githubpages&logoColor=white" alt="Website"></a>
  <a href="https://github.com/jiangxincode/DingooEmu/actions/workflows/ci.yml"><img src="https://github.com/jiangxincode/DingooEmu/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/jiangxincode/DingooEmu/releases/latest"><img src="https://img.shields.io/github/v/release/jiangxincode/DingooEmu" alt="Release"></a>
  <a href="https://github.com/jiangxincode/DingooEmu/releases"><img src="https://img.shields.io/github/downloads/jiangxincode/DingooEmu/total" alt="Downloads"></a>
  <a href="https://sonarcloud.io/dashboard?id=jiangxincode_DingooEmu"><img src="https://sonarcloud.io/api/project_badges/measure?project=jiangxincode_DingooEmu&metric=alert_status" alt="Quality Gate Status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-BSD%203--Clause-blue.svg" alt="License: BSD 3-Clause"></a>
  <a href="https://discord.gg/7XDdSrYD"><img src="https://img.shields.io/badge/Discord-Join%20Us-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
  <a href="https://qm.qq.com/q/LAO7DKAWUC"><img src="https://img.shields.io/badge/QQ%E7%BE%A4-Join%20Us-12B7F5?logo=tencent-qq&logoColor=white" alt="QQ Group"></a>
</p>

A cross-platform, open-source Dingoo A320 emulator with RetroArch integration.

## Features

- **MIPS32 CPU interpreter** — Ingenic JZ4740 XBurst compatible
- **HLE (High-Level Emulation)** — Dingoo SDK functions implemented in Rust
- **Cross-platform** — Windows, Linux, macOS, Android, iOS
- **RetroArch integration** — libretro core for use with RetroArch frontend
- **`.app` file support** — Load Dingoo A320 game files

## Status

🚧 **Under Active Development**

This project is in early development. The basic architecture is being established, and simple `.app` titles can now reach the rendering path. Compatibility remains experimental.

See the [game compatibility notes](docs/Game-Compatibility.md) for verified behavior.

## Quick Start

### Standalone Mode

```bash
cargo build -p dingooemu --release
cargo run -p dingooemu --release -- path/to/game.app
```

### Screenshot Mode

Take a headless screenshot for automated testing or preview generation:

```bash
# Take screenshot after 30 frames (default) and save as PNG
cargo run -p dingooemu --release -- path/to/game.app --screenshot preview.png

# Take screenshot after a custom number of frames
cargo run -p dingooemu --release -- path/to/game.app --screenshot preview.png --screenshot-frames 60
```

### Batch Screenshot Mode

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

### RetroArch Mode

```bash
cargo build -p dingooemu-libretro --release
```

The core file will be produced at `target/release/dingooemu_libretro.dll` (Windows) or `libdingooemu_libretro.so` (Linux).

## Building

Requires [Rust](https://www.rust-lang.org/tools/install) (stable).

```bash
cargo build --release
```

## Project Structure

```
DingooEmu/
├── Cargo.toml                    # Workspace
├── crates/
│   ├── dingooemu-core/          # Platform-independent emulator engine
│   ├── dingooemu/               # Standalone binary
│   └── dingooemu-libretro/      # RetroArch libretro core
├── docs/
└── README.md
```

## Keyboard Controls

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

## License

BSD-3-Clause
