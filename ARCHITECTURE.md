# Gramwave architecture

The Cargo workspace has three packages. `tg-player-domain` depends only on serde
and rand. `tg-player-application` depends on domain and supplies commands, ports,
`AppCore`, explicit persistence records, download policies and the task scheduler.
The desktop package supplies egui, native services and bootstrap. Domain cannot
import application or desktop, and application cannot import desktop adapters.
`tests/architecture.rs` also checks that screens contain no filesystem or Telegram
RPC calls. Public process entry points remain `run` and `run_cli`.

## State and commands

`DesktopApp` owns presentation state, `AppCore` and an effect executor on the main
thread. `AppCore` coordinates AuthCore, LibraryCore, PlaybackCore and OfflineCore.
Commands produce typed effects; adapters return AppEvent values. The reducers
decide when to start, retry, cancel or accept an operation. Bootstrap constructs
native services. Immutable views expose library, session and component state;
there is no mutable offline escape hatch or State Deref. Internal mutation
commands are private to application. UI row selection resolves the visible
filtered/sorted list before submitting the session commands also used by CLI,
tray and MPRIS.

Domain `TrackMetadata` contains message identity and metadata only. A `QueueEntry`
adds a distinct occurrence ID. Session entries live once in an ID map; original
order, remaining order, current, history and preceding positions reference IDs.
Shuffle accepts an injected RNG for reproducible checks. Library edits cannot
alter an already started queue's metadata or order. Legacy queue fields live only
in `legacy::LegacyImport`, and old session documents deserialize into the normalized
session. Application Track and Playlist contain metadata only. `media::MediaTrack`,
`MediaPlaylist` and `MediaPatch` explicitly carry transient gateway results;
LibraryCore separates their file IDs/paths into connection-scoped caches before
updating metadata. The desktop has its own nonserialized presentation Track.
Artwork is cached by connection-local file ID and chat; offline copies have an
independent metadata registry.

## SQLite schema v4

One dedicated thread owns the connection. `Repository::apply(ChangeSet)` accepts
settings, session, affected library records/order and offline changes. Settings
are explicit typed records. Session and volume updates never serialize the
library. Library rows have stable occurrence IDs, so duplicate messages and order
survive reordering and restart. Record contents and order are separate tables.
Offline copies, pending downloads, file removals, policies/cursors and deletion
exclusions have keyed tables. OfflineDelta contains changed keys and deletion
tombstones. Indexed queues preserve enqueue order; SQLite preserves that order
across restart and requeue. A page's jobs and both scan cursors commit together.
Completing one of 10,000 downloads writes a copy and deletes one job, without
serializing or querying the entire backlog.

Pending changes coalesce by entity before a flush barrier. The UI retains rejected
changes and retries them on a later command; rejection does not recursively
resubmit the same write. Flush acknowledges prior writes and returns write, closure or timeout
errors. Write failures remain sticky through shutdown. No production port accepts
whole-State snapshots.

Before upgrading an existing database, the SQLite Backup API captures
committed WAL pages. The copy is integrity-checked, synced and published without
clobbering any earlier backup. Migration writes are transactional; original state,
queue backup, old track rows and legacy documents are preserved. Pre-v3 imports
use `.pre-v3-N.sqlite` backups; v3 upgrades use `.pre-v4-N.sqlite`. Schema creation,
records and user_version=4 commit in one transaction. Reopening reads v4 without
reimporting. Normal writes neither query nor change user_version.
Missing tables, corrupt records and unsupported newer schemas fail startup instead of producing
an empty library. Rollback requires closing the player and restoring the backup,
with the old database's matching WAL/SHM files kept separate.

## Async services and lifecycle

A single Tokio runtime uses two worker threads. Playback has one active task and
one replaceable pending request. A new selection cancels the old operation and
replaces queued preparation synchronously; the newest request cannot be rejected
while an old cancellation releases its permit. Interactive/auth has one active
slot, background search/index work four, offline downloads two and scanning one.
The ready background queue is bounded at 64; the download backlog stays in SQLite.
Operation IDs, generations and cancellation tokens originate in the core and pass
unchanged through the scheduler. Panics, cancellation and overload produce addressed terminal outcomes;
terminal delivery is tracked through shutdown. Progress uses a replaceable slot.
The UI processes bounded batches and caches filtered/sorted playlist views by
library, media and offline versions plus search and sort parameters.

