const signalUrlInput = document.getElementById("signal-url") as HTMLInputElement;
const authTokenInput = document.getElementById("auth-token") as HTMLInputElement;
const navUrlInput = document.getElementById("nav-url") as HTMLInputElement;
const connectBtn = document.getElementById("connect-btn") as HTMLButtonElement;
const disconnectBtn = document.getElementById("disconnect-btn") as HTMLButtonElement;
const remoteVideo = document.getElementById("remote-video") as HTMLVideoElement;
const videoWrapper = document.getElementById("video-wrapper") as HTMLDivElement;
const wsStatus = document.getElementById("ws-status") as HTMLDivElement;
const rtcStatus = document.getElementById("rtc-status") as HTMLDivElement;
const statsDiv = document.getElementById("stats") as HTMLDivElement;
const logDiv = document.getElementById("log") as HTMLDivElement;
const volumeLabel = document.getElementById("volume-label") as HTMLSpanElement;
const volumeInput = document.getElementById("volume") as HTMLInputElement;
const softwareCursor = document.getElementById("software-cursor") as HTMLDivElement;
const statsHud = document.getElementById("stats-hud") as HTMLDivElement;
const hudFps = document.getElementById("hud-fps") as HTMLDivElement;
const hudBitrate = document.getElementById("hud-bitrate") as HTMLDivElement;
const hudRtt = document.getElementById("hud-rtt") as HTMLDivElement;
const hudLoss = document.getElementById("hud-loss") as HTMLDivElement;
const hudRes = document.getElementById("hud-res") as HTMLDivElement;
let hudVisible = false;

let ws: WebSocket | null = null;
let pc: RTCPeerConnection | null = null;
let dataChannel: RTCDataChannel | null = null;
let statsInterval: ReturnType<typeof setInterval> | null = null;
let instanceId = "dev";
let remoteDescriptionSet = false;
let iceCandidateBuffer: { candidate: string; sdpMLineIndex: number | null; sdpMid: string | null }[] = [];

let intentionalDisconnect = false;
let reconnectAttempt = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
const MAX_RECONNECT_ATTEMPTS = 20;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 15000;

// Actual remote dimensions — updated from the video stream metadata
let remoteWidth = 1920;
let remoteHeight = 1080;

function log(msg: string, level: "info" | "error" | "debug" = "debug") {
  const entry = document.createElement("div");
  entry.className = `log-entry ${level}`;
  entry.textContent = `[${new Date().toISOString().slice(11, 23)}] ${msg}`;
  logDiv.appendChild(entry);
  logDiv.scrollTop = logDiv.scrollHeight;
  if (logDiv.children.length > 200) {
    logDiv.removeChild(logDiv.firstChild!);
  }
}

function setWsStatus(connected: boolean) {
  wsStatus.className = `status-item ${connected ? "connected" : "disconnected"}`;
  wsStatus.textContent = `WS: ${connected ? "connected" : "disconnected"}`;
}

function setRtcStatus(state: string) {
  const ok = state === "connected";
  rtcStatus.className = `status-item ${ok ? "connected" : "disconnected"}`;
  rtcStatus.textContent = `RTC: ${state}`;
}

function scheduleReconnect() {
  if (intentionalDisconnect || reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
    if (reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
      log(`Reconnect failed after ${MAX_RECONNECT_ATTEMPTS} attempts`, "error");
      setWsStatus(false);
    }
    return;
  }

  reconnectAttempt++;
  const delay = Math.min(RECONNECT_BASE_MS * Math.pow(1.5, reconnectAttempt - 1), RECONNECT_MAX_MS);
  log(`Reconnecting in ${Math.round(delay / 1000)}s (attempt ${reconnectAttempt}/${MAX_RECONNECT_ATTEMPTS})...`, "info");
  wsStatus.textContent = `WS: reconnecting (${reconnectAttempt})...`;
  wsStatus.className = "status-item disconnected";

  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect();
  }, delay);
}

function cancelReconnect() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  reconnectAttempt = 0;
}

function apiBase(): string {
  return "";
}

