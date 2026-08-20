#!/usr/bin/env python3
"""Minimal HTTP signaling server for synapse.

Stores SDP offers/answers and ICE candidates under path keys and returns them on
GET. ICE candidate keys are append-only (newline-delimited JSON); SDP keys are
overwrite-once.

Run: python3 signaling_server.py [port]   (default 8080)
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import sys

# key -> bytes. ICE keys accumulate newline-delimited JSON.
store: dict[str, bytes] = {}


class H(BaseHTTPRequestHandler):
    def _send(self, code: int, body: bytes = b"", ctype: str = "application/json"):
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(body)))
        self.send_header("access-control-allow-origin", "*")
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        k = self.path.lstrip("/")
        body = store.get(k)
        if body is None:
            self._send(404)
        else:
            self._send(200, body)

    def do_POST(self):
        k = self.path.lstrip("/")
        n = int(self.headers.get("content-length", 0))
        data = self.rfile.read(n)
        # ICE candidate paths look like ice/<room>/<side>; append a newline.
        if k.startswith("ice/"):
            prev = store.get(k, b"")
            store[k] = prev + (b"\n" if prev else b"") + data
        elif k.startswith("offer/"):
            # New offer -> clear all stale data for this room.
            room = k.split("/", 1)[1]
            for key in list(store):
                if key == f"offer/{room}" or key == f"answer/{room}" or key.startswith(f"ice/{room}/"):
                    del store[key]
            store[k] = data
        else:
            store[k] = data
        self._send(200)

    def do_DELETE(self):
        k = self.path.lstrip("/")
        store.pop(k, None)
        self._send(200)

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
    print(f"synapse signaling server on http://0.0.0.0:{port}")
    ThreadingHTTPServer(("0.0.0.0", port), H).serve_forever()
