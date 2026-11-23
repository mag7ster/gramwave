use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn byte_ranges() {
        assert_eq!(parse_range(None, 100), Ok((0, 99)));
        assert_eq!(parse_range(Some("30-"), 100), Ok((30, 99)));
        assert_eq!(parse_range(Some("-20"), 100), Ok((80, 99)));
        assert_eq!(parse_range(Some("20-300"), 100), Ok((20, 99)));
        for r in ["100-", "9-3", "0-1,4-5", "-0", "a-"] {
            assert!(parse_range(Some(r), 100).is_err());
        }
    }
}

#[cfg(all(test, feature = "native-tests"))]
mod streaming_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
    };
    #[test]
    fn cached_range_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let file = dir.join("sample");
        std::fs::write(&file, b"0123456789abcdef").unwrap();
        let workers = crate::application::tasks::Scheduler::new();
        let (base, mut server) = serve(Telegram::mock_file(file), workers.handle()).unwrap();
        let authority = base.strip_prefix("http://").unwrap();
        let (host, path) = authority.split_once('/').unwrap();
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            socket,
            "GET /{path}/1 HTTP/1.1\r\nHost: {host}\r\nRange: bytes=4-8\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 206"));
        assert!(response.ends_with("45678"));
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(socket, "GET /wrong/1 HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"));
        server.shutdown(Duration::from_secs(7)).unwrap();
    }
}

#[test]
fn head_ranges_permits_and_cancellation_follow_body_lifetime() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("media");
    std::fs::write(&path, b"0123456789abcdef").unwrap();
    let downloads = Arc::new(AtomicUsize::new(0));
    let count = downloads.clone();
    let tg = Telegram::mock_rpc(move |v| {
        let id = v["file_id"].as_i64().unwrap_or(1);
        if v["@type"] == "downloadFile" {
            count.fetch_add(1, Ordering::Relaxed);
        }
        if v["@type"] == "getFileDownloadedPrefixSize" {
            json!({"size":16-v["offset"].as_u64().unwrap()})
        } else {
            json!({"@type":"file","id":id,"size":16,"local":{"path":path,"is_downloading_completed":true}})
        }
    });
    let endpoint = Endpoint {
        tg,
        token: "secret".into(),
        cancel: Default::default(),
        active: Arc::new(Semaphore::new(4)),
        admission: Arc::new(Semaphore::new(20)),
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        async fn request(endpoint: Endpoint, method: Method, range: Option<&str>) -> Response {
            let mut headers = HeaderMap::new();
            if let Some(range) = range {
                headers.insert(header::RANGE, range.parse().unwrap());
            }
            let active = endpoint.active.clone().acquire_owned().await.unwrap();
            let admission = endpoint.admission.clone().acquire_owned().await.unwrap();
            response(endpoint, 1, method, headers, admission, active)
                .await
                .unwrap()
        }
        let head = request(endpoint.clone(), Method::HEAD, None).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "16");
        assert!(axum::body::to_bytes(head.into_body(), 32)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(downloads.load(Ordering::Relaxed), 0);
        let invalid = request(endpoint.clone(), Method::GET, Some("bytes=16-")).await;
        assert_eq!(invalid.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(invalid.headers()[header::CONTENT_RANGE], "bytes */16");
        let full = request(endpoint.clone(), Method::GET, Some("bytes=0-1,4-5")).await;
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(endpoint.active.available_permits(), 3);
        assert_eq!(
            axum::body::to_bytes(full.into_body(), 32).await.unwrap(),
            &b"0123456789abcdef"[..]
        );
        assert_eq!(endpoint.active.available_permits(), 4);
        let range = request(endpoint.clone(), Method::GET, Some("bytes=4-8")).await;
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            axum::body::to_bytes(range.into_body(), 32).await.unwrap(),
            &b"45678"[..]
        );
        let body = request(endpoint.clone(), Method::GET, None).await;
        endpoint.cancel.cancel();
        assert!(axum::body::to_bytes(body.into_body(), 32).await.is_err());
        assert_eq!(endpoint.active.available_permits(), 4);
    });
}