async function connect() {
  intentionalDisconnect = false;

  const raw = signalUrlInput.value.trim();
  const match = raw.match(/\/signal\/([^/?]+)/);
  if (match) instanceId = match[1];

  // Build full WebSocket URL from the page origin when given a relative path
  let url = raw;
  if (raw.startsWith("/")) {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    url = `${proto}//${location.host}${raw}`;
  }

  const token = authTokenInput.value.trim();
  if (token) {
    url += (url.includes("?") ? "&" : "?") + `token=${encodeURIComponent(token)}`;
  }

  log(`Connecting to ${url}...`, "info");

  ws = new WebSocket(url);
  connectBtn.disabled = true;

  ws.onopen = async () => {
    reconnectAttempt = 0;
    setWsStatus(true);
    log("WebSocket connected", "info");
    disconnectBtn.disabled = false;
    await createPeerConnection();
  };

  ws.onmessage = async (event) => {
    const msg = JSON.parse(event.data);
    log(`← ${msg.type}: ${JSON.stringify(msg).slice(0, 120)}`);

    if (msg.type === "answer" && pc) {
      await pc.setRemoteDescription({ type: "answer", sdp: msg.sdp });
      remoteDescriptionSet = true;
      log("Remote description set", "info");

      for (const buffered of iceCandidateBuffer) {
        try {
          await pc.addIceCandidate(buffered);
        } catch (e) {
          log(`Buffered ICE candidate error: ${e}`, "error");
        }
      }
      if (iceCandidateBuffer.length > 0) {
        log(`Flushed ${iceCandidateBuffer.length} buffered ICE candidates`, "debug");
      }
      iceCandidateBuffer = [];
    } else if (msg.type === "ice_candidate" && pc) {
      const candidate = {
        candidate: msg.candidate,
        sdpMLineIndex: msg.sdpMLineIndex ?? null,
        sdpMid: msg.sdpMid ?? null,
      };
      if (!remoteDescriptionSet) {
        iceCandidateBuffer.push(candidate);
        log(`Buffered ICE candidate (remote description not set)`, "debug");
      } else {
        try {
          await pc.addIceCandidate(candidate);
        } catch (e) {
          log(`ICE candidate error: ${e}`, "error");
        }
      }
    } else if (msg.type === "connected") {
      log(`Peer ID: ${msg.peer_id}`, "info");
    } else if (msg.type === "clipboard" && msg.text) {
      navigator.clipboard.writeText(msg.text).then(
        () => log(`Clipboard synced (${msg.text.length} chars)`, "debug"),
        (e) => log(`Clipboard sync failed: ${e}`, "error")
      );
    } else if (msg.type === "error") {
      log(`Server error: ${msg.message}`, "error");
    }
  };

  ws.onclose = () => {
    setWsStatus(false);
    log("WebSocket disconnected");
    disconnectBtn.disabled = true;
    if (!intentionalDisconnect) {
      scheduleReconnect();
    } else {
      connectBtn.disabled = false;
    }
  };

  ws.onerror = () => {
    log("WebSocket error", "error");
  };
}

async function createPeerConnection() {
  pc = new RTCPeerConnection({
    iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
  });

  pc.onicecandidate = (event) => {
    if (event.candidate && ws) {
      ws.send(
        JSON.stringify({
          type: "ice_candidate",
          candidate: event.candidate.candidate,
          sdpMLineIndex: event.candidate.sdpMLineIndex,
          sdpMid: event.candidate.sdpMid,
        })
      );
    } else if (!event.candidate && ws) {
      ws.send(JSON.stringify({ type: "ice_complete" }));
    }
  };

  pc.onconnectionstatechange = () => {
    if (pc) {
      setRtcStatus(pc.connectionState);
      if (pc.connectionState === "connected") {
        startStats();
      } else if (pc.connectionState === "failed") {
        log("WebRTC connection failed, triggering reconnect", "error");
        stopStats();
        if (dataChannel) dataChannel.close();
        if (pc) pc.close();
        if (ws) ws.close();
        pc = null;
        ws = null;
        dataChannel = null;
        remoteDescriptionSet = false;
        iceCandidateBuffer = [];
        // ws.onclose will fire and call scheduleReconnect()
      }
    }
  };

  pc.ontrack = (event) => {
    log(`Track received: ${event.track.kind}`, "info");
    if (event.streams[0]) {
      remoteVideo.srcObject = event.streams[0];
    }
  };

  pc.addTransceiver("video", { direction: "recvonly" });
  pc.addTransceiver("audio", { direction: "recvonly" });

  // Input data channel
  dataChannel = pc.createDataChannel("input", { ordered: true });
  dataChannel.onopen = () => log("DataChannel open", "info");
  dataChannel.onclose = () => log("DataChannel closed");

  const offer = await pc.createOffer();
  await pc.setLocalDescription(offer);

  if (ws && offer.sdp) {
    ws.send(JSON.stringify({ type: "offer", sdp: offer.sdp }));
    log("Offer sent", "info");
  }
}

