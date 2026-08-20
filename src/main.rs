//! synapse — decentralized P2P tunneling platform with a high-end TUI.
//!
//! CLI entrypoint: parses flags with Clap, sets up the terminal, installs a
//! panic hook that always restores the terminal, spawns the network stack and
//! runs the unified event loop.

// v0.1.0 reserves several fields, enum variants, and helper functions for
// upcoming features (relay mode, packet-drop tracking, force-publish metrics,
// etc.). Suppress dead-code warnings until those land.
#![allow(dead_code)]

mod app;
mod events;
mod i18n;
mod network;
mod ui;
mod update;

use std::io::{stdout, IsTerminal};
use std::process;

use anyhow::Result;
use clap::{Parser, ValueHint};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use network::{NetworkConfig, SeedPeer, TunnelSeed};

/// synapse — P2P tunnels in your terminal.
#[derive(Parser, Debug)]
#[command(name = "synapse", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Comma-separated list of STUN server URLs (e.g. stun:stun.l.google.com:19302).
    #[arg(long, value_delimiter = ',', value_hint = ValueHint::Url, env = "SYNAPSE_STUN")]
    stun: Vec<String>,

    /// Comma-separated list of TURN server URLs.
    #[arg(long, value_delimiter = ',', value_hint = ValueHint::Url, env = "SYNAPSE_TURN")]
    turn: Vec<String>,

    /// Seed peer to dial on startup, as `label:token`. Repeatable.
    #[arg(long = "peer", value_name = "LABEL:TOKEN")]
    peers: Vec<String>,

    /// Local tunnel to open on startup, as `local_port:peer_label:remote_host:remote_port:label`. Repeatable.
    #[arg(long = "tunnel", value_name = "PORT:PEER:HOST:PORT:LABEL")]
    tunnels: Vec<String>,

    /// Base URL of an HTTP signaling server (e.g. http://localhost:8080). When
    /// set, peers can dial each other by room name via `--dial-room` /
    /// `--answer-room` without manually exchanging SDP.
    #[arg(long, value_hint = ValueHint::Url, env = "SYNAPSE_SIGNALING")]
    signaling: Option<String>,

    /// On startup, dial a peer via the signaling server. Format: `label:room`.
    /// Requires --signaling. Repeatable.
    #[arg(long = "dial-room", value_name = "LABEL:ROOM")]
    dial_rooms: Vec<String>,

    /// On startup, answer a peer via the signaling server. Format: `label:room`.
    /// Requires --signaling. Repeatable.
    #[arg(long = "answer-room", value_name = "LABEL:ROOM")]
    answer_rooms: Vec<String>,

    /// Run without a TTY (headless): just start the network stack and log to stdout.
    #[arg(long)]
    headless: bool,

    /// UI language: `en` (English, default) or `pt` (Português).
    #[arg(long, value_enum, env = "SYNAPSE_LANG", default_value = "en")]
    lang: i18n::Lang,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Download and install the latest release from GitHub.
    Update,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize i18n with the selected language.
    i18n::init(cli.lang);

    // Handle subcommands (e.g. `synapse update`).
    if let Some(Command::Update) = cli.command {
        return update::run();
    }

    let stun = if cli.stun.is_empty() {
        default_stun_servers()
    } else {
        cli.stun
    };

    let seeds = cli
        .peers
        .iter()
        .filter_map(|s| {
            let (label, token) = s.split_once(':')?;
            Some(SeedPeer {
                label: label.to_string(),
                token: token.to_string(),
            })
        })
        .collect();

    let tunnels = cli
        .tunnels
        .iter()
        .filter_map(|s| parse_tunnel_seed(s))
        .collect();

    let signaling = cli
        .signaling
        .as_ref()
        .map(|u| network::Signaling::new(u.trim_end_matches('/').to_string()))
        .transpose()?;

    let dial_rooms: Vec<(String, String)> = cli
        .dial_rooms
        .iter()
        .filter_map(|s| {
            let (label, room) = s.split_once(':')?;
            Some((label.to_string(), room.to_string()))
        })
        .collect();
    let answer_rooms: Vec<(String, String)> = cli
        .answer_rooms
        .iter()
        .filter_map(|s| {
            let (label, room) = s.split_once(':')?;
            Some((label.to_string(), room.to_string()))
        })
        .collect();

    let config = NetworkConfig {
        stun_servers: stun,
        turn_servers: cli.turn,
        seeds,
        tunnels,
        signaling,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async move {
        let net = network::spawn(config).await?;

        // Fire off signaling-driven dials/answers requested on the CLI.
        for (label, room) in &dial_rooms {
            let _ = net.engine.cmd_tx.send(network::EngineCmd::DialSignaling {
                label: label.clone(),
                room: room.clone(),
            });
        }
        for (label, room) in &answer_rooms {
            let _ = net.engine.cmd_tx.send(network::EngineCmd::AnswerSignaling {
                label: label.clone(),
                room: room.clone(),
            });
        }

        if cli.headless || !stdout().is_terminal() {
            // Headless: just keep the stack alive and print logs.
            run_headless(net).await
        } else {
            // TUI: set up the terminal, install panic hook, run the loop.
            run_tui(net).await
        }
    });

    // If anything went wrong, make sure the terminal is restored before exiting.
    restore_terminal();
    if let Err(e) = result {
        eprintln!("synapse: {e:#}");
        process::exit(1);
    }
    Ok(())
}

