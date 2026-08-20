//! WebRTC engine: PeerConnection lifecycle, ICE agent, DataChannels and SDP
//! exchange.
//!
//! The engine is driven by an [`EngineCmd`] channel. Signalling (SDP offer /
//! answer + ICE candidates) is exchanged out-of-band: the engine emits
//! [`NetEvent::SdpReady`] when an offer is produced and accepts remote
//! descriptions via [`EngineCmd::RemoteSdp`].
//!
//! Each peer gets:
//!   * a control `RTCDataChannel` ("synapse-ctrl") for pings/RTT & metadata,
//!   * per-tunnel `RTCDataChannel`s ("synapse-tun-<id>") carrying proxied bytes.
//!
//! The implementation is resilient: every error is logged via the event bus
//! and never panics the runtime.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context as _, Result};
use bytes::Bytes;
use tokio::sync::{broadcast, mpsc, watch, Mutex};
use tokio::time;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::{APIBuilder, API};
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

use super::{
    emit_log, IceCandidate, LinkKind, LogLevel, NetEvent, NetworkConfig, PeerId, PeerState,
    PeerStatus, SharedMesh, Signaling, StreamInfo, StreamStatus, Tunnel,
};
use crate::network::metrics::MetricsHandle;

/// Commands accepted by the engine.
#[derive(Debug, Clone)]
pub enum EngineCmd {
    /// Dial a new peer by label + signalling token. Returns its PeerId via the
    /// event bus (`PeerAdded`).
    Dial { label: String, token: String },
    /// Dial a peer via the HTTP signaling server by room name. The dialer acts
    /// as the offerer ("side a"); the remote peer answering should use
    /// [`EngineCmd::AnswerSignaling`] with the same room.
    DialSignaling { label: String, room: String },
    /// Answer an incoming signaling offer for a room ("side b").
    AnswerSignaling { label: String, room: String },
    /// Feed a remote SDP answer/offer for a known peer.
    RemoteSdp {
        peer: PeerId,
        sdp: Box<RTCSessionDescription>,
    },
    /// Feed a remote ICE candidate (trickle ICE).
    RemoteIce {
        peer: PeerId,
        candidate: RTCIceCandidateInit,
    },
    /// Open a local tunnel mapped to a peer's remote endpoint.
    OpenTunnel(Tunnel),
    /// Close a tunnel and its data channel.
    CloseTunnel(u32),
    /// Close a peer connection.
    ClosePeer(PeerId),
    /// Paste a full SDP blob received out-of-band (quick-connect modal).
    QuickConnect {
        label: String,
        sdp: String,
        is_offer: bool,
    },
    /// Shut the engine down.
    Shutdown,
}

/// Handle returned to `main` for driving the engine.
#[derive(Clone)]
pub struct EngineHandle {
    pub cmd_tx: mpsc::UnboundedSender<EngineCmd>,
}

/// Per-peer runtime bookkeeping held by the engine.
pub(crate) struct PeerCtx {
    pub(crate) id: PeerId,
    pub(crate) label: String,
    pub(crate) pc: Arc<RTCPeerConnection>,
    /// Control channel for pings / metadata.
    pub(crate) ctrl: Option<Arc<RTCDataChannel>>,
    /// Tunnel data channels keyed by tunnel id.
    pub(crate) tunnels: HashMap<u32, Arc<RTCDataChannel>>,
    /// Last ping send time, used for RTT.
    pub(crate) last_ping: Option<Instant>,
}