function disconnect() {
  intentionalDisconnect = true;
  cancelReconnect();
  stopStats();
  if (dataChannel) dataChannel.close();
  if (pc) pc.close();
  if (ws) ws.close();
  pc = null;
  ws = null;
  dataChannel = null;
  remoteDescriptionSet = false;
  iceCandidateBuffer = [];
  setWsStatus(false);
  setRtcStatus("disconnected");
  connectBtn.disabled = false;
  disconnectBtn.disabled = true;
  log("Disconnected", "info");
}

// --- URL Navigation ---

async function navigate() {
  const url = navUrlInput.value.trim();
  if (!url) return;

  try {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    const token = authTokenInput.value.trim();
    if (token) {
      headers["Authorization"] = `Bearer ${token}`;
    }
    const resp = await fetch(`${apiBase()}/instances/${instanceId}/navigate`, {
      method: "POST",
      headers,
      body: JSON.stringify({ url }),
    });
    const data = await resp.json();
    log(`Navigate: ${JSON.stringify(data)}`, "info");
  } catch (e) {
    log(`Navigate error: ${e}`, "error");
  }
}

// --- Volume Control ---

function setVolume(value: string) {
  const vol = parseInt(value, 10) / 100;
  remoteVideo.volume = vol;
  remoteVideo.muted = vol === 0;
  volumeLabel.textContent = `${value}%`;
}

// Set initial volume
remoteVideo.volume = 0.8;

// --- Fullscreen ---

function toggleFullscreen() {
  if (document.fullscreenElement) {
    document.exitFullscreen();
  } else {
    videoWrapper.requestFullscreen();
  }
}

// --- Coordinate Mapping ---

function mapCoords(e: MouseEvent): { x: number; y: number } {
  const rect = remoteVideo.getBoundingClientRect();

  // Use the actual video content dimensions if available
  const vw = remoteVideo.videoWidth || remoteWidth;
  const vh = remoteVideo.videoHeight || remoteHeight;

  // The video element may letterbox: compute the content area inside the element.
  // CSS `object-fit` defaults to "contain" behavior for <video>.
  const elemAspect = rect.width / rect.height;
  const vidAspect = vw / vh;

  let contentLeft = 0;
  let contentTop = 0;
  let contentWidth = rect.width;
  let contentHeight = rect.height;

  if (vidAspect > elemAspect) {
    // Letterbox top/bottom
    contentHeight = rect.width / vidAspect;
    contentTop = (rect.height - contentHeight) / 2;
  } else if (vidAspect < elemAspect) {
    // Pillarbox left/right
    contentWidth = rect.height * vidAspect;
    contentLeft = (rect.width - contentWidth) / 2;
  }

  // Position within the video content area (using clientX for reliability)
  const relX = e.clientX - rect.left - contentLeft;
  const relY = e.clientY - rect.top - contentTop;

  return {
    x: Math.round((relX / contentWidth) * vw),
    y: Math.round((relY / contentHeight) * vh),
  };
}

// --- Software Cursor ---

function updateCursor(e: MouseEvent) {
  const rect = videoWrapper.getBoundingClientRect();
  softwareCursor.style.left = `${e.clientX - rect.left}px`;
  softwareCursor.style.top = `${e.clientY - rect.top}px`;
  softwareCursor.style.display = "block";
}

videoWrapper.addEventListener("mouseenter", () => {
  softwareCursor.style.display = "block";
});

videoWrapper.addEventListener("mouseleave", () => {
  softwareCursor.style.display = "none";
});

// --- Input Forwarding ---

