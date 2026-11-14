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
- **`.github/workflows/release.yml`:** runs only when pushing a version tag (`v*.*.*`) or via `workflow_dispatch` (there is no `release` branch trigger). A `test` job runs first and gates the rest via `needs:`: it fails if the tag (minus the leading `v`) differs from the `Cargo.toml` package version, then runs `cargo clippy --all-targets -- -D warnings` and `cargo test --locked`. Asset names and the `.sha256` format are unchanged because `router-script` depends on them.
- The `build` jobs cross-compile static musl binaries with `--locked` (`aarch64`, `x86_64`, `armv7`, `arm`, `armv5te`, `mipsel`), strip symbols, and hand a versioned archive and a stable-named copy (`usque-rs-aarch64-linux-musl.tar.gz`), each with its `.sha256`, to one `publish` job. Only that job has `contents: write`, and it alone creates the release, so six jobs no longer race to create it. It publishes what was built even when one target failed.
- A manual run takes an existing tag and checks out, verifies and names its archives after that tag, not the branch it was started from. Rebuilding a tag replaces its assets with new bytes, which `router-script`'s pinned sha256 then refuses.
- Third-party actions are pinned by commit in both workflows, with the version in a trailing comment; bump the two together. `Cross.toml` installs what `boring-sys` needs to build BoringSSL in the cross images (cmake, a C++ compiler, libclang); Go is no longer needed. x86_64 builds in a newer cross image pinned by digest, because the 0.2.5 image's GCC 9 crashes on BoringSSL's AVX2 Keccak code.
- A push to `main` that changes `Cargo.toml`, `Cargo.lock`, `Cross.toml` or `release.yml` runs the release workflow's test and build jobs for all six targets without publishing, so a target that stopped building shows before a tag is cut.
- `Cargo.toml` declares `rust-version` (the newest toolchain a locked dependency needs); clippy takes its MSRV from there.

## Handoff to `router-script` for deployment and runtime flags

Once a release artifact is published from `usque-rs-openwrt`, all router-side rollout, `--mtu` / TCP MSS clamping adjustments, and live verification belong to the `router-script` repository. Always ask the user to delegate deployment and router-script changes to the `router-script` agent and generate a tailored prompt for it.

## Sources

- `../../../.github/workflows/ci.yml`
- `../../../.github/workflows/release.yml`
- `../../../Cross.toml`
- `../../../Cargo.toml`
