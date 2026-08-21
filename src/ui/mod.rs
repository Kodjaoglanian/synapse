//! UI layer: master layout + per-panel renderers.
//!
//! The layout is a responsive grid:
//!
//! ```text
//! ┌──────────────────────── Header ────────────────────────┐
//! │  ASCII banner + global stats (IP/NAT/throughput/RTT)   │
//! ├──────────────────────────┬─────────────────────────────┤
//! │   Graph (mesh canvas)    │   Tunnels & streams table   │
//! │   interactive nodes      │   inspector                 │
//! ├──────────────────────────┴─────────────────────────────┤
//! │   Sparklines (up/down) + Event log                     │
//! └────────────────────────────────────────────────────────┘
//! ```

pub mod graph;
pub mod header;
pub mod sparklines;
pub mod theme;
pub mod tunnels;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear};
use ratatui::Frame;

use crate::app::{App, Modal};
use theme as t;

/// Top-level draw entrypoint called every frame.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.size();

    // Compute layout dynamically based on available height.
    // Priority: give the middle panel (graph + tunnels) as much space as possible.
    // compact = no ASCII banner; ultra = minimal header + bottom.
    let (header_h, bottom_h, compact) = if area.height >= 40 {
        (10, 10, false) // full mode: ASCII banner + sparklines
    } else if area.height >= 24 {
        (5, 7, true) // compact: stats only, log only
    } else {
        (4, 4, true) // ultra compact: 2-line header, 2-line log
    };

    // Background fill so the palette reads consistently.
    f.render_widget(
        Block::default().style(ratatui::style::Style::default().bg(t::palette::BG)),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h),
            Constraint::Min(8), // middle always gets at least 8 rows
            Constraint::Length(bottom_h),
        ])
        .split(area);

    header::draw(f, app, chunks[0], compact);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);
    graph::draw(f, app, middle[0]);
    tunnels::draw(f, app, middle[1]);

    sparklines::draw(f, app, chunks[2], compact);

    // Modal overlay.
    draw_modal(f, app);
}

/// Render the active modal on top of everything.
fn draw_modal(f: &mut Frame, app: &mut App) {
    match &app.modal {
        Modal::None => {}
        Modal::Help => draw_help(f, app),
        Modal::PeerDetail(pid) => draw_peer_detail(f, app, *pid),
        Modal::QuickConnect(form) => draw_quick_connect(f, app, form),
        Modal::SignalingRoom(form) => draw_signaling_room(f, app, form),
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(h)) / 2),
            Constraint::Length(h),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width.saturating_sub(w)) / 2),
            Constraint::Length(w),
            Constraint::Min(0),
        ])
        .split(popup[1])[1]
}

fn draw_help(f: &mut Frame, _app: &App) {
    let area = centered(f.size(), 52, 14);
    f.render_widget(Clear, area);
    let tx = crate::i18n::t();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t::panel_border_active_style())
        .title(t::panel_title("?", tx.title_help));
    f.render_widget(block, area);

    let lines = vec![
        ratatui::text::Line::from(vec![ratatui::text::Span::styled(
            "Keybindings",
            t::title_style(),
        )]),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  Tab / Shift-Tab ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line1)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  j / k  or  ↑/↓  ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line2)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  Enter           ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line3)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  n               ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line4)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  s               ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line5)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  d               ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line6)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  ?               ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line7)),
        ]),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled("  q / Ctrl-C      ", t::status_ok_style()),
            ratatui::text::Span::raw(format!("  {}", tx.help_line8)),
        ]),
        ratatui::text::Line::raw(""),
        ratatui::text::Line::styled(tx.help_close, t::dim_style()),
    ];
    let p = ratatui::widgets::Paragraph::new(lines).style(
        ratatui::style::Style::default()
            .bg(t::palette::BG)
            .fg(t::palette::FG),
    );
    f.render_widget(
        p,
        area.inner(&ratatui::layout::Margin {
            vertical: 1,
            horizontal: 1,
        }),
    );
}

