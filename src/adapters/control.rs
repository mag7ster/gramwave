//! CLI client and bridge to the existing private single-instance channel.
use crate::application::control::{Pending, Reply, Request, REPLY_LIMIT};
use crate::{domain::model::QueueEntryId, RepeatMode};
use fastframe_instance::Slot;
use std::{
    process::ExitCode,
    sync::mpsc,
    time::{Duration, Instant},
};

const PREFIX: &str = "control-v1:";
const TIMEOUT: Duration = Duration::from_secs(3);
const USAGE: &str = "Usage: tg-player [--demo] --ctl status|playlists [offset]|playlist [offset]|queue [offset]|open <chat-id>|select <row>|queue-play <entry-id>|next|previous|pause|resume|seek <seconds>|shuffle on|off|repeat off|all|one";

pub(crate) fn slot(demo: bool) -> Slot {
    let slot = Slot::new("org.tgplayer.App");
    if demo {
        slot.scoped("demo")
    } else {
        slot
    }
}

fn parse(args: &[String]) -> Result<Request, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(USAGE.into());
    };
    let value = args.get(1).map(String::as_str);
    if args.len() > 2 {
        return Err(USAGE.into());
    }
    let number = || value.ok_or_else(|| USAGE.to_owned());
    let offset = || {
        value
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| "Page offset must be a nonnegative integer".to_owned())
    };
    let request = match command {
        "status" if value.is_none() => Request::Status,
        "playlists" => Request::Playlists { offset: offset()? },
        "playlist" => Request::Playlist { offset: offset()? },
        "queue" => Request::Queue { offset: offset()? },
        "open" => Request::Open {
            chat_id: number()?
                .parse()
                .map_err(|_| "Chat ID must be an integer")?,
        },
        "select" => Request::Select {
            row: number()?
                .parse::<usize>()
                .ok()
                .filter(|r| *r > 0)
                .ok_or("Row must be a positive integer (starting at 1)")?,
        },
        "queue-play" => Request::QueuePlay {
            entry_id: QueueEntryId(
                number()?
                    .parse::<u64>()
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or("Entry ID must be a positive integer")?,
            ),
        },
        "next" if value.is_none() => Request::Next,
        "previous" if value.is_none() => Request::Previous,
        "pause" if value.is_none() => Request::Pause,
        "resume" if value.is_none() => Request::Resume,
        "seek" => Request::Seek {
            seconds: number()?
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.)
                .ok_or("Seek position must be finite and nonnegative")?,
        },
        "shuffle" => Request::Shuffle {
            enabled: match number()? {
                "on" => true,
                "off" => false,
                _ => return Err("Use shuffle on|off".into()),
            },
        },
        "repeat" => Request::Repeat {
            mode: match number()? {
                "off" => RepeatMode::Off,
                "all" => RepeatMode::All,
                "one" => RepeatMode::One,
                _ => return Err("Use repeat off|all|one".into()),
            },
        },
        _ => return Err(USAGE.into()),
    };
    Ok(request)
}

pub(crate) fn bridge(
    line: &str,
    sender: &mpsc::SyncSender<Pending>,
    wake: impl FnOnce(),
) -> Option<String> {
    let line = line.strip_prefix(PREFIX)?;
    let result = (|| {
        let request =
            serde_json::from_str(line).map_err(|e| format!("Invalid control request: {e}"))?;
        let (reply, receiver) = mpsc::sync_channel(1);
        sender
            .try_send(Pending {
                request,
                deadline: Instant::now() + TIMEOUT,
                reply,
            })
            .map_err(|_| "TG Player control is busy or unavailable".to_owned())?;
        wake();
        receiver
            .recv_timeout(TIMEOUT)
            .map_err(|_| "TG Player did not process the command within 3 seconds".to_owned())
    })();
    let reply = result.unwrap_or_else(Reply::failure);
    let encoded = serde_json::to_string(&reply).expect("serializable control reply");
    Some(if encoded.len() > REPLY_LIMIT {
        serde_json::to_string(&Reply::failure("Control response exceeds the page limit")).unwrap()
    } else {
        encoded
    })
}

fn send(slot: &Slot, args: &[String]) -> Reply {
    let result = (|| {
        let request = parse(args)?;
        let request = format!(
            "{PREFIX}{}",
            serde_json::to_string(&request).map_err(|e| e.to_string())?
        );
        let response = slot.send(&request).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => "TG Player is not running; start the app first".to_owned(),
            std::io::ErrorKind::PermissionDenied => "Control unavailable: check socket permissions and restart TG Player with the updated binary".to_owned(),
            _ => format!("Cannot contact TG Player: {e}"),
        })?;
        serde_json::from_str(&response).map_err(|_| "The running TG Player does not support this CLI; restart it with the updated binary".to_owned())
    })();
    result.unwrap_or_else(Reply::failure)
}

/// Run before constructing the GUI or opening account data. A control command
/// never claims a slot, creates an application, or starts a replacement instance.
pub fn run_cli() -> Option<ExitCode> {
    let options = crate::launch::control_options()?;
    let reply = match options {
        Ok(options)
            if !options.light
                && !options.desktop_integrations
                && !options.smoke_test
                && options.scenario.is_none()
                && options.preview_auth.is_none()
                && options.size.is_none()
                && options.scale.is_none()
                && options.screenshot.is_none() =>
        {
            send(
                &slot(options.demo),
                options.ctl.as_deref().unwrap_or_default(),
            )
        }
        _ => Reply::failure(USAGE),
    };
    println!(
        "{}",
        serde_json::to_string(&reply).expect("serializable control reply")
    );
    Some(if reply.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(test)]
#[path = "control/tests.rs"]
mod tests;
