use crate::tr;
use tg_player_application::notice::Notice;

pub(crate) fn notice_text(notice: Notice) -> String {
    match notice {
        Notice::Clear => String::new(),
        Notice::Connecting => tr("Connecting…"),
        Notice::LoadingTrack => tr("Loading track…"),
        Notice::DemoTracks => tr("Demo tracks are fictional"),
        Notice::LoadingChats => tr("Loading chat list…"),
        Notice::OpenChatPrompt => tr("Open a chat to load its music"),
        Notice::MusicLoaded => tr("Music loaded"),
        Notice::OfflineDownloading => tr("Downloading offline tracks…"),
        Notice::OfflineInterrupted => tr("Download interrupted"),
        Notice::OfflineSaved => tr("Saved for offline listening"),
    }
}
