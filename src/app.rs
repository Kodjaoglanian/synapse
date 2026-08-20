//! Global application state: focus, selection, modal forms, log buffer and the
//! latest network snapshot. Input keys are translated into [`AppAction`]s here.

use std::collections::VecDeque;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::network::{EngineCmd, LogLevel, MetricsSnapshot, NetEvent, Network, PeerId, Tunnel};

/// Which panel currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Graph,
    Tunnels,
    Log,
}

impl Focus {
    /// Cycle to the next panel (Tab / Shift-Tab).
    pub fn next(self) -> Self {
        match self {
            Focus::Graph => Focus::Tunnels,
            Focus::Tunnels => Focus::Log,
            Focus::Log => Focus::Graph,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Focus::Graph => Focus::Log,
            Focus::Tunnels => Focus::Graph,
            Focus::Log => Focus::Tunnels,
        }
    }
}

/// Modal dialog state.
#[derive(Debug, Clone, Default)]
pub enum Modal {
    #[default]
    None,
    /// Quick-connect form with editable fields + cursor.
    QuickConnect(QuickConnectForm),
    /// Signaling-driven connect form (dial or answer by room name).
    SignalingRoom(SignalingRoomForm),
    /// Peer detail inspector.
    PeerDetail(PeerId),
    Help,
}

/// Form for the signaling-driven connect modal (`s` key).
#[derive(Debug, Clone)]
pub struct SignalingRoomForm {
    pub label: String,
    pub room: String,
    pub mode: SignalingMode,
    pub field: SignalingField,
    pub cursor: usize,
}

