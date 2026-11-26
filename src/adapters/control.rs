//! CLI client and bridge to the existing private single-instance channel.
use crate::application::control::{Pending, Reply, Request, REPLY_LIMIT};
use crate::identity::{APP_ID, APP_NAME, IS_RELEASE};
use crate::{domain::model::QueueEntryId, RepeatMode};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use fastframe_instance::Slot;
use std::{
    num::{NonZeroU64, NonZeroUsize},
    process::ExitCode,
    sync::mpsc,
    time::{Duration, Instant},
};

const PREFIX: &str = "control-v1:";
const TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn slot(demo: bool) -> Slot {
    let slot = Slot::new(if IS_RELEASE {
        "org.gramwave.App"
    } else {
        "org.gramwave.Dev"
    });
    if demo {
        slot.scoped("demo")
    } else {
        slot
    }
}

#[derive(Parser)]
#[command(
    bin_name = format!("{APP_ID} [--demo] --ctl"),
    no_binary_name = true,
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
struct ControlArgs {
    #[command(subcommand)]
    command: ControlCommand,
}

#[derive(Subcommand)]
enum ControlCommand {
    Status,
    Playlists {
        #[arg(default_value_t)]
        offset: usize,
    },
    Playlist {
        #[arg(default_value_t)]
        offset: usize,
    },
    Queue {
        #[arg(default_value_t)]
        offset: usize,
    },
    Open {
        #[arg(allow_negative_numbers = true)]
        chat_id: i64,
    },
    Select {
        row: NonZeroUsize,
    },
    QueuePlay {
        entry_id: NonZeroU64,
    },
    Next,
    Previous,
    Pause,
    Resume,
    Seek {
        #[arg(allow_negative_numbers = true, value_parser = seek_seconds)]
        seconds: f64,
    },
    Shuffle {
        state: ShuffleValue,
    },
    Repeat {
        mode: RepeatValue,
    },
}

#[derive(Clone, ValueEnum)]
enum ShuffleValue {
    On,
    Off,
}

#[derive(Clone, ValueEnum)]
enum RepeatValue {
    Off,
    All,
    One,
}

fn seek_seconds(value: &str) -> Result<f64, &'static str> {
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.)
        .ok_or("Seek position must be finite and nonnegative")
}

fn parse(args: &[String]) -> Result<Request, String> {
    use ControlCommand::*;
    let args = ControlArgs::try_parse_from(args).map_err(|error| error.to_string())?;
    Ok(match args.command {
        Status => Request::Status,
        Playlists { offset } => Request::Playlists { offset },
        Playlist { offset } => Request::Playlist { offset },
        Queue { offset } => Request::Queue { offset },
        Open { chat_id } => Request::Open { chat_id },
        Select { row } => Request::Select { row: row.get() },
        QueuePlay { entry_id } => Request::QueuePlay {
            entry_id: QueueEntryId(entry_id.get()),
        },
        Next => Request::Next,
        Previous => Request::Previous,
        Pause => Request::Pause,
        Resume => Request::Resume,
        Seek { seconds } => Request::Seek { seconds },
        Shuffle { state } => Request::Shuffle {
            enabled: matches!(state, ShuffleValue::On),
        },
        Repeat { mode } => Request::Repeat {
            mode: match mode {
                RepeatValue::Off => RepeatMode::Off,
                RepeatValue::All => RepeatMode::All,
                RepeatValue::One => RepeatMode::One,
            },
        },
    })
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
            .map_err(|_| format!("{APP_NAME} control is busy or unavailable"))?;
        wake();
        receiver
            .recv_timeout(TIMEOUT)
            .map_err(|_| format!("{APP_NAME} did not process the command within 3 seconds"))
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
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => format!("{APP_NAME} is not running; start the app first"),
            std::io::ErrorKind::PermissionDenied => format!("Control unavailable: check socket permissions and restart {APP_NAME} with the updated binary"),
            _ => format!("Cannot contact {APP_NAME}: {e}"),
        })?;
        serde_json::from_str(&response).map_err(|_| {
            format!("The running {APP_NAME} does not support this CLI; restart it with the updated binary")
        })
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
        Ok(_) => Reply::failure(ControlArgs::command().render_help().to_string()),
        Err(error) => Reply::failure(error),
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
