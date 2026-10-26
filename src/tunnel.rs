use anyhow::{bail, Context, Result};
use portable_atomic::{AtomicU64, Ordering};
use quiche::h3::NameValue;
use ring::rand::SecureRandom;
use std::cell::Cell;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::events::{self, Severity};
use crate::icmp;
use crate::packet;
use crate::tls;

const MAX_DATAGRAM_SIZE: usize = 1350;
// QUIC datagrams (RFC 9221) are never retransmitted. A deep receive queue
// absorbs download bursts while the event loop is draining 64-packet batches
// to the TUN device, whereas the send queue stays small (and managed by
// --upload-codel / --tun-backpressure) to prevent upload bufferbloat (F10).
const DGRAM_RECV_QUEUE_LEN: usize = 8192;
const DGRAM_SEND_QUEUE_LEN: usize = 1000;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
// A session connected at least this long counts as having worked: a clean close
// reconnects at once and a failure restarts the backoff. Anything shorter keeps
// backing off, so a server that accepts and then drops sessions cannot cause a
// reconnect storm.
const STABLE_SESSION: Duration = Duration::from_secs(10);
// A keepalive goes out every period, so three periods without any packet from
// the server means the path is dead, not that a keepalive or two was lost.
const IDLE_TIMEOUT_KEEPALIVES: u32 = 3;
// quiche can report a zero-duration timeout while loss recovery is pending.
// Passing that directly to tokio::time::sleep creates a hot loop that can pin
// one router CPU core even when the tunnel is carrying little traffic.
const MIN_EVENT_LOOP_SLEEP: Duration = Duration::from_millis(5);
// Every forwarding stage must eventually yield to the others. In particular,
// an unbounded UDP drain can remain readable indefinitely during a download,
// starving decoded datagrams, TUN writes, and QUIC ACK transmission.
const MAX_FORWARDING_BATCH: usize = 64;
// A failed TUN write loses that one packet. Writes that keep failing mean the
// device is gone, so the session ends instead of silently dropping all traffic.
const MAX_CONSECUTIVE_TUN_WRITE_FAILURES: u32 = 32;
// CoDel's interval (RFC 8289): the upload queue may stay above its target this
// long before the first drop, and later drops come interval/sqrt(count) apart.
const CODEL_INTERVAL: Duration = Duration::from_millis(100);
// A queue back above target this soon after the last drop resumes at the drop
// rate that controlled it then (the Linux variant the RFC describes).
const CODEL_RECENT: Duration = CODEL_INTERVAL.saturating_mul(16);
// Upload drops are reported once they have stopped for UPLOAD_DROP_QUIET, or
// gone on for a report period, and at most once a period, so a long upload
// cannot flood the RAM log.
const UPLOAD_DROP_REPORT_PERIOD: Duration = Duration::from_secs(60);
const UPLOAD_DROP_QUIET: Duration = Duration::from_secs(5);

/// Whether to read another packet from the TUN device. With backpressure on,
/// reads pause once `limit` datagrams wait in quiche's send queue: an unmanaged
/// queue there (up to 1,000 datagrams) adds latency to every flow during an
/// upload, where the kernel's qdisc could manage it instead.
const fn tun_read_allowed(queued: usize, limit: usize) -> bool {
    limit == 0 || queued < limit
}

/// Controlled delay (RFC 8289, in its Linux form) for the upload datagrams that
/// wait in quiche's send queue. quiche dequeues them itself, so the oldest one
/// is judged just before each send: dropped, or left for quiche to send.
struct Codel {
    target: Duration,
    /// Nothing is dropped with at most this many bytes behind the oldest
    /// datagram: one datagram is not a standing queue.
    max_packet: usize,
    first_above_time: Option<Instant>,
    drop_next: Instant,
    count: u32,
    last_count: u32,
    dropping: bool,
}

impl Codel {
    const fn new(target: Duration, max_packet: usize, now: Instant) -> Self {
        Self {
            target,
            max_packet,
            first_above_time: None,
            drop_next: now,
            count: 0,
            last_count: 0,
            dropping: false,
        }
    }

    /// Whether the oldest datagram, and so the queue, has stayed above target
    /// for a whole interval.
    fn ok_to_drop(&mut self, now: Instant, oldest: Option<(Duration, usize)>) -> bool {
        match oldest {
            Some((sojourn, behind)) if sojourn >= self.target && behind > self.max_packet => {
                now >= *self.first_above_time.get_or_insert(now + CODEL_INTERVAL)
            }
            _ => {
                self.first_above_time = None;
                false
            }
        }
    }

    /// Whether to drop the oldest datagram, given how long it has waited and
    /// how many bytes are queued behind it (`None`: the queue is empty). Asked
    /// again about the next datagram after each drop.
    fn should_drop(&mut self, now: Instant, oldest: Option<(Duration, usize)>) -> bool {
        let ok_to_drop = self.ok_to_drop(now, oldest);
        if self.dropping {
            if !ok_to_drop {
                self.dropping = false;
                return false;
            }
            if now < self.drop_next {
                return false;
            }
            self.count = self.count.saturating_add(1);
            self.drop_next = codel_control_law(self.drop_next, self.count);
            return true;
        }
        if !ok_to_drop {
            return false;
        }
        let delta = self.count.saturating_sub(self.last_count);
        let recent = now.saturating_duration_since(self.drop_next) < CODEL_RECENT;
        self.count = if delta > 1 && recent { delta } else { 1 };
        self.last_count = self.count;
        self.drop_next = codel_control_law(now, self.count);
        self.dropping = true;
        true
    }
}

/// When to drop next: sooner the longer the queue has stayed above target.
fn codel_control_law(last: Instant, count: u32) -> Instant {
    last + CODEL_INTERVAL.div_f64(f64::from(count).sqrt())
}

/// The upload datagrams in quiche's send queue, oldest first: when each was
/// queued, and its size. quiche sends them only from the front, so while it
/// holds `n`, all but the newest `n` recorded here have left.
struct UploadQueue {
    waiting: VecDeque<(Instant, usize)>,
    bytes: usize,
    codel: Codel,
    /// Drops not yet reported, when the first and the last of them happened,
    /// and when the last report went out.
    unreported: u64,
    unreported_since: Option<Instant>,
    last_drop: Instant,
    last_report: Option<Instant>,
}

impl UploadQueue {
    const fn new(target: Duration, max_packet: usize, now: Instant) -> Self {
        Self {
            waiting: VecDeque::new(),
            bytes: 0,
            codel: Codel::new(target, max_packet, now),
            unreported: 0,
            unreported_since: None,
            last_drop: now,
            last_report: None,
        }
    }

    /// Record a datagram that quiche accepted.
    fn push(&mut self, now: Instant, len: usize) {
        self.waiting.push_back((now, len));
        self.bytes += len;
    }

    /// How many of the oldest datagrams to drop before quiche sends again,
    /// now that it holds `queued`. The caller removes them from quiche.
    fn stale(&mut self, now: Instant, queued: usize) -> usize {
        while self.waiting.len() > queued {
            self.pop_oldest();
        }
        let mut dropped = 0;
        while self.codel.should_drop(now, self.oldest(now)) {
            self.pop_oldest();
            dropped += 1;
        }
        if dropped > 0 {
            self.unreported += dropped as u64;
            self.unreported_since.get_or_insert(now);
            self.last_drop = now;
        }
        dropped
    }

    /// How long the oldest datagram has waited, and the bytes queued behind it.
    fn oldest(&self, now: Instant) -> Option<(Duration, usize)> {
        self.waiting
            .front()
            .map(|&(queued_at, len)| (now.saturating_duration_since(queued_at), self.bytes - len))
    }

    fn pop_oldest(&mut self) {
        if let Some((_, len)) = self.waiting.pop_front() {
            self.bytes -= len;
        }
    }

    /// A summary of the drops once they have stopped for a while or gone on
    /// for a report period, and at most once a period.
    fn report(&mut self, now: Instant) -> Option<String> {
        let span = now.saturating_duration_since(self.unreported_since?);
        let stopped = now.saturating_duration_since(self.last_drop) >= UPLOAD_DROP_QUIET;
        let spaced = self.last_report.map_or(true, |at| {
            now.saturating_duration_since(at) >= UPLOAD_DROP_REPORT_PERIOD
        });
        if !(spaced && (stopped || span >= UPLOAD_DROP_REPORT_PERIOD)) {
            return None;
        }
        let report = format!(
            "[upload] CoDel dropped {} datagram{} in the last {}s (target {} ms)",
            self.unreported,
            if self.unreported == 1 { "" } else { "s" },
            span.as_secs(),
            self.codel.target.as_millis()
        );
        self.unreported = 0;
        self.unreported_since = None;
        self.last_report = Some(now);
        Some(report)
    }
}

/// Remove the `count` oldest datagrams from quiche's send queue.
fn drop_oldest_datagrams(conn: &mut quiche::Connection, count: usize) {
    // quiche visits its queue oldest first.
    let left = Cell::new(count);
    conn.dgram_purge_outgoing(|_| {
        let n = left.get();
        left.set(n.saturating_sub(1));
        n > 0
    });
}