impl Default for SignalingRoomForm {
    fn default() -> Self {
        Self {
            label: String::new(),
            room: String::new(),
            mode: SignalingMode::Dial,
            field: SignalingField::Label,
            cursor: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignalingMode {
    #[default]
    Dial,
    Answer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignalingField {
    #[default]
    Label,
    Room,
    Mode,
    Submit,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct QuickConnectForm {
    pub label: String,
    pub sdp: String,
    pub is_offer: bool,
    pub field: FormField,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Label,
    Sdp,
    IsOffer,
    Submit,
    Cancel,
}

impl Default for QuickConnectForm {
    fn default() -> Self {
        Self {
            label: String::new(),
            sdp: String::new(),
            is_offer: true,
            field: FormField::Label,
            cursor: 0,
        }
    }
}

/// A single log line kept in the rolling buffer.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub msg: String,
    pub at: Instant,
}

/// Actions produced by translating key events.
#[derive(Debug, Clone)]
pub enum AppAction {
    Quit,
    Redraw,
    None,
}

/// The top-level application state.
pub struct App {
    pub focus: Focus,
    pub modal: Modal,
    pub log: VecDeque<LogEntry>,
    pub snapshot: MetricsSnapshot,
    pub public_ip: Option<String>,
    pub nat_type: String,
    pub mode: String,
    pub selected_peer: Option<usize>,
    pub selected_tunnel: Option<usize>,
    pub selected_log: Option<usize>,
    pub quit: bool,
    /// Animation tick counter (frames), drives pulse motion.
    pub tick: u64,
    pub network: Network,
}

const LOG_CAP: usize = 512;

impl App {
    pub fn new(network: Network) -> Self {
        Self {
            focus: Focus::Graph,
            modal: Modal::None,
            log: VecDeque::with_capacity(LOG_CAP),
            snapshot: MetricsSnapshot::default(),
            public_ip: None,
            nat_type: "unknown".into(),
            mode: "P2P".into(),
            selected_peer: None,
            selected_tunnel: None,
            selected_log: None,
            quit: false,
            tick: 0,
            network,
        }
    }

    /// Push a log entry, capping the buffer.
    pub fn push_log(&mut self, level: LogLevel, msg: String) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            level,
            msg,
            at: Instant::now(),
        });
    }

    /// Drain any pending network events into app state.
    pub fn drain_network_events(&mut self) {
        // Non-blocking: take only what's ready right now.
        loop {
            match self.network.events_tx.subscribe().try_recv() {
                Ok(ev) => self.apply_net_event(ev),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            }
        }
        // Refresh public IP / NAT from watch channels (non-blocking: borrow latest).
        if *self.network.public_ip.borrow() != self.public_ip {
            self.public_ip = self.network.public_ip.borrow().clone();
        }
        if *self.network.nat_type.borrow() != self.nat_type {
            self.nat_type = self.network.nat_type.borrow().clone();
        }
    }

    /// Apply a single network event to app state.
    fn apply_net_event(&mut self, ev: NetEvent) {
        match ev {
            NetEvent::Log(level, msg) => self.push_log(level, msg),
            NetEvent::PeerAdded(_) => self.push_log(LogLevel::Info, "peer added".into()),
            NetEvent::PeerUpdated(_) => {}
            NetEvent::PeerRemoved(id) => {
                self.push_log(LogLevel::Warn, format!("peer {id} removed"))
            }
            NetEvent::PeerConnected(id) => {
                self.push_log(LogLevel::Handshake, format!("peer {id} connected"))
            }
            NetEvent::PeerFailed(id, why) => {
                self.push_log(LogLevel::Error, format!("peer {id} failed: {why}"))
            }
            NetEvent::StreamOpened(s) => {
                self.push_log(LogLevel::Info, format!("stream {} open", s.id))
            }
            NetEvent::StreamUpdated(_) => {}
            NetEvent::StreamClosed(id) => {
                self.push_log(LogLevel::Info, format!("stream {id} closed"))
            }
            NetEvent::TunnelAdded(t) => {
                self.push_log(LogLevel::Info, format!("tunnel '{}' added", t.label))
            }
            NetEvent::TunnelRemoved(id) => {
                self.push_log(LogLevel::Warn, format!("tunnel {id} removed"))
            }
            NetEvent::SdpReady { peer, sdp } => {
                self.push_log(
                    LogLevel::Handshake,
                    format!("SDP ready for peer {peer} ({} bytes)", sdp.len()),
                );
            }
        }
    }

    /// Refresh the metrics snapshot from the watch channel (non-blocking).
    pub fn refresh_snapshot(&mut self) {
        // watch::Receiver exposes the latest value via borrow; mark seen so
        // has_changed() stays accurate. We just take the latest snapshot.
        if self
            .network
            .metrics
            .snapshot_rx
            .has_changed()
            .unwrap_or(false)
        {
            self.snapshot = self.network.metrics.snapshot_rx.borrow_and_update().clone();
        }
    }

    /// Advance the animation tick.
    pub fn tick_anim(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Handle a keyboard event. Returns an action for the event loop.
    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        // Modal input takes priority.
        if !matches!(self.modal, Modal::None) {
            return self.handle_modal_key(key);
        }

        // Global keys.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('c') {
                self.quit = true;
                return AppAction::Quit;
            }
        }
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return AppAction::Quit;
            }
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.prev(),
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char('n') => {
                self.modal = Modal::QuickConnect(QuickConnectForm::default());
            }
            KeyCode::Char('s') => {
                self.modal = Modal::SignalingRoom(SignalingRoomForm::default());
            }
            _ => return self.handle_panel_key(key),
        }
        AppAction::None
    }

    /// Keys scoped to the focused panel.
    fn handle_panel_key(&mut self, key: KeyEvent) -> AppAction {
        let peer_count = self.snapshot.peers.len();
        let tunnel_count = {
            // We don't keep tunnels in the snapshot; approximate from streams.
            self.snapshot.peers.values().count()
        };
        match self.focus {
            Focus::Graph => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    if peer_count > 0 {
                        let cur = self.selected_peer.unwrap_or(0);
                        self.selected_peer = Some((cur + 1) % peer_count.max(1));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if peer_count > 0 {
                        let cur = self.selected_peer.unwrap_or(0);
                        self.selected_peer =
                            Some(cur.checked_sub(1).unwrap_or(peer_count.saturating_sub(1)));
                    }
                }
                KeyCode::Enter => {
                    if let Some(idx) = self.selected_peer {
                        if let Some(pid) = self.snapshot.peers.keys().nth(idx).copied() {
                            self.modal = Modal::PeerDetail(pid);
                        }
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(idx) = self.selected_peer {
                        if let Some(pid) = self.snapshot.peers.keys().nth(idx).copied() {
                            let _ = self.network.engine.cmd_tx.send(EngineCmd::ClosePeer(pid));
                        }
                    }
                }
                _ => {}
            },
            Focus::Tunnels => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    if tunnel_count > 0 {
                        let cur = self.selected_tunnel.unwrap_or(0);
                        self.selected_tunnel = Some((cur + 1) % tunnel_count.max(1));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if tunnel_count > 0 {
                        let cur = self.selected_tunnel.unwrap_or(0);
                        self.selected_tunnel =
                            Some(cur.checked_sub(1).unwrap_or(tunnel_count.saturating_sub(1)));
                    }
                }
                KeyCode::Char('x') => {
                    // Close tunnel by index (best-effort: pick first stream's tunnel).
                    if let Some(s) = self.snapshot.peers.values().next() {
                        let _ = s;
                    }
                }
                _ => {}
            },
            Focus::Log => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    let n = self.log.len();
                    if n > 0 {
                        let cur = self.selected_log.unwrap_or(0);
                        self.selected_log = Some((cur + 1).min(n.saturating_sub(1)));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let n = self.log.len();
                    if n > 0 {
                        let cur = self.selected_log.unwrap_or(0);
                        self.selected_log = Some(cur.checked_sub(1).unwrap_or(0));
                    }
                }
                KeyCode::Char('G') => {
                    let n = self.log.len();
                    if n > 0 {
                        self.selected_log = Some(n - 1);
                    }
                }
                KeyCode::Char('g') => self.selected_log = Some(0),
                _ => {}
            },
        }
        AppAction::None
    }

    /// Keys while a modal is open.
    fn handle_modal_key(&mut self, key: KeyEvent) -> AppAction {
        match &mut self.modal {
            Modal::None => AppAction::None,
            Modal::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.modal = Modal::None;
                }
                AppAction::None
            }
            Modal::PeerDetail(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    self.modal = Modal::None;
                }
                AppAction::None
            }
            Modal::SignalingRoom(form) => {
                // Collect any side-effect to perform after the borrow on
                // `self.modal` ends, so we never borrow `self` twice.
                enum Pending {
                    None,
                    Submit { cmd: EngineCmd, msg: String },
                    Close,
                }
                let mut pending = Pending::None;
                match key.code {
                    KeyCode::Esc => pending = Pending::Close,
                    KeyCode::Tab => form.field = next_signaling_field(form.field),
                    KeyCode::BackTab => form.field = prev_signaling_field(form.field),
                    KeyCode::Enter => match form.field {
                        SignalingField::Submit => {
                            let label = form.label.clone();
                            let room = form.room.clone();
                            let (cmd, verb) = match form.mode {
                                SignalingMode::Dial => (
                                    EngineCmd::DialSignaling {
                                        label: label.clone(),
                                        room: room.clone(),
                                    },
                                    "dial",
                                ),
                                SignalingMode::Answer => (
                                    EngineCmd::AnswerSignaling {
                                        label: label.clone(),
                                        room: room.clone(),
                                    },
                                    "answer",
                                ),
                            };
                            pending = Pending::Submit {
                                cmd,
                                msg: format!("signaling {verb} submitted for room '{room}'"),
                            };
                        }
                        SignalingField::Cancel => pending = Pending::Close,
                        _ => form.field = next_signaling_field(form.field),
                    },
                    KeyCode::Backspace => {
                        if matches!(form.field, SignalingField::Label | SignalingField::Room) {
                            if form.cursor > 0 {
                                form.cursor -= 1;
                                let pos = form.cursor;
                                signaling_text_mut(form).remove(pos);
                            }
                        }
                    }
                    KeyCode::Left => {
                        if form.cursor > 0 {
                            form.cursor -= 1;
                        }
                    }
                    KeyCode::Right => {
                        let len = signaling_text_mut(form).len();
                        if form.cursor < len {
                            form.cursor += 1;
                        }
                    }
                    KeyCode::Char(' ') => {
                        if matches!(form.field, SignalingField::Label | SignalingField::Room) {
                            insert_signaling_char(form, ' ');
                        }
                    }
                    KeyCode::Char(c) => match form.field {
                        SignalingField::Label | SignalingField::Room => {
                            insert_signaling_char(form, c)
                        }
                        SignalingField::Mode => {
                            if c == 'd' {
                                form.mode = SignalingMode::Dial;
                            } else if c == 'a' {
                                form.mode = SignalingMode::Answer;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                // Apply side-effects now that the borrow on self.modal is over.
                match pending {
                    Pending::None => {}
                    Pending::Submit { cmd, msg } => {
                        let _ = self.network.engine.cmd_tx.send(cmd);
                        self.push_log(LogLevel::Info, msg);
                        self.modal = Modal::None;
                    }
                    Pending::Close => self.modal = Modal::None,
                }
                AppAction::None
            }
            Modal::QuickConnect(form) => {
                match key.code {
                    KeyCode::Esc => {
                        self.modal = Modal::None;
                    }
                    KeyCode::Tab => form.field = next_field(form.field),
                    KeyCode::BackTab => form.field = prev_field(form.field),
                    KeyCode::Enter => match form.field {
                        FormField::Submit => {
                            let label = form.label.clone();
                            let sdp = form.sdp.clone();
                            let is_offer = form.is_offer;
                            let cmd = EngineCmd::QuickConnect {
                                label,
                                sdp,
                                is_offer,
                            };
                            let _ = self.network.engine.cmd_tx.send(cmd);
                            self.push_log(LogLevel::Info, "quick-connect submitted".into());
                            self.modal = Modal::None;
                        }
                        FormField::Cancel => self.modal = Modal::None,
                        _ => form.field = next_field(form.field),
                    },
                    KeyCode::Backspace => {
                        if matches!(form.field, FormField::Label | FormField::Sdp) {
                            if form.cursor > 0 {
                                form.cursor -= 1;
                                let pos = form.cursor;
                                active_text_mut(form).remove(pos);
                            }
                        }
                    }
                    KeyCode::Left => {
                        if form.cursor > 0 {
                            form.cursor -= 1;
                        }
                    }
                    KeyCode::Right => {
                        let len = active_text_mut(form).len();
                        if form.cursor < len {
                            form.cursor += 1;
                        }
                    }
                    KeyCode::Char(' ') => {
                        if matches!(form.field, FormField::Label | FormField::Sdp) {
                            insert_char(form, ' ');
                        }
                    }
                    KeyCode::Char(c) => match form.field {
                        FormField::Label | FormField::Sdp => insert_char(form, c),
                        FormField::IsOffer => {
                            if c == 'y' {
                                form.is_offer = true;
                            } else if c == 'n' {
                                form.is_offer = false;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                AppAction::None
            }
        }
    }
}

fn active_text_mut(form: &mut QuickConnectForm) -> &mut String {
    match form.field {
        FormField::Label => &mut form.label,
        FormField::Sdp => &mut form.sdp,
        _ => &mut form.label, // unused for non-text fields
    }
}

fn insert_char(form: &mut QuickConnectForm, c: char) {
    // Clamp cursor first (no outstanding borrow), then insert.
    let len = active_text_mut(form).len();
    if form.cursor > len {
        form.cursor = len;
    }
    let pos = form.cursor;
    active_text_mut(form).insert(pos, c);
    form.cursor += 1;
}

// --- SignalingRoom form helpers ---

fn signaling_text_mut(form: &mut SignalingRoomForm) -> &mut String {
    match form.field {
        SignalingField::Label => &mut form.label,
        SignalingField::Room => &mut form.room,
        _ => &mut form.label, // unused for non-text fields
    }
}

fn insert_signaling_char(form: &mut SignalingRoomForm, c: char) {
    let len = signaling_text_mut(form).len();
    if form.cursor > len {
        form.cursor = len;
    }
    let pos = form.cursor;
    signaling_text_mut(form).insert(pos, c);
    form.cursor += 1;
}

fn next_signaling_field(f: SignalingField) -> SignalingField {
    match f {
        SignalingField::Label => SignalingField::Room,
        SignalingField::Room => SignalingField::Mode,
        SignalingField::Mode => SignalingField::Submit,
        SignalingField::Submit => SignalingField::Cancel,
        SignalingField::Cancel => SignalingField::Label,
    }
}

fn prev_signaling_field(f: SignalingField) -> SignalingField {
    match f {
        SignalingField::Label => SignalingField::Cancel,
        SignalingField::Room => SignalingField::Label,
        SignalingField::Mode => SignalingField::Room,
        SignalingField::Submit => SignalingField::Mode,
        SignalingField::Cancel => SignalingField::Submit,
    }
}

fn next_field(f: FormField) -> FormField {
    match f {
        FormField::Label => FormField::Sdp,
        FormField::Sdp => FormField::IsOffer,
        FormField::IsOffer => FormField::Submit,
        FormField::Submit => FormField::Cancel,
        FormField::Cancel => FormField::Label,
    }
}
fn prev_field(f: FormField) -> FormField {
    match f {
        FormField::Label => FormField::Cancel,
        FormField::Sdp => FormField::Label,
        FormField::IsOffer => FormField::Sdp,
        FormField::Submit => FormField::IsOffer,
        FormField::Cancel => FormField::Submit,
    }
}

/// Helper to open a new tunnel from the UI (used by future actions).
#[allow(dead_code)]
pub fn open_tunnel(app: &mut App, t: Tunnel) {
    let _ = app.network.engine.cmd_tx.send(EngineCmd::OpenTunnel(t));
}

/// Helper to dial a peer from the UI.
#[allow(dead_code)]
pub fn dial(app: &mut App, label: String, token: String) {
    let _ = app
        .network
        .engine
        .cmd_tx
        .send(EngineCmd::Dial { label, token });
}
