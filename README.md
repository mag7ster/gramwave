# Gramwave

Gramwave is a read-only Telegram music player for Linux. Listen to audio from
channels, groups, private chats, and Saved Messages.

https://github.com/user-attachments/assets/85dc199a-1325-4383-bdb4-9b5e1838e32b

## Features

- Search chats and find tracks within a playlist. Pin favorite chats and filter
  music, audio files, and voice messages.
- Save tracks or whole playlists for offline listening. Saved playlists
  automatically download new tracks that match your filters.
- Reorder the queue, see playback history, and undo removals. After a restart,
  press Play to resume the saved queue from where you left off.
- Open a track's original message in Telegram when a link is available.
- Shrink the window to a mini player and control playback with media keys or the
  system tray.

## Install with Homebrew

Prebuilt packages are available for Linux x86_64.

```sh
brew trust --tap mag7ster/gramwave
brew tap mag7ster/gramwave
brew install mag7ster/gramwave/gramwave
gramwave
```

To update, run `brew update && brew upgrade mag7ster/gramwave/gramwave`.
Packaging details are in the [Gramwave Homebrew tap](https://github.com/mag7ster/homebrew-gramwave).

## Connect Telegram

Scan the QR code from **Settings → Devices → Link Desktop Device** in Telegram
on your phone, or sign in with your phone number.

Closing the window keeps playback running when a system tray is available.
Choose **Quit** in the tray menu or press **Ctrl+Shift+Q** to exit completely.

## Build and run

Install a recent stable Rust toolchain and the libmpv development package.
Cargo downloads TDLib automatically. On Fedora, install the native dependencies
with `dnf`.

```sh
sudo dnf install mpv-devel openssl-libs zlib-ng-compat libcxx libcxxabi llvm-libunwind
```

Create a Telegram application to get an API ID and API hash. Put the ID on the
first line and the hash on the second line of a private file (mode `600`) outside
the checkout. The build embeds these credentials, so keep the file and your
local build artifacts private. [Telegram API credentials](https://my.telegram.org/apps)

From the repository root, pass the path to your file and run Gramwave.

```sh
GRAMWAVE_TELEGRAM_CONFIG=/path/to/telegram.conf cargo run --release --locked
```

To try the interface with fictional data, run the demo.

```sh
cargo run --locked --no-default-features -- --demo
```

See the developer documentation for [build variants](ARCHITECTURE.md#local-build-variants)
and [verification](ARCHITECTURE.md#verification).

## License

[MIT](LICENSE). Built with [TDLib](https://github.com/tdlib/td),
[mpv](https://mpv.io), [egui](https://github.com/emilk/egui) and
[fastframe](https://github.com/crmne/fastframe).
