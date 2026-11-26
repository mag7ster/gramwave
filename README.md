# Gramwave

A Telegram music player for Linux, built with Rust and egui. Listen to audio from
channels, groups, private chats and Saved Messages in a dedicated desktop app.

## Features

- Browse and search chats, pin favorites, and filter music, audio files and voice messages.
- Open source chats in the system Telegram client and choose which playlist appears on startup.
- Edit the playback queue, shuffle tracks and use repeat modes.
- Save tracks or entire playlists for offline listening and manage temporary media by category and chat.
- Resize down to a mini player and view color emoji and animated Telegram emoji.
- Control playback with media keys, MPRIS and the system tray.
- Switch between light and dark themes.

The player does not send messages or mark them as read. It currently supports
one Telegram account and has an English interface.

## Install with Homebrew

On Linux, add the Gramwave tap and install the player:

```sh
brew trust --tap mag7ster/gramwave
brew tap mag7ster/gramwave
brew install mag7ster/gramwave/gramwave
gramwave
```

Homebrew builds Gramwave and TDLib from source. To update, run `brew update`
and `brew upgrade mag7ster/gramwave/gramwave`. For the launcher to appear in a
desktop menu, the session needs Homebrew's `bin` on `PATH` and Homebrew's
`share` in `XDG_DATA_DIRS`.

## Build and run

You need a recent stable Rust toolchain and the libmpv development package
to build Gramwave. The executable needs `libmpv.so.2` at runtime. The default
build downloads TDLib automatically. Its runtime dependencies are OpenSSL 3,
zlib, libc++, libc++abi and LLVM libunwind.

Opening a source chat uses GIO (`gio`) and a registered `tg://` handler such as
Telegram Desktop. Video emoji use FFmpeg with the `libvpx-vp9` decoder for
transparency. Without that decoder the player shows a thumbnail or the associated
Unicode emoji.

On Fedora:

```sh
sudo dnf install mpv-devel openssl-libs zlib-ng-compat libcxx libcxxabi llvm-libunwind
cargo run --release --locked
```

Choose a renderer when building the release executable at `target/release/gramwave`.

- Vulkan (default): `cargo build --release --locked`
- OpenGL: `cargo build --release --locked --no-default-features --features mpv,download-tdlib`

To build with an installed TDLib 1.8.61, set `GRAMWAVE_TDLIB_DIR` to its
installation prefix and enable `mpv,telegram,vulkan` with `--no-default-features`.

## Connect Telegram

1. Create an application at [my.telegram.org/apps](https://my.telegram.org/apps).
2. Enter its API ID and API hash in the player.
3. Sign in with a QR code or your phone number; enter your two-step verification
   password if requested.

Your session is saved for future launches. Account data and downloaded music live
in `$XDG_DATA_HOME/gramwave`, or `~/.local/share/gramwave` by default. Keep this
directory private.

Closing the window keeps playback in the tray when a tray host is available.
Use **Quit** in the tray menu or **Ctrl+Shift+Q** to exit completely.

## Development

The demo uses fictional data for development and UI testing. Build it without
the `mpv` and `telegram` features to run it without either native library:

```sh
cargo run --locked --no-default-features -- --demo
```

```sh
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the project structure and full check
matrix.

## License

[MIT](LICENSE). Built with [TDLib](https://github.com/tdlib/td),
[mpv](https://mpv.io), [egui](https://github.com/emilk/egui) and
[fastframe](https://github.com/crmne/fastframe).
