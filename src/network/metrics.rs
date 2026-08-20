//! Telemetry aggregator.
//!
//! A background task polls peer statistics, computes throughput (bytes/sec),
//! round-trip latency and packet-loss, then publishes an immutable
//! [`MetricsSnapshot`] through a `tokio::sync::watch` channel roughly every
//! 100 ms. The UI thread reads the latest snapshot without locking the network.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use super::{LinkKind, PeerId, PeerState, SharedMesh};

/// How often a fresh snapshot is published to the UI.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);
/// Window used for the throughput sparkline (seconds).
pub const SPARK_WINDOW_SECS: usize = 60;

/// Per-peer live statistics.
#[derive(Debug, Clone, Default)]
pub struct PeerStats {
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub rtt_ms: u32,
    pub packets_sent: u64,
    pub packets_recv: u64,
    pub packets_lost: u64,
    pub link: LinkKind,
}

/// Immutable snapshot of the whole mesh, consumed by the UI.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub timestamp: Instant,
    pub peers: HashMap<PeerId, PeerState>,
    pub total_up: u64,
    pub total_down: u64,
    pub up_rate_bps: u64,
    pub down_rate_bps: u64,
    pub avg_rtt_ms: u32,
    pub total_packets: u64,
    pub total_lost: u64,
    /// Last `SPARK_WINDOW_SECS` seconds of up/down rates (oldest first).
    pub up_history: Vec<u64>,
    pub down_history: Vec<u64>,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            timestamp: Instant::now(),
            peers: HashMap::new(),
            total_up: 0,
            total_down: 0,
            up_rate_bps: 0,
            down_rate_bps: 0,
            avg_rtt_ms: 0,
            total_packets: 0,
            total_lost: 0,
            up_history: Vec::new(),
            down_history: Vec::new(),
        }
    }
}

/// Commands the collector understands from the rest of the system.
#[derive(Debug, Clone)]
pub enum MetricsCmd {
    /// (Re)publish a snapshot immediately.
    ForcePublish,
    /// Record bytes transferred for a peer. `(peer, sent, recv)`
    Bytes { peer: PeerId, sent: u64, recv: u64 },
    /// Update RTT for a peer.
    Rtt { peer: PeerId, rtt_ms: u32 },
    /// Update packet counters for a peer.
    Packets {
        peer: PeerId,
        sent: u64,
        recv: u64,
        lost: u64,
    },
    /// Mark a peer as relayed (or not).
    Link { peer: PeerId, relayed: bool },
}

/// Shared handle used by the engine/proxy to feed the collector and by the UI
/// to read the latest snapshot.
#[derive(Clone)]
pub struct MetricsHandle {
    pub cmd_tx: mpsc::UnboundedSender<MetricsCmd>,
    pub snapshot_rx: watch::Receiver<MetricsSnapshot>,
}

/// Internal accumulator kept by the collector task.
#[derive(Debug, Default)]
struct Acc {
    sent: u64,
    recv: u64,
    rtt_ms: u32,
    packets_sent: u64,
    packets_recv: u64,
    packets_lost: u64,
    relayed: bool,
}

