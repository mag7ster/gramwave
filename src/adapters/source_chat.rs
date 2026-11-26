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

fn message_uri(link: &str) -> Option<String> {
    let link = url::Url::parse(link).ok()?;
    if link.scheme() != "https"
        || link.host_str() != Some("t.me")
        || !link.username().is_empty()
        || link.password().is_some()
        || link.port().is_some()
    {
        return None;
    }
    let mut path = link.path_segments()?;
    let user = path.next()?;
    let positive_id = |value: &str| value.parse::<u64>().is_ok_and(|id| id > 0);
    let base = if user == "c" {
        let channel = path.next().filter(|value| positive_id(value))?;
        format!("tg://privatepost?channel={channel}")
    } else {
        username_uri(user)?
    };
    let first = path.next().filter(|value| positive_id(value))?;
    let (post, thread) = match path.next() {
        Some(post) if positive_id(post) && path.next().is_none() => (post, Some(first)),
        Some(_) => return None,
        None => (first, None),
    };
    let mut uri = url::Url::parse(&base).ok()?;
    {
        let mut query = uri.query_pairs_mut();
        query.append_pair("post", post);
        if let Some(thread) = thread {
            query.append_pair("thread", thread);
        }
        for (key, value) in link.query_pairs() {
            if matches!(key.as_ref(), "single" | "comment" | "t" | "task" | "option")
                || (key == "thread" && thread.is_none())
            {
                query.append_pair(&key, &value);
            }
        }
    }
    Some(uri.into())
}

impl Telegram {
    pub(crate) async fn source_message_uri(
        &self,
        chat: i64,
        message: i64,
        cancel: &CancellationToken,
    ) -> Option<String> {
        let link = self.message_link(chat, message, cancel).await.ok()?;
        message_uri(&link)
    }

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
    async fn resolves_public_and_private_source_message_links() {
        let tg = Telegram::mock_rpc(|request| {
            assert_eq!(request["@type"], "getMessageLink");
            assert_eq!(request["chat_id"], 51);
            match request["message_id"].as_i64().unwrap() {
                1 => json!({"link":"https://t.me/Music_42/24"}),
                2 => json!({"link":"https://t.me/c/42/3/25?t=3"}),
                3 => json!({"link":"https://example.org/Music_42/24"}),
                _ => json!({"@type":"error","code":400,"message":"Unavailable"}),
            }
        });
        for (message, uri) in [
            (1, Some("tg://resolve?domain=Music_42&post=24")),
            (2, Some("tg://privatepost?channel=42&post=25&thread=3&t=3")),
            (3, None),
            (4, None),
        ] {
            assert_eq!(
                tg.source_message_uri(51, message, &CancellationToken::new())
                    .await
                    .as_deref(),
                uri
            );
        }
    }

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
