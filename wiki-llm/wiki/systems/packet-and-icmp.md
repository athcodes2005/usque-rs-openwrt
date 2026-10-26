# Packet validation and ICMP PMTUD translation

## IPv4 and IPv6 header validation

`src/packet.rs` validates every outbound TUN packet before encapsulation:
- **IPv4:** verifies version `4`, checks `IHL >= 5` and `header_len <= packet.len()`, decrements TTL (rejecting `TTL <= 1`), and recomputes the header checksum across the full `IHL * 4` header including any IPv4 options.
- **IPv6:** verifies version `6`, checks minimum 40-byte header length, and decrements the Hop Limit.

## ICMP Packet Too Big / Fragmentation Needed

When an outbound packet exceeds the negotiated MASQUE datagram payload MTU and cannot be forwarded:
- For IPv4packets with the Don't Fragment (`DF`) bit set, `src/icmp.rs` synthesizes an ICMPv4 Type 3 Code 4 (Fragmentation Needed) message quoting the full IPv4 header (including options) plus 8 payload bytes.
- For IPv6 packets, `src/icmp.rs` synthesizes an ICMPv6 Type 2 Code 0 (Packet Too Big) message with the effective tunnel MTU.

`src/pipeline_tests.rs` unit-tests these production functions directly, including mutation checks on TTL decrement and checksum recalculation.

## Sources

- `../../../src/packet.rs`
- `../../../src/icmp.rs`
- `../../../src/pipeline_tests.rs`
