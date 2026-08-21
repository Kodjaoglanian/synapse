# synapse

<p align="center">
  <strong>Decentralized P2P tunneling platform with a high-end terminal UI</strong>
</p>

<p align="center">
  <a href="https://github.com/Kodjaoglanian/synapse/actions/workflows/ci.yml"><img src="https://github.com/Kodjaoglanian/synapse/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/Kodjaoglanian/synapse/releases"><img src="https://img.shields.io/github/v/release/Kodjaoglanian/synapse?display_name=tag&sort=semver" alt="Release"></a>
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue" alt="Platform">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
  <img src="https://img.shields.io/badge/Rust-1.70+-orange" alt="Rust">
</p>

---

## What is synapse?

synapse is a decentralized peer-to-peer tunneling tool that exposes local
services to remote peers over WebRTC data channels — no central server, no
port forwarding, no configuration files. It ships with a beautiful
terminal UI built with [Ratatui](https://ratatui.rs) that visualizes the
mesh network in real time: animated topology graph, throughput sparklines,
stream inspector, and a color-coded event log.

```
      ::::::::  :::   ::: ::::    :::     :::     :::::::::   ::::::::  ::::::::::
    :+:    :+: :+:   :+: :+:+:   :+:   :+: :+:   :+:    :+: :+:    :+: :+:
   +:+         +:+ +:+  :+:+:+  +:+  +:+   +:+  +:+    +:+ +:+        +:+
  +#++:++#++   +#++:   +#+ +:+ +#+ +#++:++#++: +#++:++#+  +#++:++#++ +#++:++#
        +#+    +#+    +#+  +#+#+# +#+     +#+ +#+               +#+ +#+
#+#    #+#    #+#    #+#   #+#+# #+#     #+# #+#        #+#    #+# #+#
########     ###    ###    #### ###     ### ###         ########  ##########
```

## Quick install

### One-line install (Linux & macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.sh | sh
```

Or pin a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.sh | sh -s -- --version v0.2.6
```

### One-line install (Windows — PowerShell)

```powershell
irm https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.ps1 | iex
```

Or pin a specific version:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.ps1))) -Version "v0.2.6"
```

The script downloads the correct binary, installs it to `%LOCALAPPDATA%\Programs\synapse`,
and adds it to your user PATH automatically.

### Manual download

Prebuilt binaries for all platforms are available on the
[releases page](https://github.com/Kodjaoglanian/synapse/releases):

| Platform | Asset |
|----------|-------|
| Linux x86_64 | `synapse-v0.2.6-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `synapse-v0.2.6-aarch64-unknown-linux-gnu.tar.gz` |
| macOS x86_64 (Intel) | `synapse-v0.2.6-x86_64-apple-darwin.tar.gz` |
| macOS ARM64 (Apple Silicon) | `synapse-v0.2.6-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `synapse-v0.2.6-x86_64-pc-windows-msvc.zip` |

Extract the archive and add the `synapse` (or `synapse.exe`) binary to your PATH.

### Build from source

```bash
git clone https://github.com/Kodjaoglanian/synapse.git
cd synapse
cargo build --release
# Binary: ./target/release/synapse
```

## Usage

### Basic TUI

```bash
synapse
```

### Connect two peers via signaling

1. **Run a signaling server** (use the included reference server):

   ```bash
   python3 signaling_server.py 8080
   ```

2. **Machine A (offerer):**

   ```bash
   synapse --signaling http://your-server:8080 --dial-room alice:room1
   ```

3. **Machine B (answerer):**

   ```bash
   synapse --signaling http://your-server:8080 --answer-room bob:room1
   ```

4. ICE connects — Alice remains the local identity on machine A and sees Bob
   as the remote peer; Bob remains local on machine B and sees Alice remotely.
   Each graph and participant list shows both the named local node and all
   remote nodes. The edge indicates link quality (green = fast direct, yellow =
   moderate, purple = relay).

### Open a tunnel

Once two peers are connected, expose a local port that tunnels to the remote
peer's endpoint:

```bash
# On Alice, forward local port 3000 to Bob's 127.0.0.1:8080
synapse --tunnel 3000:bob:127.0.0.1:8080:web
```

Or open tunnels at startup alongside a dial:

```bash
synapse --signaling http://server:8080 \
  --dial-room alice:room1 \
  --tunnel 3000:bob:127.0.0.1:8080:web \
  --tunnel 2222:bob:127.0.0.1:22:ssh
```

### Test a tunnel locally

Use five terminals on the same machine:

```bash
# 1. Signaling server
python3 signaling_server.py 8083

# 2. HTTP service that represents Bob's remote endpoint
python3 -m http.server 9090 --bind 127.0.0.1

# 3. Bob
synapse --signaling http://127.0.0.1:8083 --answer-room bob:tunnel-test

# 4. Alice: localhost:3000 -> Bob's localhost:9090
synapse --signaling http://127.0.0.1:8083 --dial-room alice:tunnel-test \
  --tunnel 3000:bob:127.0.0.1:9090:web

