use super::{telegram::Telegram, telegram_mapping::telegram_int64};
use gramwave_application::AppError;
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn username_uri(username: &str) -> Option<String> {
    (!username.is_empty()
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then(|| format!("tg://resolve?domain={username}"))
}

impl Telegram {
    pub(crate) async fn source_chat_uri(
        &self,
        chat: i64,
        link: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, AppError> {
        if let Some(uri) = link.strip_prefix("https://t.me/").and_then(username_uri) {
            return Ok(Some(uri));
        }
        let response = self
            .call_async(json!({"@type":"getChat","chat_id":chat}), cancel)
            .await?;
        let kind = &response["type"];
        match kind["@type"].as_str() {
            Some("chatTypeSupergroup") => Ok(kind["supergroup_id"]
                .as_i64()
                .filter(|id| *id > 0)
                .map(|id| format!("tg://privatepost?channel={id}"))),
            Some("chatTypePrivate") => {
                let Some(user_id) = kind["user_id"].as_i64().filter(|id| *id > 0) else {
                    return Ok(None);
                };
                let me = self
                    .call_async(json!({"@type":"getOption","name":"my_id"}), cancel)
                    .await?;
                if telegram_int64(&me["value"]) == Some(user_id) {
                    return Ok(Some("tg://settings/saved-messages".into()));
                }
                let user = self
                    .call_async(json!({"@type":"getUser","user_id":user_id}), cancel)
                    .await?;
                Ok(user["usernames"]["active_usernames"][0]
                    .as_str()
                    .and_then(username_uri))
            }
            _ => Ok(None),
        }
    }
}

pub(crate) async fn open(uri: &str) -> bool {
    let mut command = tokio::process::Command::new("gio");
    command
        .args(["open", uri])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(8), command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builds_only_supported_telegram_chat_links() {
        let tg = Telegram::mock_rpc(|request| match request["@type"].as_str() {
            Some("getChat") if request["chat_id"] == 1 => {
                json!({"type":{"@type":"chatTypeSupergroup","supergroup_id":42}})
            }
            Some("getChat") if request["chat_id"] == 2 => {
                json!({"type":{"@type":"chatTypePrivate","user_id":12}})
            }
            Some("getChat") if request["chat_id"] == 4 => {
                json!({"type":{"@type":"chatTypePrivate","user_id":13}})
            }
            Some("getChat") => json!({"type":{"@type":"chatTypeBasicGroup"}}),
            Some("getOption") => json!({"@type":"optionValueInteger","value":"12"}),
            Some("getUser") => {
                json!({"usernames":{"active_usernames":["personal_name"]}})
            }
            _ => unreachable!(),
        });
        let cancel = CancellationToken::new();
        for (chat, link, result) in [
            (1, "", Some("tg://privatepost?channel=42")),
            (2, "", Some("tg://settings/saved-messages")),
            (
                3,
                "https://t.me/Music_42",
                Some("tg://resolve?domain=Music_42"),
            ),
            (3, "https://t.me/foo?text=bar", None),
            (3, "https://example.org/Music_42", None),
            (4, "", Some("tg://resolve?domain=personal_name")),
        ] {
            assert_eq!(
                tg.source_chat_uri(chat, link, &cancel)
                    .await
                    .unwrap()
                    .as_deref(),
                result
            );
        }
    }
}