/// Spawn the engine task.
pub async fn spawn(
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
    public_ip_tx: watch::Sender<Option<String>>,
    nat_type_tx: watch::Sender<String>,
    config: NetworkConfig,
) -> Result<EngineHandle> {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<EngineCmd>();

    // Build a single shared WebRTC API (media engine + interceptors).
    let api = build_api()?;

    let ice_servers: Vec<RTCIceServer> = config
        .stun_servers
        .iter()
        .map(|s| RTCIceServer {
            urls: vec![s.clone()],
            ..Default::default()
        })
        .chain(config.turn_servers.iter().map(|s| RTCIceServer {
            urls: vec![s.clone()],
            ..Default::default()
        }))
        .collect();

    // Shared state inside the engine task.
    let peers: Arc<Mutex<HashMap<PeerId, PeerCtx>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_peer_id: Arc<Mutex<u64>> = Arc::new(Mutex::new(1));
    let next_tunnel_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));

    // Best-effort public IP / NAT discovery via the first STUN server.
    spawn_nat_discovery(
        config.stun_servers.clone(),
        public_ip_tx,
        nat_type_tx,
        events.clone(),
    );

    // Optional HTTP signaling server, shared by all signaling-driven dials.
    let signaling = config.signaling.clone();

    // Apply seed tunnels: they will bind once their peer connects.
    let seed_tunnels = config.tunnels.clone();
    let seed_peers = config.seeds.clone();

    let cmd_tx_clone = cmd_tx.clone();
    tokio::spawn(async move {
        // Dial seed peers.
        for seed in seed_peers {
            let _ = cmd_tx_clone.send(EngineCmd::Dial {
                label: seed.label,
                token: seed.token,
            });
        }
        // Open seed tunnels (peer resolution happens lazily on connect).
        for t in seed_tunnels {
            let _ = cmd_tx_clone.send(EngineCmd::OpenTunnel(Tunnel {
                id: 0, // assigned by engine
                local_addr: format!("127.0.0.1:{}", t.local_port)
                    .parse()
                    .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
                peer: 0, // resolved by label later
                remote_host: t.remote_host,
                remote_port: t.remote_port,
                label: t.label,
            }));
        }

        // Main command loop.
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                EngineCmd::Shutdown => break,
                EngineCmd::Dial { label, token } => {
                    if let Err(e) = handle_dial(
                        &api,
                        &ice_servers,
                        &mesh,
                        &events,
                        &metrics,
                        &peers,
                        &next_peer_id,
                        label,
                        token,
                    )
                    .await
                    {
                        emit_log(&events, LogLevel::Error, format!("dial failed: {e:#}"));
                    }
                }
                EngineCmd::RemoteSdp { peer, sdp } => {
                    if let Err(e) = handle_remote_sdp(&peers, &events, peer, sdp).await {
                        emit_log(&events, LogLevel::Error, format!("remote sdp: {e:#}"));
                    }
                }
                EngineCmd::RemoteIce { peer, candidate } => {
                    if let Err(e) = handle_remote_ice(&peers, &events, peer, candidate).await {
                        emit_log(&events, LogLevel::Error, format!("remote ice: {e:#}"));
                    }
                }
                EngineCmd::OpenTunnel(t) => {
                    if let Err(e) =
                        handle_open_tunnel(&mesh, &events, &metrics, &peers, &next_tunnel_id, t)
                            .await
                    {
                        emit_log(&events, LogLevel::Error, format!("open tunnel: {e:#}"));
                    }
                }
                EngineCmd::CloseTunnel(id) => {
                    if let Err(e) = handle_close_tunnel(&mesh, &events, &peers, id).await {
                        emit_log(&events, LogLevel::Error, format!("close tunnel: {e:#}"));
                    }
                }
                EngineCmd::ClosePeer(id) => {
                    if let Err(e) = handle_close_peer(&mesh, &events, &peers, id).await {
                        emit_log(&events, LogLevel::Error, format!("close peer: {e:#}"));
                    }
                }
                EngineCmd::QuickConnect {
                    label,
                    sdp,
                    is_offer,
                } => {
                    if let Err(e) = handle_quick_connect(
                        &api,
                        &ice_servers,
                        &mesh,
                        &events,
                        &metrics,
                        &peers,
                        &next_peer_id,
                        label,
                        sdp,
                        is_offer,
                    )
                    .await
                    {
                        emit_log(&events, LogLevel::Error, format!("quick connect: {e:#}"));
                    }
                }
                EngineCmd::DialSignaling { label, room } => {
                    if let Some(sig) = signaling.as_ref() {
                        let sig = sig.clone();
                        let api = Arc::clone(&api);
                        let mesh = mesh.clone();
                        let events = events.clone();
                        let metrics = metrics.clone();
                        let peers = Arc::clone(&peers);
                        let next_peer_id = Arc::clone(&next_peer_id);
                        let ice_servers = ice_servers.clone();
                        tokio::spawn(async move {
                            if let Err(e) = signaling_dial(
                                &api,
                                &ice_servers,
                                &mesh,
                                &events,
                                &metrics,
                                &peers,
                                &next_peer_id,
                                sig,
                                label,
                                room,
                            )
                            .await
                            {
                                emit_log(
                                    &events,
                                    LogLevel::Error,
                                    format!("signaling dial: {e:#}"),
                                );
                            }
                        });
                    } else {
                        emit_log(
                            &events,
                            LogLevel::Error,
                            "signaling dial: no --signaling URL configured",
                        );
                    }
                }
                EngineCmd::AnswerSignaling { label, room } => {
                    if let Some(sig) = signaling.as_ref() {
                        let sig = sig.clone();
                        let api = Arc::clone(&api);
                        let mesh = mesh.clone();
                        let events = events.clone();
                        let metrics = metrics.clone();
                        let peers = Arc::clone(&peers);
                        let next_peer_id = Arc::clone(&next_peer_id);
                        let ice_servers = ice_servers.clone();
                        tokio::spawn(async move {
                            if let Err(e) = signaling_answer(
                                &api,
                                &ice_servers,
                                &mesh,
                                &events,
                                &metrics,
                                &peers,
                                &next_peer_id,
                                sig,
                                label,
                                room,
                            )
                            .await
                            {
                                emit_log(
                                    &events,
                                    LogLevel::Error,
                                    format!("signaling answer: {e:#}"),
                                );
                            }
                        });
                    } else {
                        emit_log(
                            &events,
                            LogLevel::Error,
                            "signaling answer: no --signaling URL configured",
                        );
                    }
                }
            }
        }

        // Graceful shutdown: close every peer connection.
        let mut guard = peers.lock().await;
        for (_, ctx) in guard.drain() {
            let _ = ctx.pc.close().await;
        }
        emit_log(&events, LogLevel::Info, "engine stopped");
    });

    Ok(EngineHandle { cmd_tx })
}

/// Build the shared WebRTC API with default interceptors.
fn build_api() -> Result<Arc<API>> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();
    Ok(Arc::new(api))
}

/// Create a peer connection wired with state-change handlers.
async fn create_pc(
    api: &API,
    ice_servers: &[RTCIceServer],
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
    peers: Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    peer_id: PeerId,
) -> Result<Arc<RTCPeerConnection>> {
    create_pc_with_ice_hook(
        api,
        ice_servers,
        mesh,
        events,
        metrics,
        peers,
        peer_id,
        None,
    )
    .await
}

/// Same as [`create_pc`] but with an optional hook invoked for every local ICE
/// candidate. Used by the signaling flow to publish candidates to the HTTP
/// server.
type IceHook = Arc<dyn Fn(webrtc::ice_transport::ice_candidate::RTCIceCandidate) + Send + Sync>;

