use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub entry_id: u64,
    pub chat: i64,
    pub message: i64,
    #[serde(skip)]
    pub file: i64,
    pub title: String,
    pub artist: String,
    pub duration: u64,
    pub kind: usize,
    pub pinned: Option<String>,
    #[serde(skip)]
    pub cached: Option<String>,
    #[serde(skip)]
    pub image_file: i64,
    #[serde(skip)]
    pub image_path: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub id: i64,
    pub title: String,
    pub url: String,
    #[serde(skip)]
    pub image_file: i64,
    #[serde(skip)]
    pub image_path: Option<String>,
    pub tracks: Vec<Track>,
    pub filters: [bool; 3],
    pub pinned: bool,
    pub cursors: [i64; 3],
    pub done: [bool; 3],
}
impl Playlist {
    pub fn merge_tracks(&mut self, tracks: Vec<Track>, protected: &std::collections::HashSet<i64>) {
        for mut t in tracks {
            if protected.contains(&t.message) {
                continue;
            }
            if let Some(old) = self.tracks.iter_mut().find(|old| old.message == t.message) {
                t.pinned = old.pinned.clone();
                if t.file == 0 {
                    t.file = old.file;
                    t.cached = old.cached.clone();
                    t.image_file = old.image_file;
                }
                if t.image_file == old.image_file {
                    t.image_path = t.image_path.or(old.image_path.clone());
                }
                if *old != t {
                    *old = t;
                }
            } else {
                self.tracks.push(t);
            }
        }
        self.tracks.sort_by_key(|t| std::cmp::Reverse(t.message));
    }
    pub fn recover_empty_index(&mut self) {
        // Older versions could drop cached message metadata while keeping the
        // pagination cursors. Start over only when opening that empty library.
        if self.tracks.is_empty() && self.cursors.iter().any(|c| *c != 0) {
            self.cursors = [0; 3];
            self.done = [false; 3];
        }
    }
    pub fn has_more(&self) -> bool {
        self.filters
            .iter()
            .zip(self.done)
            .any(|(enabled, done)| *enabled && !done)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub playlists: Vec<Playlist>,
    pub chat_pins: Vec<i64>,
    // Legacy queue remains the explicit manual queue.
    pub queue: Vec<Track>,
    #[serde(default)]
    pub queue_version: u8,
    pub previous_queue: Vec<Track>,
    pub session: crate::playback::Session,
    pub repeat: crate::playback::RepeatMode,
    pub shuffle: bool,
    pub volume: f32,
    pub unmuted_volume: f32,
    pub api_id: String,
    pub api_hash: String,
    pub light: bool,
    pub pending_pins: Vec<Track>,
    #[serde(default = "default_filters")]
    pub filters: [bool; 3],
}
impl Default for State {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            chat_pins: Vec::new(),
            queue: Vec::new(),
            queue_version: 1,
            previous_queue: Vec::new(),
            session: Default::default(),
            repeat: Default::default(),
            shuffle: false,
            volume: 75.,
            unmuted_volume: 75.,
            api_id: String::new(),
            api_hash: String::new(),
            light: false,
            pending_pins: Vec::new(),
            filters: default_filters(),
        }
    }
}
impl State {
    /// Older releases stored the automatic tail in `queue`; the first session
    /// release misclassified it as manual. Repair only a substantial, exact
    /// suffix already present in the automatic continuation. Keep a backup.
    pub fn repair_legacy_queue(&mut self) -> bool {
        if self.queue_version >= 1 {
            return false;
        }
        self.queue_version = 1;
        let count = self.queue.len();
        if count < 8 || count > self.session.snapshot.len() {
            return false;
        }
        let suffix = &self.session.snapshot[self.session.snapshot.len() - count..];
        let automatic = &self.session.remaining;
        if self
            .queue
            .iter()
            .zip(suffix)
            .all(|(a, b)| crate::playback::same(a, b))
            && self
                .queue
                .iter()
                .all(|q| automatic.iter().any(|a| crate::playback::same(q, a)))
        {
            self.previous_queue = std::mem::take(&mut self.queue);
            return true;
        }
        false
    }

