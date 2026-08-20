//! Theme: Tokyo Night / cyberpunk-minimalist palette, Unicode symbols and border styles.
//!
//! All visual constants live here so the rest of the UI can reference a single
//! source of truth for colors, glyphs and reusable `Style`s.

use ratatui::style::{Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};

/// Full palette inspired by the Tokyo Night VSCode theme.
pub mod palette {
    use ratatui::style::Color;

    pub const BG: Color = Color::Rgb(26, 27, 38); // #1a1b26
    pub const BG_ALT: Color = Color::Rgb(36, 40, 59); // #24283b
    pub const FG: Color = Color::Rgb(169, 177, 214); // #a9b1d6
    pub const FG_DIM: Color = Color::Rgb(88, 91, 112); // #585b70
    pub const BLUE: Color = Color::Rgb(122, 162, 247); // #7aa2f7
    pub const CYAN: Color = Color::Rgb(187, 154, 247); // #bb9af7 (tokyo "purple")
    pub const MAGENTA: Color = Color::Rgb(197, 122, 219); // #c57adb
    pub const GREEN: Color = Color::Rgb(158, 206, 106); // #9ece6a
    pub const YELLOW: Color = Color::Rgb(224, 175, 104); // #e0af68
    pub const RED: Color = Color::Rgb(247, 118, 118); // #f7768e
    pub const ORANGE: Color = Color::Rgb(255, 158, 100); // #ff9e64
    pub const TEAL: Color = Color::Rgb(95, 214, 193); // #5fd6c1
}

/// Reusable styles composed from the palette.
pub fn title_style() -> Style {
    Style::default()
        .fg(palette::BLUE)
        .add_modifier(Modifier::BOLD)
}

pub fn header_label_style() -> Style {
    Style::default().fg(palette::FG_DIM)
}

pub fn header_value_style() -> Style {
    Style::default().fg(palette::FG)
}

pub fn panel_title_style() -> Style {
    Style::default()
        .fg(palette::CYAN)
        .add_modifier(Modifier::BOLD)
}

pub fn panel_border_style() -> Style {
    Style::default().fg(palette::BG_ALT)
}

pub fn panel_border_active_style() -> Style {
    Style::default().fg(palette::BLUE)
}

pub fn status_ok_style() -> Style {
    Style::default().fg(palette::GREEN)
}

pub fn status_warn_style() -> Style {
    Style::default().fg(palette::YELLOW)
}

pub fn status_err_style() -> Style {
    Style::default().fg(palette::RED)
}

pub fn status_relay_style() -> Style {
    Style::default().fg(palette::MAGENTA)
}

pub fn log_info_style() -> Style {
    Style::default().fg(palette::TEAL)
}

pub fn log_warn_style() -> Style {
    Style::default().fg(palette::YELLOW)
}

pub fn log_err_style() -> Style {
    Style::default().fg(palette::RED)
}

pub fn log_packet_style() -> Style {
    Style::default().fg(palette::ORANGE)
}

pub fn log_handshake_style() -> Style {
    Style::default().fg(palette::GREEN)
}

pub fn dim_style() -> Style {
    Style::default().fg(palette::FG_DIM)
}

/// Border set using rounded corners + thin separators for a modern look.
pub fn rounded_border_set() -> border::Set {
    border::ROUNDED
}

/// Glyphs used across the graph and stats.
pub mod glyphs {
    pub const NODE_LOCAL: &str = "◉";
    pub const NODE_PEER: &str = "●";
    pub const NODE_RELAY: &str = "◇";
    pub const PULSE: &str = "·";
    pub const ARROW_R: &str = "→";
    pub const ARROW_L: &str = "←";
    pub const DOT: &str = "•";
    pub const SPARK_FULL: &str = "█";
    pub const SPARK_TOP: &str = "▀";
    pub const SPARK_BOT: &str = "▄";
    pub const SPARK_EMPTY: &str = " ";
    pub const BRAILLE_DOTS: &str = "⠁⠂⠄⠈⠐⠠⡀⢀";
}

/// Build a styled title line for a panel, with an optional accent prefix.
pub fn panel_title(prefix: &str, name: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(prefix.to_string(), panel_title_style()),
        Span::raw(" "),
        Span::styled(name.to_string(), Style::default().fg(palette::FG)),
    ])
}

/// ASCII art for the header banner.
pub fn ascii_banner() -> &'static str {
    r"//   ) ) \\    / / /|    / / // | |     //   ) ) //   ) )  //   / /
((         \\  / / //|   / / //__| |    //___/ / ((        //____
  \\        \\/ / // |  / / / ___  |   / ____ /    \\     / ____
    ) )      / / //  | / / //    | |  //             ) ) //
((___ / /      / / //   |/ / //     | | //       ((___ / / //____/ /"
}
