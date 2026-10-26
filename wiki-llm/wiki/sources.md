# Source inventory

The repository root is the canonical Rust crate.

## Implementation files

- `Cargo.toml`, `Cargo.lock`: crate metadata and locked dependency graph (`quiche` 0.22 with vendored BoringSSL).
- `Cross.toml`, `.cargo/config.toml`: cross-compilation targets and flags.
- `clippy.toml`, `rustfmt.toml`: static analysis and formatting configuration.
- `src/`: all Rust modules (`main.rs`, `config.rs`, `register.rs`, `tls.rs`, `tun_device.rs`, `tunnel.rs`, `packet.rs`, `icmp.rs`, `events.rs`, `pipeline_tests.rs`).
- `tests/tunnel_mtu.rs`: Linux TUN integration tests.
- `.github/workflows/ci.yml`: automated format, Clippy, and unit test workflow.
- `.github/workflows/release.yml`: multi-architecture static `musl` binary release builder triggered on `v*.*.*` tags or the `release` branch.

## Imported raw sources

- `../raw/sources/upstream-readme.md`: original upstream README from `Diniboy1123/usque-rs`.
- `../raw/sources/upstream-setup-notes.md`: original upstream OpenWrt IPv6 setup notes.

## Sources

- `../../Cargo.toml`
- `../../.github/workflows/ci.yml`
- `../../.github/workflows/release.yml`
