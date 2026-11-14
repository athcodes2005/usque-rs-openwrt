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
- Documented the `router-script` delegation contract in [[overview]] and [[operations/build-and-release]]: any router deployment, binary rollout, or TUN MTU / TCP MSS clamping change must be delegated via a handoff prompt to the agent working in the `router-script` repository.
- Implemented `[I3]` batch outbound UDP transmission (`UdpSendBatch` + Linux `sendmmsg(2)`), sub-millisecond pacing coalescing (`PACING_COALESCE_WINDOW = 500 µs` to prevent Tokio 1 ms timer-wheel stalls), pre-allocated `paced_buf` reuse, and 64-packet non-blocking TUN burst drains (`try_read_tun_packet`) in commit `2323f29` (101 unit tests + 4 integration tests).
- Fixed musl cross-compilation in `756ec94` (explicit `libc = "0.2"` dependency and `libc::MSG_DONTWAIT as _`) and published `v0.2.1` (`7e8e54a…` archive SHA-256). Live router verification (`measure_load_latency.sh` with `--mtu 1320 --congestion-control bbr2`) measured `+42%` upload throughput (`93.6` $\to$ `133.3 Mbit/s`), `-59%` median upload loaded RTT (`46 ms` $\to$ `19 ms`), and `0 ms` median download bloat (`5 ms` under load = `5 ms` idle).
- Ingested `audit/2026-10-25-warp-official-vs-usque/` into `wiki-llm/raw/sources/audit/2026-10-25-warp-official-vs-usque/`, synthesized [[bugs/warp-official-audit-2026-10-25]], removed the top-level `audit/` directory, and removed `-C target-cpu=native` from `.cargo/config.toml` so `Swatinem/rust-cache` artifacts remain portable across heterogeneous GitHub Actions runner CPUs.

## 2026-10-26 — Validation fixes (C1, C10, C11, D6, D7)

