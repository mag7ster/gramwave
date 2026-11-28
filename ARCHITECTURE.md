# Gramwave architecture

The Cargo workspace has three packages. `gramwave-domain` depends only on serde
and rand. `gramwave-application` depends on domain and supplies commands, ports,
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
filtered list before submitting the session commands also used by CLI,
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

On normal exit the app captures the playback position before stopping the player
and saves it with the queue in the session document. The session restores paused
and Play loads the current occurrence at its saved position. Startup selection
can use either the last opened playlist or the chat of the last loaded track.
The latter is persisted separately from the session so stopping playback keeps
the startup choice available.

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
The UI processes bounded batches and caches filtered playlist metadata by
library version, filters, search and chronological order. Visible rows resolve
current media and offline state when drawn. The desktop prioritizes visible
covers, the current and upcoming tracks, and the chat under the pointer.
Chat search warms avatars near the viewport first, then the remaining results
in bounded batches. Pinned and recent chats warm their first screen. A cold chat can share the
first page of its track list between hover and open. The desktop bounds
thumbnail downloads and decoded textures, and keeps a verified index of local
TDLib thumbnail paths for repeat launches.
Producers publish before waking the desktop shell. The main loop waits for
events or the next deadline for playback position, search, retries or visible
timed UI. On Linux the same waker interrupts the shell's wait in the tray.
The `--headless` launch starts in that loop. `--daemon` runs the same binary in
a separate session and checks that the child can answer control requests before
returning. The local socket carries typed requests encoded as JSON. The CLI
formats replies for the terminal or prints JSON when given `--json`.
`--ctl quit` uses the normal shutdown path and waits for the identified
process to exit.

SQLite, TDLib receiving and audio output have dedicated owner threads. Telegram's managed
transport uses oneshot responses, a 60-second RPC deadline and short receive
intervals. Dropping or cancelling a future removes its registry entry immediately;
late replies are discarded. Gateway requests/updates and auth operations are typed.
JSON and the C API stay in adapters; tdlib-rs 1.4.0 remains pinned and supplies API
types. A closed allowlist preserves read-only behavior and online=false.

The `audio` feature uses Symphonia for audio decoding, libopus for Opus,
and Rodio over CPAL for output. Decoder workers read network data, resample
PCM with Rubato when the device rate differs, and fill bounded PCM rings
outside the audio callback. One persistent Rodio source
combines tracks in the output device's format. Generations fence stale EOF
and worker results after Stop, Seek or a new load.

HTTP uses axum on 127.0.0.1 with a random port and a getrandom token. GET and HEAD
support single byte ranges via http-range-header. Unsatisfiable ranges return 416;
multiple ranges are ignored with a full 200 response. HEAD never downloads a body.
Four transfers may be active with sixteen additional admitted requests. Permits
live with response bodies. Partial-file reads observe cancellation and a 30-second
no-progress timeout. Each body keeps its file open until TDLib changes the path.
Latest-state subscriptions to TDLib file updates cover the known contiguous
downloaded range. A prefix query checks other offsets and backs up waits for
missing data. Download leases coordinate streaming and offline consumers. Release
cancels the TDLib transfer only when its last consumer disappears.

TDLib's temporary media cache is measured and trimmed through its storage API.
The desktop serializes cleanup with statistics reads. Audio and artwork use
separate cache leases. Manual cleanup runs available categories and retains the
rest until their leases and any active Telegram stream release access. The decoder keeps
an opened local file usable while its cache path is removed. SQLite,
authorization data and published offline copies are outside the cleanup scope. An unpinned file can remain in the
automatic audio cache at the same path. Gramwave accounts for these files by
chat and reserves their size before applying the remaining limit to TDLib.
Custom emoji use the same temporary media cache.
Caption entities and chat status IDs are persisted alongside library metadata.
The desktop decodes emoji off the UI thread and repaints visible animations.

Shutdown stops commands, cancels operations, stops streaming and audio output, closes Telegram,
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
Removing playlist downloads cancels that chat's scan and downloads and clears
its policy. A recent file changes to cached status in its existing record.
Retention uses the last successful download or playback load, and promoting a
cached file back to saved status uses its existing bytes. Once the publication
and its record are durable, a background handoff releases matching temporary
bytes through TDLib's local file cache API after active readers finish.
Cancellation stops scans and pending downloads, disables policies and keeps ready
copies. Purging removes the copy from the registry immediately and persists its
file removal job; a late deletion result cannot remove a newer copy. Network/storage
failures retain jobs for retry. Successful publications from superseded attempts
also enter the durable removal queue; failed cleanup is retried after restart.

