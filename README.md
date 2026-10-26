# usque-rs (OpenWrt Hardened Fork)

An OpenWrt-optimized Rust client for Cloudflare WARP using MASQUE (`CONNECT-IP` over HTTP/3 and QUIC), forked from [Diniboy1123/usque-rs](https://github.com/Diniboy1123/usque-rs).

All architecture, QUIC pacing/congestion-control details, CoDel upload queue management, and build/release documentation live in **[`wiki-llm/wiki/index.md`](wiki-llm/wiki/index.md)**:

- **Overview & CLI Options:** [`wiki-llm/wiki/overview.md`](wiki-llm/wiki/overview.md) · [`wiki-llm/wiki/architecture.md`](wiki-llm/wiki/architecture.md)
- **QUIC Pacing & Upload CoDel:** [`wiki-llm/wiki/systems/quic-and-pacing.md`](wiki-llm/wiki/systems/quic-and-pacing.md) · [`wiki-llm/wiki/systems/upload-codel.md`](wiki-llm/wiki/systems/upload-codel.md)
- **Builds & Releases:** [`wiki-llm/wiki/operations/build-and-release.md`](wiki-llm/wiki/operations/build-and-release.md)
