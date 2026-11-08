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

The player uses compact dark or light surfaces, SVG controls, an optional queue
panel, and Telegram artwork when available. The queue floats over the content
on narrow windows. At large UI scales the library moves into a compact menu. Set API credentials once; later
launches automatically restore the saved session. QR is the initial sign-in
method; phone, verification code, email and 2FA are separate form steps.

Closing the window (including niri Mod+Q) destroys only the window and keeps
playback in the tray. Launching again restores the same process. Choose
**Your library → … → Quit TG Player**, tray **Quit**, or Ctrl+Shift+Q to exit completely. Without
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
- Click anywhere on a track row to play it; click the current track to pause or
  resume. A double click does not immediately pause a newly started track.
  Row menus have independent actions, including copying the title.
- Starting a track snapshots the currently loaded, filtered and sorted list.
  Changing chats, search, sorting or loading more pages leaves this session intact.
  Without shuffle, playback continues with the following row. With shuffle, the
  chosen track plays first and the other snapshot tracks play once per cycle.
- The queue is one editable list. **Play next** inserts immediately after the
  current track; **Add to queue** appends. Duplicate additions are independent.
  Drag the handle or use Move up/down to reorder; remove any future entry.
  Clear queue keeps the current track and disables repeat.
- Selecting another track in the source playlist always creates a new queue from
  that loaded, filtered and sorted list, replacing previous additions and edits.
  Without shuffle, Previous can reach the preceding playlist rows: selecting
  track 2, then track 5, then Previous plays track 4.
- Selecting a track in the queue keeps its edited order. Previous can reach the
  skipped positions, including independent duplicate entries.
- Shuffle randomizes the remaining entries once; manual edits apply literally.
  Turning shuffle off restores the base order with those edits.
- Repeat cycles **Off → All → One → Off**. One repeats natural EOF only;
  Next always advances. All repeats the edited queue including additions, without
  resurrecting removals. Previous restarts after three seconds; earlier it returns
  through the queue's previous positions, with the departed track immediately next.
  A new shuffled queue starts with the selected track and has no previous position
  until advancing. An unshuffled queue also permits preceding source playlist rows.
- Tab reaches controls and rows; Enter activates a focused row. Space controls
  playback outside text fields. Seek supports click, drag preview and keyboard
  arrows (five seconds), Home and End. Volume arrows change one percent. Unknown
  duration disables seek. Mute restores the last nonzero volume.
- Modes, volume, current entry and exact queue order survive restart on pause.
  Play starts the saved track from the beginning. Legacy queues are imported once
  in their visible order after legacy repair; the original state is backed up in
  SQLite. Queue changes never edit Telegram messages.
- The download button saves a separate offline copy. Saving a playlist searches
  every page of the enabled Music/Audio files/Voice filters, including tracks
  that are not displayed yet. Screen search and sorting do not limit saving.
  Scans and download jobs resume after restart; at most two files download at once.
  Reconnect checks for new tracks while retaining the saved history cursor.
  New matching messages are saved while the playlist policy remains enabled.
