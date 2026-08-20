//! Network layer: WebRTC engine, TCP proxy and metrics.
//!
//! This layer is fully decoupled from the UI. It exposes async channels
//! ([`NetEvent`] broadcast + [`MetricsHandle`]) that the presentation layer
//! subscribes to. No `ratatui` types are imported here.

pub mod engine;
pub mod metrics;
pub mod proxy;
pub mod signaling;

pub use engine::EngineCmd;
pub use metrics::{fmt_bytes, fmt_rate, MetricsHandle, MetricsSnapshot};
pub use signaling::{IceCandidate, Signaling};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, watch, Mutex};

/// Stable identifier for a peer (random u64 rendered as short hex).
pub type PeerId = u64;

/// Quality/classification of a peer link, drives edge colour in the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkKind {
    /// UDP hole-punching active, RTT < 40 ms.
    #[default]
    DirectFast,
    /// Direct P2P, moderate latency (< 120 ms).
    DirectModerate,
    /// Direct P2P, high latency.
    DirectSlow,
    /// Routed through a relay/TURN server.
    Relay,
}

impl LinkKind {
    pub fn is_relay(self) -> bool {
        matches!(self, LinkKind::Relay)
    }

    /// Classify the link quality from the current RTT and relay flag.
    pub fn classify(rtt_ms: u32, relayed: bool) -> LinkKind {
        if relayed {
            LinkKind::Relay
        } else if rtt_ms < 40 {
            LinkKind::DirectFast
        } else if rtt_ms < 120 {
            LinkKind::DirectModerate
        } else {
            LinkKind::DirectSlow
        }
    }
}

/// Lifecycle of a peer connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeerStatus {
    #[default]
    Idle,
    Gathering,
    Connecting,
    Connected,
    Failed,
    Closed,
}

/// A configured local TCP port mapped to a remote destination on a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tunnel {
    pub id: u32,
    pub local_addr: SocketAddr,
    pub peer: PeerId,
    pub remote_host: String,
    pub remote_port: u16,
    pub label: String,
}

/// A single active TCP sub-stream carried over a peer's data channel.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub id: u64,
    pub tunnel_id: u32,
    pub peer: PeerId,
    pub status: StreamStatus,
    pub opened_at: Instant,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamStatus {
    #[default]
    Established,
    Transferring,
    Closed,
}

/// Runtime state of a peer in the mesh.
#[derive(Debug, Clone, Default)]
pub struct PeerState {
    pub id: PeerId,
    pub label: String,
    pub remote_addr: Option<String>,
    pub public_ip: Option<String>,
    pub nat_type: Option<String>,
    pub status: PeerStatus,
    pub link: LinkKind,
    pub relayed: bool,
    pub rtt_ms: u32,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub connected_at: Option<Instant>,
}

/// Events emitted by the network layer for the UI / app to react to.
#[derive(Debug, Clone)]
pub enum NetEvent {
    PeerAdded(PeerState),
    PeerUpdated(PeerState),
    PeerRemoved(PeerId),
    PeerConnected(PeerId),
    PeerFailed(PeerId, String),
    StreamOpened(StreamInfo),
    StreamUpdated(StreamInfo),
    StreamClosed(u64),
    TunnelAdded(Tunnel),
    TunnelRemoved(u32),
    Log(LogLevel, String),
    /// A new SDP offer/answer was produced and should be shared out-of-band.
    SdpReady {
        peer: PeerId,
        sdp: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    PacketDrop,
    Handshake,
}

/// Shared, thread-safe mesh state used by both the engine and metrics.
#[derive(Debug, Default)]
pub struct MeshState {
    pub peers: HashMap<PeerId, PeerState>,
    pub tunnels: HashMap<u32, Tunnel>,
    pub streams: HashMap<u64, StreamInfo>,
}

pub type SharedMesh = Arc<Mutex<MeshState>>;

/// Top-level network orchestrator handle returned to `main`.
pub struct Network {
    pub mesh: SharedMesh,
    pub metrics: MetricsHandle,
    pub events_tx: broadcast::Sender<NetEvent>,
    pub events_rx: broadcast::Receiver<NetEvent>,
    pub public_ip: watch::Receiver<Option<String>>,
    pub nat_type: watch::Receiver<String>,
    pub engine: engine::EngineHandle,
}

/// Build and spawn the whole network stack. Returns a [`Network`] handle.
pub async fn spawn(config: NetworkConfig) -> anyhow::Result<Network> {
    let mesh: SharedMesh = Arc::new(Mutex::new(MeshState::default()));
    let (events_tx, events_rx) = broadcast::channel::<NetEvent>(256);
    let (public_ip_tx, public_ip_rx) = watch::channel(None);
    let (nat_type_tx, nat_type_rx) = watch::channel("unknown".to_string());

    // Metrics collector owns a clone of the mesh to snapshot peer state.
    let metrics = metrics::spawn_collector(mesh.clone());

    // Engine drives peer connections / ICE / data channels.
    let engine = engine::spawn(
        mesh.clone(),
        events_tx.clone(),
        metrics.clone(),
        public_ip_tx,
        nat_type_tx,
        config,
    )
    .await?;

    Ok(Network {
        mesh,
        metrics,
        events_tx,
        events_rx,
        public_ip: public_ip_rx,
        nat_type: nat_type_rx,
        engine,
    })
}

/// Configuration passed from the CLI into the network layer.
#[derive(Clone)]
pub struct NetworkConfig {
    pub stun_servers: Vec<String>,
    pub turn_servers: Vec<String>,
    /// Seed peers (label, signalling token/SDP) to dial on startup.
    pub seeds: Vec<SeedPeer>,
    /// Local tunnels to open immediately.
    pub tunnels: Vec<TunnelSeed>,
    /// Optional HTTP signaling server. When set, peers can dial each other by
    /// room name without manually exchanging SDP.
    pub signaling: Option<Signaling>,
}

#[derive(Debug, Clone)]
pub struct SeedPeer {
    pub label: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct TunnelSeed {
    pub local_port: u16,
    pub peer_label: String,
    pub remote_host: String,
    pub remote_port: u16,
    pub label: String,
}

/// Helper: log an event both to the broadcast bus and stdout (best-effort).
pub fn emit_log(events: &broadcast::Sender<NetEvent>, level: LogLevel, msg: impl Into<String>) {
    let msg = msg.into();
    let _ = events.send(NetEvent::Log(level, msg));
}
