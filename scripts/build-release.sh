#!/usr/bin/env bash
# Builds release binaries (fully static where the platform supports it) into dist/.
# Usage: scripts/build-release.sh [--musl] [--macos]
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n1)"
DIST="dist"
mkdir -p "$DIST"
rm -f "$DIST/SHA256SUMS"

build() {
    local target="$1"
    shift
    echo "==> release build for $target"
    RUSTFLAGS="$*" cargo build --release --target "$target"
    local bin="silv"
    [[ "$target" == *windows* ]] && bin="silv.exe"
    local out="$DIST/silv-$VERSION-$target${bin/silv/}"
    cp -f "target/$target/release/$bin" "$out"
    (cd "$DIST" && sha256sum "$(basename "$out")" >>SHA256SUMS)
    echo "    -> $out"
}

native="$(rustc -vV | sed -n 's/^host: //p')"
case "$native" in
    *linux*) build "$native" "-C target-feature=+crt-static" ;;
    *windows*) build "$native" "-C target-feature=+crt-static" ;;
    *) build "$native" ;;  # macOS: system libc must stay dynamic
esac

if [[ "${1:-}" == "--musl" || "${1:-}" == "all" ]]; then
    if rustup target list --installed | grep -qx "x86_64-unknown-linux-musl"; then
        build "x86_64-unknown-linux-musl" "-C target-feature=+crt-static"
    else
        echo "musl target not installed: rustup target add x86_64-unknown-linux-musl" >&2
    fi
fi

echo "==> dist contents:"
ls -l "$DIST"
