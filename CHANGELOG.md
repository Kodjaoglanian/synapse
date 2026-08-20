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

## [0.1.2] - 2025-01-20

### Added

- **NAT discovery (real)**: STUN Binding Request (RFC 5389) over UDP with
  XOR-MAPPED-ADDRESS parsing. Falls back to HTTP IP lookup APIs if STUN fails.
  The public IP and NAT type now appear in the header within seconds.
- **i18n support**: `--lang` CLI flag (or `SYNAPSE_LANG` env var) to select
  UI language. Supports `en` (English) and `pt` (Português).

### Changed

- **Release pipeline**: removed fat LTO override (use `thin` from Cargo.toml)
  for faster Windows builds. Added `--locked` for reproducible builds.

## [0.1.1] - 2025-01-20

### Added

- **PowerShell install script** (`install.ps1`): one-line install for Windows
  with automatic PATH configuration.
- **SHA256 checksums**: per-target checksums and a combined
  `synapse-checksums.txt` file published with each release.
- **Post-release verification job**: CI downloads and verifies binaries after
  publishing.
- **Lockfile check**: CI verifies `Cargo.lock` is committed and up to date.

### Changed

- **ASCII banner**: replaced generic figlet banner with the synapse logo.
- **CI pipeline**: split into parallel lint/build/smoke-test jobs with
  fail-fast lint gate, concurrency control, and per-job timeouts.
- **Release pipeline**: fat LTO + single codegen unit for optimized binaries,
  debug symbol stripping on Unix, cache keyed by `Cargo.lock` hash.
- **Release notes**: auto-generated with download table and checksum
  verification instructions.
- **macOS x86_64 builds**: cross-compiled on `macos-14` (Apple Silicon) instead
  of deprecated `macos-13` runner.

### Fixed

- Resolved all clippy warnings (large_enum_variant, collapsible_if,
  manual_saturating_arithmetic, unnecessary_cast, identity_op, for_kv_map).
- `EngineCmd::RemoteSdp` now boxes `RTCSessionDescription` (592 → 16 bytes).

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