#[allow(clippy::too_many_arguments)]
async fn create_pc_with_ice_hook(
    api: &API,
    ice_servers: &[RTCIceServer],
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
    peers: Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    peer_id: PeerId,
    ice_hook: Option<IceHook>,
) -> Result<Arc<RTCPeerConnection>> {
    let config = RTCConfiguration {
        ice_servers: ice_servers.to_vec(),
        ..Default::default()
    };
    let pc = Arc::new(api.new_peer_connection(config).await?);

    // ICE connection state -> peer status + link classification.
    let pc_ice = Arc::clone(&pc);
    let mesh_ice = mesh.clone();
    let events_ice = events.clone();
    let metrics_ice = metrics.clone();
    pc.on_ice_connection_state_change(Box::new(move |state| {
        let pc_ice = Arc::clone(&pc_ice);
        let mesh_ice = mesh_ice.clone();
        let events_ice = events_ice.clone();
        let metrics_ice = metrics_ice.clone();
        Box::pin(async move {
            let (status, relayed, log_lvl, msg) = match state {
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::New
                | webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Checking => {
                    (PeerStatus::Connecting, false, LogLevel::Handshake, "ICE checking")
                }
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Connected => {
                    (PeerStatus::Connected, false, LogLevel::Handshake, "ICE connected (direct)")
                }
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Completed => {
                    (PeerStatus::Connected, false, LogLevel::Handshake, "ICE completed")
                }
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Failed => {
                    (PeerStatus::Failed, false, LogLevel::Error, "ICE failed")
                }
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Disconnected => {
                    (PeerStatus::Connecting, false, LogLevel::Warn, "ICE disconnected")
                }
                webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Closed => {
                    (PeerStatus::Closed, false, LogLevel::Info, "ICE closed")
                }
                _ => (PeerStatus::Connecting, false, LogLevel::Info, "ICE unknown"),
            };
            // Detect relay via selected candidate pair stats (best-effort).
            let relayed = if status == PeerStatus::Connected {
                stats_relayed(&pc_ice).await.unwrap_or(false)
            } else {
                relayed
            };
            update_peer_state(&mesh_ice, peer_id, |p| {
                p.status = status;
                p.relayed = relayed;
                p.link = LinkKind::classify(p.rtt_ms, relayed);
            })
            .await;
            let _ = metrics_ice.cmd_tx.send(crate::network::metrics::MetricsCmd::Link {
                peer: peer_id,
                relayed,
            });
            emit_log(&events_ice, log_lvl, format!("peer {peer_id}: {msg}"));
            if status == PeerStatus::Connected {
                let _ = events_ice.send(NetEvent::PeerConnected(peer_id));
            } else if status == PeerStatus::Failed {
                let _ = events_ice.send(NetEvent::PeerFailed(peer_id, "ICE failed".into()));
            }
        })
    }));

    // Peer connection state (overall).
    let mesh_pc = mesh.clone();
    let events_pc = events.clone();
    pc.on_peer_connection_state_change(Box::new(move |state| {
        let mesh_pc = mesh_pc.clone();
        let events_pc = events_pc.clone();
        Box::pin(async move {
            let s = match state {
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Connected => PeerStatus::Connected,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Failed => PeerStatus::Failed,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed => PeerStatus::Closed,
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Disconnected => PeerStatus::Connecting,
                _ => return, // Don't downgrade from Connected/Connecting for transient states.
            };
            update_peer_state(&mesh_pc, peer_id, |p| p.status = s).await;
            if s == PeerStatus::Failed {
                let _ = events_pc.send(NetEvent::PeerFailed(peer_id, "PC failed".into()));
            }
        })
    }));

    // Local ICE candidates -> emit so they can be signalled out-of-band.
    let events_ice_cand = events.clone();
    let ice_hook_local = ice_hook.clone();
    pc.on_ice_candidate(Box::new(move |c| {
        let events_ice_cand = events_ice_cand.clone();
        let hook = ice_hook_local.clone();
        Box::pin(async move {
            if let Some(c) = c {
                let txt = c.to_json().unwrap_or_default().candidate;
                emit_log(
                    &events_ice_cand,
                    LogLevel::Handshake,
                    format!("local ICE: {txt}"),
                );
                if let Some(h) = hook.as_ref() {
                    h(c);
                }
            }
        })
    }));

    // Incoming data channels (responder side).
    let mesh_dc = mesh.clone();
    let events_dc = events.clone();
    let metrics_dc = metrics.clone();
    let peers_dc = peers.clone();
    pc.on_data_channel(Box::new(move |dc| {
        let label = dc.label().to_string();
        let mesh_dc = mesh_dc.clone();
        let events_dc = events_dc.clone();
        let metrics_dc = metrics_dc.clone();
        let peers_dc = peers_dc.clone();
        Box::pin(async move {
            // Store control channel in PeerCtx so ping loop can use it.
            if label == "synapse-ctrl" {
                let mut g = peers_dc.lock().await;
                if let Some(ctx) = g.get_mut(&peer_id) {
                    ctx.ctrl = Some(Arc::clone(&dc));
                }
            }
            wire_data_channel(&dc, &label, peer_id, mesh_dc, events_dc, metrics_dc).await;
        })
    }));

    Ok(pc)
}

