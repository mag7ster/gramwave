use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
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
    pub queue: Vec<Track>,
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
            api_id: String::new(),
            api_hash: String::new(),
            light: false,
            pending_pins: Vec::new(),
            filters: default_filters(),
        }
    }
}
impl State {
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
