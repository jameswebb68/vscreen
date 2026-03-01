#!/usr/bin/env bash
set -euo pipefail

BINARY="target/release/vscreen"
DEPLOY_DIR="${DEPLOY_DIR:-/opt/vscreen}"
CONFIG_FILE="${CONFIG_FILE:-/etc/vscreen/config.toml}"
SERVICE_NAME="vscreen"

echo "=== vscreen deployment ==="

# Build release binary
if [ ! -f "$BINARY" ]; then
    echo "Building release binary..."
    cargo build --workspace --release
fi

echo "Binary: $BINARY ($(stat -c%s "$BINARY" 2>/dev/null || echo "?") bytes)"

# Create deployment directory
sudo mkdir -p "$DEPLOY_DIR"
sudo mkdir -p "$(dirname "$CONFIG_FILE")"

# Backup previous version
if [ -f "$DEPLOY_DIR/vscreen" ]; then
    BACKUP="$DEPLOY_DIR/vscreen.$(date +%Y%m%d%H%M%S).bak"
    echo "Backing up previous version to $BACKUP"
    sudo cp "$DEPLOY_DIR/vscreen" "$BACKUP"
fi

# Deploy new binary
echo "Deploying to $DEPLOY_DIR..."
sudo cp "$BINARY" "$DEPLOY_DIR/vscreen"
sudo chmod +x "$DEPLOY_DIR/vscreen"

# Deploy default config if none exists
if [ ! -f "$CONFIG_FILE" ]; then
    echo "Creating default config at $CONFIG_FILE..."
    sudo tee "$CONFIG_FILE" >/dev/null <<'TOML'
[server]
listen = "0.0.0.0:8450"

[limits]
max_instances = 16
max_peers_per_instance = 8

[logging]
level = "info"
json = true
TOML
fi

# Restart service if systemd is available
if command -v systemctl &>/dev/null; then
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        echo "Restarting $SERVICE_NAME service..."
        sudo systemctl restart "$SERVICE_NAME"
    else
        echo "Service $SERVICE_NAME not running. Start with:"
        echo "  sudo systemctl start $SERVICE_NAME"
    fi
fi

echo ""
echo "=== Deployment complete ==="
echo "Binary: $DEPLOY_DIR/vscreen"
echo "Config: $CONFIG_FILE"
echo ""
echo "To rollback, restore the backup and restart the service."