fn draw_peer_detail(f: &mut Frame, app: &App, pid: crate::network::PeerId) {
    let area = centered(f.size(), 56, 12);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t::panel_border_active_style())
        .title(t::panel_title("◉", "Peer Inspector"));
    f.render_widget(block, area);

    let p = app.snapshot.peers.get(&pid);
    let lines = if let Some(p) = p {
        vec![
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  label     ", t::header_label_style()),
                ratatui::text::Span::styled(p.label.clone(), t::header_value_style()),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  id        ", t::header_label_style()),
                ratatui::text::Span::raw(format!("{:#018x}", p.id)),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  status    ", t::header_label_style()),
                ratatui::text::Span::styled(format!("{:?}", p.status), status_style(p.status)),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  link      ", t::header_label_style()),
                ratatui::text::Span::styled(
                    format!("{:?} (rtt {} ms)", p.link, p.rtt_ms),
                    link_style(p.link),
                ),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  public ip ", t::header_label_style()),
                ratatui::text::Span::raw(p.public_ip.clone().unwrap_or_else(|| "—".into())),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  nat       ", t::header_label_style()),
                ratatui::text::Span::raw(p.nat_type.clone().unwrap_or_else(|| "—".into())),
            ]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  bytes     ", t::header_label_style()),
                ratatui::text::Span::raw(format!(
                    "↑ {}  ↓ {}",
                    crate::network::fmt_bytes(p.bytes_sent),
                    crate::network::fmt_bytes(p.bytes_recv)
                )),
            ]),
            ratatui::text::Line::raw(""),
            ratatui::text::Line::styled("  Esc/Enter to close", t::dim_style()),
        ]
    } else {
        vec![ratatui::text::Line::styled(
            "peer not found",
            t::status_err_style(),
        )]
    };
    let para = ratatui::widgets::Paragraph::new(lines).style(
        ratatui::style::Style::default()
            .bg(t::palette::BG)
            .fg(t::palette::FG),
    );
    f.render_widget(
        para,
        area.inner(&ratatui::layout::Margin {
            vertical: 1,
            horizontal: 1,
        }),
    );
}

fn draw_quick_connect(f: &mut Frame, _app: &App, form: &crate::app::QuickConnectForm) {
    use crate::app::FormField;
    let area = centered(f.size(), 64, 16);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t::panel_border_active_style())
        .title(t::panel_title("⇄", crate::i18n::t().title_quick_connect));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin {
        vertical: 1,
        horizontal: 2,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // label
            Constraint::Length(3), // is_offer
            Constraint::Min(3),    // sdp
            Constraint::Length(1), // buttons
            Constraint::Length(1), // hint
        ])
        .split(inner);

    let field_style = |active: bool| {
        if active {
            ratatui::style::Style::default()
                .fg(t::palette::BLUE)
                .add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            ratatui::style::Style::default().fg(t::palette::FG)
        }
    };

    // Label field.
    let label_block = Block::default()
        .borders(Borders::ALL)
        .border_style(field_style(matches!(form.field, FormField::Label)))
        .title("Peer label");
    let label_text = render_text_field(
        &form.label,
        form.cursor,
        matches!(form.field, FormField::Label),
    );
    f.render_widget(
        ratatui::widgets::Paragraph::new(label_text).block(label_block),
        chunks[0],
    );

    // is_offer toggle.
    let off = format!(
        "Type: [{}] offer  [{}] answer",
        if form.is_offer { "x" } else { " " },
        if !form.is_offer { "x" } else { " " }
    );
    let off_block = Block::default()
        .borders(Borders::ALL)
        .border_style(field_style(matches!(form.field, FormField::IsOffer)))
        .title("SDP type (y=offer, n=answer)");
    f.render_widget(
        ratatui::widgets::Paragraph::new(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(off, field_style(matches!(form.field, FormField::IsOffer))),
        ]))
        .block(off_block),
        chunks[1],
    );

    // SDP textarea.
    let sdp_block = Block::default()
        .borders(Borders::ALL)
        .border_style(field_style(matches!(form.field, FormField::Sdp)))
        .title("SDP (JSON)");
    let sdp_text = render_text_field(&form.sdp, form.cursor, matches!(form.field, FormField::Sdp));
    f.render_widget(
        ratatui::widgets::Paragraph::new(sdp_text)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(sdp_block),
        chunks[2],
    );

    // Buttons.
    let btns = ratatui::text::Line::from(vec![
        ratatui::text::Span::styled(
            " [Submit] ",
            field_style(matches!(form.field, FormField::Submit)),
        ),
        ratatui::text::Span::raw("   "),
        ratatui::text::Span::styled(
            " [Cancel] ",
            field_style(matches!(form.field, FormField::Cancel)),
        ),
    ]);
    f.render_widget(btns, chunks[3]);

    f.render_widget(
        ratatui::text::Line::styled(
            "Tab to move • Enter to submit • Esc to cancel",
            t::dim_style(),
        ),
        chunks[4],
    );
}

