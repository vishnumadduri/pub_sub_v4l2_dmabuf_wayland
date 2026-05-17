#!/usr/bin/env bash
set -euo pipefail

BIN="$(cd "$(dirname "$0")" && pwd)/target/release"
SOCKET=/tmp/dma_buf_socket_dma

cleanup() {
    [[ -n "${PUB_PID:-}" ]] && kill "$PUB_PID" 2>/dev/null || true
    [[ -n "${SUB_PID:-}" ]] && kill "$SUB_PID" 2>/dev/null || true
    wait 2>/dev/null || true
    rm -f "$SOCKET"
}
trap cleanup EXIT INT TERM

rm -f "$SOCKET"

"$BIN/publisher_dma" \
    --device /dev/video0 --width 640 --height 480 --yuyv \
    --heap /dev/dma_heap/system --bufs 4 \
    --socket "$SOCKET" &
PUB_PID=$!

sleep 0.5

"$BIN/subscriber" --socket "$SOCKET" &
SUB_PID=$!

echo "publisher_dma PID=$PUB_PID  subscriber PID=$SUB_PID — running for 6 seconds..."
sleep 6
echo "test complete"
