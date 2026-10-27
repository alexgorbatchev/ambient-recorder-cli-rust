#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$PWD/.tmp"
export TMPDIR="$PWD/.tmp"
export PKG_CONFIG_PATH="$project_dir/.tmp/native/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export PKG_CONFIG_ALL_STATIC=1
if [[ "$(uname -s)" == Linux ]]; then
  export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:-} -isystem $(cc -print-file-name=include)"
fi
exec cargo "$@"