fn draw_signaling_room(f: &mut Frame, _app: &App, form: &crate::app::SignalingRoomForm) {
    use crate::app::{SignalingField, SignalingMode};
    let area = centered(f.size(), 56, 12);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t::panel_border_active_style())
        .title(t::panel_title("⇄", crate::i18n::t().title_signaling));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin {
        vertical: 1,
        horizontal: 2,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // label
            Constraint::Length(3), // room
            Constraint::Length(3), // mode
            Constraint::Length(1), // buttons
            Constraint::Length(1), // hint
        ])
        .split(inner);

    let field_style = |active: bool| {
        if active {
            ratatui::style::Style::default()
                .fg(t::palette::BLUE)
                .add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            ratatui::style::Style::default().fg(t::palette::FG)
        }
    };

    // Label.
    let label_block = Block::default()
        .borders(Borders::ALL)
        .border_style(field_style(matches!(form.field, SignalingField::Label)))
        .title("Local name");
    let label_text = render_text_field(
        &form.label,
        form.cursor,
        matches!(form.field, SignalingField::Label),
    );
    f.render_widget(
        ratatui::widgets::Paragraph::new(label_text).block(label_block),
        chunks[0],
    );

    // Room.
    let room_block = Block::default()
        .borders(Borders::ALL)
        .border_style(field_style(matches!(form.field, SignalingField::Room)))
        .title("Room name (shared with the other peer)");
    let room_text = render_text_field(
        &form.room,
        form.cursor,
        matches!(form.field, SignalingField::Room),
    );
    f.render_widget(
        ratatui::widgets::Paragraph::new(room_text).block(room_block),
        chunks[1],
    );

    // Mode toggle.
    let mode = format!(
        "Mode: [{}] dial (offer)  [{}] answer",
        if form.mode == SignalingMode::Dial {
            "x"
        } else {
            " "
        },
        if form.mode == SignalingMode::Answer {
            "x"
        } else {
            " "
        },
    );
    let mode_block = Block::default()
        .borders(Borders::ALL)
        .border_style(field_style(matches!(form.field, SignalingField::Mode)))
        .title("Role (d=dial/offer, a=answer)");
    f.render_widget(
        ratatui::widgets::Paragraph::new(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                mode,
                field_style(matches!(form.field, SignalingField::Mode)),
            ),
        ]))
        .block(mode_block),
        chunks[2],
    );

    // Buttons.
    let btns = ratatui::text::Line::from(vec![
        ratatui::text::Span::styled(
            " [Submit] ",
            field_style(matches!(form.field, SignalingField::Submit)),
        ),
        ratatui::text::Span::raw("   "),
        ratatui::text::Span::styled(
            " [Cancel] ",
            field_style(matches!(form.field, SignalingField::Cancel)),
        ),
    ]);
    f.render_widget(btns, chunks[3]);

    f.render_widget(
        ratatui::text::Line::styled(
            "Tab to move • Enter to submit • Esc to cancel • needs --signaling",
            t::dim_style(),
        ),
        chunks[4],
    );
}

