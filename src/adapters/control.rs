//! CLI client and bridge to the existing private single-instance channel.
use crate::application::control::{Pending, Reply, Request, RowRange, REPLY_LIMIT};
use crate::identity::{APP_ID, APP_NAME, IS_RELEASE};
use crate::{domain::model::QueueEntryId, launch::ProcessIdentity, tr, RepeatMode};
use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand, ValueEnum};
use fastframe_instance::Slot;
use std::{
    num::{NonZeroU64, NonZeroUsize},
    process::ExitCode,
    sync::mpsc,
    time::{Duration, Instant},
};

mod output;

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
    disable_help_subcommand = true,
    about = tr("Send a command to an already running player."),
    after_help = tr("Start {bin} --demo in one terminal. Use these commands in a second terminal.\n  {bin} --demo --ctl search Jazz\n  {bin} --demo --ctl open 2\n  {bin} --demo --ctl playlist\n  {bin} --demo --ctl select 1\n  {bin} --demo --ctl quit").replace("{bin}", APP_ID)
)]
struct ControlArgs {
    #[arg(long, global = true, help = tr("Write command results and errors as JSON for scripts"))]
    json: bool,
    #[command(subcommand)]
    command: Option<ControlCommand>,
}

#[derive(Subcommand)]
enum ControlCommand {
    #[command(about = tr("Show playback and queue status"))]
    Status,
    #[command(about = tr("Save the session and wait for the player to exit"))]
    Quit,
    #[command(about = tr("List known playlists and their chat IDs"))]
    Playlists {
        #[arg(default_value_t, help = tr("Offset for the page, using next_offset from the previous JSON reply"))]
        offset: usize,
    },
    #[command(about = tr("List numbered tracks in the open playlist"))]
    Playlist {
        #[arg(default_value_t, help = tr("Offset for the page, using next_offset from the previous JSON reply"))]
        offset: usize,
    },
    #[command(about = tr("List the current track and numbered queue rows"))]
    Queue {
        #[arg(default_value_t, allow_negative_numbers = true, help = tr("Skip upcoming rows with a positive offset. Use -20 to see up to 20 earlier rows."))]
        offset: i64,
    },
    #[command(about = tr("Add rows from the open playlist to the queue"))]
    QueueAdd {
        #[arg(required = true, value_parser = row_range, help = tr("Playlist row numbers or inclusive ranges such as 5-8"))]
        rows: Vec<RowRange>,
        #[arg(long, help = tr("Insert before the upcoming tracks"))]
        next: bool,
        #[arg(long, help = tr("Reverse all selected rows before adding them"))]
        reverse: bool,
    },
    #[command(about = tr("Remove rows from the queue"))]
    QueueRemove {
        #[arg(required = true, value_parser = row_range, help = tr("Queue row numbers or inclusive ranges such as 5-8"))]
        rows: Vec<RowRange>,
    },
    #[command(about = tr("Reverse a continuous range of queue rows"))]
    QueueReverse {
        #[arg(value_parser = row_range, help = tr("Inclusive range of at least two queue rows, such as 5-8"))]
        rows: RowRange,
    },
    #[command(about = tr("Find chats by name, with Telegram results when connected"))]
    Search {
        #[arg(value_parser = search_query, help = tr("Chat name or part of it. Repeat the command to see pending results."))]
        query: String,
        #[arg(default_value_t, help = tr("Offset for the page, using next_offset from the previous JSON reply"))]
        offset: usize,
    },
    #[command(about = tr("List locally pinned chats"))]
    Pinned {
        #[arg(default_value_t, help = tr("Offset for the page, using next_offset from the previous JSON reply"))]
        offset: usize,
    },
    #[command(about = tr("Open a chat as a playlist"))]
    Open {
        #[arg(allow_negative_numbers = true, help = tr("chat_id from search, playlists or pinned"))]
        chat_id: i64,
    },
    #[command(about = tr("Pin a chat in Gramwave"))]
    Pin {
        #[arg(allow_negative_numbers = true, help = tr("chat_id from search or playlists"))]
        chat_id: i64,
    },
    #[command(about = tr("Unpin a chat in Gramwave"))]
    Unpin {
        #[arg(allow_negative_numbers = true, help = tr("chat_id from pinned"))]
        chat_id: i64,
    },
    #[command(about = tr("Play a track from the open playlist"))]
    Select {
        #[arg(help = tr("Row number from playlist, starting at 1"))]
        row: NonZeroUsize,
    },
    #[command(about = tr("Play a track from the queue by row number"))]
    QueuePlay {
        #[arg(required_unless_present = "id", conflicts_with = "id", help = tr("Queue row number, starting at 1"))]
        row: Option<NonZeroUsize>,
        #[arg(long, value_name = "ENTRY_ID", help = tr("Select an upcoming track by entry_id from queue --json"))]
        id: Option<NonZeroU64>,
    },
    #[command(about = tr("Play the next track"))]
    Next,
    #[command(about = tr("Restart the track or go back in playback history"))]
    Previous,
    #[command(about = tr("Pause playback"))]
    Pause,
    #[command(about = tr("Resume playback"))]
    Resume,
    #[command(about = tr("Go to a position in the current track"))]
    Seek {
        #[arg(allow_negative_numbers = true, value_parser = seek_seconds, help = tr("Seconds from the start, for example 90.5"))]
        seconds: f64,
    },
    #[command(about = tr("Turn shuffle on or off"))]
    Shuffle { state: ShuffleValue },
    #[command(about = tr("Set repeat mode for the queue or current track"))]
    Repeat { mode: RepeatValue },
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

fn search_query(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(tr("Enter a chat name to search for"))
    } else {
        Ok(value.to_owned())
    }
}

