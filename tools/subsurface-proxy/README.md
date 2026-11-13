# Subsurface cloud CORS proxy

Subsurface cloud storage is a git smart-HTTP server that sends no CORS
headers, so the web build of benthic cannot reach it directly. `proxy.py`
forwards only the git endpoints of a cloud account and adds CORS headers for
an allowlist of origins. It uses only the Python standard library.

```
GET  /healthz
GET  /git/<account>/info/refs?service=git-upload-pack|git-receive-pack
POST /git/<account>/git-upload-pack
POST /git/<account>/git-receive-pack
```

Everything else is a 404, and requests from other origins are a 403, so it is
not an open proxy. `http://localhost` and `http://127.0.0.1` on any port are
always allowed for local development. The upstream `WWW-Authenticate` header
is renamed to `X-WWW-Authenticate`, so a wrong password does not make the
browser show its own login prompt.

Credentials pass through the proxy (HTTPS on both sides) but are never
logged or stored.

## Configuration

| Variable          | Default                         |
| ----------------- | ------------------------------- |
| `LISTEN`          | `127.0.0.1:8787`                |
| `UPSTREAM`        | `cloud.subsurface-divelog.org`  |
| `ALLOWED_ORIGINS` | `https://awoimbee.github.io`     |
| `MAX_BODY_MB`     | `200`                           |

## Deployment

The instance benthic uses by default
(`https://mi-11i-termux.ghost-skate.ts.net/subsurface`) runs on a rooted
Android phone in Termux as a runit service, exposed with Tailscale Funnel
under a path prefix so the host can serve other things:

```sh
tailscale funnel --bg --set-path=/subsurface http://127.0.0.1:8787
```

Any host works; point benthic's "Server" setting (Sync → Subsurface cloud →
Advanced) at the proxy's base URL.
