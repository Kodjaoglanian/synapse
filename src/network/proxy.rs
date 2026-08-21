//! Local TCP proxy.
//!
//! For each tunnel we bind a `tokio::net::TcpListener`. On every accepted TCP
//! connection we open (or reuse) a peer data channel and pump bytes both ways
//! using `tokio::io::copy_bidirectional` against an adapter that wraps the
//! data channel into an `AsyncRead`/`AsyncWrite` stream.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Mutex};
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::RTCDataChannel;

use super::{
    emit_log, LinkKind, LogLevel, MeshState, NetEvent, PeerId, SharedMesh, StreamInfo,
    StreamStatus, Tunnel,
};
use crate::network::engine::PeerCtx;
use crate::network::metrics::MetricsHandle;

fn encode_tunnel_endpoint(host: &str, port: u16) -> Result<String> {
    Ok(serde_json::to_string(&(host, port))?)
}

pub(crate) fn decode_tunnel_endpoint(protocol: &str) -> Result<(String, u16)> {
    Ok(serde_json::from_str(protocol)?)
}

/// Spawn a local TCP listener for a tunnel. Runs until the listener errors or
/// the runtime shuts down.
pub async fn spawn_listener(
    tunnel: Tunnel,
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
    peers: Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
) -> Result<()> {
    let listener = TcpListener::bind(tunnel.local_addr)
        .await
        .with_context(|| format!("bind {}", tunnel.local_addr))?;
    emit_log(
        &events,
        LogLevel::Info,
        format!(
            "listening {} for tunnel '{}'",
            tunnel.local_addr, tunnel.label
        ),
    );

    loop {
        let (mut tcp, peer_addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                emit_log(
                    &events,
                    LogLevel::Error,
                    format!("accept {}: {e}", tunnel.local_addr),
                );
                continue;
            }
        };

        // Resolve the peer's data channel for this tunnel.
        let dc = resolve_tunnel_dc(&peers, &mesh, &tunnel).await;
        let dc = match dc {
            Ok(dc) => dc,
            Err(e) => {
                emit_log(
                    &events,
                    LogLevel::Warn,
                    format!(
                        "tunnel '{}': no peer channel yet ({e}); closing tcp",
                        tunnel.label
                    ),
                );
                let _ = tcp.shutdown().await;
                continue;
            }
        };

        let mesh_c = mesh.clone();
        let events_c = events.clone();
        let metrics_c = metrics.clone();
        let events_err = events.clone();
        let t = tunnel.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(tcp, peer_addr, dc, t, mesh_c, events_c, metrics_c).await {
                emit_log(
                    &events_err,
                    LogLevel::Error,
                    format!("conn {peer_addr}: {e:#}"),
                );
            }
        });
    }
}

/// Resolve (creating if needed) the data channel for a tunnel on its peer.
async fn resolve_tunnel_dc(
    peers: &Arc<Mutex<HashMap<PeerId, PeerCtx>>>,
    mesh: &SharedMesh,
    tunnel: &Tunnel,
) -> Result<Arc<RTCDataChannel>> {
    // If tunnel.peer is 0, resolve by label convention.
    let peer_id = if tunnel.peer == 0 {
        let g = mesh.lock().await;
        g.peers
            .iter()
            .find(|(_, p)| p.label == tunnel.peer_label)
            .map(|(k, _)| *k)
            .ok_or_else(|| anyhow!("peer not connected yet"))?
    } else {
        tunnel.peer
    };

    let label = format!("synapse-tun-{}", tunnel.id);
    let mut g = peers.lock().await;
    let ctx = g
        .get_mut(&peer_id)
        .ok_or_else(|| anyhow!("peer {peer_id} gone"))?;
    let protocol = encode_tunnel_endpoint(&tunnel.remote_host, tunnel.remote_port)?;
    let init = RTCDataChannelInit {
        protocol: Some(protocol),
        ..Default::default()
    };
    // Create a new data channel (dialer side).
    let dc = ctx
        .pc
        .create_data_channel(&label, Some(init))
        .await
        .context("create data channel")?;
    ctx.tunnels.insert(tunnel.id, Arc::clone(&dc));
    Ok(dc)
}

