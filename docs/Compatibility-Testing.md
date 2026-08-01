# Compatibility Testing

This guide explains the reproducible L0/L1/L2 compatibility system, why each
level exists, how external game fixtures are handled, how to run the batch
suite, and how to create or review deterministic input scenarios.

## Scope

Compatibility levels describe specific observed evidence. They are cumulative,
but none of the current levels means that a game is fully playable or
completable.

| Level | Requirement | What it proves | What it does not prove |
|---|---|---|---|
| L0 Load | Load the requested content and emit matching diagnostics. | The container was parsed and execution was initialized. | Rendering, input, audio, saves, or gameplay. |
| L1 Boot | Complete every requested frame and produce a non-black, non-solid RGB565 framebuffer. | The configured startup run produced meaningful video output. | That input works or that the program progressed beyond the observed screen. |
| L2 Input | Replay versioned input and match every expected framebuffer checkpoint while differing from no-input controls. | The configured input caused the named, reviewed interaction. | Extended gameplay, audio correctness, save correctness, or completion. |

Later levels are planned for audio, save data, extended playability, and full
completion. They must not be inferred from an L2 result.

## Why L2 Needs a No-Input Control

An animated title screen changes over time even when no button is pressed. A
test that only compares an early screenshot with a later screenshot can
therefore pass while guest input is completely broken.

Each L2 checkpoint records two expected values:

- `expected_framebuffer_crc32`: the framebuffer after scripted input;
- `control_framebuffer_crc32`: the framebuffer at the same completed frame
  without input.

The checkpoint passes only when the actual raw RGB565 CRC32 equals the expected
value and differs from the control value. The screenshot must also be reviewed
to confirm that the difference represents the named interaction rather than an
unrelated animation.

## External Game Fixtures

Game files are not distributed in this repository. The entire `tmp` directory
is ignored, and users must provide their own legally obtained `.app` files
under `tmp/dingoo_game`.

Files under `compatibility/l2-input` are external-fixture manifests, not game
resources. They intentionally contain only:

- the expected relative path and file name;
- the SHA-256 of the exact tested content version;
- the deterministic frame count and button events;
- reviewed framebuffer checkpoints and no-input controls.

Keeping these manifests in version control makes compatibility claims
reviewable and prevents them from depending on hidden machine-local settings.
The manifests cannot run without matching external content, and a file with a
different SHA-256 is rejected even if its name is identical.

The game-level batch suite is therefore not a self-contained public CI suite.
It is a reproducible local suite for contributors who possess the exact legal
fixtures. Unit tests that do not require game data continue to run normally in
public CI.

## Repository Layout

```text
compatibility/l2-input/       Versioned L2 external-fixture manifests
tmp/dingoo_game/              User-provided game files; ignored by Git
tmp/hle-reports/              Default generated reports; ignored by Git
docs/images/                  Published screenshots from verified runs
scripts/batch-screenshots.ps1 Batch runner and L0/L1/L2 grader
```

## Running the Batch Suite

Place game files below `tmp/dingoo_game`, preserving any subdirectories used by
the scenario `relative_path` values, then run:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1
```

The runner builds the latest Release standalone binary when `-Binary` is not
provided. Useful options include:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1 `
  -Binary target/release/dingoo-emu.exe `
  -ReportDirectory tmp/compatibility-run `
  -TimeoutSeconds 120
```

The default `compatibility/l2-input` directory is loaded automatically. A
different scenario set can be selected explicitly:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1 `
  -InputScenarioDirectory tmp/experimental-scenarios `
  -ReportDirectory tmp/experimental-run
```

Games without a matching scenario are graded at L0/L1 and reported as
`not_tested` at L2. Every configured scenario must match a discovered game;
unused scenarios make the batch command fail.

For stricter SDK coverage, run with the unknown-HLE stop policy:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1 `
  -UnknownHlePolicy stop `
  -ReportDirectory tmp/compatibility-strict