/// Drop the upload datagrams that have waited too long, before quiche sends
/// again, and now and then report how many went.
fn drop_stale_uploads_at(
    now: Instant,
    queue: &mut UploadQueue,
    conn: &mut quiche::Connection,
    stats: &Stats,
) {
    let stale = queue.stale(now, conn.dgram_send_queue_len());
    if stale > 0 {
        drop_oldest_datagrams(conn, stale);
        stats.dropped.fetch_add(stale as u64, Ordering::Relaxed);
    }
    if let Some(report) = queue.report(now) {
        events::event(Severity::Info, &report);
    }
}

fn drop_stale_uploads(queue: &mut UploadQueue, conn: &mut quiche::Connection, stats: &Stats) {
    drop_stale_uploads_at(Instant::now(), queue, conn, stats);
}

fn idle_timeout(keepalive: Duration) -> Duration {
    keepalive * IDLE_TIMEOUT_KEEPALIVES
}

/// How the server ended the CONNECT stream that carries the tunnel.
#[derive(Debug, PartialEq, Eq)]
enum ConnectStreamEnd {
    Finished,
    Reset(u64),
}

/// Datagrams for the flow are useless once its CONNECT stream is gone.
const fn connect_stream_end(
    event_stream: u64,
    connect_stream: u64,
    event: &quiche::h3::Event,
) -> Option<ConnectStreamEnd> {
    if event_stream != connect_stream {
        return None;
    }
    match event {
        quiche::h3::Event::Finished => Some(ConnectStreamEnd::Finished),
        quiche::h3::Event::Reset(code) => Some(ConnectStreamEnd::Reset(*code)),
        _ => None,
    }
}

/// Returns how long to wait before reconnecting, and the delay to use after that.
fn reconnect_backoff(
    connected_for: Option<Duration>,
    clean: bool,
    delay: Duration,
) -> (Duration, Duration) {
    let stable = connected_for.is_some_and(|lived| lived >= STABLE_SESSION);
    let delay = if stable {
        Duration::from_secs(1)
    } else {
        delay
    };
    if clean && stable {
        (Duration::ZERO, delay)
    } else {
        (delay, (delay * 2).min(MAX_RECONNECT_DELAY))
    }
}

fn event_loop_timeout(quic_timeout: Option<Duration>, keepalive: Duration) -> Duration {
    quic_timeout
        .unwrap_or(keepalive)
        .min(keepalive)
        .max(MIN_EVENT_LOOP_SLEEP)
}

const fn forwarding_batch_limit_reached(index: usize) -> bool {
    index + 1 == MAX_FORWARDING_BATCH
}

fn pacing_delay(send_at: Instant, now: Instant) -> Duration {
    send_at.saturating_duration_since(now)
}

async fn write_tun<W>(
    tun_writer: &mut W,
    packet: &[u8],
    stats: &Stats,
    failures: &mut u32,
) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    match tokio::io::AsyncWriteExt::write_all(tun_writer, packet).await {
        Ok(()) => {
            *failures = 0;
        }
        Err(error) => {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            *failures += 1;
            if *failures >= MAX_CONSECUTIVE_TUN_WRITE_FAILURES {
                bail!("{failures} consecutive TUN writes failed: {error}");
            }
            log::debug!("TUN write failed: {error}");
        }
    }
    Ok(())
}

async fn send_paced_packet(
    socket: &tokio::net::UdpSocket,
    packet: &[u8],
    send_info: quiche::SendInfo,
) -> std::io::Result<usize> {
    let delay = pacing_delay(send_info.at, Instant::now());
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    socket.send_to(packet, send_info.to).await
}

/// Configuration for a MASQUE tunnel session.
pub struct TunnelConfig {
    pub endpoint: SocketAddr,
    pub sni: String,
    pub keepalive_period: Duration,
    pub mtu: u32,
    pub congestion_control: String,
    pub ready_file: Option<String>,
    /// Pause TUN reads at this many queued datagrams; 0 disables.
    pub tun_backpressure: usize,
    /// Target queueing delay for upload datagrams (controlled delay); zero
    /// disables.
    pub upload_codel: Duration,
}

struct AbortTaskOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortTaskOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn clear_ready_file(path: Option<&str>) {
    let Some(path) = path else {
        return;
    };
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("failed to remove readiness marker {path}: {error}");
        }
    }
}

fn mark_ready(path: Option<&str>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create readiness directory for {path}"))?;
    }
    std::fs::write(path, format!("{}\n", std::process::id()))
        .with_context(|| format!("failed to write readiness marker {path}"))
}

struct Stats {
    tx_packets: AtomicU64,
    rx_packets: AtomicU64,
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
    dropped: AtomicU64,
    quic_lost: AtomicU64,
    quic_retrans: AtomicU64,
}

impl Stats {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            tx_packets: AtomicU64::new(0),
            rx_packets: AtomicU64::new(0),
            tx_bytes: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            quic_lost: AtomicU64::new(0),
            quic_retrans: AtomicU64::new(0),
        })
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!(
            "{}h {:02}m {:02}s",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}

/// Effective IP MTU that fits inside a QUIC DATAGRAM frame after subtracting
/// the MASQUE `flow_id` + `context_id` prefix, floored at the IPv6 minimum
/// link MTU (1280 bytes).
const fn effective_datagram_ip_mtu(
    max_writable_dgram: Option<usize>,
    flow_prefix_len: usize,
) -> u16 {
    match max_writable_dgram {
        Some(max_dgram) => match max_dgram.checked_sub(flow_prefix_len) {
            Some(ip_mtu) if ip_mtu >= 1280 && ip_mtu <= u16::MAX as usize => ip_mtu as u16,
            _ => 1280,
        },
        None => 1280,
    }
}

/// Apply transport parameters, asymmetric datagram queues, PMTUD, and 0-RTT
/// early data to a `quiche::Config`.
fn configure_quic(
    quic_config: &mut quiche::Config,
    idle: Duration,
    congestion_control: &str,
) -> Result<()> {
    quic_config.verify_peer(false);
    quic_config
        .set_application_protos(quiche::h3::APPLICATION_PROTOCOL)
        .map_err(|e| anyhow::anyhow!("set ALPN: {e}"))?;
    quic_config.set_max_idle_timeout(u64::try_from(idle.as_millis()).unwrap_or(u64::MAX));
    quic_config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    quic_config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    // Keep quiche's internal DPLPMTUD disabled on quiche 0.22.0 clients: in
    // quiche 0.22.0 (`src/path.rs:339`), `should_send_pmtu_probe` checks
    // `hs_confirmed && hs_done` (`self.handshake_done_sent`), which is only
    // ever true on a server. Enabling `discover_pmtu(true)` on a client sets
    // `dgram_max_writable_len()` to 1,322 bytes in `send()` (`src/lib.rs:3384`,
    // which uses `||`) while `send_single()` (`src/lib.rs:3608`) stays clamped
    // at `pmtud.get_current() == 1200`, wedging `dgram_send_queue` on the first
    // packet > 1,157 bytes. With `discover_pmtu(false)` and the 1,350-byte UDP
    // payload limit, quiche negotiates a 1,322-byte writable datagram payload
    // (~1,320-byte IP MTU) and `effective_datagram_ip_mtu` advertises that
    // exact ceiling via ICMP PMTUD.
    quic_config.discover_pmtu(false);
    quic_config.enable_early_data();
    quic_config.set_initial_max_data(10_000_000);
    quic_config.set_initial_max_stream_data_bidi_local(1_000_000);
    quic_config.set_initial_max_stream_data_bidi_remote(1_000_000);
    quic_config.set_initial_max_stream_data_uni(1_000_000);
    quic_config.set_initial_max_streams_bidi(100);
    quic_config.set_initial_max_streams_uni(100);
    quic_config.set_disable_active_migration(true);
    quic_config.enable_dgram(true, DGRAM_RECV_QUEUE_LEN, DGRAM_SEND_QUEUE_LEN);
    quic_config
        .set_cc_algorithm_name(congestion_control)
        .map_err(|e| anyhow::anyhow!("invalid congestion control algorithm: {e}"))?;
    Ok(())
}

/// Offer a cached TLS 1.3 / QUIC session ticket on a newly created connection
/// before any packet is sent. Invalid or corrupted session blobs are dropped
/// so the connection proceeds with a full handshake.
fn restore_quic_session(
    conn: &mut quiche::Connection,
    offered_session: &mut Option<Vec<u8>>,
) -> bool {
    let Some(session) = offered_session.as_deref() else {
        return false;
    };
    match conn.set_session(session) {
        Ok(()) => {
            log::debug!("Offering cached QUIC session ({} bytes)", session.len());
            true
        }
        Err(error) => {
            log::debug!("Discarding unusable cached QUIC session: {error}");
            *offered_session = None;
            false
        }
    }
}

