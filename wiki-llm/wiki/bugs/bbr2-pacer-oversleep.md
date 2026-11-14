# Upload collapse under quiche 0.30's BBR2 pacer

## Symptom

Found on 2026-11-13 while testing the `quiche` 0.22 to 0.30 upgrade on the
router (`--congestion-control bbr2`, `--upload-codel 20`), before any release:
upload through the tunnel fell to 2.5 and 3.5 Mbit/s, against 58 and 78 Mbit/s
for v0.2.5 in the same interleaved test. Download, idle latency and the tunnel
itself were fine.

## Root cause

`quiche` 0.30 has one BBR2, ported from Google's congestion control, and its
pacer (`recovery/gcongestion/pacer.rs`) schedules the next packet at
`max(schedule, time the last packet was generated) + size / pacing rate`. A
sender that generates a packet after its scheduled time moves the schedule to
that moment: lateness is not made up.

The session loop held the first packet whose `at` lay 500 µs or more ahead and
slept until `at`. Tokio's timer wheel ticks in whole milliseconds, and the
loop's `select!` can let ready TUN and UDP reads win ahead of the pacing timer,
so such a sleep ends late; by how much was not measured, a millisecond or two
is the estimate. At 60 Mbit/s the pacer's gap between packets is 180 µs, so
each cycle sent about half a millisecond's worth of packets in a multiple of
that time. BBR2 paces at the rate it measures, and a rate that is measured
short shrinks again in the next cycle, until the gaps are several milliseconds
and the lateness a small part of them. The roughly 3 Mbit/s seen fits that.
The explanation was confirmed by its fix, not by a trace.

`quiche` 0.22's pacer worked in bursts (up to a `capacity` of bytes per
interval), and the loop, written against it, did not show this.

## Fix

Each packet may leave up to `PACING_LEAD` (3 ms) ahead of its `at`, and the
loop sleeps only until the held packet is within that lead. A late wake-up then
still finds it ahead of the schedule, so the schedule is never restarted and
the packets that came due meanwhile go out together. A held packet is also
released in the first loop pass in which it is due, not only when its timer
branch wins the `select!`.

Measured on the router against v0.2.5, two interleaved rounds each
(`measure_load_latency.sh 12`):

| Client | Upload Mbit/s | Download Mbit/s | Upload latency median |
| --- | --- | --- | --- |
| v0.2.5 (`quiche` 0.22), 21:22 | 58.3, 78.3 | 41.2, 103.8 | 22, 30 ms |
| 0.30 without the fix, 21:23 | 2.5, 3.5 | 50.0, 84.5 | 32, 33 ms |
| v0.2.5, 01:02 | 78.0, 18.9 | 52.6, 116.9 | 17, 34 ms |
| 0.30 with the fix, 01:03 | 96.6, 71.4 | 145.3, 159.3 | 30, 18 ms |

The campus uplink varies from minute to minute, which is why the rounds are
interleaved; the two v0.2.5 rows show how much.

`late_timers_do_not_cost_any_of_the_pacing_rate` runs the loop's rule against a
model of the pacer with a timer that is 1.5 ms late, with and without the lead.

## Not changed

`cubic` and `reno` are not paced at all in `quiche` 0.30 (`at` is the time of
the `send()` call), so they send what the congestion window allows in bursts.
Nothing in the client makes up for that.

## Sources

- `../../../src/tunnel.rs`
- `quiche` 0.30.1: `src/recovery/gcongestion/pacer.rs`,
  `src/recovery/congestion/recovery.rs`; `quiche` 0.22.0:
  `src/recovery/congestion/pacer.rs`
- [[systems/quic-and-pacing]]
