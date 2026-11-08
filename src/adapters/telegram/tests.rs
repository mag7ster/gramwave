use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn read_only_presence_policy() {
        assert!(!privacy_safe(
            &json!({"@type":"setOption","name":"online","value":{"@type":"optionValueEmpty"}})
        ));
        assert!(!privacy_safe(
            &json!({"@type":"setOption","name":"online","value":{"value":true}})
        ));
        assert!(privacy_safe(
            &json!({"@type":"setOption","name":"online","value":{"value":false}})
        ));
        for method in [
            "viewMessages",
            "openMessageContent",
            "sendChatAction",
            "sendMessage",
        ] {
            assert!(!privacy_safe(&json!({"@type":method})));
        }
        assert!(privacy_safe(&json!({"@type":"downloadFile"})));
        assert!(!privacy_safe(&json!({"@type":"unknownFutureMethod"})));
        assert!(!privacy_safe(
            &json!({"@type":"setOption","name":"unknown","value":{"value":false}})
        ));
    }
}

#[cfg(all(test, feature = "telegram", feature = "native-tests"))]
mod native_tests {
    #[test]
    fn native_tdlib_version() {
        let (tg, _) = super::Telegram::start().unwrap();
        let v = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(tg.call_async(
                serde_json::json!({"@type":"getOption","name":"version"}),
                &Default::default(),
            ))
            .unwrap();
        assert_eq!(v["value"], "1.8.61");
        tg.shutdown(std::time::Duration::from_secs(5)).unwrap();
    }
}

#[test]
fn cancelled_and_dropped_rpc_remove_waiters_and_ignore_late_replies() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (started, ready) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let tg = Telegram::mock_rpc(move |_| {
        started.send(()).unwrap();
        blocked.recv().unwrap();
        json!({"ok":true})
    });
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let client = tg.clone();
    let task = runtime.spawn(async move {
        client
            .call_async(json!({"@type":"getFile","file_id":1}), &token)
            .await
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(tg.pending.lock().unwrap().len(), 1);
    cancel.cancel();
    assert_eq!(
        runtime.block_on(task).unwrap().unwrap_err(),
        AppError::Cancelled
    );
    assert!(tg.pending.lock().unwrap().is_empty());
    release.send(()).unwrap();
    let client = tg.clone();
    let task = runtime.spawn(async move {
        client
            .call_async(json!({"@type":"getFile","file_id":1}), &Default::default())
            .await
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    task.abort();
    let _ = runtime.block_on(task);
    assert!(tg.pending.lock().unwrap().is_empty());
    release.send(()).unwrap();
    tg.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn shared_download_is_cancelled_only_after_its_last_consumer() {
    let (cancelled, done) = mpsc::channel();
    let count = Arc::new(AtomicU64::new(0));
    let calls = count.clone();
    let tg = Telegram::mock_rpc(move |v| {
        if v["@type"] == "downloadFile" {
            calls.fetch_add(1, Ordering::Relaxed);
        }
        if v["@type"] == "cancelDownloadFile" {
            cancelled.send(()).unwrap();
        }
        json!({"ok":true})
    });
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let offline = tg
            .begin_download(1, 0, true, &Default::default())
            .await
            .unwrap();
        let streaming = tg
            .begin_download(1, 50, false, &Default::default())
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 2);
        drop(offline);
        assert!(done.try_recv().is_err());
        drop(streaming);
        done.recv_timeout(Duration::from_secs(1)).unwrap();
    });
}

#[test]
fn rpc_deadline_is_typed_and_removes_its_registration() {
    let (started, ready) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let tg = Telegram::mock_rpc(move |_| {
        started.send(()).unwrap();
        blocked.recv().unwrap();
        json!({"ok":true})
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = tg.clone();
    let task = runtime.spawn(async move {
        client
            .call_with_timeout(
                json!({"@type":"getFile","file_id":1}),
                &Default::default(),
                Duration::from_millis(20),
            )
            .await
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        runtime.block_on(task).unwrap(),
        Err(AppError::Timeout("Telegram RPC"))
    );
    assert!(tg.pending.lock().unwrap().is_empty());
    release.send(()).unwrap();
}
#[test]
fn streaming_seek_temporarily_prioritizes_its_range_and_offline_then_resumes() {
    let (sent, received) = mpsc::channel();
    let tg = Telegram::mock_rpc(move |v| {
        if v["@type"] == "downloadFile" {
            sent.send(v["offset"].as_u64().unwrap()).unwrap();
        }
        json!({"ok":true})
    });
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let offline = tg
            .begin_download(1, 0, true, &Default::default())
            .await
            .unwrap();
        assert_eq!(received.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
        let stream = tg
            .begin_download(1, 500_000, false, &Default::default())
            .await
            .unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            500_000
        );
        drop(stream);
        assert_eq!(received.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
        drop(offline);
    });
}
