# usque-rs (OpenWrt Hardened Fork)

[![CI](https://github.com/athcodes2005/usque-rs-openwrt/actions/workflows/ci.yml/badge.svg)](https://github.com/athcodes2005/usque-rs-openwrt/actions/workflows/ci.yml)
[![Release](https://github.com/athcodes2005/usque-rs-openwrt/actions/workflows/release.yml/badge.svg)](https://github.com/athcodes2005/usque-rs-openwrt/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE.md)

An OpenWrt-optimized Rust client for Cloudflare WARP using **MASQUE** (`CONNECT-IP` over HTTP/3 and QUIC), forked from [Diniboy1123/usque-rs](https://github.com/Diniboy1123/usque-rs).

## Key Enhancements over Upstream `v0.1.0`

- **QUIC Pacing & Congestion Control (`--congestion-control cubic|bbr|bbr2|reno`):** Honors `quiche` `SendInfo::at` pacing deadlines via a dedicated timer branch in the event loop to eliminate burst loss and congestion-window collapse on shared uplinks.
- **Upload CoDel Queue Management (`--upload-codel <MS>`):** Applies RFC 8289 CoDel sojourn-time dropping at the head of `quiche`'s outgoing datagram FIFO to eliminate upload bufferbloat while preserving BBR2/CUBIC throughput.
- **5 ms Timer Floor & 64-Item Fair Batching:** Prevents zero-duration `quiche` loss-recovery timers from spinning at 100% CPU, and bounds UDP intake, HTTP/3 polling, TUN reads, and QUIC output to 64 items per pass with cooperative continuation.
- **Router Daemon Integration (`--ready-file`, `--syslog`):** Writes a PID readiness file only after `CONNECT-IP` HTTP 200 establishment, emits non-blocking syslog events to `/dev/log` (`logread -e usque-rs`), enforces a 90 s dead-peer idle timeout, and unlinks temporary PEM files immediately after load.
- **Pre-Built Static `musl` Releases:** Automated builds for `aarch64`, `x86_64`, `armv7`, `arm`, `armv5te`, and `mipsel` on [GitHub Releases](https://github.com/athcodes2005/usque-rs-openwrt/releases).

## Documentation (`wiki-llm`)

All architecture, QUIC pacing/congestion-control details, CoDel upload queue management, and build/release documentation live in **[`wiki-llm/wiki/index.md`](wiki-llm/wiki/index.md)**:

- **Overview & CLI Options:** [`wiki-llm/wiki/overview.md`](wiki-llm/wiki/overview.md) · [`wiki-llm/wiki/architecture.md`](wiki-llm/wiki/architecture.md)
- **QUIC Pacing & Upload CoDel:** [`wiki-llm/wiki/systems/quic-and-pacing.md`](wiki-llm/wiki/systems/quic-and-pacing.md) · [`wiki-llm/wiki/systems/upload-codel.md`](wiki-llm/wiki/systems/upload-codel.md)
- **Builds & Releases:** [`wiki-llm/wiki/operations/build-and-release.md`](wiki-llm/wiki/operations/build-and-release.md)
- **Audits & Reverse Engineering:** [`audit/README.md`](audit/README.md) · [`audit/2026-10-25-warp-official-vs-usque/HANDOFF.md`](audit/2026-10-25-warp-official-vs-usque/HANDOFF.md)
