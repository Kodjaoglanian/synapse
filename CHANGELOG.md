# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Relay mode (TURN-based fallback for restrictive NATs)
- Persistent peer/tunnel configuration file
- Encrypted control channel (Noise Protocol)
- Bandwidth throttling per tunnel
- IPv6 support

## [0.1.0] - 2025-01-20

### Added

- **Core P2P tunneling over WebRTC**: peer connections with ICE/STUN/TURN,
  data channels for control (ping/pong RTT) and tunnel traffic.
- **Terminal UI** (Ratatui + Crossterm):
  - ASCII art header with live global stats (public IP, NAT type, mode,
    throughput, RTT, peer count, packet count, loss, total bytes)
  - Canvas mesh graph with animated traffic pulses, link-quality-colored
    edges, and a pulsing local node
  - Tunnels & streams inspector table with status and byte counters
  - Throughput sparklines (60-second window) and color-coded event log
  - Modal overlays: help, peer detail, quick-connect (SDP paste), signaling
    room connect
  - Vim-style keybindings (j/k, Tab, Enter, n, s, d, ?, q)
- **HTTP signaling client**: exchange SDP offers/answers and trickle ICE
  candidates via a simple REST endpoint. Supports dial and answer roles
  with automatic ICE draining.
- **TCP bidirectional proxy**: local TcpListener pumps bytes to/from a
  peer's RTCDataChannel with DataChannelReader/Writer adapters.
- **Telemetry aggregator**: per-peer byte counts, RTT, and packet-loss
  published as immutable snapshots via `watch` channel every 100ms.
- **CLI** (Clap): `--stun`, `--turn`, `--peer`, `--tunnel`, `--signaling`,
  `--dial-room`, `--answer-room`, `--headless` flags with env var support.
- **Panic hook**: terminal is always restored before a panic message prints.
- **Reference signaling server** (`signaling_server.py`): minimal Python
  HTTP server for local testing.
- **One-line install script** (`install.sh`): detects OS/arch and downloads
  the correct prebuilt binary from GitHub Releases.
- **CI/CD**: GitHub Actions workflows for multi-target builds (Linux
  x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64), format check,
  clippy, smoke test, and automated release packaging on version tags.
- **Tokyo Night color palette** across all UI panels.

### Technical details

- Built with Rust 2021 edition, webrtc-rs 0.11, Ratatui 0.26, Crossterm
  0.27, Tokio, Clap 4, Reqwest (rustls).
- Network and UI layers fully decoupled via async channels.
- No `unwrap()` in critical paths; terminal always restored on exit.
