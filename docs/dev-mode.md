# Dev Mode Guide

Dev mode is a built-in development environment that lets you test vscreen without manually configuring Xvfb, PulseAudio, or Chromium. A single `--dev` flag spawns everything automatically and tears it down on exit.

## What dev mode does

When you run `vscreen --dev`, the following happens in sequence:

1. **Starts Xvfb** on display `:99` (configurable via `--dev-display`) at 1920x1080 with 24-bit color
2. **Creates a PulseAudio null-sink** named `vscreen_dev` with a monitor source for audio capture
3. **Launches Chromium** with remote debugging enabled, configured to:
   - Use the virtual X11 display
   - Route audio to the `vscreen_dev` sink
   - Disable GPU acceleration and sandbox (headless-friendly)
   - Expose a CDP endpoint on a random port
4. **Starts the media pipeline** — CDP screencast for video, PulseAudio capture for audio
5. **Registers a `dev` instance** in the server's instance registry
6. **Optionally navigates** to the URL specified by `--dev-url`
7. **Starts the HTTP/WebSocket server** on port 8450

On shutdown (Ctrl+C or SIGTERM), dev mode tears down processes in reverse order: Chromium, PulseAudio module, Xvfb.

## Prerequisites

### System packages

```bash
sudo apt install -y \
  chromium xvfb pulseaudio \
  fonts-noto fonts-noto-color-emoji \
  libvpx-dev libopus-dev libpulse-dev
```

If the `chromium` package is named differently on your system (e.g., `chromium-browser`, `google-chrome-stable`), vscreen searches for multiple common names automatically.

### PulseAudio

PulseAudio must be running. Dev mode creates a null-sink module within the existing daemon — it does not start a new PulseAudio server.

```bash
# Check if PulseAudio is running
pulseaudio --check && echo "running" || echo "not running"

# Start it if needed (user-level daemon)
pulseaudio --start
```

### Build with PulseAudio support

The `pulse-audio` feature must be enabled at build time:

```bash
cargo build --release -p vscreen --features pulse-audio
```

Without this feature, audio capture falls back to silence generation.

## Usage

### Basic dev mode

```bash
./target/release/vscreen --dev
```

Starts the virtual display, browser, and server. Output looks like:

```
2026-02-19T12:00:00Z  INFO vscreen: starting vscreen listen=0.0.0.0:8450 dev=true
2026-02-19T12:00:00Z  INFO vscreen: starting dev environment...
2026-02-19T12:00:01Z  INFO vscreen: dev environment ready cdp=ws://127.0.0.1:34567/devtools/browser/... source=vscreen_dev.monitor
2026-02-19T12:00:01Z  INFO vscreen: dev instance pipeline started
2026-02-19T12:00:01Z  INFO vscreen: server listening addr=0.0.0.0:8450
```

### With a pre-loaded URL

```bash
./target/release/vscreen --dev --dev-url "https://www.youtube.com"
```

Chromium navigates to the URL immediately after startup.

### Custom display number

Use `--dev-display` if display `:99` is already in use:

```bash
./target/release/vscreen --dev --dev-display 42
```

### Combined with other options

Dev mode flags combine freely with all other CLI options:

```bash
./target/release/vscreen \
  --dev \
  --dev-url "https://example.com" \
  --listen 127.0.0.1:9000 \
  --log-level debug
```

## Connecting a client

### Using the test client

The test client uses Vite with a proxy so that all API and WebSocket traffic is routed through a single port. This means you only need to forward port 5173 for remote access.

```bash
cd tools/test-client
pnpm install   # or npm install
pnpm dev       # or npx vite
```

Then open `http://localhost:5173`.

Vite automatically proxies `/signal/*`, `/instances/*`, and `/health` to the vscreen backend on port 8450, so everything works through a single port.

**Steps:**

1. The Signal URL field defaults to `/signal/dev` — leave it as-is
2. Click **Connect**
3. The video feed should appear within a second or two
4. Use the URL input to navigate the remote browser
5. Click the video area to focus it, then use mouse and keyboard normally

**Port forwarding / remote access:**
If you are SSH-forwarding or otherwise exposing only the Vite port (5173), everything works out of the box. The client uses relative URLs, and Vite proxies all backend requests to `localhost:8450` on the server.

### Client controls

