# Official Cloudflare WARP client vs our `usque-rs` — what REA found, and what we can improve

Written 25 October 2026; re-verified against this repository's tree on
26 October 2026 after the project split from `router-script` (the original
copy under `router-script/audit/` was removed in the split — this folder is
now the canonical home). Read-only examination: nothing in this repository
or on the router was changed to produce it. Analysis performed with REA
6.8.0 (reverse-engineer-anything) plus targeted disassembly of the official
binary.

Start with `HANDOFF.md` if you are the reviewing agent: it maps the evidence
files and lists what is verified versus unknown.

Every claim below is labeled **[O]** observed (from the shipped artifact or our
own source), **[I]** inference, or **[U]** unknown. Findings use W-numbers to
avoid colliding with the F01–F45 audit series inherited from `router-script`.

---

## 1. What was examined

| Artifact | Size | SHA-256 |
|---|---|---|
| `cloudflare-warp_2026.8.2115.0_amd64.deb` (Linux, jammy pool) | 74,937,866 B | `474fd033…0db074` |
| `Cloudflare_WARP_2026.8.2115.0.pkg` (macOS, `downloads.cloudflareclient.com/v1/download/macos/ga`) | 165,873,080 B | `1b81c512…faea5d` |
| `bin/warp-svc` (the daemon inside the deb) | 96,301,248 B | `40ab147c…07676` |

Full digests in `evidence/digests.txt`. **[O]** Both platforms ship the
**same version, 2026.8.2115.0** — the same daemon source. Analysis focused
on the Linux `warp-svc` ELF (x86-64, dynamically linked, **not stripped** —
212,625 symbols, 3,772 mentioning quiche). See `evidence/deb-inventory.txt`
for what the package contains; the GUI (`warp-taskbar`) is Flutter and was
not analyzed.

**[O]** REA Evidence records for `warp-svc`:
`ev_e882c4c5fa18cd9e90571ba2cf4035b812f285e0ab27f4380e7610ca0ca428b2`
(artifact inventory) and
`ev_bbe9f9da21bc44f02b954b365d3302a896184ab9aeb039286a9eabd7da243153`
(symbol table via `inspect-with-objdump`). The two key disassemblies are
saved as `evidence/quicsettings-default-disasm.txt` and
`evidence/cc-variant-decoder-disasm.txt`; the full strings dump is
`evidence/warp-svc-strings.txt.gz`; the architecture symbol extract is
`evidence/architecture-symbols.txt`.

### How the facts were obtained (method, so you can judge them)

- REA CLI `inspect-artifact` / `inspect-with-objdump` — Evidence-recorded.
- Targeted disassembly of two small functions with `llvm-objdump`
  (`QuicSettings::default`, 508 B; `SupportedCongestionControl::algorithm_name`,
  210 B) — direct observation of the shipped bytes, saved under `evidence/`.
- Symbol-table queries (`nm`) and `strings` — labeled as such where used.
- Our own side: read directly from `src/*.rs` and `Cargo.toml` in this
  repository, and the quiche 0.22.0 upstream source (snapshot in
  `evidence/upstream/`) for library defaults.

---

## 2. What the official client is, identically

**W1 — stack identity [O].** `warp-svc` is built from a **pinned git checkout
of cloudflare/quiche, rev `4ec612d`**, including **tokio-quiche**
(`/…/.cargo/git/checkouts/quiche-4103ffa15137cf39/4ec612d/tokio-quiche/…`
appears in embedded source paths). Internal crates: `warp_edge` (the MASQUE
client: `h3_tun::MasqueTunnel`, `h2_tun::H2Tunnel`,
`wireguard_tun::WireguardTunnel`, BoringSSL `ClientCertificateHook`),
`warp_connection` (tunnel racing), `warp_tun` (`PacketIo` with
`ReadinessReadLease`/`ReadinessWriteLease`), `warp_settings` (MDM policy),
`warp_storage`, plus an embedded `recovery::gcongestion` module (Google
congestion control — BBR2). The language is Rust throughout (Rust-mangled
symbols, cargo build paths, tokio runtime); BoringSSL (C) is statically
linked via the `boring` crate.

**W2 — wire protocol parity [O].** The official client's MASQUE mode speaks
exactly what this client speaks: `cf-connect-ip` CONNECT, `:authority
cloudflareaccess.com`, SNI `consumer-masque.cloudflareclient.com`,
`quarter_stream_id`/`flow_id` (`stream_id / 4`), context ID 0. Our
implementation of the protocol itself is correct and current.