fn row_range(value: &str) -> Result<RowRange, String> {
    let (first, last) = value.split_once('-').unwrap_or((value, value));
    if let (Ok(first), Ok(last)) = (first.parse::<NonZeroUsize>(), last.parse::<NonZeroUsize>()) {
        if first <= last {
            return Ok(RowRange {
                first: first.get(),
                last: last.get(),
            });
        }
    }
    Err(tr(
        "Use a positive row number or an ascending range such as 5-8",
    ))
}

fn parse(args: &[String]) -> Result<Option<Request>, clap::Error> {
    use ControlCommand::*;
    let Some(command) = ControlArgs::try_parse_from(args)?.command else {
        return Err(clap::Error::raw(
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
            ControlArgs::command().render_help().to_string(),
        ));
    };
    Ok(Some(match command {
        Status => Request::Status,
        Quit => return Ok(None),
        Playlists { offset } => Request::Playlists { offset },
        Playlist { offset } => Request::Playlist { offset },
        Queue { offset } => Request::Queue { offset },
        QueueAdd {
            rows,
            next,
            reverse,
        } => Request::QueueAdd {
            rows,
            next,
            reverse,
        },
        QueueRemove { rows } => Request::QueueRemove { rows },
        QueueReverse { rows } => Request::QueueReverse { rows },
        Search { query, offset } => Request::Search { query, offset },
        Pinned { offset } => Request::Pinned { offset },
        Open { chat_id } => Request::Open { chat_id },
        Pin { chat_id } => Request::Pin {
            chat_id,
            pinned: true,
        },
        Unpin { chat_id } => Request::Pin {
            chat_id,
            pinned: false,
        },
        Select { row } => Request::Select { row: row.get() },
        QueuePlay { id: Some(id), .. } => Request::QueuePlay {
            entry_id: QueueEntryId(id.get()),
        },
        QueuePlay { row, .. } => Request::QueuePlayRow {
            row: row.expect("required by Clap").get(),
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
    }))
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

pub(crate) fn send(slot: &Slot, request: &Request) -> Reply {
    let result = (|| {
        let request = format!(
            "{PREFIX}{}",
            serde_json::to_string(request).map_err(|e| e.to_string())?
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

pub(crate) fn instance(slot: &Slot) -> Result<ProcessIdentity, String> {
    let reply = send(slot, &Request::Instance);
    reply
        .data
        .and_then(|data| serde_json::from_value(data).ok())
        .ok_or_else(|| reply.error.unwrap_or_else(|| tr("Invalid player identity")))
}

fn quit(slot: &Slot) -> Result<(), String> {
    let identity = instance(slot)?;
    let reply = send(slot, &Request::Quit { identity });
    if !reply.ok && identity.is_running()? {
        return Err(reply
            .error
            .unwrap_or_else(|| tr("Player rejected the quit command")));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while identity.is_running()? {
        if Instant::now() >= deadline {
            return Err(tr("Timed out waiting for the player to exit"));
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Ok(())
}

/// Run before constructing the GUI or opening account data.
pub fn run_cli() -> Option<ExitCode> {
    let (json, options) = crate::launch::control_options()?;
    let options = match options {
        Ok(options) => options,
        Err(error) => return Some(output::parse_error(json, error)),
    };
    let request = match parse(options.ctl.as_deref().unwrap_or_default()) {
        Ok(request) => request,
        Err(error) => return Some(output::parse_error(json, &error)),
    };
    if options.headless
        || options.daemon
        || options.light
        || options.desktop_integrations
        || options.smoke_test
        || options.scenario.is_some()
        || options.preview_auth.is_some()
        || options.size.is_some()
        || options.scale.is_some()
        || options.screenshot.is_some()
    {
        return Some(output::print(
            json,
            None,
            Reply::failure(tr("Only --demo can be combined with --ctl")),
        ));
    }
    let slot = slot(options.demo);
    let reply = match &request {
        Some(request) => send(&slot, request),
        None => match quit(&slot) {
            Ok(()) => Reply::success(serde_json::json!({"stopped": true})),
            Err(error) => Reply::failure(error),
        },
    };
    Some(output::print(json, request.as_ref(), reply))
}

#[cfg(test)]
#[path = "control/tests.rs"]
mod tests;