- Offline copies survive cache cleanup and deletion of their source messages.
  Removing a copy excludes that track from automatic saving until you explicitly
  save it or save the playlist again. Cancelling offline downloads preserves
  ready files and disables the affected automatic policies.
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
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --no-default-features
# Explicit native suite: requires libmpv2, TDLib and loopback sockets
cargo test --workspace --locked --features native-tests --lib -- --test-threads=1
cargo build --workspace --locked --release
cargo build --workspace --locked --release --no-default-features
scripts/package.sh
```

The default suite covers domain rules, headless UI, parsing and temporary SQLite
repositories without starting system services. The explicit `native-tests` suite
covers TDLib bootstrap, mpv null output and loopback streaming. Full QR/2FA
login and playback from Telegram still need verification with a real account.

`--demo` uses fictional library metadata and starts no SQLite, Telegram, mpv,
tray or MPRIS services. Add `--desktop-integrations` to enable tray and MPRIS
for desktop integration checks. `--demo --smoke-test` opens the window briefly,
saves an application-only preview to `/tmp/tg-player-preview.png`, and exits.
Reproduce visual fixtures with long Latin/Cyrillic metadata, missing artwork,
unknown duration, offline badges and active controls:

```sh
cargo run -- --demo --scenario=playing --size=850x550 --smoke-test --screenshot=/tmp/dark-small.png
cargo run -- --demo --scenario=playing --size=1240x800 --light --smoke-test --screenshot=/tmp/light-large.png
cargo run -- --demo --scenario=loading --size=1240x800 --scale=2 --smoke-test --screenshot=/tmp/loading-200.png
cargo run -- --demo --scenario=error --size=1240x800 --scale=2 --light --smoke-test --screenshot=/tmp/error-200.png
```

Scenarios: `playing`, `paused`, `loading`, `error`, `settings`, `empty`, `filters`, `search`,
`queue` (empty queue). `--light` and `--scale=2` work with every scenario.
Sizes are logical window units; the compositor may override requested sizes,
and native display scaling changes screenshot pixel dimensions. Resize or float
this demo window when comparing exact sizes under a tiling compositor.

Tests include session transitions and legacy JSON, egui pointer/keyboard events,
text/control contrast in both palettes, and libmpv EOF, pause, volume and seeking
using null audio output. For real MPRIS round trips in an isolated bus, build the
debug binary and run `dbus-run-session python3 scripts/check-mpris.py` (requires
`gdbus` and a graphical session). Demo and null-output tests do not verify audible
output or authenticated Telegram streaming; those still require a real account
and listening device.

For a UI-only build without TDLib, use `cargo run --no-default-features -- --demo`.
The local bundle keeps TDLib in `lib/`; mpv and its C++ dependencies remain system
dependencies.

The library entry is `src/lib.rs`; `src/main.rs` only launches it. Domain types
and queue rules live in `crates/domain`; `crates/application` contains `AppCore`,
commands, ports, persistence deltas and the Tokio scheduler. The desktop controller
and views live in `src/application`, `src/ui.rs` and `src/screens.rs`; SQLite,
TDLib, mpv, offline files and HTTP live in `src/adapters`.
`src/bootstrap.rs` assembles the desktop lifecycle. See [architecture and migration
notes](ARCHITECTURE.md) for boundaries and verification requirements.
For an offline build using an already installed TDLib 1.8.61, set
`TG_PLAYER_TDLIB_DIR` to the directory containing `lib/libtdjson.so.1.8.61`.

Playback uses a loopback HTTP range bridge to feed TDLib's existing cache to
mpv, with an unpredictable per-process URL. It does not expose a LAN server.

## Control a running player

The binary can inspect and control an already running instance through its
private per-user instance socket. Restart an older running binary once to enable
this interface. Commands never launch a replacement app or open account data.
Use `--demo --ctl …` to target a running demo instead of the real player.

```sh
target/release/tg-player --ctl status
target/release/tg-player --ctl playlists
target/release/tg-player --ctl open <chat-id>
target/release/tg-player --ctl playlist
target/release/tg-player --ctl select 2
target/release/tg-player --ctl select 5
target/release/tg-player --ctl previous
target/release/tg-player --ctl queue
target/release/tg-player --ctl queue-play <entry-id>
```

`select` uses the current central playlist's search, filters and sort order, with
row numbers starting at 1; selecting the current row toggles pause. `queue-play`
uses the exact `entry_id` returned by `queue`. Other commands are `next`,
`previous`, `pause`, `resume`, `seek <seconds>`, `shuffle on|off` and
`repeat off|all|one`. Seek positions must be finite and nonnegative.

Replies are JSON: `{"ok":true,"data":…}` or `{"ok":false,"error":"…"}`.
Errors exit with a nonzero status, including an absent or outdated running app.
Action replies describe the state after the command is applied; audio can still
be `loading`, so poll `status` for completion or a playback error. Control also
works while the window is hidden in the tray.

Lists return `items`, `total`, `offset` and `next_offset`. Pass a non-null
`next_offset` as the command's next argument, for example `--ctl queue 50`.
Each page contains at most 50 entries and fits within the socket's 16 KiB reply
limit. Display titles, artists and errors are limited to 512 Unicode characters;
IDs remain exact. Replies contain playback/library metadata, never credentials
or media cache paths.

Check actual CLI round trips on an isolated demo instance with
`python3 scripts/check-control.py` after building the debug binary. A graphical
session is required; CI runs this with Xvfb. Existing media keys and MPRIS remain
available independently of this interface.

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

Track menus open with either right click or the ellipsis button. Click the middle
mouse button to start Firefox-style autoscroll in track, chat and queue lists.
Move away from the anchor to control direction and speed; click again or press
Escape to stop. Holding the middle button, moving and releasing also works. Previous follows
queue positions, including skipped entries, then preceding rows from the original
unshuffled playlist snapshot when playback began in the middle. Chat search combines cached matches with TDLib
account search after a short typing pause; it does not load message history.

## Database upgrade and native bindings

Upgrading an existing database first creates a verified SQLite Backup API copy
beside it. Pre-v3 imports use `library.sqlite.pre-v3-N.sqlite`; upgrading schema v3
uses `library.sqlite.pre-v4-N.sqlite`. Committed WAL data is included and previous
backups are never overwritten. Backup or migration failure reports an error and
leaves the previous schema/data intact. Original state rows and legacy documents
remain available. Schema v4 preserves v3's occurrence IDs and normalized sessions,
and stores offline policy/exclusion changes by key. Jobs, scan progress and files
survive the upgrade; ordinary saves do not change the schema version.

Older binaries cannot read schema v4. To roll back, close the player, preserve the
v4 database and its WAL/SHM files together, and restore the appropriate pre-upgrade
backup under the original name without stale WAL/SHM files beside it. Account and
media files remain in the same application data directory.

Normal builds do not require libclang or link libmpv at build time. The checked-in
mpv ABI comes from `vendor/mpv/client.h` (mpv v0.40.0, ISC licensed). To regenerate
it deliberately, install libclang and run:

```sh
cargo run --locked --manifest-path tools/mpv-bindings/Cargo.toml
cargo fmt --all
```

The generator is a separate developer tool and is not part of normal workspace
builds. See `ARCHITECTURE.md` for the complete check matrix.