```

An exception must name the exact reviewed function:

```powershell
-AllowUnknownHle legacy_function
```

Strict HLE validation and L2 validate different properties. A game can pass
its scripted input scenario in report mode while still recording unsupported
SDK calls. Review both the level result and `unknown_hle` evidence.

## Generated Evidence

The report directory contains:

- one diagnostic JSON file per game;
- `summary.json` with nested evidence for the entire run;
- `summary.csv` for filtering and comparison;
- failed or unpublished screenshots when a capture was produced.

Each summary records the Git commit and dirty state, binary/content/screenshot
hashes, platform and configuration, duration, process result, frame and
instruction counts, RGB565 metrics, log tail, unknown HLE calls, and optional
input evidence.

The important L2 fields are:

```json
{
  "levels": {
    "highest": "L2",
    "l2": {
      "status": "pass",
      "reason": "scripted_checkpoints_matched"
    }
  },
  "input": {
    "event_count": 2,
    "nonzero_input_frames": 3,
    "all_checkpoints_passed": true,
    "checkpoints": [
      {
        "name": "stopwatch-started",
        "expected_framebuffer_crc32": "cdbd9a8f",
        "control_framebuffer_crc32": "9552ed19",
        "actual_framebuffer_crc32": "cdbd9a8f",
        "differs_from_control": true,
        "status": "pass"
      }
    ]
  }
}
```

Screenshots are first written into the report directory. A screenshot is copied
to `docs/images` only after the game's configured L1 or L2 requirement passes.
A runtime or checkpoint failure therefore cannot overwrite the previous
verified documentation image.

## Input Scenario Format

An input scenario is a JSON document with schema version 1:

```json
{
  "schema_version": 1,
  "content": "game.app",
  "relative_path": "game.app",
  "content_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "frames": 180,
  "events": [
    { "frame": 60, "buttons": ["a"] },
    { "frame": 63, "buttons": [] }
  ],
  "checkpoints": [
    {
      "name": "menu-opened",
      "frame": 180,
      "expected_framebuffer_crc32": "12345678",
      "control_framebuffer_crc32": "90abcdef"
    }
  ]
}
```

### Timing Semantics

- Event frames are zero-based.
- Checkpoint frames are one-based completed-frame counts.
- An event replaces the complete held-button state until the next event.
- An empty `buttons` array releases every button.
- Event and checkpoint frames must be strictly increasing and within the run.
- The scenario `frames` value must equal the CLI headless or screenshot frame
  count.

Supported button names are:

```text
up down left right a b x y start select l r
```

The parser rejects unknown or duplicate buttons, unsupported schema versions,
invalid hashes, duplicate checkpoint names, and malformed ordering before the
emulation run begins.

## Creating a New L2 Scenario

### 1. Choose a Meaningful State

Select a deterministic interaction such as moving a menu selection, entering a
main menu, moving a board cursor, confirming a difficulty, or starting a
timer. Do not use arbitrary framebuffer change as the intended outcome.

Prefer a stable screen reached in a reasonable number of frames. Remove or
isolate existing save files if they alter the startup state.

### 2. Record the Content Identity

```powershell
(Get-FileHash -LiteralPath "tmp/dingoo_game/game.app" -Algorithm SHA256).Hash.ToLowerInvariant()
```

Use that exact value for `content_sha256`.

### 3. Capture the No-Input Control

```powershell
cargo build --release -p dingooemu

.\target\release\dingoo-emu.exe `
  "tmp\dingoo_game\game.app" `
  --screenshot "tmp\control.png" `
  --screenshot-frames 180 `
  --hle-report "tmp\control.json"
```

Inspect `tmp/control.png`, then convert the final raw framebuffer CRC32 to its
eight-character hexadecimal representation:

```powershell
$control = Get-Content -Raw "tmp\control.json" | ConvertFrom-Json
"{0:x8}" -f [uint32]$control.framebuffer.crc32_rgb565
```

### 4. Create a Provisional Script

Create an ignored scenario under `tmp` first. Set
`control_framebuffer_crc32` to the measured control value and use a different
valid placeholder such as `00000000` for `expected_framebuffer_crc32`.

Choose short button holds that include explicit release events. Avoid relying
on host keyboard timing or synthetic key repeat.

### 5. Run the Scripted Probe

```powershell
.\target\release\dingoo-emu.exe `
  "tmp\dingoo_game\game.app" `
  --screenshot "tmp\scripted.png" `
  --screenshot-frames 180 `
  --input-script "tmp\scenario.json" `
  --hle-report "tmp\scripted.json"
```

