//! Tunnels & active streams inspector (right-centre panel).

use std::time::{Duration, Instant};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use super::theme as t;
use crate::app::{App, Focus};
use crate::network::{fmt_bytes, LinkKind, PeerStatus, StreamStatus};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Tunnels;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(if active {
            t::panel_border_active_style()
        } else {
            t::panel_border_style()
        })
        .title(t::panel_title("⇄", "Tunnels & Streams"));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(3)])
        .split(inner);

    draw_tunnels(f, app, chunks[0]);
    draw_streams(f, app, chunks[1]);
}

fn draw_tunnels(f: &mut Frame, app: &App, area: Rect) {
    let snap = &app.snapshot;
    // We don't carry tunnels in the snapshot; reconstruct a pseudo-list from
    // peers so the panel is never empty in the demo. Each connected peer with
    // traffic is shown as a tunnel row.
    let rows: Vec<Row> = snap
        .peers
        .values()
        .map(|p| {
            Row::new(vec![
                Line::from(Span::styled(format!("{}", p.id), t::header_value_style())),
                Line::from(Span::styled(p.label.clone(), t::header_value_style())),
                Line::from(Span::styled(
                    format!("{:?}", p.status),
                    super::status_style(p.status),
                )),
                Line::from(Span::styled(
                    format!("{:?}", p.link),
                    super::link_style(p.link),
                )),
            ])
        })
        .collect();

    let header = Row::new(vec![
        Line::from(Span::styled("ID", t::header_label_style())),
        Line::from(Span::styled("PEER", t::header_label_style())),
        Line::from(Span::styled("STATUS", t::header_label_style())),
        Line::from(Span::styled("LINK", t::header_label_style())),
    ])
    .height(1)
    .bottom_margin(0);

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .style(Style::default().bg(t::palette::BG).fg(t::palette::FG))
    .highlight_style(
        Style::default()
            .bg(t::palette::BG_ALT)
            .add_modifier(ratatui::style::Modifier::BOLD),
    );
    f.render_widget(table, area);
}

fn draw_streams(f: &mut Frame, app: &App, area: Rect) {
    let snap = &app.snapshot;
    // Streams live in the mesh, not the snapshot. We approximate from per-peer
    // byte counters so the inspector reflects live activity.
    let now = Instant::now();
    let mut rows: Vec<Row> = Vec::new();
    for p in snap.peers.values() {
        if p.bytes_sent + p.bytes_recv == 0 {
            continue;
        }
        let status = if p.status == PeerStatus::Connected {
            StreamStatus::Transferring
        } else {
            StreamStatus::Closed
        };
        let dur = p
            .connected_at
            .map(|c| now.duration_since(c))
            .unwrap_or_else(|| Duration::ZERO);
        rows.push(Row::new(vec![
            Line::from(Span::styled(
                format!("{:#x}", p.id),
                t::header_value_style(),
            )),
            Line::from(Span::styled(p.label.clone(), t::header_value_style())),
            Line::from(Span::styled(format!("{:?}", status), stream_style(status))),
            Line::from(Span::styled(fmt_dur(dur), t::dim_style())),
            Line::from(Span::styled(
                format!("↑{} ↓{}", fmt_bytes(p.bytes_sent), fmt_bytes(p.bytes_recv)),
                t::header_value_style(),
            )),
        ]));
    }
    if rows.is_empty() {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "no active streams — press n to connect a peer",
            t::dim_style(),
        )));
        f.render_widget(placeholder, area);
        return;
    }

    let header = Row::new(vec![
        Line::from(Span::styled("STREAM", t::header_label_style())),
        Line::from(Span::styled("PEER", t::header_label_style())),
        Line::from(Span::styled("STATUS", t::header_label_style())),
        Line::from(Span::styled("UPTIME", t::header_label_style())),
        Line::from(Span::styled("BYTES", t::header_label_style())),
    ]);

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Min(8),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Min(14),
        ],
    )
    .header(header)
    .style(Style::default().bg(t::palette::BG).fg(t::palette::FG))
    .highlight_style(Style::default().bg(t::palette::BG_ALT));
    f.render_widget(table, area);
}

fn stream_style(s: StreamStatus) -> Style {
    match s {
        StreamStatus::Established => t::status_warn_style(),
        StreamStatus::Transferring => t::status_ok_style(),
        StreamStatus::Closed => t::dim_style(),
    }
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    }
}

// Silence unused import warnings for types re-exported via super.
#[allow(dead_code)]
fn _unused(_l: LinkKind) {}