## Local build variants

The [default build](README.md#build-and-run) uses Vulkan and downloads TDLib.
Set `GRAMWAVE_TELEGRAM_CONFIG` at build time to enable Telegram sign-in in either
of these variants.

- OpenGL with downloaded TDLib uses
  `cargo build --release --locked --no-default-features --features audio,webm-emoji,download-tdlib`.
- With an installed TDLib 1.8.61, set `GRAMWAVE_TDLIB_DIR` to its installation
  prefix and run `cargo build --release --locked --no-default-features --features audio,webm-emoji,telegram,vulkan`.

Source chat and message links use `gio` and a registered `tg://` handler such as
Telegram Desktop. Transparent video emoji use webm-iterable for the container
and rusty_vp9 for the colour stream and the alpha stream in BlockAdditional ID 1.
They fall back to a thumbnail or the associated Unicode emoji on decode failure.
The decoder checks VP9 dimensions before allocating planes, including hidden
frames, and preserves the renderer's input, frame, time and cancellation limits.

### Streaming and queue transitions

A session started from a chat stores its media cursors, buffered track metadata
and manually appended occurrence IDs in the session document. Playback requests
pages independently of the displayed chat and merges the media filters by message
ID. Oldest-first browsing and playback use verified windows from the old end of
the same filtered TDLib search. A regular descending search checks the interval
covered by each window and supplies a fallback when the old boundary cannot be
confirmed. The library stores independent cursors and marks a filter complete
when the two indexed ranges meet. Tracks added to the end follow the source history. Play Next stays ahead of
the history boundary. The application waits at the boundary and uses the normal
playback preparation path when a page is ready. Existing sessions without
continuation data restore as finite queues.

Scrolling near the loaded boundary requests one page of queue or playlist
metadata. A temporary failure keeps that request eligible for a timed retry
while the viewport stays near the boundary. Playback can keep requesting its
required queue page when the queue pane is closed.

Opening an incomplete playlist prepares a small shuffle sample using TDLib
media positions and one batch of message metadata. This request leaves the
sequential page cursors at their existing positions. The session tracks sampled
occurrences until sequential loading reaches them. Turning shuffle off restores the original order around the current track. If
the current sample is beyond the main cursor, an ordered window loads
contiguous neighbors in both directions without advancing that cursor.

The transport's reverse action reverses the timeline around the current
occurrence. When the source history is incomplete, the action first loads the
remaining metadata. The current audio keeps its position. An advancing request
waits until the reverse can apply to the whole timeline.

The application reserves the next queue occurrence and resolves its local path
or loopback stream during playback. Speculative Telegram reads use lower
priority and ranges of up to 1 MiB. Once the next track has PCM available,
Rodio can switch at a frame boundary on natural EOF or manual Next. If the
matching stream still needs bytes, it continues at playback priority.
A manual promotion is addressed to its request and advances the queue once.

CBR MP3 with a valid Info header can seek directly to an undownloaded range.
VBR or unclassified MP3 uses accurate frame scanning, which can wait for the
intervening bytes. The saved position updates after PCM at the requested
position is ready. Native tests cover a split-tone VBR track, direct CBR range
requests and cancellation.

Ogg can open from its initial pages. The local Cargo patch pins the ISO/MP4
demuxer from Symphonia 0.6.1 and skips known-size media payloads during the
index scan. This lets M4A preparation fetch a trailing index independently
while intervening audio is still unavailable. Remove the patch when the
published demuxer passes `m4a_prefetch_reads_the_index_from_the_tail`.

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

Native checks use the platform audio development libraries, TDLib with default
features, and permission to bind loopback sockets. The opt-in CPAL test also
needs an output device or an ALSA null device.
Desktop checks run scripts/check-control.py and scripts/check-mpris.py in a graphical
D-Bus session. Run `scripts/check-control.py --headless` to exercise the CLI and
detached startup without a display. Capture demo screenshots in both light and dark themes. Default demo
starts no SQLite, Telegram, audio output, tray or MPRIS services; desktop integrations require
--desktop-integrations. Authenticated playback, audible output and tray/window
restoration require manual verification before release.
