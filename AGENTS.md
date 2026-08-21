# synapse

Decentralized P2P tunneling platform with a high-end terminal UI (Rust + Tokio + Ratatui + webrtc-rs).

## Build / Run

- `cargo build` — debug build
- `cargo build --release` — release build (LTO, `panic = abort`)
- `cargo run` — launch the TUI (requires a TTY)
- `cargo run -- --headless` — run without a TTY, logs network events to stdout
- `cargo run -- --help` — CLI flags

## CLI flags

- `--stun <URLS>` — comma-separated STUN servers (env `SYNAPSE_STUN`; defaults to Google STUN)
- `--turn <URLS>` — comma-separated TURN servers (env `SYNAPSE_TURN`)
- `--peer LABEL:TOKEN` — seed peer to dial on startup (repeatable)
- `--tunnel PORT:PEER:HOST:PORT:LABEL` — local tunnel to open on startup (repeatable)
- `--signaling <URL>` — HTTP signaling server base URL (env `SYNAPSE_SIGNALING`)
- `--dial-room LOCAL_NAME:ROOM` — dial via signaling with this local identity (repeatable; needs `--signaling`)
- `--answer-room LOCAL_NAME:ROOM` — answer via signaling with this local identity (repeatable; needs `--signaling`)
- `--headless` — no TUI, log to stdout
- `--lang <en|pt>` — UI language (env `SYNAPSE_LANG`; default `en`)

## Subcommands

- `synapse update` — self-update: checks GitHub for a newer release, downloads
  the matching binary for the platform, verifies the SHA-256 checksum, and
  replaces the running executable in place.

## Keybindings (TUI)

- `Tab` / `Shift-Tab` — switch panel
- `j`/`k` or arrows — navigate list
- `Enter` — inspect node / submit modal
- `n` — quick-connect modal (paste raw SDP, manual out-of-band exchange)
- `s` — connect via signaling server (by room name; needs `--signaling`)
- `d` — disconnect selected peer
- `?` — help
- `q` / `Ctrl-C` — quit

## Connecting two peers via signaling

1. Run a signaling server (any HTTP server that stores a body under a key and
   returns it on GET; see the reference snippet below).
2. On machine A (offerer):
   `synapse --signaling http://server:8080 --dial-room alice:room1`
   or press `s` in the TUI, enter label `alice`, room `room1`, mode `dial`.
3. On machine B (answerer):
   `synapse --signaling http://server:8080 --answer-room bob:room1`
   or press `s`, enter label `bob`, room `room1`, mode `answer`.
4. ICE connects → each header keeps its local identity while the graph lists the remote identity with the right edge color.

### Reference signaling server (Python, ~30 lines)

```python
# signaling_server.py — run: python3 signaling_server.py 8080
from http.server import BaseHTTPRequestHandler, HTTPServer
store = {}  # key -> bytes
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        k = self.path.lstrip("/")
        body = store.get(k, None)
        if body is None:
            self.send_response(404); self.end_headers()
        else:
            self.send_response(200); self.send_header("content-type","application/json")
            self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        k = self.path.lstrip("/")
        n = int(self.headers.get("content-length", 0))
        data = self.rfile.read(n)
        store[k] = data  # for ICE keys, append instead of overwrite
        self.send_response(200); self.end_headers()
    def log_message(self, *a): pass
HTTPServer(("0.0.0.0", int(__import__("sys").argv[1])), H).serve_forever()
```

Note: for ICE candidates the server should *append* to the stored body (newline-
delimited JSON). The minimal server above overwrites; for production use a
server that appends, or run synapse with `--peer`/`n` for manual SDP exchange.

## Architecture

- `src/main.rs` — CLI (clap), terminal setup, panic hook, clean shutdown
- `src/app.rs` — global state, focus, modal forms, input → actions
- `src/events.rs` — unified event loop (crossterm keys + tick + network wakeup)
- `src/network/` — `engine.rs` (WebRTC ICE/DataChannel), `proxy.rs` (TCP bidirectional), `metrics.rs` (telemetry snapshots via `watch` every 100ms), `signaling.rs` (HTTP signaling client)
- `src/ui/` — `mod.rs` (layout + modals), `header.rs`, `graph.rs` (Canvas mesh), `tunnels.rs`, `sparklines.rs`, `theme.rs` (Tokyo Night palette)
- `src/i18n.rs` — English/Português strings
- `src/update.rs` — self-update logic (GitHub Releases, SHA-256 verify, in-place replace)

Network and UI are decoupled via `tokio::sync::mpsc`/`watch`/`broadcast`. No `unwrap()` in critical paths; terminal is always restored via a panic hook.

## Tunnel data flow (v0.2.6+)

1. `--tunnel PORT:PEER:HOST:PORT:LABEL` registers a listener spec keyed by
   `PEER` (label). The listener is created immediately; the peer is resolved
   by label when the signaling `hello` arrives.
2. Each accepted local TCP connection opens a **new** data channel carrying
   its destination metadata (`tunnel:<label>:<host>:<port>`), instead of
   reusing a stale per-tunnel channel.
3. The remote side parses the metadata, dials `HOST:PORT`, and bridges bytes
   bidirectionally (TcpStream ↔ RTCDataChannel).
4. The local side waits for the data channel to open before pumping bytes,
   and closes cleanly when the remote endpoint finishes.

## Tests

- `cargo test` — unit + regression tests (rendering, metrics, tunnel metadata)
- Regression: 80x24 rendering shows `PEERS 2`, Alice local, Bob remote.
- Regression: HTTP request Alice → tunnel → Bob → local server returns 200.

## CI

- `.github/workflows/ci.yml` — lint (fmt + clippy), smoke test, lockfile
  validation via `cargo metadata --locked --no-deps --format-version 1`.
- `.github/workflows/release.yml` — multi-target build on `v*` tags, uploads
  binaries + checksums, runs post-release verification.

## Notes

- Uses pinned crate versions: `webrtc = "0.11"`, `ratatui = "0.26"`, `crossterm = "0.27"`.
- `webrtc 0.11`'s `RTCDataChannel::send` takes `&bytes::Bytes`; `RTCDataChannelState` is private (compared via Debug string in `engine::is_dc_open`).
- `ratatui 0.26`: `Frame::size()` (not `area()`), `Rect::inner(&Margin)`, `Table::highlight_style` (not `row_highlight_style`).
- Remaining warnings are dead-code (reserved helpers/fields) — harmless.
- Full technical docs: https://github.com/Kodjaoglanian/synapse/wiki