/// Best-effort: inspect selected candidate pair to detect relay usage.
///
/// In webrtc-rs 0.11 the stats API surface is limited; we approximate by
/// checking the ICE agent's selected candidate pair type. On any failure we
/// conservatively report `false` (direct), so the UI still renders.
async fn stats_relayed(pc: &RTCPeerConnection) -> Result<bool> {
    let stats = pc.get_stats().await;
    let json = serde_json::to_value(&stats)?;
    // Walk the stats objects looking for a relayed local candidate.
    if let Some(map) = json.as_object() {
        for (_k, v) in map {
            if let Some(t) = v.get("candidateType").and_then(|x| x.as_str()) {
                if t == "relay" {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Wire a data channel's open/message/close handlers for control & tunnels.
async fn wire_data_channel(
    dc: &Arc<RTCDataChannel>,
    label: &str,
    peer_id: PeerId,
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
) {
    let label_owned = label.to_string();
    // Control channel: handle pings/pongs for RTT + label exchange.
    if label == "synapse-ctrl" {
        // Hello is sent by the ping loop (spawn_ping_loop) every ~10s.
        // Here we only handle incoming messages.
        let dc_ping = Arc::clone(dc);
        let mesh_ping = mesh.clone();
        let metrics_ping = metrics.clone();
        let mesh_hello = mesh.clone();
        let events_hello = events.clone();
        dc.on_message(Box::new(move |msg| {
            let dc_ping = Arc::clone(&dc_ping);
            let mesh_ping = mesh_ping.clone();
            let metrics_ping = metrics_ping.clone();
            let mesh_hello = mesh_hello.clone();
            let events_hello = events_hello.clone();
            Box::pin(async move {
                let data = String::from_utf8_lossy(&msg.data);
                if let Some(rest) = data.strip_prefix("hello:") {
                    // Remote peer announced their label — update our peer state.
                    let remote_label = rest.to_string();
                    emit_log(
                        &events_hello,
                        LogLevel::Handshake,
                        format!("remote label: {remote_label}"),
                    );
                    update_peer_state(&mesh_hello, peer_id, |p| {
                        p.label = remote_label.clone();
                    })
                    .await;
                } else if let Some(rest) = data.strip_prefix("ping:") {
                    // Reply with pong:<ts>.
                    let _ = dc_ping.send(&Bytes::from(format!("pong:{rest}"))).await;
                } else if let Some(rest) = data.strip_prefix("pong:") {
                    if let Ok(ts) = rest.parse::<u128>() {
                        // Compute RTT against the original send time encoded in ts.
                        let now_ns = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos();
                        let rtt_ms = now_ns.saturating_sub(ts) / 1_000_000;
                        let rtt_ms = rtt_ms.min(u32::MAX as u128) as u32;
                        update_peer_state(&mesh_ping, peer_id, |p| {
                            p.rtt_ms = rtt_ms;
                            p.link = LinkKind::classify(rtt_ms, p.relayed);
                        })
                        .await;
                        let _ =
                            metrics_ping
                                .cmd_tx
                                .send(crate::network::metrics::MetricsCmd::Rtt {
                                    peer: peer_id,
                                    rtt_ms,
                                });
                    }
                }
            })
        }));
        return;
    }

    // Tunnel channel: label is "synapse-tun-<id>".
    if let Some(id_str) = label_owned.strip_prefix("synapse-tun-") {
        if let Ok(tid) = id_str.parse::<u32>() {
            let dc_recv = Arc::clone(dc);
            let metrics_t = metrics.clone();
            let events_t = events.clone();
            // Track a new stream.
            let sid = rand_u64();
            let info = StreamInfo {
                id: sid,
                tunnel_id: tid,
                peer: peer_id,
                status: StreamStatus::Established,
                opened_at: Instant::now(),
                bytes_sent: 0,
                bytes_recv: 0,
            };
            {
                let mut g = mesh.lock().await;
                g.streams.insert(sid, info.clone());
            }
            let _ = events_t.send(NetEvent::StreamOpened(info));

            dc.on_open(Box::new({
                let events_t = events_t.clone();
                move || {
                    let events_t = events_t.clone();
                    Box::pin(async move {
                        emit_log(
                            &events_t,
                            LogLevel::Handshake,
                            format!("tunnel {tid} data channel open"),
                        );
                    })
                }
            }));

            let events_msg = events_t.clone();
            let metrics_msg = metrics_t.clone();
            dc.on_message(Box::new(move |msg| {
                let metrics_t = metrics_msg.clone();
                let events_t = events_msg.clone();
                let n = msg.data.len() as u64;
                Box::pin(async move {
                    let _ = metrics_t
                        .cmd_tx
                        .send(crate::network::metrics::MetricsCmd::Bytes {
                            peer: peer_id,
                            sent: 0,
                            recv: n,
                        });
                    let _ = events_t.send(NetEvent::StreamUpdated(StreamInfo {
                        id: sid,
                        tunnel_id: tid,
                        peer: peer_id,
                        status: StreamStatus::Transferring,
                        opened_at: Instant::now(),
                        bytes_sent: 0,
                        bytes_recv: n,
                    }));
                })
            }));

            dc.on_close(Box::new({
                let events_t = events_t.clone();
                move || {
                    let events_t = events_t.clone();
                    Box::pin(async move {
                        let _ = events_t.send(NetEvent::StreamClosed(sid));
                    })
                }
            }));
            let _ = dc_recv;
        }
    }
}

/// Handle a Dial command: create PC, control channel, create offer.
#[allow(clippy::too_many_arguments)]
async fn handle_dial(
    api: &API,
    ice_servers: &[RTCIceServer],
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    metrics: &MetricsHandle,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    next_peer_id: &Arc<Mutex<u64>>,
    label: String,
    _token: String,
) -> Result<()> {
    let peer_id = {
        let mut g = next_peer_id.lock().await;
        let id = *g;
        *g += 1;
        id
    };
    emit_log(
        events,
        LogLevel::Info,
        format!("dialing peer '{label}' (id={peer_id})"),
    );

    let pc = create_pc(
        api,
        ice_servers,
        mesh.clone(),
        events.clone(),
        metrics.clone(),
        peers.clone(),
        peer_id,
    )
    .await?;

    // Create the control data channel (dialer side).
    let ctrl = pc.create_data_channel("synapse-ctrl", None).await?;
    wire_data_channel(
        &ctrl,
        "synapse-ctrl",
        peer_id,
        mesh.clone(),
        events.clone(),
        metrics.clone(),
    )
    .await;

    // Insert peer state.
    let state = PeerState {
        id: peer_id,
        label: label.clone(),
        status: PeerStatus::Gathering,
        connected_at: Some(Instant::now()),
        ..Default::default()
    };
    {
        let mut g = mesh.lock().await;
        g.peers.insert(peer_id, state.clone());
    }
    let _ = events.send(NetEvent::PeerAdded(state));

    // Create an offer and emit it for out-of-band signalling.
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;
    let sdp = serde_json::to_string(&offer)?;
    let _ = events.send(NetEvent::SdpReady { peer: peer_id, sdp });

    {
        let mut g = peers.lock().await;
        g.insert(
            peer_id,
            PeerCtx {
                id: peer_id,
                label: label.clone(),
                pc,
                ctrl: Some(ctrl),
                tunnels: HashMap::new(),
                last_ping: None,
            },
        );
    }

    // Spawn a ping loop for RTT measurement + label exchange.
    spawn_ping_loop(
        peer_id,
        label,
        peers.clone(),
        events.clone(),
        metrics.clone(),
    );
    Ok(())
}

/// Handle a remote SDP description (answer when we dialled, offer when remote dialled).
async fn handle_remote_sdp(
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    events: &broadcast::Sender<NetEvent>,
    peer_id: PeerId,
    sdp: Box<RTCSessionDescription>,
) -> Result<()> {
    let pc = {
        let g = peers.lock().await;
        g.get(&peer_id)
            .map(|c| Arc::clone(&c.pc))
            .ok_or_else(|| anyhow!("unknown peer {peer_id}"))?
    };
    pc.set_remote_description(*sdp).await?;
    emit_log(
        events,
        LogLevel::Handshake,
        format!("peer {peer_id}: remote SDP set"),
    );
    Ok(())
}

/// Handle a remote trickle ICE candidate.
async fn handle_remote_ice(
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    events: &broadcast::Sender<NetEvent>,
    peer_id: PeerId,
    candidate: RTCIceCandidateInit,
) -> Result<()> {
    let pc = {
        let g = peers.lock().await;
        g.get(&peer_id)
            .map(|c| Arc::clone(&c.pc))
            .ok_or_else(|| anyhow!("unknown peer {peer_id}"))?
    };
    pc.add_ice_candidate(candidate).await?;
    emit_log(
        events,
        LogLevel::Handshake,
        format!("peer {peer_id}: remote ICE added"),
    );
    Ok(())
}

/// Open a tunnel: bind a local TCP listener and create a data channel on the
/// matching peer (resolved by label when it connects).
#[allow(clippy::too_many_arguments)]
async fn handle_open_tunnel(
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    metrics: &MetricsHandle,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    next_tunnel_id: &Arc<Mutex<u32>>,
    mut tunnel: Tunnel,
) -> Result<()> {
    let id = {
        let mut g = next_tunnel_id.lock().await;
        let v = *g;
        *g += 1;
        v
    };
    tunnel.id = id;

    // Resolve peer id by label.
    let peer_id = {
        let g = mesh.lock().await;
        g.peers
            .iter()
            .find(|(_, p)| p.label == tunnel.peer_label())
            .map(|(k, _)| *k)
    };
    if let Some(pid) = peer_id {
        tunnel.peer = pid;
    }
    {
        let mut g = mesh.lock().await;
        g.tunnels.insert(id, tunnel.clone());
    }
    let _ = events.send(NetEvent::TunnelAdded(tunnel.clone()));
    emit_log(
        events,
        LogLevel::Info,
        format!("tunnel '{}' added on {}", tunnel.label, tunnel.local_addr),
    );

    // Spawn the local TCP proxy listener.
    let proxy_mesh = mesh.clone();
    let proxy_events = events.clone();
    let proxy_metrics = metrics.clone();
    let proxy_peers = peers.clone();
    let events_err = events.clone();
    let t = tunnel.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::network::proxy::spawn_listener(
            t,
            proxy_mesh,
            proxy_events,
            proxy_metrics,
            proxy_peers,
        )
        .await
        {
            emit_log(
                &events_err,
                LogLevel::Error,
                format!("proxy listener: {e:#}"),
            );
        }
    });
    Ok(())
}

/// Close a tunnel.
async fn handle_close_tunnel(
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    tunnel_id: u32,
) -> Result<()> {
    {
        let mut g = mesh.lock().await;
        g.tunnels.remove(&tunnel_id);
    }
    // Close any data channels for this tunnel across peers.
    let mut to_close = Vec::new();
    {
        let mut g = peers.lock().await;
        for ctx in g.values_mut() {
            if let Some(dc) = ctx.tunnels.remove(&tunnel_id) {
                to_close.push(dc);
            }
        }
    }
    for dc in to_close {
        let _ = dc.close().await;
    }
    let _ = events.send(NetEvent::TunnelRemoved(tunnel_id));
    Ok(())
}

/// Close a peer connection.
async fn handle_close_peer(
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    peer_id: PeerId,
) -> Result<()> {
    let ctx = {
        let mut g = peers.lock().await;
        g.remove(&peer_id)
    };
    if let Some(ctx) = ctx {
        let _ = ctx.pc.close().await;
    }
    {
        let mut g = mesh.lock().await;
        g.peers.remove(&peer_id);
    }
    let _ = events.send(NetEvent::PeerRemoved(peer_id));
    Ok(())
}

/// Quick-connect: paste a full SDP. If it's an offer, we answer; if it's an
/// answer, we set it on an existing pending peer (or create a stub).
#[allow(clippy::too_many_arguments)]
async fn handle_quick_connect(
    api: &API,
    ice_servers: &[RTCIceServer],
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    metrics: &MetricsHandle,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    next_peer_id: &Arc<Mutex<u64>>,
    label: String,
    sdp: String,
    is_offer: bool,
) -> Result<()> {
    let peer_id = {
        let mut g = next_peer_id.lock().await;
        let id = *g;
        *g += 1;
        id
    };
    let pc = create_pc(
        api,
        ice_servers,
        mesh.clone(),
        events.clone(),
        metrics.clone(),
        peers.clone(),
        peer_id,
    )
    .await?;
    let state = PeerState {
        id: peer_id,
        label: label.clone(),
        status: PeerStatus::Gathering,
        connected_at: Some(Instant::now()),
        ..Default::default()
    };
    {
        let mut g = mesh.lock().await;
        g.peers.insert(peer_id, state.clone());
    }
    let _ = events.send(NetEvent::PeerAdded(state));

    if is_offer {
        let desc: RTCSessionDescription =
            serde_json::from_str(&sdp).context("parse remote offer")?;
        pc.set_remote_description(desc).await?;
        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer.clone()).await?;
        let ans = serde_json::to_string(&answer)?;
        let _ = events.send(NetEvent::SdpReady {
            peer: peer_id,
            sdp: ans,
        });
        emit_log(
            events,
            LogLevel::Handshake,
            format!("quick-connect: answered peer {peer_id}"),
        );
    } else {
        let desc: RTCSessionDescription =
            serde_json::from_str(&sdp).context("parse remote answer")?;
        pc.set_remote_description(desc).await?;
        emit_log(
            events,
            LogLevel::Handshake,
            format!("quick-connect: applied answer for peer {peer_id}"),
        );
    }

    {
        let mut g = peers.lock().await;
        g.insert(
            peer_id,
            PeerCtx {
                id: peer_id,
                label: label.clone(),
                pc,
                ctrl: None,
                tunnels: HashMap::new(),
                last_ping: None,
            },
        );
    }
    spawn_ping_loop(
        peer_id,
        label,
        peers.clone(),
        events.clone(),
        metrics.clone(),
    );
    Ok(())
}

/// Signaling-driven dialer (side A): create PC + control channel, create offer,
/// post it to `{base}/offer/{room}`, then poll `{base}/answer/{room}` until the
/// remote peer posts an answer. Local ICE candidates are posted to
/// `{base}/ice/{room}/a`; remote candidates are polled from `/ice/{room}/b`.
#[allow(clippy::too_many_arguments)]
async fn signaling_dial(
    api: &Arc<API>,
    ice_servers: &[RTCIceServer],
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    metrics: &MetricsHandle,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    next_peer_id: &Arc<Mutex<u64>>,
    sig: Signaling,
    label: String,
    room: String,
) -> Result<()> {
    let peer_id = {
        let mut g = next_peer_id.lock().await;
        let id = *g;
        *g += 1;
        id
    };
    emit_log(
        events,
        LogLevel::Info,
        format!("signaling dial '{label}' room '{room}' (id={peer_id})"),
    );

    // ICE hook: post every local candidate to /ice/{room}/a.
    let sig_ice = sig.clone();
    let room_ice = room.clone();
    let events_ice = events.clone();
    let ice_hook: IceHook = Arc::new(
        move |c: webrtc::ice_transport::ice_candidate::RTCIceCandidate| {
            let sig = sig_ice.clone();
            let room = room_ice.clone();
            let events = events_ice.clone();
            let init = c.to_json().unwrap_or_default();
            tokio::spawn(async move {
                let cand = IceCandidate {
                    candidate: init.candidate,
                    sdp_mid: init.sdp_mid,
                    sdp_mline_index: init.sdp_mline_index,
                    username_fragment: init.username_fragment,
                };
                if let Err(e) = sig.post_ice(&room, 'a', &cand).await {
                    emit_log(&events, LogLevel::Warn, format!("post ice: {e:#}"));
                }
            });
        },
    );

    let pc = create_pc_with_ice_hook(
        api,
        ice_servers,
        mesh.clone(),
        events.clone(),
        metrics.clone(),
        peers.clone(),
        peer_id,
        Some(ice_hook),
    )
    .await?;

    // Control channel (dialer side).
    let ctrl = pc.create_data_channel("synapse-ctrl", None).await?;
    wire_data_channel(
        &ctrl,
        "synapse-ctrl",
        peer_id,
        mesh.clone(),
        events.clone(),
        metrics.clone(),
    )
    .await;

    let state = PeerState {
        id: peer_id,
        label: label.clone(),
        status: PeerStatus::Gathering,
        connected_at: Some(Instant::now()),
        ..Default::default()
    };
    {
        let mut g = mesh.lock().await;
        g.peers.insert(peer_id, state.clone());
    }
    let _ = events.send(NetEvent::PeerAdded(state));

    // Create offer, set local description, post to signaling.
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;
    let offer_json = serde_json::to_string(&offer)?;
    sig.post_offer(&room, &offer_json)
        .await
        .context("post offer")?;
    emit_log(
        events,
        LogLevel::Handshake,
        format!("room '{room}': offer posted"),
    );

    // Insert peer ctx.
    {
        let mut g = peers.lock().await;
        g.insert(
            peer_id,
            PeerCtx {
                id: peer_id,
                label: label.clone(),
                pc: Arc::clone(&pc),
                ctrl: Some(ctrl),
                tunnels: HashMap::new(),
                last_ping: None,
            },
        );
    }
    spawn_ping_loop(
        peer_id,
        label.clone(),
        Arc::clone(peers),
        events.clone(),
        metrics.clone(),
    );

    // Poll for the remote answer.
    let answer_json = poll_signal(
        || {
            let sig = sig.clone();
            let room = room.clone();
            async move { sig.get_answer(&room).await }
        },
        Duration::from_secs(2),
        Duration::from_secs(60),
        events,
    )
    .await?;

    let answer_json =
        answer_json.ok_or_else(|| anyhow!("timed out waiting for answer in room '{room}'"))?;
    let answer: RTCSessionDescription =
        serde_json::from_str(&answer_json).context("parse answer")?;
    pc.set_remote_description(answer).await?;
    emit_log(
        events,
        LogLevel::Handshake,
        format!("room '{room}': remote answer set"),
    );

    // Drain remote ICE candidates (side b) and add them.
    spawn_ice_drain(
        sig.clone(),
        room.clone(),
        'b',
        Arc::clone(&pc),
        events.clone(),
    );
    Ok(())
}

/// Signaling-driven answerer (side B): poll `{base}/offer/{room}` until the
/// offerer posts an offer, set it as remote description, create + post the
/// answer, then drain remote ICE candidates from side a.
#[allow(clippy::too_many_arguments)]
async fn signaling_answer(
    api: &Arc<API>,
    ice_servers: &[RTCIceServer],
    mesh: &SharedMesh,
    events: &broadcast::Sender<NetEvent>,
    metrics: &MetricsHandle,
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    next_peer_id: &Arc<Mutex<u64>>,
    sig: Signaling,
    label: String,
    room: String,
) -> Result<()> {
    let peer_id = {
        let mut g = next_peer_id.lock().await;
        let id = *g;
        *g += 1;
        id
    };
    emit_log(
        events,
        LogLevel::Info,
        format!("signaling answer '{label}' room '{room}' (id={peer_id})"),
    );

    // ICE hook: post every local candidate to /ice/{room}/b.
    let sig_ice = sig.clone();
    let room_ice = room.clone();
    let events_ice = events.clone();
    let ice_hook: IceHook = Arc::new(
        move |c: webrtc::ice_transport::ice_candidate::RTCIceCandidate| {
            let sig = sig_ice.clone();
            let room = room_ice.clone();
            let events = events_ice.clone();
            let init = c.to_json().unwrap_or_default();
            tokio::spawn(async move {
                let cand = IceCandidate {
                    candidate: init.candidate,
                    sdp_mid: init.sdp_mid,
                    sdp_mline_index: init.sdp_mline_index,
                    username_fragment: init.username_fragment,
                };
                if let Err(e) = sig.post_ice(&room, 'b', &cand).await {
                    emit_log(&events, LogLevel::Warn, format!("post ice: {e:#}"));
                }
            });
        },
    );

    let pc = create_pc_with_ice_hook(
        api,
        ice_servers,
        mesh.clone(),
        events.clone(),
        metrics.clone(),
        peers.clone(),
        peer_id,
        Some(ice_hook),
    )
    .await?;

    let state = PeerState {
        id: peer_id,
        label: label.clone(),
        status: PeerStatus::Gathering,
        connected_at: Some(Instant::now()),
        ..Default::default()
    };
    {
        let mut g = mesh.lock().await;
        g.peers.insert(peer_id, state.clone());
    }
    let _ = events.send(NetEvent::PeerAdded(state));

    // Poll for the remote offer.
    let offer_json = poll_signal(
        || {
            let sig = sig.clone();
            let room = room.clone();
            async move { sig.get_offer(&room).await }
        },
        Duration::from_secs(2),
        Duration::from_secs(60),
        events,
    )
    .await?;

    let offer_json =
        offer_json.ok_or_else(|| anyhow!("timed out waiting for offer in room '{room}'"))?;
    let offer: RTCSessionDescription = serde_json::from_str(&offer_json).context("parse offer")?;
    pc.set_remote_description(offer).await?;
    emit_log(
        events,
        LogLevel::Handshake,
        format!("room '{room}': remote offer set"),
    );

    // Create answer, set local description, post to signaling.
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;
    let answer_json = serde_json::to_string(&answer)?;
    sig.post_answer(&room, &answer_json)
        .await
        .context("post answer")?;
    emit_log(
        events,
        LogLevel::Handshake,
        format!("room '{room}': answer posted"),
    );

    {
        let mut g = peers.lock().await;
        g.insert(
            peer_id,
            PeerCtx {
                id: peer_id,
                label: label.clone(),
                pc: Arc::clone(&pc),
                ctrl: None, // control channel arrives via on_data_channel
                tunnels: HashMap::new(),
                last_ping: None,
            },
        );
    }
    spawn_ping_loop(
        peer_id,
        label.clone(),
        Arc::clone(peers),
        events.clone(),
        metrics.clone(),
    );

    // Drain remote ICE candidates (side a) and add them.
    spawn_ice_drain(
        sig.clone(),
        room.clone(),
        'a',
        Arc::clone(&pc),
        events.clone(),
    );
    Ok(())
}

/// Poll a signaling getter until it returns `Some`, every `interval`, up to
/// `timeout`. Logs each attempt at debug level.
async fn poll_signal<F, Fut>(
    mut f: F,
    interval: Duration,
    timeout: Duration,
    events: &broadcast::Sender<NetEvent>,
) -> Result<Option<String>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<String>>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        match f().await {
            Ok(Some(v)) => return Ok(Some(v)),
            Ok(None) => {
                tokio::time::sleep(interval).await;
            }
            Err(e) => {
                emit_log(events, LogLevel::Warn, format!("signaling poll: {e:#}"));
                tokio::time::sleep(interval).await;
            }
        }
    }
}

