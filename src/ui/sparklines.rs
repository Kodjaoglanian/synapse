//! Bottom panel: throughput sparklines (up/down) + semantic event log.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Sparkline};
use ratatui::Frame;

use super::theme as t;
use crate::app::{App, Focus};
use crate::network::LogLevel;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t::panel_border_style())
        .title(t::panel_title("≋", "Throughput & Events"));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(inner);

    draw_sparklines(f, app, cols[0]);
    draw_log(f, app, cols[1]);
}

fn draw_sparklines(f: &mut Frame, app: &App, area: Rect) {
    let snap = &app.snapshot;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // up label
            Constraint::Length(3), // up spark
            Constraint::Length(1), // down label
            Constraint::Length(3), // down spark
            Constraint::Min(0),
        ])
        .split(area);

    let up_max = snap.up_history.iter().copied().max().unwrap_or(1).max(1);
    let down_max = snap.down_history.iter().copied().max().unwrap_or(1).max(1);

    f.render_widget(
        Line::from(vec![
            Span::styled("↑ UP  ", t::status_ok_style()),
            Span::styled(
                crate::network::fmt_rate(snap.up_rate_bps),
                t::header_value_style(),
            ),
        ]),
        chunks[0],
    );
    f.render_widget(
        Sparkline::default()
            .block(Block::default())
            .data(&snap.up_history)
            .max(up_max)
            .direction(ratatui::widgets::RenderDirection::LeftToRight)
            .style(Style::default().fg(t::palette::GREEN)),
        chunks[1],
    );

    f.render_widget(
        Line::from(vec![
            Span::styled("↓ DOWN ", t::status_warn_style()),
            Span::styled(
                crate::network::fmt_rate(snap.down_rate_bps),
                t::header_value_style(),
            ),
        ]),
        chunks[2],
    );
    f.render_widget(
        Sparkline::default()
            .block(Block::default())
            .data(&snap.down_history)
            .max(down_max)
            .direction(ratatui::widgets::RenderDirection::LeftToRight)
            .style(Style::default().fg(t::palette::YELLOW)),
        chunks[3],
    );

    // Tiny summary line.
    let summary = format!(
        "window 60s · peak ↑{} ↓{}",
        crate::network::fmt_rate(up_max),
        crate::network::fmt_rate(down_max),
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(summary, t::dim_style()))),
        chunks[4],
    );
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Log;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(if active {
            t::panel_border_active_style()
        } else {
            t::panel_border_style()
        })
        .title(t::panel_title("≡", "Event Stream"));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });
    // Show the most recent entries (bottom of buffer = newest).
    let items: Vec<ListItem> = app
        .log
        .iter()
        .rev()
        .take(inner.height as usize)
        .map(|e| {
            let style = log_style(e.level);
            let tag = log_tag(e.level);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{tag} "), style.add_modifier(Modifier::BOLD)),
                Span::styled(e.msg.clone(), style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .style(Style::default().bg(t::palette::BG).fg(t::palette::FG))
        .highlight_style(Style::default().bg(t::palette::BG_ALT));
    f.render_widget(list, inner);
}

fn log_style(l: LogLevel) -> Style {
    match l {
        LogLevel::Info => t::log_info_style(),
        LogLevel::Warn => t::log_warn_style(),
        LogLevel::Error => t::log_err_style(),
        LogLevel::PacketDrop => t::log_packet_style(),
        LogLevel::Handshake => t::log_handshake_style(),
    }
}

fn log_tag(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERR ",
        LogLevel::PacketDrop => "DROP",
        LogLevel::Handshake => "HS  ",
    }
}
