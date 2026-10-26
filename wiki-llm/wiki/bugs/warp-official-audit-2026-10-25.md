# Official Cloudflare WARP (`warp-svc` 2026.8.2115.0) audit and `v0.2.1` optimizations

A reverse-engineering audit of Cloudflare's unstripped Linux daemon (`warp-svc` `2026.8.2115.0`, SHA-256 `40ab147c…`) compared its internal `tokio-quiche` / `quiche` (`rev 4ec612d`) MASQUE stack against `usque-rs-openwrt`.

## Key audit findings (`W1`–`W6`)

- **Wire parity (`W1`, `W2`):** `warp-svc` is written in Rust on `tokio` + `quiche` + BoringSSL and speaks identical MASQUE `CONNECT-IP` headers (`:protocol cf-connect-ip`, `:authority cloudflareaccess.com`, SNI `consumer-masque.cloudflareclient.com`, `flow_id = stream_id / 4`, context `0`).
- **Transport parameters (`W3`, `W4`):** Both clients default to `cubic`, `1,350 B` max UDP payload, `10 MB` `initial_max_data`, `1 MB` stream windows, `10`-packet initial `cwnd`, and `quiche`'s `24 MiB` / `16 MiB` max flow-control windows. `warp-svc` sets both datagram queues to `65,536` (`0x10000`) and idle timeout to `56 s`.
- **Architecture gaps (`W5`):** `warp-svc` implements 0-RTT session resumption (`InitialQuicConnection::resume`), batched UDP GSO / `sendmmsg` I/O, and kernel pacing offload (`SO_TXTIME`).

## Implemented optimizations (`v0.2.0`–`v0.2.1`)

1. **`[I1]` Asymmetric datagram queues (`src/tunnel.rs`):** decoupled `enable_dgram(true, 1000, 1000)` into `DGRAM_RECV_QUEUE_LEN = 8192` and `DGRAM_SEND_QUEUE_LEN = 1000` ([[../systems/quic-and-pacing]], [[../systems/upload-codel]]). Absorbs download bursts without silent RFC 9221 drops while keeping the upload queue small and CoDel-managed.
2. **`[I2]` QUIC 0-RTT & TLS 1.3 session resumption (`src/tunnel.rs`):** enabled `Config::enable_early_data()` and cached `Connection::session()` tickets across reconnects (`restore_quic_session`), cutting reconnect handshake + `CONNECT-IP` setup from 2 RTTs to 1 RTT and reducing handshake CPU time by ~49% (`208.1 µs` $\to$ `107.0 µs`).
3. **`[I3]` `sendmmsg(2)` UDP batching, sub-ms pacing coalescing, and TUN burst drains (`src/tunnel.rs`):** added `UdpSendBatch` (`send_batch_mmsg` on Linux), coalesced sub-millisecond pacing gaps (`PACING_COALESCE_WINDOW = 500 µs`) to prevent Tokio's 1 ms timer wheel from throttling `conn.send()`, reused a pre-allocated `paced_buf`, and drained up to 64 ready TUN packets per wakeup (`try_read_tun_packet`).
4. **`[I4]` Dynamic 1,320 B datagram IP MTU (`src/tunnel.rs`):** added `effective_datagram_ip_mtu` to advertise the negotiated `1,320 B` datagram IP payload ceiling (`1,322 B` minus 2-byte MASQUE prefix) via ICMP PMTUD ([[../systems/packet-and-icmp]]). Kept `Config::discover_pmtu(false)` on the client to avoid an upstream `quiche` 0.22.0 bug (`src/path.rs:339` checks `hs_confirmed && hs_done` where `handshake_done_sent` is server-only, wedging `dgram_send_queue` on clients for packets `> 1,157 B`).

## Anti-recommendations rejected

- **Do not copy `warp-svc`'s `65,536` send queue:** ~88 MiB of unmanaged client buffering causes severe upload bufferbloat on router CPUs (`F10`).
- **Do not enable `cwnd_max`:** disables congestion control on shared links.

## Live router benchmark results (GL-MT3000, `bbr2`, `MTU 1320`)

| Metric | `v0.2.0` (per-packet `send_to`) | `v0.2.1` (`sendmmsg` + `500 µs` pacing coalescing) | Delta |
|---|---|---|---|
| Idle RTT (min / med / max) | `4 / 5 / 6 ms` | `5 / 5 / 6 ms` | same `5 ms` baseline |
| Upload throughput | `93.6 Mbit/s` | `133.3 Mbit/s` | **`+42%` upload speed** |
| Upload loaded RTT (min / med / max) | `27 / 46 / 67 ms` | `13 / 19 / 28 ms` | **`-59%` median upload lag** |
| Download loaded RTT (min / med / max) | `8 / 23 / 54 ms` | `5 / 5 / 14 ms` | **`0 ms` median download bloat** |

## Sources

- `../../raw/sources/audit/2026-10-25-warp-official-vs-usque/HANDOFF.md`
- `../../raw/sources/audit/2026-10-25-warp-official-vs-usque/REPORT.md`
- `../../raw/sources/audit/2026-10-25-warp-official-vs-usque/evidence/quicsettings-default-disasm.txt`
- `../../../src/tunnel.rs`
