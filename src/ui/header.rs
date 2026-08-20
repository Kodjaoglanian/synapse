//! Header: ASCII banner + global network stats.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::network::{fmt_bytes, fmt_rate};
use super::theme as t;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t::panel_border_style())
        .title(t::panel_title("◆", "synapse"));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin { vertical: 1, horizontal: 2 });
    // Left: ASCII art. Right: stats grid.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(46), Constraint::Min(0)])
        .split(inner);

    // Banner.
    let banner = t::ascii_banner();
    let banner_lines: Vec<Line> = banner
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), t::title_style())))
        .collect();
    f.render_widget(
        Paragraph::new(banner_lines).alignment(Alignment::Left),
        cols[0],
    );

    // Stats grid: two rows of stat cells.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2), Constraint::Length(1)])
        .split(cols[1]);

    let snap = &app.snapshot;
    let up = fmt_rate(snap.up_rate_bps);
    let down = fmt_rate(snap.down_rate_bps);
    let rtt = if snap.avg_rtt_ms > 0 { format!("{} ms", snap.avg_rtt_ms) } else { "—".into() };
    let pkts = snap.total_packets;
    let lost = snap.total_lost;
    let ip = app.public_ip.clone().unwrap_or_else(|| "discovering…".into());
    let nat = app.nat_type.clone();
    let mode = app.mode.clone();
    let peers = snap.peers.len();

    let row1 = Line::from(stat_spans(&[
        ("PUBLIC IP", &ip),
        ("NAT", &nat),
        ("MODE", &mode),
    ]));
    let row2 = Line::from(stat_spans(&[
        ("↑ UP", &up),
        ("↓ DOWN", &down),
        ("RTT", &rtt),
    ]));
    let row3 = Line::from(stat_spans(&[
        ("PEERS", &format!("{peers}")),
        ("PKTS", &format!("{pkts}")),
        ("LOST", &format!("{lost}")),
        ("TOTAL", &format!("↑{} ↓{}", fmt_bytes(snap.total_up), fmt_bytes(snap.total_down))),
    ]));

    f.render_widget(row1, rows[0]);
    f.render_widget(row2, rows[1]);
    f.render_widget(row3, rows[2]);
}

/// Build a flat list of styled spans: `LABEL value  LABEL value ...`.
fn stat_spans(pairs: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for (i, (label, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(Span::raw("  "));
        }
        out.push(Span::styled(format!("{label} "), t::header_label_style()));
        out.push(Span::styled(
            value.to_string(),
            Style::default().fg(t::palette::FG).add_modifier(Modifier::BOLD),
        ));
    }
    out
}
