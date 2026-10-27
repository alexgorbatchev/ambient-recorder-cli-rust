#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
if [[ "$(uname -s)" == Darwin ]]; then
  export MACOSX_DEPLOYMENT_TARGET=14.2
fi
exec bash "$project_dir/crates/ambient-recorder/scripts/cargo.sh" "$@"