# 5. Send traffic through the tunnel
curl http://127.0.0.1:3000/
```

### Headless mode (no TUI)

```bash
synapse --headless --signaling http://server:8080 --dial-room alice:room1
```

## CLI flags

| Flag | Description | Default |
|------|-------------|---------|
| `--stun <URLS>` | Comma-separated STUN server URLs | Google STUN |
| `--turn <URLS>` | Comma-separated TURN server URLs | (none) |
| `--peer LABEL:TOKEN` | Seed peer to dial on startup (repeatable) | |
| `--tunnel PORT:PEER:HOST:PORT:LABEL` | Local tunnel to open (repeatable) | |
| `--signaling <URL>` | HTTP signaling server URL | (env `SYNAPSE_SIGNALING`) |
| `--dial-room LOCAL_NAME:ROOM` | Dial via signaling with this local identity (repeatable) | |
| `--answer-room LOCAL_NAME:ROOM` | Answer via signaling with this local identity (repeatable) | |
| `--headless` | Run without TUI, log to stdout | |
| `--lang <en\|pt>` | UI language: English or Português | `en` (env `SYNAPSE_LANG`) |

### Self-update

```bash
synapse update
```

Checks GitHub for a newer release, downloads the matching binary for your
platform, and replaces the running executable in place.

## TUI keybindings

| Key | Action |
|-----|--------|
| `Tab` / `Shift-Tab` | Switch panel |
| `j` / `k` / ↑ / ↓ | Navigate list |
| `Enter` | Inspect node / submit modal |
| `n` | Quick-connect modal (paste raw SDP) |
| `s` | Connect via signaling server (by room) |
| `d` | Disconnect selected peer |
| `?` | Help |
| `q` / `Ctrl-C` | Quit |

## Architecture

```
src/
├── main.rs              CLI entrypoint (clap), terminal setup, panic hook
├── app.rs               Global state, focus, modal forms, input → actions
├── events.rs            Unified event loop (crossterm + tick + network)
├── network/
│   ├── mod.rs           Shared state (SharedMesh), NetEvent bus, NetworkConfig
│   ├── engine.rs        WebRTC peer connection, ICE, DataChannel, signaling
│   ├── proxy.rs         TCP bidirectional proxy (TcpListener ↔ DataChannel)
│   ├── metrics.rs       Telemetry aggregator (watch-channel snapshots @ 100ms)
│   └── signaling.rs     HTTP signaling client (SDP + trickle ICE exchange)
└── ui/
    ├── mod.rs           Master layout, modal overlays, text field rendering
    ├── theme.rs         Tokyo Night palette, Unicode glyphs, reusable styles
    ├── header.rs        ASCII art banner + live global stats
    ├── graph.rs         Canvas mesh graph with animated traffic pulses
    ├── tunnels.rs       Tunnels & streams inspector table
    └── sparklines.rs    Throughput sparklines + color-coded event log
```

The network and UI layers are fully decoupled via `tokio::sync` channels
(`mpsc` for commands, `watch` for metrics snapshots, `broadcast` for events).
No `ratatui` types leak into the network module.

## Tech stack

- **WebRTC**: [webrtc-rs](https://github.com/webrtc-rs/webrtc) 0.11 (pure Rust)
- **Async runtime**: [Tokio](https://tokio.rs)
- **TUI**: [Ratatui](https://ratatui.rs) 0.26 + [Crossterm](https://github.com/crossterm-rs/crossterm) 0.27
- **CLI**: [Clap](https://docs.rs/clap) 4
- **HTTP client**: [Reqwest](https://docs.rs/reqwest) (rustls, no OpenSSL)

## Signaling protocol

synapse uses a simple REST-based signaling protocol. Any HTTP server that
stores a body under a key and returns it on GET works:

```
POST /offer/{room}       Store SDP offer
GET  /offer/{room}       Retrieve SDP offer (404 if not posted)
POST /answer/{room}      Store SDP answer
GET  /answer/{room}      Retrieve SDP answer (404 if not posted)
POST /ice/{room}/{side}  Append ICE candidate (side = a or b)
GET  /ice/{room}/{side}  Retrieve newline-delimited ICE candidates
```

A reference Python server is included (`signaling_server.py`).

## Building from source

```bash
# Prerequisites: Rust 1.70+ (rustup recommended)
cargo build --release
```

### Cross-compilation

```bash
# Add a target
rustup target add aarch64-unknown-linux-gnu

# Build
cargo build --release --target aarch64-unknown-linux-gnu
```

## License

[MIT](LICENSE)

## Documentation

Full technical documentation — architecture deep-dive, signaling protocol,
WebRTC engine internals, tunnel data flow, TUI layout, build/release pipeline,
and troubleshooting — lives in the
[GitHub Wiki](https://github.com/Kodjaoglanian/synapse/wiki).
