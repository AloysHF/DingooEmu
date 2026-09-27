# A330 DOOM Compatibility

The supplied `DOOM-A330.cc` and `DOOM2-A330.cc` ports are supported for the
scenarios checked below. They use the Gemei A330 Homebrew ARM ABI and contain
their WAD data inside the executable. No external WAD is needed for these
particular packages. Legally obtained game packages must be supplied locally;
game executables, WADs, source archives, captures, and logs are not distributed
with DingooEmu.

## Build identification

The standalone packages and the packages inside the supplied ZIP archives are
different builds. Do not infer slot filenames or behavior from a filename alone.

| Build | SHA-256 |
|---|---|
| DOOM, standalone package | `660c56ff4ab89e66e4e9c6dc219e03b850d4195027411cb91b8a9c8ecf0377ed` |
| DOOM II, standalone package | `47d23468ff78507a3f9a055e023a2ed57329bc4d70a6b4202fc126cb2c5e0d95` |
| DOOM, archive package | `5ccf6fc8bb08f4c9db383471f4eb60977c93ace04ffbc181e7d40068abec9397` |
| DOOM II, archive package | `35c54cae87fff25233d2dd3b492e9f9be190bb99d3d8bd792a20167cd07fda48` |

## Failure causes and changes

Before the fix, the standalone DOOM package still had a black framebuffer after
300 ticks, with no unknown SDK HLE calls reported. Its PC remained at
`0x13843ef4`, polling bit 2 of graphics status register `0x09303054`.

- The 32 MiB Homebrew heap overlaps the system MMIO window. Heap initialization
  cleared the graphics-ready value because scalar register accesses incorrectly
  selected the heap backing store. MMIO accesses now select separate register
  storage before the heap. JIT loads and stores in that window fall back to the
  bus, preserving the mapping and presentation side effects.
- Statically linked libc uses Linux OABI SVCs, including `0x9000c5` (`fstat64`).
  These were mistaken for dynamic SDK import indexes. Console writes to stdout
  and stderr now return byte counts and resume inline. Unavailable OABI services
  return `-ENOSYS`; writes to other descriptors return `-EBADF`. This is a
  limited libc compatibility bridge, not a Linux kernel or general Linux runtime.
- Embedded WAD fields are unaligned. The Homebrew compatibility ABI reads words
  at their byte addresses; rotating aligned loads corrupted the PNAMES count
  (350 became `0x7d00015e`) and led to an invalid allocation. ARM, Thumb, and
  native JIT now use the same Homebrew read behavior. Retail rotated word loads
  retain their previous semantics.
- Homebrew cache flushes no longer submit unfinished offscreen buffers. The
  graphics surface register remains the presentation boundary, preventing black
  frames while the guest draws.
- Legacy IIS device APIs now configure and deliver stereo signed 16-bit PCM,
  return written byte counts, respect device-buffer backpressure, close handles,
  and expose the SDK volume scale. Device handles live in guest heap memory, so
  existing snapshots retain them without changing the state format. Unsupported
  optional `wavaopen`, `waveioc`, and `waveclose` paths return zero; the tested
  ports use IIS for their sound effects.

## Validation scope

On Windows x64, both cached interpretation and native JIT passed the following
checks for each of the four builds identified above:

| Scenario | DOOM | DOOM II |
|---|---|---|
| Title and new-game menus | Passed | Passed |
| First level, movement, firing, and automap | Passed | Passed |
| Nonzero sound-effect PCM | Passed | Passed |
| Save, restart, load, and save a second slot | Passed | Passed |
| In-game quit confirmation and guest termination | Passed | Passed |

Validation uses the shared emulator core with scripted logical buttons, real
elapsed time for guest timers, and strict unknown instruction/SDK policies.
The test requires a guest-created save with a valid header and end marker,
restarts the emulator, loads that save, and saves a second slot to prove that
loading restored an active level. It also requires nonzero PCM and normal
termination after the in-game quit confirmation. Optional PNG captures support
visual review of title, menu, level, movement/fire, automap, saves, and quit.

The standalone builds use zero-based filenames such as `a/doom1sav0.dsg` and
`a/doom2sav0.dsg`, with descriptions such as `E1M1 SLOT 1`. The archive builds
use one-based filenames such as `a/doom1sav1.dsg` and `a/doom2sav1.dsg`, with
`GAME SAVE 1` descriptions. Save paths are relative to the configured save root;
the default standalone frontend uses the content directory. Existing guest save
files are not renamed or migrated.

Background music is disabled in these ports: the supplied startup code adds
`-nomusic`, and `InitMusicModule` excludes `A330_CC` builds. Enabling music would
require changes to the game port, rather than only an emulator correction.
Full campaign completion,
long-duration stability, every weapon/door/level, and hardware-specific effects
remain unverified. Core results apply to both frontends' shared emulation code;
they do not substitute for an interactive RetroArch session or a subjective
audio quality check.

## Reproduce locally

Set these variables to your legally obtained packages, then run:

```powershell
$env:DINGOOEMU_DOOM1_CC = 'D:\Games\DOOM-A330.cc'
$env:DINGOOEMU_DOOM2_CC = 'D:\Games\DOOM2-A330.cc'
$env:DINGOOEMU_DOOM_CAPTURES = 'D:\Checks\doom-captures'
cargo test -p dingooemu-core --release --features jit --test a330_doom_test -- --ignored --nocapture
```

For the cached interpreter, set `DINGOOEMU_DOOM_INTERPRETER=1` before the same
command. Remove that variable to enable the JIT again. Repeat with the archive
package paths to verify those versions separately. These tests are ignored by
default because copyrighted game content is not included. They use temporary
save directories and remove them on success. Run without the `standalone`
feature when collecting PCM: that feature sends samples to the host or its
virtual device buffer instead of the core sample queue.

Synthetic memory, packed-load, OABI, offscreen-cache, and IIS regressions run
without game files:

```text
cargo test -p dingooemu-core --release --features jit --lib a330::
```
