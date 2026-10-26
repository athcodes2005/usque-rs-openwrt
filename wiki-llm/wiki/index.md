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

## Operations

- [[operations/build-and-release]] — reproducible musl cross-compilation, CI, and GitHub Release artifacts.

## Bugs and performance fixes

- [[bugs/quic-timer-floor]] — 5 ms floor on expired quiche timers to prevent 100% CPU core spin.
- [[bugs/forwarding-fairness]] — 64-item stage batching and cooperative continuation to prevent UDP intake starvation.

## Sources

- All pages linked above.
- `../../README.md`
- `../../Cargo.toml`
