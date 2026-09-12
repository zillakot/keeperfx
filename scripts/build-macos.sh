#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
brew_prefix=$(brew --prefix)
export PKG_CONFIG_PATH="$brew_prefix/opt/openal-soft/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

cmake -S . -B out/macos -G Ninja \
    -DCMAKE_BUILD_TYPE=RelWithDebInfo \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DBUILD_NUMBER="$(git rev-list --count HEAD)" \
    -DPACKAGE_SUFFIX=Mac-Prototype
cmake --build out/macos --target keeperfx --parallel 8

app=out/game/KeeperFX.app
mkdir -p "$app/Contents/MacOS"
cp out/macos/keeperfx "$app/Contents/MacOS/keeperfx"
cp build/macos/Info.plist "$app/Contents/Info.plist"
codesign --force --sign - "$app"