| Control | Action |
|---------|--------|
| URL bar + **Go** | Navigate the remote browser to a URL |
| Volume slider | Adjust playback volume (0–100%) |
| **Fullscreen** | Enter fullscreen mode for the video |
| Mouse click on video | Forwarded as click at the corresponding remote coordinate |
| Mouse wheel | Forwarded as scroll events |
| Right-click | Forwarded (browser context menu is suppressed) |
| Keyboard | All key events forwarded when the video wrapper is focused |

### Coordinate mapping

The test client automatically maps mouse coordinates from the video element's display size to the remote resolution (1920x1080). The video element can be any size on your screen — input coordinates are scaled proportionally.

## Receiving audio externally

In dev mode, Opus audio is also sent via RTP to `127.0.0.1:5004`. You can receive this with any RTP-capable tool.

### FFmpeg / FFplay

```bash
# Get the SDP from the API and play
ffplay -protocol_whitelist file,rtp,udp \
  -i <(curl -s http://localhost:8450/instances/dev/sdp)
```

### GStreamer

```bash
gst-launch-1.0 \
  udpsrc port=5004 \
    caps="application/x-rtp,media=audio,encoding-name=OPUS,clock-rate=48000,payload=111" \
  ! rtpopusdepay \
  ! opusdec \
  ! autoaudiosink
```

### VLC

```bash
curl -s http://localhost:8450/instances/dev/sdp > /tmp/vscreen.sdp
vlc /tmp/vscreen.sdp
```

## API interaction in dev mode

Dev mode creates an instance with the ID `dev`. All API calls use this ID.

### Navigate

```bash
curl -X POST http://localhost:8450/instances/dev/navigate \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

### Check health

```bash
curl http://localhost:8450/instances/dev/health
```

### Get audio SDP

```bash
curl http://localhost:8450/instances/dev/sdp
```

### List instances

```bash
curl http://localhost:8450/instances
```

## Troubleshooting

### "CDP endpoint not available"

Chromium failed to start or expose its debugging port.

- Check that Chromium is installed: `which chromium || which chromium-browser || which google-chrome-stable`
- Check if the display number is in use: `ls /tmp/.X11-unix/`
- Try a different display: `--dev-display 42`
- Run with `--log-level debug` for detailed startup logs

### No video / black screen

- Verify the screencast is active: check the debug logs for CDP `Page.startScreencast` messages
- Some pages may have minimal visual content — try `--dev-url "https://www.wikipedia.org"`
- Ensure fonts are installed (`fonts-noto` package) to avoid blank text rendering

### No audio

- Verify PulseAudio is running: `pulseaudio --check`
- Check that the null-sink was created: `pactl list short sinks | grep vscreen`
- Ensure the binary was built with `--features pulse-audio`
- Without the feature flag, the audio pipeline generates silence

### Connection fails in test client

- Confirm the signal URL matches the server's listen address and uses the `dev` instance ID
- Check browser console for WebSocket errors
- Ensure no firewall is blocking port 8450
- WebRTC may fail behind strict NATs — dev mode uses Google's public STUN server by default

### High CPU usage

- Reduce the framerate: set `framerate = 15` in the TOML config
- Lower the resolution: set `width = 1280, height = 720`
- Increase `cpu_used` (higher = faster/lower quality, range 0–9, default 6)

## Dev mode architecture

```
vscreen --dev
│
├── Xvfb :99 -screen 0 1920x1080x24
│
├── pactl load-module module-null-sink sink_name=vscreen_dev
│
├── chromium --remote-debugging-port=0 --display=:99
│   │         --audio-server-info=vscreen_dev
│   │
│   └── CDP WebSocket ──────────────┐
│                                    │
├── InstanceSupervisor               │
│   ├── CDP screencast task  ◄───────┘
│   │   └── RawFrame → VideoPipeline → VP9 EncodedPacket
│   │       (broadcast to all connected peers)
│   │
│   ├── Audio capture thread (PulseAudio)
│   │   └── f32 samples → OpusEncoder → Opus EncodedPacket
│   │       (broadcast to all connected peers)
│   │
│   ├── RTP sender task
│   │   └── Opus packets → UDP 127.0.0.1:5004
│   │
│   └── Input relay task
│       └── DataChannel events → CDP Input.dispatch*
│
└── HTTP/WS Server (axum) on :8450
    ├── REST API
    └── WebSocket signaling → PeerSession (WebRTC)
```