/// Run the MASQUE tunnel and reconnect with bounded backoff.
pub async fn maintain_tunnel(
    config: &Config,
    tunnel_cfg: &TunnelConfig,
    tun_dev: tun::Device,
) -> Result<()> {
    let async_dev = tun::AsyncDevice::new(tun_dev)
        .map_err(|e| anyhow::anyhow!("failed to create async TUN device: {e}"))?;
    let (mut tun_reader, mut tun_writer) = tokio::io::split(async_dev);

    let mut reconnect_delay = Duration::from_secs(1);
    let mut cached_session: Option<Vec<u8>> = None;
    clear_ready_file(tunnel_cfg.ready_file.as_deref());

    loop {
        events::event(
            Severity::Info,
            &format!("[connecting] {} ...", tunnel_cfg.endpoint),
        );

        let mut connected_at = None;
        let session_result = run_tunnel_session(
            config,
            tunnel_cfg,
            &mut tun_reader,
            &mut tun_writer,
            &mut connected_at,
            &mut cached_session,
        )
        .await;
        clear_ready_file(tunnel_cfg.ready_file.as_deref());

        match &session_result {
            Ok(()) => events::event(Severity::Info, "[disconnected] Session ended"),
            Err(e) => events::event(Severity::Warning, &format!("[error] {e:#}")),
        }
        let connected_for = connected_at.as_ref().map(Instant::elapsed);
        let (wait, next_delay) =
            reconnect_backoff(connected_for, session_result.is_ok(), reconnect_delay);
        if !wait.is_zero() {
            events::event(
                Severity::Info,
                &format!("[retry] Waiting {}s before reconnecting", wait.as_secs()),
            );
            tokio::time::sleep(wait).await;
        }
        reconnect_delay = next_delay;
    }
}