// Throttle mousemove to ~30 Hz to avoid flooding the CDP connection
let lastMoveTime = 0;
const MOVE_INTERVAL_MS = 1000 / 30;

remoteVideo.addEventListener("mousemove", (e) => {
  updateCursor(e);
  const now = performance.now();
  if (now - lastMoveTime < MOVE_INTERVAL_MS) return;
  lastMoveTime = now;
  const { x, y } = mapCoords(e);
  sendInput({ t: "mm", x, y, b: e.buttons, m: modifiers(e) });
});

remoteVideo.addEventListener("mousedown", (e) => {
  const { x, y } = mapCoords(e);
  sendInput({ t: "mm", x, y, b: e.buttons, m: modifiers(e) });
  sendInput({ t: "md", x, y, b: e.button, m: modifiers(e) });
});

remoteVideo.addEventListener("mouseup", (e) => {
  const { x, y } = mapCoords(e);
  sendInput({ t: "mu", x, y, b: e.button, m: modifiers(e) });
});

remoteVideo.addEventListener("wheel", (e) => {
  e.preventDefault();
  const { x, y } = mapCoords(e);
  sendInput({ t: "wh", x, y, dx: e.deltaX, dy: e.deltaY, m: modifiers(e) });
}, { passive: false });

// Right-click: prevent context menu, forward as mouse event
remoteVideo.addEventListener("contextmenu", (e) => {
  e.preventDefault();
});

// Keyboard capture: when video is focused, capture all keys
videoWrapper.tabIndex = 0;

videoWrapper.addEventListener("keydown", (e) => {
  // Let Ctrl+V / Cmd+V through so the paste event fires
  if ((e.ctrlKey || e.metaKey) && e.key === "v") return;
  e.preventDefault();
  e.stopPropagation();
  sendInput({ t: "kd", key: e.key, code: e.code, m: keyModifiers(e) });
});

videoWrapper.addEventListener("keyup", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === "v") return;
  e.preventDefault();
  e.stopPropagation();
  sendInput({ t: "ku", key: e.key, code: e.code, m: keyModifiers(e) });
});

// --- Clipboard Paste ---

videoWrapper.addEventListener("paste", (e) => {
  e.preventDefault();
  const text = e.clipboardData?.getData("text/plain");
  if (text) {
    sendInput({ t: "paste", text });
    log(`Pasted ${text.length} chars`, "debug");
  }
});

// F2 toggles stats HUD
document.addEventListener("keydown", (e) => {
  if (e.key === "F2") {
    e.preventDefault();
    hudVisible = !hudVisible;
    statsHud.style.display = hudVisible ? "block" : "none";
  }
});

// Auto-focus video wrapper when clicking on it
videoWrapper.addEventListener("mousedown", () => {
  videoWrapper.focus();
});

// Navigate on Enter in the URL input
navUrlInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") navigate();
});

function modifiers(e: MouseEvent): number {
  return (
    (e.altKey ? 1 : 0) |
    (e.ctrlKey ? 2 : 0) |
    (e.metaKey ? 4 : 0) |
    (e.shiftKey ? 8 : 0)
  );
}

function keyModifiers(e: KeyboardEvent): number {
  return (
    (e.altKey ? 1 : 0) |
    (e.ctrlKey ? 2 : 0) |
    (e.metaKey ? 4 : 0) |
    (e.shiftKey ? 8 : 0)
  );
}

function sendInput(event: object) {
  if (dataChannel && dataChannel.readyState === "open") {
    dataChannel.send(JSON.stringify(event));
  }
}

// --- WebRTC Stats ---

// --- Adaptive Bitrate ---
const ABR_MIN_KBPS = 1000;
const ABR_MAX_KBPS = 6000;
const ABR_DEFAULT_KBPS = 4000;
const ABR_LOSS_HIGH_PCT = 5;   // above 5% loss: cut bitrate
const ABR_LOSS_LOW_PCT = 1;    // below 1% loss: may increase
const ABR_GOOD_STREAK_NEEDED = 10; // 10s of low loss before ramping up
const ABR_STEP_DOWN = 0.7;     // aggressive cut on loss
const ABR_STEP_UP = 1.05;      // gentle 5% increase

