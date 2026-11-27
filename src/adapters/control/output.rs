use super::{Reply, Request};
use crate::tr;
use clap::error::ErrorKind;
use serde_json::Value;
use std::{fmt::Write, process::ExitCode};

fn label(value: &str) -> String {
    value.replace(char::is_control, " ")
}

pub(super) fn json(reply: &Reply) {
    println!(
        "{}",
        serde_json::to_string(reply).expect("serializable control reply")
    );
}

pub(super) fn parse_error(json_mode: bool, error: &clap::Error) -> ExitCode {
    if error.kind() == ErrorKind::DisplayHelp {
        let _ = error.print();
        return ExitCode::SUCCESS;
    }
    if json_mode {
        json(&Reply::failure(error.to_string()));
    } else {
        let _ = error.print();
    }
    ExitCode::FAILURE
}

pub(super) fn print(json_mode: bool, request: Option<&Request>, reply: Reply) -> ExitCode {
    if json_mode {
        json(&reply);
    } else if reply.ok {
        if let Some(data) = reply.data {
            print!("{}", text(request, &data));
        }
    } else {
        eprintln!("{}", label(reply.error.as_deref().unwrap_or("")));
    }
    if reply.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn track(item: &Value) -> String {
    let title = label(item["title"].as_str().unwrap_or_default());
    match item["artist"].as_str().filter(|artist| !artist.is_empty()) {
        Some(artist) => format!("{} — {title}", label(artist)),
        None => title,
    }
}

pub(super) fn text(request: Option<&Request>, data: &Value) -> String {
    let (name, field) = match request {
        None => return format!("{}\n", tr("Player stopped.")),
        Some(Request::Playlists { .. }) => ("playlists", "chat_id"),
        Some(Request::Playlist { .. }) => ("playlist", "row"),
        Some(Request::Queue { .. }) => ("queue", "row"),
        Some(Request::Search { .. }) => ("search", "chat_id"),
        Some(Request::Pinned { .. }) => ("pinned", "chat_id"),
        Some(Request::Pin { .. }) => {
            let message = if data["pinned"].as_bool() == Some(true) {
                tr("Pinned")
            } else {
                tr("Unpinned")
            };
            return format!("{message} {}\n", track(data));
        }
        _ => return status(data),
    };
    let mut out = String::new();
    let current = data.get("current").filter(|track| !track.is_null());
    let past = matches!(request, Some(Request::Queue { offset }) if *offset < 0);
    if let Some(current) = current.filter(|_| !past) {
        let _ = writeln!(out, "> {}  {}", current["row"], track(current));
    }
    if let Some(items) = data["items"].as_array() {
        if items.is_empty() {
            let _ = writeln!(out, "{}", tr("No entries."));
        }
        for item in items {
            let _ = writeln!(out, "{}  {}", item[field], track(item));
        }
    }
    if let Some(current) = current.filter(|_| past) {
        let _ = writeln!(out, "> {}  {}", current["row"], track(current));
    }
    if let Some(row) = data["deferred_from_row"].as_u64() {
        let _ = writeln!(
            out,
            "{}",
            tr("More tracks will play before row {row}").replace("{row}", &row.to_string())
        );
    } else if data["playlist_pending"].as_bool() == Some(true) {
        let _ = writeln!(out, "{}", tr("More tracks from the source chat"));
    }
    if let Some(next) = data["next_offset"].as_i64() {
        let command = if let Some(Request::Search { query, .. }) = request {
            let query = label(query).replace('\'', "'\\''");
            format!("--ctl search '{query}' {next}")
        } else {
            format!("--ctl {name} {next}")
        };
        let _ = writeln!(
            out,
            "{}",
            tr("Run {command} for the next page.").replace("{command}", &command)
        );
    }
    if data["loading"].as_bool() == Some(true) {
        let _ = writeln!(
            out,
            "{}",
            tr("Searching Telegram. Repeat this command for updated results.")
        );
    }
    if let Some(error) = data["search_error"].as_str() {
        let _ = writeln!(out, "{} {}", tr("Telegram search failed."), label(error));
    }
    out
}

fn status(data: &Value) -> String {
    let mut out = String::new();
    if let Some(error) = data["queue_error"].as_str() {
        let _ = writeln!(
            out,
            "{} {}",
            tr("Could not load the rest of the queue"),
            label(error)
        );
    }
    if data["current"].is_null() {
        let _ = writeln!(out, "{}", tr("No track selected"));
    } else {
        let state = if data["queue_waiting"].as_bool() == Some(true) {
            tr("Waiting for more tracks…")
        } else if data["loading"].as_bool() == Some(true) {
            tr("Loading")
        } else if data["paused"].as_bool() == Some(true) {
            tr("Paused")
        } else {
            tr("Playing")
        };
        let position = data["position"].as_f64().unwrap_or_default();
        let _ = writeln!(out, "{state}  {position:.1}s  {}", track(&data["current"]));
    }
    if let Some(chat) = data["opened_chat"].as_i64() {
        let _ = writeln!(out, "{} {chat}", tr("Open playlist"));
    }
    let shuffle = if data["shuffle"].as_bool() == Some(true) {
        tr("on")
    } else {
        tr("off")
    };
    let repeat = match data["repeat"].as_str() {
        Some("all") => tr("all"),
        Some("one") => tr("one"),
        _ => tr("off"),
    };
    let _ = writeln!(
        out,
        "{} {}  {} {shuffle}  {} {repeat}",
        tr("Queue"),
        data["queue_length"],
        tr("Shuffle"),
        tr("Repeat")
    );
    if let Some(error) = data["error"].as_str().filter(|error| !error.is_empty()) {
        let _ = writeln!(out, "{}", label(error));
    }
    out
}
