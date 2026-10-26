# QUIC congestion control, pacing, and keepalives

## Congestion control selection

`--congestion-control` exposes `quiche`'s four congestion control algorithms:
- `cubic` (default for upstream compatibility)
- `bbr`
- `bbr2` (recommended on shared/campus wireless and router uplinks)
- `reno`

On long-lived sessions over shared links, `bbr2` maintains steady throughput with minimal UDP receive backlog and lower CPU usage than `cubic`.

## Honoring `SendInfo::at` pacing deadlines

`quiche::Connection::send()` returns a `SendInfo` containing an `at: Instant` pacing deadline. Emitting future-scheduled packets immediately causes micro-bursts, UDP socket drops, and congestion-window collapse.

`src/tunnel.rs` retains a future-scheduled outbound QUIC packet until its `SendInfo::at` deadline and exposes that deadline as a first-class timer branch in the main `tokio::select!` loop. While waiting for a paced send deadline, the loop continues servicing inbound UDP datagrams and TUN packets.

## ACK-eliciting keepalives and dead-peer detection

A timer wakeup alone does not elicit QUIC traffic. At each keepalive interval (30 s default), the client sends an explicit ACK-eliciting PING/keepalive frame and sets `quiche`'s idle timeout to three keepalive periods (90 s), ensuring dead paths are detected promptly.

## Asymmetric datagram queues, 0-RTT resumption, and datagram MTU

- **Asymmetric datagram queues (`[I1]`):** `enable_dgram(true, 8192, 1000)` decouples receive burst capacity (`DGRAM_RECV_QUEUE_LEN = 8192`) from the upload queue (`DGRAM_SEND_QUEUE_LEN = 1000`), preventing silent incoming datagram drops during TUN write stalls without introducing upload bufferbloat.
- **QUIC 0-RTT / session resumption (`[I2]`):** `configure_quic` enables `Config::enable_early_data()`, caches the serialized session (`Connection::session()`) after SPKI verification and `CONNECT-IP` establishment, and restores it via `Connection::set_session()` on reconnect so HTTP/3 `CONNECT` can be sent in 0-RTT early data.
- **Dynamic datagram IP MTU (`[I4]`):** `effective_datagram_ip_mtu` derives the ICMP Packet Too Big / Fragmentation Needed MTU from `conn.dgram_max_writable_len() - flow_prefix.len()` (~1,320 B for a 1,350 B QUIC payload, floored at 1,280 B). `Config::discover_pmtu(false)` is kept explicitly disabled because `quiche` 0.22.0's `Path::should_send_pmtu_probe` (`src/path.rs:339`) checks `hs_confirmed && hs_done` (`handshake_done_sent`, server-only), which otherwise wedges `dgram_send_queue` on clients for packets > 1,157 B.

## Sources

- `../../../src/main.rs`
- `../../../src/tunnel.rs`
