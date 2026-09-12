#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
run_dir="${1:-out/graphics-feedback-$(date +%Y%m%d-%H%M%S)}"
scale="${2:-1}"
if [[ $# -gt 2 || -e "$run_dir" || ! "$scale" =~ ^[1-8]$ ]]; then
    echo "Usage: $0 [NEW_OUTPUT_DIRECTORY] [INTEGER_SCALE]" >&2
    exit 2
fi

brew_prefix=$(brew --prefix)
export PKG_CONFIG_PATH="$brew_prefix/opt/openal-soft/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cmake -S . -B out/macos -G Ninja -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_OSX_ARCHITECTURES=arm64
cmake --build out/macos --target keeperfx --parallel 8
python3 scripts/capture-frame.py --out "$run_dir/capture"
scripts/preview-frame.sh "$run_dir/capture" "$run_dir/comparison" "$scale"
