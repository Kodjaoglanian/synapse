//! Internationalization (i18n) support.
//!
//! Currently supports English (`en`) and Portuguese (`pt-BR`).
//! The language is selected via `--lang` CLI flag or `SYNAPSE_LANG` env var.

use std::sync::OnceLock;

/// Supported languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Lang {
    /// English (default).
    #[value(name = "en", aliases = ["en-US", "english"])]
    En,
    /// Portuguese (Brazil).
    #[value(name = "pt", aliases = ["pt-BR", "ptbr", "portuguese"])]
    Pt,
}

/// All UI strings, resolved for a specific language.
#[derive(Debug, Clone)]
pub struct Texts {
    pub lang: Lang,
    // Header.
    pub discovering_ip: &'static str,
    // Panel titles.
    pub title_mesh_graph: &'static str,
    pub title_tunnels: &'static str,
    pub title_throughput: &'static str,
    pub title_events: &'static str,
    pub title_quick_connect: &'static str,
    pub title_signaling: &'static str,
    pub title_help: &'static str,
    pub title_peer_detail: &'static str,
    // Stats labels.
    pub label_public_ip: &'static str,
    pub label_nat: &'static str,
    pub label_mode: &'static str,
    pub label_up: &'static str,
    pub label_down: &'static str,
    pub label_rtt: &'static str,
    pub label_peers: &'static str,
    pub label_pkts: &'static str,
    pub label_lost: &'static str,
    pub label_total: &'static str,
    // Tunnels table headers.
    pub col_label: &'static str,
    pub col_local: &'static str,
    pub col_peer: &'static str,
    pub col_status: &'static str,
    pub col_link: &'static str,
    // Help modal.
    pub help_line1: &'static str,
    pub help_line2: &'static str,
    pub help_line3: &'static str,
    pub help_line4: &'static str,
    pub help_line5: &'static str,
    pub help_line6: &'static str,
    pub help_line7: &'static str,
    pub help_line8: &'static str,
    pub help_close: &'static str,
    // Quick connect modal.
    pub qc_label_field: &'static str,
    pub qc_type_field: &'static str,
    pub qc_sdp_field: &'static str,
    pub qc_submit: &'static str,
    pub qc_cancel: &'static str,
    pub qc_hint: &'static str,
    // Signaling modal.
    pub sig_label_field: &'static str,
    pub sig_room_field: &'static str,
    pub sig_mode_field: &'static str,
    pub sig_mode_dial: &'static str,
    pub sig_mode_answer: &'static str,
    pub sig_submit: &'static str,
    pub sig_cancel: &'static str,
    pub sig_hint: &'static str,
    // Peer detail.
    pub peer_id: &'static str,
    pub peer_label: &'static str,
    pub peer_status: &'static str,
    pub peer_link: &'static str,
    pub peer_public_ip: &'static str,
    pub peer_nat: &'static str,
    pub peer_bytes: &'static str,
    pub peer_not_found: &'static str,
    // Log messages.
    pub log_peer_added: &'static str,
    pub log_peer_removed: &'static str,
    pub log_peer_connected: &'static str,
    pub log_quick_connect: &'static str,
    pub log_signaling_submitted: &'static str,
    // Headless.
    pub headless_banner: &'static str,
}

impl Lang {
    /// Resolve all UI texts for this language.
    pub fn texts(self) -> Texts {
        match self {
            Lang::En => english(),
            Lang::Pt => portuguese(),
        }
    }
}

fn english() -> Texts {
    Texts {
        lang: Lang::En,
        discovering_ip: "discovering…",
        title_mesh_graph: "Mesh Graph",
        title_tunnels: "Tunnels & Streams",
        title_throughput: "Throughput & Events",
        title_events: "Event Stream",
        title_quick_connect: "Quick Connect — paste SDP",
        title_signaling: "Connect via signaling server",
        title_help: "Help",
        title_peer_detail: "Peer Detail",
        label_public_ip: "PUBLIC IP",
        label_nat: "NAT",
        label_mode: "MODE",
        label_up: "↑ UP",
        label_down: "↓ DOWN",
        label_rtt: "RTT",
        label_peers: "PEERS",
        label_pkts: "PKTS",
        label_lost: "LOST",
        label_total: "TOTAL",
        col_label: "LABEL",
        col_local: "LOCAL",
        col_peer: "PEER",
        col_status: "STATUS",
        col_link: "LINK",
        help_line1: "Tab / Shift-Tab   Switch panel",
        help_line2: "j / k / ↑ / ↓     Navigate list",
        help_line3: "Enter             Inspect node / submit modal",
        help_line4: "n                 Quick-connect (paste raw SDP)",
        help_line5: "s                 Connect via signaling server",
        help_line6: "d                 Disconnect selected peer",
        help_line7: "?                 Toggle this help",
        help_line8: "q / Ctrl-C        Quit",
        help_close: "Press Esc/Enter to close",
        qc_label_field: "Peer label",
        qc_type_field: "SDP type (y=offer, n=answer)",
        qc_sdp_field: "SDP (JSON)",
        qc_submit: "[ Submit ]",
        qc_cancel: "[ Cancel ]",
        qc_hint: "Tab to move • Enter to submit • Esc to cancel",
        sig_label_field: "Peer label",
        sig_room_field: "Room name",
        sig_mode_field: "Mode (d=dial, a=answer)",
        sig_mode_dial: "Dial (offer)",
        sig_mode_answer: "Answer",
        sig_submit: "[ Submit ]",
        sig_cancel: "[ Cancel ]",
        sig_hint: "Tab to move • Enter to submit • Esc to cancel",
        peer_id: "id",
        peer_label: "label",
        peer_status: "status",
        peer_link: "link",
        peer_public_ip: "public ip",
        peer_nat: "nat",
        peer_bytes: "bytes",
        peer_not_found: "peer not found",
        log_peer_added: "peer added",
        log_peer_removed: "peer removed",
        log_peer_connected: "peer connected",
        log_quick_connect: "quick-connect submitted",
        log_signaling_submitted: "signaling submitted",
        headless_banner: "synapse headless — Ctrl-C to stop",
    }
}

