# Contributing to DingooEmu

Thank you for your interest in contributing!

## Getting Started

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request

## Development Setup

```bash
git clone https://github.com/your-username/DingooEmu.git
cd DingooEmu
cargo build
```

## Code Style

- Use English for all comments and documentation
- Follow Rust API Guidelines
- Use `cargo fmt` before committing
- Use `cargo clippy` to check for warnings

## Testing

```bash
cargo test --workspace
```

For compatibility changes, run the sample collection and retain the generated
unknown-HLE diagnostics:

```powershell
pwsh -NoProfile -File scripts/batch-screenshots.ps1
```

The command writes screenshots to `docs/images` and one JSON report per game to
`tmp/hle-reports`. Review each report's `unknown_hle` list instead of treating a
non-empty screenshot as sufficient proof. Before declaring a representative
sample clean, rerun it with `-UnknownHlePolicy stop`; any exception must use an
explicit, reviewed `-AllowUnknownHle` name.

## Areas Where Help Is Needed

We welcome contributions in the following areas:

- **Game compatibility testing** — Test `.app` files and report results
- **MIPS instruction implementation** — Complete the CPU interpreter
- **SDK HLE functions** — Implement missing Dingoo SDK calls
- **Platform porting** — Improve support for macOS, Android, iOS
- **Documentation** — Improve guides, add examples, fix typos
- **Bug reports** — Report issues with specific games or features
- **RetroArch integration** — Improve the libretro core

Check the [good first issue](https://github.com/jiangxincode/DingooEmu/labels/good%20first%20issue)
and [help wanted](https://github.com/jiangxincode/DingooEmu/labels/help%20wanted)
labels for beginner-friendly tasks.

## License

By contributing, you agree that your contributions will be licensed under the BSD-3-Clause License.