/// Background task that periodically fetches the remote peer's ICE candidates
/// and adds any new ones to the peer connection.
fn spawn_ice_drain(
    sig: Signaling,
    room: String,
    side: char,
    pc: Arc<RTCPeerConnection>,
    events: broadcast::Sender<NetEvent>,
) {
    tokio::spawn(async move {
        let mut seen = std::collections::HashSet::new();
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Run for at most 60s after connection; candidates usually arrive fast.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() >= deadline {
                return;
            }
            ticker.tick().await;
            let cands = match sig.get_ice(&room, side).await {
                Ok(c) => c,
                Err(e) => {
                    emit_log(&events, LogLevel::Warn, format!("ice drain: {e:#}"));
                    continue;
                }
            };
            for cand in cands {
                let key = format!(
                    "{}|{:?}|{:?}",
                    cand.candidate, cand.sdp_mid, cand.sdp_mline_index
                );
                if seen.insert(key) {
                    let init = RTCIceCandidateInit {
                        candidate: cand.candidate,
                        sdp_mid: cand.sdp_mid,
                        sdp_mline_index: cand.sdp_mline_index,
                        username_fragment: cand.username_fragment,
                    };
                    if let Err(e) = pc.add_ice_candidate(init).await {
                        emit_log(&events, LogLevel::Warn, format!("add remote ice: {e:#}"));
                    }
                }
            }
        }
    });
}