**W3 — QUIC transport parameters, side by side [O].** Our-side values were
re-verified against this tree on 2026-10-26 (`src/tunnel.rs:525-536`,
`src/main.rs` defaults).

| Setting | usque-rs (our source) | official (`QuicSettings::default` disasm) | verdict |
|---|---|---|---|
| CC algorithm | `cubic` default, CLI-selectable (cubic/bbr/bbr2/reno) | `cubic` default | same |
| dgram **send** queue | **1000** | **65536** | see §3 — do NOT copy |
| dgram **recv** queue | **1000** | **65536** | **improvement candidate** |
| max_datagram_frame_size | 65536 (auto via `enable_dgram`) | 65536 | same |
| max UDP payload | 1350 / 1350 | 1350 / 1350 (0x546) | same |
| initial_max_data | 10,000,000 | 10,000,000 (0x989680) | same |
| initial stream data | 1,000,000 ×3 | 1,000,000 ×3 (0xF4240) | same |
| initial streams | 100 / 100 | 100 / 100 (0x64) | same |
| conn/stream max window | quiche defaults: 24 MiB / 16 MiB | 24 MiB (0x1800000) / 16 MiB (0x1000000) | same |
| initial cwnd | quiche default 10 packets | 10 packets (0xa) | same |
| pacing | quiche 0.22 default **on** | tokio-quiche `enable_pacing` + kernel SO_TXTIME offload | we lack the offload layer, see W5 |
| hystart | quiche default on | on | same |
| idle timeout | 90 s (3 × 30 s keepalive) | **56 s** (Duration secs=56) | minor gap |
| handshake timeout | 15 s hard-coded | configurable (`handshake_timeout`, unset by default) | comparable |
| MTU / PMTUD | fixed TUN MTU 1280, no PMTUD | `discover_path_mtu`, `pmtud_max_probes` 3, MDM `enable_pmtud` + `pmtud_search_interval_secs` | **improvement candidate** |
| ALPN | `h3` (quiche default) | `h3` | same |

A key sub-finding: quiche **0.22 defaults already give us** pacing-on, CUBIC,
24 MiB/16 MiB windows, 10-packet initial cwnd, hystart — so our config is far
closer to the official one than it looks; almost every "big" official number
matches a quiche default we get for free. The exceptions are the two dgram
queue lengths and the idle timeout.

**W4 — the official CC menu [O].** The MDM override `proxy_cc_override`
deserializes into `SupportedCongestionControl` whose variants map to
algorithm names (disassembly of `algorithm_name`, variant order 0/1/2):

- `cwnd_max` (an 8-byte immediate in the shipped code) — a fixed/large
  congestion-window mode in the quiche fork,
- `cubic`,
- `bbr2_gcongestion` (BBR2 from the gcongestion module).

The quiche fork's `set_cc_algorithm_name` parses `cwnd_max` as well, and
tokio-quiche additionally knows `congestion_window_unchecked`. The
consumer-facing default remains **cubic**; BBR2 is opt-in via MDM. **[I]**
Cloudflare likely uses `bbr2_gcongestion`/`cwnd_max` for its own proxy
deployments, not by default for consumer WARP.

**W5 — architecture the official client has that we don't [O].**

- **Protocol racing**: `warp_connection::tunnel::WarpTunnel` races
  WireGuard vs MASQUE-H3 (and H2) — `perform_happy_eyeballs_race`,
  `connect_with_protocol_racing`, `RacingConnection`; it also races
  post-quantum vs non-PQ connections, and retries with captive-portal
  awareness (`retry_with_captive_portal_until`). MDM
  `masque_http_version`: `h2_only` / `h3_only` / `h3_with_h2_fallback`.
- **Session resumption**: `tokio_quiche … InitialQuicConnection … resume`
  symbols — it can resume a QUIC session (0-RTT) instead of handshaking cold.
- **Dedicated I/O worker per connection** (`IoWorker`,
  `InboundPacketRouter`, `ClientConnector`, pooled `BufFactory` buffers)
  with **UDP GSO** and **kernel pacing offload (SO_TXTIME)** when the
  socket supports them.
- **TUN gating** of its own: `warp_tun::completion::wrapper`
  `ReadinessReadLease`/`ReadinessWriteLease` around `PacketIo` — i.e. it
  also manages read/write readiness rather than blindly pumping.
- **Adaptive timeouts**: outside-tunnel latency probes against
  `engage.cloudflareclient.com`, network-health monitoring
  (`loss_rate`, `latency_ms` thresholds), `dynamic_timeout_overrides`,
  `high_timeouts_enabled` on captive/degraded networks.
