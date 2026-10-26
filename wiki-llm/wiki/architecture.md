# Architecture and modules

## Modules

- `src/main.rs`: CLI argument parsing (`clap`), registration workflow, TUN setup, and option validation.
- `src/config.rs`: `config.json` schema, base64 key decoding, atomic mode-`0600` writes, and peer validation.
- `src/register.rs`: Cloudflare WARP API enrollment, P-256 keypair generation, and bounded HTTP timeouts.
- `src/tls.rs`: ephemeral client TLS certificate construction, immediate PEM temp-file unlinking after load, and mandatory peer SPKI verification.
- `src/tun_device.rs`: Linux TUN interface creation and netlink address/MTU configuration.
- `src/tunnel.rs`: QUIC/H3 handshake, `CONNECT-IP` capsule negotiation, reconnect backoff, CoDel queue management, and the async `tokio::select!` forwarding loop.
- `src/packet.rs`: IPv4/IPv6 header parsing, options-aware IPv4 checksums, and TTL/hop-limit decrement.
- `src/icmp.rs`: ICMPv4 Fragmentation Needed and ICMPv6 Packet Too Big generation.
- `src/events.rs`: non-blocking `/dev/log` syslog writer and terminal-only status output.
- `src/pipeline_tests.rs`: unit tests exercising the production packet and ICMP pipeline.

## Session lifecycle

1. Creates the TUN device and connects immediately without waiting for an initial outbound packet.
2. Enforces bounded 15 s deadlines on both the QUIC handshake and the HTTP/3 `CONNECT-IP` response, caching the TLS 1.3 / QUIC session ticket (`Connection::session()`) after a verified connection and offering it via `Connection::set_session()` + `Config::enable_early_data()` on reconnects for 0-RTT / 1-RTT session resumption.
3. Verifies the server certificate's SubjectPublicKeyInfo against the configured WARP endpoint public key on both cold and resumed sessions.
4. Writes the PID readiness file once `CONNECT-IP` succeeds and removes it when the session ends.
5. Configures asymmetric QUIC datagram queues (`8192` receive depth to absorb download bursts without silent RFC 9221 drops, `1000` send depth managed by `--upload-codel` / `--tun-backpressure`), detects dead paths via a 30 s ACK-eliciting keepalive and 90 s idle timeout, aborts after 32 consecutive TUN write errors, and applies exponential reconnect backoff (1 s to 30 s) for sessions lasting under 10 s.

## Sources

- `../../src/main.rs`, `../../src/config.rs`, `../../src/register.rs`, `../../src/tls.rs`
- `../../src/tunnel.rs`, `../../src/packet.rs`, `../../src/icmp.rs`, `../../src/events.rs`
