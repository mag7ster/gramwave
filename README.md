# TG Player

A native Rust / egui music player for Telegram on Linux. Channels, groups,
private chats and Saved Messages become read-only playlists. The interface
currently ships in English.

## Run

```sh
cargo run --release -- --demo
cargo run --release
```

Rust 1.99 or newer is recommended. The default Cargo feature downloads the
pinned TDLib distribution through `tdlib-rs`. Audio needs `libmpv.so.2`.
The TDLib binary needs OpenSSL 3, zlib, libc++, libc++abi and LLVM libunwind.
On Fedora these are provided by `mpv-libs openssl-libs zlib-ng-compat libcxx
libcxxabi llvm-libunwind`; distro package names can vary. No Python runtime
or setup script is used by the application.

Create application credentials at <https://my.telegram.org/apps>, enter the
API ID/hash, then sign in with QR or your phone number. Two-step verification
and Telegram's email login states are supported. Telegram decides how codes
are delivered; SMS is not guaranteed for third-party clients.

## Interface and sign-in

The player uses a Spotify-inspired dark layout, SVG controls, an optional queue
panel, and Telegram artwork when available. Set API credentials once; later
launches automatically restore the saved session. QR is the initial sign-in
method; phone, verification code, email and 2FA are separate form steps.

Closing the window (including niri Mod+Q) destroys only the window and keeps
playback in the tray. Launching again restores the same process. Choose
**Quit TG Player**, tray **Quit**, or Ctrl+Shift+Q to exit completely. Without
a tray host, closing exits so the app cannot become inaccessible.

Preview sign-in screens without an account:

```sh
cargo run -- --demo --preview-auth=setup
cargo run -- --demo --preview-auth=qr
cargo run -- --demo --preview-auth=code
cargo run -- --demo --preview-auth=password
cargo run -- --demo --preview-auth=email
cargo run -- --demo --preview-auth=error
```

## Behavior

- The sidebar shows locally pinned chats. Click search to browse all chats and
  type to filter by name; Escape or the close button returns to pins. Right-click
  a chat or use its header button to pin/unpin it. Pins do not download music
  or change Telegram.
- Artwork loads for visible rows and supports JPEG, PNG and WebP. Failed downloads
  can retry, and placeholders remain visible while images load.
- Startup loads chat names only, without searching message history or counting tracks.
- Opening a chat immediately shows saved tracks and refreshes that chat in the
  background: new tracks are merged, changed metadata is updated, and indexed
  messages are checked for removal. A failed check keeps the saved list intact.
  Scroll to load older music; switching chats cancels the previous job. No track
  totals are shown. TDLib cache eviction does not remove playlist entries.
- Saved tracks retain message identities, not TDLib runtime file IDs. After a
  restart, file metadata is resolved only for visible tracks or a requested track,
  including offline downloads. This prevents audio and artwork IDs from being mixed.
- Music, audio documents and voice messages have separate playlist filters.
- Playback advances on mpv end-of-file events; manual stop does not advance it.
- Double-click a track to play it. `+` adds it to the queue; `⤒` plays it next.
- Queue order is independent of Telegram. Nothing edits source messages.
- The download button saves a separate offline copy; saving a playlist also
  enables saving new matching tracks. Offline copies survive cache cleanup.
- Public chat links and available message links open in Telegram. Some private
  chats have no shareable link; the player never generates invitations.
- MPRIS and media keys use fastframe; close-to-tray is enabled only when a tray
  host is present. Use the tray's Quit command to exit in that case.
- TDLib's `online` option is explicitly false. The player never sends read,
  listened or typing receipts. Other Telegram clients still control their own
  presence; this does not hide the session from Telegram's Devices screen.

Configuration, session, SQLite index, automatic cache and offline copies live
under `$XDG_DATA_HOME/tg-player` (normally `~/.local/share/tg-player`). This
folder is private to the current user and is not part of the repository.
Keep it private: it contains account credentials and cached media.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
scripts/package.sh
```

Automated checks cover the native TDLib bootstrap, read-only presence policy,
audio parsing, SQLite persistence and loopback range streaming. Full QR/2FA
login and playback from Telegram still need verification with a real account.

`--demo` uses fictional library metadata and does not connect to Telegram.
`--demo --smoke-test` opens the window briefly, saves an application-only
preview to `/tmp/tg-player-preview.png`, and exits.

For a UI-only build without TDLib, use `cargo run --no-default-features -- --demo`.
The local bundle keeps TDLib in `lib/`; mpv and its C++ dependencies remain system
dependencies.

The application separates the resident/window lifecycle (`lifecycle.rs`),
screens (`screens.rs`), sign-in state (`auth.rs`), Telegram requests (`telegram.rs`),
media output/range streaming (`audio.rs`) and persisted data (`model.rs`).
Playback uses a loopback HTTP range bridge to feed TDLib's existing cache to
mpv, with an unpredictable per-process URL. It does not expose a LAN server.

## Localization

We reuse **fastframe-i18n**, the gettext approach used by Spotifast: English
source messages, compiled PO catalogs, context/plural support and English
fallback for untranslated entries. Normal builds do not require GNU gettext.

1. Run `scripts/update-translations.sh` to update `assets/i18n/tg-player.pot`
   (requires GNU gettext tools).
2. Copy the template to `assets/i18n/<language>.po` and translate with Poedit
   or another gettext editor. Preserve placeholders such as `{count}`.
3. Add a variant to `Locale` in `src/i18n.rs` and return the generated catalog
   in its `catalog()` implementation (for example `Some(&de::Translator)`).
   Add language selection there when a second translation is ready.
4. Rebuild; the build script discovers and compiles the catalogs. Fuzzy and
   unfinished translations fall back to English.

Product strings belong in translation calls. Telegram titles, artist names,
filenames and password hints are content and must never be translated.

## References and dependencies

- [Spotifast](https://github.com/crmne/spotifast): native layout and queue ideas.
- [fastframe](https://github.com/crmne/fastframe): fonts, gettext, MPRIS and tray.
- [TDLib](https://github.com/tdlib/td) / [tdlib-rs](https://github.com/FedericoBruzzone/tdlib-rs): Telegram sessions, updates and downloads.
- [mpv](https://github.com/mpv-player/mpv): decoding, playback and seeking.
- [tg-music-cli](https://github.com/andrwvaz2/tg-music-cli): channel playlists and next-track prefetch ideas.
- [SoundGrammy](https://github.com/rastuhacode/soundgrammy): streaming and cache behavior reference; its GPL source was not copied.

## Roadmap

Alternative compact and cover-card layouts; additional translations;
Windows/macOS packaging; richer embedded cover extraction. The first release
supports one Telegram account and cloud chats, not secret chats.
