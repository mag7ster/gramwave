use crate::playback::RepeatMode;
impl From<fastframe_now_playing::Repeat> for RepeatMode {
    fn from(value: fastframe_now_playing::Repeat) -> Self {
        match value {
            fastframe_now_playing::Repeat::Off => Self::Off,
            fastframe_now_playing::Repeat::Playlist => Self::All,
            fastframe_now_playing::Repeat::Track => Self::One,
        }
    }
}
impl From<RepeatMode> for fastframe_now_playing::Repeat {
    fn from(value: RepeatMode) -> Self {
        match value {
            RepeatMode::Off => Self::Off,
            RepeatMode::All => Self::Playlist,
            RepeatMode::One => Self::Track,
        }
    }
}
