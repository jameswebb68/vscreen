#!/usr/bin/env bash
set -euo pipefail

FIXTURE_DIR="crates/vscreen-video/tests/fixtures"
mkdir -p "$FIXTURE_DIR"

echo "Generating test JPEG fixtures..."

# 2x2 white
ffmpeg -f lavfi -i "color=white:size=2x2:duration=0.04:rate=25" \
    -frames:v 1 -q:v 2 "$FIXTURE_DIR/white_2x2.jpg" -y 2>/dev/null

# 1080p test pattern
ffmpeg -f lavfi -i "testsrc=size=1920x1080:duration=0.04:rate=25" \
    -frames:v 1 -q:v 2 "$FIXTURE_DIR/test_1080p.jpg" -y 2>/dev/null

# 720p test pattern
ffmpeg -f lavfi -i "testsrc=size=1280x720:duration=0.04:rate=25" \
    -frames:v 1 -q:v 2 "$FIXTURE_DIR/test_720p.jpg" -y 2>/dev/null

# 480p test pattern
ffmpeg -f lavfi -i "testsrc=size=854x480:duration=0.04:rate=25" \
    -frames:v 1 -q:v 2 "$FIXTURE_DIR/test_480p.jpg" -y 2>/dev/null

echo "Generating test audio fixtures..."

AUDIO_DIR="crates/vscreen-audio/tests/fixtures"
mkdir -p "$AUDIO_DIR"

# 440Hz sine wave, 1 second, stereo, 48kHz
ffmpeg -f lavfi -i "sine=frequency=440:duration=1:sample_rate=48000" \
    -ac 2 -f f32le "$AUDIO_DIR/sine_440hz_48k_stereo.raw" -y 2>/dev/null

# Silence, 1 second, stereo, 48kHz
ffmpeg -f lavfi -i "anullsrc=channel_layout=stereo:sample_rate=48000" \
    -t 1 -f f32le "$AUDIO_DIR/silence_48k_stereo.raw" -y 2>/dev/null

echo "Fixtures generated successfully."
ls -la "$FIXTURE_DIR"
ls -la "$AUDIO_DIR"
