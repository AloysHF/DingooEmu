# Emulator Architecture

DingooEmu separates content classification, package validation, device
selection, and device execution. A filename extension is never used as a CPU
or runtime switch by itself.

## Content Loading and Runtime Selection

```text
path extension ──> ContentFormat ───────────────┐
                                                ├──> compatible pair
CCDL bytes ──────> PackageImage ──> TargetDevice┘          │
                                                           v
                                                     Emulator facade
                                                       /        \
                                                A320 runtime  A330 runtime
```

Loading proceeds in this order:

1. The case-insensitive extension is classified as `app`, `cc`, `c2s`, or
   `c3s`. This produces a `ContentFormat`, which describes the distribution
   category visible to users and frontends.
2. `PackageImage` validates the CCDL chunks, payload bounds, entry point, and
   RAWD metadata.
3. The RAWD load origin determines `TargetDevice`. Known A330 origins also
   select the retail or homebrew ARM ABI profile; supported high-address MIPS
   packages select A320.
4. `ContentFormat::supports_target` rejects incompatible pairs. An `app`
   package must target A320, while `cc`, `c2s`, and `c3s` packages must target
   A330.
5. The `Emulator` facade selects `a320::runtime::Runtime` or
   `a330::runtime::Runtime` from `TargetDevice`.

Consequently, changing a suffix cannot change the guest CPU. The three A330
extensions retain their firmware/category meaning, but all use the A330
ARM32/Thumb runtime after metadata validation.

## Module Ownership

| Area | Responsibility |
|---|---|
| `emulator.rs` | Device-neutral lifecycle and frontend facade |
| `content.rs` | Content categories, target devices, and ABI profiles |
| `package.rs` | Shared CCDL parsing and metadata validation |
| `common/` | Device-independent audio, video conversion, logical input, cheat syntax, save-state codec, and execution policies |
| `a320/` | A320 MIPS CPU, memory map, JIT, runtime, cheat backend, and SDK HLE |
| `a330/` | A330 ARM32/Thumb CPU, memory map, runtime, cheat backend, firmware archive, and SDK HLE |
| `dingooemu` / `dingooemu-libretro` | Standalone and libretro frontend integration |

Dependencies point from the facade into one device runtime and from each
device runtime into shared services. `common/` does not depend on either device,
and device implementations do not import one another.

Both device runtimes keep their SDK bridges under `runtime/sdk_hle/`. A330 HLE
dispatch is divided into display, files/resources, input, audio, system/memory,
tasks/synchronization, and semihosting services.

## Extending Format Support

When adding a format, keep category detection and device detection separate:

1. Add the user-visible category to `ContentFormat`.
2. Define which validated `TargetDevice` values may carry that category.
3. Add metadata detection only if the package introduces a genuinely new
   device or ABI profile.
4. Route execution by `TargetDevice`, not by extension.
5. Add tests for valid pairs, mismatches, unknown metadata, and frontend
   extension advertisement.

This preserves one validated dispatch path for standalone, libretro, tests,
and future frontends.
