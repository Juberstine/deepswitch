#!/bin/sh
set -eu

python3 -m unittest discover -s tests -p 'test_*.py'
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked

cargo build --release --locked --target x86_64-unknown-linux-musl
static_binary="target/x86_64-unknown-linux-musl/release/deepswitch"
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
    deepswitch-linux-x86_64
tar -tzf dist/deepswitch-linux-x86_64.tar.gz >/dev/null
