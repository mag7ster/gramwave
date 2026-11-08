#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked
bundle=dist/tg-player
mkdir -p "$bundle/lib" "$bundle/share/applications" "$bundle/share/icons"
cp target/release/tg-player "$bundle/"
# Both copies come from the same pinned tdlib-rs release.
library=$(find target/release/build -path '*/tdlib/lib/libtdjson.so.1.8.61' -print -quit)
test -n "$library"
cp "$library" "$bundle/lib/libtdjson.so.1.8.61"
ln -sf libtdjson.so.1.8.61 "$bundle/lib/libtdjson.so"
cp assets/tgplayer.desktop "$bundle/share/applications/"
cp assets/tgplayer.svg "$bundle/share/icons/"
cp README.md LICENSE "$bundle/"
printf 'Local bundle: %s\n' "$bundle"
