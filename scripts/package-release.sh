#!/bin/sh
set -eu

target="${1:?usage: package-release.sh TARGET ASSET_NAME [BINARY_SUFFIX]}"
asset_name="${2:?usage: package-release.sh TARGET ASSET_NAME [BINARY_SUFFIX]}"
binary_suffix="${3:-}"

project_root="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
binary_path="$project_root/target/$target/release/deepswitch$binary_suffix"
archive_path="$project_root/dist/$asset_name.tar.gz"
staging_root="$(mktemp -d)"

cleanup() {
    rm -rf -- "$staging_root"
}
trap cleanup EXIT HUP INT TERM

if [ ! -f "$binary_path" ]; then
    echo "release binary not found: $binary_path" >&2
    exit 1
fi

package_root="$staging_root/$asset_name"
mkdir -p "$package_root" "$project_root/dist"
cp "$binary_path" "$package_root/"
cp "$project_root/README.md" "$project_root/LICENSE" "$package_root/"

if [ -z "$binary_suffix" ]; then
    chmod 755 "$package_root/deepswitch"
fi

tar -czf "$archive_path" -C "$staging_root" "$asset_name"
echo "$archive_path"
