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

## Sources

- `../../../src/main.rs`
- `../../../src/tunnel.rs`
