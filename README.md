# Gramwave

A Telegram music player for Linux, built with Rust and egui. Listen to audio from
channels, groups, private chats and Saved Messages in a dedicated desktop app.

## Features

- Browse and search chats, pin favorites, and filter music, audio files and voice messages.
- Edit the playback queue, shuffle tracks and use repeat modes.
- Save tracks or entire playlists for offline listening.
- Control playback with media keys, MPRIS and the system tray.
- Switch between light and dark themes.

The player does not send messages or mark them as read. It currently supports
one Telegram account and has an English interface.

The executable and data directory are named `tg-player`.

## Build and run

You need a recent stable Rust toolchain and `libmpv.so.2`. The default build
downloads TDLib automatically; its runtime dependencies are OpenSSL 3, zlib,
libc++, libc++abi and LLVM libunwind.

On Fedora:

```sh
sudo dnf install mpv-libs openssl-libs zlib-ng-compat libcxx libcxxabi llvm-libunwind
cargo run --release --locked
```

To try the interface without a Telegram account or native playback dependencies:

```sh
cargo run --locked --no-default-features -- --demo
```

## Connect Telegram

1. Create an application at [my.telegram.org/apps](https://my.telegram.org/apps).
2. Enter its API ID and API hash in the player.
3. Sign in with a QR code or your phone number; enter your two-step verification
   password if requested.

Your session is saved for future launches. Account data and downloaded music live
in `$XDG_DATA_HOME/tg-player`, or `~/.local/share/tg-player` by default. Keep this
directory private.

Closing the window keeps playback in the tray when a tray host is available.
Use **Quit** in the tray menu or **Ctrl+Shift+Q** to exit completely.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the project structure and full check
matrix. Run `scripts/package.sh` to create a local bundle in `dist/tg-player`.

## License

[MIT](LICENSE). Built with [TDLib](https://github.com/tdlib/td),
[mpv](https://mpv.io), [egui](https://github.com/emilk/egui) and
[fastframe](https://github.com/crmne/fastframe).
