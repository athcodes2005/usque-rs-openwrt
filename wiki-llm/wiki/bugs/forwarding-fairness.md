# Forwarding loop fairness and batching

## Symptom

Under sustained download bursts, `usque-rs` exhibited 1.5–4.2 s latency spikes and a large UDP receive queue backlog.

## Root cause

An unbounded `while socket.try_recv()` drain loop after each event wakeup allowed continuous inbound UDP traffic to monopolize the task, starving HTTP/3 `poll()`, `CONNECT-IP` TUN writes, and outbound QUIC packet flushing.

## Fix

Every stage in `src/tunnel.rs` (UDP intake, HTTP/3 event polling, TUN reads, and QUIC output) is capped at `MAX_STAGE_BATCH = 64` items per pass. When any stage hits its 64-item cap, it sets a cooperative continuation flag so the main loop yields fairly across all directions and immediately resumes remaining work without waiting for an external timer.

## Sources

- `../../../src/tunnel.rs`
