#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
native_dir="$project_dir/.tmp/native"
mkdir -p "$native_dir"
build_component() {
  local component="$1" version="$2" checksum="$3"
  local archive="$component-$version.tar.gz"
  local source_dir="$native_dir/$component-$version"
  if [[ ! -f "$native_dir/$archive" ]]; then
    curl --fail --location --retry 3 "https://downloads.xiph.org/releases/opus/$archive" --output "$native_dir/$archive"
  fi
  printf '%s  %s\n' "$checksum" "$native_dir/$archive" | shasum -a 256 -c -
  if [[ ! -d "$source_dir" ]]; then
    tar -xzf "$native_dir/$archive" -C "$native_dir"
  fi
  if [[ "$component" == opus ]]; then
    (cd "$source_dir" && ./configure --prefix="$native_dir" --disable-shared --enable-static --disable-doc --disable-extra-programs && make -j 4 && make install)
  else
    (cd "$source_dir" && PKG_CONFIG_PATH="$native_dir/lib/pkgconfig" ./configure --prefix="$native_dir" --disable-shared --enable-static --disable-examples && make -j 4 && make install)
  fi
}
build_component opus 1.6.1 6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1
build_component libopusenc 0.3 f616d3aff9b2034547894ccb8ab56c36cf1a4acb0d922c5d7119f97bbe58642c