SQLite, TDLib receiving and mpv have dedicated owner threads. Telegram's managed
transport uses oneshot responses, a 60-second RPC deadline and short receive
intervals. Dropping or cancelling a future removes its registry entry immediately;
late replies are discarded. Gateway requests/updates and auth operations are typed.
JSON and the C API stay in adapters; tdlib-rs 1.4.0 remains pinned and supplies API
types. A closed allowlist preserves read-only behavior and online=false.

mpv loads libmpv.so.2 with libloading. Checked-in bindgen ABI definitions come from
the pinned vendor header; ordinary builds need neither libclang nor a link-time
libmpv dependency. The owner thread validates symbols, owns and destroys the handle,
executes commands and samples native properties outside the snapshot mutex.
A brief lock publishes a complete typed snapshot; readers never wait for native
property calls. Submission and native failures use AppError. Commands/events are correlated with
loads and mpv playlist entry IDs, so stale EOF and manual Stop cannot advance a
new queue entry. Initialization errors retain the original cause.

HTTP uses axum on 127.0.0.1 with a random port and a getrandom token. GET and HEAD
support single byte ranges via http-range-header. Unsatisfiable ranges return 416;
multiple ranges are ignored with a full 200 response. HEAD never downloads a body.
Four transfers may be active with sixteen additional admitted requests. Permits
live with response bodies. Partial-file reads observe cancellation and a 30-second
no-progress timeout. Download leases coordinate streaming and offline consumers;
release cancels the TDLib transfer only when its last consumer disappears.

Shutdown stops commands, cancels operations, stops streaming/mpv, closes Telegram,
awaits task results and flushes/stops the repository against one 15-second deadline.
Timeout is an error and Drop performs no repeated blocking wait. A stuck native
thread cannot be forcibly killed by Rust and may remain until process exit.

## Full playlist saving

Saving captures enabled media filters and follows TDLib next_from_message_id for
each type independently. Zero ends traversal; short pages do not. Music search
uses Audio, Document (with MIME/extension validation) and VoiceNote filters, never
an unfiltered text-history walk. Display cursors and offline scan cursors are
independent; saving does not expand the displayed list or active playback queue.
Each media type persists separate history and head-refresh cursors. Reconnect
retains history progress and resumes interrupted refreshes. Only a completed head
scan advances its watermark. The highest server message ID is recorded before
music filtering, so pages containing only non-audio documents do not force
repeated historical scans. Legacy policies initialize a missing watermark once;
duplicate Ready updates are idempotent.

Jobs deduplicate by TrackId. Policies, cursors, exclusions, ready files and pending
work survive restart. Live message changes take precedence over old scan pages;
ready copies survive source deletion. Publication checks the completed byte count,
syncs a temporary file inside offline storage and publishes a unique final name.
Each attempt has its own cancellation identity. Old results cannot publish over,
remove or satisfy newer attempts. Removing a copy records an automatic-download
exclusion. Explicit track/playlist saving clears the corresponding exclusion.
Cancellation stops scans and pending downloads, disables policies and keeps ready
copies. Deletion removes a copy from the registry immediately and persists its
file removal job; a late deletion result cannot remove a newer copy. Network/storage
failures retain jobs for retry. Successful publications from superseded attempts
also enter the durable removal queue; failed cleanup is retried after restart.

## Verification

```sh
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --no-default-features
cargo build --workspace --locked --release
cargo build --workspace --locked --release --no-default-features
cargo test --workspace --locked --features native-tests --lib -- --test-threads=1
```

Native checks require libmpv, TDLib and permission to bind loopback sockets.
Desktop checks run scripts/check-control.py and scripts/check-mpris.py in a graphical
D-Bus session. Capture demo screenshots in both light and dark themes. Default demo
starts no SQLite, Telegram, mpv, tray or MPRIS services; desktop integrations require
--desktop-integrations. Authenticated playback, audible output and tray/window
restoration require manual verification before release.
