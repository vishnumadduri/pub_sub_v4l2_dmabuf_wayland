#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/build"

DEVICE="${DEVICE:-/dev/video0}"
WIDTH="${WIDTH:-640}"
HEIGHT="${HEIGHT:-480}"
FORMAT="${FORMAT:---yuyv}"
SOCKET="${SOCKET:-/tmp/dma_buf_socket}"
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

cleanup() {
    if [[ -n "${PUB_PID:-}" ]] && kill -0 "$PUB_PID" 2>/dev/null; then
        kill "$PUB_PID"
        wait "$PUB_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET"
}
trap cleanup EXIT INT TERM

"$BUILD_DIR/publisher" \
    --device "$DEVICE" \
    --width  "$WIDTH"  \
    --height "$HEIGHT" \
    $FORMAT            \
    --socket "$SOCKET" &
PUB_PID=$!

# Give the publisher a moment to bind the socket before the subscriber connects.
sleep 0.3

"$BUILD_DIR/subscriber" --socket "$SOCKET"
