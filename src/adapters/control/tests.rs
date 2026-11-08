use super::*;

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cli_arguments_are_explicit_and_validated_before_connecting() {
    for words in [
        vec![],
        vec!["select", "0"],
        vec!["seek", "NaN"],
        vec!["seek", "inf"],
        vec!["seek", "-1"],
        vec!["next", "1"],
        vec!["queue", "-1"],
        vec!["open"],
        vec!["shuffle", "yes"],
        vec!["status", "extra", "extra"],
    ] {
        assert!(parse(&args(&words)).is_err(), "{words:?}");
    }
    assert!(matches!(
        parse(&args(&["open", "-123"])).unwrap(),
        Request::Open { chat_id: -123 }
    ));
    assert!(matches!(
        parse(&args(&["select", "5"])).unwrap(),
        Request::Select { row: 5 }
    ));
    assert!(matches!(
        parse(&args(&["repeat", "all"])).unwrap(),
        Request::Repeat {
            mode: RepeatMode::All
        }
    ));
    assert!(matches!(
        parse(&args(&["playlist", "50"])).unwrap(),
        Request::Playlist { offset: 50 }
    ));
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