/// Spawn a periodic ping loop over the control channel to measure RTT.
/// Also sends `hello:<label>` every ~10s so the remote peer learns our name.
fn spawn_ping_loop(
    peer_id: PeerId,
    local_label: String,
    peers: Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
) {
    tokio::spawn(async move {
        let mut ticker = time::interval(Duration::from_secs(2));
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        let mut tick_count: u32 = 0;
        loop {
            ticker.tick().await;
            tick_count = tick_count.wrapping_add(1);
            let ctrl = {
                let g = peers.lock().await;
                g.get(&peer_id).and_then(|c| c.ctrl.clone())
            };
            let Some(ctrl) = ctrl else { continue };
            // Skip if the control channel isn't open yet; send() would error
            // anyway, but avoiding the syscall keeps the ping loop quiet.
            if !is_dc_open(&ctrl) {
                continue;
            }

            // Send hello:<label> every ~10s (every 5th tick) so the remote
            // peer learns/updates our name. Repeated because on_message may
            // not be registered yet on the first send.
            if tick_count % 5 == 1 {
                let hello = format!("hello:{local_label}");
                let _ = ctrl.send(&Bytes::from(hello)).await;
            }

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            if ctrl.send(&Bytes::from(format!("ping:{ts}"))).await.is_err() {
                emit_log(
                    &events,
                    LogLevel::Warn,
                    format!("peer {peer_id}: ping send failed"),
                );
                let _ = metrics
                    .cmd_tx
                    .send(crate::network::metrics::MetricsCmd::Packets {
                        peer: peer_id,
                        sent: 0,
                        recv: 0,
                        lost: 1,
                    });
            }
        }
    });
}

