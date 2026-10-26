# Reviewing Agent Handoff Guide: Cloudflare WARP Audit

**Target:** Official Cloudflare WARP client daemon (`warp-svc` 2026.8.2115.0) vs `usque-rs-openwrt`  
**Date:** 26 October 2026  
**Canonical Directory:** `audit/2026-10-25-warp-official-vs-usque/`  
**Repository:** `usque-rs-openwrt` (migrated after splitting from `router-script`)

---

## 1. Executive Summary & Purpose

This directory contains the complete reverse-engineering audit and comparative analysis of Cloudflare's official desktop WARP daemon (`warp-svc` version `2026.8.2115.0`) against our OpenWrt-optimized MASQUE client (`usque-rs`).

The analysis was performed using **REA (reverse-engineer-anything) 6.8.0**, symbol-table analysis, and targeted `llvm-objdump` disassembly of the official unstripped ELF binary. All evidence is archived locally so that every finding and line citation can be verified without network access.

Start by reading this guide, then proceed to [`REPORT.md`](REPORT.md) for the full 310-line engineering report.

---

## 2. Directory Structure & Evidence Map

```
audit/2026-10-25-warp-official-vs-usque/
├── HANDOFF.md                                # This document (reviewing agent entry point)
├── REPORT.md                                 # Full comparative audit report (W1–W6, I1–I7, anti-recommendations)
└── evidence/
    ├── digests.txt                           # SHA-256 hashes of analyzed packages and binary
    ├── deb-inventory.txt                     # Package file layout and ELF file headers
    ├── quicsettings-default-disasm.txt       # Disassembly of tokio_quiche::settings::quic::QuicSettings::default
    ├── cc-variant-decoder-disasm.txt         # Disassembly of SupportedCongestionControl::algorithm_name
    ├── architecture-symbols.txt              # Filtered symbol extracts (tokio-quiche, MASQUE, leases, racing)
    ├── warp-svc-strings.txt.gz               # Gzipped strings dump of the 96 MB binary
    └── upstream/
        ├── ORIGIN.md                         # Upstream source provenance and line citation index
        ├── quiche-0.22.0-lib.rs              # Upstream quiche 0.22.0 lib.rs (verifies library defaults)
        ├── quiche-0.22.0-recovery-mod.rs     # Upstream quiche 0.22.0 recovery module
        ├── tokio-quiche-config-master.rs     # tokio-quiche config builder reference
        └── tokio-quiche-settings-quic-master.rs # tokio-quiche settings struct layout reference
```

### Detailed Evidence File Catalog

| File | Size | SHA-256 Digest | Primary Significance |
|---|---|---|---|
| [`REPORT.md`](REPORT.md) | 16.8 KB | — | Canonical audit report containing W1–W6, I1–I7, anti-recommendations, and unknowns. |
| [`evidence/digests.txt`](evidence/digests.txt) | 283 B | `474fd033…` (deb)<br>`1b81c512…` (pkg)<br>`40ab147c…` (warp-svc) | Records exact SHA-256 hashes for bit-for-bit reproduction. |
| [`evidence/deb-inventory.txt`](evidence/deb-inventory.txt) | 1.3 KB | — | Shows that `warp-svc` is an unstripped 64-bit ELF with 212,625 symbols and build ID `d5e9f027…`. |
| [`evidence/quicsettings-default-disasm.txt`](evidence/quicsettings-default-disasm.txt) | 4.2 KB | — | Targeted disassembly of `0x2b8bf40`–`0x2b8c13c`. Proves CUBIC (`0x69627563`), 65536 dgram queues (`0x10000`), 10MB max data (`0x989680`), 56s idle timeout (`0x38`), 1350 payload (`0x546`), and 10 initial cwnd (`0xa`). |
| [`evidence/cc-variant-decoder-disasm.txt`](evidence/cc-variant-decoder-disasm.txt) | 3.2 KB | — | Targeted disassembly of `0x3bced60`–`0x3bcee32`. Proves the 3 supported CC algorithms: `cwnd_max`, `cubic`, `bbr2_gcongestion`. |
| [`evidence/architecture-symbols.txt`](evidence/architecture-symbols.txt) | 430 KB | — | Architecture symbol dump showing `MasqueTunnel`, `IoWorker`, `ReadinessReadLease`, protocol racing, and session resumption. |
| [`evidence/warp-svc-strings.txt.gz`](evidence/warp-svc-strings.txt.gz) | 2.5 MB | — | Gzipped text dump of all printable strings in `warp-svc`. |
| [`evidence/upstream/ORIGIN.md`](evidence/upstream/ORIGIN.md) | 2.1 KB | — | Map of upstream GitHub URLs and verified line citations for quiche 0.22 defaults. |

---

## 3. Evidence Classification Taxonomy

Every statement and conclusion across the audit uses this rigorous taxonomy:

- **`[O]` Observed:** Directly verified facts extracted from shipped binary disassembly, symbol tables, package metadata, or our in-tree Rust code.
- **`[I]` Inference:** Engineering explanations or deductions based on observed facts (e.g., explaining why a desktop client can tolerate a 65536 send queue whereas an OpenWrt router cannot).
- **`[U]` Unknown:** Explicitly noted open questions that require dynamic tracing (e.g. LLDB breakpoints on a running daemon contacting Cloudflare servers) or MDM profile testing.

---

## 4. Key Findings Summary (W1–W6)

