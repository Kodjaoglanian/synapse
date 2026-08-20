//! Interactive mesh graph drawn on a Ratatui `Canvas`.
//!
//! The local node sits at the centre; peers are arranged on a circle. Edges are
//! coloured by link quality. When a peer is actively transferring data, a pulse
//! travels along its edge, animated by the app tick counter.

use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::Marker;
use ratatui::widgets::{
    canvas::{Canvas, Line as CanvasLine, Points},
    Block, Borders, Paragraph,
};
use ratatui::Frame;

use super::theme as t;
use crate::app::{App, Focus};
use crate::network::{LinkKind, PeerId, PeerStatus};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Graph;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(if active {
            t::panel_border_active_style()
        } else {
            t::panel_border_style()
        })
        .title(t::panel_title("⬡", "Mesh Graph"));
    f.render_widget(block, area);

    let inner = area.inner(&ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });
    // Reserve a one-line legend at the bottom.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let snap = &app.snapshot;
    let peer_ids: Vec<PeerId> = snap.peers.keys().copied().collect();
    let n = peer_ids.len();

    // Centre the local node; place peers on a circle.
    let cx = 50.0f64;
    let cy = 50.0f64;
    let radius = 38.0f64;

    let mut positions: HashMap<PeerId, (f64, f64)> = HashMap::new();
    for (i, pid) in peer_ids.iter().enumerate() {
        let angle =
            (i as f64) * (std::f64::consts::TAU / n.max(1) as f64) - std::f64::consts::FRAC_PI_2;
        positions.insert(*pid, (cx + radius * angle.cos(), cy + radius * angle.sin()));
    }

    let tick = app.tick as f64;

    let canvas = Canvas::default()
        .background_color(t::palette::BG)
        .marker(Marker::Braille)
        .x_bounds([0.0, 100.0])
        .y_bounds([0.0, 100.0])
        .paint(move |ctx| {
            // Draw edges first (so nodes sit on top).
            for (pid, (px, py)) in &positions {
                let peer = snap.peers.get(pid);
                let link = peer.map(|p| p.link).unwrap_or_default();
                let active = peer
                    .map(|p| p.status == PeerStatus::Connected && (p.bytes_sent + p.bytes_recv) > 0)
                    .unwrap_or(false);
                let color = link_color(link);
                ctx.draw(&CanvasLine {
                    x1: cx,
                    y1: cy,
                    x2: *px,
                    y2: *py,
                    color,
                });

                // Pulse animation along the edge when active.
                if active {
                    let pulses = 3;
                    for p in 0..pulses {
                        let phase = ((tick * 0.03) + (p as f64 / pulses as f64)) % 1.0;
                        let x = cx + (px - cx) * phase;
                        let y = cy + (py - cy) * phase;
                        ctx.draw(&Points {
                            coords: &[(x, y)],
                            color,
                        });
                        // trailing dot for a comet effect.
                        let tx = cx + (px - cx) * (phase - 0.04).max(0.0);
                        let ty = cy + (py - cy) * (phase - 0.04).max(0.0);
                        ctx.draw(&Points {
                            coords: &[(tx, ty)],
                            color: t::palette::FG_DIM,
                        });
                    }
                }
            }

            // Draw peer nodes.
            for (pid, (px, py)) in &positions {
                let peer = snap.peers.get(pid);
                let color = peer
                    .map(|p| status_color(p.status))
                    .unwrap_or(t::palette::FG_DIM);
                ctx.draw(&Points {
                    coords: &[(*px, *py)],
                    color,
                });
                // halo for connected peers.
                if let Some(p) = peer {
                    if p.status == PeerStatus::Connected {
                        let r = 2.0 + (tick * 0.08).sin().abs() * 1.5;
                        ctx.draw(&Points {
                            coords: &[
                                (*px + r, *py),
                                (*px - r, *py),
                                (*px, *py + r),
                                (*px, *py - r),
                            ],
                            color: t::palette::BG_ALT,
                        });
                    }
                }
            }

            // Draw the local centre node (larger, pulsing).
            let pulse = 1.0 + (tick * 0.1).sin().abs() * 1.0;
            ctx.draw(&Points {
                coords: &[(cx, cy)],
                color: t::palette::BLUE,
            });
            ctx.draw(&Points {
                coords: &[
                    (cx + pulse, cy),
                    (cx - pulse, cy),
                    (cx, cy + pulse),
                    (cx, cy - pulse),
                    (cx + pulse * 0.6, cy + pulse * 0.6),
                    (cx - pulse * 0.6, cy - pulse * 0.6),
                    (cx + pulse * 0.6, cy - pulse * 0.6),
                    (cx - pulse * 0.6, cy + pulse * 0.6),
                ],
                color: t::palette::CYAN,
            });
        });

    f.render_widget(canvas, chunks[0]);

    // Legend.
    let legend = Paragraph::new(ratatui::text::Line::from(vec![
        ratatui::text::Span::styled("● ", t::status_ok_style()),
        ratatui::text::Span::styled("direct<40ms  ", t::dim_style()),
        ratatui::text::Span::styled("● ", t::status_warn_style()),
        ratatui::text::Span::styled("direct<120ms  ", t::dim_style()),
        ratatui::text::Span::styled("● ", t::status_relay_style()),
        ratatui::text::Span::styled("relay/TURN  ", t::dim_style()),
        ratatui::text::Span::styled("◉ ", Style::default().fg(t::palette::BLUE)),
        ratatui::text::Span::styled("local", t::dim_style()),
    ]));
    f.render_widget(legend, chunks[1]);
}

fn link_color(l: LinkKind) -> ratatui::style::Color {
    match l {
        LinkKind::DirectFast => t::palette::GREEN,
        LinkKind::DirectModerate => t::palette::YELLOW,
        LinkKind::DirectSlow => t::palette::ORANGE,
        LinkKind::Relay => t::palette::MAGENTA,
    }
}

fn status_color(s: PeerStatus) -> ratatui::style::Color {
    match s {
        PeerStatus::Connected => t::palette::GREEN,
        PeerStatus::Connecting | PeerStatus::Gathering => t::palette::YELLOW,
        PeerStatus::Failed => t::palette::RED,
        PeerStatus::Closed | PeerStatus::Idle => t::palette::FG_DIM,
    }
}