The process can complete while the provisional checkpoint reports `fail`;
this is expected until the actual CRC32 is recorded.

### 6. Review the Result

Open `tmp/scripted.png` and compare it with `tmp/control.png`. Confirm that the
scripted image represents the intended named state. A different CRC32 without a
meaningful visual or semantic difference is not acceptable L2 evidence.

Read the actual value:

```powershell
$scripted = Get-Content -Raw "tmp\scripted.json" | ConvertFrom-Json
$scripted.input.checkpoints |
  Select-Object name, expected_framebuffer_crc32,
    control_framebuffer_crc32, actual_framebuffer_crc32,
    differs_from_control, status
```

Copy `actual_framebuffer_crc32` into the final
`expected_framebuffer_crc32`, choose an accurate checkpoint name, and move the
reviewed scenario to `compatibility/l2-input`.

### 7. Run Positive and Negative Validation

Run the complete batch and require the new scenario to pass. Then perform a
temporary negative run with an intentionally incorrect expected CRC32. The
negative run must:

- report the affected game as L2 `fail`;
- use `framebuffer_checkpoint_mismatch` as the reason;
- exit with a nonzero status;
- leave the previous `docs/images` screenshot unchanged.

Do not commit the intentionally incorrect scenario.

## L2 Pass Conditions

The batch runner requires all of the following:

1. The scenario path matches a discovered game.
2. The content name and SHA-256 match.
3. The run and scenario frame counts match.
4. L0 and L1 pass.
5. The diagnostic scenario metadata matches the source manifest.
6. At least one frame contains nonzero guest input.
7. Every configured checkpoint is recorded.
8. Every actual CRC32 equals its expected CRC32.
9. Every actual CRC32 differs from its no-input control.
10. No scenario is left unmatched.

Only then is `levels.highest` set to `L2`.

## Common Failure Reasons

| Reason | Meaning |
|---|---|
| `no_input_scenario` | No L2 manifest applies; this is `not_tested`, not a failure. |
| `scenario_content_mismatch` | The manifest file name does not match the discovered content. |
| `scenario_content_hash_mismatch` | The local game is not the exact tested version. |
| `l1_failed` | Startup execution or framebuffer validation failed before L2. |
| `missing_input_diagnostics` | The frontend did not emit input evidence. |
| `input_diagnostics_mismatch` | Runtime metadata differs from the source scenario. |
| `no_nonzero_input_frames` | The scenario never held a guest button. |
| `incomplete_input_checkpoints` | The run ended before every checkpoint was recorded. |
| `framebuffer_checkpoint_mismatch` | The actual CRC32 missed the expected value or matched the no-input control. |
| `screenshot_publish_failed` | The verified screenshot could not be copied to `docs/images`. |

Inspect the process result, log tail, input checkpoints, framebuffer metrics,
and unknown HLE list together when diagnosing a failure.

## Determinism and Maintenance

Exact full-frame CRC32 is intentionally strict. It catches rendering and timing
regressions, but it is unsuitable when a checkpoint includes uncontrolled
randomness, a real-time clock, nondeterministic audio visualization, or
save-dependent content.

When repeated identical runs produce different CRC32 values:

- do not add or update the scenario merely to make it pass;
- identify whether time, random state, saves, scheduling, or incomplete HLE is
  responsible;
- choose a more stable semantic checkpoint or add a better diagnostic method;
- keep the game at L1 if trustworthy L2 evidence is not yet available.

When an emulator change causes an existing checkpoint to fail, compare the old
and new screenshots and controls before updating any expected value. Update a
manifest only when the new state is understood and correct. An unexplained CRC
change is a regression signal, not baseline maintenance.

There is no fixed limit on the number of L2 scenarios. Additional games should
be added incrementally after the same positive, control, visual, and negative
validation. Coverage count must never take priority over evidence quality.
