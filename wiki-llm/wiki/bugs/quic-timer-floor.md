# QUIC timer spin fix (5 ms floor)

## Symptom

During long-lived sessions, `usque-rs` occasionally pinned one CPU core at ~100% and added 700–1300 ms of latency to tunneled traffic even at low bitrates (<2 Mbps).

## Root cause

When `quiche::Connection::timeout()` returned `Some(Duration::ZERO)` for an expired loss-recovery timer that `on_timeout()` did not clear, `tokio::time::sleep(Duration::ZERO)` yielded immediately on every select iteration, creating a tight CPU spin loop.

## Fix

In the established session loop (`src/tunnel.rs`), expired or sub-5ms `quiche` timeouts are floored to `Duration::from_millis(5)`. Because TUN and UDP socket readiness branches in `tokio::select!` still wake immediately on actual packet arrival, active throughput and latency are unaffected while CPU spin is eliminated.

## Sources

- `../../../src/tunnel.rs`