async fn wait_data_channel_open(dc: &RTCDataChannel) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if format!("{:?}", dc.ready_state()) == "Open" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("data channel open timeout")?;
    Ok(())
}

/// Pump bytes between a TCP socket and a data channel.
async fn handle_conn(
    tcp: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    dc: Arc<RTCDataChannel>,
    tunnel: Tunnel,
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
) -> Result<()> {
    let _ = peer_addr;
    wait_data_channel_open(&dc).await?;
    let sid = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::time::Instant::now().hash(&mut h);
        h.finish()
    };

    let tunnel_id = tunnel.id;
    let peer_id = tunnel.peer;
    let tunnel_label = tunnel.label.clone();

    let info = StreamInfo {
        id: sid,
        tunnel_id,
        peer: peer_id,
        status: StreamStatus::Established,
        opened_at: std::time::Instant::now(),
        bytes_sent: 0,
        bytes_recv: 0,
    };
    {
        let mut g = mesh.lock().await;
        g.streams.insert(sid, info.clone());
    }
    let _ = events.send(NetEvent::StreamOpened(info));
    emit_log(
        &events,
        LogLevel::Info,
        format!("stream {sid} open on tunnel '{tunnel_label}'"),
    );

    // Split the TCP stream.
    let (mut tcp_rd, mut tcp_wr) = tcp.into_split();

    // Adapter wrapping the data channel into an AsyncWrite (tcp <- peer).
    let mut dc_writer = DataChannelWriter::new(dc.clone());
    // Adapter wrapping the data channel into an AsyncRead (peer -> tcp).
    let mut dc_reader = DataChannelReader::new(dc.clone());
    // Keep a handle for the final close() call (the up task moves its own clone).
    let dc_for_close = dc.clone();

    let metrics_up = metrics.clone();
    let metrics_down = metrics.clone();
    let events_up = events.clone();
    let events_down = events.clone();
    let mesh_up = mesh.clone();
    let mesh_down = mesh.clone();
    let dc_up = dc.clone();

    // tcp -> dc (upload)
    let up = tokio::spawn(async move {
        let mut total = 0u64;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match tcp_rd.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    emit_log(&events_up, LogLevel::Warn, format!("tcp read: {e}"));
                    break;
                }
            };
            if dc_up
                .send(&bytes::Bytes::copy_from_slice(&buf[..n]))
                .await
                .is_err()
            {
                break;
            }
            total += n as u64;
            {
                let mut g = mesh_up.lock().await;
                if let Some(stream) = g.streams.get_mut(&sid) {
                    stream.status = StreamStatus::Transferring;
                    stream.bytes_sent = total;
                }
            }
            let _ = metrics_up
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Bytes {
                    peer: peer_id,
                    sent: n as u64,
                    recv: 0,
                });
            let _ = metrics_up
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Packets {
                    peer: peer_id,
                    sent: 1,
                    recv: 0,
                    lost: 0,
                });
            let _ = events_up.send(NetEvent::StreamUpdated(StreamInfo {
                id: sid,
                tunnel_id,
                peer: peer_id,
                status: StreamStatus::Transferring,
                opened_at: std::time::Instant::now(),
                bytes_sent: total,
                bytes_recv: 0,
            }));
        }
        total
    });

    // dc -> tcp (download)
    let down = tokio::spawn(async move {
        let mut total = 0u64;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match dc_reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if tcp_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
            total += n as u64;
            {
                let mut g = mesh_down.lock().await;
                if let Some(stream) = g.streams.get_mut(&sid) {
                    stream.status = StreamStatus::Transferring;
                    stream.bytes_recv = total;
                }
            }
            let _ = metrics_down
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Bytes {
                    peer: peer_id,
                    sent: 0,
                    recv: n as u64,
                });
            let _ = metrics_down
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Packets {
                    peer: peer_id,
                    sent: 0,
                    recv: 1,
                    lost: 0,
                });
            let _ = events_down.send(NetEvent::StreamUpdated(StreamInfo {
                id: sid,
                tunnel_id,
                peer: peer_id,
                status: StreamStatus::Transferring,
                opened_at: std::time::Instant::now(),
                bytes_sent: 0,
                bytes_recv: total,
            }));
        }
        total
    });

    let _ = up.await;
    let _ = down.await;
    let _ = dc_writer.flush().await;
    let _ = dc_for_close.close().await;

    {
        let mut g = mesh.lock().await;
        if let Some(s) = g.streams.get_mut(&sid) {
            s.status = StreamStatus::Closed;
        }
        g.streams.remove(&sid);
    }
    let _ = events.send(NetEvent::StreamClosed(sid));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_remote_endpoint(
    dc: Arc<RTCDataChannel>,
    tunnel_id: u32,
    peer_id: PeerId,
    remote_host: String,
    remote_port: u16,
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
) {
    let dc_reader = DataChannelReader::new(Arc::clone(&dc));
    let events_err = events.clone();
    tokio::spawn(async move {
        if let Err(e) = handle_remote_endpoint(
            dc,
            dc_reader,
            tunnel_id,
            peer_id,
            remote_host,
            remote_port,
            mesh,
            events,
            metrics,
        )
        .await
        {
            emit_log(
                &events_err,
                LogLevel::Error,
                format!("remote tunnel {tunnel_id}: {e:#}"),
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn handle_remote_endpoint(
    dc: Arc<RTCDataChannel>,
    mut dc_reader: DataChannelReader,
    tunnel_id: u32,
    peer_id: PeerId,
    remote_host: String,
    remote_port: u16,
    mesh: SharedMesh,
    events: broadcast::Sender<NetEvent>,
    metrics: MetricsHandle,
) -> Result<()> {
    let tcp = TcpStream::connect((remote_host.as_str(), remote_port))
        .await
        .with_context(|| format!("connect {remote_host}:{remote_port}"))?;
    wait_data_channel_open(&dc).await?;

    let sid = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::time::Instant::now().hash(&mut h);
        h.finish()
    };
    let info = StreamInfo {
        id: sid,
        tunnel_id,
        peer: peer_id,
        status: StreamStatus::Established,
        opened_at: std::time::Instant::now(),
        bytes_sent: 0,
        bytes_recv: 0,
    };
    mesh.lock().await.streams.insert(sid, info.clone());
    let _ = events.send(NetEvent::StreamOpened(info));
    emit_log(
        &events,
        LogLevel::Info,
        format!("remote tunnel {tunnel_id} connected to {remote_host}:{remote_port}"),
    );

    let (mut tcp_rd, mut tcp_wr) = tcp.into_split();
    let mesh_recv = mesh.clone();
    let mesh_send = mesh.clone();
    let events_recv = events.clone();
    let events_send = events.clone();
    let metrics_recv = metrics.clone();
    let metrics_send = metrics.clone();
    let dc_send = Arc::clone(&dc);

    let mut recv_task = tokio::spawn(async move {
        let mut total = 0u64;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match dc_reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if tcp_wr.write_all(&buf[..n]).await.is_err() {
                break;
            }
            total += n as u64;
            if let Some(stream) = mesh_recv.lock().await.streams.get_mut(&sid) {
                stream.status = StreamStatus::Transferring;
                stream.bytes_recv = total;
            }
            let _ = metrics_recv
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Bytes {
                    peer: peer_id,
                    sent: 0,
                    recv: n as u64,
                });
            let _ = metrics_recv
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Packets {
                    peer: peer_id,
                    sent: 0,
                    recv: 1,
                    lost: 0,
                });
            let _ = events_recv.send(NetEvent::StreamUpdated(StreamInfo {
                id: sid,
                tunnel_id,
                peer: peer_id,
                status: StreamStatus::Transferring,
                opened_at: std::time::Instant::now(),
                bytes_sent: 0,
                bytes_recv: total,
            }));
        }
    });

    let mut send_task = tokio::spawn(async move {
        let mut total = 0u64;
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match tcp_rd.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if dc_send
                .send(&bytes::Bytes::copy_from_slice(&buf[..n]))
                .await
                .is_err()
            {
                break;
            }
            total += n as u64;
            if let Some(stream) = mesh_send.lock().await.streams.get_mut(&sid) {
                stream.status = StreamStatus::Transferring;
                stream.bytes_sent = total;
            }
            let _ = metrics_send
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Bytes {
                    peer: peer_id,
                    sent: n as u64,
                    recv: 0,
                });
            let _ = metrics_send
                .cmd_tx
                .send(crate::network::metrics::MetricsCmd::Packets {
                    peer: peer_id,
                    sent: 1,
                    recv: 0,
                    lost: 0,
                });
            let _ = events_send.send(NetEvent::StreamUpdated(StreamInfo {
                id: sid,
                tunnel_id,
                peer: peer_id,
                status: StreamStatus::Transferring,
                opened_at: std::time::Instant::now(),
                bytes_sent: total,
                bytes_recv: 0,
            }));
        }
    });

    tokio::select! {
        _ = &mut recv_task => send_task.abort(),
        _ = &mut send_task => recv_task.abort(),
    }
    let _ = dc.close().await;
    mesh.lock().await.streams.remove(&sid);
    let _ = events.send(NetEvent::StreamClosed(sid));
    Ok(())
}

