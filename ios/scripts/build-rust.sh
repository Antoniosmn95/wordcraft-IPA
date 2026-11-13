#!/bin/sh
# Builds the Rust static library the Xcode app links. Run from the repository root:
#   ios/scripts/build-rust.sh [device|simulator]
set -eu

mode="${1:-device}"
case "$mode" in
    device) target=aarch64-apple-ios ;;
    simulator) target=aarch64-apple-ios-sim ;;
    *) echo "usage: $0 [device|simulator]" >&2; exit 2 ;;
esac

rustup target add "$target"
cargo build --release -p wordcraft-ios --target "$target"
echo "built target/$target/release/libwordcraft_ios.a"
