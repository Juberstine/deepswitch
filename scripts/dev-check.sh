#!/bin/sh
set -eu

cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked

cargo build --release --locked --target x86_64-unknown-linux-musl
static_binary="target/x86_64-unknown-linux-musl/release/codex-deepseek-switcher"
"$static_binary" --version
binary_description="$(file "$static_binary")"
case "$binary_description" in
    *"statically linked"* | *"static-pie linked"*) ;;
    *)
        echo "expected a static Linux binary, got: $binary_description" >&2
        exit 1
        ;;
esac

rm -rf dist
sh scripts/package-release.sh \
    x86_64-unknown-linux-musl \
    codex-deepseek-switcher-linux-x86_64
tar -tzf dist/codex-deepseek-switcher-linux-x86_64.tar.gz >/dev/null