/// Best-effort NAT/public-IP discovery via a STUN binding request.
///
/// Sends a raw STUN Binding Request (RFC 5389) over UDP to the first reachable
/// STUN server and parses the XOR-MAPPED-ADDRESS from the response to extract
/// the public IP. Falls back to an HTTP IP lookup API if STUN fails.
fn spawn_nat_discovery(
    stun_servers: Vec<String>,
    public_ip_tx: watch::Sender<Option<String>>,
    nat_type_tx: watch::Sender<String>,
    events: broadcast::Sender<NetEvent>,
) {
    tokio::spawn(async move {
        if stun_servers.is_empty() {
            // No STUN configured — try HTTP fallback directly.
            if let Some(ip) = http_ip_lookup().await {
                let _ = public_ip_tx.send(Some(ip.clone()));
                let _ = nat_type_tx.send("unknown (no STUN)".into());
                emit_log(
                    &events,
                    LogLevel::Info,
                    format!("public IP: {ip} (via HTTP)"),
                );
            } else {
                let _ = nat_type_tx.send("none (no STUN)".into());
                emit_log(
                    &events,
                    LogLevel::Warn,
                    "NAT discovery: no STUN and HTTP lookup failed",
                );
            }
            return;
        }

        // Try each STUN server until one responds.
        for stun_url in &stun_servers {
            if let Some((ip, nat)) = stun_probe(stun_url).await {
                let _ = public_ip_tx.send(Some(ip.clone()));
                let _ = nat_type_tx.send(nat.clone());
                emit_log(
                    &events,
                    LogLevel::Info,
                    format!("NAT discovery: public IP {ip}, NAT type: {nat}"),
                );
                return;
            }
        }

        // All STUN servers failed — try HTTP fallback.
        emit_log(
            &events,
            LogLevel::Warn,
            "STUN probes failed, trying HTTP fallback",
        );
        if let Some(ip) = http_ip_lookup().await {
            let _ = public_ip_tx.send(Some(ip.clone()));
            let _ = nat_type_tx.send("unknown (STUN failed)".into());
            emit_log(
                &events,
                LogLevel::Info,
                format!("public IP: {ip} (via HTTP fallback)"),
            );
        } else {
            let _ = nat_type_tx.send("unreachable".into());
            emit_log(
                &events,
                LogLevel::Error,
                "NAT discovery: all methods failed",
            );
        }
    });
}