/// Spawn the metrics collector. Returns a handle for producers and consumers.
pub fn spawn_collector(mesh: SharedMesh) -> MetricsHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<MetricsCmd>();
    let (snapshot_tx, snapshot_rx) = watch::channel(MetricsSnapshot::default());

    tokio::spawn(async move {
        let mut acc: HashMap<PeerId, Acc> = HashMap::new();
        let mut last_snapshot = Instant::now();
        let mut last_bytes_sent: u64 = 0;
        let mut last_bytes_recv: u64 = 0;
        let mut up_hist: Vec<u64> = Vec::with_capacity(SPARK_WINDOW_SECS);
        let mut down_hist: Vec<u64> = Vec::with_capacity(SPARK_WINDOW_SECS);
        let mut last_hist_tick = Instant::now();

        let mut ticker = tokio::time::interval(SNAPSHOT_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Drain commands without blocking the publish cadence.
                Some(cmd) = cmd_rx.recv() => { handle_cmd(cmd, &mut acc); }
                _ = ticker.tick() => {
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_snapshot).as_secs_f64().max(0.001);
                    last_snapshot = now;

                    // Aggregate totals from accumulators.
                    let mut total_sent = 0u64;
                    let mut total_recv = 0u64;
                    let mut rtt_sum = 0u64;
                    let mut rtt_n = 0u32;
                    let mut pkts = 0u64;
                    let mut lost = 0u64;

                    let peers_snapshot = {
                        let guard = mesh.lock().await;
                        guard.peers.clone()
                    };

                    for (pid, peer) in &peers_snapshot {
                        let a = acc.entry(*pid).or_default();
                        // Reflect accumulated link classification.
                        a.relayed = a.relayed || peer.relayed;
                        total_sent += a.sent;
                        total_recv += a.recv;
                        if a.rtt_ms > 0 { rtt_sum += a.rtt_ms as u64; rtt_n += 1; }
                        pkts += a.packets_sent + a.packets_recv;
                        lost += a.packets_lost;
                    }

                    let up_rate = ((total_sent.saturating_sub(last_bytes_sent) as f64) / elapsed) as u64;
                    let down_rate = ((total_recv.saturating_sub(last_bytes_recv) as f64) / elapsed) as u64;
                    last_bytes_sent = total_sent;
                    last_bytes_recv = total_recv;

                    // Push per-second samples into the sparkline history.
                    if now.duration_since(last_hist_tick) >= Duration::from_secs(1) {
                        last_hist_tick = now;
                        push_sample(&mut up_hist, up_rate);
                        push_sample(&mut down_hist, down_rate);
                    }

                    let avg_rtt = if rtt_n > 0 { (rtt_sum / rtt_n as u64) as u32 } else { 0 };

                    let snap = MetricsSnapshot {
                        timestamp: now,
                        peers: peers_snapshot,
                        total_up: total_sent,
                        total_down: total_recv,
                        up_rate_bps: up_rate,
                        down_rate_bps: down_rate,
                        avg_rtt_ms: avg_rtt,
                        total_packets: pkts,
                        total_lost: lost,
                        up_history: up_hist.clone(),
                        down_history: down_hist.clone(),
                    };
                    // Ignore send errors: UI may have dropped the receiver on shutdown.
                    let _ = snapshot_tx.send(snap);
                }
            }
        }
    });

    MetricsHandle {
        cmd_tx,
        snapshot_rx,
    }
}

fn handle_cmd(cmd: MetricsCmd, acc: &mut HashMap<PeerId, Acc>) {
    match cmd {
        MetricsCmd::ForcePublish => {} // publishing is driven by the ticker.
        MetricsCmd::Bytes { peer, sent, recv } => {
            let a = acc.entry(peer).or_default();
            a.sent += sent;
            a.recv += recv;
        }
        MetricsCmd::Rtt { peer, rtt_ms } => {
            let a = acc.entry(peer).or_default();
            a.rtt_ms = rtt_ms;
        }
        MetricsCmd::Packets {
            peer,
            sent,
            recv,
            lost,
        } => {
            let a = acc.entry(peer).or_default();
            a.packets_sent += sent;
            a.packets_recv += recv;
            a.packets_lost += lost;
        }
        MetricsCmd::Link { peer, relayed } => {
            let a = acc.entry(peer).or_default();
            a.relayed = relayed;
        }
    }
}

fn push_sample(hist: &mut Vec<u64>, sample: u64) {
    hist.push(sample);
    if hist.len() > SPARK_WINDOW_SECS {
        hist.remove(0);
    }
}

/// Human-readable byte-rate formatter (binary units).
pub fn fmt_rate(bps: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = bps as f64;
    if v >= GB {
        format!("{:.2} GB/s", v / GB)
    } else if v >= MB {
        format!("{:.2} MB/s", v / MB)
    } else if v >= KB {
        format!("{:.2} KB/s", v / KB)
    } else {
        format!("{} B/s", bps)
    }
}

/// Human-readable byte total formatter.
pub fn fmt_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = b as f64;
    if v >= GB {
        format!("{:.2} GB", v / GB)
    } else if v >= MB {
        format!("{:.2} MB", v / MB)
    } else if v >= KB {
        format!("{:.2} KB", v / KB)
    } else {
        format!("{} B", b)
    }
}
