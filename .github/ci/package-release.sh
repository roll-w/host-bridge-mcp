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

usage() {
  cat <<'EOF'
Usage:
  bash .github/ci/package-release.sh \
    --binary <path> \
    --binary-name <name> \
    --archive <tar.gz|zip|none> \
    --artifact-name <name> \
    [--output-dir <dir>]
EOF
}

binary_path=""
binary_name=""
archive_format=""
artifact_name=""
output_dir="dist"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      binary_path="$2"
      shift 2
      ;;
    --binary-name)
      binary_name="$2"
      shift 2
      ;;
    --archive)
      archive_format="$2"
      shift 2
      ;;
    --artifact-name)
      artifact_name="$2"
      shift 2
      ;;
    --output-dir)
      output_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Unknown argument: %s\n\n' "$1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$binary_path" || -z "$archive_format" || -z "$artifact_name" ]]; then
  usage >&2
  exit 1
fi

if [[ -z "$binary_name" ]]; then
  binary_name=$(basename "$binary_path")
fi

if [[ -z "$binary_name" ]]; then
  printf 'Binary name cannot be empty\n' >&2
  exit 1
fi

case "$archive_format" in
  tar.gz|zip|none)
    ;;
  *)
    printf 'Unsupported archive format: %s\n' "$archive_format" >&2
    exit 1
    ;;
esac

if [[ ! -f "$binary_path" ]]; then
  printf 'Binary not found: %s\n' "$binary_path" >&2
  exit 1
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
output_dir=${output_dir%/}
staging_dir="$repo_root/$output_dir/$artifact_name"
archive_path=""
artifact_path="$staging_dir"

if [[ "$archive_format" != "none" ]]; then
  archive_path="$repo_root/$output_dir/$artifact_name.$archive_format"
fi

rm -rf "$staging_dir"
if [[ -n "$archive_path" ]]; then
  rm -f "$archive_path"
fi

mkdir -p "$staging_dir"

cp "$binary_path" "$staging_dir/$binary_name"

for relative_path in host-bridge.yaml README.md LICENSE; do
  cp "$repo_root/$relative_path" "$staging_dir/$relative_path"
done

case "$archive_format" in
  none)
    ;;
  tar.gz)
    tar -czf "$archive_path" -C "$repo_root/$output_dir" "$artifact_name"
    artifact_path="$archive_path"
    ;;
  zip)
    python3 - <<'PY' "$repo_root/$output_dir" "$artifact_name"
import sys
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

output_dir = Path(sys.argv[1])
artifact_name = sys.argv[2]
archive_path = output_dir / f"{artifact_name}.zip"
staging_dir = output_dir / artifact_name

with ZipFile(archive_path, "w", compression=ZIP_DEFLATED, compresslevel=9) as archive:
    for path in staging_dir.rglob("*"):
        if path.is_file():
            archive.write(path, path.relative_to(output_dir))
PY
    artifact_path="$archive_path"
    ;;
esac

printf '%s\n' "$artifact_path"
