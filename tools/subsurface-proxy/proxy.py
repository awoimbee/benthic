#!/usr/bin/env python3
"""CORS proxy for Subsurface cloud storage.

Subsurface cloud is a git smart-HTTP server that sends no CORS headers, so a
web app cannot talk to it directly. This proxy forwards only the git
endpoints of a cloud account and adds CORS headers for an allowlist of
origins. It listens on localhost; Tailscale Funnel exposes it over HTTPS.

The upstream `WWW-Authenticate` header is renamed to `X-WWW-Authenticate` so
a failed login does not make the browser show its own password prompt.

Configuration (environment):
  LISTEN           host:port to listen on (default 127.0.0.1:8787)
  UPSTREAM         upstream host (default cloud.subsurface-divelog.org)
  ALLOWED_ORIGINS  comma-separated browser origins allowed to use the proxy;
                   http://localhost and http://127.0.0.1 on any port are
                   always allowed for local development
  MAX_BODY_MB      largest accepted request body (default 200)
"""

import http.client
import os
import re
import ssl
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

UPSTREAM = os.environ.get("UPSTREAM", "cloud.subsurface-divelog.org")
LISTEN = os.environ.get("LISTEN", "127.0.0.1:8787")
ORIGINS = {
    origin.strip().rstrip("/")
    for origin in os.environ.get(
        "ALLOWED_ORIGINS",
        "https://awoimbee.github.io",
    ).split(",")
    if origin.strip()
}
MAX_BODY = int(os.environ.get("MAX_BODY_MB", "200")) * 1024 * 1024
TIMEOUT = 120
CHUNK = 64 * 1024

LOCAL_ORIGIN = re.compile(r"^http://(localhost|127\.0\.0\.1)(:\d+)?$")
GIT_PATH = re.compile(r"^/git/[^/]+/(info/refs|git-upload-pack|git-receive-pack)$")
SERVICES = {"git-upload-pack", "git-receive-pack"}
REQUEST_HEADERS = {
    "accept",
    "accept-encoding",
    "authorization",
    "content-encoding",
    "content-type",
    "git-protocol",
    "user-agent",
}
RESPONSE_HEADERS = {
    "cache-control",
    "content-encoding",
    "content-type",
    "etag",
    "expires",
    "last-modified",
    "pragma",
}
ALLOW_HEADERS = "Authorization, Content-Type, Content-Encoding, Accept, Git-Protocol"
EXPOSE_HEADERS = "Content-Type, Content-Encoding, X-WWW-Authenticate"
TLS = ssl.create_default_context()


def origin_allowed(origin: str | None) -> bool:
    return origin is not None and (origin in ORIGINS or bool(LOCAL_ORIGIN.match(origin)))


def allowed(path: str) -> bool:
    url = urlsplit(path)
    match = GIT_PATH.match(url.path)
    if not match:
        return False
    if match.group(1) == "info/refs":
        return parse_qs(url.query).get("service", [""])[0] in SERVICES
    return True


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "subsurface-proxy"
    sys_version = ""

    def origin_ok(self) -> bool:
        origin = self.headers.get("Origin")
        return origin is None or origin_allowed(origin)

    def send_cors(self) -> None:
        origin = self.headers.get("Origin")
        if origin_allowed(origin):
            self.send_header("Access-Control-Allow-Origin", origin)
            self.send_header("Access-Control-Expose-Headers", EXPOSE_HEADERS)
        self.send_header("Vary", "Origin")

    def reply(self, status: int, message: str = "") -> None:
        body = message.encode()
        self.send_response(status)
        self.send_cors()
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def do_OPTIONS(self) -> None:
        if not allowed(self.path):
            return self.reply(404, "not found\n")
        if not origin_allowed(self.headers.get("Origin")):
            return self.reply(403, "origin not allowed\n")
        self.send_response(204)
        self.send_cors()
        self.send_header("Access-Control-Allow-Methods", "GET, POST")
        self.send_header("Access-Control-Allow-Headers", ALLOW_HEADERS)
        self.send_header("Access-Control-Max-Age", "86400")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self) -> None:
        if self.path == "/healthz":
            return self.reply(200, "ok\n")
        self.forward(None)

    def do_POST(self) -> None:
        length = self.headers.get("Content-Length")
        if length is None:
            return self.reply(411, "Content-Length required\n")
        try:
            size = int(length)
        except ValueError:
            return self.reply(400, "bad Content-Length\n")
        if size < 0 or size > MAX_BODY:
            return self.reply(413, "request too large\n")
        self.forward(self.rfile.read(size))

    def forward(self, body: bytes | None) -> None:
        if not allowed(self.path):
            return self.reply(404, "not found\n")
        if not self.origin_ok():
            return self.reply(403, "origin not allowed\n")
        headers = {
            name: value
            for name, value in self.headers.items()
            if name.lower() in REQUEST_HEADERS
        }
        upstream = http.client.HTTPSConnection(UPSTREAM, timeout=TIMEOUT, context=TLS)
        try:
            upstream.request(self.command, self.path, body=body, headers=headers)
            response = upstream.getresponse()
        except (OSError, http.client.HTTPException) as error:
            upstream.close()
            return self.reply(502, f"upstream error: {error}\n")
        try:
            self.relay(response)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            upstream.close()

    def relay(self, response: http.client.HTTPResponse) -> None:
        self.send_response(response.status, response.reason)
        for name, value in response.getheaders():
            if name.lower() in RESPONSE_HEADERS:
                self.send_header(name, value)
        challenge = response.getheader("WWW-Authenticate")
        if challenge:
            self.send_header("X-WWW-Authenticate", challenge)
        self.send_cors()
        length = response.getheader("Content-Length")
        if length is not None:
            self.send_header("Content-Length", length)
            self.end_headers()
            while chunk := response.read(CHUNK):
                self.wfile.write(chunk)
            return
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        while chunk := response.read1(CHUNK):
            self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
        self.wfile.write(b"0\r\n\r\n")

    def log_message(self, format: str, *args) -> None:
        sys.stderr.write("%s %s\n" % (self.address_string(), format % args))


def main() -> None:
    host, _, port = LISTEN.rpartition(":")
    server = ThreadingHTTPServer((host or "127.0.0.1", int(port)), Handler)
    server.daemon_threads = True
    sys.stderr.write(f"proxying {LISTEN} -> https://{UPSTREAM} for {sorted(ORIGINS)}\n")
    server.serve_forever()


if __name__ == "__main__":
    main()
