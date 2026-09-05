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

## Areas Where Help Is Needed

We welcome contributions in the following areas:

- **Game compatibility testing** — Test `.app`, `.cc`, `.c2s`, and `.c3s` files and report results
- **CPU instruction implementation** — Complete the MIPS32 and ARM32/Thumb interpreters
- **SDK HLE functions** — Implement missing A320 and A330 SDK calls
- **Platform porting** — Improve support for macOS, Android, iOS
- **Documentation** — Improve guides, add examples, fix typos
- **Bug reports** — Report issues with specific games or features
- **RetroArch integration** — Improve the libretro core

Check the [good first issue](https://github.com/AloysHF/DingooEmu/labels/good%20first%20issue)
and [help wanted](https://github.com/AloysHF/DingooEmu/labels/help%20wanted)
labels for beginner-friendly tasks.

## License

By contributing, you agree that your contributions will be licensed under the BSD-3-Clause License.
