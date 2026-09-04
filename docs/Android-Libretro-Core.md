# Android Libretro Core

The DingooEmu libretro core also runs on Android, so it can be reused by most
Android RetroArch-based frontends.

## Install in RetroArch on Android

### Via Online Updater (Recommended)

The easiest way is to download the core directly from RetroArch's built-in Online Updater:

1. Open RetroArch
2. Go to **Main Menu → Online Updater → Core Downloader**
3. Find and select **Dingoo A320 / Gemei A330 (DingooEmu)**, wait for the download to complete
4. Go back to **Main Menu → Load Core** — the DingooEmu core should appear

To update an installed core:

1. Open RetroArch
2. Go to **Main Menu → Online Updater → Update Installed Cores**

### Manual Installation (Alternative)

If the Online Updater is not available, you can install the core manually:

1. **Download** `dingoo-emu-android-libretro.tar.gz` from the
   [Releases](https://github.com/AloysHF/DingooEmu/releases) page. It
   contains `dingooemu_libretro_android.so` for the `arm64-v8a`,
   `armeabi-v7a`, `x86` and `x86_64` ABIs.
2. **Install the core**: copy the `dingooemu_libretro_android.so` matching
   your device's ABI (most modern devices are `arm64-v8a`) into RetroArch's
   `cores/` directory (typically
   `/storage/emulated/0/RetroArch/cores/` or the app's internal `cores/` path),
   and copy `dingooemu_libretro.info` into RetroArch's `info/` directory.
3. **Load** the core and content the same way as on desktop.

The Android core accepts Dingoo A320 `.app` content, Gemei A330 firmware 1.0
`.cc` content, and later A330 `.c2s` (2D) and `.c3s` (3D) content.

## CPU execution engines

For APP/MIPS content, the `arm64-v8a` and `x86_64` cores use a tiered JIT by
default. Frequently executed MIPS32 blocks are translated to native code,
while unsupported or low-frequency paths continue through the cached
interpreter. Compilation is rate-limited to avoid introducing frame-time
spikes while a game warms up. CC/C2S/C3S content always uses the ARM32/Thumb
interpreter, regardless of this option.

Use **Quick Menu → Core Options → CPU Execution Engine** to switch to
`interpreter` for compatibility testing. The `armeabi-v7a` and `x86` cores
always use the interpreter and do not expose this option.

## Sharing a performance diagnostic

To collect a report without Android developer tools:

1. Load the affected game, then open **Quick Menu → Core Options**.
2. Set **Performance Diagnostic Log** to `enabled`.
3. Resume the game and reproduce the slowdown for at least 30 seconds.
4. Close the content so the final counters are written.
5. Send `dingooemu-diagnostic.txt` from RetroArch's save directory, normally
   `/storage/emulated/0/RetroArch/saves/`.

Diagnostics are disabled by default. The file is replaced for each session and
contains timing and aggregate counters, not game data. It is refreshed once per
second while enabled, so it can still be copied if the frontend cannot close
content cleanly. The report includes cumulative and recent frame timing, video
and audio callback costs, audio short writes, frontend buffer status,
asynchronous queue statistics, the 48 kHz host output rate, and JIT execution
and fallback counters. The core leaves frontend latency settings unchanged.
Audio produced while the frontend callback is disabled is discarded, and the
active callback uses a short bounded queue to avoid stale playback.

## Building the Android core locally

Building for Android requires the [Android NDK](https://developer.android.com/ndk)
and [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk):

```bash
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/android-ndk

# Build all four ABIs (artifacts land in target/<triple>/release/)
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86 -t x86_64 --platform 21 \
  build -p dingooemu-libretro --release
```

Each ABI produces `libdingooemu.so`; rename it to
`dingooemu_libretro_android.so` when installing into RetroArch on Android.
The CI release workflow performs this packaging automatically.
