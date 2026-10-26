# Audit Archive

This directory contains audits, reverse-engineering examinations, and performance benchmarks for `usque-rs-openwrt`.

## Available Audits

- **[2026-10-25 Cloudflare Official WARP Client vs usque-rs](2026-10-25-warp-official-vs-usque/)**:
  - **[Reviewing Agent Handoff Guide](2026-10-25-warp-official-vs-usque/HANDOFF.md)**: Entry point for reviewing agents, mapping all evidence files and outlining candidate improvements.
  - **[Audit Report](2026-10-25-warp-official-vs-usque/REPORT.md)**: Full 310-line comparative analysis covering wire protocol parity, QUIC transport parameters, CC algorithm variants, and architectural differences.
  - **[Evidence & Disassembly](2026-10-25-warp-official-vs-usque/evidence/)**: Targeted `llvm-objdump` disassemblies, symbol extractions, debian package inventory, and upstream source snapshots.
