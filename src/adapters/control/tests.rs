use super::*;

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cli_arguments_are_explicit_and_validated_before_connecting() {
    for words in [
        vec![],
        vec!["select", "0"],
        vec!["queue-play", "0"],
        vec!["queue-play", "--id", "0"],
        vec!["queue-play", "1", "--id", "2"],
        vec!["seek", "NaN"],
        vec!["seek", "inf"],
        vec!["seek", "-1"],
        vec!["next", "1"],
        vec!["queue-add"],
        vec!["queue-add", "0-1"],
        vec!["queue-remove", "9-3"],
        vec!["queue-reverse", "1--3"],
        vec!["open"],
        vec!["shuffle", "yes"],
        vec!["status", "extra", "extra"],
        vec!["--quit"],
        vec!["quit", "status"],
        vec!["search", "  "],
    ] {
        assert!(parse(&args(&words)).is_err(), "{words:?}");
    }
    assert!(matches!(
        parse(&args(&["open", "-123"])).unwrap().unwrap(),
        Request::Open { chat_id: -123 }
    ));
    assert!(matches!(
        parse(&args(&["select", "5"])).unwrap().unwrap(),
        Request::Select { row: 5 }
    ));
    assert!(matches!(
        parse(&args(&["repeat", "all"])).unwrap().unwrap(),
        Request::Repeat {
            mode: RepeatMode::All
        }
    ));
    assert!(matches!(
        parse(&args(&["playlist", "50"])).unwrap().unwrap(),
        Request::Playlist { offset: 50 }
    ));
    assert!(matches!(
        parse(&args(&["queue"])).unwrap().unwrap(),
        Request::Queue { offset: 0 }
    ));
    assert!(matches!(
        parse(&args(&["queue-play", "--id", "18446744073709551615"]))
            .unwrap()
            .unwrap(),
        Request::QueuePlay {
            entry_id: QueueEntryId(u64::MAX)
        }
    ));
    assert!(matches!(
        parse(&args(&["queue-play", "5"])).unwrap(),
        Some(Request::QueuePlayRow { row: 5 })
    ));
    assert!(matches!(
        parse(&args(&["queue", "-20"])).unwrap(),
        Some(Request::Queue { offset: -20 })
    ));
    let Some(Request::QueueAdd {
        rows,
        next: true,
        reverse: true,
    }) = parse(&args(&["queue-add", "2", "--next", "5-8", "--reverse"])).unwrap()
    else {
        panic!("expected queue-add with both flags");
    };
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[1].first, rows[1].last), (5, 8));
    assert!(parse(&args(&["--help"]))
        .unwrap_err()
        .to_string()
        .contains("queue-play"));
    for words in [["--json", "quit"], ["quit", "--json"]] {
        assert!(parse(&args(&words)).unwrap().is_none());
    }
    for words in [["--json", "status"], ["status", "--json"]] {
        assert!(matches!(
            parse(&args(&words)).unwrap(),
            Some(Request::Status)
        ));
    }
}

#[test]
fn text_queue_uses_row_numbers_and_keeps_metadata_on_one_line() {
    let data = serde_json::json!({
        "items": [{"row": 1, "entry_id": u64::MAX, "title": "Night\nTrain", "artist": "Jazz\u{1b}[31m"}],
        "current": {"row": 4, "title": "On air"},
        "next_offset": -2,
    });
    let text = output::text(Some(&Request::Queue { offset: -3 }), &data);
    assert_eq!(text.lines().next(), Some("1  Jazz [31m — Night Train"));
    assert_eq!(text.lines().nth(1), Some("> 4  On air"));
    assert!(text.contains("--ctl queue -2"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn bridge_waits_for_application_reply_and_rejects_unknown_protocols() {
    let (tx, rx) = mpsc::sync_channel::<Pending>(8);
    assert!(bridge("unknown", &tx, || panic!("must not wake")).is_none());
    let worker = std::thread::spawn(move || {
        let pending = rx.recv().unwrap();
        assert!(matches!(pending.request, Request::Select { row: 5 }));
        pending
            .reply
            .send(Reply::success(serde_json::json!({"applied": true})))
            .unwrap();
    });
    let result = bridge("control-v1:{\"command\":\"select\",\"row\":5}", &tx, || {}).unwrap();
    let reply: Reply = serde_json::from_str(&result).unwrap();
    assert!(reply.ok);
    assert_eq!(reply.data.unwrap()["applied"], true);
    worker.join().unwrap();
    let result = bridge(
        "control-v1:{\"command\":\"next\",\"unexpected\":1}",
        &tx,
        || {},
    )
    .unwrap();
    assert!(!serde_json::from_str::<Reply>(&result).unwrap().ok);
}
