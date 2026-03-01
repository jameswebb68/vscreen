#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')}"
ARCH="$(uname -m)"
OS="linux"
NAME="vscreen-${VERSION}-${OS}-${ARCH}"
DIST="dist/${NAME}"

echo "Packaging ${NAME}..."

rm -rf dist/
mkdir -p "${DIST}"

cp target/release/vscreen "${DIST}/"
cp LICENSE "${DIST}/"
cp README.md "${DIST}/"

cat > "${DIST}/INSTALL.md" << 'INSTALL'
# Installing vscreen

## Runtime dependencies (Debian/Ubuntu)

```bash
sudo apt install -y \
  xvfb pulseaudio chromium \
  libvpx7 libopus0 libpulse0 \
  fonts-noto fonts-noto-color-emoji \
  ca-certificates
```

## Install the binary

```bash
sudo cp vscreen /usr/local/bin/
sudo chmod +x /usr/local/bin/vscreen
```

## Run

```bash
vscreen --dev
```

See README.md for full usage.
INSTALL

cd dist
tar czf "${NAME}.tar.gz" "${NAME}"
sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256"

echo ""
echo "Done:"
echo "  dist/${NAME}.tar.gz"
echo "  dist/${NAME}.tar.gz.sha256"
