# RetroArch Core

DingooEmu is available as a libretro core for RetroArch on Windows, Linux,
macOS, Android, and iOS. This guide covers installation, loading content,
supported frontend features, and controls.

## Installation

### Online Updater

<!-- TODO: Publish DingooEmu in the official RetroArch Core Downloader index. -->

1. Open RetroArch.
2. Go to **Main Menu > Online Updater > Core Downloader**.
3. Select **Dingoo A320 / Gemei A330 (DingooEmu)**.

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

Cargo names the cdylib after its lib target, producing `dingooemu.dll` on
Windows, `libdingooemu.so` on Linux, or `libdingooemu.dylib` on macOS under
`target/release/`. Rename it to `dingooemu_libretro.<ext>` before copying it
into RetroArch's `cores/` directory.

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

1. Open RetroArch and select **Load Core > Dingoo A320 / Gemei A330 (DingooEmu)**.
2. Select **Load Content**.
3. Choose an `.app`, `.cc`, `.c2s`, or `.c3s` file.

| Device generation | Native content | Guest CPU |
|---|---|---|
| Dingoo A320 | `.app` | MIPS32 |
| Gemei A330 firmware 1.0 | `.cc` | ARM32/Thumb |
| Later Gemei A330 firmware, 2D software | `.c2s` | ARM32/Thumb |
| Later Gemei A330 firmware, 3D software | `.c3s` | ARM32/Thumb |

Renaming a file is not enough to change its type. The core validates the CCDL
container, load address, and architecture before selecting a runtime.

## Supported Features

- Video output using the native RGB565 pixel format
- PCM audio output resampled to 48 kHz stereo for common host audio devices
- Asynchronous audio delivery when supported by the frontend, with automatic
  synchronous fallback
- RetroPad input handling
- `.app`, `.cc`, `.c2s`, and `.c3s` content loading
- Cold reset through RetroArch's **Reset** command
- Persistent guest save files in RetroArch's configured save directory
- Save states with content identity and corruption checks
- Frontend cheat slots for 8/16/32-bit memory and guest registers
- Frontend memory access for the active runtime's system RAM and video RAM
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

RetroArch save and load state commands capture the active runtime's complete
mutable CPU, memory, video, input, audio, scheduler, semaphore, heap, dynamic
import, and open-file state. Architecture-specific state such as A320 file
enumeration and focused-window input dispatch is included when applicable.
Each state contains a format version, content checksum, payload length, and
payload checksum. States for different content and damaged or incompatible
states are rejected without changing the running emulator. The fixed state
capacity is selected per runtime because the A330 memory map is larger.

## Cheats

RetroArch cheat slots accept `TARGET=VALUE` rules. Supported targets are
`mem8:ADDRESS`, `mem16:ADDRESS`, `mem32:ADDRESS`, and `reg:rN`. APP/MIPS
content exposes registers `r0` through `r31`; A330/ARM content exposes `r0`
through `r15`. Numbers may be decimal or use a `0x` hexadecimal prefix.
Enabled slots are applied at the start of every emulated frame; disabled slots
remain configured but do not modify state. Memory rules must target writable
RAM or framebuffer mappings, not A330 MMIO.

## Memory Access

Compatible frontend tools can access system RAM and framebuffer memory through
the standard libretro memory API. APP content exposes 32 MiB of system RAM and
its LCD mapping. A330 content exposes 64 MiB of system RAM and an 8 MiB
framebuffer region. The core also registers both regions as memory-map
descriptors with their guest addresses. Region pointers remain stable across
Reset and save-state loads while content remains loaded.

## Core Options

| Option | Values | Default | Behavior |
|---|---|---|---|
| Audio Volume (%) | `100` to `0` in steps of 10 | `100` | Applies a host master gain without replacing the game's own volume. |
| Key Auto-Repeat Delay | frame counts including `0` | `24` | Sets how long a held button waits before repeating; `0` disables repeat. |
| Key Auto-Repeat Period | `1`–`30` frame choices | `6` | Sets the interval between repeat press events. |
| Swap A/B Buttons | `disabled`, `enabled` | `disabled` | Exchanges the emulated A and B button meanings. |
| Performance Diagnostic Log | `disabled`, `enabled` | `disabled` | Writes a compact `dingooemu-diagnostic.txt` performance report to the frontend save directory without enabling verbose frontend logs. |
| Unknown Guest Instruction Policy | `skip`, `stop` | `skip` | Logs and skips unsupported MIPS or ARM instructions, or stops with an execution error. Memory failures always remain errors. |
| CPU Execution Engine (64-bit Android) | `jit`, `interpreter` | `jit` | Selects native translation or cached interpretation for APP/MIPS content on arm64-v8a and x86_64 Android. A330 content always uses the ARM interpreter. |

Core option changes are applied while content is running and restored after a
RetroArch reset.

For APP content, the JIT waits until a block has executed 256 times and
rate-limits native compilation to one block per frame. Short blocks and
repeatedly unsupported memory paths remain on the MIPS interpreter to avoid
runtime stutter.

When diagnostics are enabled, the report is refreshed once per second and when
content is unloaded. It includes cumulative and recent 60-frame timing,
separate video and audio callback costs, audio short-write and frontend buffer
status counters, asynchronous queue statistics, the 48 kHz output rate,
plus JIT execution, compilation, and fallback counters. Supported frontends
deliver audio outside the emulation thread so audio backpressure does not stall
video and input. The core leaves frontend latency settings unchanged, discards
audio while the frontend callback is disabled, and uses only a short bounded
queue so stale audio cannot build up across pauses.
Diagnostics are disabled by default; timing, report writes, frontend buffer
callbacks, and JIT counter updates remain inactive until explicitly enabled.

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
