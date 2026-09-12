#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
export CARGO_HOME="${CARGO_HOME:-$PWD/out/cargo-home}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/out/rust-target}"

if [[ $# -lt 1 || $# -gt 3 ]]; then
    echo "Usage: $0 CAPTURE_DIRECTORY [OUTPUT_DIRECTORY] [INTEGER_SCALE]" >&2
    exit 2
fi
capture="$1"
output="${2:-out/frame-preview-$(date +%Y%m%d-%H%M%S)}"
scale="${3:-1}"
cargo run --locked --manifest-path tools/frame-replay/Cargo.toml --release -- \
    "$capture/frame.kfx" --reference "$capture/reference.png" --out "$output" --scale "$scale"
