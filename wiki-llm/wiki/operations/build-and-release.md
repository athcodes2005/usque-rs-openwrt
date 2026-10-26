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

- **`.github/workflows/ci.yml`:** runs `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` (no `--all-features`), and `cargo test --all` on pushes to and pull requests against `main`.
- **`.github/workflows/release.yml`:** runs only when pushing a version tag (`v*.*.*`) or via `workflow_dispatch` (there is no `release` branch trigger). A `test` job runs first and gates every build/publish job via `needs:`: it fails if the tag (minus the leading `v`) differs from the `Cargo.toml` package version, then runs `cargo clippy --all-targets -- -D warnings` and `cargo test --locked`. Asset names and the `.sha256` format are unchanged because `router-script` depends on them. After the gate it cross-compiles static musl binaries (`aarch64`, `x86_64`, `armv7`, `arm`, `armv5te`, `mipsel`), strips symbols, and publishes both versioned archives and stable asset names (`usque-rs-aarch64-linux-musl.tar.gz`, `usque-rs-aarch64-linux-musl.tar.gz.sha256`) to GitHub Releases so `router-script` can fetch and verify them automatically.

## Handoff to `router-script` for deployment and runtime flags

Once a release artifact is published from `usque-rs-openwrt`, all router-side rollout, `--mtu` / TCP MSS clamping adjustments, and live verification belong to `/Users/atharva/projects/router-script`. Always ask the user to delegate deployment and router-script changes to the `router-script` agent and generate a tailored prompt for it.

## Sources

- `../../../.github/workflows/ci.yml`
- `../../../.github/workflows/release.yml`
- `../../../Cross.toml`
- `../../../Cargo.toml`
