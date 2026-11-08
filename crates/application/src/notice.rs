//! Semantic status messages; the desktop supplies their translated presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    Clear,
    Connecting,
    LoadingTrack,
    DemoTracks,
    LoadingChats,
    OpenChatPrompt,
    MusicLoaded,
    OfflineDownloading,
    OfflineInterrupted,
    OfflineSaved,
}