async fn run_tunnel_session<R, W>(
    config: &Config,
    tunnel_cfg: &TunnelConfig,
    tun_reader: &mut R,
    tun_writer: &mut W,
    connected_at: &mut Option<Instant>,
    cached_session: &mut Option<Vec<u8>>,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut offered_session = cached_session.take();
    let tls_material = tls::prepare_tls_material(config)?;

    let mut quic_config = quiche::Config::new(quiche::PROTOCOL_VERSION)
        .map_err(|e| anyhow::anyhow!("quiche config: {e}"))?;

    let idle = idle_timeout(tunnel_cfg.keepalive_period);
    configure_quic(&mut quic_config, idle, &tunnel_cfg.congestion_control)?;
    let endpoint_pub_key_spki_der = tls_material.load_into(&mut quic_config)?;

    let bind_addr: SocketAddr = match tunnel_cfg.endpoint {
        SocketAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        SocketAddr::V6(_) => "[::]:0".parse().unwrap(),
    };

    let socket = tokio::net::UdpSocket::bind(bind_addr).await?;
    socket.connect(tunnel_cfg.endpoint).await?;
    let local_addr = socket.local_addr()?;

    let mut scid = [0u8; quiche::MAX_CONN_ID_LEN];
    ring::rand::SystemRandom::new()
        .fill(&mut scid)
        .map_err(|_| anyhow::anyhow!("RNG failure"))?;
    let scid = quiche::ConnectionId::from_ref(&scid);

    let mut conn = quiche::connect(
        Some(&tunnel_cfg.sni),
        &scid,
        local_addr,
        tunnel_cfg.endpoint,
        &mut quic_config,
    )
    .map_err(|e| anyhow::anyhow!("quiche connect: {e}"))?;

    restore_quic_session(&mut conn, &mut offered_session);

    let mut out = vec![0u8; MAX_DATAGRAM_SIZE];
    let mut buf = vec![0u8; 65535];

    let (write, send_info) = conn
        .send(&mut out)
        .map_err(|e| anyhow::anyhow!("initial send: {e}"))?;
    send_paced_packet(&socket, &out[..write], send_info).await?;

    // Complete handshake with a hard deadline (or enter 0-RTT early data when
    // resuming a 0-RTT-capable session ticket). The idle timeout (three
    // keepalives) would end a blackholed handshake far later than this.
    let handshake_started = Instant::now();
    while !(conn.is_established() || conn.is_in_early_data()) {
        if handshake_started.elapsed() >= HANDSHAKE_TIMEOUT {
            bail!("timed out during QUIC handshake");
        }
        let remaining = HANDSHAKE_TIMEOUT.saturating_sub(handshake_started.elapsed());
        let timeout = conn
            .timeout()
            .unwrap_or(Duration::from_millis(100))
            .min(remaining);

        tokio::select! {
            result = socket.recv(&mut buf) => {
                let len = result?;
                let recv_info = quiche::RecvInfo {
                    to: local_addr,
                    from: tunnel_cfg.endpoint,
                };
                conn.recv(&mut buf[..len], recv_info).ok();
            }
            () = tokio::time::sleep(timeout) => {
                conn.on_timeout();
            }
        }

        loop {
            match conn.send(&mut out) {
                Ok((write, send_info)) => {
                    send_paced_packet(&socket, &out[..write], send_info).await?;
                }
                Err(quiche::Error::Done) => break,
                Err(e) => bail!("send during handshake: {e}"),
            }
        }

        if conn.is_closed() {
            bail!("connection closed during handshake");
        }
    }

    let sent_in_early_data = conn.is_in_early_data() && !conn.is_established();

    // Verify endpoint key pinning
    let peer_cert = conn
        .peer_cert()
        .context("WARP endpoint did not provide a peer certificate")?;
    if !tls::verify_endpoint_key(peer_cert, &endpoint_pub_key_spki_der) {
        bail!("peer certificate public key does not match pinned endpoint key");
    }
    log::debug!("Endpoint key pinning verified");

    // Set up HTTP/3
    let mut h3_config = quiche::h3::Config::new().map_err(|e| anyhow::anyhow!("h3 config: {e}"))?;
    h3_config.enable_extended_connect(true);

    let mut h3_conn = quiche::h3::Connection::with_transport(&mut conn, &h3_config)
        .map_err(|e| anyhow::anyhow!("h3 connection: {e}"))?;

    // Send CONNECT request for cf-connect-ip
    let req = vec![
        quiche::h3::Header::new(b":method", b"CONNECT"),
        quiche::h3::Header::new(b":protocol", b"cf-connect-ip"),
        quiche::h3::Header::new(b":scheme", b"https"),
        quiche::h3::Header::new(b":authority", b"cloudflareaccess.com"),
        quiche::h3::Header::new(b":path", b"/"),
        quiche::h3::Header::new(b"capsule-protocol", b"?1"),
        quiche::h3::Header::new(b"user-agent", b""),
    ];

    let stream_id = h3_conn
        .send_request(&mut conn, &req, false)
        .map_err(|e| anyhow::anyhow!("send CONNECT request: {e}"))?;

    let flow_id = stream_id / 4;
    log::debug!("CONNECT request sent on stream {stream_id}, flow_id={flow_id}");

    loop {
        match conn.send(&mut out) {
            Ok((write, send_info)) => {
                send_paced_packet(&socket, &out[..write], send_info).await?;
            }
            Err(quiche::Error::Done) => break,
            Err(e) => bail!("send after CONNECT: {e}"),
        }
    }

    // Wait for 2xx
    let mut connect_established = false;
    let connect_started = Instant::now();
    for _ in 0..100 {
        if connect_started.elapsed() >= HANDSHAKE_TIMEOUT {
            bail!("timed out waiting for CONNECT response");
        }
        let remaining = HANDSHAKE_TIMEOUT.saturating_sub(connect_started.elapsed());
        let timeout = conn
            .timeout()
            .unwrap_or(Duration::from_millis(100))
            .min(remaining);

        tokio::select! {
            result = socket.recv(&mut buf) => {
                let len = result?;
                let recv_info = quiche::RecvInfo {
                    to: local_addr,
                    from: tunnel_cfg.endpoint,
                };
                conn.recv(&mut buf[..len], recv_info).ok();
            }
            () = tokio::time::sleep(timeout) => {
                conn.on_timeout();
            }
        }

        if sent_in_early_data
            && !conn.is_in_early_data()
            && (!conn.is_established() || !conn.is_resumed())
        {
            bail!("server rejected 0-RTT early data; reconnecting with full handshake");
        }

        loop {
            match h3_conn.poll(&mut conn) {
                Ok((sid, quiche::h3::Event::Headers { list, has_body: _ })) if sid == stream_id => {
                    for h in &list {
                        if h.name() == b":status" {
                            let status = std::str::from_utf8(h.value()).unwrap_or("?");
                            log::debug!("CONNECT response status: {status}");
                            if status.starts_with('2') {
                                connect_established = true;
                            } else {
                                bail!("CONNECT rejected with status {status}");
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(quiche::h3::Error::Done) => break,
                Err(e) => bail!("h3 poll error: {e}"),
            }
        }

        // Flush
        loop {
            match conn.send(&mut out) {
                Ok((write, send_info)) => {
                    send_paced_packet(&socket, &out[..write], send_info).await?;
                }
                Err(quiche::Error::Done) => break,
                Err(e) => bail!("send during CONNECT wait: {e}"),
            }
        }

        if connect_established && conn.is_established() {
            break;
        }
        if conn.is_closed() {
            bail!("connection closed before CONNECT response");
        }
    }

    if !connect_established {
        bail!("timed out waiting for CONNECT response");
    }

    if sent_in_early_data {
        let peer_cert = conn
            .peer_cert()
            .context("WARP endpoint did not provide a peer certificate after 0-RTT")?;
        if !tls::verify_endpoint_key(peer_cert, &endpoint_pub_key_spki_der) {
            bail!("peer certificate public key does not match pinned endpoint key");
        }
    }

    *cached_session = conn.session().map(<[u8]>::to_vec).or(offered_session);

    events::event(Severity::Info, "[connected] MASQUE tunnel established");
    mark_ready(tunnel_cfg.ready_file.as_deref())?;
    *connected_at = Some(Instant::now());

    let stats = Stats::new();
    let session_start = Instant::now();

    // The status line is redrawn every second: useful on a terminal, but noise
    // in a log, and a wakeup and write per second when stderr is discarded.
    let _stats_guard = events::stderr_is_terminal().then(|| {
        let stats_display = stats.clone();
        AbortTaskOnDrop(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let tx_p = stats_display.tx_packets.load(Ordering::Relaxed);
                let rx_p = stats_display.rx_packets.load(Ordering::Relaxed);
                let tx_b = stats_display.tx_bytes.load(Ordering::Relaxed);
                let rx_b = stats_display.rx_bytes.load(Ordering::Relaxed);
                let dropped = stats_display.dropped.load(Ordering::Relaxed);
                let lost = stats_display.quic_lost.load(Ordering::Relaxed);
                let retrans = stats_display.quic_retrans.load(Ordering::Relaxed);
                let uptime = session_start.elapsed();
                events::status(&format!(
                    "[connected {}] tx: {} ({})  rx: {} ({})  drop: {}  lost: {}  retrans: {}",
                    format_duration(uptime),
                    tx_p,
                    format_bytes(tx_b),
                    rx_p,
                    format_bytes(rx_b),
                    dropped,
                    lost,
                    retrans,
                ));
            }
        }))
    });

    // Build the flow_id varint prefix + context_id zero
    let mut flow_prefix = Vec::with_capacity(16);
    {
        let mut tmp = [0u8; 8];
        let mut b = octets::OctetsMut::with_slice(&mut tmp);
        b.put_varint(flow_id).unwrap();
        let len = b.off();
        flow_prefix.extend_from_slice(&tmp[..len]);
    }
    flow_prefix.push(0x00);

    // Main data forwarding loop
    let mtu = tunnel_cfg.mtu as usize;
    let mut tun_buf = vec![0u8; mtu + 128];
    let mut dgram_buf = vec![0u8; mtu + flow_prefix.len() + 128];
    let keepalive_interval = tunnel_cfg.keepalive_period;
    let mut last_keepalive = Instant::now();
    let mut last_quic_stats_update = Instant::now();
    let mut internal_work_pending = false;
    let mut paced_packet: Option<(Vec<u8>, quiche::SendInfo)> = None;
    let mut tun_write_failures = 0;
    // With --upload-codel, upload datagrams that wait too long are dropped.
    let mut upload_queue = (!tunnel_cfg.upload_codel.is_zero()).then(|| {
        UploadQueue::new(
            tunnel_cfg.upload_codel,
            mtu + flow_prefix.len(),
            Instant::now(),
        )
    });

    let result: Result<()> = 'session: loop {
        let timeout = event_loop_timeout(conn.timeout(), keepalive_interval);
        let paced_send_delay = paced_packet
            .as_ref()
            .map_or(keepalive_interval, |(_, info)| {
                pacing_delay(info.at, Instant::now())
            });

        tokio::select! {
            // Read from TUN -> send to QUIC
            result = tokio::io::AsyncReadExt::read(tun_reader, &mut tun_buf),
                if tun_read_allowed(conn.dgram_send_queue_len(), tunnel_cfg.tun_backpressure) => {
                let n = result?;
                if n == 0 {
                    bail!("TUN device closed");
                }

                let pkt = &mut tun_buf[..n];
                match packet::prepare_outgoing(pkt) {
                    Ok(_) => {
                        let pkt_len = n as u64;
                        let mut dgram = Vec::with_capacity(flow_prefix.len() + n);
                        dgram.extend_from_slice(&flow_prefix);
                        dgram.extend_from_slice(pkt);

                        let dgram_len = dgram.len();
                        match conn.dgram_send_vec(dgram) {
                            Ok(()) => {
                                stats.tx_packets.fetch_add(1, Ordering::Relaxed);
                                stats.tx_bytes.fetch_add(pkt_len, Ordering::Relaxed);
                                if let Some(queue) = upload_queue.as_mut() {
                                    queue.push(Instant::now(), dgram_len);
                                }
                            }
                            Err(quiche::Error::InvalidState) => {
                                log::warn!("datagram send: peer doesn't support datagrams");
                            }
                            Err(quiche::Error::Done) => {
                                stats.dropped.fetch_add(1, Ordering::Relaxed);
                                log::trace!("datagram send queue full, dropping packet");
                            }
                            Err(e) => {
                                stats.dropped.fetch_add(1, Ordering::Relaxed);
                                log::debug!("datagram send error: {e}, generating ICMP");
                                let icmp_mtu = effective_datagram_ip_mtu(
                                    conn.dgram_max_writable_len(),
                                    flow_prefix.len(),
                                );
                                if let Some(icmp_pkt) =
                                    icmp::compose_icmp_too_large(&tun_buf[..n], icmp_mtu)
                                {
                                    write_tun(
                                        tun_writer,
                                        &icmp_pkt,
                                        &stats,
                                        &mut tun_write_failures,
                                    )
                                    .await?;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        stats.dropped.fetch_add(1, Ordering::Relaxed);
                        log::trace!("dropping outgoing packet: {e}");
                    }
                }
            }

            // Read from QUIC socket
            result = socket.recv(&mut buf) => {
                let len = result?;
                let recv_info = quiche::RecvInfo {
                    to: local_addr,
                    from: tunnel_cfg.endpoint,
                };
                if let Err(e) = conn.recv(&mut buf[..len], recv_info) {
                    log::debug!("quic recv error: {e}");
                }
            }

            // Timeout handling
            () = tokio::time::sleep(timeout) => {
                conn.on_timeout();
            }

            // Continue a capped internal batch without waiting for a new
            // socket/TUN event. yield_now keeps this branch cooperative and
            // lets the other ready branches win fairly.
            () = tokio::task::yield_now(), if internal_work_pending => {}

            // quiche's SendInfo::at is a congestion-control pacing deadline.
            // Holding one packet until that deadline prevents packet bursts,
            // loss-recovery collapse, and a compute-only conn.send() loop.
            () = tokio::time::sleep(paced_send_delay), if paced_packet.is_some() => {
                let (packet, send_info) = paced_packet
                    .take()
                    .expect("paced packet must exist when its timer is enabled");
                if let Err(error) = socket.send_to(&packet, send_info.to).await {
                    log::warn!("paced UDP send error: {error}");
                }
            }
        }
        internal_work_pending = false;

        // Keep the campus/WAN UDP mapping alive and make a dead peer observable
        // even while no client packets are crossing the tunnel.
        if last_keepalive.elapsed() >= keepalive_interval {
            if let Err(error) = conn.send_ack_eliciting() {
                log::warn!("failed to schedule QUIC keepalive: {error}");
            }
            last_keepalive = Instant::now();
        }

        // After any event, drain all pending UDP packets from the socket.
        // This prevents stale ACKs and reduces unnecessary retransmissions
        // when the TUN or timeout branch wins the select.
        for index in 0..MAX_FORWARDING_BATCH {
            match socket.try_recv(&mut buf) {
                Ok(len) => {
                    let recv_info = quiche::RecvInfo {
                        to: local_addr,
                        from: tunnel_cfg.endpoint,
                    };
                    conn.recv(&mut buf[..len], recv_info).ok();
                    if forwarding_batch_limit_reached(index) {
                        internal_work_pending = true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }

        // Process H3 events (capsules, etc.)
        for index in 0..MAX_FORWARDING_BATCH {
            match h3_conn.poll(&mut conn) {
                Ok((sid, event)) => match connect_stream_end(sid, stream_id, &event) {
                    Some(ConnectStreamEnd::Finished) => break 'session Ok(()),
                    Some(ConnectStreamEnd::Reset(code)) => {
                        break 'session Err(anyhow::anyhow!(
                            "server reset the CONNECT stream (code {code:#x})"
                        ));
                    }
                    None => {}
                },
                Err(quiche::h3::Error::Done) => break,
                Err(e) => {
                    log::warn!("h3 poll error: {e}");
                    break;
                }
            }
            if forwarding_batch_limit_reached(index) {
                internal_work_pending = true;
            }
        }

        // Drain received datagrams -> TUN
        for index in 0..MAX_FORWARDING_BATCH {
            match conn.dgram_recv(&mut dgram_buf) {
                Ok(len) => {
                    if let Some(ip_payload) = parse_datagram(&dgram_buf[..len], flow_id) {
                        if packet::validate_incoming(ip_payload).is_ok() {
                            stats.rx_packets.fetch_add(1, Ordering::Relaxed);
                            stats
                                .rx_bytes
                                .fetch_add(ip_payload.len() as u64, Ordering::Relaxed);
                            write_tun(tun_writer, ip_payload, &stats, &mut tun_write_failures)
                                .await?;
                        }
                    }
                }
                Err(quiche::Error::Done) => break,
                Err(e) => {
                    log::debug!("dgram recv error: {e}");
                    break;
                }
            }
            if forwarding_batch_limit_reached(index) {
                internal_work_pending = true;
            }
        }

        if let Some(queue) = upload_queue.as_mut() {
            drop_stale_uploads(queue, &mut conn, &stats);
        }

        // Flush an immediately-due QUIC batch. If quiche schedules a packet in
        // the future, retain that packet and resume output from the select loop
        // when its pacing deadline arrives.
        if paced_packet.is_none() {
            for index in 0..MAX_FORWARDING_BATCH {
                match conn.send(&mut out) {
                    Ok((write, send_info)) => {
                        if pacing_delay(send_info.at, Instant::now()).is_zero() {
                            if let Err(e) = socket.send_to(&out[..write], send_info.to).await {
                                log::warn!("UDP send error: {e}");
                                break;
                            }
                        } else {
                            paced_packet = Some((out[..write].to_vec(), send_info));
                            break;
                        }
                    }
                    Err(quiche::Error::Done) => break,
                    Err(e) => {
                        log::error!("quic send error: {e}");
                        bail!("quic send error: {e}");
                    }
                }
                if forwarding_batch_limit_reached(index) {
                    internal_work_pending = true;
                }
            }
        }

        // Stats are display-only; avoid querying the connection on every packet.
        if last_quic_stats_update.elapsed() >= Duration::from_secs(1) {
            let qs = conn.stats();
            stats.quic_lost.store(qs.lost as u64, Ordering::Relaxed);
            stats
                .quic_retrans
                .store(qs.retrans as u64, Ordering::Relaxed);
            if let Some(session) = conn.session() {
                *cached_session = Some(session.to_vec());
            }
            last_quic_stats_update = Instant::now();
        }

        if conn.is_closed() {
            if conn.is_timed_out() {
                break Err(anyhow::anyhow!(
                    "no packets from the server for {}s",
                    idle.as_secs()
                ));
            }
            break Ok(());
        }
    };

    if let Some(session) = conn.session() {
        *cached_session = Some(session.to_vec());
    }

    result
}

/// Parse an H3 datagram: `varint(flow_id)` + `varint(context_id)` + IP packet
/// Returns the IP payload slice if `flow_id` matches and `context_id` == 0.
pub(crate) fn parse_datagram(dgram: &[u8], expected_flow_id: u64) -> Option<&[u8]> {
    let mut b = octets::Octets::with_slice(dgram);

    let fid = b.get_varint().ok()?;
    if fid != expected_flow_id {
        return None;
    }

    let ctx_id = b.get_varint().ok()?;
    if ctx_id != 0 {
        return None;
    }

    let off = b.off();
    if off >= dgram.len() {
        return None;
    }

    Some(&dgram[off..])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_datagram tests ----

    fn encode_varint(val: u64) -> Vec<u8> {
        let mut tmp = [0u8; 8];
        let mut b = octets::OctetsMut::with_slice(&mut tmp);
        b.put_varint(val).unwrap();
        let len = b.off();
        tmp[..len].to_vec()
    }

    fn make_datagram(flow_id: u64, context_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut dgram = Vec::new();
        dgram.extend_from_slice(&encode_varint(flow_id));
        dgram.extend_from_slice(&encode_varint(context_id));
        dgram.extend_from_slice(payload);
        dgram
    }

    #[test]
    fn parse_datagram_valid() {
        let payload = b"hello world";
        let dgram = make_datagram(0, 0, payload);
        let result = parse_datagram(&dgram, 0);
        assert_eq!(result, Some(payload.as_ref()));
    }

    #[test]
    fn parse_datagram_flow_id_mismatch() {
        let dgram = make_datagram(1, 0, b"data");
        assert_eq!(parse_datagram(&dgram, 0), None);
    }

    #[test]
    fn parse_datagram_nonzero_context() {
        let dgram = make_datagram(0, 1, b"data");
        assert_eq!(parse_datagram(&dgram, 0), None);
    }

    #[test]
    fn parse_datagram_empty_payload() {
        let dgram = make_datagram(0, 0, b"");
        // Empty payload means off == dgram.len(), should return None
        assert_eq!(parse_datagram(&dgram, 0), None);
    }

    #[test]
    fn parse_datagram_large_flow_id() {
        // flow_id that requires 4-byte varint encoding
        let flow_id = 16384;
        let payload = vec![0xABu8; 1300];
        let dgram = make_datagram(flow_id, 0, &payload);
        let result = parse_datagram(&dgram, flow_id);
        assert_eq!(result, Some(payload.as_ref()));
    }

    #[test]
    fn parse_datagram_truncated() {
        // Just a single byte - can't even decode flow_id
        let dgram = vec![0xFF];
        assert_eq!(parse_datagram(&dgram, 0), None);
    }

    // ---- format_bytes tests ----

    #[test]
    fn format_bytes_values() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }

    // ---- format_duration tests ----

    #[test]
    fn format_duration_values() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
        assert_eq!(format_duration(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h 00m 00s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 01m 01s");
    }

    #[test]
    fn event_loop_timeout_prevents_zero_duration_spin() {
        let keepalive = Duration::from_secs(30);
        assert_eq!(event_loop_timeout(None, keepalive), keepalive);
        assert_eq!(
            event_loop_timeout(Some(Duration::ZERO), keepalive),
            MIN_EVENT_LOOP_SLEEP
        );
        assert_eq!(
            event_loop_timeout(Some(Duration::from_millis(2)), keepalive),
            MIN_EVENT_LOOP_SLEEP
        );
        assert_eq!(
            event_loop_timeout(Some(Duration::from_millis(20)), keepalive),
            Duration::from_millis(20)
        );
        assert_eq!(
            event_loop_timeout(Some(Duration::from_secs(60)), keepalive),
            keepalive
        );
    }

    #[test]
    fn forwarding_batch_only_schedules_continuation_at_its_limit() {
        assert!(!forwarding_batch_limit_reached(0));
        assert!(!forwarding_batch_limit_reached(MAX_FORWARDING_BATCH - 2));
        assert!(forwarding_batch_limit_reached(MAX_FORWARDING_BATCH - 1));
    }

    // ---- upload queue backpressure ----

    #[test]
    fn tun_reads_pause_only_when_backpressure_is_on_and_the_queue_is_full() {
        // F10: 1,000 queued datagrams (~1.3 MB) is upload latency the qdisc never sees.
        assert!(tun_read_allowed(1000, 0));
        assert!(tun_read_allowed(31, 32));
        assert!(!tun_read_allowed(32, 32));
        assert!(!tun_read_allowed(1000, 32));
    }

    // ---- upload CoDel ----

    const TARGET: Duration = Duration::from_millis(20);
    // The largest upload datagram at the default MTU: 1,280 bytes and the prefix.
    const DATAGRAM: usize = 1282;

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Whether the oldest datagram is dropped at `at` ms, having waited `wait`
    /// with a standing queue behind it.
    fn drops(codel: &mut Codel, start: Instant, at: u64, wait: Duration) -> bool {
        codel.should_drop(start + ms(at), Some((wait, 10 * DATAGRAM)))
    }

    /// The milliseconds, from `from` to `to`, at which the oldest datagram is
    /// dropped when it has always waited `wait`.
    fn drop_times(
        codel: &mut Codel,
        start: Instant,
        from: u64,
        to: u64,
        wait: Duration,
    ) -> Vec<u64> {
        (from..=to)
            .filter(|&at| drops(codel, start, at, wait))
            .collect()
    }

    #[test]
    fn a_queue_under_its_target_is_never_dropped() {
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        assert_eq!(
            drop_times(&mut codel, start, 0, 2000, ms(19)),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn the_oldest_datagram_goes_once_the_queue_stays_above_target_for_an_interval() {
        // F10: nothing bounded how long a datagram waited (up to 1,000 queued).
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        assert!(!drops(&mut codel, start, 0, ms(25)));
        assert!(!drops(&mut codel, start, 99, ms(124)));
        assert!(drops(&mut codel, start, 100, ms(125)));
    }

    #[test]
    fn drops_come_closer_together_while_the_queue_stays_above_target() {
        // RFC 8289's control law: the n-th drop comes interval/sqrt(n) after the last.
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        assert_eq!(
            drop_times(&mut codel, start, 0, 500, ms(50)),
            [100, 200, 271, 329, 379, 424, 464]
        );
    }

    #[test]
    fn dropping_stops_once_the_oldest_datagram_is_back_under_target() {
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        assert_eq!(drop_times(&mut codel, start, 0, 150, ms(30)), [100]);
        assert!(!drops(&mut codel, start, 151, ms(10)));
        // Above target again: a whole interval passes before the next drop.
        assert_eq!(drop_times(&mut codel, start, 152, 300, ms(30)), [252]);
    }

    #[test]
    fn a_short_queue_is_never_dropped_however_long_it_waits() {
        // A datagram or two waiting is a slow link, not a standing queue.
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        for step in 0..20 {
            let now = start + ms(100 * step);
            assert!(!codel.should_drop(now, Some((ms(500), 0))));
            assert!(!codel.should_drop(now, Some((ms(500), DATAGRAM))));
        }
        assert!(!codel.should_drop(start + ms(2000), None));
    }

    #[test]
    fn a_queue_back_above_target_soon_after_resumes_the_last_drop_rate() {
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        assert_eq!(drop_times(&mut codel, start, 0, 500, ms(50)).len(), 7);
        assert!(!drops(&mut codel, start, 510, ms(5)));
        // The last episode needed six drops beyond its first, so this one
        // starts with drops interval/sqrt(6) apart.
        assert_eq!(drop_times(&mut codel, start, 520, 690, ms(50)), [620, 661]);
    }

    #[test]
    fn a_queue_back_above_target_much_later_starts_again_at_one_drop_an_interval() {
        let start = Instant::now();
        let mut codel = Codel::new(TARGET, DATAGRAM, start);
        assert_eq!(drop_times(&mut codel, start, 0, 500, ms(50)).len(), 7);
        assert!(!drops(&mut codel, start, 510, ms(5)));
        assert_eq!(
            drop_times(&mut codel, start, 2520, 2790, ms(50)),
            [2620, 2720]
        );
    }

    /// 100 datagrams of 1,000 bytes queued at `start`, none sent; at 50 ms the
    /// oldest went above target.
    fn standing_upload_queue(start: Instant) -> UploadQueue {
        let mut queue = UploadQueue::new(TARGET, DATAGRAM, start);
        for _ in 0..100 {
            queue.push(start, 1000);
        }
        assert_eq!(queue.stale(start + ms(50), 100), 0);
        queue
    }

    #[test]
    fn datagrams_quiche_has_sent_are_forgotten_oldest_first() {
        let start = Instant::now();
        let mut queue = UploadQueue::new(TARGET, DATAGRAM, start);
        for step in 0..5 {
            queue.push(start + ms(10 * step), 1000);
        }
        // quiche sent the two oldest, so the oldest left was queued at 20 ms.
        assert_eq!(queue.stale(start + ms(45), 3), 0);
        assert_eq!(queue.oldest(start + ms(45)), Some((ms(25), 2000)));
    }

    #[test]
    fn a_standing_upload_queue_loses_its_oldest_datagrams() {
        let start = Instant::now();
        let mut queue = standing_upload_queue(start);
        // quiche sent nothing (its window is full), so the queue still stands.
        assert_eq!(queue.stale(start + ms(150), 100), 1);
        assert_eq!(queue.oldest(start + ms(150)), Some((ms(150), 98_000)));
        assert_eq!(queue.stale(start + ms(249), 99), 0);
        assert_eq!(queue.stale(start + ms(250), 99), 1);
    }

    #[test]
    fn an_upload_queue_that_keeps_draining_is_left_alone() {
        // However long the queue, datagrams that leave within the target stay.
        let start = Instant::now();
        let mut queue = UploadQueue::new(TARGET, DATAGRAM, start);
        let mut queued = 0;
        for step in 0..2000 {
            let now = start + ms(step);
            for _ in 0..10 {
                queue.push(now, DATAGRAM);
            }
            // quiche sends all but the newest 15 ms of datagrams.
            queued = (queued + 10).min(150);
            assert_eq!(queue.stale(now, queued), 0, "at {step} ms");
        }
    }

    #[test]
    fn upload_drops_are_reported_once_they_stop() {
        // A measurement restarts the client soon after its upload phase, so
        // the drops must be in the log by then.
        let start = Instant::now();
        let mut queue = standing_upload_queue(start);
        assert_eq!(queue.report(start + ms(100)), None);
        assert_eq!(queue.stale(start + ms(150), 100), 1);
        assert_eq!(queue.stale(start + ms(250), 99), 1);
        assert_eq!(queue.report(start + ms(5249)), None);
        assert_eq!(
            queue.report(start + ms(5250)).as_deref(),
            Some("[upload] CoDel dropped 2 datagrams in the last 5s (target 20 ms)")
        );
        assert_eq!(queue.report(start + ms(9000)), None);
    }

    #[test]
    fn upload_drops_are_reported_at_most_once_a_minute() {
        // A long upload: datagrams keep coming, quiche sends none, and CoDel
        // keeps dropping. One line a minute, so the RAM log is not flooded.
        let start = Instant::now();
        let mut queue = standing_upload_queue(start);
        let mut held = 100;
        let mut reports = Vec::new();
        for at in (150..=150_000).step_by(10) {
            let now = start + ms(at);
            queue.push(now, 1000);
            held += 1;
            held -= queue.stale(now, held);
            if let Some(report) = queue.report(now) {
                reports.push((at, report));
            }
        }
        assert_eq!(reports.len(), 2, "{reports:?}");
        assert_eq!(reports[0].0, 60_150);
        assert!(reports[1].0 - reports[0].0 >= 60_000, "{reports:?}");
        for (_, report) in &reports {
            assert!(
                report.starts_with("[upload] CoDel dropped ")
                    && report.ends_with(" datagrams in the last 60s (target 20 ms)"),
                "{report}"
            );
        }
    }

    #[test]
    fn upload_drop_reports_stay_a_minute_apart_when_drops_come_and_go() {
        // Two seconds of overload in every ten: each burst of drops stops, but
        // the log still gets one line a minute at most.
        let start = Instant::now();
        let mut queue = UploadQueue::new(TARGET, DATAGRAM, start);
        let mut held = 0;
        let mut reports = Vec::new();
        for at in (0..150_000).step_by(10) {
            let now = start + ms(at);
            queue.push(now, 1000);
            held += 1;
            if at % 10_000 >= 2_000 {
                // quiche sent everything queued.
                held = 0;
            }
            held -= queue.stale(now, held);
            if queue.report(now).is_some() {
                reports.push(at);
            }
        }
        assert_eq!(reports.len(), 3, "{reports:?}");
        assert!(
            reports.windows(2).all(|pair| pair[1] - pair[0] >= 60_000),
            "{reports:?}"
        );
    }

    // ---- quiche's datagram send queue, as the upload CoDel relies on it ----

    /// Hand every packet that `from` sends now to `to`.
    fn deliver(from: &mut quiche::Connection, to: &mut quiche::Connection) {
        let mut packet = [0u8; MAX_DATAGRAM_SIZE];
        for _ in 0..100 {
            match from.send(&mut packet) {
                Ok((len, info)) => {
                    let info = quiche::RecvInfo {
                        from: info.from,
                        to: info.to,
                    };
                    to.recv(&mut packet[..len], info).expect("recv");
                }
                Err(quiche::Error::Done) => return,
                Err(error) => panic!("send: {error}"),
            }
        }
        panic!("more than 100 packets at once");
    }

    /// A client and a server that completed their handshake in memory, both
    /// with datagrams on. The server's certificate is a throwaway.
    fn connected_pair() -> (quiche::Connection, quiche::Connection) {
        use std::io::Write;

        let config = |server: bool| {
            let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).expect("config");
            config.set_application_protos(&[b"test"]).expect("alpn");
            config.verify_peer(false);
            config.enable_dgram(true, 1000, 1000);
            if server {
                let key_pair = rcgen::KeyPair::generate().expect("test key");
                let cert = rcgen::CertificateParams::new(Vec::<String>::new())
                    .expect("test params")
                    .self_signed(&key_pair)
                    .expect("test cert");
                let mut cert_file = tempfile::NamedTempFile::new().expect("temp cert");
                cert_file
                    .write_all(cert.pem().as_bytes())
                    .expect("write cert");
                let mut key_file = tempfile::NamedTempFile::new().expect("temp key");
                key_file
                    .write_all(key_pair.serialize_pem().as_bytes())
                    .expect("write key");
                config
                    .load_cert_chain_from_pem_file(cert_file.path().to_str().expect("path"))
                    .expect("load cert");
                config
                    .load_priv_key_from_pem_file(key_file.path().to_str().expect("path"))
                    .expect("load key");
            }
            config
        };
        let client_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:4434".parse().unwrap();
        let mut client = quiche::connect(
            None,
            &quiche::ConnectionId::from_ref(&[1; 16]),
            client_addr,
            server_addr,
            &mut config(false),
        )
        .expect("connect");
        let mut server = quiche::accept(
            &quiche::ConnectionId::from_ref(&[2; 16]),
            None,
            server_addr,
            client_addr,
            &mut config(true),
        )
        .expect("accept");
        for _ in 0..10 {
            deliver(&mut client, &mut server);
            deliver(&mut server, &mut client);
            if client.is_established() && server.is_established() {
                return (client, server);
            }
        }
        panic!("the in-memory handshake did not complete");
    }

    /// The first byte of each datagram the connection has received.
    fn received(conn: &mut quiche::Connection) -> Vec<u8> {
        let mut dgram = [0u8; MAX_DATAGRAM_SIZE];
        let mut ids = Vec::new();
        while let Ok(len) = conn.dgram_recv(&mut dgram) {
            assert!(len > 0);
            ids.push(dgram[0]);
        }
        ids
    }

    #[test]
    fn quiche_sends_queued_datagrams_oldest_first() {
        // UploadQueue relies on it: whatever has left came from the front.
        let (mut client, mut server) = connected_pair();
        for id in 0..5u8 {
            client.dgram_send(&[id; 1000]).expect("queue");
        }
        let mut packet = [0u8; MAX_DATAGRAM_SIZE];
        let mut seen = Vec::new();
        for _ in 0..20 {
            if let Ok((len, info)) = client.send(&mut packet) {
                let info = quiche::RecvInfo {
                    from: info.from,
                    to: info.to,
                };
                server.recv(&mut packet[..len], info).expect("recv");
            }
            seen.extend(received(&mut server));
            let left = 5 - client.dgram_send_queue_len() as u8;
            assert_eq!(seen, (0..left).collect::<Vec<u8>>());
        }
        assert_eq!(seen, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn dropping_the_oldest_datagrams_leaves_the_rest_to_be_sent_in_order() {
        let (mut client, mut server) = connected_pair();
        for id in 0..10u8 {
            client.dgram_send(&[id]).expect("queue");
        }
        drop_oldest_datagrams(&mut client, 3);
        assert_eq!(client.dgram_send_queue_len(), 7);
        deliver(&mut client, &mut server);
        assert_eq!(received(&mut server), [3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn stale_uploads_leave_quiche_and_count_as_dropped() {
        let (mut client, mut server) = connected_pair();
        let queued_at = Instant::now();
        let first_check = queued_at + ms(500);
        let mut queue = UploadQueue::new(TARGET, DATAGRAM, queued_at);
        for id in 0..10u8 {
            client.dgram_send(&[id; 200]).expect("queue");
            queue.push(queued_at, 200);
        }
        let stats = Stats::new();
        drop_stale_uploads_at(first_check, &mut queue, &mut client, &stats);
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 0);
        // Still above target an interval later: the oldest goes.
        drop_stale_uploads_at(
            first_check + CODEL_INTERVAL,
            &mut queue,
            &mut client,
            &stats,
        );
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 1);
        assert_eq!(client.dgram_send_queue_len(), 9);
        deliver(&mut client, &mut server);
        assert_eq!(received(&mut server), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    // ---- dead-path detection ----

    #[test]
    fn idle_timeout_outlasts_two_lost_keepalives() {
        // F16: with no idle timeout only the shell watchdog noticed a dead path.
        assert_eq!(
            idle_timeout(Duration::from_secs(30)),
            Duration::from_secs(90)
        );
        assert!(idle_timeout(Duration::from_secs(1)) > Duration::from_secs(2));
    }

    #[test]
    fn only_the_connect_stream_ending_ends_the_session() {
        use quiche::h3::Event;
        assert_eq!(
            connect_stream_end(4, 4, &Event::Finished),
            Some(ConnectStreamEnd::Finished)
        );
        assert_eq!(
            connect_stream_end(4, 4, &Event::Reset(0x10c)),
            Some(ConnectStreamEnd::Reset(0x10c))
        );
        assert_eq!(connect_stream_end(8, 4, &Event::Finished), None);
        assert_eq!(connect_stream_end(4, 4, &Event::Data), None);
        assert_eq!(connect_stream_end(4, 4, &Event::GoAway), None);
    }

    // ---- reconnect backoff ----

    #[test]
    fn a_long_session_that_closes_cleanly_reconnects_at_once() {
        let (wait, next) = reconnect_backoff(Some(STABLE_SESSION), true, Duration::from_secs(8));
        assert_eq!(wait, Duration::ZERO);
        assert_eq!(next, Duration::from_secs(1));
    }

    #[test]
    fn a_short_session_that_closes_cleanly_backs_off() {
        // F17: a server that accepted and then closed at once caused a reconnect storm.
        let short = Some(Duration::from_secs(2));
        let (wait, next) = reconnect_backoff(short, true, Duration::from_secs(1));
        assert_eq!(
            (wait, next),
            (Duration::from_secs(1), Duration::from_secs(2))
        );
        let (wait, next) = reconnect_backoff(short, true, next);
        assert_eq!(
            (wait, next),
            (Duration::from_secs(2), Duration::from_secs(4))
        );
    }

    #[test]
    fn a_long_session_that_fails_restarts_the_backoff() {
        let (wait, next) = reconnect_backoff(Some(STABLE_SESSION), false, Duration::from_secs(16));
        assert_eq!(
            (wait, next),
            (Duration::from_secs(1), Duration::from_secs(2))
        );
    }

    #[test]
    fn failures_before_or_soon_after_connecting_keep_backing_off_to_the_cap() {
        let (wait, next) = reconnect_backoff(None, false, Duration::from_secs(16));
        assert_eq!((wait, next), (Duration::from_secs(16), MAX_RECONNECT_DELAY));
        let soon = Some(Duration::from_secs(1));
        let (wait, next) = reconnect_backoff(soon, false, MAX_RECONNECT_DELAY);
        assert_eq!((wait, next), (MAX_RECONNECT_DELAY, MAX_RECONNECT_DELAY));
    }

    // ---- TUN write failures ----

    struct FailingWriter;

    impl tokio::io::AsyncWrite for FailingWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Err(std::io::Error::other("TUN device gone")))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn persistent_tun_write_failures_are_counted_then_end_the_session() {
        let stats = Stats::new();
        let mut failures = 0;
        for _ in 1..MAX_CONSECUTIVE_TUN_WRITE_FAILURES {
            assert!(
                write_tun(&mut FailingWriter, b"packet", &stats, &mut failures)
                    .await
                    .is_ok()
            );
        }
        assert!(
            write_tun(&mut FailingWriter, b"packet", &stats, &mut failures)
                .await
                .is_err()
        );
        assert_eq!(
            stats.dropped.load(Ordering::Relaxed),
            u64::from(MAX_CONSECUTIVE_TUN_WRITE_FAILURES)
        );
    }

    #[tokio::test]
    async fn a_successful_tun_write_resets_the_failure_count() {
        let stats = Stats::new();
        let mut failures = MAX_CONSECUTIVE_TUN_WRITE_FAILURES - 1;
        let mut sink: Vec<u8> = Vec::new();
        assert!(write_tun(&mut sink, b"packet", &stats, &mut failures)
            .await
            .is_ok());
        assert_eq!(failures, 0);
        assert_eq!(sink, b"packet");
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn pacing_delay_waits_for_future_packets_without_rewaiting_past_packets() {
        let now = Instant::now();
        assert_eq!(pacing_delay(now, now), Duration::ZERO);
        assert_eq!(
            pacing_delay(now + Duration::from_millis(25), now),
            Duration::from_millis(25)
        );
        assert_eq!(
            pacing_delay(
                now.checked_sub(Duration::from_millis(25))
                    .expect("test instant should allow a 25 ms subtraction"),
                now
            ),
            Duration::ZERO
        );
    }

    // ---- WARP audit optimizations: I1 (asymmetric dgram queues), I4 (PMTUD), I2 (0-RTT) ----

    fn configured_test_pair() -> (quiche::Connection, quiche::Connection, Vec<u8>) {
        use std::io::Write;

        let key_pair = rcgen::KeyPair::generate().expect("test key");
        let cert = rcgen::CertificateParams::new(Vec::<String>::new())
            .expect("test params")
            .self_signed(&key_pair)
            .expect("test cert");
        let server_cert_der = cert.der().to_vec();
        let (_, parsed_cert) =
            x509_parser::parse_x509_certificate(&server_cert_der).expect("parse cert");
        let server_spki_der = parsed_cert.tbs_certificate.subject_pki.raw.to_vec();

        let make_config = |server: bool| {
            let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).expect("config");
            configure_quic(&mut config, Duration::from_secs(90), "cubic").expect("configure_quic");
            if server {
                let mut cert_file = tempfile::NamedTempFile::new().expect("temp cert");
                cert_file
                    .write_all(cert.pem().as_bytes())
                    .expect("write cert");
                let mut key_file = tempfile::NamedTempFile::new().expect("temp key");
                key_file
                    .write_all(key_pair.serialize_pem().as_bytes())
                    .expect("write key");
                config
                    .load_cert_chain_from_pem_file(cert_file.path().to_str().expect("path"))
                    .expect("load cert");
                config
                    .load_priv_key_from_pem_file(key_file.path().to_str().expect("path"))
                    .expect("load key");
            }
            config
        };

        let client_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:4434".parse().unwrap();
        let mut client_cfg = make_config(false);
        let mut server_cfg = make_config(true);

        let mut client = quiche::connect(
            Some("consumer-masque.cloudflareclient.com"),
            &quiche::ConnectionId::from_ref(&[1; 16]),
            client_addr,
            server_addr,
            &mut client_cfg,
        )
        .expect("connect");
        let mut server = quiche::accept(
            &quiche::ConnectionId::from_ref(&[2; 16]),
            None,
            server_addr,
            client_addr,
            &mut server_cfg,
        )
        .expect("accept");

        for _ in 0..10 {
            deliver(&mut client, &mut server);
            deliver(&mut server, &mut client);
        }
        assert!(client.is_established() && server.is_established());
        (client, server, server_spki_der)
    }

    #[test]
    fn asymmetric_dgram_queues_absorb_download_bursts_while_capping_upload_fifo() {
        // I1: send queue stays at 1,000 (anti-bufferbloat), while receive queue
        // expands to 8,192 so bursty downloads do not silently lose datagrams.
        let (mut client, mut server, _) = configured_test_pair();

        for i in 0..DGRAM_SEND_QUEUE_LEN {
            let payload = (i as u16).to_be_bytes();
            client.dgram_send(&payload).expect("send queue under limit");
        }
        assert_eq!(client.dgram_send_queue_len(), DGRAM_SEND_QUEUE_LEN);
        assert_eq!(client.dgram_send(&[0xFF; 2]), Err(quiche::Error::Done));
        drop_oldest_datagrams(&mut client, DGRAM_SEND_QUEUE_LEN);

        // Now deliver a 2,500-datagram download burst into the client's receive
        // queue without draining dgram_recv yet (> 2.5x the old 1,000 limit).
        let burst_count = 2_500usize;
        for i in 0..burst_count {
            let payload = (i as u16).to_be_bytes();
            server.dgram_send(&payload).expect("server queue");
            if server.dgram_send_queue_len() >= 32 {
                deliver(&mut server, &mut client);
                deliver(&mut client, &mut server);
            }
        }
        deliver(&mut server, &mut client);
        deliver(&mut client, &mut server);

        assert_eq!(client.dgram_recv_queue_len(), burst_count);
        let mut buf = [0u8; 16];
        for i in 0..burst_count {
            let len = client.dgram_recv(&mut buf).expect("recv burst dgram");
            assert_eq!(&buf[..len], &(i as u16).to_be_bytes());
        }
    }

    #[test]
    fn pmtud_probes_up_to_1350_and_expands_writable_datagram_mtu() {
        // I4: PMTUD probes from the 1,200-byte initial floor up to 1,350 bytes
        // after handshake confirmation, allowing >1,300-byte datagram payloads.
        let (mut client, mut server, _) = configured_test_pair();
        let max_writable = client.dgram_max_writable_len().expect("dgram writable");
        assert!(
            max_writable > 1300,
            "expected PMTUD-probed dgram_max_writable_len > 1300, got {max_writable}"
        );
        let large_dgram = vec![0x42u8; 1300];
        client
            .dgram_send(&large_dgram)
            .expect("send 1300-byte datagram");
        deliver(&mut client, &mut server);
        let mut recv_buf = vec![0u8; MAX_DATAGRAM_SIZE];
        let recv_len = server
            .dgram_recv(&mut recv_buf)
            .expect("recv 1300-byte datagram");
        assert_eq!(recv_len, 1300);
    }

    #[test]
    fn effective_datagram_ip_mtu_reflects_probed_payload_and_floors_at_1280() {
        assert_eq!(effective_datagram_ip_mtu(None, 2), 1280);
        assert_eq!(effective_datagram_ip_mtu(Some(1172), 2), 1280);
        assert_eq!(effective_datagram_ip_mtu(Some(1282), 2), 1280);
        assert_eq!(effective_datagram_ip_mtu(Some(1322), 2), 1320);
    }

    #[test]
    fn session_resumption_preserves_peer_cert_pin_and_enables_0rtt_early_data() {
        // I2: cache session ticket from a clean connection and resume with 0-RTT.
        use std::io::Write;

        let key_pair = rcgen::KeyPair::generate().expect("test key");
        let cert = rcgen::CertificateParams::new(Vec::<String>::new())
            .expect("test params")
            .self_signed(&key_pair)
            .expect("test cert");
        let server_cert_der = cert.der().to_vec();
        let (_, parsed_cert) =
            x509_parser::parse_x509_certificate(&server_cert_der).expect("parse cert");
        let server_spki_der = parsed_cert.tbs_certificate.subject_pki.raw.to_vec();

        let mut cert_file = tempfile::NamedTempFile::new().expect("temp cert");
        cert_file
            .write_all(cert.pem().as_bytes())
            .expect("write cert");
        let mut key_file = tempfile::NamedTempFile::new().expect("temp key");
        key_file
            .write_all(key_pair.serialize_pem().as_bytes())
            .expect("write key");

        let mut server_cfg = quiche::Config::new(quiche::PROTOCOL_VERSION).expect("server config");
        configure_quic(&mut server_cfg, Duration::from_secs(90), "cubic").expect("configure_quic");
        server_cfg
            .load_cert_chain_from_pem_file(cert_file.path().to_str().expect("path"))
            .expect("load cert");
        server_cfg
            .load_priv_key_from_pem_file(key_file.path().to_str().expect("path"))
            .expect("load key");

        let mut client_cfg = quiche::Config::new(quiche::PROTOCOL_VERSION).expect("client config");
        configure_quic(&mut client_cfg, Duration::from_secs(90), "cubic").expect("configure_quic");

        let client_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:4434".parse().unwrap();

        // 1. Cold handshake
        let mut client1 = quiche::connect(
            Some("consumer-masque.cloudflareclient.com"),
            &quiche::ConnectionId::from_ref(&[1; 16]),
            client_addr,
            server_addr,
            &mut client_cfg,
        )
        .expect("connect 1");
        let mut server1 = quiche::accept(
            &quiche::ConnectionId::from_ref(&[2; 16]),
            None,
            server_addr,
            client_addr,
            &mut server_cfg,
        )
        .expect("accept 1");
        for _ in 0..10 {
            deliver(&mut client1, &mut server1);
            deliver(&mut server1, &mut client1);
        }
        assert!(client1.is_established() && !client1.is_resumed());
        let mut cached_session = Some(
            client1
                .session()
                .expect("session ticket after cold handshake")
                .to_vec(),
        );

        // 2. Reconnect with cached session -> 0-RTT early data + resumed handshake
        let mut client2 = quiche::connect(
            Some("consumer-masque.cloudflareclient.com"),
            &quiche::ConnectionId::from_ref(&[3; 16]),
            client_addr,
            server_addr,
            &mut client_cfg,
        )
        .expect("connect 2");
        let mut server2 = quiche::accept(
            &quiche::ConnectionId::from_ref(&[4; 16]),
            None,
            server_addr,
            client_addr,
            &mut server_cfg,
        )
        .expect("accept 2");

        assert!(restore_quic_session(&mut client2, &mut cached_session));
        assert!(cached_session.is_some());

        // Emit initial flight: client2 enters 0-RTT early data immediately.
        let mut packet = [0u8; MAX_DATAGRAM_SIZE];
        let (len, info) = client2.send(&mut packet).expect("initial send");
        assert!(client2.is_in_early_data());
        let peer_cert_early = client2.peer_cert().expect("peer cert in early data");
        assert!(tls::verify_endpoint_key(peer_cert_early, &server_spki_der));

        server2
            .recv(
                &mut packet[..len],
                quiche::RecvInfo {
                    from: info.from,
                    to: info.to,
                },
            )
            .expect("server2 recv initial");

        // Open HTTP/3 CONNECT request during 0-RTT before server replies.
        let mut h3_cfg = quiche::h3::Config::new().expect("h3 config");
        h3_cfg.enable_extended_connect(true);
        let mut h3_client =
            quiche::h3::Connection::with_transport(&mut client2, &h3_cfg).expect("h3 in 0-RTT");
        let req = vec![
            quiche::h3::Header::new(b":method", b"CONNECT"),
            quiche::h3::Header::new(b":protocol", b"cf-connect-ip"),
            quiche::h3::Header::new(b":scheme", b"https"),
            quiche::h3::Header::new(b":authority", b"cloudflareaccess.com"),
            quiche::h3::Header::new(b":path", b"/"),
        ];
        let stream_id = h3_client
            .send_request(&mut client2, &req, false)
            .expect("send CONNECT in 0-RTT");
        assert_eq!(stream_id, 0);

        for _ in 0..10 {
            deliver(&mut client2, &mut server2);
            deliver(&mut server2, &mut client2);
        }
        assert!(client2.is_established());
        assert!(client2.is_resumed());
        let peer_cert_established = client2.peer_cert().expect("peer cert after resumption");
        assert!(tls::verify_endpoint_key(
            peer_cert_established,
            &server_spki_der
        ));
    }

    #[test]
    fn restore_quic_session_discards_corrupted_session_blobs() {
        let mut client_cfg = quiche::Config::new(quiche::PROTOCOL_VERSION).expect("client config");
        configure_quic(&mut client_cfg, Duration::from_secs(90), "cubic").expect("configure_quic");
        let client_addr: SocketAddr = "127.0.0.1:4433".parse().unwrap();
        let server_addr: SocketAddr = "127.0.0.1:4434".parse().unwrap();
        let mut client = quiche::connect(
            Some("consumer-masque.cloudflareclient.com"),
            &quiche::ConnectionId::from_ref(&[9; 16]),
            client_addr,
            server_addr,
            &mut client_cfg,
        )
        .expect("connect");

        let mut invalid = Some(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(!restore_quic_session(&mut client, &mut invalid));
        assert!(invalid.is_none());
    }
}
