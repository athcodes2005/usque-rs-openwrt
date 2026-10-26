# UDP receive buffer

## Problem: kernel drops under download bursts

The QUIC session socket (`tokio::net::UdpSocket::bind` in `src/tunnel.rs`) is created per session, so every reconnect inherited the kernel default. On the OpenWrt aarch64 target `net.core.rmem_default` and `rmem_max` are both 212992 bytes. During a 4-stream download through the tunnel (`nativetun` mode) the kernel UDP `RcvbufErrors` counter rose by thousands per run (799, 166, 20,180, 5,677) and download ping loss was 10–23%.

## Solution: `--udp-recv-buffer BYTES`

- `nativetun --udp-recv-buffer <BYTES>`; default `4194304` (4 MiB); `0` keeps the kernel default.
- Linux only (`cfg(target_os = "linux")`, `libc`); a no-op elsewhere.
- Applied right after the bind, in every session: `SO_RCVBUFFORCE` first (needs `CAP_NET_ADMIN`, ignores `rmem_max`), then `SO_RCVBUF` (capped by `rmem_max`) on `EPERM`.
- The effective size is read back with `getsockopt(SO_RCVBUF)`; Linux reports double the request. It is logged once per process as an info event, noting when `rmem_max` capped it.
- Failure never ends the session: one warning event, then the kernel default is used.
- `tunnel.sh`-style wrappers can detect support with `nativetun --help` (`--udp-recv-buffer <BYTES>`).

## Evidence (live A/B, 2026-10-26)

With a 4 MiB receive buffer, `RcvbufErrors` stayed at 0 in both runs, download ping loss fell to 0–8%, and UDP socket memory peaked at about 3.5 MB, so the larger buffer is cheap.

## Tests

`tunnel::tests::recv_buffer` binds a localhost socket and asserts the read-back is at least the 1 MiB request when `SO_RCVBUFFORCE` was accepted, otherwise at least the previous size (unprivileged CI, or root without `CAP_NET_ADMIN`, as in default containers); `0` leaves it unchanged. `main.rs` tests cover the help text and the default.

Related: [[systems/quic-and-pacing]], [[systems/upload-codel]].

## Sources

- `../../../src/tunnel.rs` (`set_udp_recv_buffer`, `apply_udp_recv_buffer`)
- `../../../src/main.rs`