- `release.yml` now has a `test` job (tag vs `Cargo.toml` version check, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked`) that the build/publish jobs need; asset names unchanged.
- Corrected the wiki: CI clippy has no `--all-features`, and releases trigger only on tags and `workflow_dispatch` (no `release` branch).
- Removed the `warp-svc`-derived evidence files from the working tree (history not rewritten); see [[bugs/warp-official-audit-2026-10-25]]. Scrubbed local paths, the private LAN IP, and campus/router-model mentions.

## 2026-10-26 — UDP receive buffer (v0.2.2)

- Added `nativetun --udp-recv-buffer <BYTES>` (default 4 MiB, `0` = kernel default), applied per session on Linux via `SO_RCVBUFFORCE` with an `SO_RCVBUF` fallback; effective size logged once per process. Live A/B: `RcvbufErrors` thousands to 0, download ping loss 10–23% to 0–8%. See [[systems/udp-receive-buffer]]. Version bumped to 0.2.2.

## 2026-11-12 — QUIC timer fixes (v0.2.3)

- The 5 ms timer floor starved `on_timeout()` while the tunnel was busy; the
  session loop now runs an expired timer after each `select!`, at most once per
  5 ms. The handshake and CONNECT-wait loops got the same floor, and the CONNECT
  wait lost its 100-pass cap. See [[bugs/quic-timer-floor]].
- `release.yml`: only the build job holds `contents: write`, and the cross build
  uses `--locked`. Version bumped to 0.2.3 (106 unit + 4 integration tests).

## 2026-11-12 — Dependency refresh and release pipeline hardening (0.2.4)

- `cargo update`: 137 semver-compatible updates (tokio 1.54, rustls 0.23.46,
  libc 0.2.191, among others). Left at their current major versions, each
  needing code changes and a tunnel A/B test: quiche 0.22 (0.30 is out), tun,
  rtnetlink, netlink-packet-route, reqwest, rcgen, x509-parser and the
  RustCrypto crates (p256, der, pkcs8, sec1, pem), base64, rand.
- A UDP send error no longer ends the batch: `sendmmsg` falling over hands the
  batch to the per-packet path, where a failure costs that one packet (C6).
- `rust-version = "1.88"` replaces the unverified 1.75 in `clippy.toml` (C5);
  the lints that exposed are fixed.
- `release.yml`: one `publish` job (C2), the unused bare-binary copy removed
  (C3), manual runs build the tag they name (C12), actions pinned by commit at
  current versions (checkout v7, action-gh-release v3). `Cross.toml` verifies
  the Go download (C4). See [[operations/build-and-release]]. 107 unit + 4
  integration tests on macOS; the reworked workflow has not run yet.

## 2026-11-12 — v0.2.4 released; syslog tag fix (0.2.5)

- v0.2.4 was tagged and built by the reworked workflow on its first run: all six
  targets, published by the one `publish` job (aarch64 archive `52f98ac2…`).
  Deployed to the router; two interleaved load-test rounds against v0.2.3 showed
  no regression.
- Found during that rollout: OpenWrt's `logd` drops the first 15 characters and
  a space of a syslog message as its timestamp, whatever they are. The client
  sent none, and `usque-rs[4845]: ` is exactly that long for a four-digit PID,
  so such a process logged every event without its tag and
  `logread -e usque-rs` showed nothing. Lines now carry an RFC 3164 timestamp
  (`src/events.rs`), confirmed with raw test messages on the router. Version
  0.2.5, 108 unit + 4 integration tests; not tagged yet.

## 2026-11-12 — v0.2.5 released

- v0.2.5 (the syslog timestamp) was tagged and built for all six targets
  (aarch64 archive `42f10f86…`) and deployed to the router, where
  `logread -e usque-rs` shows the client's events with their tag.

## Sources

- `../../src/`, `../../tests/tunnel_mtu.rs`
- `../../.github/workflows/ci.yml`, `../../.github/workflows/release.yml`
- `../raw/sources/audit/2026-10-25-warp-official-vs-usque/HANDOFF.md`

## 2026-11-14 — Major dependency upgrades; version 0.3.0

- Every crate that was a major version behind is current: quiche 0.30,
  reqwest 0.13, tun 0.8, rtnetlink 0.24, rcgen 0.14, x509-parser 0.18, p256
  0.14, der 0.8, pkcs8 0.11, pem 4, base64 0.23. `rand`, `sec1` and
  `netlink-packet-route` are no longer direct dependencies (keys come from the
  system RNG through the `Generate` trait; the other two were never used
  directly). Go is gone from the build: BoringSSL is built by `boring-sys`.
- reqwest 0.13's own TLS choice is aws-lc-rs and the system certificate store.
  The API client keeps ring and the built-in Mozilla roots by handing reqwest
  its own rustls config (`api_tls_config`).
- quiche 0.30 changed sending: `bbr` and `bbr2` are one new BBR2, `cubic` and
  `reno` are unpaced. The session loop's timer-driven pacing starved the new
  pacer; the first router test showed upload at 2.5 and 3.5 Mbit/s against 58
  and 78 for v0.2.5. Fixed by sending up to 3 ms ahead of the schedule
  ([[bugs/bbr2-pacer-oversleep]], [[systems/quic-and-pacing]]); with the fix,
  upload 97 and 71 Mbit/s against 78 and 19, download 145 and 159 against 53
  and 117.
- New tests pin what must survive the libraries: the SubjectPublicKeyInfo sent
  to the API, a private key written by an earlier release still producing a
  client certificate quiche loads, the API client accepting the TLS config (and,
  ignored by default, a live handshake), a real TUN device created and
  configured over netlink (root only), and the pacing rule against a model of
  the pacer. 115 unit tests on macOS, 118 on Linux as root, 4 integration.
- Both test binaries ran on the router: the tunnel came up, `tun0` got its MTU
  and both addresses over rtnetlink 0.24, and the trace showed `warp=on` on IPv4
  and IPv6. The second ran as PID 2064 and logged with its tag, the four-digit
  case of the v0.2.5 syslog fix that had not been seen live.
- The aarch64 binary grows from 6.0 to 7.4 MB.
- Built locally in the cross images: aarch64 in cross 0.2.5's image; x86_64
  only in the current one (GCC 14), since 0.2.5's GCC 9 crashes on BoringSSL's
  AVX2 Keccak, so `Cross.toml` pins that image by digest. The four 32-bit
  targets are left to the release workflow's new build on `main`.

## Sources

- `../../Cargo.toml`, `../../Cross.toml`, `../../src/tunnel.rs`,
  `../../src/register.rs`, `../../src/tls.rs`, `../../src/tun_device.rs`
- [[bugs/bbr2-pacer-oversleep]], [[systems/quic-and-pacing]],
  [[operations/build-and-release]]

## 2026-11-14 — v0.3.0 released and deployed

- v0.3.0 was tagged and built for all six targets (aarch64 archive
  `2f87783e…`) and deployed to the router through router-script's updater.
  Interleaved against v0.2.5 there: upload 147.5 and 149.2 Mbit/s against 117.6
  and 14.5, download 131.0 and 111.0 against 118.3 and 150.0. The pacing fix
  holds in the published build.
