# Upload CoDel queue management

## Problem: unmanaged internal datagram FIFO

`quiche` configures `enable_dgram(true, 8192, 1000)` (`DGRAM_RECV_QUEUE_LEN = 8192`, `DGRAM_SEND_QUEUE_LEN = 1000`), which bounds outbound datagrams at 1,000 (~1.3 MB) inside `quiche`'s internal send FIFO without timestamps while giving the receive path an 8,192-datagram buffer against burst loss. When the uplink is congested (e.g., 10–18 Mbit/s), bulk TCP uploads can still fill the 1,000-deep send FIFO and add 120–250+ ms of standing queueing delay to interactive/gaming packets.

Pausing TUN reads (`--tun-backpressure`) only shifts the FIFO into the Linux TUN driver's 500-packet ring buffer rather than engaging `fq_codel`.

## Solution: head-of-queue CoDel (`--upload-codel MS`)

When `--upload-codel <MS>` (`1..=999`, e.g., `20`) is passed:
1. The client records the enqueue timestamp and byte size of each datagram accepted into `quiche`'s outgoing queue.
2. Because `quiche` dequeues strictly FIFO from the front, `dgram_send_queue_len()` synchronizes which entries have already been transmitted.
3. Before each send pass, an RFC 8289 CoDel state machine (target = `MS` ms, interval = `100` ms) inspects the head of the queue.
4. If the oldest datagram's sojourn time stays above `MS` for a full 100 ms interval, it is purged via `dgram_purge_outgoing` and the next drop is scheduled at `100 ms / sqrt(count)`.
5. Drop summaries are logged to syslog 5 s after drops cease (or once per minute during sustained congestion).

## Sources

- `../../../src/tunnel.rs`
- `../../../src/main.rs`
