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

## 2026-10-26 — WARP (`warp-svc` 2026.8.2115.0) audit optimizations (`c4be176`)

- Implemented prioritized performance and resilience improvements from the reverse-engineering audit of Cloudflare's official `warp-svc` (`2026.8.2115.0`) in commit `c4be176`:
  - **`[I1]` Asymmetric QUIC datagram queues (`src/tunnel.rs`):** decoupled `enable_dgram(true, 1000, 1000)` into `DGRAM_RECV_QUEUE_LEN = 8192` and `DGRAM_SEND_QUEUE_LEN = 1000`. Keeps the send queue small and CoDel-managed (`--upload-codel` / `--tun-backpressure`) to avoid the upload bufferbloat (`F10`) that `warp-svc`'s unmanaged `65,536` send queue would cause on router SoCs, while expanding the receive queue 8× so download bursts during TUN write stalls do not silently drop RFC 9221 datagrams.
  - **`[I2]` QUIC 0-RTT & TLS 1.3 session resumption (`src/tunnel.rs`):** enabled `Config::enable_early_data()`, cached `Connection::session()` tickets across reconnects after peer SPKI verification and `CONNECT-IP` readiness, and offered them via `Connection::set_session()` so HTTP/3 `CONNECT-IP` requests can be sent in 0-RTT early data with automatic fallback to a full handshake on rejection or expiry.
  - **`[I4]` Dynamic writable datagram IP MTU & `quiche` 0.22.0 client DPLPMTUD guard (`src/tunnel.rs`):** added `effective_datagram_ip_mtu(conn.dgram_max_writable_len(), flow_prefix.len())` so ICMP Packet Too Big / Fragmentation Needed responses advertise the negotiated ~1,320 B datagram IP payload ceiling instead of a hardcoded 1,280 B. Explicitly kept `Config::discover_pmtu(false)` on the client after uncovering an upstream `quiche` 0.22.0 bug (`src/path.rs:339` checks `hs_confirmed && hs_done` where `handshake_done_sent` is server-only, which wedges `dgram_send_queue` on clients for packets > 1,157 B).
  - Target-gated Linux-only netlink dependencies (`rtnetlink`, `netlink-packet-route`, `futures`) under `cfg(target_os = "linux")` in `Cargo.toml` and expanded the test suite to 99 unit tests + 4 integration tests.
- Documented the `router-script` delegation contract in [[overview]] and [[operations/build-and-release]]: any router deployment, binary rollout, or TUN MTU / TCP MSS clamping change must be delegated via a handoff prompt to the agent working in `/Users/atharva/projects/router-script`.

## Sources

- `../../src/`, `../../tests/tunnel_mtu.rs`
- `../../.github/workflows/ci.yml`, `../../.github/workflows/release.yml`
- `../../audit/2026-10-25-warp-official-vs-usque/HANDOFF.md`
