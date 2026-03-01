#!/usr/bin/env bash
set -euo pipefail

echo "=== vscreen developer environment setup ==="
echo ""

# Check Rust
if ! command -v rustc &>/dev/null; then
    echo "ERROR: Rust not found. Install from https://rustup.rs/"
    exit 1
fi
echo "Rust: $(rustc --version)"

# Check cargo
echo "Cargo: $(cargo --version)"

# Install Rust toolchain from rust-toolchain.toml
echo ""
echo "Setting up Rust toolchain..."
rustup show

# Check system dependencies
echo ""
echo "Checking system dependencies..."

check_pkg() {
    if pkg-config --exists "$1" 2>/dev/null; then
        echo "  ✓ $1"
    else
        echo "  ✗ $1 (install: $2)"
        MISSING=1
    fi
}

MISSING=0
check_pkg "libssl" "apt install libssl-dev"
check_pkg "opus" "apt install libopus-dev"

if [ "$MISSING" -eq 1 ]; then
    echo ""
    echo "WARNING: Some system dependencies are missing."
    echo "Install them and re-run this script."
fi

# Check Node.js and pnpm
echo ""
if command -v node &>/dev/null; then
    echo "Node.js: $(node --version)"
else
    echo "Node.js: not found (optional, for E2E tests and test client)"
fi

if command -v pnpm &>/dev/null; then
    echo "pnpm: $(pnpm --version)"
else
    echo "pnpm: not found (install: npm install -g pnpm)"
fi

# Verify workspace
echo ""
echo "Verifying workspace..."
cargo check --workspace 2>/dev/null && echo "  ✓ cargo check passes" || echo "  ✗ cargo check failed"

# Generate fixtures if ffmpeg is available
if command -v ffmpeg &>/dev/null; then
    echo ""
    echo "Generating test fixtures..."
    bash scripts/generate-fixtures.sh
else
    echo ""
    echo "ffmpeg not found, skipping fixture generation."
fi

echo ""
echo "=== Setup complete ==="
echo ""
echo "Quick start:"
echo "  cargo build --workspace    # Build all crates"
echo "  cargo test --workspace     # Run all tests"
echo "  cargo run -- --help        # Show CLI help"
echo ""