- **Registration surface**: far richer than ours — auth methods
  (Access service token, WARP connector token, JWT), hardware/TPM-backed
  registration, a TPM certificate-renewal actor, multi-user registration,
  rate-limit-aware retry (`RateLimited`, `MaxRateLimitAttemptsReached`,
  `RegistrationMismatch`), virtual networks, DEX posture tests, a TURN-based
  speed test, and server-pushed client-version config
  (`target_client_version`, `VersionConfig`).

**W6 — our registration vs theirs [O].** We register as a fake Android
client (`User-Agent: WARP for Android`, `CF-Client-Version: a-6.35-4471`) at
`api.cloudflareclient.com/v0a4471/reg` with a curve25519 key, then PATCH the
device to a secp256r1 key with `tunnel_type: "masque"` — the working
community approach. It has served us fine. **[I]** The `a-6.35-4471` version
string is now years behind the official `2026.8.x`; if Cloudflare ever
enforces client versions (they already push a `VersionConfig` /
`target_client_version` field), registration is the part that breaks.

---

## 3. What this says about our F10 work (upload latency)

The official client queues up to **65,536 datagrams** (≈ 88 MiB at 1350 B
each) inside the client before shedding load — 65× our 1000. **[I]** Cloudflare
accepts that because on a desktop the queue drains through a dedicated
`IoWorker` with GSO (many datagrams per syscall) and kernel-thread pacing,
and a desktop CPU can absorb the burst; the queue is never *managed*, it is
just big.

Our diagnosis (queue inside the client is invisible to `fq_codel`) and our
CoDel direction therefore remain **more conservative than Cloudflare's own
client** for a slow router CPU. This examination does not change F10's plan;
it supports it. Do not copy the 65536 send queue.

**Note for this repository:** the upload CoDel is now in-tree here
(`src/tunnel.rs`: `Codel` + `UploadQueue`, RFC 8289 constants — interval
100 ms, target set by `--upload-codel <MS>`, default 0 = off), imported in
the project split. It is not yet validated against a congested uplink;
the official client's unmanaged 65,536-deep queue is the counter-evidence
that makes validating ours worthwhile rather than copying theirs.

---

## 4. Improvement candidates for this client

Ranked by expected value ÷ effort. All are testable with the
`measure_load_latency.sh` harness (in the `router-script` repository) and
the repo's test-first rules.

### I1 — Split the dgram queues: keep send small, raise recv (LOW effort)
`enable_dgram(true, 1000, 1000)` (`src/tunnel.rs`) uses one depth for both
directions. They have different jobs:

- **Send queue** is the upload-bufferbloat lever (F10). Keep it small /
  CoDel-managed (`--upload-codel`).
- **Recv queue** bounds download-burst tolerance. QUIC **datagrams are never
  retransmitted** — if the recv queue overflows while the event loop is busy
  (TUN write stall, batch limit), those IP packets are silently gone and TCP
  inside the tunnel must recover them. The official client allows 65,536.

Proposed: `enable_dgram(true, RECV_RAISED, SEND_MANAGED)` with
`RECV_RAISED` ≈ 4096–8192 (measure, don't guess: log
`conn.dgram_recv_queue_len()` high-water per session for a week first).
Risk: none — it cannot add latency on the send path.

### I2 — QUIC session resumption / 0-RTT (MEDIUM effort, HIGH reconnect value)
quiche 0.22 already has the whole API (`Connection::session()` /
`set_session()` / `Config::enable_early_data()` — verified in the 0.22.0
source, `evidence/upstream/quiche-0.22.0-lib.rs`). Persist the session blob
to the config after a healthy handshake, feed it back on reconnect.
Reconnects are exactly what our backoff logic triggers; today every one
pays a full handshake (1–2 RTT + TLS). The official client resumes
(`InitialQuicConnection::resume`).

### I3 — Batch QUIC sends (UDP GSO / sendmmsg) (MEDIUM effort, CPU relief)
We send **one datagram per `send_to` syscall**. The official `IoWorker`
batches with GSO (`has_gso`, `pool_send_buffer`, `send_capacity_factor`).
On a saturated router CPU this is syscall-per-packet overhead. quiche hands
us one packet per `conn.send()`; collecting the paced batch into a
`sendmmsg` (or `sendmsg`+`UDP_SEGMENT` when the OpenWrt kernel supports it)
cuts per-packet cost during both saturations. This also makes the pacing
loop cheaper than `sleep()`-per-packet.

