use crate::playback::RepeatMode;
pub(crate) fn repeat_from_system(value: fastframe_now_playing::Repeat) -> RepeatMode {
    match value {
        fastframe_now_playing::Repeat::Off => RepeatMode::Off,
        fastframe_now_playing::Repeat::Playlist => RepeatMode::All,
        fastframe_now_playing::Repeat::Track => RepeatMode::One,
    }
}
pub(crate) fn repeat_to_system(value: RepeatMode) -> fastframe_now_playing::Repeat {
    match value {
        RepeatMode::Off => fastframe_now_playing::Repeat::Off,
        RepeatMode::All => fastframe_now_playing::Repeat::Playlist,
        RepeatMode::One => fastframe_now_playing::Repeat::Track,
    }
}
