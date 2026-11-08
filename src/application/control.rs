//! Small, explicit views and requests for local automation. Never export State.
use super::commands::AppCommand;
use crate::{domain::model::QueueEntryId, App, RepeatMode, Track};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::mpsc, time::Instant};

pub(crate) const REPLY_LIMIT: usize = 12 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Request {
    Status,
    Playlists { offset: usize },
    Playlist { offset: usize },
    Queue { offset: usize },
    Open { chat_id: i64 },
    Select { row: usize },
    QueuePlay { entry_id: QueueEntryId },
    Next,
    Previous,
    Pause,
    Resume,
    Seek { seconds: f64 },
    Shuffle { enabled: bool },
    Repeat { mode: RepeatMode },
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
fn track_view(track: &Track) -> Value {
    json!({
        "chat_id": track.chat, "message_id": track.message,
        "entry_id": track.entry_id, "title": label(&track.title),
        "artist": label(&track.artist), "duration": track.duration,
    })
}
fn page(items: impl ExactSizeIterator<Item = Value>, offset: usize) -> Result<Value, String> {
    let total = items.len();
    if offset > total {
        return Err("Page offset is outside the list".into());
    }
    let mut result = Vec::new();
    let mut bytes = 512; // Envelope and pagination metadata.
    for item in items.skip(offset).take(50) {
        let size = serde_json::to_vec(&item).map_err(|e| e.to_string())?.len() + 1;
        if bytes + size > REPLY_LIMIT {
            break;
        }
        bytes += size;
        result.push(item);
    }
    let next = offset + result.len();
    Ok(json!({"items": result, "total": total, "offset": offset,
        "next_offset": (next < total).then_some(next)}))
}

impl App {
    fn control_status(&self) -> Value {
        let position = self
            .player
            .as_ref()
            .map(|p| p.snapshot().position)
            .unwrap_or(0.);
        json!({
            "current": self.state.view().session.current().map(|t|track_view(&t.into())),
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
            return Err("TG Player is shutting down".into());
        }
        let command = match request {
            Request::Status => return Ok(self.control_status()),
            Request::Playlists { offset } => return page(self.state.view().playlists.iter().map(|p| json!({"chat_id": p.id, "title": label(&p.title), "loaded_tracks": p.tracks.len()})), offset),
            Request::Playlist { offset } => {
                let index = self.control_playlist()?;
                let tracks = self.playlist_tracks(index);
                return page(tracks.iter().enumerate().map(|(i, t)| {
                    let mut view = track_view(t);
                    view["row"] = json!(i + 1);
                    view
                }), offset);
            }
            Request::Queue { offset } => return page(self.state.view().session.remaining().iter().enumerate().map(|(i, t)| {
                let mut view = track_view(&t.into());
                view["row"] = json!(i + 1);
                view
            }), offset),
            Request::Open { chat_id } => {
                if !self.state.view().playlists.iter().any(|p| p.id == chat_id) {
                    return Err("Unknown playlist chat ID".into());
                }
                AppCommand::OpenPlaylist(chat_id)
            }
            Request::Select { row } => {
                let index = self.control_playlist()?;
                if row == 0 || row > self.playlist_tracks(index).len() {
                    return Err("Playlist row is outside the list (rows start at 1)".into());
                }
                AppCommand::SelectPlaylistRow(row - 1)
            }
            Request::QueuePlay { entry_id } => {
                if !self.state.view().session.remaining().iter().any(|t| t.entry_id == entry_id) {
                    return Err("Unknown upcoming queue entry ID; read --ctl queue again".into());
                }
                AppCommand::PlayQueue(entry_id)
            }
            Request::Next => AppCommand::Next,
            Request::Previous => AppCommand::Previous,
            Request::Pause => AppCommand::Pause(true),
            Request::Resume => AppCommand::Pause(false),
            Request::Seek { seconds } => {
                if !seconds.is_finite() || seconds < 0. {
                    return Err("Seek position must be a finite, nonnegative number of seconds".into());
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
