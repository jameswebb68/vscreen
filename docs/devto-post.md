---
title: I built a tool that lets AI agents browse the real internet — and you can watch them do it
published: true
tags: rust, ai, webrtc, opensource
---

AI agents can write code and analyze data, but they can't browse a website, click a button, or fill out a form. They don't have a browser.

So I built one.

## What is vscreen?

A Rust service that gives AI agents a real Chromium browser and streams it to you live over WebRTC. You see what the AI sees in real-time — video, audio, everything. Mouse and keyboard relay back bidirectionally, so you can take over at any time.

AI agents connect via MCP (Model Context Protocol) with **63 automation tools**: navigate, screenshot, click, type, find elements, wait for page loads, dismiss cookie banners, solve CAPTCHAs, manage cookies, and more.

Spin up **multiple isolated instances** — different agents working different tasks in parallel, with lease-based locking so they don't step on each other.

## The tech

Written in Rust from scratch. Not a Puppeteer wrapper. A purpose-built media pipeline: tokio, axum, webrtc-rs, openh264/libvpx, Opus audio.

- `unsafe_code = "forbid"` across the workspace
- `unwrap()` denied, `panic` denied — every error path handled
- Clippy pedantic + nursery enforced
- 510+ tests, 3 fuzz targets, supply chain auditing via `cargo-deny`
- ~31,000 lines of Rust across 8 crates

## Get started

Pre-built binaries for **Linux and Windows** are available on the [releases page](https://github.com/jameswebb68/vscreen/releases/latest). Or build from source and run:

```bash
vscreen --dev
```

One command. Spins up Xvfb, PulseAudio, and Chromium. Or use Docker:

```bash
docker run -p 8450:8450 vscreen
```

Source-available, non-commercial license.

**GitHub: [github.com/jameswebb68/vscreen](https://github.com/jameswebb68/vscreen)**

*Built by Jonathan Retting.*
