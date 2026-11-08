use crate::{model::Track, telegram::Telegram};
use tg_player_application::media::MediaTrack;

// File IDs are connection-local; only runtime fields may refresh an existing queue entry.
#[cfg(test)]
#[path = "media_loader/tests.rs"]
mod tests;

pub(crate) async fn resolve_async(
    tg: &Telegram,
    old: &Track,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<MediaTrack, tg_player_application::AppError> {
    use tg_player_application::telegram::TelegramGateway;
    let resolved = tg.message(old.id(), cancel).await?;
    let mut track = MediaTrack::from(old.clone());
    track.file = resolved.file;
    track.cached = resolved.cached;
    track.image_file = resolved.image_file;
    track.image_path = resolved.image_path;
    Ok(track)
}
