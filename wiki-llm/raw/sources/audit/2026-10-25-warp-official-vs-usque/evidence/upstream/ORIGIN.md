# Upstream source snapshots used by this analysis

Fetched 2026-10-25 during the examination. These are verbatim snapshots of
public upstream files, kept here so every line citation in `../REPORT.md`
can be checked without network access.

| File | Origin | Why it is here |
|---|---|---|
| `quiche-0.22.0-lib.rs` | `https://raw.githubusercontent.com/cloudflare/quiche/0.22.0/quiche/src/lib.rs` | The exact quiche **our binary** builds against (`Cargo.toml`: quiche 0.22). Cited for: pacing default `true` (line 867), `MAX_CONNECTION_WINDOW` 24 MiB (line 481), `MAX_STREAM_WINDOW` 16 MiB (doc at line 1325), `DEFAULT_INITIAL_CONGESTION_WINDOW_PACKETS` 10 (line 492), `enable_dgram` semantics — 3 args, sets `max_datagram_frame_size` 65536 (lines 1294–1304), `discover_pmtu` (line 971), `set_session` (line 2177), `session()` (line 6412), `enable_early_data` (line 1009). |
| `quiche-0.22.0-recovery-mod.rs` | `https://raw.githubusercontent.com/cloudflare/quiche/0.22.0/quiche/src/recovery/mod.rs` | Loss-recovery/pacing internals of our quiche (supporting context). |
| `tokio-quiche-config-master.rs` | `https://raw.githubusercontent.com/cloudflare/quiche/master/tokio-quiche/src/settings/config.rs` | How the official client's settings layer maps `QuicSettings` → `quiche::Config` calls (which setters exist and are actually invoked). **Caveat:** master snapshot; the official binary pins a non-public git rev (`4ec612d`) — see REPORT §6. |
| `tokio-quiche-settings-quic-master.rs` | `https://raw.githubusercontent.com/cloudflare/quiche/master/tokio-quiche/src/settings/quic.rs` | Field-by-field `QuicSettings` documentation used to decode the offsets in the shipped `QuicSettings::default` disassembly. Same master-vs-pinned caveat. |

**Verification rule applied in the report:** every number attributed to the
official client in the comparison table was read directly from the shipped
`warp-svc` binary's disassembly (`evidence/quicsettings-default-disasm.txt`),
not from these snapshots; the snapshots only label which field each
immediate is. Numbers attributed to our own binary come from this
repository's `src/` and `Cargo.toml`.
