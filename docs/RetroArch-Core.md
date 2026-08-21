# RetroArch Core

DingooEmu is available as a libretro core for RetroArch on Windows, Linux,
macOS, Android, and iOS. This guide covers installation, loading content,
supported frontend features, and controls.

## Installation

### Online Updater

<!-- TODO: Publish DingooEmu in the official RetroArch Core Downloader index. -->

1. Open RetroArch.
2. Go to **Main Menu > Online Updater > Core Downloader**.
3. Select **Dingoo A320 (DingooEmu)**.

### Manual Installation

Download the core from the
[Releases](https://github.com/AloysHF/DingooEmu/releases) page and extract
the archive for your operating system and CPU architecture. Copy the core file
to RetroArch's `cores/` directory, and copy `dingooemu_libretro.info` to its
`info/` directory.

| Platform | Core file |
|---|---|
| Windows | `dingooemu_libretro.dll` |
| Linux | `dingooemu_libretro.so` |
| macOS | `dingooemu_libretro.dylib` |

### Building from Source

```bash
cargo build -p dingooemu-libretro --release
```

Cargo names the cdylib after its lib target, producing
`dingooemu_libretro.dll` on Windows, `libdingooemu_libretro.so` on Linux, or
`libdingooemu_libretro.dylib` on macOS under `target/release/`. Rename the
Linux or macOS output to remove the leading `lib` before copying it into
RetroArch's `cores/` directory.

## Supported Platforms

| Platform | Architectures | Distribution |
|---|---|---|
| Windows | x86_64 | Core Downloader or release archive |
| Linux | x86_64, aarch64 | Core Downloader or release archive |
| macOS | x86_64, Apple silicon | Core Downloader or release archive |
| Android | arm64-v8a, armeabi-v7a, x86, x86_64 | See the Android guide |
| iOS | arm64 devices, Apple silicon simulator | See the iOS guide |

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

- Video output using the native RGB565 pixel format
- PCM audio output resampled to 22050 Hz stereo
- RetroPad input handling
- `.app` content loading
- Cold reset through RetroArch's **Reset** command
- Persistent guest save files in RetroArch's configured save directory
- Save states with content identity and corruption checks
- Frontend cheat slots for 8/16/32-bit memory and MIPS registers
- Frontend memory access for 32 MiB system RAM and LCD video RAM
- Live core options, including host master volume

The current basic core does not yet provide subsystem loading. The metadata
marks it unavailable so RetroArch does not present unsupported capabilities.

## Game Save Files

Files created through the emulated file API are stored beneath RetroArch's
configured save directory and reopened from there on later sessions. Guest
paths are normalized inside that directory; parent-directory traversal is
rejected. Modified files are flushed when the guest closes them and when the
core resets or unloads content.

## Save States

RetroArch save and load state commands capture the complete mutable CPU,
memory, video, input, audio, scheduler, semaphore, and open-file state. Each
state also preserves active file enumeration and focused-window input dispatch,
and contains a format version, content checksum, payload length, and payload
checksum. States for different content and damaged or incompatible states are
rejected without changing the running emulator.

## Cheats

RetroArch cheat slots accept `TARGET=VALUE` rules. Supported targets are
`mem8:ADDRESS`, `mem16:ADDRESS`, `mem32:ADDRESS`, and `reg:rN` for MIPS
registers `r0` through `r31`. Numbers may be decimal or use a `0x` hexadecimal
prefix. Enabled slots are applied at the start of every emulated frame;
disabled slots remain configured but do not modify state.

## Memory Access

Compatible frontend tools can access the complete 32 MiB system RAM and the
LCD framebuffer mapping through the standard libretro memory API. The core
also registers both regions as memory-map descriptors, including the guest
framebuffer address. Region pointers remain stable across Reset and save-state
loads while content remains loaded.

## Core Options

| Option | Values | Default | Behavior |
|---|---|---|---|
| Audio Volume (%) | `100` to `0` in steps of 10 | `100` | Applies a host master gain without replacing the game's own volume. |
| Key Auto-Repeat Delay | frame counts including `0` | `24` | Sets how long a held button waits before repeating; `0` disables repeat. |
| Key Auto-Repeat Period | `1`–`30` frame choices | `6` | Sets the interval between repeat press events. |
| Swap A/B Buttons | `disabled`, `enabled` | `disabled` | Exchanges the emulated A and B button meanings. |
| CPU/HLE Debug Logging | `disabled`, `enabled` | `disabled` | Enables detailed interpreter and HLE records in the frontend log. |
| Unknown MIPS Instruction Policy | `skip`, `stop` | `skip` | Logs and skips unsupported instructions or stops with an execution error. |
| CPU Execution Engine (64-bit Android) | `jit`, `interpreter` | `jit` | Uses native translation for hot CPU blocks on arm64-v8a and x86_64 Android. Other targets and unsupported instructions use the interpreter. Select `interpreter` for compatibility testing. |

Core option changes are applied while content is running and restored after a
RetroArch reset.

The JIT waits until a block has executed 256 times and rate-limits native
compilation to one block every four frames. Short blocks and repeatedly
unsupported memory paths remain on the interpreter to avoid runtime stutter.

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