1. **W1 — Stack Identity `[O]`:** The official client is written in **Rust** (not C/C++ or Go), using tokio and a pinned git checkout of `cloudflare/quiche` (rev `4ec612d`), with BoringSSL linked via the `boring` crate.
2. **W2 — Wire Protocol Parity `[O]`:** Cloudflare's MASQUE mode matches our implementation exactly: `CONNECT-IP`, `:authority cloudflareaccess.com`, SNI `consumer-masque.cloudflareclient.com`, `flow_id = stream_id / 4`.
3. **W3 — Transport Parameters `[O]`:**
   - Both default to **CUBIC**, 1350 B max UDP payload, 10,000,000 B initial max data, 10 initial cwnd packets, pacing on, and HyStart on.
   - Most "large" official settings are simply **quiche 0.22 defaults** which `usque-rs` already inherits!
   - Key divergence: Official client sets dgram send & recv queues to **65,536** (vs our **1,000**), and idle timeout to **56 s** (vs our **90 s**).
4. **W4 — CC Menu `[O]`:** Shipped binary supports `cubic` (default), `bbr2_gcongestion` (BBR2), and `cwnd_max` (unbounded cwnd mode).
5. **W5 — Architecture Parity Gaps `[O]`:** Official client features protocol racing (WireGuard vs MASQUE H3 vs H2), UDP GSO / kernel pacing offload (`SO_TXTIME`), 0-RTT session resumption, adaptive timeouts, and TPM-backed registration.
6. **W6 — Registration Surface `[O]`:** Our client uses the community fake Android registration (`CF-Client-Version: a-6.35-4471`), while official WARP uses desktop registration with MDM posture and version negotiation (`target_client_version`).

---

## 5. Action Items & Improvement Candidates (Ranked)

If you are an agent implementing improvements to `usque-rs-openwrt`, follow this prioritized roadmap:

### 1. [I1] Asymmetric Datagram Queues (Effort: LOW | Impact: HIGH)
- **Problem:** `enable_dgram(true, 1000, 1000)` in `src/tunnel.rs` couples send and recv queues.
- **Action:** Keep the send queue small/CoDel-managed (bufferbloat prevention on upload). Increase the receive queue to **4096–8192** datagrams. Since QUIC datagrams are never retransmitted, a burst that overflows a 1000-deep receive queue drops IP packets permanently.
- **Code:** Update `conn.enable_dgram(...)` in `src/tunnel.rs`.

### 2. [I2] QUIC 0-RTT Session Resumption (Effort: MEDIUM | Impact: HIGH)
- **Problem:** Every tunnel reconnection performs a full 1–2 RTT cryptographic handshake.
- **Action:** quiche 0.22 provides `Connection::session()`, `set_session()`, and `Config::enable_early_data()`. Cache the session token after a successful connection to eliminate cold reconnect delays.
- **Upstream ref:** [`evidence/upstream/quiche-0.22.0-lib.rs`](evidence/upstream/quiche-0.22.0-lib.rs) lines 1009, 2177, 6412.

### 3. [I3] Batch Sends via UDP GSO / `sendmmsg` (Effort: MEDIUM | Impact: HIGH)
- **Problem:** Sending one datagram per `send_to` syscall causes heavy context switching and CPU load on low-power router SoCs.
- **Action:** Batch outgoing paced datagrams using `sendmmsg` (or `sendmsg` + `UDP_SEGMENT` if kernel supports GSO).

### 4. [I4] Path MTU Discovery (PMTUD) (Effort: LOW-MEDIUM | Impact: MEDIUM)
- **Problem:** Fixed 1280 TUN MTU leaves throughput on the table.
- **Action:** Enable `Config::discover_pmtu()` (quiche 0.22 line 971) to probe higher MTUs toward ~1320 bytes, reducing packet count and CPU overhead by ~3–4%.

### Anti-Recommendations (What NOT to do)
- ❌ **Do NOT increase the send queue to 65,536.** Desktop machines can drain deep queues via dedicated worker threads and multi-gigabit uplinks. On an OpenWrt router, an unmanaged 65k datagram queue creates massive upload bufferbloat (F10). Keep upload CoDel active.
- ❌ **Do NOT enable `cwnd_max`.** Bypasses congestion control discipline and destroys fairness on shared links.
- ❌ **Do NOT switch to BBR2 blindly by default.** The official client leaves CUBIC as default and keeps BBR2 as an MDM opt-in. Keep `--congestion-control bbr2` selectable via CLI.

---

## 6. How to Verify Findings Locally

You can re-verify the disassembly and symbol extractions using the commands below:

```bash
# 1. Verify SHA-256 digests
cd audit/2026-10-25-warp-official-vs-usque
shasum -a 256 -c evidence/digests.txt

# 2. Inspect QuicSettings::default disassembly (matches evidence/quicsettings-default-disasm.txt)
# If warp-svc is extracted in ~/.hermes/cache/scratch/warp-analysis/deb-root/bin/warp-svc:
llvm-objdump -d --start-address=0x2b8bf40 --stop-address=0x2b8c13c --no-show-raw-insn \
  ~/.hermes/cache/scratch/warp-analysis/deb-root/bin/warp-svc

# 3. Inspect CC algorithm selector disassembly (matches evidence/cc-variant-decoder-disasm.txt)
llvm-objdump -d --start-address=0x3bced60 --stop-address=0x3bcee32 --no-show-raw-insn \
  ~/.hermes/cache/scratch/warp-analysis/deb-root/bin/warp-svc
```

---

## 7. Next Steps for Reviewer

1. Review [`REPORT.md`](REPORT.md) for full context on each parameter and architectural component.
2. Evaluate candidate **I1** (asymmetric dgram queue depths) for immediate patch in `src/tunnel.rs`.
3. If investigating reconnection improvements, inspect **I2** against quiche 0.22 APIs in `evidence/upstream/quiche-0.22.0-lib.rs`.