#[test]
fn completed_stream_reads_multiple_blocks_with_constant_metadata_calls() {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("media");
    let data = [vec![1; 65536], vec![2; 65536], vec![3; 65536]].concat();
    std::fs::write(&path, &data).unwrap();
    let size = data.len();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let tg = Telegram::mock_rpc(move |request| {
        if request["@type"] != "cancelDownloadFile" {
            count.fetch_add(1, Relaxed);
        }
        json!({"@type":"file","id":1,"size":size,"local":{"path":path,"is_downloading_completed":true}})
    });
    let endpoint = Endpoint {
        tg,
        token: "secret".into(),
        cancel: Default::default(),
        active: Arc::new(Semaphore::new(4)),
        admission: Arc::new(Semaphore::new(20)),
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for range in [Some("bytes=0-9"), None] {
            calls.store(0, Relaxed);
            let mut headers = HeaderMap::new();
            if let Some(range) = range {
                headers.insert(header::RANGE, range.parse().unwrap());
            }
            let response = respond(
                State(endpoint.clone()),
                Path(("secret".into(), 1)),
                Method::GET,
                headers,
            )
            .await;
            let expected = if range.is_some() { &data[..10] } else { &data };
            assert_eq!(
                axum::body::to_bytes(response.into_body(), size)
                    .await
                    .unwrap()
                    .as_ref(),
                expected
            );
            assert_eq!(calls.load(Relaxed), 2);
        }
    });
}

#[test]
fn stream_reopens_on_file_update_without_cancelling_offline_reader() {
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let temp = tempfile::tempdir().unwrap();
    let partial = temp.path().join("partial");
    let complete = temp.path().join("complete");
    std::fs::write(&partial, b"0123").unwrap();
    std::fs::write(&complete, b"01234567").unwrap();
    let cancellations = Arc::new(AtomicUsize::new(0));
    let count = cancellations.clone();
    let tg = Telegram::mock_rpc(move |request| match request["@type"].as_str() {
        Some("cancelDownloadFile") => {
            count.fetch_add(1, Relaxed);
            json!({})
        }
        Some("getFileDownloadedPrefixSize") => {
            json!({"size": if request["offset"] == 2 { 2 } else { 0 }})
        }
        _ => json!({"@type":"file","id":1,"size":8,"local":{"path":partial}}),
    });
    let endpoint = Endpoint {
        tg: tg.clone(),
        token: "secret".into(),
        cancel: Default::default(),
        active: Arc::new(Semaphore::new(4)),
        admission: Arc::new(Semaphore::new(20)),
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let offline = tg
            .begin_download(1, 0, true, &endpoint.cancel)
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::RANGE, "bytes=2-7".parse().unwrap());
        let response = respond(
            State(endpoint.clone()),
            Path(("secret".into(), 1)),
            Method::GET,
            headers,
        )
        .await;
        let mut body = response.into_body().into_data_stream();
        assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"23");
        let read = body.next();
        let publish = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            tg.mock_file_update(
                json!({"id":1,"size":8,"local":{"path":complete,"is_downloading_completed":true}}),
            );
        };
        let (read, ()) = tokio::join!(tokio::time::timeout(Duration::from_secs(2), read), publish);
        assert_eq!(read.unwrap().unwrap().unwrap().as_ref(), b"4567");
        assert!(body.next().await.is_none());
        tg.call_async(json!({"@type":"getFile","file_id":1}), &endpoint.cancel)
            .await
            .unwrap();
        assert_eq!(cancellations.load(Relaxed), 0);
        drop(offline);
        tg.call_async(json!({"@type":"getFile","file_id":1}), &endpoint.cancel)
            .await
            .unwrap();
        assert_eq!(cancellations.load(Relaxed), 1);
    });
}
