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

Dingoo A320 is a handheld game console powered by the Ingenic JZ4740 MIPS SoC. This emulator runs `.app` game files from the Dingoo ecosystem through high-level emulation of the MIPS32 CPU and Dingoo SDK.

## Features

- **MIPS32 CPU interpreter** — Ingenic JZ4740 XBurst compatible instruction set
- **Real-time scheduling** — Guest timing stays at 60 Hz without requiring one host-side dispatch per hardware clock cycle
- **HLE (High-Level Emulation)** — Dingoo SDK functions (graphics, input, audio, timing) implemented in Rust
- **`.app` file support** — Parse and load Dingoo A320 game container format
- **Frame rendering** — 320×240 RGB565 framebuffer with XRGB8888 output
- **PCM audio output** — Dingoo waveout playback with format conversion, volume, and resampling
- **Screenshot mode** — Headless frame capture for automated testing and preview generation
- **Batch screenshot** — Process multiple `.app` files with `scripts/batch-screenshots.ps1`
- **RetroArch integration** — libretro core for use with RetroArch frontend
- **Cross-platform** — Windows, Linux, macOS

## Usage

### Standalone Mode

Download the latest binary from the
[Releases](https://github.com/jiangxincode/DingooEmu/releases) page and run:

```bash
dingooemu path/to/game.app
```

See the [Standalone Emulator](docs/Standalone-Emulator.md) guide for
installation, keyboard controls, screenshot mode, and all command-line options.

### RetroArch Mode

Build the libretro core and load it in RetroArch:

```bash
cargo build -p dingooemu-libretro --release
```

The core file will be produced at `target/release/dingooemu_libretro.dll` (Windows) or `libdingooemu_libretro.so` (Linux).

See the [RetroArch Core](docs/RetroArch-Core.md) guide for installation,
supported features, and RetroPad mapping.

## Building

Requires [Rust](https://www.rust-lang.org/tools/install) (stable).

### Standalone Mode (Default)

```bash
cargo build -p dingooemu --release
cargo run -p dingooemu --release -- path/to/game.app
```

The binary is produced at `target/release/dingooemu` (`.exe` on Windows).

### Libretro Core (for RetroArch)

```bash
cargo build -p dingooemu-libretro --release
```

Cargo names the cdylib after its lib target, so this produces `dingooemu.dll`
on Windows (`libdingooemu.so` on Linux) under `target/release/`. RetroArch
expects the core file to be named `dingooemu_libretro.<ext>`, so rename it
accordingly before dropping it into RetroArch's `cores/` directory.

## Testing

Run the unit tests:

```bash
cargo test --workspace
```

## Architecture

```
crates/
├── dingooemu-core/              # Platform-independent emulator engine (library)
│   └── src/
│       ├── lib.rs               # Crate root (module declarations)
│       ├── emulator.rs          # Shared Emulator (both front-ends)
│       ├── cpu/                 # MIPS32 CPU interpreter
│       │   ├── mod.rs           # CPU module root
│       │   ├── registers.rs     # GPR, HI/LO, PC management
│       │   ├── instructions.rs  # MIPS32 instruction decoder and execution
│       │   └── cop0.rs          # Coprocessor 0 (system control)
│       ├── memory.rs            # Memory bus (32MB address space)
│       ├── video.rs             # Framebuffer and screen rendering
│       ├── audio.rs             # Audio engine (PCM output)
│       ├── input.rs             # Button state management
│       ├── app_loader.rs        # .app container parser
│       ├── hle/                 # High-Level Emulation bridge
│       │   ├── mod.rs           # HLE module root
│       │   └── sdk.rs           # Dingoo SDK function implementations
│       └── error.rs             # Error types
├── dingooemu/                   # Standalone binary (-> dingooemu)
│   └── src/
│       └── main.rs              # Window loop and CLI front-end
└── dingooemu-libretro/          # libretro cdylib (-> dingooemu_libretro.{dll,so,dylib})
    ├── dingooemu_libretro.info  # RetroArch core metadata
    └── src/
        ├── lib.rs               # cdylib crate root
        └── libretro/
            ├── api.rs           # Exported libretro functions
            ├── callbacks.rs     # Callback management
            └── types.rs         # libretro type definitions
```

## Game Compatibility

🚧 **Under Active Development**

This project is in early development. The basic architecture is being established, and simple `.app` titles can now reach the rendering path. Compatibility remains experimental.

| Category | Count | Status |
|----------|-------|--------|
| Verified Games | 8 | ⚠️ Partial |

For detailed game list with screenshots and descriptions, see [Game Compatibility](docs/Game-Compatibility.md).

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

## Contribute

Contributions are welcome! Whether you're interested in fixing bugs, adding features, improving documentation, or testing game compatibility, we'd love your help. See [CONTRIBUTING.md](docs/CONTRIBUTING.md) for details.

## License

This project is licensed under the [BSD 3-Clause License](LICENSE).
