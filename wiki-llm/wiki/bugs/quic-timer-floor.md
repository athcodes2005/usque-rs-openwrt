# QUIC timer spin fix (5 ms floor)

## Symptom

During long-lived sessions, `usque-rs` occasionally pinned one CPU core at ~100% and added 700–1300 ms of latency to tunneled traffic even at low bitrates (<2 Mbps).

## Root cause

When `quiche::Connection::timeout()` returned `Some(Duration::ZERO)` for an expired loss-recovery timer that `on_timeout()` did not clear, `tokio::time::sleep(Duration::ZERO)` yielded immediately on every select iteration, creating a tight CPU spin loop.

## Fix

In the established session loop (`src/tunnel.rs`), expired or sub-5ms `quiche` timeouts are floored to `Duration::from_millis(5)`. Because TUN and UDP socket readiness branches in `tokio::select!` still wake immediately on actual packet arrival, active throughput and latency are unaffected while CPU spin is eliminated.

## Follow-up: expired timers under load

The floor turned an expired timer into "5 ms without a TUN or UDP event": the
sleep is rebuilt on every loop pass, and above roughly 200 packets a second
another branch always wins first. `on_timeout()` is the only place quiche
handles probe timeouts, the idle timeout and the end of a draining connection,
so a dead peer or a server close went unnoticed for as long as LAN traffic kept
flowing. After each `select!` the loop now runs an already expired timer itself
(`run_expired_timers`), at most once per 5 ms, so the spin cannot return.

## Follow-up: handshake and CONNECT-wait loops

Both loops passed `conn.timeout()` straight to `sleep`, so the same zero
timeout could spin a core for the 15 s handshake deadline. The CONNECT wait was
also capped at 100 passes, which a run of zero timeouts used up at once and
reported as "timed out waiting for CONNECT response". Both now sleep for
`handshake_loop_timeout` (floored at 5 ms, never past the deadline), and the
CONNECT wait ends only at its deadline.

## Sources

- `../../../src/tunnel.rs`
