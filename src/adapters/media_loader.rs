use crate::{adapters::telegram_mapping::track, model::Track, telegram::Telegram};
use serde_json::json;

// TDLib's numeric file IDs belong to a running client. Resolve persisted
// message identities before using files in a new client instance.
pub fn resolve(tg: &Telegram, old: &Track) -> Result<Track, String> {
    let message = tg.call(json!({
        "@type": "getMessage", "chat_id": old.chat, "message_id": old.message
    }))?;
    let mut resolved = track(old.chat, &message)
        .filter(|t| t.message == old.message && t.file > 0)
        .ok_or_else(|| crate::tr("This message no longer contains playable audio"))?;
    resolved.entry_id = old.entry_id;
    resolved.pinned = old.pinned.clone();
    Ok(resolved)
}

#[cfg(test)]
#[path = "media_loader/tests.rs"]
mod tests;
