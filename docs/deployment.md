# vscreen Deployment Guide

## TLS / HTTPS

WebRTC requires a secure context (HTTPS or localhost) for most browser APIs, including `getUserMedia` and `navigator.clipboard`. For production deployments, TLS is strongly recommended.

### Option 1: Reverse Proxy (Recommended)

Use nginx, Caddy, or another reverse proxy to terminate TLS in front of vscreen.

**Caddy** (automatic HTTPS):

```
vscreen.example.com {
    reverse_proxy localhost:8450
}
```

**nginx**:

```nginx
server {
    listen 443 ssl http2;
    server_name vscreen.example.com;

    ssl_certificate     /etc/letsencrypt/live/vscreen.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/vscreen.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8450;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

### Option 2: Built-in TLS

Build vscreen with the `tls` feature and provide certificate files directly.

```bash
cargo build --release --features tls
```

Configure via TOML:

```toml
[server]
listen = "0.0.0.0:8450"

[server.tls]
cert_path = "/etc/letsencrypt/live/example.com/fullchain.pem"
key_path = "/etc/letsencrypt/live/example.com/privkey.pem"
```

Or run directly:

```bash
vscreen --config vscreen.toml --dev
```

## Authentication

Set a bearer token to protect the API and WebSocket endpoints:

```toml
[server]
auth_token = "your-secret-token-here"
```

- REST endpoints require `Authorization: Bearer <token>` header.
- WebSocket connections use `?token=<token>` query parameter.
- `/health` and `/metrics` are exempt from authentication.

## Docker

### Build and run:

```bash
docker compose up --build
```

### Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `VSCREEN_LISTEN` | `0.0.0.0:8450` | Listen address |
| `VSCREEN_LOG_LEVEL` | `info` | Log level (debug, info, warn, error) |
| `VSCREEN_CONFIG` | - | Path to TOML config file |

### Custom config with Docker:

```bash
docker run -p 8450:8450 \
  -v ./vscreen.toml:/etc/vscreen/vscreen.toml \
  vscreen --config /etc/vscreen/vscreen.toml --dev
```

## Prometheus Metrics

Metrics are exposed at `GET /metrics` in Prometheus text exposition format.

Available metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `vscreen_frames_encoded_total` | Counter | Total video frames encoded |
| `vscreen_frames_dropped_total` | Counter | Total video frames dropped (encode failures) |
| `vscreen_frames_skipped_total` | Counter | Stale video frames skipped |
| `vscreen_audio_frames_total` | Counter | Total audio frames encoded |
| `vscreen_active_peers` | Gauge | Currently connected WebRTC peers |
| `vscreen_encode_duration_seconds` | Histogram | VP9 encode time per frame |
| `vscreen_video_bitrate_kbps` | Gauge | Current video bitrate (adaptive) |

### Prometheus scrape config:

```yaml
scrape_configs:
  - job_name: vscreen
    static_configs:
      - targets: ['localhost:8450']
```
