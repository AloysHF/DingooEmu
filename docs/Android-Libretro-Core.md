# Android Libretro Core

The DingooEmu libretro core also runs on Android, so it can be reused by most
Android RetroArch-based frontends.

## Install in RetroArch on Android

### Via Online Updater (Recommended)

The easiest way is to download the core directly from RetroArch's built-in Online Updater:

1. Open RetroArch
2. Go to **Main Menu → Online Updater → Core Downloader**
3. Find and select **Dingoo A320 (DingooEmu)**, wait for the download to complete
4. Go back to **Main Menu → Load Core** — the Dingoo A320 core should appear

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

## CPU execution engines

The `arm64-v8a` and `x86_64` cores use a tiered JIT by default. Frequently
executed MIPS32 blocks are translated to native code, while unsupported or
low-frequency paths continue through the cached interpreter. Compilation is
rate-limited to avoid introducing frame-time spikes while a game warms up.

Use **Quick Menu → Core Options → CPU Execution Engine** to switch to
`interpreter` for compatibility testing. The `armeabi-v7a` and `x86` cores
always use the interpreter and do not expose this option.

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

Each ABI produces `libdingooemu_libretro.so`; rename it to
`dingooemu_libretro_android.so` when installing into RetroArch on Android.
The CI release workflow performs this packaging automatically.
