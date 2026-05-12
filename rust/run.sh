#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$SCRIPT_DIR/target/release"

DEVICE="${DEVICE:-/dev/video0}"
WIDTH="${WIDTH:-640}"
HEIGHT="${HEIGHT:-480}"
FORMAT="${FORMAT:---yuyv}"
SOCKET="${SOCKET:-/tmp/dma_buf_socket}"
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

PUB_PID=""

cleanup() {
    if [[ -n "$PUB_PID" ]] && kill -0 "$PUB_PID" 2>/dev/null; then
        kill "$PUB_PID"
        wait "$PUB_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET"
}
trap cleanup EXIT INT TERM

if [[ "${1:-}" == "--build" ]] || [[ ! -x "$BIN_DIR/publisher" ]] || [[ ! -x "$BIN_DIR/subscriber" ]]; then
    echo "Building..."
    cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"
fi

"$BIN_DIR/publisher" \
    --device "$DEVICE" \
    --width  "$WIDTH"  \
    --height "$HEIGHT" \
    $FORMAT            \
    --socket "$SOCKET" &
PUB_PID=$!

sleep 0.3

"$BIN_DIR/subscriber" --socket "$SOCKET"
