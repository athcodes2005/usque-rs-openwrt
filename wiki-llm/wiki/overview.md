# Project overview

`usque-rs-openwrt` is an OpenWrt-hardened fork of [Diniboy1123/usque-rs](https://github.com/Diniboy1123/usque-rs),
an asynchronous Rust client for Cloudflare WARP using MASQUE (`CONNECT-IP` over HTTP/3 and QUIC).

## Scope and boundary

The binary handles only:
1. Device registration (`login` subcommand) and atomic `0600` persistence of `config.json`.
2. Linux TUN creation (`tun0` by default) and IPv4/IPv6 interface address assignment.
3. QUIC/HTTP-3 extended `CONNECT` (`cf-connect-ip`), SPKI public-key pinning, and datagram forwarding.
4. IPv4/IPv6 header validation and ICMP Packet Too Big / Fragmentation Needed translation.

Routing policy, firewall rules, DNS, and watchdog orchestration belong to external router scripts.

## Delegation contract with `router-script`

Agents working in `usque-rs-openwrt` own only the Rust crate, its tests, and GitHub Release artifacts. For any router deployment, binary delivery (`fetch_usque_rs.sh` / `sync_to_router.sh`), runtime CLI flag changes (`tunnel.sh`), or TUN MTU / TCP MSS clamping updates (`defaults.sh`, `tunnel.sh`, `ensure_network.sh`), **do not edit `/Users/atharva/projects/router-script` or access the router directly**—ask the user to delegate that work to the agent in `router-script` and provide a self-contained handoff prompt.

## CLI options (`nativetun`)

- `--congestion-control <cubic|bbr|bbr2|reno>`: selects the `quiche` congestion controller (`cubic` default).
- `--ready-file <PATH>`: atomically writes the process PID only after `CONNECT-IP` HTTP 200 readiness, removing it on session disconnect.
- `--syslog`: emits structured connection lifecycle and error events to `/dev/log` (`usque-rs` tag).
- `--upload-codel <MS>`: applies CoDel (RFC 8289, 100 ms interval) to `quiche`'s outgoing datagram queue when `MS` is `1..=999` (`0` = disabled).
- `--tun-backpressure <N>`: pauses TUN reads when `N` datagrams are queued in `quiche` (`0` = disabled).

## Sources

- `../../src/main.rs`
- `../../src/tunnel.rs`
- `../../Cargo.toml`
