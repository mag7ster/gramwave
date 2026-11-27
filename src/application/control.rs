//! Small, explicit views and requests for local automation. Never export State.
use super::commands::AppCommand;
use crate::{
    domain::{
        model::{QueueEntry, QueueEntryId},
        playback::QueueRangeEdit,
    },
    launch::ProcessIdentity,
    tr, App, Playlist, RepeatMode, Track,
};
use gramwave_application::{library::LibraryCommand, telegram::AuthorizationState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeSet, ops::Range, sync::mpsc, time::Instant};

pub(crate) const REPLY_LIMIT: usize = 12 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Request {
    Instance,
    Quit {
        identity: ProcessIdentity,
    },
    Status,
    Playlists {
        offset: usize,
    },
    Playlist {
        offset: usize,
    },
    Queue {
        offset: i64,
    },
    QueueAdd {
        rows: Vec<RowRange>,
        next: bool,
        reverse: bool,
    },
    QueueRemove {
        rows: Vec<RowRange>,
    },
    QueueReverse {
        rows: RowRange,
    },
    Search {
        query: String,
        offset: usize,
    },
    Pinned {
        offset: usize,
    },
    Pin {
        chat_id: i64,
        pinned: bool,
    },
    Open {
        chat_id: i64,
    },
    Select {
        row: usize,
    },
    QueuePlay {
        entry_id: QueueEntryId,
    },
    QueuePlayRow {
        row: usize,
    },
    Next,
    Previous,
    Pause,
    Resume,
    Seek {
        seconds: f64,
    },
    Shuffle {
        enabled: bool,
    },
    Repeat {
        mode: RepeatMode,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowRange {
    pub first: usize,
    pub last: usize,
}

impl RowRange {
    fn indices(&self, len: usize) -> Result<Range<usize>, String> {
        if self.first == 0 || self.first > self.last || self.last > len {
            return Err(tr("Row or range is outside the list"));
        }
        Ok(self.first - 1..self.last)
    }
}

fn row_ranges(rows: &[RowRange], len: usize) -> Result<Vec<Range<usize>>, String> {
    if rows.is_empty() {
        return Err(tr("Specify at least one row or range"));
    }
    rows.iter().map(|span| span.indices(len)).collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Reply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl Reply {
    pub fn success(data: Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(label(&error.into())),
        }
    }
}

pub(crate) struct Pending {
    pub request: Request,
    pub deadline: Instant,
    pub reply: mpsc::SyncSender<Reply>,
}

// Bound display strings as well as page sizes, including unusually large or
// heavily escaped Telegram metadata. IDs and selection remain exact.
fn label(text: &str) -> String {
    text.chars().take(512).collect()
}
fn track_view(track: &Track, max_chars: usize) -> Value {
    json!({
        "chat_id": track.chat, "message_id": track.message,
        "entry_id": track.entry_id, "title": track.title.chars().take(max_chars).collect::<String>(),
        "artist": track.artist.chars().take(max_chars).collect::<String>(), "duration": track.duration,
    })
}
fn queue_row(index: usize, entry: &QueueEntry, max_chars: usize) -> Value {
    let mut view = track_view(&entry.into(), max_chars);
    view["row"] = json!(index + 1);
    view
}
fn chat_view(chat: &Playlist) -> Value {
    json!({"chat_id": chat.id, "title": label(&chat.title), "loaded_tracks": chat.tracks.len()})
}
fn page(items: impl ExactSizeIterator<Item = Value>, offset: usize) -> Result<Value, String> {
    paginate(items, offset, json!({}))
}

fn paginate(
    items: impl ExactSizeIterator<Item = Value>,
    offset: usize,
    mut data: Value,
) -> Result<Value, String> {
    let total = items.len();
    if offset > total {
        return Err("Page offset is outside the list".into());
    }
    let mut result = Vec::new();
    let mut bytes = 510 + data.to_string().len(); // 512 bytes for envelope and pagination.
    for item in items.skip(offset).take(50) {
        let size = serde_json::to_vec(&item).map_err(|e| e.to_string())?.len() + 1;
        if bytes + size > REPLY_LIMIT {
            break;
        }
        bytes += size;
        result.push(item);
    }
    let next = offset + result.len();
    data["items"] = json!(result);
    data["total"] = json!(total);
    data["offset"] = json!(offset);
    data["next_offset"] = json!((next < total).then_some(next));
    Ok(data)
}

impl App {
    fn control_queue(&self, offset: i64) -> Result<Value, String> {
        let timeline = self.state.view().session.timeline();
        // Leave room for an upcoming row even when both titles are heavily escaped.
        let current = timeline
            .current_index()
            .map(|index| queue_row(index, timeline.get(index).unwrap(), 256));
        let (rows, start) = if offset < 0 {
            let past = timeline.past_len();
            let depth = usize::try_from(offset.unsigned_abs()).unwrap_or(usize::MAX);
            (0..past, past.saturating_sub(depth))
        } else {
            let start =
                usize::try_from(offset).map_err(|_| tr("Page offset is outside the list"))?;
            (timeline.upcoming_start()..timeline.len(), start)
        };
        let page_len = rows.len();
        let mut data = paginate(
            rows.map(|index| queue_row(index, timeline.get(index).unwrap(), 512)),
            start,
            json!({"current": current}),
        )?;
        if offset < 0 {
            data["offset"] = json!(offset);
            if let Some(next) = data["next_offset"].as_u64() {
                data["next_offset"] = json!(-((page_len - next as usize) as i64));
            }
        }
        Ok(data)
    }

    fn control_status(&self) -> Value {
        let snapshot = self
            .player
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or_default();
        let position = self
            .restored_position(snapshot)
            .unwrap_or(snapshot.position);
        json!({
            "current": self.state.view().session.current().map(|t|track_view(&t.into(), 512)),
            "position": if position.is_finite() { position.max(0.) } else { 0. },
            "paused": self.state.playback().state().paused(), "loading": self.state.playback().state().loading(),
            "shuffle": self.state.view().shuffle,
            "repeat": match self.state.view().repeat { RepeatMode::Off => "off", RepeatMode::All => "all", RepeatMode::One => "one" },
            "opened_chat": self.opened_chat, "queue_length": self.state.view().session.remaining().len(),
            "error": label(&self.error),
        })
    }
    fn control_playlist(&self) -> Result<usize, String> {
        self.opened_chat
            .and_then(|id| self.state.view().playlists.iter().position(|p| p.id == id))
            .ok_or_else(|| {
                "No playlist is open; use --ctl playlists and --ctl open <chat-id>".into()
            })
    }
    fn apply_control(&mut self, request: Request) -> Result<Value, String> {
        if self.quitting {
            return Err(format!("{} is shutting down", crate::identity::APP_NAME));
        }
        let command = match request {
            Request::Instance => return Ok(json!(ProcessIdentity::current()?)),
            Request::Quit { identity } => {
                if identity != ProcessIdentity::current()? {
                    return Err(tr("The player instance changed before it could be stopped"));
                }
                self.execute(AppCommand::Quit);
                return Ok(json!({"accepted": true}));
            }
            Request::Status => return Ok(self.control_status()),
            Request::Playlists { offset } => {
                return page(self.state.view().playlists.iter().map(chat_view), offset)
            }
            Request::Pinned { offset } => {
                let indices = self.chat_search_indices(false, "");
                return page(
                    indices
                        .into_iter()
                        .map(|i| chat_view(&self.state.view().playlists[i])),
                    offset,
                );
            }
            Request::Search { query, offset } => return self.control_search(query, offset),
            Request::Pin { chat_id, pinned } => {
                let state = self.state.view();
                let chat = state
                    .playlists
                    .iter()
                    .find(|p| p.id == chat_id)
                    .filter(|_| {
                        !gramwave_application::telegram::is_channel_direct_messages(chat_id)
                    })
                    .ok_or_else(|| tr("Unknown playlist chat ID"))?;
                let mut data = chat_view(chat);
                if state.chat_pins.contains(&chat_id) != pinned {
                    self.toggle_chat_pin(chat_id);
                }
                data["pinned"] = json!(pinned);
                return Ok(data);
            }
            Request::Playlist { offset } => {
                let index = self.control_playlist()?;
                let tracks = self.playlist_tracks(index);
                return page(
                    tracks.iter().enumerate().map(|(i, t)| {
                        let mut view = track_view(t, 512);
                        view["row"] = json!(i + 1);
                        view
                    }),
                    offset,
                );
            }
            Request::Queue { offset } => return self.control_queue(offset),
            Request::QueueAdd {
                rows,
                next,
                reverse,
            } => {
                let tracks = self.playlist_tracks(self.control_playlist()?);
                let ranges = row_ranges(&rows, tracks.len())?;
                let mut tracks: Vec<_> = ranges
                    .into_iter()
                    .flatten()
                    .map(|index| tracks[index].metadata())
                    .collect();
                if reverse {
                    tracks.reverse();
                }
                AppCommand::InsertTracks(tracks, next)
            }
            Request::QueueRemove { rows } => {
                let timeline = self.state.view().session.timeline();
                let indices: BTreeSet<_> = row_ranges(&rows, timeline.len())?
                    .into_iter()
                    .flatten()
                    .collect();
                AppCommand::RemoveQueueRows {
                    expected: indices
                        .into_iter()
                        .map(|index| (index, timeline.get(index).unwrap().entry_id))
                        .collect(),
                    now: Instant::now(),
                }
            }
            Request::QueueReverse { rows } => {
                let timeline = self.state.view().session.timeline();
                let range = rows.indices(timeline.len())?;
                if range.len() < 2 {
                    return Err(tr("Specify a range of at least two queue rows"));
                }
                AppCommand::EditQueueRange {
                    start: range.start,
                    expected: range
                        .map(|index| timeline.get(index).unwrap().entry_id)
                        .collect(),
                    edit: QueueRangeEdit::Reverse,
                }
            }
            Request::Open { chat_id } => {
                if !self.state.view().playlists.iter().any(|p| p.id == chat_id) {
                    return Err("Unknown playlist chat ID".into());
                }
                let from_search = self.chat_search_active
                    && self
                        .chat_search_indices(true, self.chat_query.trim())
                        .into_iter()
                        .any(|i| self.state.view().playlists[i].id == chat_id);
                if from_search {
                    self.chat_search_active = false;
                    self.chat_query.clear();
                    self.pane = crate::screens::WorkspacePane::Playlist;
                    self.library_command(LibraryCommand::Search {
                        query: String::new(),
                        now: Instant::now(),
                        force: false,
                    });
                    AppCommand::OpenSearchPlaylist(chat_id)
                } else {
                    AppCommand::OpenPlaylist(chat_id)
                }
            }
            Request::Select { row } => {
                let index = self.control_playlist()?;
                if row == 0 || row > self.playlist_tracks(index).len() {
                    return Err("Playlist row is outside the list (rows start at 1)".into());
                }
                AppCommand::SelectPlaylistRow(row - 1)
            }
            Request::QueuePlay { entry_id } => {
                if !self
                    .state
                    .view()
                    .session
                    .remaining()
                    .iter()
                    .any(|t| t.entry_id == entry_id)
                {
                    return Err("Unknown upcoming queue entry ID; read --ctl queue again".into());
                }
                AppCommand::PlayQueue(entry_id)
            }
            Request::QueuePlayRow { row } => {
                let timeline = self.state.view().session.timeline();
                let (index, entry) = row
                    .checked_sub(1)
                    .and_then(|index| timeline.get(index).map(|entry| (index, entry)))
                    .ok_or_else(|| tr("Queue row is outside the list"))?;
                if timeline.current_index() == Some(index) {
                    return Ok(self.control_status());
                }
                AppCommand::PlayQueueRow {
                    index,
                    entry_id: entry.entry_id,
                }
            }
            Request::Next => AppCommand::Next,
            Request::Previous => AppCommand::Previous,
            Request::Pause => AppCommand::Pause(true),
            Request::Resume => AppCommand::Pause(false),
            Request::Seek { seconds } => {
                if !seconds.is_finite() || seconds < 0. {
                    return Err(
                        "Seek position must be a finite, nonnegative number of seconds".into(),
                    );
                }
                if self.state.playback().state().loading() {
                    return Err("Track is still loading".into());
                }
                AppCommand::Seek(seconds)
            }
            Request::Shuffle { enabled } => AppCommand::Shuffle(enabled),
            Request::Repeat { mode } => AppCommand::Repeat(mode),
        };
        self.execute(command);
        Ok(self.control_status())
    }
    fn control_search(&mut self, query: String, offset: usize) -> Result<Value, String> {
        let query = query.trim();
        if query.is_empty() {
            return Err(tr("Enter a chat name to search for"));
        }
        self.chat_query = query.to_owned();
        self.chat_search_active = true;
        let search = self.state.library().search();
        if *self.state.auth().state() == AuthorizationState::Ready
            && (search.query != query || !search.sent)
        {
            self.library_command(LibraryCommand::Search {
                query: query.to_owned(),
                now: Instant::now(),
                force: true,
            });
        }
        let indices = self.chat_search_indices(true, query);
        let search = self.state.library().search();
        let mut data = json!({"loading": search.query == query && search.loading()});
        if let Some(error) = search.error.as_ref().filter(|_| search.query == query) {
            data["search_error"] = json!(label(&error.to_string()));
        }
        paginate(
            indices
                .into_iter()
                .map(|i| chat_view(&self.state.view().playlists[i])),
            offset,
            data,
        )
    }
    pub(crate) fn handle_control(&mut self, request: Request) -> Reply {
        match self.apply_control(request) {
            Ok(data) => Reply::success(data),
            Err(error) => Reply::failure(error),
        }
    }
    pub(crate) fn process_control(&mut self) {
        let pending: Vec<_> = self
            .control
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for pending in pending {
            let reply = if Instant::now() >= pending.deadline {
                Reply::failure("Control request expired before it could be applied")
            } else {
                self.handle_control(pending.request)
            };
            let _ = pending.reply.send(reply);
        }
    }
}

#[cfg(test)]
#[path = "control/tests.rs"]
mod tests;
