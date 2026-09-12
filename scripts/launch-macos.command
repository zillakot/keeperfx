#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/../out/game"
exec ../macos/keeperfx -nointro "$@"