/// Wrap a data channel as an `AsyncWrite` (bytes going onto the channel).
struct DataChannelWriter {
    dc: Arc<RTCDataChannel>,
}

impl DataChannelWriter {
    fn new(dc: Arc<RTCDataChannel>) -> Self {
        Self { dc }
    }
}

impl AsyncWrite for DataChannelWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        // webrtc's send is async; we do a blocking-style send via try_send if
        // available, otherwise buffer. RTCDataChannel::send is async only, so we
        // use a small sync path: copy bytes and spawn a send. To keep semantics
        // correct we instead report WouldBlock and rely on the dedicated pump
        // task above. This writer is only used for flush/close semantics.
        let _ = buf;
        std::task::Poll::Ready(Ok(0))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// Wrap a data channel as an `AsyncRead` (bytes coming off the channel).
struct DataChannelReader {
    dc: Arc<RTCDataChannel>,
    rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
}

impl DataChannelReader {
    fn new(dc: Arc<RTCDataChannel>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let tx = Arc::new(tx);
        dc.on_message(Box::new({
            let tx = Arc::clone(&tx);
            Box::new(
                move |msg: webrtc::data_channel::data_channel_message::DataChannelMessage| {
                    let tx = Arc::clone(&tx);
                    Box::pin(async move {
                        let _ = tx.send(msg.data.to_vec()).await;
                    })
                        as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>
                },
            )
        }));
        dc.on_close(Box::new({
            let tx = Arc::clone(&tx);
            move || {
                let tx = Arc::clone(&tx);
                Box::pin(async move {
                    let _ = tx.send(Vec::new()).await;
                })
            }
        }));
        Self { dc, rx }
    }
}

impl AsyncRead for DataChannelReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            std::task::Poll::Ready(Some(data)) => {
                let n = data.len().min(buf.remaining());
                buf.put_slice(&data[..n]);
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(None) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

// Silence unused-field warnings for adapters that intentionally keep handles.
#[allow(dead_code)]
fn _touch(_m: &MeshState, _l: LinkKind) {}

#[cfg(test)]
mod tests {
    use super::{decode_tunnel_endpoint, encode_tunnel_endpoint};

    #[test]
    fn tunnel_endpoint_protocol_round_trips() {
        let encoded = encode_tunnel_endpoint("::1", 19090).unwrap();
        assert_eq!(
            decode_tunnel_endpoint(&encoded).unwrap(),
            ("::1".to_string(), 19090)
        );
    }
}