fn render_text_field(text: &str, cursor: usize, active: bool) -> ratatui::text::Text<'static> {
    let mut spans: Vec<ratatui::text::Span> = Vec::new();
    let cursor = cursor.min(text.len());
    let before = text[..cursor].to_string();
    let ch = if cursor < text.len() {
        text[cursor..cursor + 1].to_string()
    } else {
        " ".to_string()
    };
    let after = if cursor < text.len() {
        text[cursor + 1..].to_string()
    } else {
        String::new()
    };
    spans.push(ratatui::text::Span::raw(before));
    if active {
        spans.push(ratatui::text::Span::styled(
            ch,
            ratatui::style::Style::default()
                .bg(t::palette::BLUE)
                .fg(t::palette::BG),
        ));
    } else {
        spans.push(ratatui::text::Span::raw(ch));
    }
    spans.push(ratatui::text::Span::raw(after));
    ratatui::text::Text::from(ratatui::text::Line::from(spans))
}

/// Map a peer status to a style.
pub fn status_style(s: crate::network::PeerStatus) -> ratatui::style::Style {
    use crate::network::PeerStatus;
    match s {
        PeerStatus::Connected => t::status_ok_style(),
        PeerStatus::Connecting | PeerStatus::Gathering => t::status_warn_style(),
        PeerStatus::Failed => t::status_err_style(),
        PeerStatus::Closed => t::dim_style(),
        PeerStatus::Idle => t::dim_style(),
    }
}

/// Map a link kind to a style.
pub fn link_style(l: crate::network::LinkKind) -> ratatui::style::Style {
    use crate::network::LinkKind;
    match l {
        LinkKind::DirectFast => t::status_ok_style(),
        LinkKind::DirectModerate => t::status_warn_style(),
        LinkKind::DirectSlow => t::status_warn_style(),
        LinkKind::Relay => t::status_relay_style(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tokio::sync::{broadcast, mpsc, watch, Mutex};

    use crate::app::App;
    use crate::network::engine::EngineHandle;
    use crate::network::metrics::{MetricsHandle, MetricsSnapshot};
    use crate::network::{MeshState, Network, PeerState, PeerStatus};

    #[test]
    fn renders_connected_peer_in_graph_and_list_at_80x24() {
        let mesh = Arc::new(Mutex::new(MeshState::default()));
        let (metrics_tx, _metrics_rx) = mpsc::unbounded_channel();
        let (_snapshot_tx, snapshot_rx) = watch::channel(MetricsSnapshot::default());
        let (events_tx, events_rx) = broadcast::channel(8);
        let (_public_ip_tx, public_ip) = watch::channel(None::<String>);
        let (_nat_type_tx, nat_type) = watch::channel("unknown".to_string());
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let network = Network {
            mesh,
            metrics: MetricsHandle {
                cmd_tx: metrics_tx,
                snapshot_rx,
            },
            events_tx,
            events_rx,
            public_ip,
            nat_type,
            engine: EngineHandle { cmd_tx },
        };
        let mut app = App::new(network);
        app.local_label = "alice".to_string();
        app.snapshot.peers.insert(
            1,
            PeerState {
                id: 1,
                label: "bob".to_string(),
                status: PeerStatus::Connected,
                ..Default::default()
            },
        );
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();

        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer.get(x, y).symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("synapse · local: alice"), "{rendered}");
        assert!(rendered.contains("● bob  direct"), "{rendered}");
        assert!(!rendered.contains("● alice"), "{rendered}");
        assert!(rendered.contains("no active streams"), "{rendered}");
    }
}