let currentBitrateKbps = ABR_DEFAULT_KBPS;
let goodStreak = 0;
let lastSentBitrate = 0;

function adaptBitrate(lossPercent: number) {
  let newBitrate = currentBitrateKbps;

  if (lossPercent > ABR_LOSS_HIGH_PCT) {
    newBitrate = Math.round(currentBitrateKbps * ABR_STEP_DOWN);
    goodStreak = 0;
  } else if (lossPercent < ABR_LOSS_LOW_PCT) {
    goodStreak++;
    if (goodStreak >= ABR_GOOD_STREAK_NEEDED) {
      newBitrate = Math.round(currentBitrateKbps * ABR_STEP_UP);
      goodStreak = 0;
    }
  } else {
    goodStreak = 0;
  }

  newBitrate = Math.max(ABR_MIN_KBPS, Math.min(ABR_MAX_KBPS, newBitrate));
  currentBitrateKbps = newBitrate;

  if (newBitrate !== lastSentBitrate) {
    lastSentBitrate = newBitrate;
    sendInput({ t: "br", kbps: newBitrate });
    log(`ABR: bitrate → ${newBitrate} kbps (loss ${lossPercent.toFixed(1)}%)`, "debug");
  }
}

function startStats() {
  if (statsInterval) return;
  let lastFrames = 0;
  let lastTime = performance.now();
  let lastBytesReceived = 0;
  let lastPacketsReceived = 0;
  let lastPacketsLost = 0;

  statsInterval = setInterval(async () => {
    if (!pc) return;
    const stats = await pc.getStats();
    let fps = 0;
    let width = 0;
    let height = 0;
    let rtt = "-";
    let lossPercent: number | null = null;

    stats.forEach((report: any) => {
      if (report.type === "inbound-rtp" && report.kind === "video") {
        const now = performance.now();
        const elapsed = (now - lastTime) / 1000;
        const frames = report.framesDecoded || 0;
        fps = Math.round((frames - lastFrames) / elapsed);
        lastFrames = frames;
        lastTime = now;
        width = report.frameWidth || 0;
        height = report.frameHeight || 0;
        if (width > 0 && height > 0) {
          remoteWidth = width;
          remoteHeight = height;
        }

        const bytesReceived = report.bytesReceived || 0;
        const bitrate = Math.round(((bytesReceived - lastBytesReceived) * 8) / elapsed / 1000);
        lastBytesReceived = bytesReceived;

        const pr = report.packetsReceived || 0;
        const pl = report.packetsLost || 0;
        const totalNew = (pr - lastPacketsReceived) + (pl - lastPacketsLost);
        lossPercent = totalNew > 0 ? ((pl - lastPacketsLost) / totalNew * 100) : 0;
        lastPacketsReceived = pr;
        lastPacketsLost = pl;

        hudFps.textContent = `${fps} fps`;
        hudBitrate.textContent = `${bitrate} kbps`;
        hudRes.textContent = `${width}x${height}`;
        hudLoss.textContent = `${lossPercent.toFixed(1)}% loss`;
        hudLoss.className = lossPercent < 1 ? "good" : lossPercent < 5 ? "warn" : "bad";
      }
      if (report.type === "candidate-pair" && report.state === "succeeded") {
        const rttMs = report.currentRoundTripTime
          ? Math.round(report.currentRoundTripTime * 1000)
          : null;
        rtt = rttMs !== null ? `${rttMs}ms` : "-";
        if (rttMs !== null) {
          hudRtt.textContent = `${rttMs} ms`;
          hudRtt.className = rttMs < 50 ? "good" : rttMs < 150 ? "warn" : "bad";
        }
      }
    });

    statsDiv.textContent = `${fps} fps | ${width}x${height} | rtt: ${rtt}`;

    if (lossPercent !== null) {
      adaptBitrate(lossPercent);
    }
  }, 1000);
}

function stopStats() {
  if (statsInterval) {
    clearInterval(statsInterval);
    statsInterval = null;
  }
}

// Double-click to fullscreen
videoWrapper.addEventListener("dblclick", () => {
  toggleFullscreen();
});

// Expose API for debugging and HTML onclick handlers
(window as any).vscreen = { connect, disconnect, navigate, setVolume, toggleFullscreen, log };
