# Build and release pipeline

## Reproducible OpenWrt ARM64 build

To build a static `aarch64-unknown-linux-musl` binary locally using the pinned cross container:

```sh
docker run --rm \
  -v "$PWD":/home/rust/src:ro \
  -v usque-cargo-target:/tmp/target \
  -v usque-cargo-registry:/root/.cargo/registry \
  -w /home/rust/src \
  -e CARGO_TARGET_DIR=/tmp/target \
  messense/rust-musl-cross:aarch64-musl \
  cargo build --release --locked --target aarch64-unknown-linux-musl
```

## GitHub Actions CI and Release

- **`.github/workflows/ci.yml`:** runs `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all` on every push and pull request.
- **`.github/workflows/release.yml`:** runs when pushing a version tag (`v*.*.*`) or pushing to the `release` branch. It cross-compiles static musl binaries (`aarch64`, `x86_64`, `armv7`, `arm`, `armv5te`, `mipsel`), strips symbols, and publishes both versioned archives and stable asset names (`usque-rs-aarch64-linux-musl`, `usque-rs-aarch64-linux-musl.sha256`, `usque-rs-aarch64-linux-musl.tar.gz`) to GitHub Releases so `router-script` can fetch and verify them automatically.

## Sources

- `../../../.github/workflows/ci.yml`
- `../../../.github/workflows/release.yml`
- `../../../Cross.toml`
- `../../../Cargo.toml`
