use crate::{adapters::source_chat, tr, App, Event};
use tg_player_application::tasks::JobClass;

impl App {
    pub(crate) fn open_source_chat(&mut self, chat: i64, url: &str) {
        let Some(tg) = self.tg.clone() else {
            self.set_inline_notice(tr("Connect to Telegram to open this chat."), false);
            return;
        };
        let url = url.to_owned();
        self.async_event(
            JobClass::Interactive,
            0,
            async move {
                match tg.source_chat_uri(chat, &url, &Default::default()).await {
                    Ok(Some(uri)) if source_chat::open(&uri).await => Event::Noop,
                    Ok(Some(_)) => Event::SourceChatError(tr(
                        "Could not open Telegram. Check that a Telegram desktop app is installed.",
                    )),
                    Ok(None) => {
                        Event::SourceChatError(tr("Telegram has no direct link for this chat."))
                    }
                    Err(error) => Event::SourceChatError(error.to_string()),
                }
            },
            |error| Event::SourceChatError(error.to_string()),
        );
    }
}
