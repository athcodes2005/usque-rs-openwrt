# QUIC congestion control, pacing, and keepalives

## Congestion control selection

`--congestion-control` takes four names, which `quiche` 0.30 maps to three
algorithms:
- `cubic` (default for upstream compatibility) and `reno`: the implementations
  `quiche` has always had. Since 0.30 they are not paced: `SendInfo::at` is the
  time `send()` was called.
- `bbr` and `bbr2` (recommended on shared wireless and router uplinks): both
  select the BBR2 that `quiche` ported from Google's congestion control
  (`bbr2_gcongestion`). The older BBR and BBR2 that 0.22 had are gone.

On long-lived sessions over shared links, `bbr2` maintains steady throughput with minimal UDP receive backlog and lower CPU usage than `cubic`.

## Following the `SendInfo::at` pacing schedule

`quiche::Connection::send()` returns a `SendInfo` containing an `at: Instant` pacing deadline. Emitting future-scheduled packets immediately causes micro-bursts, UDP socket drops, and congestion-window collapse.

BBR2's pacer makes each packet due one interval (size / pacing rate) after the
later of its own schedule and the moment the previous packet was generated. A
sender that is late therefore restarts the schedule from where it is, and the
time it lost is never made up ([[bugs/bbr2-pacer-oversleep]]).

`src/tunnel.rs` sends each packet up to `PACING_LEAD` (3 ms) ahead of its
`at`: `pacing_delay` is the time until `at` minus the lead. Everything due
within the lead goes into the active `UdpSendBatch` (`[I3]`, flushed via a
single `sendmmsg(2)` syscall on Linux). The first packet beyond it is kept in a
pre-allocated buffer and the loop sleeps until it is within the lead; it is
sent on the next pass of the loop in which it is due, whichever
`tokio::select!` branch woke it. A timer that fires up to 3 ms late thus finds
the packet still ahead of `quiche`'s schedule, and what accrued while it slept
goes out as one batch. At pacing gaps under a millisecond this is also what
keeps the loop from needing a timer per packet. Outbound TUN reads also drain
up to `MAX_FORWARDING_BATCH` (64) ready packets per wakeup via
`try_read_tun_packet`.

## ACK-eliciting keepalives and dead-peer detection

A timer wakeup alone does not elicit QUIC traffic. At each keepalive interval (30 s default), the client sends an explicit ACK-eliciting PING/keepalive frame and sets `quiche`'s idle timeout to three keepalive periods (90 s), ensuring dead paths are detected promptly.

## Asymmetric datagram queues, 0-RTT resumption, and datagram MTU

- **Asymmetric datagram queues (`[I1]`):** `enable_dgram(true, 8192, 1000)` decouples receive burst capacity (`DGRAM_RECV_QUEUE_LEN = 8192`) from the upload queue (`DGRAM_SEND_QUEUE_LEN = 1000`), preventing silent incoming datagram drops during TUN write stalls without introducing upload bufferbloat.
- **QUIC 0-RTT / session resumption (`[I2]`):** `configure_quic` enables `Config::enable_early_data()`, caches the serialized session (`Connection::session()`) after SPKI verification and `CONNECT-IP` establishment, and restores it via `Connection::set_session()` on reconnect so HTTP/3 `CONNECT` can be sent in 0-RTT early data.
- **Dynamic datagram IP MTU (`[I4]`):** `effective_datagram_ip_mtu` derives the ICMP Packet Too Big / Fragmentation Needed MTU from `conn.dgram_max_writable_len() - flow_prefix.len()` (~1,320 B for a 1,350 B QUIC payload, floored at 1,280 B). `Config::discover_pmtu(false)` is kept: with `quiche`'s own DPLPMTUD on, a connection starts at 1,200 B and probes upward, while off it uses the 1,350 B limit from the first packet and there is nothing larger to find. (`quiche` 0.22 could not probe as a client at all and wedged `dgram_send_queue` when asked to; 0.30 can.)

## Sources

- `../../../src/main.rs`
- `../../../src/tunnel.rs`
- `quiche` 0.30.1: `src/recovery/gcongestion/pacer.rs`, `src/recovery/congestion/recovery.rs`
- [[bugs/bbr2-pacer-oversleep]]
