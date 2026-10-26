# usque-rs-openwrt wiki

Start with [[overview]] for purpose and CLI contract, then [[architecture]] for
the Rust module layout and QUIC/MASQUE session lifecycle.

## Core

- [[overview]] — crate goals, CLI flags, and OpenWrt integration boundary.
- [[architecture]] — modules, registration, TLS SPKI pinning, and session loop.
- [[sources]] — source inventory, build configurations, and test suites.
- [[log]] — append-only wiki and engineering history.

## Systems

- [[systems/quic-and-pacing]] — QUIC congestion control (`cubic`, `bbr`, `bbr2`, `reno`), `SendInfo::at` pacing, and keepalives.
- [[systems/packet-and-icmp]] — IPv4/IPv6 validation, options-aware checksums, and ICMP PMTUD generation.
- [[systems/upload-codel]] — CoDel queue-delay management on quiche's outgoing datagram FIFO and TUN backpressure.
- [[systems/udp-receive-buffer]] — `--udp-recv-buffer` (default 4 MiB) on the QUIC UDP socket to stop kernel `RcvbufErrors` under download bursts.

## Operations

- [[operations/build-and-release]] — reproducible musl cross-compilation, CI, and GitHub Release artifacts.

## Bugs and performance fixes

- [[bugs/quic-timer-floor]] — 5 ms floor on expired quiche timers to prevent 100% CPU core spin.
- [[bugs/forwarding-fairness]] — 64-item stage batching and cooperative continuation to prevent UDP intake starvation.
- [[bugs/warp-official-audit-2026-10-25]] — reverse-engineering audit of official Cloudflare WARP (`warp-svc` 2026.8.2115.0) vs `usque-rs` and `v0.2.0`/`v0.2.1` optimizations.

## Sources

- All pages linked above.
- `../../README.md`
- `../../Cargo.toml`