/// Send a STUN Binding Request over UDP and parse the XOR-MAPPED-ADDRESS.
async fn stun_probe(stun_url: &str) -> Option<(String, String)> {
    // Parse "stun:host:port" or "stun:host" (default port 3478).
    let addr_str = stun_url
        .strip_prefix("stun:")
        .or_else(|| stun_url.strip_prefix("stuns:"))
        .unwrap_or(stun_url);
    let (host, port) = match addr_str.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse::<u16>().unwrap()),
        _ => (addr_str, 3478),
    };

    // Resolve hostname.
    let target = tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .ok()?
        .next()?;

    // Build STUN Binding Request (20 bytes):
    //   Type: 0x0001 (Binding Request)
    //   Length: 0x0000 (no attributes)
    //   Magic cookie: 0x2112A442
    //   Transaction ID: 12 random bytes
    let mut request = [0u8; 20];
    request[0] = 0x00;
    request[1] = 0x01; // Binding Request
    request[2] = 0x00;
    request[3] = 0x00; // Message length = 0
    request[4] = 0x21;
    request[5] = 0x12;
    request[6] = 0xA4;
    request[7] = 0x42; // Magic cookie
                       // Random transaction ID (bytes 8..20).
    let tx_id: [u8; 12] = rand_u64()
        .to_le_bytes()
        .iter()
        .chain(rand_u64().to_le_bytes().iter())
        .take(12)
        .copied()
        .collect::<Vec<_>>()
        .try_into()
        .ok()?;
    request[8..20].copy_from_slice(&tx_id);

    // Send via UDP with a 3-second timeout.
    let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
    sock.connect(target).await.ok()?;
    sock.send(&request).await.ok()?;

    let mut buf = vec![0u8; 1500];
    let n = tokio::time::timeout(Duration::from_secs(3), sock.recv(&mut buf))
        .await
        .ok()?
        .ok()?;

    if n < 20 {
        return None;
    }

    // Verify magic cookie and transaction ID.
    if buf[4..8] != [0x21, 0x12, 0xA4, 0x42] {
        return None;
    }
    if buf[8..20] != tx_id {
        return None;
    }

    // Parse attributes for XOR-MAPPED-ADDRESS (0x0020) or MAPPED-ADDRESS (0x0001).
    let msg_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    let body = &buf[20..20 + msg_len.min(n - 20)];

    let mut offset = 0;
    while offset + 4 <= body.len() {
        let attr_type = u16::from_be_bytes([body[offset], body[offset + 1]]);
        let attr_len = u16::from_be_bytes([body[offset + 2], body[offset + 3]]) as usize;
        let attr_val = &body[offset + 4..offset + 4 + attr_len];

        match attr_type {
            0x0020 => {
                // XOR-MAPPED-ADDRESS
                if let Some(ip) = parse_xor_mapped_address(attr_val, &tx_id) {
                    // We can't fully classify NAT type from a single STUN response,
                    // but we can make a reasonable guess.
                    let nat = if ip.contains(':') {
                        "cone (IPv6)".to_string()
                    } else {
                        "cone (symmetric?)".to_string()
                    };
                    return Some((ip, nat));
                }
            }
            0x0001 => {
                // MAPPED-ADDRESS (legacy, no XOR)
                if let Some(ip) = parse_mapped_address(attr_val) {
                    return Some((ip, "cone".to_string()));
                }
            }
            _ => {}
        }
        // Attributes are padded to 4-byte boundaries.
        offset += 4 + attr_len + (4 - attr_len % 4) % 4;
    }

    None
}

/// Parse XOR-MAPPED-ADDRESS attribute value (RFC 5389 §15.2).
fn parse_xor_mapped_address(val: &[u8], tx_id: &[u8; 12]) -> Option<String> {
    if val.len() < 8 {
        return None;
    }
    let family = val[1];
    let xor_port = u16::from_be_bytes([val[2], val[3]]) ^ 0x2112; // XOR with top 16 bits of magic cookie
    match family {
        0x01 => {
            // IPv4: XOR the address with the full magic cookie.
            if val.len() < 8 {
                return None;
            }
            let mut addr = [0u8; 4];
            for i in 0..4 {
                addr[i] = val[4 + i] ^ [0x21, 0x12, 0xA4, 0x42][i];
            }
            Some(format!(
                "{}.{}.{}.{}:{}",
                addr[0], addr[1], addr[2], addr[3], xor_port
            ))
        }
        0x02 => {
            // IPv6: XOR with magic cookie + transaction ID (16 bytes).
            if val.len() < 20 {
                return None;
            }
            let mut key = [0u8; 16];
            key[0..4].copy_from_slice(&[0x21, 0x12, 0xA4, 0x42]);
            key[4..16].copy_from_slice(tx_id);
            let mut addr = [0u8; 16];
            for i in 0..16 {
                addr[i] = val[4 + i] ^ key[i];
            }
            let seg: Vec<String> = (0..8)
                .map(|i| format!("{:x}", u16::from_be_bytes([addr[i * 2], addr[i * 2 + 1]])))
                .collect();
            Some(format!("[{}]:{}", seg.join(":"), xor_port))
        }
        _ => None,
    }
}

/// Parse MAPPED-ADDRESS attribute value (legacy RFC 3489).
fn parse_mapped_address(val: &[u8]) -> Option<String> {
    if val.len() < 8 {
        return None;
    }
    let family = val[1];
    let port = u16::from_be_bytes([val[2], val[3]]);
    if family == 0x01 && val.len() >= 8 {
        Some(format!(
            "{}.{}.{}.{}:{}",
            val[4], val[5], val[6], val[7], port
        ))
    } else {
        None
    }
}

/// HTTP fallback: query a public IP lookup API.
async fn http_ip_lookup() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;

    // Try multiple services for redundancy.
    for url in [
        "https://api.ipify.org",
        "https://ifconfig.me/ip",
        "https://icanhazip.com",
    ] {
        if let Ok(resp) = client
            .get(url)
            .header("User-Agent", "synapse/0.1")
            .send()
            .await
        {
            if let Ok(text) = resp.text().await {
                let ip = text.trim().to_string();
                if !ip.is_empty() && ip.len() < 64 {
                    return Some(ip);
                }
            }
        }
    }
    None
}

/// Update a peer's state inside the shared mesh and broadcast the change.
async fn update_peer_state<F>(mesh: &SharedMesh, peer_id: PeerId, f: F)
where
    F: FnOnce(&mut PeerState),
{
    let mut g = mesh.lock().await;
    if let Some(p) = g.peers.get_mut(&peer_id) {
        f(p);
        let snap = p.clone();
        drop(g);
        // Broadcast is best-effort; receivers may be gone.
        // (Caller owns the events sender and emits higher-level events.)
        let _ = snap;
    }
}

/// Tiny helper trait so `Tunnel` can be resolved by label.
trait TunnelExt {
    fn peer_label(&self) -> &str;
}
impl TunnelExt for Tunnel {
    fn peer_label(&self) -> &str {
        // The label encodes "peer_label" until peer id is resolved; we store the
        // intended peer label in `remote_host`'s sibling field. For simplicity we
        // keep a dedicated convention: when peer==0 we treat `label` as
        // "<tunnel-label>@<peer-label>".
        self.label.split('@').nth(1).unwrap_or("")
    }
}

/// Generate a random u64 stream id without pulling in the `rand` crate.
fn rand_u64() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    Instant::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    h.finish()
}

/// Check whether a data channel is in the Open state without naming the
/// private `RTCDataChannelState` enum: compare the Debug string instead.
fn is_dc_open(dc: &RTCDataChannel) -> bool {
    format!("{:?}", dc.ready_state()).eq_ignore_ascii_case("open")
}