### I4 — PMTUD instead of a fixed 1280 TUN MTU (LOW-MEDIUM effort)
The official client exposes `enable_pmtud` / `pmtud_search_interval_secs`
MDM and quiche's `discover_pmtu` with `pmtud_max_probes = 3`. quiche 0.22
has `Config::discover_pmtu()` (`evidence/upstream/quiche-0.22.0-lib.rs`
line 971). Our QUIC payload is already 1350; raising the TUN MTU toward
~1320 (1350 minus the flow-prefix bytes) under PMTUD protection yields
~3–4% fewer packets for the same bytes — throughput and CPU, not just
cosmetics. Fallback stays 1280.

### I5 — Idle timeout: 90 s → 56–60 s (LOW effort, LOW value)
Align with the official 56 s: with our 30 s keepalive, idle = 2 keepalives
(60 s) instead of 3. Faster dead-path detection; interacts with the
dead-peer timeout logic, so touch both together.

### I6 — Registration version-string maintenance (LOW effort, insurance)
Track `CF-Client-Version`/`User-Agent` freshness (W6). Cheap; do it the next
time registration is touched for any other reason. **[I]** Not urgent.

### I7 — Endpoint/protocol fallback (HIGH effort, optional)
The official client races WireGuard + H3 + H2 and PQ/non-PQ. A router-sized
subset would be IPv4/v6 racing or an alternate port. Our single-endpoint
setup has worked; treat this as an idea, not a recommendation.

---

## 5. Anti-recommendations (things in the official client not to copy)

- **65,536-deep send queue** — would make F10 much worse (§3).
- **`cwnd_max` / `congestion_window_unchecked`** — disables CC discipline;
  hostile to any shared uplink. Even Cloudflare doesn't default
  to it.
- **BBR2-by-default** — the official default is cubic; `bbr2_gcongestion` is
  an MDM opt-in. We already expose bbr2 behind `--congestion-control`;
  benchmark it under congestion with `measure_load_latency.sh` rather than
  switching blindly.

---

## 6. Unknowns (stated, not hidden)

- **[U]** Whether `warp_edge` overrides `cc_algorithm`, pacing, or the dgram
  queues at runtime for the MASQUE tunnel beyond `QuicSettings::default`.
  The overrides would live inlined inside the async connect closures; the
  defaults themselves are observed (cubic, 65536/65536, 1350/1350). The MDM
  catalog exposes only `proxy_cc_override` (CC), `proxy_ip_preference`, etc. —
  **no dgram or pacing key** — so the defaults stand unless code inlined in
  the closures says otherwise. To close this: run the official client under
  REA `observe_native_calls` (LLDB breakpoints on `set_cc_algorithm_name` /
  `enable_dgram`) — that executes the official daemon and contacts
  Cloudflare, so it was **not** run without approval; or install an MDM
  profile with `proxy_cc_override` and observe behavior.
- **[U]** tokio-quiche defaults were cross-checked against the public
  master source because the pinned rev `4ec612d` is not public. Every number
  in the W3 table was read directly from the shipped binary's disassembly,
  so this caveat does not affect the table.
- **[U]** The macOS `CloudflareWARP` Mach-O (inside the pkg retained in the
  analysis scratch space) was not decompiled — same source version as the
  analyzed ELF; a Hopper pass is available if we ever need Mach-O-specific
  detail.

---

## 7. Reproduction

```sh
# packages (same version on both platforms; digests in evidence/digests.txt)
curl -sL -o cloudflare-warp_2026.8.2115.0_amd64.deb \
  "https://pkg.cloudflareclient.com/pool/jammy/main/c/cloudflare-warp/cloudflare-warp_2026.8.2115.0_amd64.deb"
curl -sL -o Cloudflare_WARP_ga.pkg "https://downloads.cloudflareclient.com/v1/download/macos/ga"

# deb members are named with trailing slashes (BSD ar chokes); extract ar format manually,
# then: tar -xzf data.tar.gz  →  bin/warp-svc

# REA inventory + symbol Evidence (sha256 40ab147c…07676)
npx -y rea-agents@6.8.0 inspect-artifact  <path>/warp-svc --json
npx -y rea-agents@6.8.0 inspect-with-objdump <path>/warp-svc symbols --json

# the two decisive disassemblies (see evidence/)
llvm-objdump -d --start-address=0x2b8bf40 --stop-address=0x2b8c13c --no-show-raw-insn warp-svc
llvm-objdump -d --start-address=0x3bced60 --stop-address=0x3bcee32 --no-show-raw-insn warp-svc
```

Quiche 0.22.0 sources for the defaults we rely on are snapshotted in
`evidence/upstream/` (see `evidence/upstream/ORIGIN.md` for the exact
fetched URLs and the line citations).
