# RetroArch Core

DingooEmu is available as a libretro core for RetroArch on Windows, Linux,
macOS, Android, and iOS. This guide covers installation, loading content,
supported frontend features, and controls.

## Installation

### Manual Installation

Download the core from the
[Releases](https://github.com/jiangxincode/DingooEmu/releases) page. Copy
`dingooemu_libretro.dll` (`.so` on Linux or `.dylib` on macOS) to
RetroArch's `cores/` directory, and copy `dingooemu_libretro.info` to its
`info/` directory.

### Building from Source

```bash
cargo build -p dingooemu-libretro --release
```

Cargo names the cdylib after its lib target, so this produces `dingooemu.dll`
on Windows (`libdingooemu.so` on Linux) under `target/release/`. Rename it to
`dingooemu_libretro.<ext>` before dropping it into RetroArch's `cores/`
directory.

## Mobile Platforms

The same libretro core architecture is available on mobile platforms, with
platform-specific installation requirements:

- [Android Libretro Core](Android-Libretro-Core.md)
- [iOS Libretro Core](iOS-Libretro-Core.md)

## Loading Games

1. Open RetroArch and select **Load Core > Dingoo A320 (DingooEmu)**.
2. Select **Load Content**.
3. Choose a `.app` file.

## Supported Features

- Video output using the XRGB8888 pixel format
- PCM audio output resampled to 22050 Hz stereo
- RetroPad input handling
- `.app` content loading

## RetroPad Button Mapping

| RetroPad Button | Dingoo Button |
|---|---|
| D-Pad Left | Left |
| D-Pad Right | Right |
| D-Pad Up | Up |
| D-Pad Down | Down |
| A (SNES East) | A |
| B (SNES South) | B |
| X (SNES North) | X |
| Y (SNES West) | Y |
| Start | Start |
| Select | Select |
| L1 | L shoulder |
| R1 | R shoulder |