async fn run_tui(net: network::Network) -> Result<()> {
    setup_terminal()?;
    install_panic_hook();

    let app = app::App::new(net);
    // The event loop owns the terminal lifecycle; it restores on exit.
    let res = events::run(app).await;
    restore_terminal();
    res
}

async fn run_headless(mut net: network::Network) -> Result<()> {
    // Print a startup banner.
    println!("{}", i18n::t().headless_banner);
    loop {
        match net.events_rx.recv().await {
            Ok(ev) => print_event(&ev),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    Ok(())
}

fn print_event(ev: &network::NetEvent) {
    use network::{LogLevel, NetEvent};
    match ev {
        NetEvent::Log(LogLevel::Info, m) => println!("[INFO ] {m}"),
        NetEvent::Log(LogLevel::Warn, m) => println!("[WARN ] {m}"),
        NetEvent::Log(LogLevel::Error, m) => println!("[ERR  ] {m}"),
        NetEvent::Log(LogLevel::PacketDrop, m) => println!("[DROP ] {m}"),
        NetEvent::Log(LogLevel::Handshake, m) => println!("[HS   ] {m}"),
        NetEvent::PeerAdded(p) => println!("[PEER ] added {} ({})", p.label, p.id),
        NetEvent::PeerConnected(id) => println!("[PEER ] connected {id}"),
        NetEvent::PeerFailed(id, why) => println!("[PEER ] failed {id}: {why}"),
        NetEvent::PeerRemoved(id) => println!("[PEER ] removed {id}"),
        NetEvent::TunnelAdded(t) => println!("[TUN  ] added '{}' on {}", t.label, t.local_addr),
        NetEvent::TunnelRemoved(id) => println!("[TUN  ] removed {id}"),
        NetEvent::StreamOpened(s) => println!("[STRM ] open {} on tunnel {}", s.id, s.tunnel_id),
        NetEvent::StreamClosed(id) => println!("[STRM ] closed {id}"),
        NetEvent::SdpReady { peer, sdp } => {
            println!("[SDP  ] ready for {peer} ({} bytes)", sdp.len())
        }
        _ => {}
    }
}

fn parse_tunnel_seed(s: &str) -> Option<TunnelSeed> {
    let parts: Vec<&str> = s.splitn(5, ':').collect();
    if parts.len() != 5 {
        return None;
    }
    Some(TunnelSeed {
        local_port: parts[0].parse().ok()?,
        peer_label: parts[1].to_string(),
        remote_host: parts[2].to_string(),
        remote_port: parts[3].parse().ok()?,
        label: parts[4].to_string(),
    })
}

fn default_stun_servers() -> Vec<String> {
    vec![
        "stun:stun.l.google.com:19302".into(),
        "stun:stun1.l.google.com:19302".into(),
        "stun:stun2.l.google.com:19302".into(),
    ]
}

/// Enter raw mode + alternate screen.
fn setup_terminal() -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    // Hide the cursor for a cleaner TUI.
    execute!(out, crossterm::cursor::Hide)?;
    Ok(())
}

/// Restore the terminal to its default state. Safe to call multiple times.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = stdout();
    let _ = execute!(out, crossterm::cursor::Show);
    let _ = execute!(out, LeaveAlternateScreen);
}

/// Install a panic hook that restores the terminal before printing the panic.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

// Suppress unused import warnings for symbols only used in type signatures.
#[allow(dead_code)]
fn _unused_type_anchor(_t: &Terminal<CrosstermBackend<std::io::Stdout>>) {}