    pub fn apply_message_deletions(&mut self, update: &serde_json::Value) {
        // TDLib also sends updateDeleteMessages when evicting its own cache.
        // The source messages still exist and our playlist metadata stays valid.
        if update["from_cache"] == true {
            return;
        }
        if let Some(p) = self
            .playlists
            .iter_mut()
            .find(|p| Some(p.id) == update["chat_id"].as_i64())
        {
            p.tracks.retain(|t| {
                !update["message_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&serde_json::json!(t.message)))
            });
        }
    }
    pub fn sidebar_chats(&self, searching: bool, query: &str) -> Vec<usize> {
        if searching {
            let query = query.trim().to_lowercase();
            self.playlists
                .iter()
                .enumerate()
                .filter(|(_, p)| p.title.to_lowercase().contains(&query))
                .map(|(i, _)| i)
                .collect()
        } else {
            self.chat_pins
                .iter()
                .filter_map(|id| self.playlists.iter().position(|p| p.id == *id))
                .collect()
        }
    }
    pub fn toggle_chat_pin(&mut self, id: i64) {
        if self.chat_pins.contains(&id) {
            self.chat_pins.retain(|x| *x != id);
        } else {
            self.chat_pins.push(id);
        }
    }
}
pub fn audio_document(name: &str, mime: &str) -> bool {
    mime.starts_with("audio/")
        || [
            "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "aiff", "alac",
        ]
        .iter()
        .any(|e| name.to_lowercase().ends_with(&format!(".{e}")))
}
pub fn track(chat: i64, v: &serde_json::Value) -> Option<Track> {
    let c = &v["content"];
    let ty = c["@type"].as_str()?;
    let (kind, a) = match ty {
        "messageAudio" => (0, &c["audio"]),
        "messageDocument" => {
            let d = &c["document"];
            if !audio_document(
                d["file_name"].as_str().unwrap_or(""),
                d["mime_type"].as_str().unwrap_or(""),
            ) {
                return None;
            }
            (1, d)
        }
        "messageVoiceNote" => (2, &c["voice_note"]),
        _ => return None,
    };
    let f = match kind {
        0 => &a["audio"],
        1 => &a["document"],
        _ => &a["voice"],
    };
    let title = a["title"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| a["file_name"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| crate::i18n::tr("Voice message"));
    Some(Track {
        entry_id: 0,
        chat,
        message: v["id"].as_i64()?,
        file: f["id"].as_i64()?,
        title,
        artist: a["performer"].as_str().unwrap_or("").into(),
        duration: a["duration"].as_u64().unwrap_or(0),
        kind,
        pinned: None,
        cached: local_path(f),
        image_file: a["album_cover_thumbnail"]["file"]["id"]
            .as_i64()
            .or_else(|| a["thumbnail"]["file"]["id"].as_i64())
            .unwrap_or(0),
        image_path: local_path(&a["album_cover_thumbnail"]["file"])
            .or_else(|| local_path(&a["thumbnail"]["file"])),
    })
}
fn local_path(file: &serde_json::Value) -> Option<String> {
    (file["local"]["is_downloading_completed"] == true)
        .then(|| {
            file["local"]["path"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
        })
        .flatten()
}
pub fn default_filters() -> [bool; 3] {
    [true, true, false]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_eviction_keeps_second_pinned_playlist() {
        let playlist = |id| Playlist {
            id,
            tracks: vec![Track {
                chat: id,
                message: 10,
                ..Default::default()
            }],
            filters: default_filters(),
            cursors: [10, 0, 0],
            done: [true; 3],
            ..Default::default()
        };
        let mut state = State {
            playlists: vec![playlist(1), playlist(2)],
            chat_pins: vec![1, 2],
            ..Default::default()
        };
        state.apply_message_deletions(&serde_json::json!({
            "@type":"updateDeleteMessages", "chat_id":2, "message_ids":[10],
            "is_permanent":false, "from_cache":true
        }));
        assert_eq!(state.playlists[1].tracks.len(), 1);
        assert_eq!(state.playlists[1].cursors, [10, 0, 0]);
        assert_eq!(state.sidebar_chats(false, ""), vec![0, 1]);
        // Actual source deletion affects only the corresponding chat, even
        // when both playlists contain the same message ID.
        state.apply_message_deletions(&serde_json::json!({
            "@type":"updateDeleteMessages", "chat_id":2, "message_ids":[10],
            "is_permanent":true, "from_cache":false
        }));
        assert!(state.playlists[1].tracks.is_empty());
        assert_eq!(state.playlists[0].tracks.len(), 1);
    }

    #[test]
    fn opening_nonempty_index_preserves_pagination() {
        let mut p = Playlist {
            tracks: vec![Track::default()],
            cursors: [10, 20, 0],
            done: [true, true, false],
            ..Default::default()
        };
        p.recover_empty_index();
        assert_eq!(p.cursors, [10, 20, 0]);
        assert_eq!(p.done, [true, true, false]);
    }
    #[test]
    fn document_filter() {
        assert!(audio_document("Song.FLAC", "application/octet-stream"));
        assert!(!audio_document("movie.mp4", "video/mp4"));
    }
    #[test]
    fn message_identity() {
        let v = serde_json::json!({"id":123,"content":{"@type":"messageAudio","audio":{"title":"Song","performer":"Artist","audio":{"id":8}}}});
        let t = track(7, &v).unwrap();
        assert_eq!((t.chat, t.message, t.file), (7, 123, 8));
    }
}

#[cfg(test)]
mod pin_tests {
    use super::*;
    fn library() -> State {
        State {
            playlists: vec![
                Playlist {
                    id: 1,
                    title: "Ambient".into(),
                    ..Default::default()
                },
                Playlist {
                    id: 2,
                    title: "Музыка".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn sidebar_switches_between_pins_and_search() {
        let mut s = library();
        assert!(s.sidebar_chats(false, "").is_empty());
        assert_eq!(s.sidebar_chats(true, ""), vec![0, 1]);
        s.toggle_chat_pin(2);
        assert_eq!(s.sidebar_chats(false, ""), vec![1]);
        assert_eq!(s.sidebar_chats(true, "МУЗ"), vec![1]);
        assert_eq!(s.sidebar_chats(true, "amB"), vec![0]);
        s.toggle_chat_pin(2);
        assert!(s.sidebar_chats(false, "").is_empty());
    }
    #[test]
    fn pinning_does_not_enable_offline_downloads() {
        let mut s = library();
        s.toggle_chat_pin(1);
        assert!(!s.playlists[0].pinned);
        let restored: State = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(restored.chat_pins, vec![1]);
        let old: State = serde_json::from_str(r#"{"playlists":[{"id":1,"pinned":true}]}"#).unwrap();
        assert!(old.chat_pins.is_empty());
        assert!(old.playlists[0].pinned);
    }
}

#[cfg(test)]
mod queue_repair_tests {
    use super::*;
    fn old_state() -> State {
        let tracks: Vec<_> = (0..131)
            .map(|message| Track {
                chat: 7,
                message,
                ..Default::default()
            })
            .collect();
        let mut state = State {
            queue_version: 0,
            queue: tracks[20..].to_vec(),
            ..Default::default()
        };
        state.session.start(&tracks, 0, false);
        state.session.remaining.reverse();
        state
    }
    #[test]
    fn repairs_duplicated_legacy_tail_without_losing_tracks_or_order() {
        let mut state = old_state();
        let before = state.session.remaining.clone();
        let legacy = state.queue.clone();
        assert!(state.repair_legacy_queue());
        assert!(state.queue.is_empty());
        assert_eq!(state.session.remaining, before);
        assert_eq!(state.previous_queue, legacy);
        state.session.reorder(false);
        assert_eq!(state.session.remaining[0].message, 1);
        assert_eq!(state.session.remaining.last().unwrap().message, 130);
        assert!(!state.repair_legacy_queue());
        let saved: State = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(saved.previous_queue, legacy);
        assert_eq!(saved.queue_version, 1);
    }
    #[test]
    fn leaves_explicit_or_nonduplicated_queues_untouched() {
        let mut state = old_state();
        state.queue.swap(0, 1);
        let queue = state.queue.clone();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue, queue);
        let mut state = old_state();
        state.session.remaining.clear();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 111);
        let mut state = old_state();
        state.queue_version = 1;
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 111);
        let mut state = old_state();
        state.queue = state.queue[105..].to_vec();
        assert!(!state.repair_legacy_queue());
        assert_eq!(state.queue.len(), 6);
    }
    #[test]
    fn missing_version_is_detected_but_new_queues_are_versioned() {
        let old: State = serde_json::from_str(r#"{"queue":[]}"#).unwrap();
        assert_eq!(old.queue_version, 0);
        assert_eq!(State::default().queue_version, 1);
    }
}
