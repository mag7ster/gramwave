# Refactoring status

This is an incremental implementation of the architecture plan. The full target
architecture is not yet complete; the remaining boundaries are listed below.

## Implemented boundaries

- `domain`: queue algorithms, `TrackId`, `QueueEntryId`, `MediaKind`, playlist and
  persisted state types. Production domain files import no GUI, SQL, filesystem,
  Telegram JSON or fastframe APIs. Integration tests enforce this restriction.
- `application`: shared playback commands, player/repository ports, desktop
  controller and a bounded worker pool. UI controls, tray and media keys route
  playback changes through the same command dispatcher.
- `adapters`: Telegram RPC allowlist and mapping, discovery, SQLite, mpv,
  independent HTTP server, offline file publication and fastframe conversions.
- `bootstrap`: process/window lifecycle. `main` invokes the library entry point.
- UI tests and default demo use no persistent store, player, tray or MPRIS.
  Player fakes can be injected through `PlayerBackend`. Desktop demo integrations
  are enabled explicitly with `--desktop-integrations`.

## Persistence and migration

`Store::open` imports the old `state` document into a transaction. It leaves that
original document intact and preserves an existing `queue_backup` using
`INSERT OR IGNORE`. A failed import cannot publish a partial migrated document.
Opening the database again reads the new schema rather than importing twice.

Settings, versioned playback session, offline requests, legacy queue backup and
library order are separate documents. Playlist headers and individual track
records are stored separately; unchanged records are not updated. Track rows
retain playlist position as well as message identity, preserving exact order
and duplicate records. Session documents preserve queue occurrence IDs, order,
history and snapshots. The schema has `user_version=2`.

Volume changes enqueue only two numbers and read/write only the settings
record. Pending snapshots and volume updates coalesce between flush barriers.
`flush` acknowledges all preceding work, returning write failure, channel
closure or timeout. Write failures remain sticky through shutdown, so a later
successful write cannot hide an earlier failed one.

## Tasks and shutdown

The application runs four workers with at most 64 waiting jobs. HTTP has a
separate four-worker pool and 16 waiting connections. Queue rejection is an
explicit error. Offline progress uses a single replaceable slot; terminal
results use a separate channel. Offline terminal results carry their generation
and cannot mutate state after cancellation. Offline publication filenames also
include the generation.

Shutdown rejects playback commands, invalidates operation generations, closes
the offline input queue, stops streaming and the player, requests TDLib close,
waits for TDLib and workers, then saves and flushes the repository. Blocking RPC
waits observe shutdown every 50 ms. HTTP and worker joins have deadlines; timeout
is reported rather than presented as success. Rust cannot forcibly kill a stuck
thread, so timed-out native work may still exist until process exit.

mpv resolves symbols before creating its handle. Its owner destroys the handle
on initialization/option failure and normal drop. Invalid commands and negative
FFI command results surface as playback errors.

## Remaining target work

- The desktop controller still owns substantial mutable state, and screens still
  access it. Separate auth/library/playback/download components and immutable UI
  views have not yet replaced all shared access.
- `State` and `Track` retain compatibility fields. Runtime media/offline data are
  not yet exclusively keyed by `TrackId`, and queue entries still use `Track`
  snapshots rather than a dedicated `QueueEntry` structure.
- Telegram raw RPC is crate-private and allowlisted, but JSON still crosses the
  application/auth boundary. A fully typed gateway and offline-storage port are
  still required. Not every task has a distinct `JobId`.
- Full saves still clone/serialize the library, although volume does not, and
  SQLite updates only changed records. Further use cases should emit repository
  deltas instead of snapshots.
- Native tests are explicitly feature-gated unit suites, not separate integration
  targets. Desktop CI now runs demo and MPRIS smoke tests; authenticated Telegram,
  real sound, tray restoration and all partial-initialization failure paths need
  further coverage.

## Verification commands

```sh
cargo fmt --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo clippy --locked --offline --all-targets --no-default-features -- -D warnings
cargo test --locked --offline
cargo test --locked --offline --no-default-features
cargo test --locked --offline --features native-tests --lib -- --test-threads=1
cargo build --locked --offline --release
cargo build --locked --offline --release --no-default-features
```

Native tests require libmpv and TDLib; missing dependencies fail the suite.
Loopback tests require socket permission. Default fast tests require neither
network sockets nor a graphical session. Desktop checks require a display and
D-Bus (`scripts/check-mpris.py`); CI supplies Xvfb and a private session bus.
