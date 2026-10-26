# Wiki and engineering log

Append new entries at the end.

## 2026-10-26 — Forked `Diniboy1123/usque-rs` and imported OpenWrt optimizations

- Forked `Diniboy1123/usque-rs` (`v0.1.0`) to `athcodes2005/usque-rs-openwrt` with `upstream` tracking `https://github.com/Diniboy1123/usque-rs.git`.
- Imported all OpenWrt/router optimizations and reliability fixes:
  - Buffer reuse for UDP datagram reception and 1 Hz QUIC statistics sampling.
  - `--congestion-control cubic|bbr|bbr2|reno` CLI flag.
  - `SendInfo::at` outbound packet pacing timer branch.
  - 5 ms timer floor for expired `quiche` timeouts.
  - 64-item bounded stage batching and cooperative continuation.
  - Atomic `0600` `config.json` writes, mandatory peer SPKI pinning, and immediate PEM temp-file deletion after load.
  - `--ready-file <PATH>` PID readiness marker and `--syslog` `/dev/log` event logging.
  - 90 s dead-peer idle timeout, 32-error TUN write failure threshold, and session-duration-aware reconnect backoff.
  - `--upload-codel <MS>` CoDel queue-delay management and `--tun-backpressure <N>`.
  - Full IPv4 options-aware header validation/checksumming and production `src/pipeline_tests.rs` test suite (94 unit + 4 integration tests).
- Set up `wiki-llm/` and updated `.github/workflows/release.yml` to publish verified static `aarch64-unknown-linux-musl` assets on `v*.*.*` tags and the `release` branch.
- Fixed Rust 1.99 Clippy lints (`const fn ip_version`, `write_tun_packet` shared return), bumped crate metadata to `0.2.0`, and archived the official Cloudflare WARP (`warp-svc` 2026.8.2115.0) vs `usque-rs` comparative reverse-engineering audit under `audit/2026-10-25-warp-official-vs-usque/`.

## Sources

- `../../src/`, `../../tests/tunnel_mtu.rs`
- `../../.github/workflows/ci.yml`, `../../.github/workflows/release.yml`
- `../../audit/2026-10-25-warp-official-vs-usque/HANDOFF.md`