fn portuguese() -> Texts {
    Texts {
        lang: Lang::Pt,
        discovering_ip: "descobrindo…",
        title_mesh_graph: "Grafo da Rede",
        title_tunnels: "Túneis & Fluxos",
        title_throughput: "Transferência & Eventos",
        title_events: "Fluxo de Eventos",
        title_quick_connect: "Conexão Rápida — cole o SDP",
        title_signaling: "Conectar via servidor de sinalização",
        title_help: "Ajuda",
        title_peer_detail: "Detalhes do Peer",
        label_public_ip: "IP PÚBLICO",
        label_nat: "NAT",
        label_mode: "MODO",
        label_up: "↑ ENV",
        label_down: "↓ REC",
        label_rtt: "RTT",
        label_peers: "PEERS",
        label_pkts: "PCTS",
        label_lost: "PERD",
        label_total: "TOTAL",
        col_label: "RÓTULO",
        col_local: "LOCAL",
        col_peer: "PEER",
        col_status: "ESTADO",
        col_link: "LINK",
        help_line1: "Tab / Shift-Tab   Trocar painel",
        help_line2: "j / k / ↑ / ↓     Navegar lista",
        help_line3: "Enter             Inspecionar / enviar modal",
        help_line4: "n                 Conexão rápida (colar SDP)",
        help_line5: "s                 Conectar via sinalização",
        help_line6: "d                 Desconectar peer selecionado",
        help_line7: "?                 Alternar esta ajuda",
        help_line8: "q / Ctrl-C        Sair",
        help_close: "Pressione Esc/Enter para fechar",
        qc_label_field: "Rótulo do peer",
        qc_type_field: "Tipo de SDP (y=offer, n=answer)",
        qc_sdp_field: "SDP (JSON)",
        qc_submit: "[ Enviar ]",
        qc_cancel: "[ Cancelar ]",
        qc_hint: "Tab para mover • Enter para enviar • Esc para cancelar",
        sig_label_field: "Rótulo do peer",
        sig_room_field: "Nome da sala",
        sig_mode_field: "Modo (d=dial, a=answer)",
        sig_mode_dial: "Discar (offer)",
        sig_mode_answer: "Responder",
        sig_submit: "[ Enviar ]",
        sig_cancel: "[ Cancelar ]",
        sig_hint: "Tab para mover • Enter para enviar • Esc para cancelar",
        peer_id: "id",
        peer_label: "rótulo",
        peer_status: "estado",
        peer_link: "link",
        peer_public_ip: "ip público",
        peer_nat: "nat",
        peer_bytes: "bytes",
        peer_not_found: "peer não encontrado",
        log_peer_added: "peer adicionado",
        log_peer_removed: "peer removido",
        log_peer_connected: "peer conectado",
        log_quick_connect: "conexão rápida enviada",
        log_signaling_submitted: "sinalização enviada",
        headless_banner: "synapse headless — Ctrl-C para parar",
    }
}

// --- Global singleton ---

static TEXTS: OnceLock<Texts> = OnceLock::new();

/// Initialize the global i18n texts. Call once at startup.
pub fn init(lang: Lang) {
    let _ = TEXTS.set(lang.texts());
}

/// Get the current texts. Falls back to English if not initialized.
pub fn t() -> &'static Texts {
    TEXTS.get().unwrap_or_else(|| {
        // Fallback: initialize with English if not set.
        static FALLBACK: OnceLock<Texts> = OnceLock::new();
        FALLBACK.get_or_init(english)
    })
}
