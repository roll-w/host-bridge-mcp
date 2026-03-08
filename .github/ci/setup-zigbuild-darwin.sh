#!/usr/bin/env bash
#
# Copyright 2026-present RollW
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#        http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#

set -euo pipefail

sdk_version="${1:-}"
zig_version="${2:-0.15.2}"

if [[ -z "$sdk_version" ]]; then
  printf 'Usage: bash .github/ci/setup-zigbuild-darwin.sh <sdk-version> [zig-version]\n' >&2
  exit 1
fi

if [[ -f "$HOME/.cargo/env" ]]; then
  . "$HOME/.cargo/env"
fi

export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v curl >/dev/null 2>&1 || ! command -v tar >/dev/null 2>&1; then
  sudo apt-get update
  sudo apt-get install -y curl tar xz-utils
fi

arch=$(uname -m)
case "$arch" in
  x86_64)
    zig_arch_key="x86_64-linux"
    ;;
  aarch64)
    zig_arch_key="aarch64-linux"
    ;;
  *)
    printf 'Unsupported host architecture for Zig: %s\n' "$arch" >&2
    exit 1
    ;;
esac

zig_root="$HOME/.local/zig/$zig_version"

resolve_zig_path() {
  local root="$1"

  if [[ -x "$root/zig" ]]; then
    printf '%s\n' "$root/zig"
    return 0
  fi

  if [[ -x "$root/bin/zig" ]]; then
    printf '%s\n' "$root/bin/zig"
    return 0
  fi

  return 1
}

zig_executable="$(resolve_zig_path "$zig_root" || true)"

if [[ -z "$zig_executable" ]]; then
  mkdir -p "$zig_root"
  zig_url=$(python3 - <<'PY' "$zig_version" "$zig_arch_key"
import json
import sys
import urllib.request

version = sys.argv[1]
arch_key = sys.argv[2]
with urllib.request.urlopen('https://ziglang.org/download/index.json', timeout=30) as response:
    index = json.load(response)

print(index[version][arch_key]['tarball'])
PY
)
  archive_path="$HOME/.cache/zig-${zig_version}-${zig_arch_key}.tar.xz"
  mkdir -p "$(dirname "$archive_path")"
  curl --proto '=https' --tlsv1.2 -LsSf "$zig_url" -o "$archive_path"
  rm -rf "$zig_root"
  mkdir -p "$zig_root"
  tar -xf "$archive_path" -C "$zig_root" --strip-components=1
  zig_executable="$(resolve_zig_path "$zig_root")"
fi

zig_bin_dir="$(dirname "$zig_executable")"
export PATH="$zig_bin_dir:$PATH"

if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "$zig_bin_dir" >> "$GITHUB_PATH"
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  cargo install cargo-zigbuild --locked
fi

sdk_cache_root="$HOME/.cache/macos-sdk/$sdk_version"
sdk_dir="$sdk_cache_root/MacOSX${sdk_version}.sdk"
sdk_archive="$HOME/.cache/macos-sdk/tarballs/MacOSX${sdk_version}.sdk.tar.xz"
sdk_url="https://github.com/joseluisq/macosx-sdks/releases/download/${sdk_version}/MacOSX${sdk_version}.sdk.tar.xz"

mkdir -p "$(dirname "$sdk_archive")"
if [[ ! -f "$sdk_archive" ]]; then
  curl --proto '=https' --tlsv1.2 -LsSf "$sdk_url" -o "$sdk_archive"
fi

if [[ ! -d "$sdk_dir" ]]; then
  rm -rf "$sdk_cache_root"
  mkdir -p "$sdk_cache_root"
  tar -xf "$sdk_archive" -C "$sdk_cache_root"
fi

if [[ ! -d "$sdk_dir" ]]; then
  printf 'Failed to prepare macOS SDK at %s\n' "$sdk_dir" >&2
  exit 1
fi

if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    printf 'SDKROOT=%s\n' "$sdk_dir"
    printf 'PKG_CONFIG_SYSROOT_DIR=%s\n' "$sdk_dir"
  } >> "$GITHUB_ENV"
fi

zig version
printf 'macOS SDK ready at %s\n' "$sdk_dir"
